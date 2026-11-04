//! Capability-aware selection (wave A, Track D): ranks every provider of a
//! capability, agents and workflows, local (`capability_providers`) and
//! remote (the relay's presence).
//!
//! Filters: the capability's `required_tools` + `routing.require_tools` +
//! the caller's (a provider must have every one), `excluded` providers,
//! paused instances, open provider circuits (`circuit`).
//!
//! Score: capability match 25%, trust 20% (this instance / trusted peer >
//! known > unknown), recent success rate 20%, load 15%, ETA 10%, cost 5%,
//! tool richness 5%. The strategy (`routing.strategy` unless the caller
//! picks one) makes its own part dominant; `RoundRobin` takes turns over
//! the providers left. `preferred` providers get +50%.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use super::{
    circuit, eta_from, CandidateFacts, CapabilityRanking, CapabilitySelectionContext, DbSelection,
    ProviderCandidate, SelectionError, ETA_WINDOW, MIN_SUCCESS_SAMPLES, SUCCESS_WINDOW,
};
use crate::services::a2a::task::TaskState;
use crate::services::capability::{CapabilityRouteStrategy, ProviderKind};
use crate::services::relay::protocol::InstancePresence;
use crate::services::tools::definition::normalize_id;

/// What a capability's definition asks of its providers, merged with the
/// caller's context
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Requirements {
    /// Normalized tool IDs, sorted
    pub required_tools: Vec<String>,
    pub strategy: CapabilityRouteStrategy,
    pub preferred: Vec<String>,
    pub excluded: Vec<String>,
    /// `policy.max_concurrent_per_provider` (0: the provider's own)
    pub max_concurrent_per_provider: u32,
}

impl Requirements {
    /// From the definition (when there is one) and `ctx`
    pub fn merge(definition: Option<&DefinitionFacts>, ctx: &CapabilitySelectionContext) -> Self {
        let default = DefinitionFacts::default();
        let def = definition.unwrap_or(&default);
        let mut tools: Vec<String> = def
            .required_tools
            .iter()
            .chain(&def.routing.require_tools)
            .chain(&ctx.required_tools)
            .map(|t| normalize_id(t))
            .filter(|t| t != "builtin.")
            .collect();
        tools.sort();
        tools.dedup();
        let union = |a: &[String], b: &[String]| {
            let mut all: Vec<String> = a.iter().chain(b).cloned().collect();
            all.dedup();
            all
        };
        Self {
            required_tools: tools,
            strategy: ctx.strategy.unwrap_or(def.routing.strategy),
            preferred: union(&ctx.preferred, &def.routing.preferred_providers),
            excluded: union(&ctx.excluded, &def.routing.excluded_providers),
            max_concurrent_per_provider: def.policy.max_concurrent_per_provider,
        }
    }
}

/// The parts of `capabilities_projection.definition_json` selection reads.
/// Lenient: a field it can't read is its default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DefinitionFacts {
    pub required_tools: Vec<String>,
    pub routing: RoutingFacts,
    pub policy: PolicyFacts,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RoutingFacts {
    #[serde(deserialize_with = "lenient_strategy")]
    pub strategy: CapabilityRouteStrategy,
    pub preferred_providers: Vec<String>,
    pub excluded_providers: Vec<String>,
    pub require_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PolicyFacts {
    pub max_concurrent_per_provider: u32,
}

fn lenient_strategy<'de, D>(deserializer: D) -> Result<CapabilityRouteStrategy, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// One provider before scoring
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawProvider {
    pub instance_id: String,
    pub provider_kind: ProviderKind,
    pub provider_id: String,
    pub load: u32,
    pub max_concurrent: u32,
    /// Normalized tool IDs
    pub provided_tools: Vec<String>,
}

impl RawProvider {
    /// `<instance_id>/<provider_id>`
    pub fn key(&self) -> String {
        format!("{}/{}", self.instance_id, self.provider_id)
    }

    /// Listed by its key or its instance
    fn listed_in(&self, list: &[String]) -> bool {
        let key = self.key();
        list.iter()
            .any(|item| *item == key || *item == self.instance_id)
    }

    /// Required tools it lacks
    pub fn missing_tools<'a>(&self, required: &'a [String]) -> Vec<&'a str> {
        let have: HashSet<&str> = self.provided_tools.iter().map(String::as_str).collect();
        required
            .iter()
            .filter(|tool| !have.contains(tool.as_str()))
            .map(String::as_str)
            .collect()
    }
}

