//! The relay: cross-instance presence and message routing over one
//! outbound WebSocket per instance, so instances behind NAT reach each
//! other (wave 2A, Track B).
//!
//! - `protocol`: the wire messages (the `nandi-protocol` crate)
//! - `client_api`: the `RelayClient` trait
//! - `client`: `RelayClient` over tokio-tungstenite
//! - `tls`: what the client trusts for `wss://`
//! - `unavailable`: [`RelayUnavailable`], the typed "not connected" answer
//!   of every person-to-person action
//! - `device_keys`: this instance's own keys (made on first relay use, kept
//!   in the OS keyring), sent at enrollment and registration
//! - `opened`: person-to-person traffic as the app sees it, with content
//! - `sealing`: sealing it for the recipient and opening it (the client's
//!   only crypto; content never crosses the relay in the clear)
//!
//! The relay server is a separate product (`vaultdbai/nandi-relay`); the
//! app never runs or links it; the app's relay tests embed it as a
//! dev-dependency.
//!
//! # Which relay
//!
//! 1. `NANDI_RELAY_URL`, when set (handy for testing two instances)
//! 2. else `relay_url` in `~/.nandi/config.json` (`NANDI_HOME` moves it), plus
//!    the instance token from `relay_state` if the relay needs one
//! 3. else nothing: the app doesn't connect. A build's own relay
//!    ([`BuiltInRelay`]) is only offered: nothing dials it, and nothing
//!    enrolls there, until the user joins it ([`join::join_relay`]).
//!    Joining writes the URL to `config.json`, so later launches connect
//!    on their own.
//!
//! The token never goes into `config.json`: a URL saved with `?token=…` is
//! split, and the token kept in `relay_state` (key `relay_token`, with the
//! relay it is for under `relay_token_relay`). Disconnecting keeps it, so
//! joining the same relay again reuses this instance's seat.
//!
//! `relay_state` also keeps `online_agents` (every card at the relay as of
//! the last presence refresh, the fallback while disconnected). See
//! `relay-testing.md` in vaultdbai/nandi-relay.

pub mod client;
pub mod client_api;
pub mod device_keys;
pub mod join;
pub mod opened;
pub mod protocol;
pub mod sealing;
// Wave A (Track D): capability and tool presence
pub mod presence;
pub mod tls;
pub mod unavailable;

#[cfg(test)]
mod keys_tests;
#[cfg(test)]
mod sealed_tests;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, OptionalExtension};

use serde::{Deserialize, Serialize};
use specta::Type;

pub use client_api::RelayClient;
pub use unavailable::{RelayUnavailable, RELAY_UNAVAILABLE};

use super::capability_index::CapabilityIndex;
use super::domain::agent_card::card_from_agent;
use super::domain::AgentCard;
use super::domain::{ChangedBy, DomainEvent, EntityKind, Problem};
use super::events::NandiEvent;
use super::gateway::manifest as gateway_manifest;
use super::gateway::registration;
use super::network;
use super::repo_store::{EventPublisher, RepoStore};
use super::AppState;
use crate::db::Db;
use crate::model::Timestamp;
use crate::utils::nandi_config;
use client::{split_token, ClientOptions, WsRelayClient};
use device_keys::DeviceKeyring;
use nandi_protocol::RelayInstanceInfo;
use protocol::ALL_CAPABILITIES;

/// `relay_state` key holding the instance token split off the relay URL
pub const RELAY_TOKEN_KEY: &str = "relay_token";

/// `relay_state` key holding the relay (without its token) that
/// [`RELAY_TOKEN_KEY`] is for
pub const RELAY_TOKEN_RELAY_KEY: &str = "relay_token_relay";

/// Environment variable overriding the configured relay URL
pub const RELAY_URL_ENV: &str = "NANDI_RELAY_URL";

/// `relay_state` key holding the last known cards at the relay
pub const ONLINE_AGENTS_KEY: &str = "online_agents";

