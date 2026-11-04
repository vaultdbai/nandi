//! Capability selection (wave final, Track D): ranks the providers of a
//! capability (many per capability is the point) and keeps the circuit
//! breakers failover respects.
//!
//! Score: capability match 30%, trust 25% (trusted peer > known >
//! unknown), recent success rate 20% (0.5 below 5 samples), load 10%
//! (inverse of load / max_concurrent), ETA fit 10% (unknown is neutral),
//! cost 5%. A strategy overrides the weights; `preferred` instances get
//! +50%; `excluded` instances and open circuits are dropped.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::a2a::task::{EtaEstimate, TaskState, TASK_CIRCUIT_OPENED};
use crate::services::capability::{CapabilityRouteStrategy, ProviderKind};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::gateway::{GatewayCard, RouteStrategy, GATEWAY_AGENT_ID};
use crate::services::relay::protocol::InstancePresence;
use crate::services::AppState;

// Wave A (Track D): per-provider circuits and capability-aware ranking
pub mod capability;
pub mod circuit;

pub type SelectionError = String;

/// One ranked provider
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityMatch {
    pub instance_id: String,
    pub agent_id: String,
    pub capability: String,
    pub score: f32,
    /// Human-readable, e.g. "trusted peer", "circuit closed", "3 queued"
    pub reasons: Vec<String>,
    pub eta: Option<EtaEstimate>,
    pub load: u32,
    pub trusted: bool,
    pub recent_success_rate: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SelectionContext {
    pub capability: String,
    pub caller_instance: String,
    pub strategy: Option<RouteStrategy>,
    /// Instance IDs
    pub preferred: Vec<String>,
    pub excluded: Vec<String>,
}

/// A `circuit_breakers` row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CircuitBreakerState {
    pub instance_id: String,
    pub capability: String,
    pub consecutive_failures: u32,
    /// Epoch ms; open for 5 minutes from then
    pub opened_at: Option<Timestamp>,
}

/// `BoxFuture`-returning so `AppState` can hold an `Arc<dyn SelectionService>`.
pub trait SelectionService: Send + Sync {
    /// Best first
    fn rank<'a>(
        &'a self,
        ctx: &'a SelectionContext,
        candidates: Vec<GatewayCard>,
    ) -> BoxFuture<'a, Result<Vec<CapabilityMatch>, SelectionError>>;

    /// Success resets the failures and closes the circuit; the third
    /// consecutive failure opens it (`task://circuit_opened`)
    fn record_outcome<'a>(
        &'a self,
        instance_id: &'a str,
        capability: &'a str,
        success: bool,
    ) -> BoxFuture<'a, Result<(), SelectionError>>;

    /// Opened within the last 5 minutes
    fn is_circuit_open<'a>(
        &'a self,
        instance_id: &'a str,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<bool, SelectionError>>;

    /// Wave A (Track D): local providers plus relay-registered remote ones
    /// for `ctx.capability_id`, filtered by required tools, exclusions,
    /// and open circuits (`circuit`), scored and ordered by the strategy.
    /// Best first.
    fn rank_for_capability<'a>(
        &'a self,
        ctx: &'a CapabilitySelectionContext,
    ) -> BoxFuture<'a, Result<Vec<ProviderCandidate>, SelectionError>> {
        Box::pin(async move { Ok(self.capability_ranking(ctx).await?.candidates) })
    }

    /// [`Self::rank_for_capability`] plus what it ranked against (whether
    /// the capability has a definition, its required tools), so dispatch
    /// knows whether a provider missing from the ranking was filtered out
    fn capability_ranking<'a>(
        &'a self,
        _ctx: &'a CapabilitySelectionContext,
    ) -> BoxFuture<'a, Result<CapabilityRanking, SelectionError>> {
        Box::pin(async { Err(crate::services::redesign::NOT_IMPLEMENTED.to_string()) })
    }

    /// Wave A: one outcome for a provider's own circuit
    /// (`provider_circuits`); the instance-level circuit is
    /// `record_outcome`'s
    fn record_provider_outcome<'a>(
        &'a self,
        _instance_id: &'a str,
        _provider_id: &'a str,
        _capability_id: &'a str,
        _success: bool,
    ) -> BoxFuture<'a, Result<(), SelectionError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Wave A: the result of [`SelectionService::capability_ranking`]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapabilityRanking {
    /// Best first; providers that failed a filter are left out
    pub candidates: Vec<ProviderCandidate>,
    /// Whether `capabilities_projection` has the capability
    pub defined: bool,
    /// Normalized tool IDs every candidate has
    pub required_tools: Vec<String>,
    /// Every provider considered (`<instance_id>/<provider_id>`), kept or
    /// filtered out
    pub considered: Vec<String>,
}

