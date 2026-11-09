//! The gateway agent (wave final, Track D): one per Nandi instance, defined
//! by `~/.nandi/repo/NANDI.md` (YAML frontmatter + a Markdown body). It is
//! what the network sees of this instance (a `GatewayCard` at the relay),
//! takes delegations addressed to the instance, applies the policy, and
//! routes each to a local agent, a local workflow, or a federated gateway.
//! It never executes work itself.
//!
//! - `manifest`: parse / serialize NANDI.md, the default template
//! - `registration`: the card, relay registration, heartbeat, federation
//! - `policy`: rate limit, monthly budget, exposure, approval
//! - `router`: capability lookup, else LLM routing over the manifest body;
//!   `handle_gateway_task` for tasks addressed to the gateway
//! - `chat`: answering questions about this instance (never running work)

pub mod assignments;
pub mod chat;
pub mod holds;
pub mod manifest;
pub mod policy;
pub mod registration;
pub mod router;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use self::policy::RateLimiter;
use self::router::{DefaultRouter, GatewayLlm, LlmGatewayModel};
use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::a2a::queue::SlotQueue;
use crate::services::a2a::task::{Refusal, TaskState};
use crate::services::capability_index::CapabilityIndex;
use crate::services::domain::agent_card::OwnerRef;
use crate::services::domain::AgentCard;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::inbox::audit::Decided;
use crate::services::network;
use crate::services::relay::client::split_token;
use crate::services::relay::protocol::PeerMessage;
use crate::services::relay::RelayClient;
use crate::services::repo_store::RepoStore;
use crate::services::AppState;

// The wire shape lives in `nandi-protocol`
pub use nandi_protocol::{
    Availability, AvailabilityStatus, GatewayCard, GATEWAY_AGENT_ID, GATEWAY_VERSION,
};

/// `gateway://config_changed`
pub const GATEWAY_CONFIG_CHANGED: &str = "gateway://config_changed";
/// `gateway://caller_updated`
pub const GATEWAY_CALLER_UPDATED: &str = "gateway://caller_updated";
/// `gateway://rate_limited`
pub const GATEWAY_RATE_LIMITED: &str = "gateway://rate_limited";

/// Below this, the router asks instead of guessing
pub const MIN_ROUTE_CONFIDENCE: f32 = 0.6;

pub type GatewayError = String;

/// NANDI.md, parsed. `body` is the Markdown after the frontmatter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(default, rename_all = "camelCase")]
pub struct GatewayManifest {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub owner: OwnerRef,
    /// The branch slug (`nandi/<user_slug>`); identity, see `services::auth`
    #[serde(default)]
    pub user_slug: String,
    /// What the owner is responsible for ("SOX audit", "billing system"):
    /// short public statements, published on the card so other people's
    /// agents can find them by topic. Normalized by
    /// `manifest::normalize_responsibilities`.
    pub responsibilities: Vec<String>,
    /// Whether the owner is around: busy or away until a time, and who
    /// covers meanwhile. Published on the card. Parsed and checked by
    /// `manifest::parse`.
    pub availability: Availability,
    pub relay: RelayConfig,
    pub policy: GatewayPolicy,
    pub federation: FederationConfig,
    pub routing: RoutingConfig,
    pub body: String,
}

