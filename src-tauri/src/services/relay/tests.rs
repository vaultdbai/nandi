//! Relay client integration tests: the app's clients against a relay on
//! 127.0.0.1:0 (the `nandi-relay` dev-dependency). The relay's own
//! behavior is tested in that repository.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::client::{ClientOptions, WsRelayClient};
use super::protocol::{RelayMessage, TaskCompleted};
use super::RelayClient;
use super::*;
use crate::services::a2a::{DelegationDirection, Task, TaskResult, TaskStatus};
use crate::services::domain::agent_card::OwnerRef;
use crate::services::domain::capability::ServesCapability;
use nandi_relay::{issue_instance_token, serve, serve_open, serve_with, RelayServerConfig};

const SECRET: &[u8] = b"test-secret";

fn local() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn url(addr: SocketAddr, instance_id: &str) -> String {
    let token = issue_instance_token(SECRET, instance_id);
    format!("ws://{addr}/relay?token={token}")
}

fn card(instance_id: &str, agent_id: &str, capability: &str) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(instance_id, agent_id),
        name: agent_id.to_string(),
        description: String::new(),
        instance_id: instance_id.to_string(),
        endpoint: String::new(),
        capabilities: vec![ServesCapability {
            name: capability.to_string(),
            description: String::new(),
            input_schema: None,
            output_schema: None,
        }],
        accepts_delegation: true,
        max_concurrent: 1,
        current_load: 0,
        owner: OwnerRef::default(),
        signature: None,
        issued_at: 0.0,
    }
}

async fn registered(addr: SocketAddr, instance_id: &str, cards: Vec<AgentCard>) -> WsRelayClient {
    let client = WsRelayClient::new(None, instance_id);
    client
        .connect(&url(addr, instance_id), instance_id)
        .await
        .unwrap();
    client.register(cards).await.unwrap();
    client
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

#[tokio::test]
async fn the_relay_plans_who_gets_a_request() {
    use super::protocol::{RequestKind, RouteReason, RoutingQuery};
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let mut papers = card("prov", "papers", "find-papers");
    papers.owner.display_name = "Prov".into();
    let _provider = registered(addr, "prov", vec![papers]).await;
    let requester = registered(addr, "req", vec![card("req", "writer", "find-papers")]).await;

    let query = RoutingQuery::anyone(RequestKind::Delegate, "find-papers");
    let plan = requester.route(&query).await.unwrap();
    // Never the requester; the reasons are codes, not sentences
    assert_eq!(plan.candidates.len(), 1);
    assert_eq!(plan.candidates[0].target, "prov/papers");
    assert_eq!(
        plan.candidates[0].reasons[0],
        RouteReason::Free {
            name: "Prov".into()
        }
    );
    assert!(
        plan.candidates[0].public_keys.is_some(),
        "the provider's pinned keys"
    );

    // Not connected: RelayUnavailable, at once
    let offline = WsRelayClient::new(None, "lone");
    let error = offline.route(&query).await.unwrap_err();
    assert!(super::unavailable::RelayUnavailable::is(&error), "{error}");
}

#[tokio::test]
async fn one_client_finds_the_others_card() {
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let provider = registered(addr, "prov", vec![card("prov", "papers", "find-papers")]).await;
    let requester = registered(addr, "req", vec![card("req", "writer", "write")]).await;
    assert!(provider.status().connected && requester.status().connected);

    let found = requester.query_capability("find-papers").await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "prov/papers");
    assert!(requester
        .query_capability("nothing")
        .await
        .unwrap()
        .is_empty());
    // `*` is everything
    assert_eq!(requester.query_capability("*").await.unwrap().len(), 2);

    // Heartbeat load shows up in the cards
    provider
        .heartbeat(std::collections::HashMap::from([("papers".to_string(), 3)]))
        .await
        .unwrap();
    assert!(
        eventually(async || {
            requester.query_capability("find-papers").await.unwrap()[0].current_load == 3
        })
        .await
    );

    // Presence: the requester learns about both cards
    assert!(eventually(async || requester.status().agents_online == 2).await);
}

#[tokio::test]
async fn registration_needs_a_valid_token() {
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let client = WsRelayClient::new(None, "mallory");
    // A token for someone else
    let token = issue_instance_token(SECRET, "alice");
    client
        .connect(&format!("ws://{addr}/relay?token={token}"), "mallory")
        .await
        .unwrap();
    let refused = client.register(vec![card("mallory", "a", "x")]).await;
    assert!(refused.unwrap_err().contains("refused"));

    let unsigned = WsRelayClient::new(None, "eve");
    unsigned
        .connect(&format!("ws://{addr}/relay"), "eve")
        .await
        .unwrap();
    assert!(unsigned.register(vec![]).await.is_err());
    // Unregistered connections can't query
    assert!(unsigned.query_capability("x").await.is_err());
}

#[tokio::test]
async fn cards_of_other_instances_are_dropped() {
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let client = registered(
        addr,
        "honest",
        vec![card("honest", "a", "x"), card("victim", "b", "x")],
    )
    .await;
    let found = client.query_capability("x").await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "honest/a");
}

fn completed(task: &Task) -> TaskResult {
    TaskResult {
        task_id: task.id.clone(),
        provider_agent: task.target.clone(),
        output: json!({"echo": task.input.clone()}),
        status: TaskStatus::Completed,
        duration_ms: 1,
        tokens_in: 0,
        tokens_out: 0,
        error: None,
        direction: DelegationDirection::Inbound,
        capability: task.capability.clone(),
        requester_agent: task.requester.id.clone(),
        provider_owner: None,
        requester_owner: None,
        input: task.input.clone(),
        started_at: task.created_at,
        ended_at: Some(task.created_at + 1.0),
    }
}

