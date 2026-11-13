//! Evidence per capability (wave A, Track A): one `capability_evidence`
//! row per finished task and role, aggregated per provider over a window.
//!
//! `on_task_transitions` is the seam `a2a::task_state` calls after every
//! committed transition; a change into a terminal state records a row in
//! the background. Existing tasks without evidence rows are simply absent
//! from the aggregates.
//!
//! - Role: `provider` when this instance ran the task, `requester` when it
//!   asked for it; an in-process task (both sides here) gets both rows,
//!   and the aggregate counts it once (the provider row).
//! - Tasks addressed to a gateway aren't evidence: the child task the
//!   gateway routes to the real provider is.
//! - Provider kind comes from `capability_providers` (a workflow provider
//!   is registered as one); unknown providers are agents.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::definition::ProviderKind;
use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::a2a::task::{TaskRecord, TaskState, TaskStateChanged};
use crate::services::gateway::GATEWAY_AGENT_ID;
use crate::services::network;

/// `task_messages` kind of a clarification question (`a2a::clarification`)
const CLARIFICATION_REQUEST_KIND: &str = "clarification_request";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Aggregate over `window_days` for one provider of a capability
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEvidence {
    pub capability_id: String,
    pub capability_version: Option<String>,
    pub instance_id: String,
    pub provider_kind: ProviderKind,
    pub provider_id: String,
    pub window_days: u32,
    pub count: u32,
    pub success_count: u32,
    pub failed_count: u32,
    pub timeout_count: u32,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub tokens_in_total: f64,
    pub tokens_out_total: f64,
    pub cost_usd_total: Option<f64>,
    pub clarification_rounds_avg: Option<f32>,
    pub escalation_rate: Option<f32>,
    pub user_rating_avg: Option<f32>,
    pub last_observed_at: Option<Timestamp>,
}

impl CapabilityEvidence {
    /// Successes over finished tasks, when there are any
    #[cfg(test)]
    pub fn success_rate(&self) -> Option<f32> {
        (self.count > 0).then(|| self.success_count as f32 / self.count as f32)
    }
}

/// Which side of the task this instance was on
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    Requester,
    Provider,
}

impl EvidenceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requester => "requester",
            Self::Provider => "provider",
        }
    }
}

/// Seam (called by `a2a::task_state::{apply, advance}` after commit and
/// emit): for each change into a terminal state, `record_from_task` in the
/// background. Never blocks or fails the transition.
pub fn on_task_transitions(db: &Db, changes: &[TaskStateChanged]) {
    let finished: Vec<TaskRecord> = changes
        .iter()
        .filter(|c| c.event.to_state.is_terminal() && c.event.from_state != Some(c.event.to_state))
        .map(|c| c.task.clone())
        .collect();
    if finished.is_empty() {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        log::debug!(
            "No runtime: evidence for {} tasks not recorded",
            finished.len()
        );
        return;
    };
    let db = db.clone();
    runtime.spawn(async move {
        for task in &finished {
            if let Err(e) = record_from_task(&db, task).await {
                log::warn!("Failed to record evidence for task {}: {e}", task.id);
            }
        }
    });
}

/// Writes the rows for a terminal task (one per role this instance had).
/// Idempotent: a task's rows are replaced, never duplicated.
pub async fn record_from_task(db: &Db, task: &TaskRecord) -> Result<(), String> {
    if !task.state.is_terminal() || task.provider_agent == GATEWAY_AGENT_ID {
        return Ok(());
    }
    let local = network::local_instance_id(db)?;
    let task = task.clone();
    db.call(move |conn| {
        let mut roles = Vec::new();
        if task.provider_instance == local {
            roles.push(EvidenceRole::Provider);
        }
        if task.requester_instance == local {
            roles.push(EvidenceRole::Requester);
        }
        if roles.is_empty() {
            return Ok(());
        }
        let provider: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT provider_kind, capability_version FROM capability_providers
                  WHERE capability_id = ?1 AND instance_id = ?2 AND provider_id = ?3",
                params![task.capability, task.provider_instance, task.provider_agent],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let kind = provider
            .as_ref()
            .and_then(|(kind, _)| ProviderKind::parse(kind))
            .unwrap_or_default();
        let version = match provider.and_then(|(_, version)| version) {
            Some(version) => Some(version),
            None => conn
                .query_row(
                    "SELECT version FROM capabilities_projection WHERE id = ?1",
                    [&task.capability],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten(),
        };
        let clarifications: u32 = conn.query_row(
            "SELECT COUNT(*) FROM task_messages WHERE task_id = ?1 AND kind = ?2",
            params![task.id, CLARIFICATION_REQUEST_KIND],
            |row| row.get(0),
        )?;
        let finished_at = task.completed_at.unwrap_or(task.updated_at);
        let duration_ms = (finished_at >= task.created_at).then_some(finished_at - task.created_at);
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for role in roles {
            tx.execute(
                "INSERT OR REPLACE INTO capability_evidence (id, capability_id,
                    capability_version, task_id, provider_instance, provider_kind, provider_id,
                    role, status, duration_ms, clarification_rounds, escalation_required,
                    approval_required, observed_at, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?14)",
                params![
                    format!("{}:{}", task.id, role.as_str()),
                    task.capability,
                    version,
                    task.id,
                    task.provider_instance,
                    kind.as_str(),
                    task.provider_agent,
                    role.as_str(),
                    task.state.as_str(),
                    duration_ms.map(|ms| ms as i64),
                    clarifications,
                    // Nothing escalates to an owner yet (wave F)
                    0i64,
                    finished_at as i64,
                    serde_json::json!({
                        "attemptNumber": task.attempt_number,
                        "sourceKind": task.source_kind.as_str(),
                        "requesterInstance": task.requester_instance,
                    })
                    .to_string(),
                ],
            )?;
        }
        tx.commit()
    })
    .await
}