/// The structured part of NANDI.md the settings form edits (everything but
/// the identity and the body)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(default, rename_all = "camelCase")]
pub struct GatewayConfig {
    pub relay: RelayConfig,
    pub policy: GatewayPolicy,
    pub federation: FederationConfig,
    pub routing: RoutingConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayConfig {
    /// Register the gateway card with the relay
    pub enabled: bool,
    /// Listed in other members' network directory
    pub discoverable: bool,
    /// Put `responsibilities` on the card, so people can find you by them
    pub share_responsibilities: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            discoverable: false,
            share_responsibilities: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FederationConfig {
    pub enabled: bool,
    pub trusted_peers: Vec<OwnerRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayPolicy {
    pub max_concurrent: u32,
    pub rate_limit_per_hour: u32,
    /// Capabilities whose tasks need the owner's approval
    pub require_approval: Vec<String>,
    /// Empty: every local capability is exposed
    pub expose: Vec<String>,
    pub hide: Vec<String>,
}

impl Default for GatewayPolicy {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            rate_limit_per_hour: 30,
            require_approval: Vec::new(),
            expose: Vec::new(),
            hide: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoutingConfig {
    /// Capability → how to pick its provider
    pub preferences: HashMap<String, CapabilityPreference>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPreference {
    /// Instance IDs, boosted
    pub preferred: Vec<String>,
    /// Instance IDs, never picked
    pub excluded: Vec<String>,
    pub strategy: Option<RouteStrategy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RouteStrategy {
    Fastest,
    Cheapest,
    MostTrusted,
    HighestSuccessRate,
}

/// One caller instance's allowance (a `gateway_callers` row)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(default, rename_all = "camelCase")]
pub struct CallerPolicy {
    pub caller_instance: String,
    pub caller_owner: String,
    /// Empty: whatever the gateway exposes
    pub allowed_capabilities: Vec<String>,
    pub max_concurrent: u32,
    pub monthly_task_budget: Option<u32>,
    pub disabled: bool,
    /// Epoch ms
    pub created_at: Timestamp,
}

/// A caller's use this month (a `gateway_usage` row)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsage {
    pub caller_instance: String,
    /// `YYYY-MM`
    pub month: String,
    pub task_count: u32,
    pub monthly_task_budget: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    /// `relay.enabled` in NANDI.md
    pub enabled: bool,
    /// The relay accepted the card
    pub registered: bool,
    pub discoverable: bool,
    pub card: Option<GatewayCard>,
    /// Gateway tasks in flight / waiting for a slot
    pub active_tasks: u32,
    pub queued_tasks: u32,
    pub last_heartbeat_at: Option<Timestamp>,
}

/// An inbound request addressed to the gateway
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayRequest {
    pub capability: Option<String>,
    pub natural_language_request: String,
    pub input: Value,
    pub caller: CallerPolicy,
    pub submitted_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RouteDecision {
    TargetAgent {
        agent_id: String,
        reason: String,
        confidence: f32,
    },
    TargetWorkflow {
        workflow_id: String,
        reason: String,
        confidence: f32,
    },
    TargetRemoteGateway {
        instance_id: String,
        reason: String,
        confidence: f32,
    },
    Reject {
        reason: String,
    },
    NeedsClarification {
        question: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogWorkflow {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// What the router may pick from
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCatalog {
    pub agents: Vec<AgentCard>,
    pub workflows: Vec<CatalogWorkflow>,
    pub remote_gateways: Vec<GatewayCard>,
}

/// `BoxFuture`-returning so it's object-safe.
pub trait GatewayRouter: Send + Sync {
    fn route<'a>(
        &'a self,
        request: &'a GatewayRequest,
        manifest: &'a GatewayManifest,
        catalog: &'a LocalCatalog,
    ) -> BoxFuture<'a, Result<RouteDecision, GatewayError>>;
}

/// `gateway://config_changed`: NANDI.md changed (from the app or on disk)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfigChanged {
    pub manifest: GatewayManifest,
}

impl NandiEvent for GatewayConfigChanged {
    const NAME: &'static str = GATEWAY_CONFIG_CHANGED;
}

/// `gateway://caller_updated`: a caller's policy was set, created on first
/// contact, or removed (`policy: null`)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayCallerUpdated {
    pub caller_instance: String,
    pub policy: Option<CallerPolicy>,
}

impl NandiEvent for GatewayCallerUpdated {
    const NAME: &'static str = GATEWAY_CALLER_UPDATED;
}

/// What the gateway runs on
pub struct GatewayDeps {
    /// The repo root, where NANDI.md lives
    pub root: PathBuf,
    pub db: Db,
    pub bus: EventBus,
    pub store: RepoStore,
    pub instance_id: String,
    pub relay: Option<Arc<dyn RelayClient>>,
    pub index: Option<Arc<RwLock<CapabilityIndex>>>,
    pub router: Arc<dyn GatewayRouter>,
    /// For chat replies
    pub chat: Arc<dyn GatewayLlm>,
}

/// What changes while the gateway runs
#[derive(Default)]
struct Runtime {
    /// NANDI.md's mtime as last seen
    modified: Option<SystemTime>,
    /// The relay took the last card
    registered: bool,
    /// The relay was sent the withdrawn card since sharing went off. False
    /// at start, so a card left from before (sharing turned off while Nandi
    /// was closed) is withdrawn too.
    withdrawn: bool,
    /// The agent cards last registered for sharing, as (card, capabilities)
    shared_agents: Option<Vec<(String, Vec<String>)>>,
    last_heartbeat_at: Option<Timestamp>,
    /// Trusted peers' cards (federation on)
    peers: Vec<GatewayCard>,
    active: u32,
    active_by_caller: HashMap<String, u32>,
}

/// The gateway (Track D): NANDI.md, the card at the relay, policy state,
/// and the router. Background work (change polling, heartbeat,
/// federation) holds it weakly and stops when it's dropped.
pub struct GatewayService {
    deps: GatewayDeps,
    limiter: RateLimiter,
    runtime: Mutex<Runtime>,
    /// Requests held for the owner's approval, waiting for a decision
    holds: holds::HoldWaiters,
    assignments: assignments::AssignmentWaiters,
    /// Requests waiting for a free slot (`policy.max_concurrent`)
    slots: SlotQueue,
}

/// The line for gateway slots in [`GatewayService::slots`] (one line)
pub const GATEWAY_LINE: &str = "gateway";

/// A running gateway task's place in the concurrency limits; released on
/// drop
pub struct GatewaySlot<'a> {
    service: &'a GatewayService,
    caller: String,
}

impl Drop for GatewaySlot<'_> {
    fn drop(&mut self) {
        let mut runtime = self.service.lock();
        runtime.active = runtime.active.saturating_sub(1);
        if let Some(count) = runtime.active_by_caller.get_mut(&self.caller) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                runtime.active_by_caller.remove(&self.caller);
            }
        }
    }
}

impl GatewayService {
    pub fn new(deps: GatewayDeps) -> Self {
        let modified = manifest::modified(&deps.root);
        Self {
            deps,
            limiter: RateLimiter::default(),
            runtime: Mutex::new(Runtime {
                modified,
                ..Runtime::default()
            }),
            holds: holds::HoldWaiters::default(),
            assignments: assignments::AssignmentWaiters::default(),
            slots: SlotQueue::default(),
        }
    }

    pub fn holds(&self) -> &holds::HoldWaiters {
        &self.holds
    }

    pub fn assignments(&self) -> &assignments::AssignmentWaiters {
        &self.assignments
    }

    pub fn slots(&self) -> &SlotQueue {
        &self.slots
    }

    pub fn db(&self) -> &Db {
        &self.deps.db
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.runtime.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn instance_id(&self) -> &str {
        &self.deps.instance_id
    }

    pub fn bus(&self) -> &EventBus {
        &self.deps.bus
    }

    pub fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    pub fn router(&self) -> &Arc<dyn GatewayRouter> {
        &self.deps.router
    }

    /// NANDI.md as it is on disk (the default if it's missing)
    pub fn manifest(&self) -> Result<GatewayManifest, GatewayError> {
        Ok(manifest::load(&self.deps.root)?
            .unwrap_or_else(|| manifest::parse(&manifest::render_default("")).unwrap_or_default()))
    }

    /// Writes NANDI.md, then announces it and re-registers
    pub fn save_manifest(
        self: &Arc<Self>,
        manifest: &GatewayManifest,
    ) -> Result<GatewayManifest, GatewayError> {
        validate(manifest)?;
        let saved = manifest::save(&self.deps.root, manifest)?;
        self.lock().modified = manifest::modified(&self.deps.root);
        self.changed(saved.clone());
        Ok(saved)
    }

    fn changed(self: &Arc<Self>, manifest: GatewayManifest) {
        events::emit(&self.deps.bus, &GatewayConfigChanged { manifest });
        let service = self.clone();
        tokio::spawn(async move { service.sync().await });
    }

    /// Whether NANDI.md changed on disk since last seen
    /// Puts NANDI.md's owner (display name and slug, from onboarding) on
    /// the local agents' cards, and re-registers them when it changed.
    /// Part of every `sync`: at start, after a save, and after an edit.
    async fn refresh_owner(&self, manifest: &GatewayManifest) {
        let Some(index) = &self.deps.index else {
            return;
        };
        let owner = registration::card_owner(manifest);
        if *index.read().unwrap_or_else(|e| e.into_inner()).owner() == owner {
            return;
        }
        let agents: Vec<crate::services::domain::Agent> = self
            .deps
            .store
            .read(|repo| repo.agents().cloned().collect());
        let changed = index
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .set_owner(owner, &agents);
        match changed {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => return log::warn!("Failed to update the agent cards' owner: {e}"),
        }
        let Some(relay) = &self.deps.relay else {
            return;
        };
        let status = relay.status();
        let Some(url) = status.relay_url.filter(|_| status.connected) else {
            // Not connected: the cards go out on connect
            return;
        };
        let cards = crate::services::relay::local_cards_from(
            Some(index),
            &self.deps.store,
            &status.instance_id,
            &url,
        );
        if let Err(e) = relay.register(cards).await {
            log::warn!("Owner changed, but re-registering the agents failed: {e}");
        }
    }

    /// Re-registers the agent cards when what NANDI.md shares of them
    /// changed: sharing turned on or off, or `expose` / `hide` edited
    async fn share_agents(&self, relay: &dyn RelayClient) {
        let status = relay.status();
        let Some(url) = status.relay_url.filter(|_| status.connected) else {
            // Not connected: the cards go out on connect
            return;
        };
        let cards = crate::services::relay::local_cards_from(
            self.deps.index.as_ref(),
            &self.deps.store,
            &status.instance_id,
            &url,
        );
        let shared: Vec<(String, Vec<String>)> = cards
            .iter()
            .map(|card| {
                let served = card.capabilities.iter().map(|c| c.name.clone()).collect();
                (card.id.clone(), served)
            })
            .collect();
        {
            let mut runtime = self.lock();
            if runtime.shared_agents.as_ref() == Some(&shared) {
                return;
            }
            runtime.shared_agents = Some(shared);
        }
        if let Err(e) = relay.register(cards).await {
            log::warn!("Sharing changed, but re-registering the agents failed: {e}");
            self.lock().shared_agents = None;
        }
    }

    fn changed_on_disk(&self) -> bool {
        let modified = manifest::modified(&self.deps.root);
        let mut runtime = self.lock();
        if runtime.modified == modified {
            return false;
        }
        runtime.modified = modified;
        true
    }

    /// Local agents' cards (each once)
    fn local_cards(&self) -> Vec<AgentCard> {
        let Some(index) = &self.deps.index else {
            return Vec::new();
        };
        let index = index.read().unwrap_or_else(|e| e.into_inner());
        let mut seen = HashSet::new();
        index
            .list_capabilities()
            .iter()
            .flat_map(|capability| index.find_local(capability))
            .filter(|card| seen.insert(card.id.clone()))
            .collect()
    }

    /// The relay URL cards point at (without its token)
    fn relay_url(&self) -> String {
        let configured = self
            .deps
            .relay
            .as_ref()
            .and_then(|relay| relay.status().relay_url)
            .map(|url| split_token(&url).0);
        configured
            .or_else(|| {
                self.deps.index.as_ref().map(|index| {
                    let index = index.read().unwrap_or_else(|e| e.into_inner());
                    index.relay_url().to_string()
                })
            })
            .unwrap_or_default()
    }

    /// This instance's card as of now
    pub async fn card(&self, manifest: &GatewayManifest) -> GatewayCard {
        let instance = self.deps.instance_id.clone();
        let load = self
            .deps
            .db
            .call(move |conn| registration::current_load(conn, &instance))
            .await
            .unwrap_or_else(|e| {
                log::warn!("Gateway load unavailable: {e}");
                0
            });
        registration::build_card(
            manifest,
            &self.deps.instance_id,
            &self.relay_url(),
            &self.local_cards(),
            load,
        )
    }

    /// What the router may pick from: local agents (only their exposed
    /// capabilities), local workflows, and (federation on) trusted peers
    pub async fn catalog(&self, manifest: &GatewayManifest, include_remote: bool) -> LocalCatalog {
        let agents = registration::exposed_cards(self.local_cards(), &manifest.policy);
        let workflows = self.deps.store.read(|repo| {
            repo.workflows()
                .map(|workflow| {
                    let agents: Vec<&str> =
                        workflow.steps().iter().map(|s| s.performer()).collect();
                    CatalogWorkflow {
                        id: workflow.id().to_string(),
                        name: workflow.name().to_string(),
                        description: format!("{} steps: {}", agents.len(), agents.join(" → ")),
                    }
                })
                .collect()
        });
        let remote_gateways = if include_remote && manifest.federation.enabled {
            self.lock().peers.clone()
        } else {
            Vec::new()
        };
        LocalCatalog {
            agents,
            workflows,
            remote_gateways,
        }
    }

    /// The caller's row, created with the defaults on first contact
    pub async fn caller_policy(
        &self,
        caller_instance: &str,
        caller_owner: &str,
    ) -> Result<CallerPolicy, GatewayError> {
        let (instance, owner) = (caller_instance.to_string(), caller_owner.to_string());
        let (policy, created) = self
            .deps
            .db
            .call(move |conn| {
                if let Some(existing) = policy::get_caller(conn, &instance)? {
                    return Ok((existing, false));
                }
                let fresh = policy::default_caller(&instance, &owner, now_ms());
                policy::upsert_caller(conn, &fresh)?;
                Ok((fresh, true))
            })
            .await?;
        if created {
            events::emit(
                &self.deps.bus,
                &GatewayCallerUpdated {
                    caller_instance: policy.caller_instance.clone(),
                    policy: Some(policy.clone()),
                },
            );
        }
        Ok(policy)
    }

    pub async fn usage_count(&self, caller: &str, month: &str) -> Result<u32, GatewayError> {
        let (caller, month) = (caller.to_string(), month.to_string());
        self.deps
            .db
            .call(move |conn| policy::usage_count(conn, &caller, &month))
            .await
    }

    pub async fn record_usage(&self, caller: &str, month: &str) -> Result<(), GatewayError> {
        let (caller, month) = (caller.to_string(), month.to_string());
        self.deps
            .db
            .call(move |conn| policy::increment_usage(conn, &caller, &month))
            .await
    }

    /// A slot within `policy.max_concurrent` and the caller's own limit
    pub fn acquire(
        &self,
        caller: &CallerPolicy,
        manifest: &GatewayManifest,
    ) -> Option<GatewaySlot<'_>> {
        let mut runtime = self.lock();
        let mine = runtime
            .active_by_caller
            .get(&caller.caller_instance)
            .copied()
            .unwrap_or(0);
        if runtime.active >= manifest.policy.max_concurrent.max(1)
            || mine >= caller.max_concurrent.max(1)
        {
            return None;
        }
        runtime.active += 1;
        *runtime
            .active_by_caller
            .entry(caller.caller_instance.clone())
            .or_default() += 1;
        Some(GatewaySlot {
            service: self,
            caller: caller.caller_instance.clone(),
        })
    }

    /// Tells a remote requester its task was refused
    pub async fn reject_remote(&self, caller_instance: &str, task_id: &str, refusal: &Refusal) {
        if let Some(relay) = &self.deps.relay {
            let message = router::reject_message(task_id, refusal);
            if let Err(e) = relay.send_peer(caller_instance.to_string(), message).await {
                log::warn!("Couldn't tell {caller_instance} task {task_id} was refused: {e}");
            }
        }
    }

    /// Progress for a remote requester
    pub async fn send_progress(
        &self,
        caller_instance: &str,
        task_id: &str,
        percent: u8,
        note: String,
    ) {
        if let Some(relay) = &self.deps.relay {
            let message = PeerMessage::ProgressUpdate {
                task_id: task_id.to_string(),
                percent: Some(percent),
                note: Some(note),
                reason_kind: None,
                queue_position: None,
            };
            if let Err(e) = relay.send_peer(caller_instance.to_string(), message).await {
                log::debug!("Progress for {task_id} not sent: {e}");
            }
        }
    }

    pub async fn status(&self) -> Result<GatewayStatus, GatewayError> {
        let manifest = self.manifest()?;
        let card = self.card(&manifest).await;
        let instance = self.deps.instance_id.clone();
        let waiting = self
            .deps
            .db
            .call(move |conn| queued_tasks(conn, &instance))
            .await
            .unwrap_or(0);
        let runtime = self.lock();
        Ok(GatewayStatus {
            enabled: manifest.relay.enabled,
            registered: runtime.registered && manifest.relay.enabled,
            discoverable: manifest.relay.discoverable,
            card: Some(card),
            active_tasks: runtime.active,
            queued_tasks: waiting,
            last_heartbeat_at: runtime.last_heartbeat_at,
        })
    }

    /// Registers (or re-sends) the card when `relay.enabled`; withdraws it
    /// (a card that doesn't accept delegation) when that was turned off
    pub async fn sync(&self) {
        let manifest = match self.manifest() {
            Ok(manifest) => manifest,
            Err(e) => {
                log::warn!("Gateway not registered: {e}");
                return;
            }
        };
        self.refresh_owner(&manifest).await;
        let Some(relay) = self.deps.relay.clone() else {
            return;
        };
        self.share_agents(relay.as_ref()).await;
        if !manifest.relay.enabled && self.lock().withdrawn {
            self.lock().registered = false;
            return;
        }
        let mut card = self.card(&manifest).await;
        let accepting = card.accepts_delegation;
        if !accepting {
            card = registration::withdrawn_card(card);
        }
        let sent = relay.register_gateway(card).await;
        let mut runtime = self.lock();
        runtime.withdrawn = !accepting;
        runtime.registered = accepting && sent.is_ok();
        if sent.is_ok() {
            runtime.last_heartbeat_at = Some(now_ms() as f64);
        } else if let Err(e) = sent {
            log::debug!("Gateway card not sent yet (re-sent on connect): {e}");
        }
    }

    /// Refreshes the trusted peers' cards (federation on)
    pub async fn refresh_peers(&self) {
        let Ok(manifest) = self.manifest() else {
            return;
        };
        if !manifest.federation.enabled || manifest.federation.trusted_peers.is_empty() {
            self.lock().peers.clear();
            return;
        }
        let Some(relay) = self.deps.relay.clone() else {
            return;
        };
        match relay.query_directory().await {
            Ok(members) => {
                let online = members
                    .into_iter()
                    .filter(|member| member.online)
                    .map(|member| member.card)
                    .collect();
                let peers = registration::trusted_cards(
                    online,
                    &manifest.federation.trusted_peers,
                    &self.deps.instance_id,
                );
                self.lock().peers = peers;
            }
            Err(e) => log::debug!("Federated gateways not refreshed: {e}"),
        }
    }

    /// The owner chatting with their own gateway (local only)
    pub async fn chat(&self, message: &str) -> Result<String, GatewayError> {
        let manifest = self.manifest()?;
        let mut catalog = self.catalog(&manifest, true).await;
        // The owner may ask anyone on their network, not only federated
        // peers: list the online members whose gateway takes requests
        chat::add_network_gateways(
            &mut catalog.remote_gateways,
            crate::services::network::known_members(&self.deps.db),
            &self.deps.instance_id,
        );
        let status = self.status().await?;
        chat::reply(
            self.deps.chat.as_ref(),
            &manifest,
            &catalog,
            &status,
            message,
        )
        .await
    }
}

/// Gateway tasks waiting to start
fn queued_tasks(conn: &rusqlite::Connection, instance_id: &str) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE provider_instance = ?1
           AND provider_agent IN (?2, ?3) AND state IN (?4, ?5, ?6)",
        rusqlite::params![
            instance_id,
            GATEWAY_AGENT_ID,
            AgentCard::card_id(instance_id, GATEWAY_AGENT_ID),
            TaskState::Submitted.as_str(),
            TaskState::Delivered.as_str(),
            TaskState::Acknowledged.as_str(),
        ],
        |row| row.get(0),
    )
}