#[tokio::test]
async fn tasks_are_routed_to_the_provider_and_back() {
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let provider =
        Arc::new(registered(addr, "prov", vec![card("prov", "papers", "find-papers")]).await);
    let requester = registered(addr, "req", vec![card("req", "writer", "write")]).await;

    // The provider answers every task it is assigned
    let (task_tx, mut task_rx) = tokio::sync::mpsc::unbounded_channel::<Task>();
    provider
        .on_task_assigned(Box::new(move |assigned| {
            let _ = task_tx.send(assigned.task);
        }))
        .await;
    let answering = provider.clone();
    tokio::spawn(async move {
        while let Some(task) = task_rx.recv().await {
            answering.send_result(completed(&task)).await.unwrap();
        }
    });

    let results: Arc<Mutex<Vec<TaskCompleted>>> = Arc::default();
    let sink = results.clone();
    requester
        .on_task_completed(Box::new(move |done| sink.lock().unwrap().push(done)))
        .await;

    let task = Task {
        id: "t1".into(),
        requester: card("req", "writer", "write"),
        target: "prov/papers".into(),
        capability: "find-papers".into(),
        input: json!({"q": "relays"}),
        created_at: 1.0,
        deadline: None,
    };
    assert!(!requester.delegate_task(&task, &[]).await.unwrap().held);
    assert!(eventually(async || !results.lock().unwrap().is_empty()).await);
    let done = results.lock().unwrap()[0].clone();
    assert_eq!(done.task_id, "t1");
    assert_eq!(done.result.output, json!({"echo": {"q": "relays"}}));

    // A target the relay doesn't know is refused right away, as the
    // answer to the send only: it isn't a task result
    let failures: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = failures.clone();
    requester
        .on_task_failed(Box::new(move |failed| {
            sink.lock().unwrap().push(failed.task_id)
        }))
        .await;
    let missing = Task {
        id: "t2".into(),
        target: "gone/agent".into(),
        ..task
    };
    assert!(requester.delegate_task(&missing, &[]).await.is_err());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(failures.lock().unwrap().is_empty());
}

fn gateway_card(
    instance_id: &str,
    accepts_delegation: bool,
) -> crate::services::gateway::GatewayCard {
    crate::services::gateway::GatewayCard {
        instance_id: instance_id.to_string(),
        endpoint: String::new(),
        display_name: instance_id.to_string(),
        description: String::new(),
        capabilities: Vec::new(),
        accepts_delegation,
        max_concurrent: 3,
        current_load: 0,
        owner: OwnerRef::default(),
        federation_enabled: false,
        gateway_version: 1,
        agent_count: 0,
        body: None,
        direct_delegation: None,
        responsibilities: Vec::new(),
        availability: Default::default(),
    }
}

#[tokio::test]
async fn tasks_reach_a_gateway_only_instance() {
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    // Gateway card known before the first registration: still routable
    let provider = WsRelayClient::new(None, "prov");
    provider.connect(&url(addr, "prov"), "prov").await.unwrap();
    let _ = provider.register_gateway(gateway_card("prov", true)).await;
    provider.register(Vec::new()).await.unwrap();
    let assigned: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = assigned.clone();
    provider
        .on_task_assigned(Box::new(move |a| {
            sink.lock().unwrap().push(a.task.target);
        }))
        .await;
    let requester = registered(addr, "req", Vec::new()).await;

    let task = Task {
        id: "g1".into(),
        requester: card("req", "_user", "none"),
        target: "prov/gateway".into(),
        capability: "gateway".into(),
        input: json!({"text": "What is 2+2?"}),
        created_at: 1.0,
        deadline: None,
    };
    assert!(!requester.delegate_task(&task, &[]).await.unwrap().held);
    assert!(eventually(async || !assigned.lock().unwrap().is_empty()).await);
    assert_eq!(assigned.lock().unwrap()[0], "prov/gateway");
    // Presence counts the instance, but its route card isn't an agent
    let cards = requester.query_capability("*").await.unwrap();
    assert!(cards.iter().any(|c| c.id == "prov/gateway"));

    // Closing the gateway withdraws the route
    provider
        .register_gateway(gateway_card("prov", false))
        .await
        .unwrap();
    let refused = Task {
        id: "g2".into(),
        ..task
    };
    assert!(
        eventually(async || {
            requester
                .query_capability("gateway")
                .await
                .is_ok_and(|cards| cards.is_empty())
        })
        .await
    );
    assert!(requester.delegate_task(&refused, &[]).await.is_err());
}

#[tokio::test]
async fn silent_instances_are_evicted() {
    let config =
        RelayServerConfig::new(SECRET.to_vec()).with_eviction_timeout(Duration::from_millis(300));
    let addr = serve_with(local(), config).await.unwrap();

    // A raw instance that registers and then goes silent
    let (mut silent, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/relay"))
        .await
        .unwrap();
    let register = RelayMessage::RegisterInstance {
        instance_id: "silent".into(),
        agent_cards: vec![card("silent", "a", "x")],
        signature: issue_instance_token(SECRET, "silent"),
        capabilities: vec![],
        tools: vec![],
        protocol_version: super::protocol::PROTOCOL_VERSION,
        public_keys: None,
    };
    silent
        .send(Message::Text(register.to_json().unwrap().into()))
        .await
        .unwrap();
    let reply = silent.next().await.unwrap().unwrap();
    assert!(matches!(
        RelayMessage::from_json(reply.to_text().unwrap()).unwrap(),
        RelayMessage::Registered { .. }
    ));

    // A live instance heartbeating faster than the timeout
    let live = WsRelayClient::with_options(
        None,
        "live",
        ClientOptions {
            heartbeat_every: Duration::from_millis(50),
            ..ClientOptions::default()
        },
    );
    live.connect(&url(addr, "live"), "live").await.unwrap();
    live.register(vec![card("live", "b", "x")]).await.unwrap();
    assert_eq!(live.query_capability("x").await.unwrap().len(), 2);

    tokio::time::sleep(Duration::from_millis(700)).await;
    let found = live.query_capability("x").await.unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].id, "live/b");

    // The relay closed the silent socket
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match silent.next().await {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return true,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert_eq!(closed, Ok(true));
}

