//! `RelayClient` over tokio-tungstenite: outbound WebSocket, exponential
//! backoff reconnect (1s, 2s, 4s, 8s, 16s, then every 30s), re-registration
//! plus an immediate heartbeat on every reconnect, 30s heartbeats.
//!
//! Every state change (connecting, connected, reconnecting, disconnected,
//! error) emits `relay://status` with the reason.
//!
//! One supervisor task per `connect` owns the socket: it reads frames,
//! writes queued messages, and sends heartbeats. Requests that expect an
//! answer (`register`, `query_capability`, `route`, `delegate_task`, …)
//! park a waiter in a `Pending` that the reader resolves, and wait in
//! `await_reply`; a lost connection fails them with `RelayUnavailable`.
//! A new `connect` (or `connect("")`, which disconnects) cancels the
//! previous supervisor.
//!
//! The instance token is configured with the relay URL as a `token` query
//! parameter (`wss://relay.example/relay?token=…`, see `server.rs`); the
//! client strips it from the URL it dials and signs its messages with it.
//!
//! After each registration and heartbeat the client asks the relay for
//! every card (`*`), and emits `relay://agent_online` and
//! `relay://agent_offline` for the differences.
//!
//! Registration carries [`PROTOCOL_VERSION`]. A relay that speaks another
//! version refuses it (`version_mismatch`): the status turns to `Error`
//! with `version_mismatch` set, and the client counts as not connected
//! until a later connection registers.
//!
//! Registration also carries this device's public keys
//! ([`ClientOptions::device_keys`], read on the first `connect`). A relay
//! that has other keys pinned for this instance ID refuses it
//! (`key_mismatch`): the status turns to `Error` with `key_mismatch` set.
//!
//! # Sealing
//!
//! Every person-to-person payload is sealed here, and only here
//! (`sealing`): a task's input for its provider, a result for its
//! requester, notes, reasons, questions, answers and status results for the
//! other side. The rest of the app hands this client opened messages
//! (`opened`) and gets opened ones back.
//!
//! - **Keys.** The recipient's keys come from the key book
//!   ([`ClientOptions::peer_keys`], `network::peer_keys` in the app): the
//!   pinned ones, else asked of the relay (`PublicKeysQuery`) and pinned on
//!   first sight. A member whose key changed is refused
//!   (`PEER_KEY_CHANGED`) for every send, content or not, until the user
//!   accepts it; one with no keys anywhere can't get content
//!   (`PEER_KEY_MISSING`).
//! - **Opening.** Everything a task's other side sends goes through one
//!   queue, opened in order on its own task (the author's keys may need a
//!   relay query, which this reader answers). An envelope that doesn't
//!   open, isn't for us or this task, or isn't signed by its author's
//!   pinned key is refused with `SEAL_REFUSED`: logged, emitted as
//!   `relay://seal_refused`, and, for a task we asked for, its failure
//!   (`on_task_failed`). A duplicate `(task, from, seq)` is dropped.
//! - **Reassignment.** Handing a task on, we reseal the request we opened
//!   for the new provider ([`nandi_seal::reseal`], the requester's
//!   signature kept) and send `Reassign`; the relay moves it. As its
//!   requester, asked to (`Reseal`), we seal our own request again.
//! - Content over `MAX_ENVELOPE_BYTES` isn't sent (`ENVELOPE_TOO_LARGE`).
//!
//! # Store-and-forward
//!
//! The relay keeps everything for us in our mailbox until we ack it
//! (protocol 6). After each registration the client sends `InboxSync`,
//! and again whenever a handler is added, so nothing is acked before
//! something here handles it. Each delivery goes through the opening
//! queue in order, then is acked; one handled already on this connection
//! is acked again and dropped. `delegate_task` sends `SubmitTask` with the
//! request sealed for each candidate, so the relay can fail it over with
//! us offline.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::{FutureExt, SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use super::client_api::Submitted;
use super::client_api::{GroupSignal, Handler, PeopleSearch, RelayClient};
use super::protocol::{
    CapabilityPresence, DutyDelivery, DutyState, InstancePresence, MemberRecord, PeerDelivery,
    PeerMessage, RelayMessage, ResealRequest, RoutePlan, RoutingQuery, TaskAssigned, TaskCompleted,
    TaskEvent, TaskFailed, TaskRefused, ALL_CAPABILITIES, PROTOCOL_VERSION, VERSION_MISMATCH,
};
use super::sealing::{self, MemoryKeyBook, Opening, PeerKeyBook, Sealer};
use super::{
    MemberKeyChanged, RelayAgentOffline, RelayAgentOnline, RelayConnectionState, RelaySealRefused,
    RelayStatus,
};
use crate::model::Timestamp;
use crate::services::a2a::{Task, TaskResult};
use crate::services::domain::AgentCard;
use crate::services::events::{self, EventBus};
use crate::services::gateway::{registration, GatewayCard};
use crate::services::network::peer_keys::PEER_KEY_CHANGED;
use nandi_protocol::{self as wire, PROBE_PAYLOAD};
use nandi_protocol::{GroupOp, GroupOutcome, PublicKeys, KEY_MISMATCH, NO_SHARED_GROUP};
use nandi_seal::DeviceKeys;

/// What a request to someone we share no group with (on a relay whose
/// visibility is `groups`) answers, after `NO_SHARED_GROUP`
pub fn no_shared_group() -> String {
    format!("{NO_SHARED_GROUP}: You don't share a group with them on this relay")
}

/// Leaves delivery `mail_id` unacked: it couldn't be opened yet, and the
/// next `InboxSync` brings it again
fn not_acked(mail_id: u64, why: &str) {
    log::info!("Relay: delivery {mail_id} waits for the next sync: {why}");
}

/// How often the client heartbeats
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// How long `register`, `query_capability`, and `delegate_task` wait
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest wait between reconnect attempts
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The wait before reconnect attempt `attempt` (0-based): 1s, 2s, 4s, 8s,
/// 16s, then [`MAX_BACKOFF`].
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.min(16);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

pub(crate) use nandi_protocol::now_ms;
pub use nandi_protocol::split_token;

pub type PresenceStore = Box<dyn Fn(&[AgentCard]) + Send + Sync>;

/// This device's keys (made on first use)
pub type DeviceKeysSource = Arc<dyn Fn() -> Result<DeviceKeys, String> + Send + Sync>;

/// Why a peer's keys aren't at hand
enum KeysError {
    /// The relay didn't answer for them (a timeout, a disconnect): worth
    /// asking again
    Unanswered(String),
    /// There are none to use: none published, no shared group, or a key
    /// change not accepted yet
    Refused(String),
}

impl From<KeysError> for String {
    fn from(error: KeysError) -> Self {
        match error {
            KeysError::Unanswered(e) | KeysError::Refused(e) => e,
        }
    }
}

/// One delivery from our mailbox, opened in order on the client's inbound
/// queue (mail ID 0: live only, not acked)
struct Inbound {
    mail_id: u64,
    item: wire::Delivery,
}

#[derive(Default)]
struct Handlers {
    assigned: Vec<Arc<Handler<TaskAssigned>>>,
    completed: Vec<Arc<Handler<TaskCompleted>>>,
    failed: Vec<Arc<Handler<TaskFailed>>>,
    peer: Vec<Arc<Handler<PeerDelivery>>>,
    events: Vec<Arc<Handler<TaskEvent>>>,
    refused: Vec<Arc<Handler<TaskRefused>>>,
    reseal: Vec<Arc<Handler<ResealRequest>>>,
    duty: Vec<Arc<Handler<DutyDelivery>>>,
    groups: Vec<Arc<Handler<GroupSignal>>>,
}

/// Calls waiting for the relay's answer, by key. Several may wait on one
/// key (the same query asked twice); the answer goes to each.
struct Pending<K, V> {
    waiting: HashMap<K, Vec<oneshot::Sender<Result<V, String>>>>,
}