/// The relay the Relay view suggests while none is configured: `nandi-relay`
/// on this machine. Mirrored by `DEFAULT_RELAY_URL` in `useRelay.ts`.
pub const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8787/relay";

/// The relay a build comes with, offered as "the Nandi network":
/// `NANDI_DEFAULT_RELAY_URL` and `NANDI_RELAY_JOIN_CODE` when the app was
/// built (a demo or company relay; the release workflow sets them). The
/// join code is not the relay's secret: the relay checks it and issues a
/// token for this instance only (`/relay/enroll`).
///
/// Only an offer: the app neither dials it nor enrolls there until the user
/// joins it, so an install that never joins takes no seat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuiltInRelay {
    url: Option<String>,
    join_code: Option<String>,
}

impl BuiltInRelay {
    /// The relay this build was made with, if any
    pub fn of_this_build() -> Self {
        Self::new(
            option_env!("NANDI_DEFAULT_RELAY_URL"),
            option_env!("NANDI_RELAY_JOIN_CODE"),
        )
    }

    /// Blank values count as none
    pub fn new(url: Option<&str>, join_code: Option<&str>) -> Self {
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Self {
            url: clean(url),
            join_code: clean(join_code),
        }
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn join_code(&self) -> Option<&str> {
        self.join_code.as_deref()
    }
}

/// How long "Test connection" waits for the pong, and the instance list
/// for an answer
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long "Test connection" waits to open the WebSocket
pub const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RelayConnectionState {
    /// No relay configured, or disconnected on purpose
    #[default]
    Disconnected,
    /// Dialing for the first time since the URL was set
    Connecting,
    Connected,
    /// The connection dropped or a dial failed; retrying with backoff
    Reconnecting,
    /// The relay refused this instance (e.g. a bad token)
    Error,
}

/// Where the relay URL came from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RelayUrlSource {
    /// `NANDI_RELAY_URL`
    Env,
    /// `relay_url` in `config.json`
    Config,
    /// This build's default relay (`NANDI_DEFAULT_RELAY_URL` at build time)
    BuiltIn,
}

/// Connection state; also the `relay://status` payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatus {
    /// Registered and usable for person-to-person actions
    pub connected: bool,
    /// The relay refused this Nandi for speaking another protocol version
    /// ("Update Nandi or the relay")
    #[serde(default)]
    pub version_mismatch: bool,
    /// The relay has other keys pinned for this device (Nandi was
    /// reinstalled, or the keyring was reset): its operator must rekey it
    #[serde(default)]
    pub key_mismatch: bool,
    pub state: RelayConnectionState,
    /// Why `state` last changed (an error, or "…; retrying in 4s")
    pub reason: Option<String>,
    pub instance_id: String,
    /// `None` until the user sets one
    pub relay_url: Option<String>,
    pub relay_url_source: Option<RelayUrlSource>,
    /// The relay this build comes with, whether or not it's in use (what
    /// onboarding offers as "the Nandi network"); `None`: none
    #[serde(default)]
    pub built_in_url: Option<String>,
    pub agents_online: u32,
    /// Distinct instances among the agents online
    pub instances_online: u32,
    pub last_connected_at: Option<Timestamp>,
    pub last_heartbeat_at: Option<Timestamp>,
    /// Who sees whom on this relay, as it said when we last registered
    /// (`groups`: only people we share a group with); `None` until then
    #[serde(default)]
    pub visibility: Option<RelayVisibility>,
}

/// Who sees whom on a relay (`nandi_protocol::Visibility`, named apart
/// from the app's own sharing `Visibility` in the bindings)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RelayVisibility {
    /// Everyone enrolled sees everyone (a company's own relay)
    Company,
    /// Members see only people they share a group with (a public relay)
    Groups,
}