/// SQL aggregate for one capability, one entry per provider (every provider
/// when `provider_instance` is `None`). `window_days` 0 is all time. Sorted
/// by invocations, most first.
pub async fn aggregate(
    db: &Db,
    capability_id: &str,
    provider_instance: Option<&str>,
    window_days: u32,
) -> Result<Vec<CapabilityEvidence>, String> {
    let capability = capability_id.to_string();
    let instance = provider_instance.map(str::to_string);
    let since = if window_days == 0 {
        0
    } else {
        now_ms() - i64::from(window_days) * DAY_MS
    };
    let rows = db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT task_id, role, COALESCE(provider_instance, ''), COALESCE(provider_kind, 'agent'),
                        COALESCE(provider_id, ''), capability_version, status, duration_ms,
                        tokens_in, tokens_out, cost_usd, clarification_rounds,
                        escalation_required, user_rating, observed_at
                   FROM capability_evidence
                  WHERE capability_id = ?1 AND observed_at >= ?2
                    AND (?3 IS NULL OR provider_instance = ?3)
                  ORDER BY observed_at",
            )?;
            let rows = stmt
                .query_map(params![capability, since, instance], |row| {
                    Ok(Row {
                        task_id: row.get(0)?,
                        role: row.get(1)?,
                        instance: row.get(2)?,
                        kind: row.get(3)?,
                        provider_id: row.get(4)?,
                        version: row.get(5)?,
                        status: row.get(6)?,
                        duration_ms: row.get(7)?,
                        tokens_in: row.get(8)?,
                        tokens_out: row.get(9)?,
                        cost_usd: row.get(10)?,
                        clarifications: row.get::<_, Option<i64>>(11)?.unwrap_or(0),
                        escalated: row.get::<_, Option<i64>>(12)?.unwrap_or(0) != 0,
                        rating: row.get(13)?,
                        observed_at: row.get(14)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(summarize(capability_id, window_days, rows))
}

struct Row {
    task_id: String,
    role: String,
    instance: String,
    kind: String,
    provider_id: String,
    version: Option<String>,
    status: String,
    duration_ms: Option<i64>,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    cost_usd: Option<f64>,
    clarifications: i64,
    escalated: bool,
    rating: Option<i64>,
    observed_at: i64,
}

fn summarize(capability_id: &str, window_days: u32, rows: Vec<Row>) -> Vec<CapabilityEvidence> {
    // One row per task: the provider's when both sides are here
    let mut per_task: HashMap<String, Row> = HashMap::new();
    for row in rows {
        match per_task.get(&row.task_id) {
            Some(kept) if kept.role == EvidenceRole::Provider.as_str() => {}
            _ => {
                per_task.insert(row.task_id.clone(), row);
            }
        }
    }
    let mut groups: BTreeMap<(String, String, String), Vec<Row>> = BTreeMap::new();
    for row in per_task.into_values() {
        groups
            .entry((
                row.instance.clone(),
                row.kind.clone(),
                row.provider_id.clone(),
            ))
            .or_default()
            .push(row);
    }
    let mut out: Vec<CapabilityEvidence> = groups
        .into_iter()
        .map(|((instance_id, kind, provider_id), mut rows)| {
            rows.sort_by_key(|r| r.observed_at);
            let count = rows.len() as u32;
            let status_count = |names: &[&str]| {
                rows.iter()
                    .filter(|r| names.contains(&r.status.as_str()))
                    .count() as u32
            };
            let mut durations: Vec<i64> = rows
                .iter()
                .filter(|r| ended_with_outcome(&r.status))
                .filter_map(|r| r.duration_ms)
                .collect();
            durations.sort_unstable();
            let costs: Vec<f64> = rows.iter().filter_map(|r| r.cost_usd).collect();
            let ratings: Vec<i64> = rows.iter().filter_map(|r| r.rating).collect();
            CapabilityEvidence {
                capability_id: capability_id.to_string(),
                capability_version: rows.iter().rev().find_map(|r| r.version.clone()),
                instance_id,
                provider_kind: ProviderKind::parse(&kind).unwrap_or_default(),
                provider_id,
                window_days,
                count,
                success_count: status_count(&[TaskState::Completed.as_str()]),
                failed_count: status_count(&[
                    TaskState::Failed.as_str(),
                    TaskState::Rejected.as_str(),
                    TaskState::Abandoned.as_str(),
                ]),
                timeout_count: status_count(&[TaskState::TimedOut.as_str()]),
                p50_ms: percentile(&durations, 50),
                p95_ms: percentile(&durations, 95),
                tokens_in_total: rows.iter().filter_map(|r| r.tokens_in).sum::<i64>() as f64,
                tokens_out_total: rows.iter().filter_map(|r| r.tokens_out).sum::<i64>() as f64,
                cost_usd_total: (!costs.is_empty()).then(|| costs.iter().sum()),
                clarification_rounds_avg: (count > 0).then(|| {
                    rows.iter().map(|r| r.clarifications).sum::<i64>() as f32 / count as f32
                }),
                escalation_rate: (count > 0)
                    .then(|| rows.iter().filter(|r| r.escalated).count() as f32 / count as f32),
                user_rating_avg: (!ratings.is_empty())
                    .then(|| ratings.iter().sum::<i64>() as f32 / ratings.len() as f32),
                last_observed_at: rows.last().map(|r| r.observed_at as Timestamp),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.provider_id.cmp(&b.provider_id))
    });
    out
}

/// Whether an attempt's duration counts toward latency: a cancelled or
/// reassigned attempt was stopped, so it neither succeeded nor failed
pub(super) fn ended_with_outcome(status: &str) -> bool {
    status != TaskState::Cancelled.as_str() && status != TaskState::Reassigned.as_str()
}

/// Nearest-rank percentile of sorted values
fn percentile(sorted: &[i64], pct: usize) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1] as f64)
}