impl<K, V> Default for Pending<K, V> {
    fn default() -> Self {
        Self {
            waiting: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash, V: Clone> Pending<K, V> {
    /// Parks a call waiting for `key`'s answer. Calls that gave up (timed
    /// out, or failed to send) are dropped here.
    fn insert(&mut self, key: K) -> oneshot::Receiver<Result<V, String>> {
        self.waiting.retain(|_, waiters| {
            waiters.retain(|waiter| !waiter.is_closed());
            !waiters.is_empty()
        });
        let (tx, rx) = oneshot::channel();
        self.waiting.entry(key).or_default().push(tx);
        rx
    }

    /// Answers every call waiting for `key`
    fn resolve(&mut self, key: &K, answer: Result<V, String>) {
        for waiter in self.waiting.remove(key).unwrap_or_default() {
            let _ = waiter.send(answer.clone());
        }
    }

    /// Fails every waiting call with `error`
    fn fail_all(&mut self, error: &str) {
        for (_, waiters) in self.waiting.drain() {
            for waiter in waiters {
                let _ = waiter.send(Err(error.to_string()));
            }
        }
    }
}

/// Waits up to [`REQUEST_TIMEOUT`] for the relay to answer what `rx` was
/// parked for. A call dropped unanswered means the connection went
/// ([`super::RelayUnavailable`]); a timeout says the relay didn't `what`.
async fn await_reply<V>(rx: oneshot::Receiver<Result<V, String>>, what: &str) -> Result<V, String> {
    match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err(super::RelayUnavailable.into()),
        Err(_) => Err(format!(
            "The relay didn't {what} within {}s",
            REQUEST_TIMEOUT.as_secs()
        )),
    }
}

#[derive(Default)]
struct Shared {
    instance_id: String,
    /// As configured (with the token)
    url: Option<String>,
    token: Option<String>,
    /// Bumped by every `connect`; a supervisor only touches state while
    /// its generation is current
    generation: u64,
    /// Dropping it cancels the current supervisor
    cancel: Option<watch::Sender<()>>,
    outbound: Option<mpsc::UnboundedSender<RelayMessage>>,
    /// The last cards registered; re-sent on reconnect
    cards: Option<Vec<AgentCard>>,
    load: HashMap<String, u32>,
    /// Every card at the relay, as of the last presence refresh
    online: Vec<AgentCard>,
    state: RelayConnectionState,
    /// Why `state` last changed
    reason: Option<String>,
    last_connected_at: Option<Timestamp>,
    last_heartbeat_at: Option<Timestamp>,
    waiting_registered: Pending<(), ()>,
    /// The relay confirmed our registration on this connection (queries
    /// before it are refused with `not_registered`)
    registered: bool,
    /// The relay refused our registration on this connection (a bad token,
    /// or another protocol version): not connected until a new one
    refused: bool,
    /// The refusal was for the protocol version
    version_mismatch: bool,
    /// The refusal was for our keys (the relay pinned others)
    key_mismatch: bool,
    /// This device's public keys, read on the first `connect`
    public_keys: Option<PublicKeys>,
    /// Key queries by instance ID
    waiting_keys: Pending<String, Option<PublicKeys>>,
    waiting_queries: Pending<String, Vec<AgentCard>>,
    /// Submissions by task ID: held (`Ok(true)`), with the provider, or
    /// the relay's refusal
    waiting_acks: Pending<String, bool>,
    /// `Reassign`s by task ID
    waiting_reassign: Pending<String, ()>,
    /// Mail handled on this connection (dropped when it comes again)
    handled_mail: HashSet<u64>,
    /// Our gateway card (wave final); re-sent on reconnect
    gateway: Option<GatewayCard>,
    waiting_directory: Pending<(), Vec<MemberRecord>>,
    /// People searches by `(topic, limit)`
    waiting_people: Pending<(String, u32), PeopleSearch>,
    /// Wave A: what we provide; carried by every `RegisterInstance`
    capabilities: Vec<CapabilityPresence>,
    tools: Vec<String>,
    waiting_presence: Pending<String, Vec<InstancePresence>>,
    /// Route queries by query ID
    waiting_routes: Pending<String, RoutePlan>,
    /// Group calls by request ID
    waiting_groups: Pending<String, GroupOutcome>,
    /// Who sees whom, from the relay's last `Registered`
    visibility: Option<super::RelayVisibility>,
    /// The relay's `no_shared_group` errors, by message: the refusal that
    /// follows each is told as [`no_shared_group`]
    no_shared_group: HashSet<String>,
    /// The relay's `daily_limit` errors, by message: the refusal that
    /// follows each is told as [`super::limits::daily_limit`]
    daily_limit: HashSet<String>,
}

impl Shared {
    /// Fails every call waiting for an answer (not those waiting for the
    /// registration) with `error`
    fn fail_calls(&mut self, error: &str) {
        self.waiting_keys.fail_all(error);
        self.waiting_queries.fail_all(error);
        self.waiting_acks.fail_all(error);
        self.waiting_reassign.fail_all(error);
        self.waiting_directory.fail_all(error);
        self.waiting_people.fail_all(error);
        self.waiting_presence.fail_all(error);
        self.waiting_routes.fail_all(error);
        self.waiting_groups.fail_all(error);
    }

    /// A refusal's words as the app tells them: the coded ones the relay
    /// announced in an `Error` just before
    fn retell(&mut self, message: &str) -> String {
        if self.no_shared_group.remove(message) {
            no_shared_group()
        } else if self.daily_limit.remove(message) {
            super::limits::daily_limit(message)
        } else {
            message.to_string()
        }
    }
}

struct Inner {
    shared: Mutex<Shared>,
    handlers: Mutex<Handlers>,
    bus: Option<EventBus>,
    heartbeat_every: Duration,
    presence_store: Option<PresenceStore>,
    device_keys: Option<DeviceKeysSource>,
    peer_keys: Arc<dyn PeerKeyBook>,
    offered: OfferedRelays,
    /// Seals and opens with this device's keys; made on the first connect
    sealer: Mutex<Option<Arc<Sealer>>>,
    /// Feeds the opener task (started on the first connect)
    inbound: Mutex<Option<mpsc::UnboundedSender<Inbound>>>,
}

/// The tokio-tungstenite [`RelayClient`].
pub struct WsRelayClient {
    inner: Arc<Inner>,
}

/// How a [`WsRelayClient`] behaves; the default heartbeats every 30s and
/// stores nothing.
pub struct ClientOptions {
    pub heartbeat_every: Duration,
    /// Called with every card at the relay whenever presence changes
    pub presence_store: Option<PresenceStore>,
    /// This device's keys: what it seals and opens with, and the public
    /// half every registration carries. `None`: a new pair for this
    /// client's lifetime (tests)
    pub device_keys: Option<DeviceKeysSource>,
    /// Other members' pinned keys; `None`: pinned in memory only (tests)
    pub peer_keys: Option<Arc<dyn PeerKeyBook>>,
    /// The relays the app offers, carried by every status it emits
    pub offered: OfferedRelays,
}

/// The relays the app offers whether or not one is in use: the build's own
/// and the public relay ([`super::BuiltInRelay`])
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfferedRelays {
    pub built_in: Option<String>,
    pub public: Option<String>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            heartbeat_every: HEARTBEAT_INTERVAL,
            presence_store: None,
            device_keys: None,
            peer_keys: None,
            offered: OfferedRelays::default(),
        }
    }
}

impl WsRelayClient {
    /// A disconnected client for `instance_id`; `bus` receives `relay://`
    /// events.
    pub fn new(bus: Option<EventBus>, instance_id: impl Into<String>) -> Self {
        Self::with_options(bus, instance_id, ClientOptions::default())
    }

