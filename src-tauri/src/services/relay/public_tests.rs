//! Joining the public relay: the real `nandi-relay` (dev-dependency) in
//! public mode on 127.0.0.1:0, signing in against a fake GitHub device
//! flow, plus a relay address nothing answers on.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use nandi_relay::accounts::PublicConfig;
use nandi_relay::{serve_with, GitHubConfig, Limits, RelayServerConfig, SignInConfig};
use serde_json::{json, Value};

use super::*;
use crate::services::domain::group::POLICY_TEMPLATE;
use crate::services::groups::Groups;
use crate::services::relay::join::{join_relay, JoinOutcome};
use crate::services::relay::{get_relay_status, instance_token, limits, BuiltInRelay};
use crate::services::team::trust;
use crate::utils::nandi_config;

const SECRET: &[u8] = b"public-secret";

fn local() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// What the fake GitHub answers next
#[derive(Default)]
struct Fake {
    /// Polls answered `authorization_pending` before the approval
    pending: u32,
    denied: bool,
}

type Shared = Arc<Mutex<Fake>>;

async fn device_code() -> Json<Value> {
    Json(json!({
        "device_code": "dc-1", "user_code": "WDJB-MJHT",
        "verification_uri": "https://github.test/login/device",
        "expires_in": 900, "interval": 1
    }))
}

async fn access_token(State(fake): State<Shared>, body: String) -> Json<Value> {
    assert!(body.contains("device_code=dc-1"), "{body}");
    let mut fake = fake.lock().unwrap();
    if fake.denied {
        return Json(json!({ "error": "access_denied" }));
    }
    if fake.pending > 0 {
        fake.pending -= 1;
        return Json(json!({ "error": "authorization_pending" }));
    }
    Json(json!({ "access_token": "gho_583231", "token_type": "bearer" }))
}

async fn user(headers: HeaderMap) -> Json<Value> {
    assert_eq!(headers["authorization"], "Bearer gho_583231");
    Json(json!({ "id": 583_231, "login": "octocat" }))
}

