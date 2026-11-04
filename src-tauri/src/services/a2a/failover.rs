//! Failover and circuit breakers (wave final, Track B). A failed or timed
//! out attempt below [`MAX_ATTEMPTS`] becomes a new attempt
//! (`source_kind = Failover`, `previous_attempt_id`) on the next provider
//! `SelectionService::rank` offers, excluding the failed one.
//! [`CIRCUIT_THRESHOLD`] consecutive failures of an (instance, capability)
//! open its circuit for [`CIRCUIT_OPEN_SECS`].
//!
//! Two callers fail over: the delegator, for a delegation a caller still
//! waits on (its attempt loop, see `delegator`), and
//! [`TaskService::failover`] for one nobody waits on any more (timed out by
//! the sweep, rejected after a restart, a queued task sent later).
//! Without a `SelectionService` (until Track D) [`rank_next`] falls back to
//! `ranking::rank_providers`.
//!
//! The circuit-breaker helpers ([`record_failure`], [`record_success`],
//! [`is_circuit_open`]) are the only writers of `circuit_breakers`; Track
//! D's `SelectionService::record_outcome` / `is_circuit_open` are meant to
//! call them. A circuit closes on its own [`CIRCUIT_OPEN_SECS`] after it
//! opened (the next check resets it), or at the next success.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::delegator::instance_of;
use super::ranking::{self, RankContext};
use super::task::{
    self, TaskRecord, TaskService, TaskSourceKind, TaskStage, TaskState, TASK_CIRCUIT_OPENED,
};
use super::task_state::{self, Change};
use super::Task;
use crate::db::{now_ms, Db};
use crate::services::capability::ProviderKind;
use crate::services::domain::AgentCard;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::gateway::GatewayCard;
use crate::services::selection::{
    CapabilityRanking, CapabilitySelectionContext, CircuitBreakerState, SelectionContext,
    SelectionService,
};
use crate::services::team::trust::TrustFilter;

pub const MAX_ATTEMPTS: u32 = 3;
pub const CIRCUIT_THRESHOLD: u32 = 3;
pub const CIRCUIT_OPEN_SECS: u64 = 300;

/// `task://circuit_opened`: the `circuit_breakers` row
#[derive(Serialize)]
#[serde(transparent)]
struct CircuitOpened<'a>(&'a CircuitBreakerState);

impl NandiEvent for CircuitOpened<'_> {
    const NAME: &'static str = TASK_CIRCUIT_OPENED;
}

// ---------------------------------------------------------------------------
// Circuit breakers
// ---------------------------------------------------------------------------

pub fn circuit(
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
                opened_at: row.get::<_, Option<i64>>(1)?.map(|t| t as f64),
            })
        },
    )
    .optional()
}

/// Open within [`CIRCUIT_OPEN_SECS`] of `opened_at`; an expired circuit is
/// closed (reset) here.
pub fn is_circuit_open_conn(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let Some(opened_at) = circuit(conn, instance_id, capability)?.and_then(|c| c.opened_at) else {
        return Ok(false);
    };
    if now - (opened_at as i64) < CIRCUIT_OPEN_SECS as i64 * 1000 {
        return Ok(true);
    }
    record_success_conn(conn, instance_id, capability)?;
    Ok(false)
}

/// One more consecutive failure. Returns the row and whether this one
/// opened the circuit.
pub fn record_failure_conn(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
    now: i64,
) -> rusqlite::Result<(CircuitBreakerState, bool)> {
    // An expired circuit starts over
    is_circuit_open_conn(conn, instance_id, capability, now)?;
    conn.execute(
        "INSERT INTO circuit_breakers (instance_id, capability, consecutive_failures)
         VALUES (?1, ?2, 1)
         ON CONFLICT(instance_id, capability)
         DO UPDATE SET consecutive_failures = consecutive_failures + 1",
        params![instance_id, capability],
    )?;
    let mut state = circuit(conn, instance_id, capability)?.unwrap_or(CircuitBreakerState {
        instance_id: instance_id.to_string(),
        capability: capability.to_string(),
        consecutive_failures: 1,
        opened_at: None,
    });
    let opens = state.opened_at.is_none() && state.consecutive_failures >= CIRCUIT_THRESHOLD;
    if opens {
        conn.execute(
            "UPDATE circuit_breakers SET opened_at = ?3 WHERE instance_id = ?1 AND capability = ?2",
            params![instance_id, capability, now],
        )?;
        state.opened_at = Some(now as f64);
    }
    Ok((state, opens))
}