    pub fn with_options(
        bus: Option<EventBus>,
        instance_id: impl Into<String>,
        options: ClientOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                shared: Mutex::new(Shared {
                    instance_id: instance_id.into(),
                    ..Shared::default()
                }),
                handlers: Mutex::new(Handlers::default()),
                bus,
                heartbeat_every: options.heartbeat_every,
                presence_store: options.presence_store,
                device_keys: options.device_keys,
                peer_keys: options
                    .peer_keys
                    .unwrap_or_else(|| Arc::new(MemoryKeyBook::default())),
                offered: options.offered,
                sealer: Mutex::default(),
                inbound: Mutex::default(),
            }),
        }
    }

    /// Every card at the relay, as of the last presence refresh
    pub fn online_agents(&self) -> Vec<AgentCard> {
        self.inner.lock().online.clone()
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn handlers(&self) -> MutexGuard<'_, Handlers> {
        self.handlers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn status(&self) -> RelayStatus {
        let shared = self.lock();
        let instances: HashSet<&str> = shared
            .online
            .iter()
            .map(|card| card.instance_id.as_str())
            .collect();
        RelayStatus {
            connected: shared.outbound.is_some() && !shared.refused,
            version_mismatch: shared.version_mismatch,
            key_mismatch: shared.key_mismatch,
            state: shared.state,
            reason: shared.reason.clone(),
            instance_id: shared.instance_id.clone(),
            relay_url: shared.url.clone(),
            // Filled in by `get_relay_status`
            relay_url_source: None,
            built_in_url: self.offered.built_in.clone(),
            public_relay_url: self.offered.public.clone(),
            agents_online: u32::try_from(
                shared
                    .online
                    .iter()
                    .filter(|card| !registration::is_route_card(card))
                    .count(),
            )
            .unwrap_or(u32::MAX),
            instances_online: u32::try_from(instances.len()).unwrap_or(u32::MAX),
            last_connected_at: shared.last_connected_at,
            last_heartbeat_at: shared.last_heartbeat_at,
            visibility: shared.visibility,
        }
    }

    /// Moves the connection of `generation` to `state` and emits
    /// `relay://status`; ignored once a newer `connect` took over.
    fn set_state(&self, generation: u64, state: RelayConnectionState, reason: Option<String>) {
        {
            let mut shared = self.lock();
            if shared.generation != generation {
                return;
            }
            shared.state = state;
            shared.reason = reason;
        }
        self.emit_status();
    }

    fn emit_status(&self) {
        if let Some(bus) = &self.bus {
            events::emit(bus, &self.status().without_token());
        }
    }

    fn send(&self, message: RelayMessage) -> Result<(), String> {
        let shared = self.lock();
        let outbound = shared
            .outbound
            .as_ref()
            .ok_or_else(|| String::from(super::RelayUnavailable))?;
        outbound
            .send(message)
            .map_err(|_| String::from(super::RelayUnavailable))
    }

    fn token(&self) -> String {
        self.lock().token.clone().unwrap_or_default()
    }

    /// Reads this device's keys once (for `instance_id`), for sealing and
    /// registrations
    fn load_keys(&self, instance_id: &str) {
        let mut sealer = self.sealer.lock().unwrap_or_else(|e| e.into_inner());
        if sealer.as_ref().is_some_and(|s| s.me() == instance_id) {
            return;
        }
        let keys = match &self.device_keys {
            Some(source) => source(),
            None => DeviceKeys::generate().map_err(|e| e.to_string()),
        };
        match keys {
            Ok(keys) => {
                let made = Arc::new(Sealer::new(instance_id, keys));
                self.lock().public_keys = Some(made.public_keys().clone());
                *sealer = Some(made);
            }
            Err(e) => log::warn!("Registering without this device's keys: {e}"),
        }
    }

    /// The sealer; none before the first connect (so: not connected)
    fn sealer(&self) -> Result<Arc<Sealer>, String> {
        self.sealer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| String::from(super::RelayUnavailable))
    }

    /// Whether we may send to `instance` at all: not while their key
    /// changed and the user hasn't accepted it
    fn may_send_to(&self, instance: &str) -> Result<(), String> {
        self.peer_keys.pinned(instance).map(|_| ())
    }

    /// `instance`'s keys to seal for or check: the pinned ones, else the
    /// relay's, pinned now
    async fn keys_for(&self, instance: &str) -> Result<PublicKeys, String> {
        Ok(self.lookup_keys(instance).await?)
    }

    /// [`Self::keys_for`], telling a relay that didn't answer apart from a
    /// definite refusal
    async fn lookup_keys(&self, instance: &str) -> Result<PublicKeys, KeysError> {
        if let Some(keys) = self
            .peer_keys
            .pinned(instance)
            .map_err(KeysError::Refused)?
        {
            return Ok(keys);
        }
        let queried = self
            .query_keys(instance)
            .await
            .map_err(KeysError::Unanswered)?;
        match queried {
            Some(keys) => self.learn(instance, &keys).map_err(KeysError::Refused),
            // A groups relay hides the keys of people we share no group with
            None if self.lock().visibility == Some(super::RelayVisibility::Groups) => {
                Err(KeysError::Refused(no_shared_group()))
            }
            None => Err(KeysError::Refused(sealing::missing(instance))),
        }
    }

    /// Pins (or checks) keys the relay lists for `instance`; a change is
    /// told to the profile
    fn learn(&self, instance: &str, keys: &PublicKeys) -> Result<PublicKeys, String> {
        let learned = self.peer_keys.learn(instance, keys);
        if learned
            .as_ref()
            .is_err_and(|e| e.starts_with(PEER_KEY_CHANGED))
        {
            log::warn!("{instance}'s key changed; sending to them waits until it's accepted");
            if let Some(bus) = &self.bus {
                events::emit(
                    bus,
                    &MemberKeyChanged {
                        instance_id: instance.to_string(),
                    },
                );
            }
        }
        learned
    }

    /// The keys the relay pinned for `instance`, or `None`
    async fn query_keys(&self, instance: &str) -> Result<Option<PublicKeys>, String> {
        self.until_registered().await?;
        let rx = self.lock().waiting_keys.insert(instance.to_string());
        self.send(RelayMessage::PublicKeysQuery {
            instance_ids: vec![instance.to_string()],
        })?;
        await_reply(rx, &format!("answer for {instance}'s keys")).await
    }

    /// Starts the opener task, once
    fn start_inbound(self: &Arc<Self>) {
        let mut inbound = self.inbound.lock().unwrap_or_else(|e| e.into_inner());
        if inbound.is_some() {
            return;
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();
        *inbound = Some(tx);
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(item) = rx.recv().await {
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                inner.open_inbound(item).await;
            }
        });
    }

    fn queue_inbound(&self, item: Inbound) {
        let sent = self
            .inbound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|tx| tx.send(item).is_ok());
        if !sent {
            log::warn!("Relay: dropping a frame that arrived before the client started");
        }
    }

    /// Opens one delivery, hands it to the handlers and acks it once it's
    /// settled: opened, a duplicate, or refused for good. One with no
    /// handler yet, or whose author's keys the relay didn't answer for,
    /// isn't acked: the next `InboxSync` brings it again.
    async fn open_inbound(&self, inbound: Inbound) {
        let sealer = match self.sealer() {
            Ok(sealer) => sealer,
            Err(e) => return log::warn!("Relay: can't open a delivery: {e}"),
        };
        let Inbound { mail_id, item } = inbound;
        let me = self.lock().instance_id.clone();
        let handled = match item {
            wire::Delivery::Task { task: sealed } => {
                let (task_id, requester) =
                    (sealed.id.clone(), sealed.requester.instance_id.clone());
                let handlers = self.handlers().assigned.clone();
                if !handlers.is_empty() {
                    let opened = match self.author_keys(&requester, &task_id).await {
                        Ok(keys) => sealer.open_task(sealed, &keys),
                        Err(KeysError::Refused(e)) => Err(e),
                        Err(KeysError::Unanswered(e)) => return not_acked(mail_id, &e),
                    };
                    if let Some(task) = self.opened(opened, &requester, &task_id, false).await {
                        let assigned = TaskAssigned {
                            task_id,
                            provider_instance: me,
                            task,
                        };
                        for handler in handlers.iter() {
                            handler(assigned.clone());
                        }
                    }
                }
                !handlers.is_empty()
            }
            wire::Delivery::Result { result } => {
                let task_id = result.task_id.clone();
                let provider = result
                    .provider_agent
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let handlers = self.handlers().completed.clone();
                if !handlers.is_empty() {
                    let opened = match self.author_keys(&provider, &task_id).await {
                        Ok(keys) => sealer.open_result(result, (&provider, &keys)),
                        Err(KeysError::Refused(e)) => Err(e),
                        Err(KeysError::Unanswered(e)) => return not_acked(mail_id, &e),
                    };
                    if let Some(result) = self.opened(opened, &provider, &task_id, true).await {
                        let completed = TaskCompleted { task_id, result };
                        for handler in handlers.iter() {
                            handler(completed.clone());
                        }
                    }
                }
                !handlers.is_empty()
            }
            wire::Delivery::Peer {
                from_instance,
                body,
            } => {
                let handlers = self.handlers().peer.clone();
                if !handlers.is_empty() {
                    let task_id = body.task_id().to_string();
                    let opened = if body.envelopes().is_empty() {
                        sealer.open_peer(&from_instance, body, None)
                    } else {
                        match self.author_keys(&from_instance, &task_id).await {
                            Ok(keys) => sealer.open_peer(&from_instance, body, Some(&keys)),
                            Err(KeysError::Refused(e)) => Err(e),
                            Err(KeysError::Unanswered(e)) => return not_acked(mail_id, &e),
                        }
                    };
                    if let Some(body) = self.opened(opened, &from_instance, &task_id, true).await {
                        let delivery = PeerDelivery {
                            from_instance,
                            target_instance: me,
                            body,
                        };
                        for handler in handlers.iter() {
                            handler(delivery.clone());
                        }
                    }
                }
                !handlers.is_empty()
            }
            wire::Delivery::Event { event } => {
                let handlers = self.handlers().events.clone();
                if !handlers.is_empty() {
                    let event = match self.open_event(&sealer, event).await {
                        Ok(event) => event,
                        Err(e) => return not_acked(mail_id, &e),
                    };
                    for handler in handlers.iter() {
                        handler(event.clone());
                    }
                }
                !handlers.is_empty()
            }
            wire::Delivery::Reseal {
                task_id,
                target,
                by,
            } => {
                let handlers = self.handlers().reseal.clone();
                let request = ResealRequest {
                    task_id,
                    target,
                    by,
                };
                for handler in handlers.iter() {
                    handler(request.clone());
                }
                !handlers.is_empty()
            }
            wire::Delivery::DutyQuery {
                duty_id,
                backup_instance,
            } => {
                let handlers = self.handlers().duty.clone();
                let query = DutyDelivery::Query {
                    duty_id,
                    backup_instance,
                };
                for handler in handlers.iter() {
                    handler(query.clone());
                }
                !handlers.is_empty()
            }
            wire::Delivery::Duty {
                duty_id,
                decision,
                envelope,
            } => {
                let handlers = self.handlers().duty.clone();
                if !handlers.is_empty() {
                    let error = match &envelope {
                        Some(envelope) => {
                            let primary = envelope.sealed_for.from.clone();
                            let opened = match self.author_keys(&primary, &duty_id).await {
                                Ok(keys) => {
                                    sealer.open_note(Some(envelope), &duty_id, (&primary, &keys))
                                }
                                Err(KeysError::Refused(e)) => Err(e),
                                Err(KeysError::Unanswered(e)) => return not_acked(mail_id, &e),
                            };
                            self.opened(opened, &primary, &duty_id, false)
                                .await
                                .flatten()
                        }
                        None => None,
                    };
                    let decision = DutyDelivery::Decision {
                        duty_id,
                        decision,
                        error,
                    };
                    for handler in handlers.iter() {
                        handler(decision.clone());
                    }
                }
                !handlers.is_empty()
            }
        };
        if handled && mail_id != 0 {
            self.lock().handled_mail.insert(mail_id);
            let _ = self.send(RelayMessage::Ack {
                mail_ids: vec![mail_id],
            });
        }
    }

    /// A task event, its note opened (a reassignment's reason, sealed by
    /// whoever moved it), or why the note can't be checked yet (the relay
    /// didn't answer for its author's keys)
    async fn open_event(
        &self,
        sealer: &Sealer,
        event: wire::TaskEvent,
    ) -> Result<TaskEvent, String> {
        Ok(match event {
            wire::TaskEvent::Accepted {
                task_id,
                provider_instance,
                target,
                held,
            } => TaskEvent::Accepted {
                task_id,
                provider_instance,
                target,
                held,
            },
            wire::TaskEvent::Refused { task_id, message } => {
                TaskEvent::Refused { task_id, message }
            }
            wire::TaskEvent::FailedOver {
                task_id,
                from_instance,
                provider_instance,
                target,
                cause,
                reason_kind,
            } => TaskEvent::FailedOver {
                task_id,
                from_instance,
                provider_instance,
                target,
                cause,
                reason_kind,
            },
            wire::TaskEvent::Reassigned {
                task_id,
                from_instance,
                provider_instance,
                target,
                provider_name,
                by,
                envelope,
            } => {
                let note = match envelope {
                    Some(envelope) => {
                        let opened = match self.author_keys(&from_instance, &task_id).await {
                            Ok(keys) => {
                                sealer.open_note(Some(&envelope), &task_id, (&from_instance, &keys))
                            }
                            Err(KeysError::Refused(e)) => Err(e),
                            Err(KeysError::Unanswered(e)) => return Err(e),
                        };
                        // A note that doesn't open is left out; the move stands
                        self.opened(opened, &from_instance, &task_id, false)
                            .await
                            .flatten()
                    }
                    None => None,
                };
                TaskEvent::Reassigned {
                    task_id,
                    from_instance,
                    provider_instance,
                    target,
                    provider_name,
                    by,
                    note,
                }
            }
            wire::TaskEvent::Ended {
                task_id,
                state,
                reason_kind,
            } => TaskEvent::Ended {
                task_id,
                state,
                reason_kind,
            },
        })
    }

    /// The keys to check what `author` sent about `task_id` against; a
    /// refusal is worded as one of their message
    async fn author_keys(&self, author: &str, task_id: &str) -> Result<PublicKeys, KeysError> {
        self.lookup_keys(author).await.map_err(|error| match error {
            KeysError::Refused(e) => KeysError::Refused(sealing::refused(
                author,
                task_id,
                &format!("of their keys: {e}"),
            )),
            unanswered => unanswered,
        })
    }

    /// What opening came to: the value, or `None` after a duplicate was
    /// dropped or a refusal was told (failing the task, when `fails_task`
    /// and it's ours)
    async fn opened<T>(
        &self,
        opening: Result<Opening<T>, String>,
        from: &str,
        task_id: &str,
        fails_task: bool,
    ) -> Option<T> {
        match opening {
            Ok(Opening::Opened(value)) => Some(value),
            Ok(Opening::Duplicate) => {
                log::debug!("Relay: dropped a duplicate from {from} about {task_id}");
                None
            }
            Err(error) => {
                log::warn!("Relay: {error}");
                // Their key may have changed: noted for the profile
                if !error.contains(PEER_KEY_CHANGED) {
                    if let Ok(Some(keys)) = self.query_keys(from).await {
                        let _ = self.learn(from, &keys);
                    }
                }
                if let Some(bus) = &self.bus {
                    events::emit(
                        bus,
                        &RelaySealRefused {
                            from_instance: from.to_string(),
                            task_id: task_id.to_string(),
                            error: error.clone(),
                        },
                    );
                }
                if fails_task {
                    let failed = TaskFailed {
                        task_id: task_id.to_string(),
                        error,
                    };
                    for handler in self.handlers().failed.clone() {
                        handler(failed.clone());
                    }
                }
                None
            }
        }
    }

    /// Sends `SubmitTask` for `task`, its input sealed for its target and
    /// each of `alternates` (an alternate we can't seal for is left out),
    /// and waits for the relay to take it
    async fn submit(&self, task: &Task, alternates: &[String]) -> Result<Submitted, String> {
        // A fresh connection takes nothing before it's registered (an
        // outbox flushed on connect would be refused otherwise)
        self.until_registered().await?;
        let sealer = self.sealer()?;
        let mut attempts = Vec::new();
        for (n, target) in std::iter::once(&task.target).chain(alternates).enumerate() {
            let provider = target.split('/').next().unwrap_or_default();
            let sealed = match self.keys_for(provider).await {
                Ok(keys) => sealer.seal_task(task, (provider, &keys)),
                Err(e) => Err(e),
            };
            match sealed {
                Ok(envelope) => attempts.push(wire::TaskAttempt {
                    target: target.clone(),
                    envelope,
                }),
                // The first is the one asked for: its refusal is the answer
                Err(e) if n == 0 => return Err(e),
                Err(e) => log::info!("Task {}: {target} left out of the plan: {e}", task.id),
            }
        }
        let rx = self.lock().waiting_acks.insert(task.id.clone());
        let message = RelayMessage::SubmitTask {
            task_id: task.id.clone(),
            kind: crate::services::a2a::route::kind_of(&task.capability),
            capability: task.capability.clone(),
            requester_card: task.requester.clone(),
            deadline: task.deadline,
            signature: self.token(),
            attempts,
        };
        self.send(message)?;
        let held = await_reply(rx, "take the task").await?;
        Ok(Submitted { held })
    }

    /// Sends `Reassign` and waits for the relay's answer
    async fn reassign_at_relay(&self, message: RelayMessage, task_id: &str) -> Result<(), String> {
        let rx = self.lock().waiting_reassign.insert(task_id.to_string());
        self.send(message)?;
        await_reply(rx, "answer").await
    }

    /// Asks for what waits in our mailbox, when registered
    fn sync_inbox(&self) {
        let shared = self.lock();
        if let (true, Some(outbound)) = (shared.registered, shared.outbound.as_ref()) {
            let _ = outbound.send(RelayMessage::InboxSync {});
        }
    }

    /// `RegisterInstance` with `cards`, plus our gateway's route card while
    /// it accepts delegation (relays route tasks to agent cards only)
    fn register_message(shared: &Shared, mut cards: Vec<AgentCard>) -> RelayMessage {
        if let Some(gateway) = shared.gateway.as_ref().filter(|g| g.accepts_delegation) {
            let route = registration::route_card(gateway);
            if !cards.iter().any(|card| card.id == route.id) {
                cards.push(route);
            }
        }
        RelayMessage::RegisterInstance {
            instance_id: shared.instance_id.clone(),
            agent_cards: cards,
            signature: shared.token.clone().unwrap_or_default(),
            capabilities: shared.capabilities.clone(),
            tools: shared.tools.clone(),
            protocol_version: PROTOCOL_VERSION,
            public_keys: shared.public_keys.clone(),
        }
    }

    /// A supervisor of `generation` opened its socket
    fn on_connected(&self, generation: u64, outbound: mpsc::UnboundedSender<RelayMessage>) -> bool {
        {
            let mut shared = self.lock();
            if shared.generation != generation {
                return false;
            }
            let now = now_ms();
            // Re-register the last cards, ahead of anything else, then
            // heartbeat so the relay sees this instance alive right away
            if let Some(cards) = shared.cards.clone() {
                let _ = outbound.send(Self::register_message(&shared, cards));
                let _ = outbound.send(RelayMessage::Heartbeat {
                    instance_id: shared.instance_id.clone(),
                    load: shared.load.clone(),
                    ts: now,
                });
                shared.last_heartbeat_at = Some(now);
                // The gateway card follows the relay's `Registered`
            }
            shared.outbound = Some(outbound);
            shared.registered = false;
            shared.handled_mail.clear();
            shared.refused = false;
            shared.version_mismatch = false;
            shared.key_mismatch = false;
            shared.state = RelayConnectionState::Connected;
            shared.reason = None;
            shared.last_connected_at = Some(now);
        }
        self.emit_status();
        true
    }

    /// The socket of `generation` closed: fail what waits on it
    fn on_disconnected(&self, generation: u64) {
        {
            let mut shared = self.lock();
            if shared.generation != generation || shared.outbound.is_none() {
                return;
            }
            shared.outbound = None;
            shared.registered = false;
            shared.state = RelayConnectionState::Reconnecting;
            shared.reason = Some("Lost the relay connection".to_string());
            let unavailable = super::RelayUnavailable.to_string();
            shared.waiting_registered.fail_all(&unavailable);
            shared.fail_calls(&unavailable);
        }
        self.emit_status();
    }

    fn handle_incoming(self: &Arc<Self>, text: &str) {
        let message = match RelayMessage::from_json(text) {
            Ok(message) => message,
            Err(e) => return log::warn!("Relay: {e}"),
        };
        match message {
            RelayMessage::Registered { visibility, .. } => {
                {
                    let mut shared = self.lock();
                    shared.registered = true;
                    shared.visibility = Some(visibility.into());
                    // The relay takes a gateway card only from a registered
                    // instance: ours goes now
                    if let (Some(card), Some(outbound)) =
                        (shared.gateway.clone(), shared.outbound.as_ref())
                    {
                        let _ = outbound.send(RelayMessage::RegisterGateway {
                            instance_id: shared.instance_id.clone(),
                            card,
                        });
                    }
                    // Then whatever waited for us
                    if let Some(outbound) = shared.outbound.as_ref() {
                        let _ = outbound.send(RelayMessage::InboxSync {});
                    }
                    shared.waiting_registered.resolve(&(), Ok(()));
                }
                // The visibility may have changed
                self.emit_status();
                self.tell_groups(GroupSignal::Registered);
                tokio::spawn(self.clone().refresh_presence());
            }
            RelayMessage::GroupReply {
                request_id,
                outcome,
            } => {
                self.lock().waiting_groups.resolve(&request_id, Ok(outcome));
            }
            RelayMessage::GroupChanged { group_id, change } => {
                self.tell_groups(GroupSignal::Changed { group_id, change });
            }
            RelayMessage::CapabilityResult { capability, agents } => {
                self.lock().waiting_queries.resolve(&capability, Ok(agents));
            }
            RelayMessage::Deliver { mail_id, item } => {
                if mail_id != 0 && self.lock().handled_mail.contains(&mail_id) {
                    // Handled already on this connection: the ack went astray
                    let _ = self.send(RelayMessage::Ack {
                        mail_ids: vec![mail_id],
                    });
                    return;
                }
                // The relay's answer to a submission
                if let wire::Delivery::Event { event } = &item {
                    let answer = match event {
                        wire::TaskEvent::Accepted { task_id, held, .. } => {
                            Some((task_id.clone(), Ok(*held)))
                        }
                        wire::TaskEvent::Refused { task_id, message } => {
                            let said = self.lock().retell(message);
                            Some((
                                task_id.clone(),
                                Err(format!("{}{said}", super::client_api::REFUSED_BY_RELAY)),
                            ))
                        }
                        _ => None,
                    };
                    if let Some((task_id, answer)) = answer {
                        self.lock().waiting_acks.resolve(&task_id, answer);
                    }
                }
                self.queue_inbound(Inbound { mail_id, item });
            }
            RelayMessage::ReassignResult { task_id, error } => {
                let mut shared = self.lock();
                let error = error.map(|error| shared.retell(&error));
                shared
                    .waiting_reassign
                    .resolve(&task_id, error.map_or(Ok(()), Err));
            }
            RelayMessage::PublicKeysResult { keys } => {
                let mut shared = self.lock();
                for found in keys {
                    shared
                        .waiting_keys
                        .resolve(&found.instance_id, Ok(found.public_keys));
                }
            }
            RelayMessage::Error {
                code,
                message,
                task_id,
            } => {
                log::warn!("Relay error ({code}): {message}");
                if let Some(task_id) = task_id {
                    let refused = TaskRefused {
                        task_id,
                        code: code.clone(),
                        message: message.clone(),
                    };
                    for handler in self.handlers().refused.clone() {
                        handler(refused.clone());
                    }
                }
                let mut shared = self.lock();
                if code == NO_SHARED_GROUP {
                    // A refusal with these words follows
                    shared.no_shared_group.insert(message);
                    return;
                }
                if code == super::limits::DAILY_LIMIT {
                    // A refusal with these words follows a request or a
                    // reassignment; a clarification gets none, so the set
                    // is kept small
                    if shared.daily_limit.len() >= 32 {
                        shared.daily_limit.clear();
                    }
                    shared.daily_limit.insert(message);
                    return;
                }
                if code == "unauthorized" || code == VERSION_MISMATCH || code == KEY_MISMATCH {
                    let refused = format!("The relay refused us: {message}");
                    shared.waiting_registered.fail_all(&refused);
                    shared.refused = true;
                    shared.version_mismatch = code == VERSION_MISMATCH;
                    shared.key_mismatch = code == KEY_MISMATCH;
                    shared.state = RelayConnectionState::Error;
                    shared.reason = Some(refused);
                    drop(shared);
                    return self.emit_status();
                }
                if code == "not_registered" {
                    // Fails them now instead of at the timeout
                    shared.fail_calls(&super::RelayUnavailable.to_string());
                }
            }
            RelayMessage::Directory { members } => {
                self.lock().waiting_directory.resolve(&(), Ok(members));
            }
            RelayMessage::PeopleMatches {
                topic,
                limit,
                matches,
                searched,
                unstated,
                total,
            } => {
                let found = PeopleSearch {
                    matches,
                    searched,
                    unstated,
                    total,
                };
                self.lock()
                    .waiting_people
                    .resolve(&(topic, limit), Ok(found));
            }
            RelayMessage::CapabilityPresenceResponse {
                capability_id,
                providers,
            } => {
                self.lock()
                    .waiting_presence
                    .resolve(&capability_id, Ok(providers));
            }
            RelayMessage::RoutePlan { query_id, plan } => {
                self.lock().waiting_routes.resolve(&query_id, Ok(plan));
            }
            other => log::debug!("Relay: ignoring {other:?}"),
        }
    }

    /// Hands group news to every group handler
    fn tell_groups(&self, signal: GroupSignal) {
        for handler in self.handlers().groups.clone() {
            handler(signal.clone());
        }
    }

    /// One `GroupRequest`, waiting for its `GroupReply`
    async fn group_call(&self, op: GroupOp) -> Result<GroupOutcome, String> {
        self.until_registered().await?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let rx = self.lock().waiting_groups.insert(request_id.clone());
        self.send(RelayMessage::GroupRequest { request_id, op })?;
        await_reply(rx, "answer").await
    }

    /// Waits (up to [`REQUEST_TIMEOUT`]) until the relay confirms our
    /// registration: it refuses queries before that, and on a fresh
    /// connection presence and the network directory ask right away
    async fn until_registered(&self) -> Result<(), String> {
        let rx = {
            let mut shared = self.lock();
            if shared.registered {
                return Ok(());
            }
            if shared.outbound.is_none() || shared.refused {
                return Err(super::RelayUnavailable.into());
            }
            shared.waiting_registered.insert(())
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            // Dropped, or connected but still not registered: as good as
            // not connected
            Ok(Err(_)) | Err(_) => Err(super::RelayUnavailable.into()),
        }
    }

    async fn query(&self, capability: &str) -> Result<Vec<AgentCard>, String> {
        self.until_registered().await?;
        let (rx, message) = {
            let mut shared = self.lock();
            let rx = shared.waiting_queries.insert(capability.to_string());
            let message = RelayMessage::QueryCapability {
                capability: capability.to_string(),
                requester_instance: shared.instance_id.clone(),
            };
            (rx, message)
        };
        self.send(message)?;
        await_reply(rx, "answer").await
    }

    async fn refresh_presence(self: Arc<Self>) {
        match self.query(ALL_CAPABILITIES).await {
            Ok(cards) => self.update_presence(cards),
            Err(e) => log::debug!("Relay presence refresh failed: {e}"),
        }
    }

    fn update_presence(&self, cards: Vec<AgentCard>) {
        let (came, went) = {
            let mut shared = self.lock();
            let before: HashSet<&str> = shared.online.iter().map(|c| c.id.as_str()).collect();
            let after: HashSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();
            let came: Vec<AgentCard> = cards
                .iter()
                .filter(|c| !before.contains(c.id.as_str()))
                .cloned()
                .collect();
            let went: Vec<RelayAgentOffline> = shared
                .online
                .iter()
                .filter(|c| !after.contains(c.id.as_str()))
                .map(|c| RelayAgentOffline {
                    card_id: c.id.clone(),
                    instance_id: c.instance_id.clone(),
                })
                .collect();
            shared.online = cards.clone();
            (came, went)
        };
        if came.is_empty() && went.is_empty() {
            return;
        }
        if let Some(bus) = &self.bus {
            for card in came {
                events::emit(bus, &RelayAgentOnline { card });
            }
            for offline in &went {
                events::emit(bus, offline);
            }
        }
        if let Some(store) = &self.presence_store {
            store(&cards);
        }
        self.emit_status();
    }

    /// The next heartbeat; records it as sent
    fn heartbeat_message(&self) -> RelayMessage {
        let mut shared = self.lock();
        shared.last_heartbeat_at = Some(now_ms());
        RelayMessage::Heartbeat {
            instance_id: shared.instance_id.clone(),
            load: shared.load.clone(),
            ts: now_ms(),
        }
    }
}