/// Wave A: what `rank_for_capability` ranks for
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySelectionContext {
    pub capability_id: String,
    pub caller_instance: String,
    /// `None`: the capability's `routing.strategy`
    pub strategy: Option<CapabilityRouteStrategy>,
    /// On top of the capability's `required_tools` and
    /// `routing.require_tools`
    pub required_tools: Vec<String>,
    /// Provider keys (`<instance_id>/<provider_id>`) or instance IDs
    pub preferred: Vec<String>,
    pub excluded: Vec<String>,
}

/// Wave A: one ranked provider of a capability (agent or workflow)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCandidate {
    pub instance_id: String,
    pub provider_kind: ProviderKind,
    pub provider_id: String,
    pub capability_id: String,
    pub local: bool,
    pub score: f32,
    /// Human-readable, e.g. "has every required tool", "least loaded"
    pub reasons: Vec<String>,
    pub load: u32,
    pub max_concurrent: u32,
    pub provided_tools: Vec<String>,
    pub trusted: bool,
    pub recent_success_rate: Option<f32>,
    pub eta: Option<EtaEstimate>,
}

/// Consecutive failures that open a circuit
pub const CIRCUIT_FAILURES: u32 = 3;
/// How long an open circuit keeps a provider out
pub const CIRCUIT_OPEN_MS: i64 = 5 * 60 * 1000;
/// Finished tasks needed before a success rate counts
pub const MIN_SUCCESS_SAMPLES: u32 = 5;
/// Recent tasks a success rate is drawn from
const SUCCESS_WINDOW: u32 = 50;
/// Recent durations an ETA is drawn from
const ETA_WINDOW: u32 = 100;

/// How much each part of the score counts (they sum to 1)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub capability: f32,
    pub trust: f32,
    pub success: f32,
    pub load: f32,
    pub eta: f32,
    pub cost: f32,
}

impl Weights {
    /// The defaults, or the strategy's (its own part dominant)
    pub fn for_strategy(strategy: Option<RouteStrategy>) -> Self {
        let weights = |capability, trust, success, load, eta, cost| Self {
            capability,
            trust,
            success,
            load,
            eta,
            cost,
        };
        match strategy {
            None => weights(0.30, 0.25, 0.20, 0.10, 0.10, 0.05),
            Some(RouteStrategy::Fastest) => weights(0.20, 0.10, 0.10, 0.10, 0.45, 0.05),
            Some(RouteStrategy::Cheapest) => weights(0.20, 0.10, 0.10, 0.10, 0.05, 0.45),
            Some(RouteStrategy::MostTrusted) => weights(0.20, 0.45, 0.15, 0.10, 0.05, 0.05),
            Some(RouteStrategy::HighestSuccessRate) => weights(0.20, 0.15, 0.45, 0.10, 0.05, 0.05),
        }
    }
}

/// What SQLite knows about one candidate
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidateFacts {
    /// `trust_relationships.added_as_trusted_peer`
    pub trusted: bool,
    /// Any task history with the instance
    pub known: bool,
    /// `trust_relationships.delegations_paused`
    pub paused: bool,
    /// (succeeded, finished) among recent tasks for the capability
    pub outcomes: (u32, u32),
    pub eta: Option<EtaEstimate>,
    pub circuit_open: bool,
}

/// 1.0 for an exact match, 0.6 for a case-insensitive substring, else 0
pub fn capability_match(card: &GatewayCard, capability: &str) -> f32 {
    let wanted = capability.trim().to_lowercase();
    if card.capabilities.iter().any(|c| c == capability) {
        1.0
    } else if !wanted.is_empty()
        && card.capabilities.iter().any(|c| {
            let c = c.to_lowercase();
            c.contains(&wanted) || wanted.contains(&c)
        })
    {
        0.6
    } else {
        0.0
    }
}