/// Resets the failures and closes the circuit
pub fn record_success_conn(
    conn: &Connection,
    instance_id: &str,
    capability: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM circuit_breakers WHERE instance_id = ?1 AND capability = ?2",
        params![instance_id, capability],
    )?;
    Ok(())
}

/// [`record_failure_conn`] now; emits `task://circuit_opened` when it opens
pub async fn record_failure(
    db: &Db,
    bus: &EventBus,
    instance_id: &str,
    capability: &str,
) -> Result<CircuitBreakerState, String> {
    let (instance, cap) = (instance_id.to_string(), capability.to_string());
    let (state, opened) = db
        .call(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let outcome = record_failure_conn(&tx, &instance, &cap, now_ms())?;
            tx.commit()?;
            Ok(outcome)
        })
        .await?;
    if opened {
        log::info!(
            "Circuit opened for '{capability}' on {instance_id} after {} failures",
            state.consecutive_failures
        );
        events::emit(bus, &CircuitOpened(&state));
    }
    Ok(state)
}

pub async fn record_success(db: &Db, instance_id: &str, capability: &str) -> Result<(), String> {
    let (instance, cap) = (instance_id.to_string(), capability.to_string());
    db.call(move |conn| record_success_conn(conn, &instance, &cap))
        .await
}

pub async fn is_circuit_open(db: &Db, instance_id: &str, capability: &str) -> Result<bool, String> {
    let (instance, cap) = (instance_id.to_string(), capability.to_string());
    db.call(move |conn| is_circuit_open_conn(conn, &instance, &cap, now_ms()))
        .await
}

// ---------------------------------------------------------------------------
// Attempts
// ---------------------------------------------------------------------------

/// The attempt after `previous`, to `provider_card`
pub fn next_attempt(previous: &TaskRecord, id: &str, provider_card: &str) -> TaskRecord {
    let mut next = TaskRecord::requester(
        id,
        &previous.requester_instance,
        &previous.requester_agent,
        provider_card,
        &previous.capability,
        &previous.input,
        previous.deadline,
    );
    next.priority = previous.priority;
    next.parent_task_id = previous.parent_task_id.clone();
    next.source_kind = TaskSourceKind::Failover;
    next.attempt_number = previous.attempt_number + 1;
    next.previous_attempt_id = Some(previous.id.clone());
    next
}

/// One gateway card per instance, for `SelectionService::rank`
fn gateway_cards(cards: &[AgentCard]) -> Vec<GatewayCard> {
    let mut gateways: Vec<GatewayCard> = Vec::new();
    for card in cards {
        let names = card.capabilities.iter().map(|c| c.name.clone());
        match gateways
            .iter_mut()
            .find(|g| g.instance_id == card.instance_id)
        {
            Some(gateway) => {
                for name in names {
                    if !gateway.capabilities.contains(&name) {
                        gateway.capabilities.push(name);
                    }
                }
                gateway.accepts_delegation |= card.accepts_delegation;
                gateway.direct_delegation =
                    Some(gateway.direct_delegation == Some(true) || card.accepts_delegation);
                gateway.max_concurrent += card.max_concurrent;
                gateway.current_load += card.current_load;
                gateway.agent_count += 1;
            }
            None => gateways.push(GatewayCard {
                instance_id: card.instance_id.clone(),
                endpoint: card.endpoint.clone(),
                display_name: card.owner.display_name.clone(),
                description: String::new(),
                capabilities: names.collect(),
                accepts_delegation: card.accepts_delegation,
                max_concurrent: card.max_concurrent,
                current_load: card.current_load,
                owner: card.owner.clone(),
                federation_enabled: false,
                gateway_version: 0,
                agent_count: 1,
                body: None,
                direct_delegation: Some(card.accepts_delegation),
                responsibilities: Vec::new(),
                availability: Default::default(),
            }),
        }
    }
    gateways
}