/// Installs aws-lc-rs as rustls' process-wide crypto provider, once.
/// Dependencies enable both ring and aws-lc-rs, so without this a `wss://`
/// connect panics choosing between them. Another crate installing one
/// first is fine.
pub(crate) fn ensure_tls_provider() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// Opens a WebSocket to `url` (without its token), sends a ping, and waits
/// for the pong; returns the round trip (handshake included) in
/// milliseconds, or what went wrong. Opening waits up to `connect_timeout`
/// (long enough for Windows to report a refused port, ~2s), the pong up to
/// `timeout`.
pub async fn probe(url: &str, connect_timeout: Duration, timeout: Duration) -> Result<u32, String> {
    let (dial, _) = split_token(url.trim());
    let secs = timeout.as_secs_f32();
    let connect_secs = connect_timeout.as_secs_f32();
    ensure_tls_provider();
    let started = std::time::Instant::now();
    let mut socket = match tokio::time::timeout(
        connect_timeout,
        tokio_tungstenite::connect_async_tls_with_config(
            dial.as_str(),
            None,
            false,
            Some(super::tls::connector()),
        ),
    )
    .await
    {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(e)) => return Err(format!("Couldn't open a WebSocket to {dial}: {e}")),
        Err(_) => {
            return Err(format!(
                "Timed out after {connect_secs}s opening a WebSocket to {dial}"
            ))
        }
    };
    let payload = PROBE_PAYLOAD;
    if let Err(e) = socket.send(Message::Ping(payload.to_vec().into())).await {
        return Err(format!(
            "Connected to {dial}, but sending a ping failed: {e}"
        ));
    }
    let closed = || format!("The relay at {dial} closed the connection before answering the ping");
    let answered = tokio::time::timeout(timeout, async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Pong(data)) if data.as_ref() == payload => return Ok(()),
                Ok(Message::Close(_)) => return Err(closed()),
                Ok(_) => {}
                Err(e) => {
                    return Err(format!(
                        "The connection to {dial} failed while waiting for the pong: {e}"
                    ))
                }
            }
        }
        Err(closed())
    })
    .await;
    let round_trip = started.elapsed();
    let _ = socket.close(None).await;
    match answered {
        Ok(Ok(())) => Ok(u32::try_from(round_trip.as_millis()).unwrap_or(u32::MAX)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!(
            "Connected to {dial}, but the relay didn't answer a ping within {secs}s"
        )),
    }
}

