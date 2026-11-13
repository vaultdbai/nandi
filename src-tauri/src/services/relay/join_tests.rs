//! Joining a relay: against relays on 127.0.0.1:0 (the `nandi-relay`
//! dev-dependency), plus stand-ins for a relay on another protocol version.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use nandi_relay::{issue_instance_token, serve_with, RelayServerConfig};

use super::*;
use crate::services::relay::{get_relay_status, set_relay_url, BuiltInRelay, RelayUrlSource};
use crate::utils::nandi_config;

const SECRET: &[u8] = b"join-secret";
const CODE: &str = "acme-2026";

fn local() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// A company relay with a join code, keeping its users in `data`
async fn company_relay(data: &Path) -> String {
    let config = RelayServerConfig::new(SECRET.to_vec())
        .with_join_code(CODE.into())
        .with_data_dir(data);
    let addr = serve_with(local(), config).await.unwrap();
    format!("ws://{addr}/relay")
}

/// Who enrolled at the relay keeping its users in `data`
fn enrolled(data: &Path) -> Vec<String> {
    nandi_relay::Store::open(data)
        .unwrap()
        .members()
        .unwrap()
        .into_iter()
        .map(|member| member.instance_id)
        .collect()
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

fn instance_id(state: &AppState) -> String {
    network::local_instance_id(state.db()).unwrap()
}

#[tokio::test]
async fn a_fresh_install_with_a_build_relay_enrolls_only_when_it_joins() {
    let data = tempfile::tempdir().unwrap();
    let relay = company_relay(data.path()).await;
    // Booting runs `relay::init`, as a launch does
    let state = AppState::for_tests_with_relay(vec![], BuiltInRelay::new(Some(&relay), Some(CODE)));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(enrolled(data.path()).is_empty(), "enrolled before joining");
    let status = get_relay_status(&state).await.unwrap();
    assert!(!status.connected);
    assert_eq!(status.relay_url, None);
    assert_eq!(status.built_in_url.as_deref(), Some(relay.as_str()));

    // "Join the Nandi network": the build's own code, no typing
    let joined = join_relay(&state, &relay, None).await.unwrap();
    assert_eq!(
        joined,
        JoinOutcome::Joined {
            relay: relay.clone()
        }
    );
    assert_eq!(enrolled(data.path()), [instance_id(&state)]);
    assert!(connected(&state).await, "never connected");
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url_source, Some(RelayUrlSource::BuiltIn));
    // The token stays out of config.json
    assert_eq!(
        nandi_config::load_from(state.nandi_dir())
            .unwrap()
            .relay_url,
        Some(relay.clone())
    );
    // The build's code is never passed on in invites
    let invite = relay_invite(&state).unwrap().unwrap();
    assert_eq!(invite.url, relay);
    assert_eq!(invite.join_code, None);
    state.shutdown();
}

#[tokio::test]
async fn joining_a_company_relay_with_a_typed_code() {
    let data = tempfile::tempdir().unwrap();
    let relay = company_relay(data.path()).await;
    let state = AppState::for_tests(vec![]);

    // A secured relay needs a code
    assert_eq!(
        join_relay(&state, &relay, None).await.unwrap(),
        JoinOutcome::CodeNeeded
    );
    assert_eq!(
        join_relay(&state, &relay, Some("  ")).await.unwrap(),
        JoinOutcome::CodeNeeded
    );
    // A wrong one changes nothing
    assert_eq!(
        join_relay(&state, &relay, Some("guess")).await.unwrap(),
        JoinOutcome::WrongCode
    );
    assert_eq!(get_relay_status(&state).await.unwrap().relay_url, None);
    assert!(enrolled(data.path()).is_empty());

    let joined = join_relay(&state, &relay, Some(&format!(" {CODE} ")))
        .await
        .unwrap();
    assert!(matches!(joined, JoinOutcome::Joined { .. }), "{joined:?}");
    assert!(connected(&state).await, "never connected");
    // The UI never gets the token
    assert_eq!(
        get_relay_status(&state).await.unwrap().relay_url,
        Some(relay.clone())
    );
    assert_eq!(
        read_token(&state),
        Some(issue_instance_token(SECRET, &instance_id(&state)))
    );
    // The invite passes on the address and the code, never the token
    let invite = relay_invite(&state).unwrap().unwrap();
    assert_eq!(
        invite,
        RelayInvite {
            url: relay.clone(),
            join_code: Some(CODE.to_string())
        }
    );

    // Disconnect, then join again: the same seat, no second enrollment
    set_relay_url(&state, "").await.unwrap();
    assert_eq!(relay_invite(&state).unwrap(), None);
    let again = join_relay(&state, &relay, None).await.unwrap();
    assert!(matches!(again, JoinOutcome::Joined { .. }), "{again:?}");
    assert!(connected(&state).await, "never reconnected");
    assert_eq!(enrolled(data.path()).len(), 1);
    state.shutdown();
}

