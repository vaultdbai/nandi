//! Failover (wave final, Track B). A failed or timed out attempt below
//! [`MAX_ATTEMPTS`] becomes a new attempt (`source_kind = Failover`,
//! `previous_attempt_id`) on the next provider the relay plans, without
//! every instance the task already went to.
//!
//! Two callers fail over: the delegator, for a delegation a caller still
//! waits on (its attempt loop, see `delegator`), and
//! [`TaskService::failover`] for one nobody waits on any more (timed out by
//! the sweep, rejected after a restart, a queued task sent later). Both ask
//! the relay for a new plan (`route`): who to try and remote circuits are
//! the relay's. Step 11 of the relay design moves failover itself to the
//! relay (pre-sealed candidates).
//!
//! [`capability_order`] is the local half: this Nandi's own providers,
//! ordered by `selection::capability` (tools, local circuits, strategy).

use std::collections::HashSet;
use std::sync::Arc;

use super::delegator::instance_of;
use super::route;
use super::task::{self, TaskRecord, TaskService, TaskSourceKind, TaskStage, TaskState};
use super::task_state::{self, Change};
use super::Task;
use crate::db::{now_ms, Db};
use crate::services::capability::ProviderKind;
use crate::services::domain::AgentCard;
use crate::services::selection::{CapabilityRanking, CapabilitySelectionContext, SelectionService};
use crate::services::team::trust::TrustFilter;

pub const MAX_ATTEMPTS: u32 = 3;

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

/// `queue` (this Nandi's own target card IDs, best first) reordered by
/// the local capability-aware ranking (`SelectionService::capability_ranking`):
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
        let hints = route::hints(self.deps.store(), self.db(), &failed.capability, &excluded);
        let query = route::query(&failed.capability, None, hints);
        let next = match relay.route(&query).await {
            Ok(plan) => route::picks(plan).into_iter().next(),
            Err(e) => {
                log::info!("Task {}: no plan to fail over with ({e})", failed.id);
                None
            }
        };
        let Some(next) = next else {
            log::info!("Task {}: no other provider to fail over to", failed.id);
            return Ok(None);
        };

        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms() as f64;
        let record = next_attempt(failed, &id, &next.target);
        let task = Task {
            id: id.clone(),
            target: next.target.clone(),
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
        if let Some(why) = next.route_message() {
            task::add_message(self.db(), &id, route::ROUTE_KIND, why).await?;
        }
        log::info!(
            "Task {} failed over to '{}' as {id} (attempt {})",
            failed.id,
            next.target,
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
            Err(e) => log::info!("Task {id} waits for {}: {e}", instance_of(&next.target)),
        }
        Ok(Some(self.get(&id).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::a2a::task::tests::record;

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
}