#[tokio::test]
async fn a_dropped_client_reconnects_and_re_registers() {
    // The relay replaces a connection when the same instance registers
    // again; the replaced client reconnects and registers its last cards
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let client = registered(addr, "dup", vec![card("dup", "a", "x")]).await;
    let observer = registered(addr, "obs", vec![]).await;

    let (mut impostor, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/relay"))
        .await
        .unwrap();
    let register = RelayMessage::RegisterInstance {
        instance_id: "dup".into(),
        agent_cards: vec![],
        signature: issue_instance_token(SECRET, "dup"),
        capabilities: vec![],
        tools: vec![],
        protocol_version: super::protocol::PROTOCOL_VERSION,
        public_keys: None,
    };
    impostor
        .send(Message::Text(register.to_json().unwrap().into()))
        .await
        .unwrap();
    let _ = impostor.next().await;
    assert!(observer.query_capability("x").await.unwrap().is_empty());
    drop(impostor);

    // Backoff starts at 1s
    let mut back = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if observer.query_capability("x").await.unwrap().len() == 1 {
            back = true;
            break;
        }
    }
    assert!(back, "the client did not re-register");
    let status = client.status();
    assert!(status.connected);
    assert_eq!(status.state, RelayConnectionState::Connected);
    assert_eq!(status.reason, None);
    // Re-registered and heartbeated on reconnect, not 30s later
    assert!(status.last_heartbeat_at >= status.last_connected_at);
}

// ---------------------------------------------------------------------------
// The service functions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn without_a_relay_status_is_disconnected() {
    let state = AppState::for_tests(vec![]);
    let status = get_relay_status(&state).await.unwrap();
    assert!(!status.connected);
    assert_eq!(status.relay_url, None);
    assert_eq!(
        status.instance_id,
        network::local_instance_id(state.db()).unwrap()
    );
    assert!(list_online_agents(&state, None).await.unwrap().is_empty());
    state.shutdown();
}

#[tokio::test]
async fn relay_urls_are_validated_and_cleared() {
    let state = AppState::for_tests(vec![]);
    assert!(set_relay_url(&state, "http://nope").await.is_err());
    set_relay_url(&state, "").await.unwrap();
    assert_eq!(
        resolve_url_in(None, state.nandi_dir(), state.db()).unwrap(),
        None
    );
    state.shutdown();
}

#[tokio::test]
async fn offline_lists_come_from_the_last_known_agents() {
    let state = AppState::for_tests(vec![]);
    write_state(
        state.db(),
        ONLINE_AGENTS_KEY,
        &[card("a", "one", "x"), card("b", "two", "y")],
    )
    .unwrap();
    assert_eq!(list_online_agents(&state, None).await.unwrap().len(), 2);
    let only_y = list_online_agents(&state, Some("y")).await.unwrap();
    assert_eq!(only_y.len(), 1);
    assert_eq!(only_y[0].id, "b/two");
    state.shutdown();
}

#[tokio::test]
async fn setting_the_url_connects_and_lists_online_agents() {
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let _provider = registered(addr, "prov", vec![card("prov", "papers", "find-papers")]).await;

    let state = AppState::for_tests(vec![]);
    let instance_id = network::local_instance_id(state.db()).unwrap();
    let relay_url = url(addr, &instance_id);
    set_relay_url(&state, &relay_url).await.unwrap();
    assert_eq!(
        resolve_url_in(None, state.nandi_dir(), state.db()).unwrap(),
        Some((relay_url.clone(), RelayUrlSource::Config))
    );
    // The token stays out of config.json
    assert_eq!(
        nandi_config::load_from(state.nandi_dir())
            .unwrap()
            .relay_url,
        Some(format!("ws://{addr}/relay"))
    );
    assert!(
        eventually(async || get_relay_status(&state).await.unwrap().connected).await,
        "never connected"
    );
    // Registered (queries need it) and sees the provider
    assert!(
        eventually(async || {
            list_online_agents(&state, Some("find-papers"))
                .await
                .is_ok_and(|cards| cards.len() == 1)
        })
        .await
    );
    // Presence is persisted for offline use
    assert!(eventually(async || !last_known_agents(state.db()).unwrap().is_empty()).await);
    set_relay_url(&state, "").await.unwrap();
    assert!(!get_relay_status(&state).await.unwrap().connected);
    state.shutdown();
}