/// How much each part counts (they sum to 1)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapabilityWeights {
    pub capability: f32,
    pub trust: f32,
    pub success: f32,
    pub load: f32,
    pub eta: f32,
    pub cost: f32,
    pub tools: f32,
}

impl CapabilityWeights {
    pub fn for_strategy(strategy: CapabilityRouteStrategy) -> Self {
        let w = |capability, trust, success, load, eta, cost, tools| Self {
            capability,
            trust,
            success,
            load,
            eta,
            cost,
            tools,
        };
        match strategy {
            CapabilityRouteStrategy::LeastLoaded => w(0.15, 0.10, 0.10, 0.45, 0.10, 0.05, 0.05),
            CapabilityRouteStrategy::MostTrusted => w(0.15, 0.45, 0.15, 0.10, 0.05, 0.05, 0.05),
            CapabilityRouteStrategy::Fastest => w(0.15, 0.10, 0.10, 0.10, 0.45, 0.05, 0.05),
            CapabilityRouteStrategy::HighestSuccessRate => {
                w(0.15, 0.15, 0.45, 0.10, 0.05, 0.05, 0.05)
            }
            // The order is the turn; the score is informational
            CapabilityRouteStrategy::RoundRobin => w(0.25, 0.20, 0.20, 0.15, 0.10, 0.05, 0.05),
        }
    }
}

/// Tools beyond these count as fully rich
const RICH_TOOLS: f32 = 10.0;

/// Ranks `providers` best first (pure; `facts` supplies the history,
/// `local_instance` is the caller's). `turn` is the round-robin turn.
pub fn rank_providers(
    ctx: &CapabilitySelectionContext,
    req: &Requirements,
    providers: Vec<RawProvider>,
    facts: impl Fn(&RawProvider) -> CandidateFacts,
    turn: usize,
) -> Vec<ProviderCandidate> {
    let weights = CapabilityWeights::for_strategy(req.strategy);
    let mut ranked: Vec<ProviderCandidate> = providers
        .into_iter()
        .filter(|p| !p.listed_in(&req.excluded))
        .filter(|p| p.missing_tools(&req.required_tools).is_empty())
        .filter_map(|p| {
            let facts = facts(&p);
            if facts.paused || facts.circuit_open {
                return None;
            }
            Some(score(ctx, req, p, &facts, &weights))
        })
        .collect();
    if req.strategy == CapabilityRouteStrategy::RoundRobin {
        ranked.sort_by_key(key_of);
        if !ranked.is_empty() {
            let at = turn % ranked.len();
            ranked.rotate_left(at);
        }
        for candidate in &mut ranked {
            candidate.reasons.push("round robin".to_string());
        }
    } else {
        ranked.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| key_of(a).cmp(&key_of(b)))
        });
    }
    ranked
}

fn key_of(candidate: &ProviderCandidate) -> String {
    format!("{}/{}", candidate.instance_id, candidate.provider_id)
}