/// Scores `candidates` best first (pure; `facts` supplies the history).
/// Excluded instances, paused ones, and open circuits are dropped.
pub fn rank_with(
    ctx: &SelectionContext,
    candidates: Vec<GatewayCard>,
    facts: impl Fn(&GatewayCard) -> CandidateFacts,
) -> Vec<CapabilityMatch> {
    let weights = Weights::for_strategy(ctx.strategy);
    let mut ranked: Vec<CapabilityMatch> = candidates
        .into_iter()
        .filter(|card| !ctx.excluded.contains(&card.instance_id))
        .filter_map(|card| {
            let facts = facts(&card);
            if facts.paused || facts.circuit_open {
                return None;
            }
            Some(score(ctx, &card, &facts, &weights))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.instance_id.cmp(&b.instance_id))
    });
    ranked
}

fn score(
    ctx: &SelectionContext,
    card: &GatewayCard,
    facts: &CandidateFacts,
    weights: &Weights,
) -> CapabilityMatch {
    let mut reasons = Vec::new();

    let capability = capability_match(card, &ctx.capability);
    reasons.push(
        match capability {
            c if c >= 1.0 => "exact capability match",
            c if c > 0.0 => "partial capability match",
            _ => "doesn't list the capability",
        }
        .to_string(),
    );

    let trust = if facts.trusted {
        reasons.push("trusted peer".to_string());
        1.0
    } else if facts.known {
        reasons.push("worked with before".to_string());
        0.6
    } else {
        reasons.push("unknown instance".to_string());
        0.3
    };

    let (succeeded, finished) = facts.outcomes;
    let recent_success_rate =
        (finished >= MIN_SUCCESS_SAMPLES).then(|| succeeded as f32 / finished as f32);
    let success = match recent_success_rate {
        Some(rate) => {
            reasons.push(format!(
                "{:.0}% of {finished} recent tasks succeeded",
                rate * 100.0
            ));
            rate
        }
        None => {
            reasons.push("not enough history yet".to_string());
            0.5
        }
    };

    let max = card.max_concurrent.max(1);
    let load = (1.0 - card.current_load as f32 / max as f32).clamp(0.0, 1.0);
    reasons.push(format!("{}/{max} busy", card.current_load));

    let eta = match &facts.eta {
        Some(eta) if eta.sample_count > 0 => {
            reasons.push(format!("typically ~{}s", eta.median_seconds));
            1.0 / (1.0 + eta.median_seconds as f32 / 60.0)
        }
        _ => 0.5,
    };

    // Every provider is free today
    let cost = 1.0;
    reasons.push("circuit closed".to_string());

    let mut total = weights.capability * capability
        + weights.trust * trust
        + weights.success * success
        + weights.load * load
        + weights.eta * eta
        + weights.cost * cost;
    if ctx.preferred.contains(&card.instance_id) {
        total *= 1.5;
        reasons.push("preferred".to_string());
    }

    CapabilityMatch {
        instance_id: card.instance_id.clone(),
        agent_id: GATEWAY_AGENT_ID.to_string(),
        capability: ctx.capability.clone(),
        score: total,
        reasons,
        eta: facts.eta,
        load: card.current_load,
        trusted: facts.trusted,
        recent_success_rate,
    }
}

/// Median and 95th percentile of `durations` (seconds)
pub fn eta_from(mut durations: Vec<u32>) -> Option<EtaEstimate> {
    if durations.is_empty() {
        return None;
    }
    durations.sort_unstable();
    let at = |q: f64| durations[((durations.len() - 1) as f64 * q).round() as usize];
    Some(EtaEstimate {
        median_seconds: at(0.5),
        p95_seconds: at(0.95),
        sample_count: durations.len() as u32,
    })
}