impl From<nandi_protocol::Visibility> for RelayVisibility {
    fn from(visibility: nandi_protocol::Visibility) -> Self {
        match visibility {
            nandi_protocol::Visibility::Company => Self::Company,
            nandi_protocol::Visibility::Groups => Self::Groups,
        }
    }
}

/// One instance at the relay, from its `/instances` endpoint
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayPeer {
    pub instance_id: String,
    /// Cards it registered
    pub agents: u32,
    pub connected_at: Timestamp,
    pub last_heartbeat: Timestamp,
}

impl RelayStatus {
    /// The status as the UI gets it (the command, the HTTP route and
    /// `relay://status`): the relay URL without this instance's token
    pub fn without_token(mut self) -> Self {
        self.relay_url = self.relay_url.map(|url| split_token(&url).0);
        self
    }
}

impl NandiEvent for RelayStatus {
    const NAME: &'static str = "relay://status";
}

/// `relay://agent_online`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayAgentOnline {
    pub card: AgentCard,
}

impl NandiEvent for RelayAgentOnline {
    const NAME: &'static str = "relay://agent_online";
}

/// `relay://agent_offline`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayAgentOffline {
    pub card_id: String,
    pub instance_id: String,
}

impl NandiEvent for RelayAgentOffline {
    const NAME: &'static str = "relay://agent_offline";
}

/// `relay://seal_refused`: something another person sent didn't open, or
/// wasn't theirs to send (`sealing::SEAL_REFUSED`), and nothing in it was
/// used
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelaySealRefused {
    pub from_instance: String,
    pub task_id: String,
    /// Starts with `SEAL_REFUSED` (or `PEER_KEY_CHANGED`)
    pub error: String,
}

impl NandiEvent for RelaySealRefused {
    const NAME: &'static str = "relay://seal_refused";
}

/// `network://member_key_changed`: the relay shows other keys for a member
/// than the ones pinned; their profile asks the user to check them
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemberKeyChanged {
    pub instance_id: String,
}

impl NandiEvent for MemberKeyChanged {
    const NAME: &'static str = "network://member_key_changed";
}

/// Builds the relay client and, if a relay URL is set, connects in the
/// background. Never touches the network on the calling thread.
pub fn init(state: &AppState) -> Option<Arc<dyn RelayClient>> {
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Relay disabled: no instance ID: {e}");
            return None;
        }
    };
    let db = state.db().clone();
    let keyring = DeviceKeyring::new(state.secrets().clone());
    let client: Arc<dyn RelayClient> = Arc::new(WsRelayClient::with_options(
        Some(state.bus().clone()),
        instance_id,
        ClientOptions {
            presence_store: Some(Box::new(move |cards: &[AgentCard]| {
                if let Err(e) = write_state(&db, ONLINE_AGENTS_KEY, cards) {
                    log::warn!("Failed to save the online agents: {e}");
                }
            })),
            // Read on the first connect, not here: no keyring at startup
            device_keys: Some(Arc::new(move || keyring.device_keys())),
            peer_keys: Some(Arc::new(network::peer_keys::PinnedKeys::new(
                state.db().clone(),
            ))),
            ..ClientOptions::default()
        },
    ));
    state.store().subscribe(Arc::new(Reregister {
        client: client.clone(),
        index: state.capability_index().cloned(),
        store: state.store().clone(),
        runtime: state.runtime().clone(),
        pending: Arc::default(),
    }));
    match resolve_url(state) {
        Ok(Some((url, _))) => {
            let task_state = state.clone();
            let task_client = client.clone();
            state.runtime().spawn(async move {
                connect_and_register(&task_state, task_client.as_ref(), &url).await;
            });
        }
        Ok(None) => {}
        Err(e) => log::warn!("Failed to read the relay URL: {e}"),
    }
    Some(client)
}

