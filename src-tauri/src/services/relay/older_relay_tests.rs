//! The client against relay N - 1: a real relay made to speak the oldest
//! version this Nandi talks to (`RelayServerConfig::with_protocol_version`,
//! the relay's `testing` feature). The client reads it from `/health` and
//! speaks it; every feature both versions have still works, and the ones
//! that relay is too old for are hidden calmly (`missing_features`,
//! [`RelayNeedsUpdate`]) and never sent.
//!
//! While [`MIN_SUPPORTED_PROTOCOL`] equals [`PROTOCOL_VERSION`] relay N - 1
//! speaks our own version and nothing is hidden; from the next protocol
//! bump on, the same test covers what the older relay lacks.

use std::net::SocketAddr;

use nandi_protocol::{
    DeviceOp, Envelope, GroupOp, SealPurpose, SealedFor, MIN_SUPPORTED_PROTOCOL, PROTOCOL_VERSION,
};
use nandi_relay::{serve_with, RelayServerConfig};
use serde_json::json;

use super::client::WsRelayClient;
use super::features::{RelayFeature, RelayNeedsUpdate};
use super::protocol::{RequestKind, RoutingQuery};
use super::tests::{card, eventually, local, registered, SECRET};
use super::RelayClient;
use crate::services::a2a::Task;

/// Relay N - 1: the oldest version this Nandi talks to
const RELAY_BEFORE: u32 = MIN_SUPPORTED_PROTOCOL;

/// A relay speaking protocol `version`
async fn relay_at(version: u32) -> SocketAddr {
    let config = RelayServerConfig::new(SECRET.to_vec()).with_protocol_version(version);
    serve_with(local(), config).await.unwrap()
}

/// A request from `req` to `prov`'s agent `papers`
fn request() -> Task {
    Task {
        id: "t1".into(),
        requester: card("req", "writer", "write"),
        target: "prov/papers".into(),
        capability: "find-papers".into(),
        input: json!({"q": "relays"}),
        created_at: 1.0,
        deadline: None,
    }
}

/// Three members of one relay: `req` asks `prov`, who can hand on to `carl`
struct Team {
    requester: WsRelayClient,
    provider: WsRelayClient,
    _backup: WsRelayClient,
}

impl Team {
    async fn on(addr: SocketAddr) -> Self {
        let papers = |member: &str| vec![card(member, "papers", "find-papers")];
        Self {
            provider: registered(addr, "prov", papers("prov")).await,
            _backup: registered(addr, "carl", papers("carl")).await,
            requester: registered(addr, "req", vec![card("req", "writer", "write")]).await,
        }
    }

    /// Uses `feature` once, as the app does: `Ok` when the relay read the
    /// message and did it. Run in [`RelayFeature::ALL`] order: the request
    /// comes before its reassignment.
    async fn use_feature(&self, feature: RelayFeature) -> Result<(), String> {
        let client = &self.requester;
        match feature {
            RelayFeature::Directory => client.query_directory().await.map(drop),
            RelayFeature::PeopleSearch => client.find_people("papers", 5).await.map(drop),
            RelayFeature::Routing => {
                let query = RoutingQuery::anyone(RequestKind::Delegate, "find-papers");
                client.route(&query).await.map(drop)
            }
            RelayFeature::Requests => client.delegate_task(&request(), &[]).await.map(drop),
            RelayFeature::Reassignment => {
                self.provider
                    .reassign("t1", "req", "carl/papers", "prov", None)
                    .await
            }
            RelayFeature::Handover => {
                let due_at = chrono::Utc::now().timestamp_millis() + 3_600_000;
                client
                    .duty_watch("standup/2026-11-16/09:00", "prov", due_at, None)
                    .await
            }
            RelayFeature::Groups => client.group_request(GroupOp::List).await.map(drop),
            RelayFeature::OwnDevices => client.device_request(DeviceOp::List).await.map(drop),
        }
    }
}

/// A sealed envelope for a device, whatever its content
fn own_envelope() -> Envelope {
    let keys = nandi_seal::DeviceKeys::generate().unwrap();
    let device = "device.0123456789abcdef0123456789abcdef";
    let sealed_for = SealedFor {
        task_id: device.into(),
        seq: 1,
        from: "req".into(),
        to: device.into(),
        purpose: SealPurpose::OwnDevice,
        capability: String::new(),
    };
    nandi_seal::seal(&keys, keys.public_keys(), sealed_for, b"{}").unwrap()
}

#[tokio::test]
async fn a_relay_before_own_devices_is_never_sent_a_device_message() {
    let addr = relay_at(RELAY_BEFORE).await;
    let client = registered(addr, "req", vec![card("req", "writer", "write")]).await;
    let status = client.status();
    assert_eq!(
        status.missing_features.contains(&RelayFeature::OwnDevices),
        !RelayFeature::OwnDevices.available_at(RELAY_BEFORE)
    );
    if RelayFeature::OwnDevices.available_at(RELAY_BEFORE) {
        return;
    }
    for error in [
        client.device_request(DeviceOp::Pair).await.unwrap_err(),
        client.send_own_envelope(own_envelope()).await.unwrap_err(),
    ] {
        assert!(RelayNeedsUpdate::is(&error), "{error}");
    }
    assert!(client.status().connected);
}

#[tokio::test]
async fn with_relay_n_minus_1_shared_features_work_and_newer_ones_are_hidden() {
    let addr = relay_at(RELAY_BEFORE).await;
    let team = Team::on(addr).await;

    let status = team.requester.status();
    assert!(status.connected);
    assert_eq!(status.protocol_version, Some(RELAY_BEFORE));
    assert_eq!(
        status.missing_features,
        RelayFeature::missing_at(RELAY_BEFORE)
    );

    for feature in RelayFeature::ALL {
        let used = team.use_feature(feature).await;
        if feature.available_at(RELAY_BEFORE) {
            assert_eq!(used, Ok(()), "{feature:?} on relay {RELAY_BEFORE}");
        } else {
            let error = used.unwrap_err();
            assert!(RelayNeedsUpdate::is(&error), "{feature:?}: {error}");
        }
    }
    // A feature the relay lacks never costs the connection
    assert!(team.requester.status().connected);
}

#[tokio::test]
async fn with_a_relay_of_our_version_nothing_is_hidden() {
    let addr = relay_at(PROTOCOL_VERSION).await;
    let client = registered(addr, "req", vec![card("req", "writer", "write")]).await;
    let status = client.status();
    assert_eq!(status.protocol_version, Some(PROTOCOL_VERSION));
    assert!(status.missing_features.is_empty());
}

#[tokio::test]
async fn a_relay_older_than_the_window_is_refused_and_says_it_needs_the_update() {
    use super::version::UpdateNeeded;
    let addr = relay_at(RELAY_BEFORE - 1).await;
    let client = WsRelayClient::new(None, "req");
    let token = nandi_relay::issue_instance_token(SECRET, "req");
    let _ = client
        .connect(&format!("ws://{addr}/relay?token={token}"), "req")
        .await;
    let _ = client.register(Vec::new()).await;
    // The side to update comes from `/health` after the refusal
    assert!(eventually(async || client.status().update_needed.is_some()).await);
    let status = client.status();
    assert!(!status.connected && status.version_mismatch);
    assert_eq!(status.update_needed, Some(UpdateNeeded::Relay));
    assert!(status.missing_features.is_empty());
}