fn validate(manifest: &GatewayManifest) -> Result<(), GatewayError> {
    if manifest.policy.max_concurrent == 0 {
        return Err("The gateway needs to run at least one task at a time".to_string());
    }
    Ok(())
}

/// Polls NANDI.md, heartbeats the card, and follows trusted peers until
/// the service is dropped
fn spawn_background(service: &Arc<GatewayService>, runtime: &tokio::runtime::Handle) {
    let weak = Arc::downgrade(service);
    runtime.spawn(async move {
        let mut tick = tokio::time::interval(registration::POLL_INTERVAL);
        let mut heartbeat_at: Option<Instant> = None;
        let mut federation_at: Option<Instant> = None;
        loop {
            tick.tick().await;
            let Some(service) = weak.upgrade() else {
                break;
            };
            let changed = service.changed_on_disk();
            if changed {
                match service.manifest() {
                    Ok(manifest) => {
                        log::info!("NANDI.md changed; re-registering the gateway");
                        events::emit(&service.deps.bus, &GatewayConfigChanged { manifest });
                    }
                    Err(e) => log::warn!("NANDI.md is invalid: {e}"),
                }
            }
            let due = |at: Option<Instant>, every: std::time::Duration| {
                at.is_none_or(|at| at.elapsed() >= every)
            };
            if changed || due(federation_at, registration::FEDERATION_INTERVAL) {
                service.refresh_peers().await;
                federation_at = Some(Instant::now());
            }
            if changed || due(heartbeat_at, registration::HEARTBEAT_INTERVAL) {
                service.sync().await;
                heartbeat_at = Some(Instant::now());
            }
        }
    });
}