pub async fn get_relay_status(state: &AppState) -> Result<RelayStatus, String> {
    let resolved = resolve_url(state).unwrap_or_else(|e| {
        log::warn!("Failed to read the relay URL: {e}");
        None
    });
    let env = env_url();
    let source = |url: &Option<String>| {
        url.as_ref().map(|url| {
            if env.as_deref() == Some(url.as_str()) {
                RelayUrlSource::Env
            } else if state.built_in_relay().url() == Some(split_token(url).0.as_str()) {
                RelayUrlSource::BuiltIn
            } else {
                RelayUrlSource::Config
            }
        })
    };
    if let Some(client) = state.relay_client() {
        let mut status = client.status();
        // The background connect may not have started yet
        if status.relay_url.is_none() {
            status.relay_url = resolved.map(|(url, _)| url);
        }
        status.relay_url_source = source(&status.relay_url);
        status.built_in_url = state.built_in_relay().url().map(str::to_string);
        return Ok(status.without_token());
    }
    let known = last_known_agents(state.db())?;
    let relay_url = resolved.map(|(url, _)| url);
    Ok(RelayStatus {
        instance_id: network::local_instance_id(state.db())?,
        relay_url_source: source(&relay_url),
        built_in_url: state.built_in_relay().url().map(str::to_string),
        relay_url,
        agents_online: u32::try_from(known.len()).unwrap_or(u32::MAX),
        ..RelayStatus::default()
    }
    .without_token())
}

/// Saves the relay URL to `config.json` (empty: no relay) and reconnects in
/// the background. Takes effect now even when `NANDI_RELAY_URL` is set; that
/// wins again at the next launch.
pub async fn set_relay_url(state: &AppState, url: &str) -> Result<(), String> {
    let url = url.trim();
    if !url.is_empty() {
        validate_url(url)?;
    }
    // Another relay's agents are not this one's
    delete_state(state.db(), ONLINE_AGENTS_KEY)?;
    save_url(state.nandi_dir(), state.db(), url)?;
    let Some(client) = state.relay_client().cloned() else {
        return Ok(());
    };
    if url.is_empty() {
        let instance_id = network::local_instance_id(state.db())?;
        return client.connect("", &instance_id).await;
    }
    let url = url.to_string();
    let task_state = state.clone();
    state.runtime().spawn(async move {
        connect_and_register(&task_state, client.as_ref(), &url).await;
    });
    Ok(())
}

/// "Test connection": opens a WebSocket to `url` (up to 5s), sends a ping,
/// and waits up to 2s for the pong; returns the round trip in milliseconds, or the
/// exact error.
pub async fn test_relay_connection(url: &str) -> Result<u32, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Enter a relay URL first".to_string());
    }
    validate_url(url)?;
    client::probe(url, PROBE_CONNECT_TIMEOUT, PROBE_TIMEOUT).await
}

/// The instances at the relay, from its `/instances` endpoint (open relays
/// only: one with a secret answers 401, and the view falls back to the
/// instances it learned from presence).
pub async fn get_relay_instances(state: &AppState) -> Result<Vec<RelayPeer>, String> {
    let url = match state
        .relay_client()
        .and_then(|client| client.status().relay_url)
    {
        Some(url) => url,
        None => resolve_url(state)?
            .map(|(url, _)| url)
            .ok_or(RelayUnavailable)?,
    };
    let endpoint = instances_endpoint(&url)?;
    let response = reqwest::Client::new()
        .get(endpoint.as_str())
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach {endpoint}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{endpoint} answered {}", response.status()));
    }
    let instances: Vec<RelayInstanceInfo> = response
        .json()
        .await
        .map_err(|e| format!("Unexpected answer from {endpoint}: {e}"))?;
    Ok(instances
        .into_iter()
        .map(|instance| RelayPeer {
            instance_id: instance.instance_id,
            agents: u32::try_from(instance.agents.len()).unwrap_or(u32::MAX),
            connected_at: instance.connected_at,
            last_heartbeat: instance.last_heartbeat,
        })
        .collect())
}