/// `cards` without other instances we paused delegations with, nor agents
/// of theirs we blocked (Track E, `team::trust`). Our own instance is never
/// filtered. A trust table that can't be read filters nothing (logged).
pub fn without_untrusted(db: &Db, me: &str, cards: Vec<AgentCard>) -> Vec<AgentCard> {
    let trust = match TrustFilter::load(db) {
        Ok(trust) => trust,
        Err(e) => {
            log::warn!("Trust check failed: {e}");
            return cards;
        }
    };
    cards
        .into_iter()
        .filter(|card| {
            card.instance_id == me
                || !(trust.is_paused(&card.instance_id)
                    || trust.is_blocked(&card.instance_id, card.agent_id()))
        })
        .collect()
}

/// The providers to try next, best first, without `excluded` instances:
/// `selection`'s order when there is one (and it answers), else
/// `ranking::rank_providers`.
pub async fn rank_next(
    selection: Option<&Arc<dyn SelectionService>>,
    capability: &str,
    caller_instance: &str,
    requester_card: &str,
    cards: Vec<AgentCard>,
    excluded: &HashSet<String>,
) -> Vec<AgentCard> {
    let cards: Vec<AgentCard> = cards
        .into_iter()
        .filter(|card| !excluded.contains(&card.instance_id))
        .collect();
    let ctx = RankContext::bare(caller_instance, requester_card);
    let fallback: Vec<AgentCard> = ranking::rank_providers(&cards, capability, &ctx)
        .into_iter()
        .cloned()
        .collect();
    let Some(selection) = selection else {
        return fallback;
    };
    let instance_ranked =
        rank_instances(selection, capability, caller_instance, excluded, fallback).await;
    // Wave A: then by provider (tools, provider circuits, strategy)
    let queue = instance_ranked
        .into_iter()
        .map(|card| (card.id.clone(), Some(card)))
        .collect();
    capability_order(
        Some(selection),
        capability,
        caller_instance,
        excluded,
        queue,
    )
    .await
    .into_iter()
    .filter_map(|(_, card)| card)
    .collect()
}

/// `SelectionService::rank` over the instances' gateway cards, mapped
/// back to `fallback`'s agent cards
async fn rank_instances(
    selection: &Arc<dyn SelectionService>,
    capability: &str,
    caller_instance: &str,
    excluded: &HashSet<String>,
    fallback: Vec<AgentCard>,
) -> Vec<AgentCard> {
    let context = SelectionContext {
        capability: capability.to_string(),
        caller_instance: caller_instance.to_string(),
        excluded: excluded.iter().cloned().collect(),
        ..SelectionContext::default()
    };
    match selection.rank(&context, gateway_cards(&fallback)).await {
        Ok(matches) => {
            let mut ordered: Vec<AgentCard> = Vec::new();
            for m in &matches {
                for card in &fallback {
                    // Selection ranks instances (gateway cards): a match for
                    // the gateway covers every agent of that instance
                    let agent_matches = m.agent_id.is_empty()
                        || m.agent_id == crate::services::gateway::GATEWAY_AGENT_ID
                        || card.agent_id() == m.agent_id;
                    if card.instance_id == m.instance_id
                        && agent_matches
                        && !ordered.iter().any(|c| c.id == card.id)
                    {
                        ordered.push(card.clone());
                    }
                }
            }
            ordered
        }
        Err(e) => {
            log::warn!("Selection couldn't rank '{capability}' providers: {e}");
            fallback
        }
    }
}

