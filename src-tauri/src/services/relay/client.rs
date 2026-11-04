//! `RelayClient` over tokio-tungstenite: outbound WebSocket, exponential
//! backoff reconnect (1s, 2s, 4s, 8s, 16s, then every 30s), re-registration
//! plus an immediate heartbeat on every reconnect, 30s heartbeats.
//!
//! Every state change (connecting, connected, reconnecting, disconnected,
//! error) emits `relay://status` with the reason.
//!
//! One supervisor task per `connect` owns the socket: it reads frames,
//! writes queued messages, and sends heartbeats. Requests that expect an
//! answer (`register`, `query_capability`, `delegate_task`) park a oneshot
//! that the reader resolves. A new `connect` (or `connect("")`, which
//! disconnects) cancels the previous supervisor.
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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::{FutureExt, SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use super::client_api::{Handler, PeopleSearch, RelayClient};
use super::protocol::{
    CapabilityPresence, InstancePresence, MemberRecord, PeerDelivery, PeerMessage, RelayMessage,
    TaskAssigned, TaskCompleted, TaskFailed, ALL_CAPABILITIES, PROTOCOL_VERSION, VERSION_MISMATCH,
};
use super::{RelayAgentOffline, RelayAgentOnline, RelayConnectionState, RelayStatus};
use crate::model::Timestamp;
use crate::services::a2a::{Task, TaskResult};
use crate::services::domain::AgentCard;
use crate::services::events::{self, EventBus};
use crate::services::gateway::{registration, GatewayCard};
use nandi_protocol::PROBE_PAYLOAD;

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

#[derive(Default)]
struct Handlers {
    assigned: Vec<Arc<Handler<TaskAssigned>>>,
    completed: Vec<Arc<Handler<TaskCompleted>>>,
    failed: Vec<Arc<Handler<TaskFailed>>>,
    peer: Vec<Arc<Handler<PeerDelivery>>>,
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
    waiting_registered: Vec<oneshot::Sender<Result<(), String>>>,
    /// The relay confirmed our registration on this connection (queries
    /// before it are refused with `not_registered`)
    registered: bool,
    /// The relay refused our registration on this connection (a bad token,
    /// or another protocol version): not connected until a new one
    refused: bool,
    /// The refusal was for the protocol version
    version_mismatch: bool,
    waiting_queries: HashMap<String, Vec<oneshot::Sender<Vec<AgentCard>>>>,
    waiting_acks: HashMap<String, oneshot::Sender<Result<(), String>>>,
    /// Our gateway card (wave final); re-sent on reconnect
    gateway: Option<GatewayCard>,
    waiting_directory: Vec<oneshot::Sender<Vec<MemberRecord>>>,
    /// People searches by `(topic, limit)`
    waiting_people: HashMap<(String, u32), Vec<oneshot::Sender<PeopleSearch>>>,
    /// Wave A: what we provide; carried by every `RegisterInstance`
    capabilities: Vec<CapabilityPresence>,
    tools: Vec<String>,
    waiting_presence: HashMap<String, Vec<oneshot::Sender<Vec<InstancePresence>>>>,
}

struct Inner {
    shared: Mutex<Shared>,
    handlers: Mutex<Handlers>,
    bus: Option<EventBus>,
    heartbeat_every: Duration,
    presence_store: Option<PresenceStore>,
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
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            heartbeat_every: HEARTBEAT_INTERVAL,
            presence_store: None,
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
            state: shared.state,
            reason: shared.reason.clone(),
            instance_id: shared.instance_id.clone(),
            relay_url: shared.url.clone(),
            relay_url_source: None,
            // Filled in by `get_relay_status`
            built_in_url: None,
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
            .map_err(|_| "The relay connection closed".to_string())
    }

    fn token(&self) -> String {
        self.lock().token.clone().unwrap_or_default()
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
            shared.refused = false;
            shared.version_mismatch = false;
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
            for waiter in shared.waiting_registered.drain(..) {
                let _ = waiter.send(Err("Disconnected from the relay".to_string()));
            }
            shared.waiting_queries.clear();
            shared.waiting_directory.clear();
            shared.waiting_people.clear();
            shared.waiting_presence.clear();
            for (_, waiter) in shared.waiting_acks.drain() {
                let _ = waiter.send(Err("Disconnected from the relay".to_string()));
            }
        }
        self.emit_status();
    }

    fn handle_incoming(self: &Arc<Self>, text: &str) {
        let message = match RelayMessage::from_json(text) {
            Ok(message) => message,
            Err(e) => return log::warn!("Relay: {e}"),
        };
        match message {
            RelayMessage::Registered { .. } => {
                let waiters = {
                    let mut shared = self.lock();
                    shared.registered = true;
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
                    std::mem::take(&mut shared.waiting_registered)
                };
                for waiter in waiters {
                    let _ = waiter.send(Ok(()));
                }
                tokio::spawn(self.clone().refresh_presence());
            }
            RelayMessage::CapabilityResult { capability, agents } => {
                let waiters = self
                    .lock()
                    .waiting_queries
                    .remove(&capability)
                    .unwrap_or_default();
                for waiter in waiters {
                    let _ = waiter.send(agents.clone());
                }
            }
            RelayMessage::TaskAssigned(assigned) => {
                if assigned.task.is_none() {
                    if let Some(waiter) = self.lock().waiting_acks.remove(&assigned.task_id) {
                        let _ = waiter.send(Ok(()));
                    }
                }
                let handlers = self.handlers().assigned.clone();
                for handler in handlers {
                    handler(assigned.clone());
                }
            }
            RelayMessage::TaskCompleted(completed) => {
                if let Some(waiter) = self.lock().waiting_acks.remove(&completed.task_id) {
                    let _ = waiter.send(Ok(()));
                }
                let handlers = self.handlers().completed.clone();
                for handler in handlers {
                    handler(completed.clone());
                }
            }
            RelayMessage::TaskFailed(failed) => {
                // The relay refusing to route our send (provider offline):
                // that answers `delegate_task`, whose caller decides (queue
                // it, or try the next provider); it's no result of the task
                let refused = self.lock().waiting_acks.remove(&failed.task_id);
                if let Some(waiter) = refused {
                    let _ = waiter.send(Err(failed.error.clone()));
                    return;
                }
                let handlers = self.handlers().failed.clone();
                for handler in handlers {
                    handler(failed.clone());
                }
            }
            RelayMessage::Error { code, message } => {
                log::warn!("Relay error ({code}): {message}");
                let mut shared = self.lock();
                if code == "unauthorized" || code == VERSION_MISMATCH {
                    let refused = format!("The relay refused us: {message}");
                    for waiter in shared.waiting_registered.drain(..) {
                        let _ = waiter.send(Err(refused.clone()));
                    }
                    shared.refused = true;
                    shared.version_mismatch = code == VERSION_MISMATCH;
                    shared.state = RelayConnectionState::Error;
                    shared.reason = Some(refused);
                    drop(shared);
                    return self.emit_status();
                }
                if code == "not_registered" {
                    // Dropping the waiters fails them now instead of at the timeout
                    shared.waiting_queries.clear();
                    shared.waiting_presence.clear();
                    shared.waiting_directory.clear();
                    shared.waiting_people.clear();
                    for (_, waiter) in shared.waiting_acks.drain() {
                        let _ = waiter.send(Err(message.clone()));
                    }
                }
            }
            RelayMessage::Directory { members } => {
                let waiters = std::mem::take(&mut self.lock().waiting_directory);
                for waiter in waiters {
                    let _ = waiter.send(members.clone());
                }
            }
            RelayMessage::PeopleMatches {
                topic,
                limit,
                matches,
                searched,
                unstated,
                total,
            } => {
                let waiters = self
                    .lock()
                    .waiting_people
                    .remove(&(topic, limit))
                    .unwrap_or_default();
                let found = PeopleSearch {
                    matches,
                    searched,
                    unstated,
                    total,
                };
                for waiter in waiters {
                    let _ = waiter.send(found.clone());
                }
            }
            RelayMessage::CapabilityPresenceResponse {
                capability_id,
                providers,
            } => {
                let waiters = self
                    .lock()
                    .waiting_presence
                    .remove(&capability_id)
                    .unwrap_or_default();
                for waiter in waiters {
                    let _ = waiter.send(providers.clone());
                }
            }
            RelayMessage::Peer {
                from_instance,
                target_instance,
                body,
            } => self.deliver_peer(PeerDelivery {
                from_instance,
                target_instance,
                body,
                undeliverable: false,
            }),
            RelayMessage::Undeliverable {
                target_instance,
                body,
            } => {
                let from_instance = self.lock().instance_id.clone();
                self.deliver_peer(PeerDelivery {
                    from_instance,
                    target_instance,
                    body,
                    undeliverable: true,
                });
            }
            other => log::debug!("Relay: ignoring {other:?}"),
        }
    }

    fn deliver_peer(&self, delivery: PeerDelivery) {
        let handlers = self.handlers().peer.clone();
        if handlers.is_empty() {
            log::debug!("Relay: no handler for {:?}", delivery.body);
        }
        for handler in handlers {
            handler(delivery.clone());
        }
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
            let (tx, rx) = oneshot::channel();
            shared.waiting_registered.push(tx);
            rx
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Disconnected from the relay".to_string()),
            Err(_) => Err("Not registered with the relay yet".to_string()),
        }
    }

    async fn query(&self, capability: &str) -> Result<Vec<AgentCard>, String> {
        self.until_registered().await?;
        let (tx, rx) = oneshot::channel();
        let message = {
            let mut shared = self.lock();
            shared
                .waiting_queries
                .entry(capability.to_string())
                .or_default()
                .push(tx);
            RelayMessage::QueryCapability {
                capability: capability.to_string(),
                requester_instance: shared.instance_id.clone(),
            }
        };
        self.send(message)?;
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(cards)) => Ok(cards),
            Ok(Err(_)) => Err("The relay didn't answer the query".to_string()),
            Err(_) => Err(format!(
                "The relay didn't answer within {}s",
                REQUEST_TIMEOUT.as_secs()
            )),
        }
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
            let (tx, rx) = oneshot::channel();
            {
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
                shared.waiting_registered.push(tx);
                outbound
                    .send(message)
                    .map_err(|_| "The relay connection closed".to_string())?;
            }
            match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err("Disconnected from the relay".to_string()),
                Err(_) => Err(format!(
                    "The relay didn't confirm the registration within {}s",
                    REQUEST_TIMEOUT.as_secs()
                )),
            }
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

    fn delegate_task<'a>(&'a self, task: &'a Task) -> BoxFuture<'a, Result<String, String>> {
        async move {
            let (tx, rx) = oneshot::channel();
            self.inner.lock().waiting_acks.insert(task.id.clone(), tx);
            let message = RelayMessage::DelegateTask {
                task_id: task.id.clone(),
                target_agent: task.target.clone(),
                capability: task.capability.clone(),
                input: task.input.clone(),
                requester_card: task.requester.clone(),
                deadline: task.deadline,
                signature: self.inner.token(),
            };
            if let Err(e) = self.inner.send(message) {
                self.inner.lock().waiting_acks.remove(&task.id);
                return Err(e);
            }
            match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
                Ok(Ok(Ok(()))) => Ok(task.id.clone()),
                Ok(Ok(Err(e))) => Err(e),
                Ok(Err(_)) => Err("Disconnected from the relay".to_string()),
                Err(_) => {
                    self.inner.lock().waiting_acks.remove(&task.id);
                    Err(format!(
                        "The relay didn't route the task within {}s",
                        REQUEST_TIMEOUT.as_secs()
                    ))
                }
            }
        }
        .boxed()
    }

    fn send_result(&self, result: TaskResult) -> BoxFuture<'_, Result<(), String>> {
        async move {
            self.inner.send(RelayMessage::TaskResult {
                task_id: result.task_id.clone(),
                result,
                signature: self.inner.token(),
            })
        }
        .boxed()
    }

    fn on_task_assigned(&self, handler: Handler<TaskAssigned>) -> BoxFuture<'_, ()> {
        self.inner.handlers().assigned.push(Arc::new(handler));
        async {}.boxed()
    }

    fn on_task_completed(&self, handler: Handler<TaskCompleted>) -> BoxFuture<'_, ()> {
        self.inner.handlers().completed.push(Arc::new(handler));
        async {}.boxed()
    }

    fn on_task_failed(&self, handler: Handler<TaskFailed>) -> BoxFuture<'_, ()> {
        self.inner.handlers().failed.push(Arc::new(handler));
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
            self.inner.send(RelayMessage::Peer {
                from_instance: String::new(),
                target_instance,
                body,
            })
        }
        .boxed()
    }

    fn on_peer(&self, handler: Handler<PeerDelivery>) -> BoxFuture<'_, ()> {
        self.inner.handlers().peer.push(Arc::new(handler));
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
            let (tx, rx) = oneshot::channel();
            self.inner.lock().waiting_directory.push(tx);
            self.inner
                .send(RelayMessage::QueryDirectory { since: None })?;
            match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
                Ok(Ok(members)) => Ok(members),
                Ok(Err(_)) => Err("Disconnected from the relay".to_string()),
                Err(_) => Err(format!(
                    "The relay didn't list its members within {}s",
                    REQUEST_TIMEOUT.as_secs()
                )),
            }
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
            let (tx, rx) = oneshot::channel();
            self.inner
                .lock()
                .waiting_people
                .entry((topic.to_string(), limit))
                .or_default()
                .push(tx);
            self.inner.send(RelayMessage::FindPeople {
                topic: topic.to_string(),
                limit,
            })?;
            match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
                Ok(Ok(found)) => Ok(found),
                Ok(Err(_)) => Err("Disconnected from the relay".to_string()),
                Err(_) => Err(format!(
                    "The relay didn't search within {}s",
                    REQUEST_TIMEOUT.as_secs()
                )),
            }
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

    fn query_capability_presence<'a>(
        &'a self,
        capability_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<InstancePresence>, String>> {
        async move {
            self.inner.until_registered().await?;
            let (tx, rx) = oneshot::channel();
            let message = {
                let mut shared = self.inner.lock();
                shared
                    .waiting_presence
                    .entry(capability_id.to_string())
                    .or_default()
                    .push(tx);
                RelayMessage::CapabilityPresenceQuery {
                    capability_id: capability_id.to_string(),
                    requester_instance: shared.instance_id.clone(),
                }
            };
            self.inner.send(message)?;
            match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
                Ok(Ok(providers)) => Ok(providers),
                Ok(Err(_)) => Err("The relay didn't answer the presence query".to_string()),
                Err(_) => Err(format!(
                    "The relay didn't answer within {}s",
                    REQUEST_TIMEOUT.as_secs()
                )),
            }
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
}