/// `ws://host:port/relay?token=…` → `http://host:port/instances`
fn instances_endpoint(relay_url: &str) -> Result<reqwest::Url, String> {
    let (base, _) = split_token(relay_url);
    let mut url =
        reqwest::Url::parse(&base).map_err(|e| format!("Invalid relay URL {base}: {e}"))?;
    let scheme = if url.scheme() == "wss" {
        "https"
    } else {
        "http"
    };
    url.set_scheme(scheme)
        .map_err(|()| format!("Invalid relay URL {base}"))?;
    url.set_path("/instances");
    url.set_query(None);
    Ok(url)
}

fn validate_url(url: &str) -> Result<(), String> {
    if url.starts_with("ws://") || url.starts_with("wss://") {
        Ok(())
    } else {
        Err("The relay URL must start with ws:// or wss://".to_string())
    }
}

/// Cards online at the relay, optionally only those serving `capability`:
/// asked live when connected, else the last known ones.
pub async fn list_online_agents(
    state: &AppState,
    capability: Option<&str>,
) -> Result<Vec<AgentCard>, String> {
    let capability = capability.map(str::trim).filter(|c| !c.is_empty());
    // Gateways' route cards aren't agents; list them only when asked for
    let keep = |cards: Vec<AgentCard>| -> Vec<AgentCard> {
        cards
            .into_iter()
            .filter(|card| capability.is_some() || !registration::is_route_card(card))
            .collect()
    };
    if let Some(client) = state.relay_client() {
        if client.status().connected {
            match client
                .query_capability(capability.unwrap_or(ALL_CAPABILITIES))
                .await
            {
                Ok(cards) => return Ok(keep(cards)),
                Err(e) => log::warn!("Relay query failed; using the last known agents: {e}"),
            }
        }
    }
    let known = last_known_agents(state.db())?;
    Ok(keep(match capability {
        Some(capability) => known
            .into_iter()
            .filter(|card| card.serves(capability))
            .collect(),
        None => known,
    }))
}

/// Re-registers the local cards when an agent changes (accepting
/// delegation, what it serves), so peers see it without a restart or
/// reconnect. Changes within [`REREGISTER_DELAY`] are sent together.
struct Reregister {
    client: Arc<dyn RelayClient>,
    index: Option<Arc<std::sync::RwLock<CapabilityIndex>>>,
    store: RepoStore,
    runtime: tokio::runtime::Handle,
    /// A re-registration is already scheduled
    pending: Arc<std::sync::atomic::AtomicBool>,
}

/// How long agent changes are gathered before re-registering
const REREGISTER_DELAY: Duration = Duration::from_millis(500);

impl EventPublisher for Reregister {
    fn changed(&self, event: &DomainEvent) {
        use std::sync::atomic::Ordering;
        if event.kind != EntityKind::Agent || self.pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let (client, index, store, pending) = (
            self.client.clone(),
            self.index.clone(),
            self.store.clone(),
            self.pending.clone(),
        );
        self.runtime.spawn(async move {
            tokio::time::sleep(REREGISTER_DELAY).await;
            pending.store(false, Ordering::SeqCst);
            let status = client.status();
            let Some(url) = status.relay_url.filter(|_| status.connected) else {
                // Not connected: `connect_and_register` sends them on connect
                return;
            };
            let cards = local_cards_from(index.as_ref(), &store, &status.instance_id, &url);
            match client.register(cards).await {
                Ok(()) => log::info!("Agents changed; re-registered with the relay"),
                Err(e) => log::warn!("Agents changed, but re-registering failed: {e}"),
            }
        });
    }

    fn invalid(&self, _problem: &Problem, _changed_by: ChangedBy) {}
}