#[tokio::test]
async fn the_relay_url_comes_from_the_env_then_config() {
    let state = AppState::for_tests(vec![]);
    let (dir, db) = (state.nandi_dir(), state.db());
    assert_eq!(resolve_url_in(None, dir, db).unwrap(), None);

    // The URL goes to config.json, its token to relay_token
    save_url(dir, db, "ws://old:8787/relay?token=t1").unwrap();
    assert_eq!(
        resolve_url_in(None, dir, db).unwrap(),
        Some((
            "ws://old:8787/relay?token=t1".to_string(),
            RelayUrlSource::Config
        ))
    );
    assert_eq!(
        configured_url(&state).as_deref(),
        Some("ws://old:8787/relay")
    );
    assert_eq!(
        nandi_config::load_from(dir).unwrap().relay_url.as_deref(),
        Some("ws://old:8787/relay")
    );
    assert_eq!(
        read_state::<String>(db, RELAY_TOKEN_KEY)
            .unwrap()
            .as_deref(),
        Some("t1")
    );

    // NANDI_RELAY_URL wins
    assert_eq!(
        resolve_url_in(Some("ws://env:8787/relay".into()), dir, db).unwrap(),
        Some(("ws://env:8787/relay".to_string(), RelayUrlSource::Env))
    );

    // Saving a URL without a token drops the old token
    save_url(dir, db, "ws://new:8787/relay").unwrap();
    assert_eq!(
        resolve_url_in(None, dir, db).unwrap(),
        Some(("ws://new:8787/relay".to_string(), RelayUrlSource::Config))
    );
    state.shutdown();
}

#[tokio::test]
async fn a_build_s_relay_is_only_offered_until_the_user_joins_it() {
    let state = AppState::for_tests_with_relay(
        vec![],
        BuiltInRelay::new(Some("wss://demo.nandi.dev/relay"), Some("code")),
    );
    let (dir, db) = (state.nandi_dir(), state.db());
    // Not used by itself, even with a token kept for it
    assert_eq!(resolve_url_in(None, dir, db).unwrap(), None);
    assert_eq!(configured_url(&state), None);
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url, None);
    assert_eq!(status.relay_url_source, None);
    assert_eq!(
        status.built_in_url.as_deref(),
        Some("wss://demo.nandi.dev/relay")
    );
    // Joined (saved with its token): used, and named as the build's relay
    save_url(dir, db, "wss://demo.nandi.dev/relay?token=t9").unwrap();
    assert_eq!(
        resolve_url_in(None, dir, db).unwrap(),
        Some((
            "wss://demo.nandi.dev/relay?token=t9".to_string(),
            RelayUrlSource::Config
        ))
    );
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url_source, Some(RelayUrlSource::BuiltIn));
    state.shutdown();
}

#[tokio::test]
async fn a_token_is_kept_for_its_relay_across_a_disconnect() {
    let state = AppState::for_tests(vec![]);
    let (dir, db) = (state.nandi_dir(), state.db());
    save_url(dir, db, "ws://a:8787/relay?token=ta").unwrap();
    // Disconnect keeps it, and the same relay gets it back
    save_url(dir, db, "").unwrap();
    assert_eq!(resolve_url_in(None, dir, db).unwrap(), None);
    save_url(dir, db, "ws://a:8787/relay").unwrap();
    assert_eq!(
        resolve_url_in(None, dir, db).unwrap().unwrap().0,
        "ws://a:8787/relay?token=ta"
    );
    // Another relay never gets it, and drops it
    save_url(dir, db, "ws://b:8787/relay").unwrap();
    assert_eq!(
        resolve_url_in(None, dir, db).unwrap().unwrap().0,
        "ws://b:8787/relay"
    );
    save_url(dir, db, "ws://a:8787/relay").unwrap();
    assert_eq!(
        resolve_url_in(None, dir, db).unwrap().unwrap().0,
        "ws://a:8787/relay"
    );
    state.shutdown();
}

#[test]
fn enrolling_goes_next_to_the_relay_path() {
    assert_eq!(
        join::enroll_endpoint("wss://demo.nandi.dev/relay?token=x")
            .unwrap()
            .as_str(),
        "https://demo.nandi.dev/relay/enroll"
    );
    assert_eq!(
        join::enroll_endpoint("ws://10.0.0.5:8787/nandi/relay")
            .unwrap()
            .as_str(),
        "http://10.0.0.5:8787/nandi/relay/enroll"
    );
}