/// Wave A (Track D): `queue` (target card IDs, best first) reordered by
/// the capability-aware ranking (`SelectionService::capability_ranking`):
///
/// - ranked providers first, in its order; a ranked workflow provider
///   without a card joins the queue as `(<instance>/<workflow>, None)`
///   (`workflow_provider::try_dispatch` runs it)
/// - providers it considered but filtered out (missing a required tool,
///   excluded, an open provider circuit) are dropped
/// - providers it has no presence for stay
///   after the ranked ones, unless the capability requires tools: their
///   tools are unknown
///
/// Without a selection service, or when it can't rank (no capability
/// definition and no registered providers), `queue` is unchanged.
pub async fn capability_order(
    selection: Option<&Arc<dyn SelectionService>>,
    capability: &str,
    caller_instance: &str,
    excluded: &HashSet<String>,
    queue: Vec<(String, Option<AgentCard>)>,
) -> Vec<(String, Option<AgentCard>)> {
    let Some(selection) = selection else {
        return queue;
    };
    let ctx = CapabilitySelectionContext {
        capability_id: capability.to_string(),
        caller_instance: caller_instance.to_string(),
        excluded: excluded.iter().cloned().collect(),
        ..CapabilitySelectionContext::default()
    };
    let ranking = match selection.capability_ranking(&ctx).await {
        Ok(ranking) => ranking,
        Err(e) => {
            log::debug!("No capability-aware ranking for '{capability}': {e}");
            return queue;
        }
    };
    order_by_ranking(&ranking, queue)
}

/// The pure part of [`capability_order`]
pub fn order_by_ranking(
    ranking: &CapabilityRanking,
    mut queue: Vec<(String, Option<AgentCard>)>,
) -> Vec<(String, Option<AgentCard>)> {
    if !ranking.defined && ranking.considered.is_empty() {
        return queue;
    }
    let mut ordered = Vec::with_capacity(queue.len());
    for candidate in &ranking.candidates {
        let key = format!("{}/{}", candidate.instance_id, candidate.provider_id);
        if let Some(at) = queue.iter().position(|(target, _)| *target == key) {
            ordered.push(queue.remove(at));
        } else if candidate.provider_kind == ProviderKind::Workflow
            && !ordered.iter().any(|(target, _)| *target == key)
        {
            ordered.push((key, None));
        }
    }
    let strict = !ranking.required_tools.is_empty();
    ordered.extend(
        queue
            .into_iter()
            .filter(|(target, _)| !strict && !ranking.considered.contains(target)),
    );
    ordered
}