async fn listen(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(local()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

/// A public relay offering GitHub sign-in (the fake), with `limits`
async fn public_relay(fake: &Shared, limits: Limits) -> String {
    let github = listen(
        Router::new()
            .route("/login/device/code", post(device_code))
            .route("/login/oauth/access_token", post(access_token))
            .route("/user", get(user))
            .with_state(fake.clone()),
    )
    .await;
    let base = format!("http://{github}");
    let mut config = PublicConfig::new(SignInConfig {
        github: Some(GitHubConfig {
            client_id: "gh-client".into(),
            web_url: base.clone(),
            api_url: base,
        }),
        google: None,
        microsoft: None,
    });
    config.signups_per_ip = 0;
    let relay = serve_with(
        local(),
        RelayServerConfig::new(SECRET.to_vec())
            .public(config)
            .with_limits(limits),
    )
    .await
    .unwrap();
    format!("ws://{relay}/relay")
}

/// A relay address nothing answers on: a port that was free a moment ago
fn nobody_there() -> String {
    let port = std::net::TcpListener::bind(local())
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    format!("ws://127.0.0.1:{port}/relay")
}

fn app_with_public(relay: &str) -> AppState {
    AppState::for_tests_with_relay(vec![], BuiltInRelay::default().with_public(Some(relay)))
}

/// Polls `check` every 20ms for up to 5s
async fn eventually(mut check: impl AsyncFnMut() -> bool) -> bool {
    for _ in 0..250 {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn connected(state: &AppState) -> bool {
    eventually(async || get_relay_status(state).await.unwrap().connected).await
}

/// Starts a GitHub sign-in: its ID and interval
async fn started(state: &AppState) -> (String, u32) {
    match start_sign_in(state, RelaySignInProvider::Github)
        .await
        .unwrap()
    {
        PublicSignInStart::Started {
            signin_id,
            provider,
            user_code,
            verification_uri,
            interval,
            expires_in,
            ..
        } => {
            assert_eq!(provider, RelaySignInProvider::Github);
            assert_eq!(user_code, "WDJB-MJHT");
            assert_eq!(verification_uri, "https://github.test/login/device");
            assert_eq!(expires_in, 900);
            (signin_id, interval)
        }
        other => panic!("not started: {other:?}"),
    }
}

/// Polls as the UI does, every `interval` seconds, until it isn't pending:
/// the answer, and whether a pending or slow_down came first
async fn polled(state: &AppState, signin_id: &str, mut interval: u32) -> (PublicSignInPoll, bool) {
    let mut waited = false;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_secs(u64::from(interval))).await;
        match poll_sign_in(state, RelaySignInProvider::Github, signin_id)
            .await
            .unwrap()
        {
            PublicSignInPoll::Pending { interval: next }
            | PublicSignInPoll::SlowDown { interval: next } => {
                waited = true;
                interval = next;
            }
            other => return (other, waited),
        }
    }
    panic!("still pending");
}

#[tokio::test]
async fn signing_in_links_this_nandi_keeps_the_token_and_connects() {
    let fake = Shared::default();
    fake.lock().unwrap().pending = 1;
    let relay = public_relay(&fake, Limits::public()).await;
    let state = app_with_public(&relay);

    // The offer: the providers the relay lists
    assert_eq!(
        offer(&state).await,
        PublicRelayOffer::Ready {
            relay: relay.clone(),
            providers: vec![RelaySignInProvider::Github],
        }
    );
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.public_relay_url.as_deref(), Some(relay.as_str()));
    assert_eq!(status.relay_url, None);
    assert_eq!(
        account(&state).await.unwrap(),
        PublicRelayAccount::NotPublic
    );

    let (signin_id, interval) = started(&state).await;
    // Not approved yet; asked again at once, the relay says to slow down
    let poll = async || {
        poll_sign_in(&state, RelaySignInProvider::Github, &signin_id)
            .await
            .unwrap()
    };
    assert_eq!(poll().await, PublicSignInPoll::Pending { interval: 1 });
    assert_eq!(poll().await, PublicSignInPoll::SlowDown { interval: 1 });
    let (answer, _) = polled(&state, &signin_id, interval).await;
    assert_eq!(
        answer,
        PublicSignInPoll::Linked {
            relay: relay.clone(),
            account: "github:583231".into(),
            provider: RelaySignInProvider::Github,
            display: "octocat".into(),
        }
    );

    // Kept exactly like an enrolled token: in relay_state, never config.json
    assert!(instance_token(state.db()).is_some());
    assert_eq!(
        nandi_config::load_from(state.nandi_dir())
            .unwrap()
            .relay_url,
        Some(relay.clone())
    );
    assert!(connected(&state).await, "never connected");
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url.as_deref(), Some(relay.as_str()));
    assert_eq!(
        status.visibility,
        Some(crate::services::relay::RelayVisibility::Groups)
    );

    // Anyone can sign in there: relay trust doesn't apply
    assert!(on_public_relay(state.db()));
    assert!(!trust::on_a_secured_relay(state.db()));
    let relay_trust = trust::relay_trust(state.db());
    assert!(relay_trust.public && !relay_trust.secured);

    // The Account card
    let instance_id = network::local_instance_id(state.db()).unwrap();
    match account(&state).await.unwrap() {
        PublicRelayAccount::Account {
            relay: shown,
            this_instance,
            account,
        } => {
            assert_eq!(shown, relay);
            assert_eq!(this_instance, instance_id);
            assert_eq!(account.account, "github:583231");
            assert_eq!(account.provider, RelaySignInProvider::Github);
            assert_eq!(account.display, "octocat");
            assert_eq!(account.instances, [instance_id.clone()]);
            assert_eq!(
                account.limits,
                RelayLimits {
                    groups_per_person: 10,
                    members_per_group: 50,
                    messages_per_day: Some(500),
                }
            );
            assert_eq!(account.usage.groups, 0);
            assert_eq!(account.usage.messages_today, 0);
            assert!(account.usage.resets_at > 0.0);
        }
        other => panic!("no account: {other:?}"),
    }

    // A company-relay join of the same address says to sign in instead
    assert_eq!(
        join_relay(&state, &relay, Some("code")).await.unwrap(),
        JoinOutcome::SignInNeeded
    );
    state.shutdown();
}

