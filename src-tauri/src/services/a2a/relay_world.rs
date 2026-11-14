//! The real relay for the delegation tests: `nandi_relay::testing::
//! serve_with_faults` on 127.0.0.1:0 (the dev-dependency), the app's own
//! WebSocket client on every side, sealing on, nothing mocked. The relay's
//! [`Faults`] handle makes what the network does: a frame lost or late, an
//! instance offline, a task ended now. It also records every frame, which
//! is how a test sees what a side sent.
//!
//! Each test starts its own relay: the relay plans by capability across
//! every instance it knows, so a shared one would hand one test's request
//! to another test's provider. Starting one takes a few milliseconds.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nandi_protocol::RelayMessage;
use nandi_relay::testing::{serve_with_faults, Direction, Faults};
use nandi_relay::RelayServerConfig;
use serde_json::json;

use super::delegator::{A2aDeps, Delegator};
use super::history::AuditLog;
pub(crate) use super::tests::wait_until;
use super::tests::{deps, echo_agents, from_writer, instance, lifecycle, save_agent};
use super::*;
use crate::services::domain::agent_card::OwnerRef;
use crate::services::events::tests::RecordingSink;
use crate::services::events::EventSink;
use crate::services::gateway::GatewayCard;
use crate::services::relay::client::{ClientOptions, WsRelayClient};
use crate::services::relay::RelayClient;
use crate::services::trust;

/// How long a test client waits for the relay's answer: a lost answer
/// costs this, not the app's 5 s
const ANSWER_TIMEOUT: Duration = Duration::from_secs(1);

/// The provider's agent: serves `summarize` and takes delegation
pub(super) const PAPERS: &str = "serves: [{name: summarize}]\naccepts_delegation: true\n";

/// A relay of one test's own, with its fault handle
pub(crate) struct TestRelay {
    addr: SocketAddr,
    pub faults: Faults,
}

impl TestRelay {
    pub async fn start() -> Self {
        Self::start_with(RelayServerConfig::open()).await
    }

    /// A relay set up as `config` says
    pub async fn start_with(config: RelayServerConfig) -> Self {
        let bind = SocketAddr::from(([127, 0, 0, 1], 0));
        let (addr, faults) = serve_with_faults(bind, config).await.unwrap();
        Self { addr, faults }
    }

    /// A client for `instance`, connected (not registered yet)
    pub async fn client(&self, instance: &str) -> Arc<WsRelayClient> {
        self.client_with(instance, ClientOptions::default()).await
    }

    /// [`Self::client`] with `options` (the answer timeout is this
    /// harness's)
    pub async fn client_with(&self, instance: &str, options: ClientOptions) -> Arc<WsRelayClient> {
        let options = ClientOptions {
            request_timeout: ANSWER_TIMEOUT,
            ..options
        };
        let client = Arc::new(WsRelayClient::with_options(None, instance, options));
        client.connect(&self.url(), instance).await.unwrap();
        client
    }

    /// A bare member: `instance` registered with `cards`, and no app behind
    /// it (what it's sent stays unanswered)
    pub async fn member(&self, instance: &str, cards: Vec<AgentCard>) -> Arc<WsRelayClient> {
        let client = self.client(instance).await;
        client.register(cards).await.unwrap();
        client
    }

    /// Connects `client` again (its cards registered again), as the app
    /// does once the network is back, and waits until the relay has it
    pub async fn reconnect(&self, client: &WsRelayClient, instance: &str) {
        client.connect(&self.url(), instance).await.unwrap();
        wait_until(&format!("{instance} back on the relay"), || async {
            self.faults.is_online(instance) && client.status().connected
        })
        .await;
    }

    /// Lets `instance` back after [`Faults::set_online`] kept it away, and
    /// reconnects its client
    pub async fn bring_back(&self, client: &WsRelayClient, instance: &str) {
        self.faults.set_online(instance, true);
        self.reconnect(client, instance).await;
    }

    fn url(&self) -> String {
        format!("ws://{}/relay", self.addr)
    }