/// Connects (retrying in the background on failure) and registers the
/// local cards; the client re-registers them on every reconnect.
async fn connect_and_register(state: &AppState, client: &dyn RelayClient, url: &str) {
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(id) => id,
        Err(e) => return log::error!("Relay: no instance ID: {e}"),
    };
    let connected = client.connect(url, &instance_id).await;
    if let Err(e) = &connected {
        // Not a problem for anything local: the client keeps retrying, and
        // only the Connection view shows it
        log::info!("{e}; retrying in the background");
    }
    let cards = local_cards(state, &instance_id, url);
    // Wave A: the registration carries our capability presence
    let presence = presence::publish(state, client).await;
    presence::spawn_refresh(state, presence);
    // Stores the cards even when offline, for the next reconnect
    match client.register(cards).await {
        Ok(()) => log::info!("Registered with the relay"),
        Err(e) if connected.is_ok() => log::warn!("Relay registration failed: {e}"),
        Err(_) => {}
    }
}

/// The cards this instance offers at the relay: local agents that accept
/// delegation, as far as NANDI.md shares them
/// ([`registration::shared_agent_cards`]).
pub fn local_cards(state: &AppState, instance_id: &str, relay_url: &str) -> Vec<AgentCard> {
    local_cards_from(
        state.capability_index(),
        state.store(),
        instance_id,
        relay_url,
    )
}

pub(crate) fn local_cards_from(
    index: Option<&Arc<std::sync::RwLock<CapabilityIndex>>>,
    store: &RepoStore,
    instance_id: &str,
    relay_url: &str,
) -> Vec<AgentCard> {
    // No NANDI.md or an unreadable one shares nothing
    let manifest = gateway_manifest::load(&store.repos().layout().root)
        .unwrap_or_else(|e| {
            log::warn!("Sharing no agents: {e}");
            None
        })
        .unwrap_or_default();
    registration::shared_agent_cards(
        delegable_cards(index, store, instance_id, relay_url),
        &manifest,
    )
}