/// Keeps one relay connection up until cancelled.
async fn supervise(
    inner: Arc<Inner>,
    url: String,
    generation: u64,
    mut cancel: watch::Receiver<()>,
    first: oneshot::Sender<Result<(), String>>,
) {
    let mut first = Some(first);
    let mut attempt: u32 = 0;
    ensure_tls_provider();
    loop {
        let dialed = tokio::select! {
            dialed = tokio_tungstenite::connect_async_tls_with_config(
                url.as_str(),
                None,
                false,
                Some(super::tls::connector()),
            ) => dialed,
            _ = cancel.changed() => return,
        };
        let why = match dialed {
            Ok((socket, _)) => {
                attempt = 0;
                let (outbound, queued) = mpsc::unbounded_channel();
                if !inner.on_connected(generation, outbound) {
                    return;
                }
                if let Some(first) = first.take() {
                    let _ = first.send(Ok(()));
                }
                let cancelled = run_socket(&inner, socket, queued, &mut cancel).await;
                inner.on_disconnected(generation);
                if cancelled {
                    return;
                }
                log::warn!("Lost the relay connection; reconnecting");
                "Lost the relay connection".to_string()
            }
            Err(e) => {
                let error = format!("Failed to connect to the relay at {url}: {e}");
                log::debug!("{error}");
                if let Some(first) = first.take() {
                    let _ = first.send(Err(error.clone()));
                }
                error
            }
        };
        let delay = backoff_delay(attempt);
        attempt = attempt.saturating_add(1);
        inner.set_state(
            generation,
            RelayConnectionState::Reconnecting,
            Some(format!("{why}; retrying in {}s", delay.as_secs())),
        );
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            _ = cancel.changed() => return,
        }
    }
}