    /// The frames `instance` sent the relay, in order
    fn sent_by(&self, instance: &str) -> Vec<RelayMessage> {
        self.faults
            .frames()
            .into_iter()
            .filter(|frame| frame.instance == instance && frame.direction == Direction::Received)
            .map(|frame| frame.message)
            .collect()
    }

    /// The IDs of the tasks `instance` submitted, in order (a resend
    /// counts again)
    pub fn submitted(&self, instance: &str) -> Vec<String> {
        self.sent_by(instance)
            .into_iter()
            .filter_map(|message| match message {
                RelayMessage::SubmitTask { task_id, .. } => Some(task_id),
                _ => None,
            })
            .collect()
    }

    /// The peer messages `instance` sent: (kind, task ID), in order
    pub fn peer_sent(&self, instance: &str) -> Vec<(String, String)> {
        self.sent_by(instance)
            .into_iter()
            .filter_map(|message| match message {
                RelayMessage::Peer { body, .. } => {
                    let kind = serde_json::to_value(&body).ok()?["kind"]
                        .as_str()?
                        .to_string();
                    Some((kind, body.task_id().to_string()))
                }
                _ => None,
            })
            .collect()
    }

    /// How many results `instance` sent the relay
    pub fn results_sent(&self, instance: &str) -> usize {
        let sent = self.sent_by(instance);
        sent.iter()
            .filter(|message| matches!(message, RelayMessage::TaskResult { .. }))
            .count()
    }

    /// How many frames of `kind` (see [`nandi_relay::testing::frame_kind`])
    /// `instance` sent the relay
    pub fn frames_sent(&self, instance: &str, kind: &str) -> usize {
        let kinds = self.faults.kinds(instance, Direction::Received);
        kinds.iter().filter(|k| *k == kind).count()
    }

    /// How many `kind` peer messages `instance` sent
    pub fn peer_count(&self, instance: &str, kind: &str) -> usize {
        let sent = self.peer_sent(instance);
        sent.iter().filter(|(k, _)| k == kind).count()
    }
}

/// One Nandi on the relay: its state, its delegator and task service on its
/// own client, registered with its agents' cards. The state's own relay
/// client stays unconnected: what goes through `state.relay_client()`
/// (`executor::gateway_dispatched`'s messages) never leaves.
pub(super) struct Nandi {
    pub state: AppState,
    pub delegator: Arc<Delegator>,
    pub relay: Arc<WsRelayClient>,
    /// Kept alive: the registry holds task services weakly
    pub service: Arc<task::TaskService>,
}

/// How a Nandi is set up beyond its agents
#[derive(Default)]
pub(super) struct Setup {
    /// How long a delegation waits for its answer live (the app's default
    /// when `None`)
    pub live_wait: Option<Duration>,
    /// Records the app events its state emits
    pub recorder: Option<Arc<RecordingSink>>,
}

impl Nandi {
    /// A Nandi with `agents` (ID, frontmatter), each echoing its input, its
    /// audit log in `log_dir`
    pub async fn join(relay: &TestRelay, agents: &[(&str, &str)], log_dir: &Path) -> Self {
        Self::join_with(relay, agents, log_dir, Setup::default()).await
    }

    pub async fn join_with(
        relay: &TestRelay,
        agents: &[(&str, &str)],
        log_dir: &Path,
        setup: Setup,
    ) -> Self {
        let sinks: Vec<Arc<dyn EventSink>> = setup
            .recorder
            .into_iter()
            .map(|recorder| recorder as Arc<dyn EventSink>)
            .collect();
        let state = AppState::for_tests(sinks);
        echo_agents(&state).await;
        for (id, frontmatter) in agents {
            save_agent(&state, id, frontmatter);
        }
        let me = instance(&state);
        let client = relay.client(&me).await;
        let service = lifecycle(&state, client.clone(), Default::default()).await;
        let defaults = deps(&state, Some(client.clone()));
        let delegator = Delegator::new(A2aDeps {
            audit: AuditLog::at(log_dir.join(format!("{me}.log"))),
            timeout: setup.live_wait.unwrap_or(defaults.timeout),
            ..defaults
        });
        delegator.clone().register_relay_handlers().await;
        // Registered, as the app does once connected: the relay answers
        // only registered instances
        client.register(delegator.local_cards("")).await.unwrap();
        Self {
            state,
            delegator,
            relay: client,
            service,
        }
    }

