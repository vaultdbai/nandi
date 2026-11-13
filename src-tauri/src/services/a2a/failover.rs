//! The local half of picking a provider: this Nandi's own providers,
//! ordered by `selection::capability` (tools, local circuits, strategy)
//! in [`capability_order`], and the trust filter over cards
//! ([`without_untrusted`]). Failing over to someone else's provider is
//! the relay's (relay design step 11): the requester seals the request for
//! each candidate of the relay's plan and the relay gives it to the next
//! one when the first can't take it.

use std::collections::HashSet;
use std::sync::Arc;

use super::route;
use super::task::{self, TaskRecord, TaskService, TaskSourceKind, TaskState};
use super::Task;
use crate::db::{now_ms, Db};
use crate::services::capability::ProviderKind;
use crate::services::domain::AgentCard;
use crate::services::relay::unavailable;
use crate::services::selection::{CapabilityRanking, CapabilitySelectionContext, SelectionService};
use crate::services::team::trust::TrustFilter;

/// `cards` without other instances we paused delegations with, nor agents
/// of theirs we blocked (`team::trust`). Our own instance is never
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
    /// The user's retry of a failed request of ours: a new attempt
    /// (`source_kind = Failover`, linked by `previous_attempt_id`), planned
    /// by the relay without anyone it already went to, and sealed for each
    /// of the plan's candidates so the relay fails it over itself.
    /// `Ok(None)`: nobody else to try (or it didn't fail).
    pub async fn retry(&self, failed: &TaskRecord) -> Result<Option<TaskRecord>, String> {
        let me = self.instance_id().to_string();
        let retryable = matches!(
            failed.state,
            TaskState::Failed | TaskState::TimedOut | TaskState::Rejected | TaskState::Abandoned
        );
        if !retryable || !failed.is_requester(&me) {
            return Ok(None);
        }
        let Some(request) = task::request_of(self.db(), &failed.id).await? else {
            return Ok(None);
        };
        let relay = unavailable::connected(self.relay())?.clone();
        // Every instance this request already went to
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
        let picks = route::picks(relay.route(&query).await?);
        let Some((first, rest)) = picks.split_first() else {
            return Ok(None);
        };
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms() as f64;
        let mut record = TaskRecord::requester(
            &id,
            &failed.requester_instance,
            &failed.requester_agent,
            &first.target,
            &failed.capability,
            &failed.input,
            failed.deadline,
        );
        record.priority = failed.priority;
        record.parent_task_id = failed.parent_task_id.clone();
        record.source_kind = TaskSourceKind::Failover;
        record.attempt_number = failed.attempt_number + 1;
        record.previous_attempt_id = Some(failed.id.clone());
        let task = Task {
            id: id.clone(),
            target: first.target.clone(),
            created_at: now,
            // Due like any request to another Nandi
            deadline: Some(super::assignment::request_due_at(
                &request.capability,
                &request.input,
                now as i64,
            ) as f64),
            ..request
        };
        record.deadline = task.deadline;
        self.create(&record, Some(&task)).await?;
        if let Some(why) = first.route_message() {
            task::add_message(self.db(), &id, route::ROUTE_KIND, why).await?;
        }
        let alternates: Vec<String> = rest.iter().map(|pick| pick.target.clone()).collect();
        let submitted = relay.delegate_task(&task, &alternates).await?;
        super::queue::accepted(self, &id, submitted.held).await;
        log::info!(
            "Task {} retried as {id} (attempt {})",
            failed.id,
            record.attempt_number
        );
        Ok(Some(self.get(&id).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