/// Pumps one socket; `true` when cancelled, `false` when it dropped.
async fn run_socket<S>(
    inner: &Arc<Inner>,
    socket: tokio_tungstenite::WebSocketStream<S>,
    mut queued: mpsc::UnboundedReceiver<RelayMessage>,
    cancel: &mut watch::Receiver<()>,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut stream) = socket.split();
    let mut heartbeat = tokio::time::interval(inner.heartbeat_every);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick is immediate; registration already counts as one
    heartbeat.tick().await;
    loop {
        tokio::select! {
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => inner.handle_incoming(text.as_str()),
                Some(Ok(Message::Close(_)) | Err(_)) | None => return false,
                Some(Ok(_)) => {}
            },
            message = queued.recv() => {
                let Some(message) = message else { return false };
                let Ok(text) = message.to_json() else { continue };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return false;
                }
            }
            _ = heartbeat.tick() => {
                // Refused: nothing to keep alive until a new connection
                if inner.lock().refused {
                    continue;
                }
                let Ok(text) = inner.heartbeat_message().to_json() else { continue };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return false;
                }
                inner.emit_status();
                tokio::spawn(inner.clone().refresh_presence());
            }
            _ = cancel.changed() => {
                let _ = sink.close().await;
                return true;
            }
        }
    }
}