#[tokio::test]
async fn refusals_come_back_typed_and_change_nothing() {
    let fake = Shared::default();
    fake.lock().unwrap().denied = true;
    let relay = public_relay(&fake, Limits::public()).await;
    let state = app_with_public(&relay);

    // A provider the relay doesn't offer
    match start_sign_in(&state, RelaySignInProvider::Google)
        .await
        .unwrap()
    {
        PublicSignInStart::Refused { refusal, .. } => {
            assert_eq!(refusal, PublicRelayRefusal::ProviderNotSupported)
        }
        other => panic!("{other:?}"),
    }
    // A sign-in nobody started
    match poll_sign_in(&state, RelaySignInProvider::Github, "nope")
        .await
        .unwrap()
    {
        PublicSignInPoll::Refused { refusal, message } => {
            assert_eq!(refusal, PublicRelayRefusal::SigninNotFound);
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
    // Cancelled on GitHub's page
    let (signin_id, interval) = started(&state).await;
    let (answer, _) = polled(&state, &signin_id, interval).await;
    match answer {
        PublicSignInPoll::Refused { refusal, .. } => {
            assert_eq!(refusal, PublicRelayRefusal::SigninDenied)
        }
        other => panic!("{other:?}"),
    }
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url, None);
    assert!(instance_token(state.db()).is_none());
    assert!(!on_public_relay(state.db()));
    state.shutdown();
}

#[test]
fn every_refusal_code_has_its_kind() {
    for (code, refusal) in [
        ("not_public", PublicRelayRefusal::NotPublic),
        (
            "provider_not_supported",
            PublicRelayRefusal::ProviderNotSupported,
        ),
        ("signin_unavailable", PublicRelayRefusal::SigninUnavailable),
        ("signin_not_found", PublicRelayRefusal::SigninNotFound),
        ("signin_expired", PublicRelayRefusal::SigninExpired),
        ("signin_denied", PublicRelayRefusal::SigninDenied),
        ("account_suspended", PublicRelayRefusal::AccountSuspended),
        ("revoked", PublicRelayRefusal::Revoked),
        ("already_linked", PublicRelayRefusal::AlreadyLinked),
        ("key_mismatch", PublicRelayRefusal::KeyMismatch),
        ("device_limit", PublicRelayRefusal::DeviceLimit),
        ("signup_limit", PublicRelayRefusal::SignupLimit),
        ("signin_limit", PublicRelayRefusal::SigninLimit),
        ("invalid_request", PublicRelayRefusal::InvalidRequest),
        ("unauthorized", PublicRelayRefusal::Unauthorized),
        ("no_account", PublicRelayRefusal::NoAccount),
        ("something_new", PublicRelayRefusal::Other),
    ] {
        assert_eq!(PublicRelayRefusal::from_code(code), refusal, "{code}");
    }
}

#[tokio::test]
async fn an_unreachable_public_relay_answers_calmly_and_quickly() {
    let relay = nobody_there();
    let state = app_with_public(&relay);
    let started = std::time::Instant::now();
    assert!(matches!(
        offer(&state).await,
        PublicRelayOffer::Unreachable { relay: shown, .. } if shown == relay
    ));
    assert!(matches!(
        start_sign_in(&state, RelaySignInProvider::Github)
            .await
            .unwrap(),
        PublicSignInStart::Unreachable { .. }
    ));
    assert!(matches!(
        poll_sign_in(&state, RelaySignInProvider::Github, "x")
            .await
            .unwrap(),
        PublicSignInPoll::Unreachable { .. }
    ));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(get_relay_status(&state).await.unwrap().relay_url, None);
    state.shutdown();

    // No public relay offered at all
    let state = AppState::for_tests(vec![]);
    assert_eq!(offer(&state).await, PublicRelayOffer::None);
    assert_eq!(
        get_relay_status(&state).await.unwrap().public_relay_url,
        None
    );
    state.shutdown();
}

#[tokio::test]
async fn past_the_daily_limit_a_group_change_says_when_it_resets() {
    let fake = Shared::default();
    let relay = public_relay(
        &fake,
        Limits {
            messages_per_day: Some(1),
            ..Limits::public()
        },
    )
    .await;
    let state = app_with_public(&relay);
    let (signin_id, interval) = started(&state).await;
    let (answer, _) = polled(&state, &signin_id, interval).await;
    assert!(
        matches!(answer, PublicSignInPoll::Linked { .. }),
        "{answer:?}"
    );
    assert!(connected(&state).await, "never connected");

    let groups = Groups::of(&state).unwrap();
    groups
        .create("Book club", "", POLICY_TEMPLATE)
        .await
        .unwrap();
    let refused = groups
        .create("Chess club", "", POLICY_TEMPLATE)
        .await
        .unwrap_err();
    let prefix = format!("{}@", limits::DAILY_LIMIT);
    assert!(refused.starts_with(&prefix), "{refused}");
    let resets: i64 = refused[prefix.len()..]
        .split(':')
        .next()
        .and_then(|ms| ms.parse().ok())
        .unwrap();
    assert_eq!(
        resets,
        limits::next_utc_midnight(chrono::Utc::now().timestamp_millis())
    );

    // The account counts it
    match account(&state).await.unwrap() {
        PublicRelayAccount::Account { account, .. } => {
            assert_eq!(account.usage.messages_today, 1);
            assert_eq!(account.usage.groups, 1);
            assert_eq!(account.limits.messages_per_day, Some(1));
        }
        other => panic!("{other:?}"),
    }
    state.shutdown();
}