    pub fn instance(&self) -> String {
        instance(&self.state)
    }

    /// Opens this Nandi's gateway at the relay: it takes requests for its
    /// owner (`gateway`, `assign-me`) through its gateway card
    pub async fn open_gateway(&self) -> String {
        let me = self.instance();
        let card = GatewayCard {
            instance_id: me.clone(),
            endpoint: String::new(),
            display_name: me.clone(),
            description: String::new(),
            capabilities: Vec::new(),
            accepts_delegation: true,
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
        };
        self.relay.register_gateway(card).await.unwrap();
        AgentCard::card_id(&me, crate::services::gateway::GATEWAY_AGENT_ID)
    }

    /// Anyone may ask `agent`
    pub fn allow_everyone(&self, agent: &str) {
        let db = self.state.db();
        trust::set_policy(db, trust::ANY_REQUESTER, agent, trust::TrustPolicy::Allow).unwrap();
    }
}

/// Two Nandis on a real relay: a requester with `writer`, and a provider
/// with `papers`
pub(super) struct RelayWorld {
    pub relay: TestRelay,
    pub requester: Nandi,
    pub provider: Nandi,
    pub log_dir: tempfile::TempDir,
}

impl RelayWorld {
    pub async fn new(provider_frontmatter: &str) -> Self {
        Self::with(provider_frontmatter, Setup::default()).await
    }

    /// The requester set up as `setup` says
    pub async fn with(provider_frontmatter: &str, setup: Setup) -> Self {
        let relay = TestRelay::start().await;
        let log_dir = tempfile::tempdir().unwrap();
        let writer = [("writer", "delegation_outbound: true\n")];
        let requester = Nandi::join_with(&relay, &writer, log_dir.path(), setup).await;
        let papers = [("papers", provider_frontmatter)];
        let provider = Nandi::join(&relay, &papers, log_dir.path()).await;
        Self {
            relay,
            requester,
            provider,
            log_dir,
        }
    }

    /// A world whose provider lets anyone ask `papers`
    pub async fn allowing() -> Self {
        let world = Self::new(PAPERS).await;
        world.provider.allow_everyone("papers");
        world
    }

    /// Another provider like the first, letting anyone ask its `papers`
    pub async fn join_provider(&self) -> Nandi {
        let papers = [("papers", PAPERS)];
        let nandi = Nandi::join(&self.relay, &papers, self.log_dir.path()).await;
        nandi.allow_everyone("papers");
        nandi
    }

    /// The provider's `papers` card ID
    pub fn papers(&self) -> String {
        format!("{}/papers", self.provider.instance())
    }

    /// The writer asks for a summary of `input`
    pub async fn ask_for(&self, input: serde_json::Value) -> TaskResult {
        self.requester
            .delegator
            .run(from_writer("summarize", input))
            .await
            .unwrap()
    }

    pub async fn ask(&self) -> TaskResult {
        self.ask_for(json!("hi")).await
    }

    /// `task` (the requester's, to the provider) as the relay carries it
    /// once submitted, its delivery lost: the provider never heard of it,
    /// but the relay routes its messages and its result
    pub async fn carry(&self, task: &Task) {
        let provider = self.provider.instance();
        self.relay.faults.drop_next(&provider, "deliver.task");
        let held = self.requester.relay.delegate_task(task, &[]).await.unwrap();
        assert!(!held.held, "the provider is online");
    }

    pub fn shutdown(self) {
        self.requester.state.shutdown();
        self.provider.state.shutdown();
    }
}
