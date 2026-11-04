//! Device keys against a real relay: registration carries them and the
//! relay pins them, other keys read as `key_mismatch` until the operator
//! rekeys, and a member whose key changed is blocked until accepted.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use nandi_protocol::PublicKeys;
use nandi_relay::{issue_instance_token, serve_with, RelayServerConfig, Store};

use super::client::{ClientOptions, WsRelayClient};
use super::protocol::PeerMessage;
use super::sealing::PeerKeyBook;
use super::{RelayClient, RelayConnectionState};
use crate::db::Db;
use crate::services::network::directory_tests::gateway;
use crate::services::network::peer_keys::{self, PEER_KEY_CHANGED};
use crate::services::network::NetworkService;

const SECRET: &[u8] = b"keys-secret";

fn company(data: &Path) -> RelayServerConfig {
    RelayServerConfig::new(SECRET.to_vec()).with_data_dir(data)
}

/// A new device's secret keys, as the keyring keeps them
fn new_keys() -> String {
    nandi_seal::DeviceKeys::generate()
        .unwrap()
        .export_secret()
        .to_string()
}

fn public_of(secret: &str) -> PublicKeys {
    nandi_seal::DeviceKeys::import_secret(secret)
        .unwrap()
        .public_keys()
        .clone()
}

/// A client for `instance_id` with the keys `secret`, pinning others' in
/// `db` (refusing to send where a key changed)
fn client(instance_id: &str, secret: String, db: Option<Db>) -> WsRelayClient {
    WsRelayClient::with_options(
        None,
        instance_id,
        ClientOptions {
            device_keys: Some(Arc::new(move || {
                nandi_seal::DeviceKeys::import_secret(&secret).map_err(|e| e.to_string())
            })),
            peer_keys: db
                .map(|db| -> Arc<dyn PeerKeyBook> { Arc::new(peer_keys::PinnedKeys::new(db)) }),
            ..ClientOptions::default()
        },
    )
}

/// Connects and registers `client`; the registration's result
async fn join(client: &WsRelayClient, addr: SocketAddr, instance_id: &str) -> Result<(), String> {
    let token = issue_instance_token(SECRET, instance_id);
    client
        .connect(&format!("ws://{addr}/relay?token={token}"), instance_id)
        .await?;
    client.register(Vec::new()).await
}

async fn rekey(addr: SocketAddr, instance_id: &str) {
    let status = reqwest::Client::new()
        .post(format!("http://{addr}/relay/rekey"))
        .bearer_auth(std::str::from_utf8(SECRET).unwrap())
        .json(&serde_json::json!({ "instanceId": instance_id }))
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "{status}");
}

#[tokio::test]
async fn registration_carries_the_keys_and_the_relay_pins_them() {
    let tmp = tempfile::tempdir().unwrap();
    let addr = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let keys = new_keys();
    let ada = client("ada", keys.clone(), None);
    join(&ada, addr, "ada").await.unwrap();
    assert!(ada.status().connected && !ada.status().key_mismatch);
    assert_eq!(
        Store::open(tmp.path()).unwrap().pinned_keys("ada").unwrap(),
        Some(public_of(&keys))
    );
}

#[tokio::test]
async fn other_keys_read_as_key_mismatch_until_the_operator_rekeys() {
    let tmp = tempfile::tempdir().unwrap();
    let addr = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let first = client("ada", new_keys(), None);
    join(&first, addr, "ada").await.unwrap();
    first.connect("", "ada").await.unwrap();

    // Nandi reinstalled: new keys, same ID
    let reinstalled = client("ada", new_keys(), None);
    let refused = join(&reinstalled, addr, "ada").await.unwrap_err();
    assert!(refused.contains("rekey"), "{refused}");
    let status = reinstalled.status();
    assert!(status.key_mismatch && !status.version_mismatch);
    assert_eq!(status.state, RelayConnectionState::Error);
    assert!(!status.connected);
    reinstalled.connect("", "ada").await.unwrap();

    // The operator rekeys: the reinstalled device's keys are pinned now
    rekey(addr, "ada").await;
    join(&reinstalled, addr, "ada").await.unwrap();
    let status = reinstalled.status();
    assert!(status.connected && !status.key_mismatch);
}

/// Refreshes Ada's directory until Bob's key status passes `done` (the
/// relay stores a gateway card just after it confirms the registration)
async fn refreshed_until(
    network: &NetworkService,
    db: &Db,
    ada: &WsRelayClient,
    done: impl Fn(Option<&peer_keys::PeerKeyStatus>) -> bool,
) -> Option<peer_keys::PeerKeyStatus> {
    for _ in 0..250 {
        network.invalidate();
        network.refresh_with(db, Some(ada), "ada").await;
        let status = peer_keys::status(db, "bob").unwrap();
        if done(status.as_ref()) {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("Bob's key never reached the expected state");
}

#[tokio::test]
async fn a_member_whose_key_changed_is_blocked_until_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    let addr = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let db = Db::in_memory();
    let ada = client("ada", new_keys(), Some(db.clone()));
    join(&ada, addr, "ada").await.unwrap();
    let bob = client("bob", new_keys(), None);
    join(&bob, addr, "bob").await.unwrap();
    bob.register_gateway(gateway("bob", "Bob", "bob"))
        .await
        .unwrap();

    let network = NetworkService::new(&db);
    let ping = || PeerMessage::StatusQuery {
        task_id: "t1".into(),
    };
    // First sight: pinned, and sending works
    let pinned = refreshed_until(&network, &db, &ada, |status| status.is_some())
        .await
        .unwrap();
    assert!(!pinned.changed());
    ada.send_peer("bob".into(), ping()).await.unwrap();

    // Bob's device is replaced and the operator rekeys it
    bob.connect("", "bob").await.unwrap();
    rekey(addr, "bob").await;
    let replaced = client("bob", new_keys(), None);
    join(&replaced, addr, "bob").await.unwrap();
    replaced
        .register_gateway(gateway("bob", "Bob", "bob"))
        .await
        .unwrap();
    let changed = refreshed_until(&network, &db, &ada, |status| {
        status.is_some_and(peer_keys::PeerKeyStatus::changed)
    })
    .await
    .unwrap();
    assert!(changed.changed());
    assert_eq!(changed.fingerprint, pinned.fingerprint);
    let refused = ada.send_peer("bob".into(), ping()).await.unwrap_err();
    assert!(refused.starts_with(PEER_KEY_CHANGED), "{refused}");

    // The user checks it and accepts
    let accepted = peer_keys::accept(&db, "bob").unwrap();
    assert_eq!(Some(accepted.fingerprint), changed.changed_fingerprint);
    ada.send_peer("bob".into(), ping()).await.unwrap();
}