#[tokio::test]
async fn instances_enroll_once_with_the_join_code() {
    let tmp = tempfile::tempdir().unwrap();
    let config = RelayServerConfig::new(b"s3cret".to_vec())
        .with_join_code("demo-2026".into())
        .with_data_dir(tmp.path());
    let addr = serve_with("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let relay = format!("ws://{addr}/relay");

    let token = join::enroll(&relay, "desktop-dev-1a2b", "demo-2026", None)
        .await
        .unwrap();
    assert_eq!(token, issue_instance_token(b"s3cret", "desktop-dev-1a2b"));
    // Once per ID: nobody else can take it, even with the code
    let again = join::enroll(&relay, "desktop-dev-1a2b", "demo-2026", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(again.contains("409"), "{again}");
    let wrong = join::enroll(&relay, "desktop-sam-9z", "guess", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(wrong.contains("401"), "{wrong}");
    let bad = join::enroll(&relay, "../etc", "demo-2026", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(bad.contains("400"), "{bad}");
    // Kept across restarts
    let kept: Vec<String> = nandi_relay::Store::open(tmp.path())
        .unwrap()
        .members()
        .unwrap()
        .into_iter()
        .map(|member| member.instance_id)
        .collect();
    assert_eq!(kept, ["desktop-dev-1a2b"]);
    let config = RelayServerConfig::new(b"s3cret".to_vec())
        .with_join_code("demo-2026".into())
        .with_data_dir(tmp.path());
    let restarted = serve_with("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let after = join::enroll(
        &format!("ws://{restarted}/relay"),
        "desktop-dev-1a2b",
        "demo-2026",
        None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(after.contains("409"), "{after}");

    // Without a join code, or without a secret, nobody enrolls
    let open = serve_with("127.0.0.1:0".parse().unwrap(), RelayServerConfig::open())
        .await
        .unwrap();
    let closed = join::enroll(&format!("ws://{open}/relay"), "x", "demo-2026", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(closed.contains("404"), "{closed}");
}

// ---------------------------------------------------------------------------
// The app against an open relay, and raw peers
// ---------------------------------------------------------------------------

type Raw =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn raw(addr: SocketAddr) -> Raw {
    tokio_tungstenite::connect_async(format!("ws://{addr}/relay"))
        .await
        .unwrap()
        .0
}

async fn send(socket: &mut Raw, message: RelayMessage) {
    socket
        .send(Message::Text(message.to_json().unwrap().into()))
        .await
        .unwrap();
}

/// The next relay message, within 5s
async fn recv(socket: &mut Raw) -> RelayMessage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => return RelayMessage::from_json(&text).unwrap(),
                Some(Ok(_)) => {}
                other => panic!("the relay hung up: {other:?}"),
            }
        }
    })
    .await
    .expect("no message from the relay")
}

/// Registers `instance_id` with no token, as on an open relay
async fn register_raw(addr: SocketAddr, instance_id: &str, cards: Vec<AgentCard>) -> Raw {
    let mut socket = raw(addr).await;
    send(
        &mut socket,
        RelayMessage::RegisterInstance {
            instance_id: instance_id.into(),
            agent_cards: cards,
            signature: String::new(),
            capabilities: vec![],
            tools: vec![],
            protocol_version: super::protocol::PROTOCOL_VERSION,
            public_keys: None,
        },
    )
    .await;
    assert!(matches!(
        recv(&mut socket).await,
        RelayMessage::Registered { .. }
    ));
    socket
}

#[tokio::test]
async fn test_connection_answers_with_the_round_trip_or_the_exact_error() {
    let addr = serve_open(local()).await.unwrap();
    let ms = test_relay_connection(&format!("ws://{addr}/relay"))
        .await
        .unwrap();
    assert!(ms < 2000, "{ms}");
    // The token is stripped before dialing
    assert!(test_relay_connection(&format!("ws://{addr}/relay?token=t"))
        .await
        .is_ok());

    // Nothing listening
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let error = test_relay_connection(&format!("ws://127.0.0.1:{port}/relay"))
        .await
        .unwrap_err();
    assert!(
        error.starts_with(&format!(
            "Couldn't open a WebSocket to ws://127.0.0.1:{port}/relay: "
        )),
        "{error}"
    );

    // The wrong path fails the handshake
    let error = test_relay_connection(&format!("ws://{addr}/nope"))
        .await
        .unwrap_err();
    assert!(error.contains("404"), "{error}");

    // A WebSocket peer that never answers pings
    let mute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mute_addr = mute.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = mute.accept().await.unwrap();
        let _socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let error = test_relay_connection(&format!("ws://{mute_addr}/relay"))
        .await
        .unwrap_err();
    assert!(error.contains("didn't answer a ping within 2s"), "{error}");

    assert!(test_relay_connection("http://relay/relay")
        .await
        .unwrap_err()
        .contains("ws://"));
    assert!(test_relay_connection(" ")
        .await
        .unwrap_err()
        .contains("Enter a relay URL"));
}

#[tokio::test]
async fn the_app_connects_to_an_open_relay_and_lists_its_instances() {
    let addr = serve_open(local()).await.unwrap();
    let _peer = register_raw(addr, "peer", vec![card("peer", "papers", "find-papers")]).await;
    let state = AppState::for_tests(vec![]);
    set_relay_url(&state, &format!("ws://{addr}/relay"))
        .await
        .unwrap();
    assert!(
        eventually(async || get_relay_status(&state).await.unwrap().state
            == RelayConnectionState::Connected)
        .await,
        "never connected"
    );
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.relay_url_source, Some(RelayUrlSource::Config));
    assert!(status.last_connected_at.is_some());
    assert!(
        eventually(
            async || get_relay_instances(&state).await.is_ok_and(|peers| peers
                .iter()
                .any(|p| p.instance_id == "peer" && p.agents == 1))
        )
        .await
    );
    assert!(
        eventually(async || get_relay_status(&state).await.unwrap().instances_online >= 1).await
    );
    set_relay_url(&state, "").await.unwrap();
    let status = get_relay_status(&state).await.unwrap();
    assert_eq!(status.state, RelayConnectionState::Disconnected);
    assert!(get_relay_instances(&state).await.is_err());
    state.shutdown();
}

#[tokio::test]
async fn agents_turned_on_while_connected_reach_the_relay() {
    use crate::services::domain::{Agent, AgentId, EntityKind};
    use crate::services::repos::Repository;
    let addr = serve_open(local()).await.unwrap();
    let peer = registered(addr, "peer", Vec::new()).await;
    let state = AppState::for_tests(vec![]);
    set_relay_url(&state, &format!("ws://{addr}/relay"))
        .await
        .unwrap();
    assert!(eventually(async || get_relay_status(&state).await.unwrap().connected).await);
    share(&state, true);

    // An agent starts accepting delegation after the connect
    let agent = Agent::from_skill_md(
        AgentId::parse("papers").unwrap(),
        "---\nname: Papers\nserves: [{name: find-papers}]\naccepts_delegation: true\n---\nHi.\n",
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, "papers").unwrap();
    assert!(
        eventually(async || peer
            .query_capability("find-papers")
            .await
            .is_ok_and(|cards| cards.len() == 1))
        .await,
        "the peer never saw the agent"
    );
    state.shutdown();
}