fn service(state: &AppState) -> Result<&Arc<GatewayService>, GatewayError> {
    state
        .gateway()
        .ok_or_else(|| "The gateway is unavailable".to_string())
}

/// Ensures NANDI.md exists (see `manifest::DEFAULT_TEMPLATE`), then
/// registers the card when `relay.enabled` and starts the heartbeat and
/// change polling. Must not block: relay work runs in the background.
pub fn init(state: &AppState) -> Option<Arc<GatewayService>> {
    let root = state.store().repos().layout().root.clone();
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Gateway disabled: no instance ID ({e})");
            return None;
        }
    };
    // Test states start without NANDI.md, so tests of the repo's git status
    // see only their own files (`ensure_default` has its own tests)
    if !cfg!(test) {
        match manifest::ensure_default(&root) {
            Ok(true) => log::info!("Created {}", manifest::manifest_path(&root).display()),
            Ok(false) => {}
            Err(e) => log::error!("{e}"),
        }
    }
    let router_llm: Arc<dyn GatewayLlm> = Arc::new(LlmGatewayModel::new(
        state.secrets().clone(),
        state.store().clone(),
    ));
    let chat_llm: Arc<dyn GatewayLlm> = Arc::new(LlmGatewayModel::for_chat(
        state.secrets().clone(),
        state.store().clone(),
    ));
    register_executor_hook(state, &instance_id);
    let service = Arc::new(GatewayService::new(GatewayDeps {
        root,
        db: state.db().clone(),
        bus: state.bus().clone(),
        store: state.store().clone(),
        instance_id,
        relay: state.relay_client().cloned(),
        index: state.capability_index().cloned(),
        router: Arc::new(DefaultRouter::new(router_llm)),
        chat: chat_llm,
    }));
    spawn_background(&service, state.runtime());
    Some(service)
}

