//! Capability selection for this Nandi's own providers: which of its
//! agents and workflows takes a capability, and their circuits.
//!
//! Only local providers are ranked here (`capability`). Who on the network
//! gets a request is the relay's decision (`RouteQuery` → `RoutePlan`, see
//! `a2a::route`): its scorer, track records and circuits live only there.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::services::a2a::task::EtaEstimate;
use crate::services::capability::{CapabilityRouteStrategy, ProviderKind};
use crate::services::events::EventBus;
use crate::services::AppState;

pub mod capability;
pub mod circuit;

pub type SelectionError = String;

/// `BoxFuture`-returning so `AppState` can hold an `Arc<dyn SelectionService>`.
pub trait SelectionService: Send + Sync {
    /// This instance's providers of `ctx.capability_id`, filtered by
    /// required tools, exclusions and open circuits (`circuit`), scored and
    /// ordered by the strategy. Best first.
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
        ctx: &'a CapabilitySelectionContext,
    ) -> BoxFuture<'a, Result<CapabilityRanking, SelectionError>>;

    /// One outcome for a local provider's circuit (`provider_circuits`)
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

/// The result of [`SelectionService::capability_ranking`]
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

/// What `rank_for_capability` ranks for
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySelectionContext {
    pub capability_id: String,
    /// This instance: only its providers are ranked
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

/// One ranked local provider of a capability (agent or workflow)
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

/// Consecutive failures that open a local provider's circuit
pub const CIRCUIT_FAILURES: u32 = 3;
/// How long an open circuit keeps a provider out
pub const CIRCUIT_OPEN_MS: i64 = 5 * 60 * 1000;
/// Finished tasks needed before a success rate counts
pub const MIN_SUCCESS_SAMPLES: u32 = 5;
/// Recent tasks a success rate is drawn from
const SUCCESS_WINDOW: u32 = 50;
/// Recent durations an ETA is drawn from
const ETA_WINDOW: u32 = 100;

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

/// The selection service over SQLite (`capability_providers`, `tasks`,
/// `eta_history`, `provider_circuits`)
pub struct DbSelection {
    db: Db,
    bus: Option<EventBus>,
    clock: Clock,
    /// Round-robin turns per capability
    turns: std::sync::Mutex<HashMap<String, usize>>,
}

impl DbSelection {
    pub fn new(db: Db, bus: Option<EventBus>) -> Self {
        Self {
            db,
            bus,
            clock: Arc::new(now_ms),
            turns: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
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

impl SelectionService for DbSelection {
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

/// The SQLite-backed selection service. Must not block.
pub fn init(state: &AppState) -> Option<Arc<dyn SelectionService>> {
    Some(Arc::new(DbSelection::new(
        state.db().clone(),
        Some(state.bus().clone()),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_is_the_median_and_95th_percentile() {
        assert_eq!(eta_from(Vec::new()), None);
        let eta = eta_from(vec![90, 30, 60]).unwrap();
        assert_eq!(
            (eta.median_seconds, eta.p95_seconds, eta.sample_count),
            (60, 90, 3)
        );
    }
}