/// Epoch ms; injectable for tests
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The selection service over SQLite (`tasks`, `eta_history`,
/// `trust_relationships`, `circuit_breakers`)
pub struct DbSelection {
    db: Db,
    bus: Option<EventBus>,
    clock: Clock,
    /// Remote providers of a capability (the relay's presence); none
    /// without it
    presence: Option<PresenceSource>,
    /// Round-robin turns per capability
    turns: std::sync::Mutex<HashMap<String, usize>>,
}

/// Remote providers of a capability ID (`relay::presence`); injectable for
/// tests
pub type PresenceSource =
    Arc<dyn Fn(String) -> BoxFuture<'static, Result<Vec<InstancePresence>, String>> + Send + Sync>;

impl DbSelection {
    pub fn new(db: Db, bus: Option<EventBus>) -> Self {
        Self {
            db,
            bus,
            clock: Arc::new(now_ms),
            presence: None,
            turns: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_presence(mut self, presence: PresenceSource) -> Self {
        self.presence = Some(presence);
        self
    }

    /// This capability's round-robin turn, advancing it
    fn next_turn(&self, capability_id: &str) -> usize {
        let mut turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        let turn = turns.entry(capability_id.to_string()).or_insert(0);
        let current = *turn;
        *turn = turn.wrapping_add(1);
        current
    }
}

fn facts(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
    now: i64,
) -> rusqlite::Result<CandidateFacts> {
    let (trusted, paused) = conn
        .query_row(
            "SELECT added_as_trusted_peer, delegations_paused FROM trust_relationships
             WHERE instance_id = ?1",
            [instance_id],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
        )
        .optional()?
        .unwrap_or((false, false));
    let known: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE provider_instance = ?1 OR requester_instance = ?1)",
        [instance_id],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT state FROM tasks WHERE provider_instance = ?1 AND capability = ?2
         ORDER BY updated_at DESC LIMIT ?3",
    )?;
    let states = stmt
        .query_map(params![instance_id, capability, SUCCESS_WINDOW], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let finished: Vec<TaskState> = states
        .iter()
        .filter_map(|s| TaskState::parse(s))
        // Cancelled tasks say nothing about the provider
        .filter(|s| s.is_terminal() && *s != TaskState::Cancelled)
        .collect();
    let succeeded = finished
        .iter()
        .filter(|s| **s == TaskState::Completed)
        .count() as u32;

    let mut stmt = conn.prepare(
        "SELECT duration_seconds FROM eta_history
         WHERE capability = ?1 AND (provider_agent = ?2 OR substr(provider_agent, 1, length(?2) + 1) = ?2 || '/')
         ORDER BY at DESC LIMIT ?3",
    )?;
    let durations = stmt
        .query_map(params![capability, instance_id, ETA_WINDOW], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(CandidateFacts {
        trusted,
        known,
        paused,
        outcomes: (succeeded, finished.len() as u32),
        eta: eta_from(durations.into_iter().map(|d| d.max(0) as u32).collect()),
        circuit_open: circuit_open(conn, instance_id, capability, now)?,
    })
}

fn circuit_row(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
) -> rusqlite::Result<Option<CircuitBreakerState>> {
    conn.query_row(
        "SELECT consecutive_failures, opened_at FROM circuit_breakers
         WHERE instance_id = ?1 AND capability = ?2",
        params![instance_id, capability],
        |row| {
            Ok(CircuitBreakerState {
                instance_id: instance_id.to_string(),
                capability: capability.to_string(),
                consecutive_failures: row.get(0)?,
                opened_at: row.get::<_, Option<i64>>(1)?.map(|ms| ms as f64),
            })
        },
    )
    .optional()
}

fn is_open(state: &CircuitBreakerState, now: i64) -> bool {
    state
        .opened_at
        .is_some_and(|opened| now - (opened as i64) < CIRCUIT_OPEN_MS)
}

fn circuit_open(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    Ok(circuit_row(conn, instance_id, capability)?.is_some_and(|state| is_open(&state, now)))
}

/// Applies one outcome; returns the state when this failure opened the
/// circuit
fn record(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
    success: bool,
    now: i64,
) -> rusqlite::Result<Option<CircuitBreakerState>> {
    let previous = circuit_row(conn, instance_id, capability)?;
    let mut state = previous.unwrap_or(CircuitBreakerState {
        instance_id: instance_id.to_string(),
        capability: capability.to_string(),
        consecutive_failures: 0,
        opened_at: None,
    });
    let mut opened = false;
    if success {
        state.consecutive_failures = 0;
        state.opened_at = None;
    } else {
        state.consecutive_failures += 1;
        // Opens at the third failure; after the open window a failed trial
        // opens it again
        if state.consecutive_failures >= CIRCUIT_FAILURES && !is_open(&state, now) {
            state.opened_at = Some(now as f64);
            opened = true;
        }
    }
    conn.execute(
        "INSERT INTO circuit_breakers (instance_id, capability, consecutive_failures, opened_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(instance_id, capability) DO UPDATE SET
             consecutive_failures = excluded.consecutive_failures,
             opened_at = excluded.opened_at",
        params![
            instance_id,
            capability,
            state.consecutive_failures,
            state.opened_at.map(|ms| ms as i64),
        ],
    )?;
    Ok(opened.then_some(state))
}

/// `task://circuit_opened`
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct CircuitOpened(pub CircuitBreakerState);

impl NandiEvent for CircuitOpened {
    const NAME: &'static str = TASK_CIRCUIT_OPENED;
}

impl SelectionService for DbSelection {
    fn rank<'a>(
        &'a self,
        ctx: &'a SelectionContext,
        candidates: Vec<GatewayCard>,
    ) -> BoxFuture<'a, Result<Vec<CapabilityMatch>, SelectionError>> {
        Box::pin(async move {
            let now = (self.clock)();
            let instances: Vec<String> = candidates.iter().map(|c| c.instance_id.clone()).collect();
            let capability = ctx.capability.clone();
            let known: HashMap<String, CandidateFacts> = self
                .db
                .call(move |conn| {
                    instances
                        .into_iter()
                        .map(|instance| {
                            let facts = facts(conn, &instance, &capability, now)?;
                            Ok((instance, facts))
                        })
                        .collect()
                })
                .await?;
            Ok(rank_with(ctx, candidates, |card| {
                known.get(&card.instance_id).cloned().unwrap_or_default()
            }))
        })
    }

    fn record_outcome<'a>(
        &'a self,
        instance_id: &'a str,
        capability: &'a str,
        success: bool,
    ) -> BoxFuture<'a, Result<(), SelectionError>> {
        Box::pin(async move {
            let now = (self.clock)();
            let (instance, cap) = (instance_id.to_string(), capability.to_string());
            let opened = self
                .db
                .call(move |conn| record(conn, &instance, &cap, success, now))
                .await?;
            if let Some(state) = opened {
                log::warn!(
                    "Circuit opened for {instance_id} / {capability} after {} failures",
                    state.consecutive_failures
                );
                if let Some(bus) = &self.bus {
                    events::emit(bus, &CircuitOpened(state));
                }
            }
            Ok(())
        })
    }

    fn is_circuit_open<'a>(
        &'a self,
        instance_id: &'a str,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<bool, SelectionError>> {
        Box::pin(async move {
            let now = (self.clock)();
            let (instance, cap) = (instance_id.to_string(), capability.to_string());
            self.db
                .call(move |conn| circuit_open(conn, &instance, &cap, now))
                .await
        })
    }

    fn capability_ranking<'a>(
        &'a self,
        ctx: &'a CapabilitySelectionContext,
    ) -> BoxFuture<'a, Result<CapabilityRanking, SelectionError>> {
        Box::pin(self.rank_capability(ctx))
    }

    fn record_provider_outcome<'a>(
        &'a self,
        instance_id: &'a str,
        provider_id: &'a str,
        capability_id: &'a str,
        success: bool,
    ) -> BoxFuture<'a, Result<(), SelectionError>> {
        Box::pin(async move {
            circuit::record_at(
                &self.db,
                self.bus.as_ref(),
                instance_id,
                provider_id,
                capability_id,
                success,
                (self.clock)(),
            )
            .await
            .map(|_| ())
        })
    }
}