/// Tasks for `<instance>/gateway` reach the executor (delegation and the
/// task service, both built before the gateway) like any other; this hands
/// them to [`router::handle_gateway_task`]. Holds the state weakly: the
/// state owns the executors.
fn register_executor_hook(state: &AppState, instance_id: &str) {
    let weak = state.downgrade();
    crate::services::a2a::executor::set_gateway_handler(
        instance_id,
        Arc::new(move |task| {
            let weak = weak.clone();
            Box::pin(async move {
                let state = weak.upgrade().ok_or_else(|| {
                    Refusal::new(
                        crate::services::a2a::task::ReasonKind::Failed,
                        crate::services::a2a::task::TaskStage::Admit,
                        "Sharing is turned off on this Nandi right now",
                    )
                })?;
                router::handle_gateway_task(&state, task).await
            })
        }),
    );
}

/// Requests waiting for the owner's approval, oldest first (the Inbox)
pub async fn list_gateway_holds(state: &AppState) -> Result<Vec<holds::GatewayHold>, GatewayError> {
    state.db().call(|conn| holds::pending(conn)).await
}

/// Approves (routing goes on) or declines (the request is rejected) a held
/// request. `note` goes back to the requester with it: a refusal's reason,
/// or an approval's condition. Deciding one that's no longer pending is an
/// error. Recorded in the decisions file (`inbox::audit`).
pub async fn decide_gateway_hold(
    state: &AppState,
    task_id: String,
    approve: bool,
    note: Option<String>,
) -> Result<holds::GatewayHold, GatewayError> {
    let status = if approve {
        holds::HoldStatus::Approved
    } else {
        holds::HoldStatus::Rejected
    };
    let id = task_id.clone();
    let decided = state
        .db()
        .call(move |conn| holds::decide(conn, &id, status, now_ms()))
        .await?
        .ok_or_else(|| "This request was already decided, or it ended".to_string())?;
    if let Some(service) = state.gateway() {
        service.holds().set_note(&task_id, note.as_deref());
        service.holds().wake(&task_id, status);
    }
    crate::services::inbox::audit::record(
        state,
        Decided::new(
            "gatewayHold",
            &task_id,
            if approve { "approved" } else { "declined" },
        )
        .subject(hold_subject(&decided))
        .note(note.as_deref()),
    );
    events::emit(
        state.bus(),
        &holds::GatewayHoldChanged {
            hold: decided.clone(),
        },
    );
    Ok(decided)
}