/// Whether this instance has asked for `capability_id` (requester rows)
pub async fn has_requested(db: &Db, capability_id: &str) -> Result<bool, String> {
    let capability = capability_id.to_string();
    db.call(move |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM capability_evidence
              WHERE capability_id = ?1 AND role = 'requester')",
            [capability],
            |row| row.get(0),
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::a2a::task::TaskSourceKind;
    use serde_json::Value;

    fn task(
        id: &str,
        provider: &str,
        state: TaskState,
        duration_ms: f64,
        local: &str,
    ) -> TaskRecord {
        let created = now_ms() as f64 - 10_000.0;
        TaskRecord {
            id: id.into(),
            requester_instance: "remote-requester".into(),
            requester_agent: "asker".into(),
            provider_instance: local.into(),
            provider_agent: provider.into(),
            capability: "code-review.python".into(),
            input: Value::Null,
            state,
            eta_seconds: None,
            deadline: None,
            priority: 0,
            created_at: created,
            updated_at: created + duration_ms,
            acknowledged_at: None,
            completed_at: Some(created + duration_ms),
            error: None,
            result: None,
            parent_task_id: None,
            source_kind: TaskSourceKind::Direct,
            attempt_number: 1,
            previous_attempt_id: None,
        }
    }

    #[tokio::test]
    async fn aggregates_five_tasks() {
        let db = Db::in_memory();
        let local = network::local_instance_id(&db).unwrap();
        let tasks = [
            task("t1", "reviewer", TaskState::Completed, 1000.0, &local),
            task("t2", "reviewer", TaskState::Completed, 2000.0, &local),
            task("t3", "reviewer", TaskState::Failed, 3000.0, &local),
            task("t4", "reviewer", TaskState::TimedOut, 4000.0, &local),
            task("t5", "linter", TaskState::Completed, 500.0, &local),
        ];
        db.call(|conn| {
            conn.execute(
                "INSERT INTO task_messages (task_id, kind, payload, at) VALUES
                    ('t1', 'clarification_request', 'null', 0),
                    ('t1', 'clarification_request', 'null', 0),
                    ('t2', 'clarification_response', 'null', 0)",
                [],
            )
        })
        .await
        .unwrap();
        for t in &tasks {
            record_from_task(&db, t).await.unwrap();
            // Idempotent
            record_from_task(&db, t).await.unwrap();
        }
        let all = aggregate(&db, "code-review.python", None, 30)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        let reviewer = &all[0];
        assert_eq!(reviewer.provider_id, "reviewer");
        assert_eq!(reviewer.instance_id, local);
        assert_eq!(reviewer.provider_kind, ProviderKind::Agent);
        assert_eq!(reviewer.count, 4);
        assert_eq!(reviewer.success_count, 2);
        assert_eq!(reviewer.failed_count, 1);
        assert_eq!(reviewer.timeout_count, 1);
        assert_eq!(reviewer.success_rate(), Some(0.5));
        assert_eq!(reviewer.p50_ms, Some(2000.0));
        assert_eq!(reviewer.p95_ms, Some(4000.0));
        assert_eq!(reviewer.clarification_rounds_avg, Some(0.5));
        assert_eq!(reviewer.escalation_rate, Some(0.0));
        assert_eq!(reviewer.cost_usd_total, None);
        assert!(reviewer.last_observed_at.is_some());
        assert_eq!(all[1].provider_id, "linter");
        assert_eq!(all[1].count, 1);

        assert!(aggregate(&db, "code-review.python", Some("elsewhere"), 30)
            .await
            .unwrap()
            .is_empty());
        assert!(aggregate(&db, "other", None, 30).await.unwrap().is_empty());
        assert!(!has_requested(&db, "code-review.python").await.unwrap());
    }

    #[tokio::test]
    async fn skips_open_gateway_and_foreign_tasks_and_counts_in_process_once() {
        let db = Db::in_memory();
        let local = network::local_instance_id(&db).unwrap();
        record_from_task(&db, &task("open", "a", TaskState::Working, 1.0, &local))
            .await
            .unwrap();
        record_from_task(
            &db,
            &task("gw", GATEWAY_AGENT_ID, TaskState::Completed, 1.0, &local),
        )
        .await
        .unwrap();
        record_from_task(
            &db,
            &task("foreign", "a", TaskState::Completed, 1.0, "other"),
        )
        .await
        .unwrap();
        assert!(aggregate(&db, "code-review.python", None, 0)
            .await
            .unwrap()
            .is_empty());

        // Both sides here: two rows, one invocation
        let mut both = task("both", "a", TaskState::Completed, 1.0, &local);
        both.requester_instance = local.clone();
        record_from_task(&db, &both).await.unwrap();
        let rows: i64 = db
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM capability_evidence", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert_eq!(rows, 2);
        let all = aggregate(&db, "code-review.python", None, 0).await.unwrap();
        assert_eq!(all[0].count, 1);
        assert!(has_requested(&db, "code-review.python").await.unwrap());
    }

    #[tokio::test]
    async fn provider_kind_comes_from_the_provider_rows() {
        let db = Db::in_memory();
        let local = network::local_instance_id(&db).unwrap();
        let instance = local.clone();
        db.call(move |conn| {
            conn.execute(
                "INSERT INTO capability_providers (capability_id, capability_version, instance_id,
                    provider_kind, provider_id) VALUES ('code-review.python', '1.2.0', ?1, 'workflow', 'flow')",
                [instance],
            )
        })
        .await
        .unwrap();
        record_from_task(&db, &task("w", "flow", TaskState::Completed, 1.0, &local))
            .await
            .unwrap();
        let all = aggregate(&db, "code-review.python", None, 30)
            .await
            .unwrap();
        assert_eq!(all[0].provider_kind, ProviderKind::Workflow);
        assert_eq!(all[0].capability_version.as_deref(), Some("1.2.0"));
    }

    #[tokio::test]
    async fn latency_leaves_out_cancelled_and_reassigned_attempts() {
        let db = Db::in_memory();
        let local = network::local_instance_id(&db).unwrap();
        let tasks = [
            task("t1", "reviewer", TaskState::Completed, 1000.0, &local),
            task("t2", "reviewer", TaskState::Failed, 2000.0, &local),
            task("t3", "reviewer", TaskState::Cancelled, 9000.0, &local),
            task("t4", "reviewer", TaskState::Reassigned, 9000.0, &local),
        ];
        for t in &tasks {
            record_from_task(&db, t).await.unwrap();
        }
        let all = aggregate(&db, "code-review.python", None, 30)
            .await
            .unwrap();
        let reviewer = &all[0];
        assert_eq!(reviewer.count, 4, "every attempt still counts");
        assert_eq!(reviewer.p50_ms, Some(1000.0));
        assert_eq!(reviewer.p95_ms, Some(2000.0));
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[7], 95), Some(7.0));
        let values: Vec<i64> = (1..=100).collect();
        assert_eq!(percentile(&values, 50), Some(50.0));
        assert_eq!(percentile(&values, 95), Some(95.0));
    }
}