/// The SQLite-backed selection service, with the relay's capability
/// presence for remote providers. Must not block.
pub fn init(state: &AppState) -> Option<Arc<dyn SelectionService>> {
    let weak = state.downgrade();
    let presence: PresenceSource = Arc::new(move |capability_id: String| {
        let weak = weak.clone();
        Box::pin(async move {
            let Some(state) = weak.upgrade() else {
                return Ok(Vec::new());
            };
            if state.relay_client().is_none() {
                return Ok(Vec::new());
            }
            crate::services::relay::presence::query_capability_presence(&state, &capability_id)
                .await
        })
    });
    Some(Arc::new(
        DbSelection::new(state.db().clone(), Some(state.bus().clone())).with_presence(presence),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;
    use crate::services::domain::agent_card::OwnerRef;

    async fn open(selection: &DbSelection) -> bool {
        selection
            .is_circuit_open("peer", "summarize")
            .await
            .unwrap()
    }

    fn card(instance: &str, capabilities: &[&str], load: u32) -> GatewayCard {
        GatewayCard {
            instance_id: instance.to_string(),
            endpoint: String::new(),
            display_name: instance.to_string(),
            description: String::new(),
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
            accepts_delegation: true,
            max_concurrent: 4,
            current_load: load,
            owner: OwnerRef::default(),
            federation_enabled: false,
            gateway_version: 1,
            agent_count: 1,
            body: None,
            direct_delegation: None,
            responsibilities: Vec::new(),
            availability: Default::default(),
        }
    }

    fn ctx(capability: &str) -> SelectionContext {
        SelectionContext {
            capability: capability.to_string(),
            caller_instance: "me".into(),
            ..SelectionContext::default()
        }
    }

    fn insert_task(conn: &Connection, id: &str, provider: &str, state: &str) {
        conn.execute(
            "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                 provider_agent, capability, state, created_at, updated_at)
             VALUES (?1, 'me', 'me/a', ?2, 'gateway', 'summarize', ?3, 0, 0)",
            params![id, provider, state],
        )
        .unwrap();
    }

    /// trusted (busy, no history), known (idle, 0/5 succeeded), unknown
    /// (idle): trust and success decide
    fn fixture(db: &Db) -> Vec<GatewayCard> {
        let conn = db.conn().unwrap();
        conn.execute(
            "INSERT INTO trust_relationships (instance_id, added_as_trusted_peer, delegations_paused, last_updated)
             VALUES ('trusted', 1, 0, 0), ('paused', 1, 1, 0)",
            [],
        )
        .unwrap();
        for i in 0..5 {
            insert_task(&conn, &format!("k{i}"), "known", "failed");
        }
        insert_task(&conn, "k-cancelled", "known", "cancelled");
        for (i, seconds) in [30, 60, 90].iter().enumerate() {
            conn.execute(
                "INSERT INTO eta_history (capability, provider_agent, duration_seconds, at)
                 VALUES ('summarize', 'trusted/gateway', ?1, ?2)",
                params![seconds, i as i64],
            )
            .unwrap();
        }
        vec![
            card("unknown", &["summarize"], 0),
            card("known", &["summarize"], 0),
            card("trusted", &["summarize"], 3),
            card("paused", &["summarize"], 0),
        ]
    }

    #[tokio::test]
    async fn ranks_by_trust_load_and_success() {
        let db = Db::in_memory();
        let candidates = fixture(&db);
        let selection = DbSelection::new(db, None);
        let ranked = selection.rank(&ctx("summarize"), candidates).await.unwrap();
        let order: Vec<&str> = ranked.iter().map(|m| m.instance_id.as_str()).collect();
        // Paused is dropped; the trusted peer wins despite its load
        assert_eq!(order, ["trusted", "unknown", "known"]);
        let trusted = &ranked[0];
        assert!(trusted.trusted);
        assert_eq!(trusted.eta.as_ref().unwrap().median_seconds, 60);
        assert!(trusted.reasons.contains(&"trusted peer".to_string()));
        assert!(trusted.reasons.contains(&"3/4 busy".to_string()));
        assert_eq!(ranked[2].recent_success_rate, Some(0.0));
        assert!(ranked[2]
            .reasons
            .contains(&"0% of 5 recent tasks succeeded".to_string()));
        assert_eq!(ranked[1].recent_success_rate, None);
        assert!(ranked.iter().all(|m| m.agent_id == GATEWAY_AGENT_ID));
    }

    #[tokio::test]
    async fn preferred_boosts_and_excluded_drops() {
        let db = Db::in_memory();
        let candidates = fixture(&db);
        let selection = DbSelection::new(db, None);
        let context = SelectionContext {
            preferred: vec!["known".into()],
            excluded: vec!["trusted".into()],
            ..ctx("summarize")
        };
        let ranked = selection.rank(&context, candidates).await.unwrap();
        let order: Vec<&str> = ranked.iter().map(|m| m.instance_id.as_str()).collect();
        assert_eq!(order, ["known", "unknown"]);
        assert!(ranked[0].reasons.contains(&"preferred".to_string()));
    }

    #[test]
    fn scores_capability_matches_and_strategies() {
        let exact = card("a", &["summarize-paper"], 0);
        assert_eq!(capability_match(&exact, "summarize-paper"), 1.0);
        assert_eq!(capability_match(&exact, "Summarize"), 0.6);
        assert_eq!(capability_match(&exact, "translate"), 0.0);

        // Fastest: the quick, unknown provider beats the slow, trusted one
        let candidates = vec![card("slow", &["x"], 0), card("quick", &["x"], 0)];
        let facts = |card: &GatewayCard| {
            let seconds = if card.instance_id == "slow" { 600 } else { 5 };
            CandidateFacts {
                trusted: card.instance_id == "slow",
                eta: eta_from(vec![seconds]),
                ..CandidateFacts::default()
            }
        };
        let fastest = SelectionContext {
            strategy: Some(RouteStrategy::Fastest),
            ..ctx("x")
        };
        assert_eq!(
            rank_with(&fastest, candidates.clone(), facts)[0].instance_id,
            "quick"
        );
        let trusted = SelectionContext {
            strategy: Some(RouteStrategy::MostTrusted),
            ..ctx("x")
        };
        assert_eq!(
            rank_with(&trusted, candidates, facts)[0].instance_id,
            "slow"
        );
        for strategy in [
            None,
            Some(RouteStrategy::Fastest),
            Some(RouteStrategy::Cheapest),
            Some(RouteStrategy::MostTrusted),
            Some(RouteStrategy::HighestSuccessRate),
        ] {
            let w = Weights::for_strategy(strategy);
            let sum = w.capability + w.trust + w.success + w.load + w.eta + w.cost;
            assert!((sum - 1.0).abs() < 1e-5);
        }
    }

    #[tokio::test]
    async fn circuit_opens_after_three_failures_and_closes_after_five_minutes() {
        let (bus, recorder) = crate::services::events::tests::bus();
        let db = bus.db.clone();
        let now = Arc::new(AtomicI64::new(1_000_000));
        let clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst)) as Clock
        };
        let selection = DbSelection::new(db, Some(bus)).with_clock(clock);

        selection
            .record_outcome("peer", "summarize", false)
            .await
            .unwrap();
        selection
            .record_outcome("peer", "summarize", false)
            .await
            .unwrap();
        assert!(!open(&selection).await);
        selection
            .record_outcome("peer", "summarize", false)
            .await
            .unwrap();
        assert!(open(&selection).await);
        assert_eq!(recorder.names(), [TASK_CIRCUIT_OPENED]);
        // Open circuits are dropped from rankings
        let ranked = selection
            .rank(&ctx("summarize"), vec![card("peer", &["summarize"], 0)])
            .await
            .unwrap();
        assert!(ranked.is_empty());

        now.fetch_add(CIRCUIT_OPEN_MS - 1, Ordering::SeqCst);
        assert!(open(&selection).await);
        now.fetch_add(1, Ordering::SeqCst);
        assert!(!open(&selection).await);
        // A failed trial opens it again; a success resets it
        selection
            .record_outcome("peer", "summarize", false)
            .await
            .unwrap();
        assert!(open(&selection).await);
        assert_eq!(recorder.names().len(), 2);
        selection
            .record_outcome("peer", "summarize", true)
            .await
            .unwrap();
        assert!(!open(&selection).await);
        // Other capabilities are separate
        assert!(!selection
            .is_circuit_open("peer", "translate")
            .await
            .unwrap());
    }
}