/// Every local agent card that accepts delegation, shared or not
fn delegable_cards(
    index: Option<&Arc<std::sync::RwLock<CapabilityIndex>>>,
    store: &RepoStore,
    instance_id: &str,
    relay_url: &str,
) -> Vec<AgentCard> {
    let (base, _) = split_token(relay_url);
    let issued_at = client::now_ms();
    if let Some(index) = index {
        let index = index.read().unwrap_or_else(|e| e.into_inner());
        let mut seen = HashSet::new();
        let cards: Vec<AgentCard> = index
            .list_capabilities()
            .iter()
            .flat_map(|capability| index.find_local(capability))
            .filter(|card| card.accepts_delegation && seen.insert(card.id.clone()))
            .collect();
        if !cards.is_empty() {
            return cards;
        }
    }
    store.read(|repo| {
        repo.agents()
            .filter(|agent| agent.capabilities().accepts_delegation())
            .map(|agent| {
                let endpoint = network::agent_endpoint(&base, instance_id, agent.id().as_str());
                card_from_agent(agent, instance_id, &endpoint).with_issued_at(issued_at)
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Which relay
// ---------------------------------------------------------------------------

/// `NANDI_RELAY_URL`, if set
fn env_url() -> Option<String> {
    crate::utils::env::var(RELAY_URL_ENV)
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// The relay this instance uses, without its instance token: for the
/// endpoints on agent cards. `None` when there is none.
pub fn configured_url(state: &AppState) -> Option<String> {
    env_url()
        .or_else(|| {
            nandi_config::load_from(state.nandi_dir())
                .ok()?
                .relay_url
                .filter(|url| !url.trim().is_empty())
        })
        .map(|url| split_token(&url).0)
}

/// The relay to connect to and where its URL came from (see the module
/// docs), or `None` when there is none.
fn resolve_url(state: &AppState) -> Result<Option<(String, RelayUrlSource)>, String> {
    resolve_url_in(env_url(), state.nandi_dir(), state.db())
}

/// `NANDI_RELAY_URL`, else the saved URL with this instance's token for
/// it, if it has one. Never the build's own relay by itself: that one is
/// used only once the user joined it (which saved it).
fn resolve_url_in(
    env: Option<String>,
    nandi_dir: &Path,
    db: &Db,
) -> Result<Option<(String, RelayUrlSource)>, String> {
    if let Some(url) = env {
        return Ok(Some((url, RelayUrlSource::Env)));
    }
    let config = nandi_config::load_from(nandi_dir)?;
    let Some(base) = config.relay_url.filter(|url| !url.trim().is_empty()) else {
        return Ok(None);
    };
    let token = saved_token_for(db, &base)?;
    Ok(Some((
        with_token(&base, token.as_deref()),
        RelayUrlSource::Config,
    )))
}

/// The instance token kept for the relay `base` (without a token), if any
fn saved_token_for(db: &Db, base: &str) -> Result<Option<String>, String> {
    let relay = read_state::<String>(db, RELAY_TOKEN_RELAY_KEY)?;
    if relay.as_deref() != Some(base) {
        return Ok(None);
    }
    read_state::<String>(db, RELAY_TOKEN_KEY)
}

/// This instance's relay token: the `?token=` of `NANDI_RELAY_URL` when
/// that's set (it wins over everything), else the one kept in
/// `relay_state` (saved with the URL, or from enrolling)
pub fn instance_token(db: &Db) -> Option<String> {
    let env = crate::utils::env::var(RELAY_URL_ENV).filter(|url| !url.trim().is_empty());
    let token = match env {
        Some(url) => split_token(url.trim()).1,
        None => read_state::<String>(db, RELAY_TOKEN_KEY).ok().flatten(),
    };
    token.filter(|token| !token.trim().is_empty())
}

/// Saves `url` (empty: none): the URL to `config.json`, its token (if any)
/// to `relay_state` with the relay it is for. Another relay without a token
/// drops the old token; disconnecting (empty) keeps it, so joining that
/// relay again reuses this instance's seat.
fn save_url(nandi_dir: &Path, db: &Db, url: &str) -> Result<(), String> {
    let (base, token) = split_token(url.trim());
    nandi_config::update_in(nandi_dir, |config| {
        config.relay_url = Some(base.clone()).filter(|url| !url.is_empty());
    })?;
    match token {
        Some(token) => {
            write_state(db, RELAY_TOKEN_KEY, &token)?;
            write_state(db, RELAY_TOKEN_RELAY_KEY, &base)?;
        }
        None if base.is_empty() || saved_token_for(db, &base)?.is_some() => {}
        None => {
            delete_state(db, RELAY_TOKEN_KEY)?;
            delete_state(db, RELAY_TOKEN_RELAY_KEY)?;
        }
    }
    Ok(())
}

fn with_token(base: &str, token: Option<&str>) -> String {
    match token.filter(|token| !token.is_empty()) {
        Some(token) if base.contains('?') => format!("{base}&token={token}"),
        Some(token) => format!("{base}?token={token}"),
        None => base.to_string(),
    }
}

// ---------------------------------------------------------------------------
// relay_state
// ---------------------------------------------------------------------------

fn last_known_agents(db: &Db) -> Result<Vec<AgentCard>, String> {
    Ok(read_state(db, ONLINE_AGENTS_KEY)?.unwrap_or_default())
}

fn read_state<T: serde::de::DeserializeOwned>(db: &Db, key: &str) -> Result<Option<T>, String> {
    let conn = db.conn()?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM relay_state WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Database error: {e}"))?;
    Ok(value.and_then(|value| serde_json::from_str(&value).ok()))
}

fn write_state<T: Serialize + ?Sized>(db: &Db, key: &str, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(|e| format!("Failed to encode {key}: {e}"))?;
    db.conn()?
        .execute(
            "INSERT INTO relay_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )
        .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}

fn delete_state(db: &Db, key: &str) -> Result<(), String> {
    db.conn()?
        .execute("DELETE FROM relay_state WHERE key = ?1", [key])
        .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}