/// "Sam Lee, ask-me: <their request>", for the decisions file
fn hold_subject(hold: &holds::GatewayHold) -> String {
    let what = hold.summary.lines().next().unwrap_or_default();
    match &hold.capability {
        Some(capability) => format!("{}, {capability}: {what}", hold.caller_name),
        None => format!("{}: {what}", hold.caller_name),
    }
}

/// Work assigned to the owner that's still open, soonest due first (the
/// Inbox)
pub async fn list_gateway_assignments(
    state: &AppState,
) -> Result<Vec<assignments::GatewayAssignment>, GatewayError> {
    state.db().call(|conn| assignments::open_list(conn)).await
}

/// Marks an assignment Done (the requester gets `note` as the result) or
/// Declines it (their task is refused, with `note` as the reason). Closing
/// one that isn't open is an error.
pub async fn close_gateway_assignment(
    state: &AppState,
    task_id: String,
    done: bool,
    note: Option<String>,
) -> Result<assignments::GatewayAssignment, GatewayError> {
    let status = if done {
        assignments::AssignmentStatus::Done
    } else {
        assignments::AssignmentStatus::Declined
    };
    let id = task_id.clone();
    let closed = state
        .db()
        .call(move |conn| assignments::close(conn, &id, status, note.as_deref(), now_ms()))
        .await?
        .ok_or_else(|| "This assignment was already closed, or it ended".to_string())?;
    if let Some(service) = state.gateway() {
        service.assignments().wake(&task_id, status);
    }
    crate::services::inbox::audit::record(
        state,
        Decided::new(
            "assignment",
            &task_id,
            if done { "done" } else { "declined" },
        )
        .subject(format!(
            "{}: {}",
            closed.caller_name,
            closed.request.lines().next().unwrap_or_default()
        ))
        .note(closed.note.as_deref()),
    );
    events::emit(
        state.bus(),
        &assignments::GatewayAssignmentChanged {
            assignment: closed.clone(),
        },
    );
    Ok(closed)
}