impl RelayClient for WsRelayClient {
    fn connect<'a>(
        &'a self,
        url: &'a str,
        instance_id: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            let url = url.trim();
            if !url.is_empty() {
                // First relay use: make (or read) this device's keys
                self.inner.load_keys(instance_id);
                self.inner.start_inbound();
            }
            let (dial, token) = split_token(url);
            let (cancel_tx, cancel_rx) = watch::channel(());
            let generation = {
                let mut shared = self.inner.lock();
                shared.generation += 1;
                // Dropping the previous sender cancels its supervisor
                shared.cancel = None;
                shared.outbound = None;
                shared.instance_id = instance_id.to_string();
                shared.url = Some(url.to_string()).filter(|u| !u.is_empty());
                shared.token = token;
                shared.reason = None;
                if url.is_empty() {
                    shared.online.clear();
                    shared.state = RelayConnectionState::Disconnected;
                } else {
                    shared.cancel = Some(cancel_tx);
                    shared.state = RelayConnectionState::Connecting;
                }
                shared.generation
            };
            self.inner.emit_status();
            if url.is_empty() {
                return Ok(());
            }
            let (first_tx, first_rx) = oneshot::channel();
            tokio::spawn(supervise(
                self.inner.clone(),
                dial,
                generation,
                cancel_rx,
                first_tx,
            ));
            first_rx
                .await
                .unwrap_or_else(|_| Err("The connection was cancelled".to_string()))
        }
        .boxed()
    }

    fn register(&self, cards: Vec<AgentCard>) -> BoxFuture<'_, Result<(), String>> {
        async move {
            let rx = {
                let mut shared = self.inner.lock();
                for card in &cards {
                    shared.load.entry(card.agent_id().to_string()).or_insert(0);
                }
                shared.cards = Some(cards.clone());
                let message = Inner::register_message(&shared, cards);
                let outbound = shared
                    .outbound
                    .clone()
                    .ok_or_else(|| String::from(super::RelayUnavailable))?;
                let rx = shared.waiting_registered.insert(());
                outbound
                    .send(message)
                    .map_err(|_| String::from(super::RelayUnavailable))?;
                rx
            };
            await_reply(rx, "confirm the registration").await
        }
        .boxed()
    }

    fn heartbeat(&self, load: HashMap<String, u32>) -> BoxFuture<'_, Result<(), String>> {
        async move {
            self.inner.lock().load = load;
            self.inner.send(self.inner.heartbeat_message())
        }
        .boxed()
    }

    fn query_capability<'a>(
        &'a self,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
        self.inner.query(capability).boxed()
    }

    fn delegate_task<'a>(
        &'a self,
        task: &'a Task,
        alternates: &'a [String],
    ) -> BoxFuture<'a, Result<Submitted, String>> {
        self.inner.submit(task, alternates).boxed()
    }

    fn send_result(&self, result: TaskResult) -> BoxFuture<'_, Result<(), String>> {
        async move {
            // The relay drops a result from an instance not registered yet
            self.inner.until_registered().await?;
            let sealer = self.inner.sealer()?;
            let requester = result.requester_agent.split('/').next().unwrap_or_default();
            let keys = self.inner.keys_for(requester).await?;
            let report = sealer.seal_result(&result, (requester, &keys))?;
            self.inner.send(RelayMessage::TaskResult {
                task_id: result.task_id.clone(),
                result: report,
                signature: self.inner.token(),
            })
        }
        .boxed()
    }

    fn on_task_assigned(&self, handler: Handler<TaskAssigned>) -> BoxFuture<'_, ()> {
        self.inner.handlers().assigned.push(Arc::new(handler));
        self.inner.sync_inbox();
        async {}.boxed()
    }

    fn on_task_completed(&self, handler: Handler<TaskCompleted>) -> BoxFuture<'_, ()> {
        self.inner.handlers().completed.push(Arc::new(handler));
        self.inner.sync_inbox();
        async {}.boxed()
    }

    fn on_task_failed(&self, handler: Handler<TaskFailed>) -> BoxFuture<'_, ()> {
        self.inner.handlers().failed.push(Arc::new(handler));
        async {}.boxed()
    }

    fn on_task_event(&self, handler: Handler<TaskEvent>) -> BoxFuture<'_, ()> {
        self.inner.handlers().events.push(Arc::new(handler));
        self.inner.sync_inbox();
        async {}.boxed()
    }

    fn reassign<'a>(
        &'a self,
        task_id: &'a str,
        requester: &'a str,
        target: &'a str,
        by: &'a str,
        reason: Option<String>,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            let sealer = self.inner.sealer()?;
            let new_provider = target.split('/').next().unwrap_or_default();
            // Without the opened request (a restart) the relay asks the
            // requester to seal it again
            let request = if sealer.holds_request(task_id) {
                let keys = self.inner.keys_for(new_provider).await?;
                sealer
                    .reseal_request(task_id, (new_provider, &keys))?
                    .map(Box::new)
            } else {
                self.inner.may_send_to(new_provider)?;
                None
            };
            let envelope = match reason.filter(|r| !r.is_empty()) {
                Some(reason) => {
                    let keys = self.inner.keys_for(requester).await?;
                    sealer.note(task_id, (requester, &keys), Some(reason))?
                }
                None => None,
            };
            let message = RelayMessage::Reassign {
                task_id: task_id.to_string(),
                target: target.to_string(),
                by: by.to_string(),
                envelope,
                request,
            };
            self.inner.reassign_at_relay(message, task_id).await
        }
        .boxed()
    }

    fn reseal_as_author<'a>(
        &'a self,
        task: &'a Task,
        target: &'a str,
        by: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            let sealer = self.inner.sealer()?;
            let new_provider = target.split('/').next().unwrap_or_default();
            let keys = self.inner.keys_for(new_provider).await?;
            let request = sealer.seal_task(task, (new_provider, &keys))?;
            let message = RelayMessage::Reassign {
                task_id: task.id.clone(),
                target: target.to_string(),
                by: by.to_string(),
                envelope: None,
                request: Some(Box::new(request)),
            };
            self.inner.reassign_at_relay(message, &task.id).await
        }
        .boxed()
    }

    fn on_task_refused(&self, handler: Handler<TaskRefused>) -> BoxFuture<'_, ()> {
        self.inner.handlers().refused.push(Arc::new(handler));
        async {}.boxed()
    }

    fn on_reseal(&self, handler: Handler<ResealRequest>) -> BoxFuture<'_, ()> {
        self.inner.handlers().reseal.push(Arc::new(handler));
        self.inner.sync_inbox();
        async {}.boxed()
    }

    fn duty_watch<'a>(
        &'a self,
        duty_id: &'a str,
        primary_instance: &'a str,
        due_at: i64,
        after_minutes: Option<u32>,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            self.inner.until_registered().await?;
            self.inner.send(RelayMessage::DutyWatch {
                duty_id: duty_id.to_string(),
                primary_instance: primary_instance.to_string(),
                due_at,
                after_minutes,
            })
        }
        .boxed()
    }

    fn duty_report<'a>(
        &'a self,
        duty_id: &'a str,
        backup_instance: &'a str,
        state: DutyState,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            let (state, error) = match state {
                DutyState::Upcoming { due_at } => (wire::DutyState::Upcoming { due_at }, None),
                DutyState::Running { since } => (wire::DutyState::Running { since }, None),
                DutyState::Completed => (wire::DutyState::Completed, None),
                DutyState::Failed { error } => (wire::DutyState::Failed, error),
                DutyState::NotRun => (wire::DutyState::NotRun, None),
            };
            let envelope = match error.filter(|e| !e.is_empty()) {
                Some(error) => {
                    let sealer = self.inner.sealer()?;
                    let keys = self.inner.keys_for(backup_instance).await?;
                    sealer.note(duty_id, (backup_instance, &keys), Some(error))?
                }
                None => None,
            };
            self.inner.send(RelayMessage::DutyReport {
                duty_id: duty_id.to_string(),
                state,
                envelope,
            })
        }
        .boxed()
    }

    fn on_duty(&self, handler: Handler<DutyDelivery>) -> BoxFuture<'_, ()> {
        self.inner.handlers().duty.push(Arc::new(handler));
        self.inner.sync_inbox();
        async {}.boxed()
    }

    fn status(&self) -> RelayStatus {
        self.inner.status()
    }

    fn send_peer(
        &self,
        target_instance: String,
        body: PeerMessage,
    ) -> BoxFuture<'_, Result<(), String>> {
        async move {
            let sealer = self.inner.sealer()?;
            let target_keys = if Sealer::has_content(&body) {
                Some(self.inner.keys_for(&target_instance).await?)
            } else {
                self.inner.may_send_to(&target_instance)?;
                None
            };
            let body = sealer.seal_peer(&target_instance, body, target_keys.as_ref())?;
            self.inner.send(RelayMessage::Peer {
                target_instance,
                body,
            })
        }
        .boxed()
    }

    fn on_peer(&self, handler: Handler<PeerDelivery>) -> BoxFuture<'_, ()> {
        self.inner.handlers().peer.push(Arc::new(handler));
        self.inner.sync_inbox();
        async {}.boxed()
    }

    fn register_gateway(&self, card: GatewayCard) -> BoxFuture<'_, Result<(), String>> {
        async move {
            let (message, reregister) = {
                let mut shared = self.inner.lock();
                let was_accepting = shared
                    .gateway
                    .as_ref()
                    .is_some_and(|g| g.accepts_delegation);
                shared.gateway = Some(card.clone());
                // The route card comes and goes with `accepts_delegation`
                let reregister = (was_accepting != card.accepts_delegation)
                    .then(|| {
                        shared
                            .cards
                            .clone()
                            .map(|cards| Inner::register_message(&shared, cards))
                    })
                    .flatten();
                // Before the relay confirms our registration it would refuse
                // the card: it goes out with the confirmation instead
                let message = (shared.registered || shared.outbound.is_none()).then(|| {
                    RelayMessage::RegisterGateway {
                        instance_id: shared.instance_id.clone(),
                        card,
                    }
                });
                (message, reregister)
            };
            if let Some(message) = message {
                self.inner.send(message)?;
            }
            if let Some(register) = reregister {
                self.inner.send(register)?;
            }
            Ok(())
        }
        .boxed()
    }

    fn query_directory(&self) -> BoxFuture<'_, Result<Vec<MemberRecord>, String>> {
        async move {
            self.inner.until_registered().await?;
            let rx = self.inner.lock().waiting_directory.insert(());
            self.inner
                .send(RelayMessage::QueryDirectory { since: None })?;
            await_reply(rx, "list its members").await
        }
        .boxed()
    }

    fn find_people<'a>(
        &'a self,
        topic: &'a str,
        limit: u32,
    ) -> BoxFuture<'a, Result<PeopleSearch, String>> {
        async move {
            self.inner.until_registered().await?;
            let rx = self
                .inner
                .lock()
                .waiting_people
                .insert((topic.to_string(), limit));
            self.inner.send(RelayMessage::FindPeople {
                topic: topic.to_string(),
                limit,
            })?;
            await_reply(rx, "search").await
        }
        .boxed()
    }

    fn set_presence(
        &self,
        capabilities: Vec<CapabilityPresence>,
        tools: Vec<String>,
    ) -> BoxFuture<'_, Result<(), String>> {
        async move {
            let message = {
                let mut shared = self.inner.lock();
                shared.capabilities = capabilities.clone();
                shared.tools = tools.clone();
                // Before the confirmation the next `RegisterInstance`
                // carries it
                shared.registered.then(|| RelayMessage::UpdatePresence {
                    instance_id: shared.instance_id.clone(),
                    capabilities,
                    tools,
                })
            };
            match message {
                Some(message) => self.inner.send(message),
                None => Ok(()),
            }
        }
        .boxed()
    }

    fn route<'a>(&'a self, query: &'a RoutingQuery) -> BoxFuture<'a, Result<RoutePlan, String>> {
        async move {
            self.inner.until_registered().await?;
            let query_id = uuid::Uuid::new_v4().to_string();
            let rx = self.inner.lock().waiting_routes.insert(query_id.clone());
            self.inner.send(RelayMessage::RouteQuery {
                query_id,
                query: query.clone(),
            })?;
            await_reply(rx, "route").await
        }
        .boxed()
    }

    fn group_request(&self, op: GroupOp) -> BoxFuture<'_, Result<GroupOutcome, String>> {
        async move { self.inner.group_call(op).await }.boxed()
    }

    fn on_groups(&self, handler: Handler<GroupSignal>) -> BoxFuture<'_, ()> {
        self.inner.handlers().groups.push(Arc::new(handler));
        Box::pin(async {})
    }

    fn public_keys_of<'a>(
        &'a self,
        instance: &'a str,
    ) -> BoxFuture<'a, Result<PublicKeys, String>> {
        async move { self.inner.keys_for(instance).await }.boxed()
    }

    fn query_capability_presence<'a>(
        &'a self,
        capability_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<InstancePresence>, String>> {
        async move {
            self.inner.until_registered().await?;
            let (rx, message) = {
                let mut shared = self.inner.lock();
                let rx = shared.waiting_presence.insert(capability_id.to_string());
                let message = RelayMessage::CapabilityPresenceQuery {
                    capability_id: capability_id.to_string(),
                    requester_instance: shared.instance_id.clone(),
                };
                (rx, message)
            };
            self.inner.send(message)?;
            await_reply(rx, "answer").await
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wss_reaches_the_tls_handshake_without_panicking() {
        // A TCP peer that hangs up: the client gets as far as the TLS
        // handshake, which needs a crypto provider and TLS support
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        ensure_tls_provider();
        let result =
            tokio_tungstenite::connect_async(format!("wss://localhost:{}/relay", addr.port()))
                .await;
        let Err(error) = result else {
            panic!("the peer hung up, so the connect must fail");
        };
        let error = error.to_string();
        assert!(!error.contains("TLS support not compiled in"), "{error}");
    }

    #[test]
    fn backoff_doubles_up_to_thirty_seconds() {
        let schedule: Vec<u64> = (0..9).map(|n| backoff_delay(n).as_secs()).collect();
        assert_eq!(schedule, vec![1, 2, 4, 8, 16, 30, 30, 30, 30]);
        assert_eq!(backoff_delay(u32::MAX), MAX_BACKOFF);
    }

    #[tokio::test]
    async fn a_new_client_is_disconnected() {
        let client = WsRelayClient::new(None, "i1");
        let status = client.status();
        assert!(!status.connected);
        assert_eq!(status.instance_id, "i1");
        assert_eq!(status.relay_url, None);
        assert!(client.register(vec![]).await.is_err());
        assert!(client.query_capability("x").await.is_err());
    }

    #[tokio::test]
    async fn connecting_to_nothing_fails_but_keeps_the_url() {
        let client = WsRelayClient::new(None, "i1");
        // A port nothing listens on
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = format!("ws://127.0.0.1:{port}/relay");
        assert!(client.connect(&url, "i1").await.is_err());
        assert_eq!(client.status().relay_url.as_deref(), Some(url.as_str()));
        assert!(!client.status().connected);
        // Retrying, with the reason
        for _ in 0..400 {
            if client.status().state == RelayConnectionState::Reconnecting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let status = client.status();
        assert_eq!(status.state, RelayConnectionState::Reconnecting);
        let reason = status.reason.unwrap_or_default();
        assert!(reason.contains("Failed to connect"), "{reason}");
        assert!(reason.contains("retrying in 1s"), "{reason}");
        // Disconnecting clears it and stops the retries
        client.connect("", "i1").await.unwrap();
        assert_eq!(client.status().relay_url, None);
        assert_eq!(client.status().state, RelayConnectionState::Disconnected);
        assert_eq!(client.status().reason, None);
    }

    #[tokio::test]
    async fn pending_calls_get_their_keys_answer_and_fail_together() {
        let mut pending: Pending<&str, u32> = Pending::default();
        let (first, second, other) = (
            pending.insert("a"),
            pending.insert("a"),
            pending.insert("b"),
        );
        pending.resolve(&"a", Ok(7));
        assert_eq!(first.await.unwrap(), Ok(7));
        assert_eq!(second.await.unwrap(), Ok(7), "every call on a key");

        // A call that gave up is dropped on the next insert
        drop(pending.insert("gone"));
        let late = pending.insert("c");
        assert!(!pending.waiting.contains_key("gone"));

        pending.fail_all("down");
        assert_eq!(other.await.unwrap(), Err("down".to_string()));
        assert_eq!(late.await.unwrap(), Err("down".to_string()));
        assert!(pending.waiting.is_empty());
    }

    #[tokio::test]
    async fn a_call_dropped_unanswered_is_relay_unavailable() {
        let mut pending: Pending<(), ()> = Pending::default();
        let dropped = pending.insert(());
        drop(pending);
        let error = await_reply(dropped, "answer").await.unwrap_err();
        assert!(super::super::RelayUnavailable::is(&error), "{error}");
    }

    /// A client with an open socket (its frames go to the receiver), as
    /// registered as `registered` says
    fn open_client(registered: bool) -> (WsRelayClient, mpsc::UnboundedReceiver<RelayMessage>) {
        let client = WsRelayClient::new(None, "i1");
        let (outbound, sent) = mpsc::unbounded_channel();
        let mut shared = client.inner.lock();
        shared.outbound = Some(outbound);
        shared.registered = registered;
        drop(shared);
        (client, sent)
    }

    /// How the connection lets a waiting call down
    enum LetDown {
        /// The socket closes
        Disconnect,
        /// The relay forgot our registration
        NotRegistered,
    }

    #[tokio::test]
    async fn a_call_the_connection_lets_down_is_relay_unavailable() {
        for let_down in [LetDown::Disconnect, LetDown::NotRegistered] {
            let (client, mut sent) = open_client(true);
            let inner = client.inner.clone();
            let call = tokio::spawn(async move { client.query_directory().await });
            sent.recv().await.unwrap();

            match let_down {
                LetDown::Disconnect => {
                    let generation = inner.lock().generation;
                    inner.on_disconnected(generation);
                }
                LetDown::NotRegistered => {
                    let error = RelayMessage::Error {
                        code: "not_registered".into(),
                        message: "Register first".into(),
                        task_id: None,
                    };
                    inner.handle_incoming(&error.to_json().unwrap());
                }
            }

            let error = call.await.unwrap().unwrap_err();
            assert!(super::super::RelayUnavailable::is(&error), "{error}");
        }
    }

    #[tokio::test]
    async fn a_connection_that_never_registers_is_relay_unavailable() {
        let (client, _sent) = open_client(false);
        let inner = client.inner.clone();
        let call = tokio::spawn(async move { client.query_directory().await });
        while inner.lock().waiting_registered.waiting.is_empty() {
            tokio::task::yield_now().await;
        }
        let generation = inner.lock().generation;
        inner.on_disconnected(generation);

        let error = call.await.unwrap().unwrap_err();
        assert!(super::super::RelayUnavailable::is(&error), "{error}");
    }

    #[tokio::test]
    async fn only_a_refusal_naming_a_task_reaches_the_refused_handlers() {
        let (client, _sent) = open_client(true);
        let refused = Arc::new(Mutex::new(Vec::new()));
        let seen = refused.clone();
        client
            .on_task_refused(Box::new(move |r| seen.lock().unwrap().push(r)))
            .await;
        for (code, task_id) in [
            (nandi_protocol::TASK_ENDED, Some("t1")),
            ("rate_limited", None),
        ] {
            let error = RelayMessage::Error {
                code: code.into(),
                message: "No".into(),
                task_id: task_id.map(Into::into),
            };
            client.inner.handle_incoming(&error.to_json().unwrap());
        }
        assert_eq!(
            *refused.lock().unwrap(),
            [TaskRefused {
                task_id: "t1".into(),
                code: nandi_protocol::TASK_ENDED.into(),
                message: "No".into(),
            }]
        );
    }

    #[test]
    fn coded_refusals_are_retold_once() {
        let mut shared = Shared::default();
        let words = "You've sent the 500 messages a day this relay allows                      (counted across your Nandis). It resets at 2026-11-11T00:00:00Z";
        shared.daily_limit.insert(words.to_string());
        shared
            .no_shared_group
            .insert("No group with bob".to_string());
        assert_eq!(
            shared.retell(words),
            format!("daily_limit@1794355200000: {words}")
        );
        // Told once: the same words again are the relay's own
        assert_eq!(shared.retell(words), words);
        assert_eq!(shared.retell("No group with bob"), no_shared_group());
        assert_eq!(shared.retell("Something else"), "Something else");
    }
}