fn read_token(state: &AppState) -> Option<String> {
    super::super::saved_token_for(
        state,
        &nandi_config::load_from(state.nandi_dir())
            .unwrap()
            .relay_url
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_full_relay_says_why_in_its_own_words() {
    let data = tempfile::tempdir().unwrap();
    {
        let store = std::sync::Arc::new(nandi_relay::Store::open(data.path()).unwrap());
        let mut roster =
            nandi_relay::roster::Roster::load(nandi_relay::SeatPolicy::free(), store).unwrap();
        let today = chrono::Utc::now().date_naive();
        for n in 0..20 {
            roster.admit(&format!("user-{n}"), today).unwrap();
        }
    }
    let relay = company_relay(data.path()).await;
    let state = AppState::for_tests(vec![]);
    let outcome = join_relay(&state, &relay, Some(CODE)).await.unwrap();
    let JoinOutcome::RelayFull { message } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(message.contains("free limit of 20 users"), "{message}");
    assert_eq!(get_relay_status(&state).await.unwrap().relay_url, None);
    state.shutdown();
}

#[tokio::test]
async fn an_open_relay_joins_without_a_code() {
    let addr = serve_with(local(), RelayServerConfig::open())
        .await
        .unwrap();
    let relay = format!("ws://{addr}/relay");
    let state = AppState::for_tests(vec![]);
    let joined = join_relay(&state, &relay, Some("ignored")).await.unwrap();
    assert_eq!(
        joined,
        JoinOutcome::Joined {
            relay: relay.clone()
        }
    );
    assert!(connected(&state).await, "never connected");
    // No code to pass on
    assert_eq!(relay_invite(&state).unwrap().unwrap().join_code, None);
    state.shutdown();
}

#[tokio::test]
async fn an_operator_issued_token_url_joins_as_it_is() {
    let data = tempfile::tempdir().unwrap();
    let relay = company_relay(data.path()).await;
    let state = AppState::for_tests(vec![]);
    let token = issue_instance_token(SECRET, &instance_id(&state));
    let joined = join_relay(&state, &format!("{relay}?token={token}"), None)
        .await
        .unwrap();
    assert_eq!(joined, JoinOutcome::Joined { relay });
    assert!(connected(&state).await, "never connected");
    state.shutdown();
}

#[tokio::test]
async fn bad_addresses_and_unreachable_relays_change_nothing() {
    let state = AppState::for_tests(vec![]);
    assert_eq!(
        join_relay(&state, "https://relay.example", Some(CODE))
            .await
            .unwrap(),
        JoinOutcome::InvalidUrl
    );
    let port = std::net::TcpListener::bind(local())
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let outcome = join_relay(&state, &format!("ws://127.0.0.1:{port}/relay"), Some(CODE))
        .await
        .unwrap();
    assert!(
        matches!(outcome, JoinOutcome::Unreachable { .. }),
        "{outcome:?}"
    );
    assert_eq!(get_relay_status(&state).await.unwrap().relay_url, None);
    state.shutdown();
}

/// A stand-in that answers `/health` with `body`
async fn health_only(body: serde_json::Value) -> String {
    use axum::routing::get;
    let app = axum::Router::new().route("/health", get(move || async move { axum::Json(body) }));
    let listener = tokio::net::TcpListener::bind(local()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("ws://{addr}/relay")
}

#[tokio::test]
async fn a_relay_on_another_protocol_version_is_not_joined() {
    let relay = health_only(serde_json::json!({
        "status": "ok", "instances_connected": 0, "uptime_seconds": 1,
        "version": "9.9.9", "auth": "token", "protocol_version": PROTOCOL_VERSION + 1
    }))
    .await;
    let state = AppState::for_tests(vec![]);
    assert_eq!(
        join_relay(&state, &relay, Some(CODE)).await.unwrap(),
        JoinOutcome::VersionMismatch {
            relay: PROTOCOL_VERSION + 1,
            app: PROTOCOL_VERSION
        }
    );
    // Something else at that address
    let other = health_only(serde_json::json!({"hello": "world"})).await;
    let outcome = join_relay(&state, &other, Some(CODE)).await.unwrap();
    let JoinOutcome::Unreachable { detail } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(detail.contains("isn't a Nandi relay"), "{detail}");
    assert_eq!(get_relay_status(&state).await.unwrap().relay_url, None);
    state.shutdown();
}

#[test]
fn refusals_map_to_what_the_user_sees() {
    let refused = |status: u16| {
        JoinOutcome::from(EnrollError::Refused {
            status,
            message: "why".into(),
        })
    };
    assert_eq!(refused(401), JoinOutcome::WrongCode);
    assert_eq!(
        refused(402),
        JoinOutcome::RelayFull {
            message: "why".into()
        }
    );
    assert_eq!(
        refused(403),
        JoinOutcome::Removed {
            message: "why".into()
        }
    );
    assert_eq!(refused(404), JoinOutcome::NoEnrollment);
    assert_eq!(
        refused(409),
        JoinOutcome::AlreadyEnrolled {
            message: "why".into()
        }
    );
    assert_eq!(
        refused(400),
        JoinOutcome::Refused {
            message: "why".into()
        }
    );
    assert_eq!(
        JoinOutcome::from(EnrollError::Unreachable("down".into())),
        JoinOutcome::Unreachable {
            detail: "down".into()
        }
    );
}

#[test]
fn outcomes_cross_the_bridge_tagged_by_kind() {
    assert_eq!(
        serde_json::to_value(JoinOutcome::RelayFull {
            message: "m".into()
        })
        .unwrap(),
        serde_json::json!({"kind": "relayFull", "message": "m"})
    );
    assert_eq!(
        serde_json::to_value(RelayInvite {
            url: "wss://r/relay".into(),
            join_code: None
        })
        .unwrap(),
        serde_json::json!({"url": "wss://r/relay", "joinCode": null})
    );
}