pub async fn get_gateway_status(state: &AppState) -> Result<GatewayStatus, GatewayError> {
    service(state)?.status().await
}

/// "Make reachable" (Network view, onboarding): turns on `relay.enabled`
/// and `relay.discoverable`, re-registers the card right away, and returns
/// the status (`registered`: the relay took it)
pub async fn make_gateway_reachable(state: &AppState) -> Result<GatewayStatus, GatewayError> {
    let service = service(state)?;
    let mut manifest = service.manifest()?;
    if !(manifest.relay.enabled && manifest.relay.discoverable) {
        manifest.relay.enabled = true;
        manifest.relay.discoverable = true;
        service.save_manifest(&manifest)?;
    }
    service.sync().await;
    service.status().await
}

fn config_of(manifest: &GatewayManifest) -> GatewayConfig {
    GatewayConfig {
        relay: manifest.relay.clone(),
        policy: manifest.policy.clone(),
        federation: manifest.federation.clone(),
        routing: manifest.routing.clone(),
    }
}

pub async fn get_gateway_config(state: &AppState) -> Result<GatewayConfig, GatewayError> {
    Ok(config_of(&service(state)?.manifest()?))
}

/// Replaces the frontmatter's relay / policy / federation / routing; the
/// identity fields and the body stay as they are
pub async fn set_gateway_config(
    state: &AppState,
    config: GatewayConfig,
) -> Result<GatewayConfig, GatewayError> {
    let service = service(state)?;
    let mut manifest = service.manifest()?;
    manifest.relay = config.relay;
    manifest.policy = config.policy;
    manifest.federation = config.federation;
    manifest.routing = config.routing;
    Ok(config_of(&service.save_manifest(&manifest)?))
}

pub async fn get_gateway_manifest(state: &AppState) -> Result<GatewayManifest, GatewayError> {
    service(state)?.manifest()
}

pub async fn set_gateway_manifest(
    state: &AppState,
    manifest: GatewayManifest,
) -> Result<GatewayManifest, GatewayError> {
    service(state)?.save_manifest(&manifest)
}

pub async fn list_gateway_callers(state: &AppState) -> Result<Vec<CallerPolicy>, GatewayError> {
    state.db().call(|conn| policy::list_callers(conn)).await
}

pub async fn set_caller_policy(
    state: &AppState,
    policy: CallerPolicy,
) -> Result<CallerPolicy, GatewayError> {
    let mut policy = policy;
    policy.caller_instance = policy.caller_instance.trim().to_string();
    if policy.caller_instance.is_empty() {
        return Err("Name the caller's instance".to_string());
    }
    policy.allowed_capabilities = policy
        .allowed_capabilities
        .iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    if policy.created_at <= 0.0 {
        policy.created_at = now_ms() as f64;
    }
    let row = policy.clone();
    let saved = state
        .db()
        .call(move |conn| {
            policy::upsert_caller(conn, &row)?;
            policy::get_caller(conn, &row.caller_instance)
        })
        .await?
        .ok_or_else(|| "The caller policy wasn't saved".to_string())?;
    events::emit(
        state.bus(),
        &GatewayCallerUpdated {
            caller_instance: saved.caller_instance.clone(),
            policy: Some(saved.clone()),
        },
    );
    Ok(saved)
}

pub async fn remove_caller_policy(
    state: &AppState,
    caller_instance: &str,
) -> Result<(), GatewayError> {
    let instance = caller_instance.to_string();
    let removed = state
        .db()
        .call(move |conn| policy::remove_caller(conn, &instance))
        .await?;
    if !removed {
        return Err(format!("No policy for {caller_instance}"));
    }
    events::emit(
        state.bus(),
        &GatewayCallerUpdated {
            caller_instance: caller_instance.to_string(),
            policy: None,
        },
    );
    Ok(())
}

/// This month's use
pub async fn get_gateway_usage(
    state: &AppState,
    caller_instance: &str,
) -> Result<GatewayUsage, GatewayError> {
    let (instance, month) = (caller_instance.to_string(), policy::month_of(now_ms()));
    state
        .db()
        .call(move |conn| policy::usage(conn, &instance, &month))
        .await
}