fn score(
    ctx: &CapabilitySelectionContext,
    req: &Requirements,
    provider: RawProvider,
    facts: &CandidateFacts,
    weights: &CapabilityWeights,
) -> ProviderCandidate {
    let mut reasons = vec![match provider.provider_kind {
        ProviderKind::Agent => "agent provider".to_string(),
        ProviderKind::Workflow => "workflow provider".to_string(),
    }];
    let local = provider.instance_id == ctx.caller_instance;

    let trust = if local {
        reasons.push("this instance".to_string());
        1.0
    } else if facts.trusted {
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

    let mut max = provider.max_concurrent.max(1);
    if req.max_concurrent_per_provider > 0 {
        max = max.min(req.max_concurrent_per_provider);
    }
    let load = (1.0 - provider.load as f32 / max as f32).clamp(0.0, 1.0);
    reasons.push(format!("{}/{max} busy", provider.load));

    let eta = match &facts.eta {
        Some(eta) if eta.sample_count > 0 => {
            reasons.push(format!("typically ~{}s", eta.median_seconds));
            1.0 / (1.0 + eta.median_seconds as f32 / 60.0)
        }
        _ => 0.5,
    };

    // Every provider is free today
    let cost = 1.0;

    if !req.required_tools.is_empty() {
        reasons.push("has every required tool".to_string());
    }
    let tools = (provider.provided_tools.len() as f32 / RICH_TOOLS).min(1.0);
    reasons.push(format!("{} tools", provider.provided_tools.len()));
    reasons.push("circuit closed".to_string());

    let mut total = weights.capability
        + weights.trust * trust
        + weights.success * success
        + weights.load * load
        + weights.eta * eta
        + weights.cost * cost
        + weights.tools * tools;
    if provider.listed_in(&req.preferred) {
        total *= 1.5;
        reasons.push("preferred".to_string());
    }

    ProviderCandidate {
        trusted: local || facts.trusted,
        instance_id: provider.instance_id,
        provider_kind: provider.provider_kind,
        provider_id: provider.provider_id,
        capability_id: ctx.capability_id.clone(),
        local,
        score: total,
        reasons,
        load: provider.load,
        max_concurrent: max,
        provided_tools: provider.provided_tools,
        recent_success_rate,
        eta: facts.eta,
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// The capability's definition facts, if it is projected
pub fn definition(
    conn: &Connection,
    capability_id: &str,
) -> rusqlite::Result<Option<DefinitionFacts>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT definition_json FROM capabilities_projection WHERE id = ?1",
            [capability_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(json.map(
        |json| match serde_json::from_str::<DefinitionFacts>(&json) {
            Ok(facts) => facts,
            Err(e) => {
                log::warn!("Capability '{capability_id}': unreadable definition ({e})");
                DefinitionFacts::default()
            }
        },
    ))
}

fn tool_list(json: Option<String>) -> Vec<String> {
    let mut tools: Vec<String> = json
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
        .iter()
        .map(|t| normalize_id(t))
        .collect();
    tools.sort();
    tools.dedup();
    tools
}

/// Enabled `capability_providers` rows for the capability; local ones
/// carry their running task count as load
pub fn registered_providers(
    conn: &Connection,
    capability_id: &str,
    local_instance: &str,
) -> rusqlite::Result<Vec<RawProvider>> {
    let mut stmt = conn.prepare(
        "SELECT instance_id, provider_kind, provider_id, max_concurrent, provided_tools
         FROM capability_providers
         WHERE capability_id = ?1 AND COALESCE(enabled, 1) != 0",
    )?;
    let rows = stmt
        .query_map([capability_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut providers = Vec::with_capacity(rows.len());
    for (instance_id, kind, provider_id, max, tools) in rows {
        let load = if instance_id == local_instance {
            running_tasks(conn, &instance_id, &provider_id)?
        } else {
            0
        };
        providers.push(RawProvider {
            provider_kind: serde_json::from_value(serde_json::Value::String(kind))
                .unwrap_or_default(),
            instance_id,
            provider_id,
            load,
            max_concurrent: max.map_or(1, |m| m.max(1) as u32),
            provided_tools: tool_list(tools),
        });
    }
    Ok(providers)
}

/// Tasks a provider is working on now
pub fn running_tasks(
    conn: &Connection,
    instance_id: &str,
    provider_id: &str,
) -> rusqlite::Result<u32> {
    let mut stmt = conn
        .prepare("SELECT state FROM tasks WHERE provider_instance = ?1 AND provider_agent = ?2")?;
    let states = stmt
        .query_map(params![instance_id, provider_id], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(states
        .iter()
        .filter_map(|s| TaskState::parse(s))
        .filter(|s| !s.is_terminal())
        .count() as u32)
}

/// Relay presence as providers (normalized tools)
pub fn from_presence(presence: Vec<InstancePresence>) -> Vec<RawProvider> {
    presence
        .into_iter()
        .map(|p| {
            let mut tools: Vec<String> = p
                .provider
                .provided_tools
                .iter()
                .map(|t| normalize_id(t))
                .collect();
            tools.sort();
            tools.dedup();
            RawProvider {
                instance_id: p.instance_id,
                provider_kind: p.provider.provider_kind,
                provider_id: p.provider.provider_id,
                load: p.provider.load,
                max_concurrent: p.provider.max_concurrent.max(1),
                provided_tools: tools,
            }
        })
        .collect()
}

/// `registered` with `remote` merged in: a provider both list keeps the
/// relay's (fresher) load and tools, except our own instance's
pub fn merge(
    registered: Vec<RawProvider>,
    remote: Vec<RawProvider>,
    local_instance: &str,
) -> Vec<RawProvider> {
    let mut by_key: HashMap<String, RawProvider> = HashMap::new();
    let mut order = Vec::new();
    for provider in registered {
        let key = provider.key();
        if !by_key.contains_key(&key) {
            order.push(key.clone());
        }
        by_key.insert(key, provider);
    }
    for provider in remote {
        let key = provider.key();
        match by_key.get(&key) {
            Some(_) if provider.instance_id == local_instance => {}
            Some(_) => {
                by_key.insert(key, provider);
            }
            None => {
                order.push(key.clone());
                by_key.insert(key, provider);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|key| by_key.remove(&key))
        .collect()
}

/// History for one provider of `capability_id`
pub fn provider_facts(
    conn: &Connection,
    provider: &RawProvider,
    capability_id: &str,
    now: i64,
) -> rusqlite::Result<CandidateFacts> {
    let instance = provider.instance_id.as_str();
    let (trusted, paused) = conn
        .query_row(
            "SELECT added_as_trusted_peer, delegations_paused FROM trust_relationships
             WHERE instance_id = ?1",
            [instance],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
        )
        .optional()?
        .unwrap_or((false, false));
    let known: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE provider_instance = ?1 OR requester_instance = ?1)",
        [instance],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT state FROM tasks
         WHERE provider_instance = ?1 AND provider_agent = ?2 AND capability = ?3
         ORDER BY updated_at DESC LIMIT ?4",
    )?;
    let finished: Vec<TaskState> = stmt
        .query_map(
            params![
                instance,
                provider.provider_id,
                capability_id,
                SUCCESS_WINDOW
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .filter_map(|s| TaskState::parse(s))
        .filter(|s| s.is_terminal() && *s != TaskState::Cancelled)
        .collect();
    let succeeded = finished
        .iter()
        .filter(|s| **s == TaskState::Completed)
        .count() as u32;

    let mut stmt = conn.prepare(
        "SELECT duration_seconds FROM eta_history
         WHERE capability = ?1 AND provider_agent = ?2
         ORDER BY at DESC LIMIT ?3",
    )?;
    let durations = stmt
        .query_map(params![capability_id, provider.key(), ETA_WINDOW], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(CandidateFacts {
        trusted,
        known,
        paused,
        outcomes: (succeeded, finished.len() as u32),
        eta: eta_from(durations.into_iter().map(|d| d.max(0) as u32).collect()),
        circuit_open: circuit::is_open_conn(
            conn,
            instance,
            &provider.provider_id,
            capability_id,
            now,
        )?,
    })
}

impl DbSelection {
    /// `SelectionService::capability_ranking`
    pub(super) async fn rank_capability(
        &self,
        ctx: &CapabilitySelectionContext,
    ) -> Result<CapabilityRanking, SelectionError> {
        let capability_id = ctx.capability_id.trim().to_string();
        if capability_id.is_empty() {
            return Err("Choose a capability".to_string());
        }
        let remote = match &self.presence {
            Some(presence) => match presence(capability_id.clone()).await {
                Ok(found) => from_presence(found),
                Err(e) => {
                    log::debug!("No relay presence for '{capability_id}': {e}");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let now = (self.clock)();
        let local_instance = ctx.caller_instance.clone();
        let (definition, providers, facts) = self
            .db
            .call(move |conn| {
                let definition = definition(conn, &capability_id)?;
                let registered = registered_providers(conn, &capability_id, &local_instance)?;
                let providers = merge(registered, remote, &local_instance);
                let facts = providers
                    .iter()
                    .map(|p| Ok((p.key(), provider_facts(conn, p, &capability_id, now)?)))
                    .collect::<rusqlite::Result<HashMap<String, CandidateFacts>>>()?;
                Ok((definition, providers, facts))
            })
            .await?;
        let req = Requirements::merge(definition.as_ref(), ctx);
        let turn = if req.strategy == CapabilityRouteStrategy::RoundRobin {
            self.next_turn(&ctx.capability_id)
        } else {
            0
        };
        let considered = providers.iter().map(RawProvider::key).collect();
        let candidates = rank_providers(
            ctx,
            &req,
            providers,
            |p| facts.get(&p.key()).cloned().unwrap_or_default(),
            turn,
        );
        Ok(CapabilityRanking {
            candidates,
            defined: definition.is_some(),
            required_tools: req.required_tools,
            considered,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::Db;
    use crate::services::relay::protocol::ProviderPresence;
    use crate::services::selection::SelectionService;

    fn provider(instance: &str, id: &str, load: u32, tools: &[&str]) -> RawProvider {
        RawProvider {
            instance_id: instance.to_string(),
            provider_kind: ProviderKind::Agent,
            provider_id: id.to_string(),
            load,
            max_concurrent: 4,
            provided_tools: tools.iter().map(|t| normalize_id(t)).collect(),
        }
    }

    fn ctx(capability: &str) -> CapabilitySelectionContext {
        CapabilitySelectionContext {
            capability_id: capability.to_string(),
            caller_instance: "me".into(),
            ..CapabilitySelectionContext::default()
        }
    }

    fn keys(ranked: &[ProviderCandidate]) -> Vec<String> {
        ranked.iter().map(key_of).collect()
    }

    fn req(strategy: CapabilityRouteStrategy, tools: &[&str]) -> Requirements {
        Requirements {
            required_tools: tools.iter().map(|t| normalize_id(t)).collect(),
            strategy,
            ..Requirements::default()
        }
    }

    #[test]
    fn filters_by_required_tools() {
        let providers = vec![
            provider("a", "with-shell", 0, &["shell", "read_file"]),
            provider("b", "without", 0, &["read_file"]),
            provider("c", "prefixed", 0, &["builtin.shell"]),
        ];
        let ranked = rank_providers(
            &ctx("code-review"),
            &req(CapabilityRouteStrategy::LeastLoaded, &["shell"]),
            providers,
            |_| CandidateFacts::default(),
            0,
        );
        let mut found = keys(&ranked);
        found.sort();
        assert_eq!(found, ["a/with-shell", "c/prefixed"]);
        assert!(ranked
            .iter()
            .all(|c| c.reasons.contains(&"has every required tool".to_string())));
    }

    #[test]
    fn excludes_circuit_broken_and_paused_providers() {
        let providers = vec![
            provider("a", "broken", 0, &[]),
            provider("b", "paused", 0, &[]),
            provider("c", "fine", 0, &[]),
        ];
        let ranked = rank_providers(
            &ctx("x"),
            &req(CapabilityRouteStrategy::LeastLoaded, &[]),
            providers,
            |p| CandidateFacts {
                circuit_open: p.provider_id == "broken",
                paused: p.provider_id == "paused",
                ..CandidateFacts::default()
            },
            0,
        );
        assert_eq!(keys(&ranked), ["c/fine"]);
    }

    #[test]
    fn respects_preferred_and_excluded() {
        let providers = vec![
            provider("a", "one", 0, &[]),
            provider("b", "two", 1, &[]),
            provider("c", "three", 0, &[]),
        ];
        let requirements = Requirements {
            preferred: vec!["b/two".into()],
            excluded: vec!["c".into()],
            ..req(CapabilityRouteStrategy::LeastLoaded, &[])
        };
        let ranked = rank_providers(
            &ctx("x"),
            &requirements,
            providers,
            |_| CandidateFacts::default(),
            0,
        );
        assert_eq!(keys(&ranked), ["b/two", "a/one"]);
        assert!(ranked[0].reasons.contains(&"preferred".to_string()));
    }

    #[test]
    fn strategies_pick_their_part() {
        let providers = vec![
            provider("busy", "fast", 3, &[]),
            provider("idle", "slow", 0, &[]),
        ];
        let facts = |p: &RawProvider| CandidateFacts {
            eta: eta_from(vec![if p.provider_id == "fast" { 5 } else { 900 }]),
            ..CandidateFacts::default()
        };
        let pick = |strategy| {
            rank_providers(&ctx("x"), &req(strategy, &[]), providers.clone(), facts, 0)[0]
                .provider_id
                .clone()
        };
        assert_eq!(pick(CapabilityRouteStrategy::LeastLoaded), "slow");
        assert_eq!(pick(CapabilityRouteStrategy::Fastest), "fast");

        // Round robin takes turns over the same set
        let turns: Vec<String> = (0..3)
            .map(|turn| {
                rank_providers(
                    &ctx("x"),
                    &req(CapabilityRouteStrategy::RoundRobin, &[]),
                    providers.clone(),
                    facts,
                    turn,
                )[0]
                .provider_id
                .clone()
            })
            .collect();
        assert_eq!(turns, ["fast", "slow", "fast"]);

        for strategy in [
            CapabilityRouteStrategy::LeastLoaded,
            CapabilityRouteStrategy::RoundRobin,
            CapabilityRouteStrategy::MostTrusted,
            CapabilityRouteStrategy::Fastest,
            CapabilityRouteStrategy::HighestSuccessRate,
        ] {
            let w = CapabilityWeights::for_strategy(strategy);
            let sum = w.capability + w.trust + w.success + w.load + w.eta + w.cost + w.tools;
            assert!((sum - 1.0).abs() < 1e-5, "{strategy:?}");
        }
    }

    #[test]
    fn the_definition_and_context_merge() {
        let json = r#"{
            "id": "code-review.python",
            "requiredTools": ["shell"],
            "routing": {
                "strategy": "fastest",
                "requireTools": ["github.create_review"],
                "preferredProviders": ["peer/reviewer"],
                "excludedProviders": ["bad"]
            },
            "policy": { "maxConcurrentPerProvider": 2 }
        }"#;
        let def: DefinitionFacts = serde_json::from_str(json).unwrap();
        let context = CapabilitySelectionContext {
            required_tools: vec!["read_file".into()],
            excluded: vec!["worse".into()],
            ..ctx("code-review.python")
        };
        let merged = Requirements::merge(Some(&def), &context);
        assert_eq!(
            merged.required_tools,
            ["builtin.read_file", "builtin.shell", "github.create_review"]
        );
        assert_eq!(merged.strategy, CapabilityRouteStrategy::Fastest);
        assert_eq!(merged.preferred, ["peer/reviewer"]);
        assert_eq!(merged.excluded, ["worse", "bad"]);
        assert_eq!(merged.max_concurrent_per_provider, 2);
        // A strategy it can't read is the default
        let odd: DefinitionFacts =
            serde_json::from_str(r#"{"routing": {"strategy": "telepathy"}}"#).unwrap();
        assert_eq!(odd.routing.strategy, CapabilityRouteStrategy::LeastLoaded);
    }

    fn presence(instance: &str, id: &str, load: u32, tools: &[&str]) -> InstancePresence {
        InstancePresence {
            instance_id: instance.to_string(),
            capability_version: "1.0.0".into(),
            provider: ProviderPresence {
                provider_kind: ProviderKind::Workflow,
                provider_id: id.to_string(),
                load,
                max_concurrent: 2,
                provided_tools: tools.iter().map(|t| t.to_string()).collect(),
                online_since: 0.0,
                last_heartbeat: 0.0,
            },
        }
    }

    #[tokio::test]
    async fn ranks_local_and_relay_providers_from_sqlite() {
        let db = Db::in_memory();
        {
            let conn = db.conn().unwrap();
            conn.execute(
                "INSERT INTO capabilities_projection (id, version, name, definition_json)
                 VALUES ('code-review', '1.0.0', 'Code review', ?1)",
                [r#"{"requiredTools": ["shell"], "routing": {"strategy": "least_loaded"}}"#],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capability_providers
                     (capability_id, instance_id, provider_kind, provider_id, enabled, max_concurrent, provided_tools)
                 VALUES ('code-review', 'me', 'agent', 'local-reviewer', 1, 2, '[\"shell\"]'),
                        ('code-review', 'me', 'agent', 'no-shell', 1, 2, '[\"read_file\"]'),
                        ('code-review', 'me', 'agent', 'disabled', 0, 2, '[\"shell\"]')",
                [],
            )
            .unwrap();
            // The local reviewer is busy with one task
            conn.execute(
                "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                     provider_agent, capability, state, created_at, updated_at)
                 VALUES ('t1', 'peer', 'peer/a', 'me', 'local-reviewer', 'code-review', 'working', 0, 0)",
                [],
            )
            .unwrap();
        }
        let source: crate::services::selection::PresenceSource = Arc::new(|capability: String| {
            Box::pin(async move {
                if capability != "code-review" {
                    return Ok(Vec::new());
                }
                Ok(vec![
                    presence(
                        "peer",
                        "pipeline",
                        0,
                        &["builtin.shell", "github.create_review"],
                    ),
                    presence("peer", "toolless", 0, &[]),
                ])
            })
        });
        let selection = DbSelection::new(db.clone(), None).with_presence(source);
        let ranking = selection
            .capability_ranking(&ctx("code-review"))
            .await
            .unwrap();
        assert!(ranking.defined);
        assert_eq!(ranking.required_tools, ["builtin.shell"]);
        let found = keys(&ranking.candidates);
        assert_eq!(found, ["peer/pipeline", "me/local-reviewer"]);
        let local = &ranking.candidates[1];
        assert!(local.local);
        assert_eq!(local.load, 1);
        assert_eq!(ranking.candidates[0].provider_kind, ProviderKind::Workflow);

        // Three failures open the pipeline's circuit; it drops out
        for _ in 0..3 {
            selection
                .record_provider_outcome("peer", "pipeline", "code-review", false)
                .await
                .unwrap();
        }
        let ranked = selection
            .rank_for_capability(&ctx("code-review"))
            .await
            .unwrap();
        assert_eq!(keys(&ranked), ["me/local-reviewer"]);

        // An undefined capability ranks what is registered, unfiltered
        let undefined = selection.capability_ranking(&ctx("unknown")).await.unwrap();
        assert!(!undefined.defined);
    }
}