impl TaskService {
    /// A new attempt for a failed requester task nobody waits on, on the
    /// next provider (never one tried before, nor one with an open
    /// circuit). `Ok(None)`: nothing to do (not retryable, out of
    /// attempts, already failed over, or no other provider).
    pub async fn failover(&self, failed: &TaskRecord) -> Result<Option<TaskRecord>, String> {
        let me = self.instance_id().to_string();
        let retryable = matches!(
            failed.state,
            TaskState::Failed | TaskState::TimedOut | TaskState::Rejected | TaskState::Abandoned
        );
        if !retryable || !failed.is_requester(&me) || failed.attempt_number >= MAX_ATTEMPTS {
            return Ok(None);
        }
        let failed_id = failed.id.clone();
        let successors = self
            .db()
            .call(move |conn| task::select(conn, "previous_attempt_id = ?1", &[&failed_id]))
            .await?;
        if !successors.is_empty() {
            return Ok(None);
        }
        let Some(request) = task::request_of(self.db(), &failed.id).await? else {
            return Ok(None);
        };
        let Some(relay) = self.relay().cloned() else {
            return Ok(None);
        };

        // Every instance this task already went to
        let mut excluded: HashSet<String> = HashSet::from([me.clone()]);
        let mut at = Some(failed.clone());
        while let Some(attempt) = at {
            excluded.insert(attempt.provider_instance.clone());
            at = match attempt.previous_attempt_id {
                Some(previous) => task::get_row(self.db(), &previous).await?,
                None => None,
            };
        }
        let mut cards = Vec::new();
        for card in relay
            .query_capability(&failed.capability)
            .await
            .unwrap_or_default()
        {
            if !excluded.contains(&card.instance_id)
                && !is_circuit_open(self.db(), &card.instance_id, &failed.capability).await?
            {
                cards.push(card);
            }
        }
        let cards = without_untrusted(self.db(), &me, cards);
        let ranked = rank_next(
            self.selection.as_ref(),
            &failed.capability,
            &me,
            &request.requester.id,
            cards,
            &excluded,
        )
        .await;
        let Some(next) = ranked.into_iter().next() else {
            log::info!("Task {}: no other provider to fail over to", failed.id);
            return Ok(None);
        };

        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms() as f64;
        let record = next_attempt(failed, &id, &next.id);
        let task = Task {
            id: id.clone(),
            target: next.id.clone(),
            created_at: now,
            // Due like any request to another Nandi
            deadline: Some(super::assignment::request_due_at(
                &request.capability,
                &request.input,
                now as i64,
            ) as f64),
            ..request
        };
        self.create(&record, Some(&task)).await?;
        log::info!(
            "Task {} failed over to '{}' as {id} (attempt {})",
            failed.id,
            next.id,
            record.attempt_number
        );
        match relay.delegate_task(&task).await {
            Ok(_) => {
                task_state::advance(
                    self.db(),
                    self.bus(),
                    &id,
                    TaskState::Delivered,
                    Change::reason("routed by the relay").at(TaskStage::Dispatch),
                )
                .await?;
            }
            // Queued: the next flush sends it
            Err(e) => log::info!("Task {id} waits for {}: {e}", instance_of(&next.id)),
        }
        Ok(Some(self.get(&id).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::a2a::task::tests::record;

    #[tokio::test]
    async fn three_failures_open_the_circuit_for_five_minutes() {
        let (bus, sink) = crate::services::events::tests::bus();
        let db = bus.db.clone();
        for _ in 0..2 {
            record_failure(&db, &bus, "them", "summarize")
                .await
                .unwrap();
        }
        assert!(!is_circuit_open(&db, "them", "summarize").await.unwrap());
        assert!(sink.names().is_empty());
        let state = record_failure(&db, &bus, "them", "summarize")
            .await
            .unwrap();
        assert_eq!(state.consecutive_failures, 3);
        assert!(state.opened_at.is_some());
        assert!(is_circuit_open(&db, "them", "summarize").await.unwrap());
        assert!(!is_circuit_open(&db, "them", "translate").await.unwrap());
        assert_eq!(sink.names(), [TASK_CIRCUIT_OPENED]);
        // A fourth doesn't re-open (or re-announce) it
        record_failure(&db, &bus, "them", "summarize")
            .await
            .unwrap();
        assert_eq!(sink.names().len(), 1);

        let opened = state.opened_at.unwrap() as i64;
        let window = CIRCUIT_OPEN_SECS as i64 * 1000;
        let open_at = |now: i64| {
            let db = db.clone();
            async move {
                db.call(move |conn| is_circuit_open_conn(conn, "them", "summarize", now))
                    .await
                    .unwrap()
            }
        };
        assert!(open_at(opened + window - 1).await);
        assert!(!open_at(opened + window).await, "closes after 5 minutes");
        // Closed means reset: three more to open it again
        let failures = db
            .call(|conn| circuit(conn, "them", "summarize"))
            .await
            .unwrap();
        assert!(failures.is_none());
    }

    #[tokio::test]
    async fn a_success_closes_the_circuit() {
        let (bus, _) = crate::services::events::tests::bus();
        let db = bus.db.clone();
        for _ in 0..3 {
            record_failure(&db, &bus, "them", "summarize")
                .await
                .unwrap();
        }
        record_success(&db, "them", "summarize").await.unwrap();
        assert!(!is_circuit_open(&db, "them", "summarize").await.unwrap());
        let state = record_failure(&db, &bus, "them", "summarize")
            .await
            .unwrap();
        assert_eq!(state.consecutive_failures, 1);
    }

    #[test]
    fn the_next_attempt_points_back() {
        let mut previous = record("first");
        previous.priority = 4;
        let next = next_attempt(&previous, "second", "other/papers");
        assert_eq!(next.attempt_number, 2);
        assert_eq!(next.previous_attempt_id.as_deref(), Some("first"));
        assert_eq!(next.source_kind, TaskSourceKind::Failover);
        assert_eq!(next.provider_instance, "other");
        assert_eq!(next.provider_agent, "papers");
        assert_eq!(next.priority, 4);
        assert_eq!(next.state, TaskState::Submitted);
    }

    fn summarizer(id: &str) -> AgentCard {
        let (instance, _) = id.split_once('/').unwrap();
        AgentCard {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            instance_id: instance.into(),
            endpoint: String::new(),
            capabilities: vec![crate::services::domain::capability::ServesCapability {
                name: "summarize".into(),
                description: String::new(),
                input_schema: None,
                output_schema: None,
            }],
            accepts_delegation: true,
            max_concurrent: 4,
            current_load: 0,
            owner: Default::default(),
            signature: None,
            issued_at: 0.0,
        }
    }

    fn provider_row(conn: &Connection, instance: &str, kind: &str, id: &str, tools: &str) {
        conn.execute(
            "INSERT INTO capability_providers
                 (capability_id, instance_id, provider_kind, provider_id, enabled, max_concurrent, provided_tools)
             VALUES ('summarize', ?1, ?2, ?3, 1, 2, ?4)",
            params![instance, kind, id, tools],
        )
        .unwrap();
    }

    #[test]
    fn capability_ranking_orders_drops_and_adds_workflows() {
        use crate::services::selection::ProviderCandidate;
        let candidate = |instance: &str, id: &str, kind| ProviderCandidate {
            instance_id: instance.into(),
            provider_kind: kind,
            provider_id: id.into(),
            capability_id: "summarize".into(),
            local: false,
            score: 1.0,
            reasons: vec![],
            load: 0,
            max_concurrent: 1,
            provided_tools: vec![],
            trusted: false,
            recent_success_rate: None,
            eta: None,
        };
        let queue = || {
            ["a/one", "b/two", "legacy/three"]
                .iter()
                .map(|id| (id.to_string(), Some(summarizer(id))))
                .collect::<Vec<_>>()
        };
        let mut ranking = CapabilityRanking {
            candidates: vec![
                candidate("b", "two", ProviderKind::Agent),
                candidate("me", "flow", ProviderKind::Workflow),
            ],
            defined: true,
            required_tools: vec![],
            // `a/one` was filtered out (an open circuit, say)
            considered: vec!["a/one".into(), "b/two".into(), "me/flow".into()],
        };
        let targets = |q: Vec<(String, Option<AgentCard>)>| -> Vec<String> {
            q.into_iter().map(|(t, _)| t).collect()
        };
        assert_eq!(
            targets(order_by_ranking(&ranking, queue())),
            ["b/two", "me/flow", "legacy/three"]
        );
        // With required tools, providers of unknown tools drop too
        ranking.required_tools = vec!["builtin.shell".into()];
        assert_eq!(
            targets(order_by_ranking(&ranking, queue())),
            ["b/two", "me/flow"]
        );
        // Nothing to go by: unchanged
        let empty = CapabilityRanking::default();
        assert_eq!(
            targets(order_by_ranking(&empty, queue())),
            ["a/one", "b/two", "legacy/three"]
        );
    }

    #[tokio::test]
    async fn failover_skips_providers_without_the_required_tools() {
        let db = Db::in_memory();
        {
            let conn = db.conn().unwrap();
            conn.execute(
                "INSERT INTO capabilities_projection (id, version, name, definition_json)
                 VALUES ('summarize', '1.0.0', 'Summarize', ?1)",
                [r#"{"requiredTools": ["shell"]}"#],
            )
            .unwrap();
            provider_row(&conn, "failed", "agent", "papers", r#"["shell"]"#);
            provider_row(&conn, "toolless", "agent", "papers", "[]");
            provider_row(&conn, "capable", "agent", "papers", r#"["builtin.shell"]"#);
        }
        let selection: Arc<dyn SelectionService> = Arc::new(
            crate::services::selection::DbSelection::new(db.clone(), None),
        );
        let cards = vec![
            summarizer("failed/papers"),
            summarizer("toolless/papers"),
            summarizer("capable/papers"),
            summarizer("unknown/papers"),
        ];
        let excluded = HashSet::from(["me".to_string(), "failed".to_string()]);
        let ranked = rank_next(
            Some(&selection),
            "summarize",
            "me",
            "me/writer",
            cards,
            &excluded,
        )
        .await;
        let ids: Vec<&str> = ranked.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["capable/papers"]);

        // The failover attempt points back at the failed one
        let failed = record("first");
        let next = next_attempt(&failed, "second", &ranked[0].id);
        assert_eq!(next.previous_attempt_id.as_deref(), Some("first"));
        assert_eq!(next.source_kind, TaskSourceKind::Failover);
        assert_eq!(next.provider_instance, "capable");
    }
}