/// Writes NANDI.md with sharing (`relay.enabled`) on or off
fn share(state: &AppState, enabled: bool) {
    use crate::services::gateway::manifest;
    let root = state.store().repos().layout().root.clone();
    let mut nandi = manifest::parse(&manifest::render_default("Ada")).unwrap();
    nandi.relay.enabled = enabled;
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join(manifest::MANIFEST_FILE),
        manifest::serialize(&nandi, None).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn agents_stay_off_the_relay_while_sharing_is_off() {
    use crate::services::domain::{Agent, AgentId, EntityKind};
    use crate::services::repos::Repository;
    let addr = serve_open(local()).await.unwrap();
    let peer = registered(addr, "peer", Vec::new()).await;
    let state = AppState::for_tests(vec![]);
    set_relay_url(&state, &format!("ws://{addr}/relay"))
        .await
        .unwrap();
    assert!(eventually(async || get_relay_status(&state).await.unwrap().connected).await);
    share(&state, false);

    let agent = Agent::from_skill_md(
        AgentId::parse("papers").unwrap(),
        "---\nname: Papers\nserves: [{name: find-papers}]\naccepts_delegation: true\n---\nHi.\n",
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, "papers").unwrap();
    // Past the re-registration delay: still nothing listed
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(peer
        .query_capability("find-papers")
        .await
        .is_ok_and(|cards| cards.is_empty()));
    state.shutdown();
}

#[tokio::test]
async fn queries_sent_before_registration_wait_for_it() {
    let addr = serve_open(local()).await.unwrap();
    let _peer = registered(addr, "peer", vec![card("peer", "papers", "find-papers")]).await;
    let client = Arc::new(WsRelayClient::new(None, "me"));
    client
        .connect(&format!("ws://{addr}/relay"), "me")
        .await
        .unwrap();
    // Startup: presence and the directory ask before `register` is done
    let asking = client.clone();
    let early = tokio::spawn(async move {
        (
            asking.query_capability("find-papers").await,
            asking.query_directory().await,
        )
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    client.register(Vec::new()).await.unwrap();
    let (cards, members) = early.await.unwrap();
    assert_eq!(cards.unwrap().len(), 1);
    assert!(members.is_ok());
}

// ---------------------------------------------------------------------------
// TLS and the seat limit, as the app sees them
// ---------------------------------------------------------------------------

fn testdata(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/services/relay/testdata")
        .join(name)
}

/// Sends a `RegisterInstance` for `instance_id` with its token
async fn register_signed(socket: &mut Raw, instance_id: &str) {
    send(
        socket,
        RelayMessage::RegisterInstance {
            instance_id: instance_id.into(),
            agent_cards: vec![],
            signature: issue_instance_token(SECRET, instance_id),
            capabilities: vec![],
            tools: vec![],
            protocol_version: super::protocol::PROTOCOL_VERSION,
            public_keys: None,
        },
    )
    .await;
}

#[tokio::test]
async fn the_client_trusts_a_company_ca_for_wss() {
    let config = RelayServerConfig::new(SECRET.to_vec()).with_tls(nandi_relay::TlsFiles {
        cert: testdata("server.pem"),
        key: testdata("server-key.pem"),
    });
    let addr = serve_with(local(), config).await.unwrap();
    let wss = format!("wss://localhost:{}/relay", addr.port());

    // Trusting the company's CA: connects and registers
    let connector = tokio_tungstenite::Connector::Rustls(super::tls::client_config_trusting(
        &testdata("ca.pem"),
    ));
    let (mut socket, _) = tokio_tungstenite::connect_async_tls_with_config(
        wss.as_str(),
        None,
        false,
        Some(connector),
    )
    .await
    .unwrap();
    register_signed(&mut socket, "tls-instance").await;
    assert!(matches!(
        recv(&mut socket).await,
        RelayMessage::Registered { .. }
    ));

    // Public CAs only: the handshake is refused
    super::client::ensure_tls_provider();
    assert!(tokio_tungstenite::connect_async(wss.as_str())
        .await
        .is_err());
}

#[tokio::test]
async fn a_full_relay_s_reason_reaches_the_app() {
    // A relay at its free limit of 20 users
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = std::sync::Arc::new(nandi_relay::Store::open(tmp.path()).unwrap());
        let mut roster =
            nandi_relay::roster::Roster::load(nandi_relay::SeatPolicy::free(), store).unwrap();
        let today = chrono::Utc::now().date_naive();
        for n in 0..20 {
            roster.admit(&format!("user-{n}"), today).unwrap();
        }
    }
    let config = RelayServerConfig::new(SECRET.to_vec())
        .with_join_code("code".into())
        .with_data_dir(tmp.path());
    let addr = serve_with(local(), config).await.unwrap();
    let relay = format!("ws://{addr}/relay");

    // Enrolling says why, with the relay's own words
    let refused = join::enroll(&relay, "newcomer", "code", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("402"), "{refused}");
    assert!(refused.contains("free limit of 20 users"), "{refused}");

    // So does the connection the app then tries without a token
    let client = WsRelayClient::new(None, "newcomer");
    client.connect(&relay, "newcomer").await.unwrap();
    assert!(client.register(Vec::new()).await.is_err());
    let status = client.status();
    assert_eq!(status.state, RelayConnectionState::Error);
    let reason = status.reason.unwrap_or_default();
    assert!(reason.contains("free limit of 20 users"), "{reason}");
}

// ---------------------------------------------------------------------------
// The member directory, people search and the protocol version
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_directory_keeps_members_who_left_and_people_are_found_at_the_relay() {
    use crate::services::gateway::{Availability, AvailabilityStatus};
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    let priya = registered(addr, "priya", Vec::new()).await;
    let mut card = gateway_card("priya", true);
    card.responsibilities = vec!["SOX audit".into()];
    card.availability = Availability {
        status: AvailabilityStatus::Away,
        until: None,
        delegate_to: Some("lena".into()),
    };
    priya.register_gateway(card).await.unwrap();
    let me = registered(addr, "me", Vec::new()).await;

    assert!(
        eventually(async || me
            .query_directory()
            .await
            .is_ok_and(|members| members.len() == 1 && members[0].online))
        .await
    );
    let found = me.find_people("who owns the SOX audit", 5).await.unwrap();
    assert_eq!(found.total, 1);
    assert_eq!(found.matches[0].instance_id, "priya");
    assert_eq!(
        found.matches[0].availability.delegate_to.as_deref(),
        Some("lena")
    );
    assert!(me
        .find_people("payroll", 5)
        .await
        .unwrap()
        .matches
        .is_empty());

    // Priya closes her laptop: still in the directory, offline
    priya.connect("", "priya").await.unwrap();
    assert!(
        eventually(async || me
            .query_directory()
            .await
            .is_ok_and(|members| members.len() == 1 && !members[0].online))
        .await
    );
}

#[tokio::test]
async fn a_relay_on_another_protocol_version_reads_as_not_connected() {
    use super::protocol::VERSION_MISMATCH;
    // A relay that refuses every registration for its version
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(frame)) = socket.next().await {
                    if !frame.is_text() {
                        continue;
                    }
                    let refusal = RelayMessage::Error {
                        code: VERSION_MISMATCH.into(),
                        message: "This Nandi speaks relay protocol 2 and the relay speaks 3. Update Nandi or the relay so they match.".into(),
                    };
                    let text = refusal.to_json().unwrap();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    let client: Arc<dyn RelayClient> = Arc::new(WsRelayClient::new(None, "me"));
    client
        .connect(&format!("ws://{addr}/relay"), "me")
        .await
        .unwrap();
    let refused = client.register(Vec::new()).await.unwrap_err();
    assert!(refused.contains("Update Nandi or the relay"), "{refused}");
    let status = client.status();
    assert_eq!(status.state, RelayConnectionState::Error);
    assert!(status.version_mismatch);
    assert!(!status.connected);
    // Person-to-person actions: not connected
    assert!(unavailable::connected(Some(&client)).is_err());
    let searched = client.find_people("x", 5).await.unwrap_err();
    assert!(RelayUnavailable::is(&searched), "{searched}");
}

/// A relay that registers everyone in `groups` visibility, answers every
/// group call (pushing a `GroupChanged` first), and hangs up on `Leave`
/// without answering
async fn scripted_groups_relay() -> SocketAddr {
    use nandi_protocol::{GroupChange, GroupOp, GroupOutcome, Visibility};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(frame)) = socket.next().await {
                    let Ok(text) = frame.to_text() else { continue };
                    let replies = match RelayMessage::from_json(text) {
                        Ok(RelayMessage::RegisterInstance { instance_id, .. }) => {
                            vec![RelayMessage::Registered {
                                instance_id,
                                visibility: Visibility::Groups,
                            }]
                        }
                        Ok(RelayMessage::GroupRequest {
                            op: GroupOp::Leave { .. },
                            ..
                        }) => {
                            return;
                        }
                        Ok(RelayMessage::GroupRequest { request_id, op }) => vec![
                            RelayMessage::GroupChanged {
                                group_id: "g_0000000000000001".into(),
                                change: GroupChange::PolicyChanged { version: 2 },
                            },
                            RelayMessage::GroupReply {
                                request_id,
                                outcome: match op {
                                    GroupOp::List => GroupOutcome::List {
                                        groups: Vec::new(),
                                        invites: Vec::new(),
                                    },
                                    _ => GroupOutcome::Refused {
                                        code: "group_not_admin".into(),
                                        message: "no".into(),
                                    },
                                },
                            },
                        ],
                        _ => Vec::new(),
                    };
                    for reply in replies {
                        let text = reply.to_json().unwrap();
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    addr
}

#[tokio::test]
async fn group_calls_are_answered_by_request_id_and_fail_on_disconnect() {
    use super::client_api::GroupSignal;
    use nandi_protocol::{GroupOp, GroupOutcome};
    let addr = scripted_groups_relay().await;
    let client: Arc<dyn RelayClient> = Arc::new(WsRelayClient::new(None, "me"));
    // Not connected yet: the typed "not connected"
    let early = client.group_request(GroupOp::List).await.unwrap_err();
    assert!(RelayUnavailable::is(&early), "{early}");
    let signals = Arc::new(Mutex::new(Vec::new()));
    let seen = signals.clone();
    client
        .on_groups(Box::new(move |signal| seen.lock().unwrap().push(signal)))
        .await;
    client
        .connect(&format!("ws://{addr}/relay"), "me")
        .await
        .unwrap();
    client.register(Vec::new()).await.unwrap();
    assert_eq!(client.status().visibility, Some(RelayVisibility::Groups));

    // Two calls at once each get their own answer
    let (list, get) = tokio::join!(
        client.group_request(GroupOp::List),
        client.group_request(GroupOp::Get {
            group_id: "g_0000000000000001".into()
        })
    );
    assert!(matches!(list.unwrap(), GroupOutcome::List { .. }));
    assert!(
        matches!(get.unwrap(), GroupOutcome::Refused { code, .. } if code == "group_not_admin")
    );
    let signals_now = signals.lock().unwrap().clone();
    assert_eq!(signals_now.first(), Some(&GroupSignal::Registered));
    assert!(signals_now.iter().any(
        |s| matches!(s, GroupSignal::Changed { group_id, .. } if group_id == "g_0000000000000001")
    ));

    // The relay hangs up mid-call: the waiter fails at once, not at the timeout
    let started = std::time::Instant::now();
    let left = client
        .group_request(GroupOp::Leave {
            group_id: "g_0000000000000001".into(),
        })
        .await
        .unwrap_err();
    assert!(RelayUnavailable::is(&left), "{left}");
    assert!(started.elapsed() < Duration::from_secs(4));
}

/// Against the real relay (nandi-relay, protocol 7): a group made on one
/// Nandi is joined from another, whose copy verifies against the
/// creator's pinned key and holds their requests for approval
#[tokio::test]
async fn on_the_real_relay_a_member_verifies_and_enforces_the_policy() {
    use crate::services::domain::group::{Decision, POLICY_TEMPLATE};
    use crate::services::groups::{enforce, Groups, InviteRequest};
    use crate::services::repos::{RepoLayout, Repos};
    use nandi_seal::DeviceKeys;
    let addr = serve(local(), SECRET.to_vec()).await.unwrap();
    struct World {
        _tmp: tempfile::TempDir,
        repos: Repos,
        db: crate::db::Db,
    }
    let world = || {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        World {
            _tmp: tmp,
            repos,
            db: crate::db::Db::in_memory(),
        }
    };
    async fn nandi_on(
        addr: std::net::SocketAddr,
        world: &World,
        me: &str,
        keys: &DeviceKeys,
    ) -> Groups {
        let secret = keys.export_secret().to_string();
        let source = secret.clone();
        let client = WsRelayClient::with_options(
            None,
            me,
            ClientOptions {
                device_keys: Some(Arc::new(move || {
                    DeviceKeys::import_secret(&source).map_err(|e| e.to_string())
                })),
                ..ClientOptions::default()
            },
        );
        let token = issue_instance_token(SECRET, me);
        client
            .connect(&format!("ws://{addr}/relay?token={token}"), me)
            .await
            .unwrap();
        client.register(Vec::new()).await.unwrap();
        let public = keys.public_keys().clone();
        Groups::new(
            Some(Arc::new(client)),
            world.repos.groups.clone(),
            world.db.clone(),
            me.to_string(),
            Arc::new(move || DeviceKeys::import_secret(&secret).map_err(|e| e.to_string())),
            Arc::new(move || Ok(public.clone())),
            None,
        )
    }
    let (alice_world, bob_world) = (world(), world());
    let (alice_keys, bob_keys) = (
        DeviceKeys::generate().unwrap(),
        DeviceKeys::generate().unwrap(),
    );
    let alice = nandi_on(addr, &alice_world, "alice-1", &alice_keys).await;
    let bob = nandi_on(addr, &bob_world, "bob-1", &bob_keys).await;

    let created = alice
        .create("Finance", "Month-end", POLICY_TEMPLATE)
        .await
        .unwrap();
    assert!(created.policy.verified, "{:?}", created.policy.problem);
    let id = created.group.summary.id.clone();
    let overview = bob.overview().await.unwrap();
    assert!(overview
        .groups
        .iter()
        .any(|g| g.id == id && g.role.is_none()));

    let invite = alice
        .invite(
            &id,
            InviteRequest {
                instance_id: Some("bob-1".into()),
                max_uses: 1,
                expires_at: None,
            },
        )
        .await
        .unwrap();
    let joined = bob.join(&invite.code).await.unwrap();
    assert!(joined.policy.verified, "{:?}", joined.policy.problem);
    assert_eq!(joined.group.members.len(), 2);

    // Bob's Nandi holds Alice's requests for his approval (approval: always)
    let gate = enforce::Enforcer {
        repo: &bob_world.repos.groups,
        db: &bob_world.db,
        me: "bob-1",
        own_keys: Some(bob_keys.public_keys()),
    }
    .decide("alice-1", "review-pr", enforce::Moment::now())
    .unwrap();
    assert!(matches!(gate.decision, Decision::Ask(_)));

    // A new version from Alice reaches Bob verified
    let text = "---\nrequests:\n  approval: per-member\n---\n";
    assert_eq!(alice.set_policy(&id, text).await.unwrap().policy.version, 2);
    let seen = bob.get(&id).await.unwrap();
    assert!(seen.policy.verified && seen.policy.version == 2);
    assert_eq!(
        bob_world.repos.groups.load(&id).unwrap().unwrap().text,
        text
    );
}

#[tokio::test]
async fn on_a_groups_relay_someone_outside_our_groups_is_named_as_such() {
    use nandi_protocol::{Visibility, NO_SHARED_GROUP};
    use nandi_seal::DeviceKeys;
    let config = RelayServerConfig::new(SECRET.to_vec()).with_visibility(Visibility::Groups);
    let addr = serve_with(local(), config).await.unwrap();
    let connect = |me: &'static str| async move {
        let keys = DeviceKeys::generate().unwrap().export_secret().to_string();
        let client = WsRelayClient::with_options(
            None,
            me,
            ClientOptions {
                device_keys: Some(Arc::new(move || {
                    DeviceKeys::import_secret(&keys).map_err(|e| e.to_string())
                })),
                ..ClientOptions::default()
            },
        );
        let token = issue_instance_token(SECRET, me);
        client
            .connect(&format!("ws://{addr}/relay?token={token}"), me)
            .await
            .unwrap();
        client.register(Vec::new()).await.unwrap();
        client
    };
    let alice = connect("alice-1").await;
    let _bob = connect("bob-1").await;
    // Bob published keys, but the relay hides them from Alice: not "update Nandi"
    let err = alice.public_keys_of("bob-1").await.unwrap_err();
    assert!(err.starts_with(NO_SHARED_GROUP), "{err}");
}