/// Talks with the gateway: answers from NANDI.md, the catalog, the queue,
/// and the trusted peers; never runs work. No command calls it yet (see
/// the merge notes in `GatewayChat.tsx`).
pub async fn gateway_chat(state: &AppState, message: &str) -> Result<String, GatewayError> {
    service(state)?.chat(message).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;

    #[tokio::test]
    async fn serves_the_config_from_nandi_md() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let root = state.store().repos().layout().root.clone();
        // Missing NANDI.md reads as the default template
        assert!(!manifest::manifest_path(&root).exists());

        let mut config = get_gateway_config(&state).await.unwrap();
        assert_eq!(config.policy.require_approval, ["assign-me"]);
        assert_eq!(config.policy.max_concurrent, 3);
        let body = get_gateway_manifest(&state).await.unwrap().body;
        config.policy.rate_limit_per_hour = 5;
        config.policy.hide = vec!["secret".into()];
        let saved = set_gateway_config(&state, config.clone()).await.unwrap();
        assert_eq!(saved, config);
        assert!(manifest::manifest_path(&root).exists());
        let manifest = get_gateway_manifest(&state).await.unwrap();
        assert_eq!(manifest.body, body);
        assert_eq!(manifest.policy.rate_limit_per_hour, 5);
        assert!(recorder.names().contains(&GATEWAY_CONFIG_CHANGED));

        config.policy.max_concurrent = 0;
        assert!(set_gateway_config(&state, config).await.is_err());

        let status = get_gateway_status(&state).await.unwrap();
        assert!(!status.enabled && !status.registered);
        assert_eq!(status.card.unwrap().max_concurrent, 3);
        state.shutdown();
    }

    #[tokio::test]
    async fn onboarding_puts_the_owner_on_agent_cards() {
        use crate::services::domain::{Agent, AgentId, EntityKind};
        use crate::services::repos::Repository;

        let state = AppState::for_tests(vec![]);
        let agent = Agent::from_skill_md(
            AgentId::parse("papers").unwrap(),
            "---\nname: Papers\nserves: [{name: find-papers}]\naccepts_delegation: true\n---\n",
        )
        .unwrap();
        state.store().repos().agents.save(&agent).unwrap();
        state.store().commit(EntityKind::Agent, "papers").unwrap();
        let card_owner = || {
            state
                .capability_index()
                .unwrap()
                .read()
                .unwrap()
                .find_local("find-papers")[0]
                .owner
                .clone()
        };
        assert_eq!(card_owner(), OwnerRef::default(), "nobody yet");

        // Onboarding writes NANDI.md; the gateway's next sync (at start,
        // on a change, or on the heartbeat) picks it up
        crate::services::auth::set_user_slug(&state, "ada-l".into())
            .await
            .unwrap();
        crate::services::auth::set_display_name(&state, "Ada Lovelace".into())
            .await
            .unwrap();
        state.gateway().unwrap().sync().await;
        assert_eq!(
            card_owner(),
            OwnerRef {
                github_handle: "ada-l".into(),
                display_name: "Ada Lovelace".into(),
            }
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn caller_policies_round_trip_with_events() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        assert!(list_gateway_callers(&state).await.unwrap().is_empty());
        let saved = set_caller_policy(
            &state,
            CallerPolicy {
                caller_instance: " bob-laptop ".into(),
                allowed_capabilities: vec![" summarize ".into(), "".into()],
                max_concurrent: 2,
                monthly_task_budget: Some(10),
                ..CallerPolicy::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(saved.caller_instance, "bob-laptop");
        assert_eq!(saved.allowed_capabilities, ["summarize"]);
        assert!(saved.created_at > 0.0);
        assert_eq!(list_gateway_callers(&state).await.unwrap(), [saved]);
        let usage = get_gateway_usage(&state, "bob-laptop").await.unwrap();
        assert_eq!((usage.task_count, usage.monthly_task_budget), (0, Some(10)));
        remove_caller_policy(&state, "bob-laptop").await.unwrap();
        assert!(remove_caller_policy(&state, "bob-laptop").await.is_err());
        let updates = recorder
            .names()
            .into_iter()
            .filter(|name| *name == GATEWAY_CALLER_UPDATED)
            .count();
        assert_eq!(updates, 2);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_slot_is_held_per_task_within_both_limits() {
        let state = AppState::for_tests(vec![]);
        let service = state.gateway().unwrap().clone();
        let manifest = GatewayManifest {
            policy: GatewayPolicy {
                max_concurrent: 2,
                ..GatewayPolicy::default()
            },
            ..GatewayManifest::default()
        };
        let alice = policy::default_caller("alice", "", 0);
        let bob = policy::default_caller("bob", "", 0);
        let first = service.acquire(&alice, &manifest).unwrap();
        // Alice's own limit is 1
        assert!(service.acquire(&alice, &manifest).is_none());
        let second = service.acquire(&bob, &manifest).unwrap();
        let carol = policy::default_caller("carol", "", 0);
        assert!(service.acquire(&carol, &manifest).is_none());
        drop(first);
        assert!(service.acquire(&alice, &manifest).is_some());
        drop(second);
        state.shutdown();
    }
}
