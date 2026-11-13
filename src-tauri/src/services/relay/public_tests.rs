//! Joining the public relay: the real `nandi-relay` (dev-dependency) in
//! public mode on 127.0.0.1:0, trusting its fake identity provider
//! (`nandi_relay::testing::FakeIdp`), with a Gmail mailbox "connected"
//! (its ID token in the keychain), plus a relay address nothing answers on.

use std::net::SocketAddr;
use std::time::Duration;

use nandi_relay::accounts::PublicConfig;
use nandi_relay::testing::FakeIdp;
use nandi_relay::{serve_with, Limits, RelayServerConfig};
use serde_json::json;

use super::*;
use crate::services::domain::group::POLICY_TEMPLATE;
use crate::services::email::settings::EmailAccount;
use crate::services::email::{EmailSettings, MailProvider};
use crate::services::groups::Groups;
use crate::services::identity::VerifiedIdentity;
use crate::services::relay::join::{join_relay, JoinOutcome};
use crate::services::relay::{get_relay_status, has_instance_token, limits, BuiltInRelay};
use crate::services::team::trust;
use crate::utils::nandi_config;

const SECRET: &[u8] = b"public-secret";
const EMAIL: &str = "ada@example.com";

fn local() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// A public relay trusting `idp`, with `limits`
async fn public_relay(idp: &FakeIdp, limits: Limits) -> String {
    let mut config = PublicConfig::new(idp.identity_config());
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

/// As if Gmail `email` were connected: the account, its sign-in (with
/// `id_token`) in the keychain, and the locked identity
fn connect_gmail(state: &AppState, email: &str, id_token: &str) {
    let account = EmailAccount {
        provider: MailProvider::Gmail,
        email: email.to_string(),
        name: Some("Ada".into()),
        profile: Default::default(),
        paused: false,
    };
    let identity = VerifiedIdentity {
        provider: MailProvider::Gmail,
        email: email.to_string(),
        name: Some("Ada".into()),
        locked_at: 1.0,
    };
    nandi_config::update_in(state.nandi_dir(), move |config| {
        config
            .email
            .get_or_insert_with(EmailSettings::default)
            .accounts
            .push(account);
        config.identity = Some(identity);
    })
    .unwrap();
    let token = json!({ "access_token": "ya29", "refresh_token": "r", "id_token": id_token });
    state
        .secrets()
        .set(&MailProvider::Gmail.token_entry(), &token.to_string())
        .unwrap();
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

#[tokio::test]
async fn joining_with_the_connected_email_keeps_the_token_and_connects() {
    let idp = FakeIdp::start().await;
    let relay = public_relay(&idp, Limits::public()).await;
    let state = app_with_public(&relay);

    // No email yet: the offer and a join both say to connect one
    assert_eq!(
        offer(&state).await,
        PublicRelayOffer::NeedsEmail {
            relay: relay.clone()
        }
    );
    match join(&state).await.unwrap() {
        PublicRelayJoin::NeedsEmail { message } => {
            assert!(message.contains("Settings › Email"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert!(!has_instance_token(state.db()));

    connect_gmail(&state, EMAIL, &idp.google("1099", EMAIL, "Ada Lovelace"));
    assert_eq!(
        offer(&state).await,
        PublicRelayOffer::Ready {
            relay: relay.clone(),
            email: EMAIL.into(),
        }
    );
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.public_relay_url.as_deref(), Some(relay.as_str()));
    assert_eq!(status.relay_url, None);
    assert_eq!(
        account(&state).await.unwrap(),
        PublicRelayAccount::NotPublic
    );

    assert_eq!(
        join(&state).await.unwrap(),
        PublicRelayJoin::Joined {
            relay: relay.clone(),
            account: "google:1099".into(),
            email: EMAIL.into(),
            name: "Ada Lovelace".into(),
        }
    );

    // Kept exactly like an enrolled token: in the keyring, never config.json
    assert!(has_instance_token(state.db()));
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

    // Anyone with an email can join there: relay trust doesn't apply
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
            assert_eq!(account.account, "google:1099");
            assert_eq!(account.provider, "google");
            assert_eq!(account.email, EMAIL);
            assert_eq!(account.instances, std::slice::from_ref(&instance_id));
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

    // A company-relay join of the same address says to join it this way
    assert_eq!(
        join_relay(&state, &relay, Some("code")).await.unwrap(),
        JoinOutcome::PublicRelay
    );
    state.shutdown();
}

#[tokio::test]
async fn refusals_come_back_typed_and_change_nothing() {
    let idp = FakeIdp::start().await;
    let relay = public_relay(&idp, Limits::public()).await;
    let state = app_with_public(&relay);
    // A token for another app (not the relay's audience)
    let foreign = idp.sign(json!({
        "iss": "https://accounts.google.com", "aud": "another-app", "sub": "1",
        "email": EMAIL, "email_verified": true, "exp": FakeIdp::in_an_hour(),
    }));
    connect_gmail(&state, EMAIL, &foreign);
    match join(&state).await.unwrap() {
        PublicRelayJoin::Refused { refusal, message } => {
            assert_eq!(refusal, PublicRelayRefusal::IdTokenInvalid);
            assert!(!message.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url, None);
    assert!(!has_instance_token(state.db()));
    assert!(!on_public_relay(state.db()));
    state.shutdown();
}

#[tokio::test]
async fn a_disconnected_mailbox_needs_connecting_again() {
    let idp = FakeIdp::start().await;
    let relay = public_relay(&idp, Limits::public()).await;
    let state = app_with_public(&relay);
    connect_gmail(&state, EMAIL, &idp.google("1099", EMAIL, ""));
    // The mailbox is gone, the identity stays locked
    nandi_config::update_in(state.nandi_dir(), |config| {
        config.email = None;
    })
    .unwrap();
    match join(&state).await.unwrap() {
        PublicRelayJoin::NeedsEmail { message } => {
            assert!(message.contains(EMAIL), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert!(!has_instance_token(state.db()));
    state.shutdown();
}

#[test]
fn every_refusal_code_has_its_kind() {
    for (code, refusal) in [
        ("not_public", PublicRelayRefusal::NotPublic),
        ("id_token_invalid", PublicRelayRefusal::IdTokenInvalid),
        ("id_token_expired", PublicRelayRefusal::IdTokenExpired),
        ("email_unverified", PublicRelayRefusal::EmailUnverified),
        (
            "identity_unavailable",
            PublicRelayRefusal::IdentityUnavailable,
        ),
        ("account_suspended", PublicRelayRefusal::AccountSuspended),
        ("revoked", PublicRelayRefusal::Revoked),
        ("already_linked", PublicRelayRefusal::AlreadyLinked),
        ("key_mismatch", PublicRelayRefusal::KeyMismatch),
        ("device_limit", PublicRelayRefusal::DeviceLimit),
        ("machine_limit", PublicRelayRefusal::MachineLimit),
        ("signup_limit", PublicRelayRefusal::SignupLimit),
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
    connect_gmail(&state, EMAIL, "unused");
    let started = std::time::Instant::now();
    assert!(matches!(
        offer(&state).await,
        PublicRelayOffer::Unreachable { relay: shown, .. } if shown == relay
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
    let idp = FakeIdp::start().await;
    let relay = public_relay(
        &idp,
        Limits {
            messages_per_day: Some(1),
            ..Limits::public()
        },
    )
    .await;
    let state = app_with_public(&relay);
    connect_gmail(&state, EMAIL, &idp.google("1099", EMAIL, ""));
    let joined = join(&state).await.unwrap();
    assert!(
        matches!(joined, PublicRelayJoin::Joined { .. }),
        "{joined:?}"
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
