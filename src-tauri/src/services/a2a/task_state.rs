//! The task state machine (wave final, Track B): which transitions are
//! legal (see `task`), and applying one (row update + `task_events` row in
//! one transaction, then `task://state_changed`).
//!
//! - [`apply`]: one transition, refused unless legal
//! - [`advance`]: walks the shortest legal path to a state, e.g. a result
//!   whose acknowledgement never arrived takes the
//!   task `Delivered → Acknowledged → Working → Completed`, each step
//!   recorded. A task already there, or that can't get there (terminal),
//!   is left alone.
//! - [`record_resume`]: the restart-resume event. Self-transitions aren't
//!   legal, so it writes a `task_events` row with `from_state = to_state`
//!   (reason [`RESUMED_REASON`]) without touching the row's state.

use std::collections::{HashMap, VecDeque};

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

use super::task::{
    self, ReasonKind, Refusal, TaskEvent, TaskRecord, TaskStage, TaskState, TaskStateChanged,
    TASK_TIMED_OUT,
};
use crate::db::{now_ms, Db};
use crate::services::capability;
use crate::services::events::{self, EventBus, NandiEvent};

/// The `task_events` reason of a restart-resume row
pub const RESUMED_REASON: &str = "resumed after restart";

pub fn is_legal(from: TaskState, to: TaskState) -> bool {
    use TaskState::*;
    matches!(
        (from, to),
        (Submitted, Delivered | Cancelled | Abandoned)
            | (
                Delivered,
                Acknowledged | Rejected | Cancelled | Abandoned | Reassigned
            )
            | (
                Acknowledged,
                Working | InputRequired | Cancelled | Failed | TimedOut | Reassigned
            )
            | (
                Working,
                Completed | Failed | InputRequired | TimedOut | Cancelled | Reassigned
            )
            | (
                InputRequired,
                Working | Failed | Cancelled | TimedOut | Reassigned
            )
    )
}

/// The shortest legal path from `from` to `to` (excluding `from`); `None`
/// when `to` is unreachable, empty when they're equal.
pub fn path(from: TaskState, to: TaskState) -> Option<Vec<TaskState>> {
    if from == to {
        return Some(Vec::new());
    }
    let mut came_from: HashMap<TaskState, TaskState> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    while let Some(state) = queue.pop_front() {
        for next in TaskState::ALL {
            if next == from || came_from.contains_key(&next) || !is_legal(state, next) {
                continue;
            }
            came_from.insert(next, state);
            if next == to {
                let mut steps = vec![to];
                let mut at = to;
                while let Some(&prev) = came_from.get(&at) {
                    if prev == from {
                        break;
                    }
                    steps.push(prev);
                    at = prev;
                }
                steps.reverse();
                return Some(steps);
            }
            queue.push_back(next);
        }
    }
    None
}

/// What changes along with a transition (on its final step)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub reason: Option<String>,
    pub reason_kind: Option<ReasonKind>,
    pub stage: Option<TaskStage>,
    pub eta_seconds: Option<u32>,
    pub error: Option<String>,
    pub result: Option<Value>,
}

impl Change {
    /// A refusal: its message is the error and reason, with its kind
    pub fn refused(refusal: &Refusal) -> Self {
        Self {
            reason_kind: Some(refusal.kind),
            stage: Some(refusal.stage),
            ..Self::error(refusal.message.clone())
        }
    }

    /// Tags the change with why and where
    pub fn kind(mut self, kind: ReasonKind, stage: TaskStage) -> Self {
        self.reason_kind = Some(kind);
        self.stage = Some(stage);
        self
    }

    /// Tags the change with where only
    pub fn at(mut self, stage: TaskStage) -> Self {
        self.stage = Some(stage);
        self
    }

    pub fn reason(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::default()
        }
    }

    /// The error is also the event's reason
    pub fn error(error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            reason: Some(error.clone()),
            error: Some(error),
            ..Self::default()
        }
    }
}

/// `task://timed_out`: the same payload as `task://state_changed`
#[derive(Serialize)]
#[serde(transparent)]
struct TimedOut<'a>(&'a TaskStateChanged);

impl NandiEvent for TimedOut<'_> {
    const NAME: &'static str = TASK_TIMED_OUT;
}

/// Emits `task://state_changed` (and `task://timed_out`) for each step.
pub fn emit_all(bus: &EventBus, changes: &[TaskStateChanged]) {
    for changed in changes {
        events::emit(bus, changed);
        if changed.event.to_state == TaskState::TimedOut && changed.event.from_state.is_some() {
            events::emit(bus, &TimedOut(changed));
        }
    }
}

/// One step inside the caller's transaction. The inner `Err` is a refused
/// transition (or a missing task); nothing was written.
pub fn apply_conn(
    conn: &Connection,
    task_id: &str,
    to: TaskState,
    change: &Change,
    now: i64,
) -> rusqlite::Result<Result<TaskStateChanged, String>> {
    let Some(mut task) = task::load(conn, task_id)? else {
        return Ok(Err(format!("Task {task_id} not found")));
    };
    let from = task.state;
    if !is_legal(from, to) {
        return Ok(Err(format!(
            "Task {task_id} can't go from {} to {}",
            from.as_str(),
            to.as_str()
        )));
    }
    task.state = to;
    task.updated_at = now as f64;
    if to == TaskState::Acknowledged && task.acknowledged_at.is_none() {
        task.acknowledged_at = Some(now as f64);
    }
    if to.is_terminal() {
        task.completed_at = Some(now as f64);
    }
    if let Some(eta) = change.eta_seconds {
        task.eta_seconds = Some(eta);
    }
    if let Some(error) = &change.error {
        task.error = Some(error.clone());
    }
    if let Some(result) = &change.result {
        task.result = Some(result.clone());
    }
    let updated = conn.execute(
        "UPDATE tasks SET state = ?1, updated_at = ?2, acknowledged_at = ?3, completed_at = ?4,
                eta_seconds = ?5, error = ?6, result = ?7
          WHERE id = ?8 AND state = ?9",
        params![
            to.as_str(),
            now,
            task.acknowledged_at.map(|t| t as i64),
            task.completed_at.map(|t| t as i64),
            task.eta_seconds,
            task.error,
            task.result.as_ref().map(Value::to_string),
            task_id,
            from.as_str(),
        ],
    )?;
    if updated == 0 {
        return Ok(Err(format!("Task {task_id} changed state meanwhile")));
    }
    let event = TaskEvent {
        task_id: task_id.to_string(),
        from_state: Some(from),
        to_state: to,
        reason: change.reason.clone(),
        reason_kind: change.reason_kind,
        stage: change.stage,
        at: now as f64,
    };
    task::insert_event(conn, &event)?;
    Ok(Ok(TaskStateChanged { task, event }))
}

/// One legal transition: the row and its `task_events` row in one
/// transaction, then `task://state_changed`.
pub async fn apply(
    db: &Db,
    bus: &EventBus,
    task_id: &str,
    to: TaskState,
    change: Change,
) -> Result<TaskStateChanged, String> {
    let id = task_id.to_string();
    let changed = db
        .call(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let outcome = apply_conn(&tx, &id, to, &change, now_ms())?;
            if outcome.is_ok() {
                tx.commit()?;
            }
            Ok(outcome)
        })
        .await??;
    emit_all(bus, std::slice::from_ref(&changed));
    capability::evidence::on_task_transitions(db, std::slice::from_ref(&changed));
    Ok(changed)
}

/// Every step to `to` along the shortest legal path, in one transaction.
/// Intermediate steps get the reason "implied by <to>"; `change` goes with
/// the last. `Ok(None)`: already there or unreachable (nothing written).
pub async fn advance(
    db: &Db,
    bus: &EventBus,
    task_id: &str,
    to: TaskState,
    change: Change,
) -> Result<Option<TaskRecord>, String> {
    let id = task_id.to_string();
    let changes = db
        .call(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let Some(task) = task::load(&tx, &id)? else {
                return Ok(Err(format!("Task {id} not found")));
            };
            let steps = match path(task.state, to) {
                Some(steps) if !steps.is_empty() => steps,
                _ => return Ok(Ok(Vec::new())),
            };
            let now = now_ms();
            let implied = Change::reason(format!("implied by {}", to.as_str()));
            let mut changes = Vec::new();
            for (i, step) in steps.iter().enumerate() {
                let change = if i + 1 == steps.len() {
                    &change
                } else {
                    &implied
                };
                match apply_conn(&tx, &id, *step, change, now)? {
                    Ok(changed) => changes.push(changed),
                    Err(e) => return Ok(Err(e)),
                }
            }
            tx.commit()?;
            Ok(Ok(changes))
        })
        .await??;
    emit_all(bus, &changes);
    capability::evidence::on_task_transitions(db, &changes);
    Ok(changes.last().map(|changed| changed.task.clone()))
}

/// The relay gave our request to another provider (`provider_card`): a
/// failover or a reassignment. The row follows it, back to `Submitted`
/// (its acknowledgement and ETA were the last provider's), with an event
/// saying why. A task that ended stays as it is (`Ok(None)`).
pub async fn hand_on(
    db: &Db,
    bus: &EventBus,
    task_id: &str,
    provider_card: &str,
    change: Change,
) -> Result<Option<TaskRecord>, String> {
    let (id, card) = (task_id.to_string(), provider_card.to_string());
    let changed = db
        .call(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let Some(mut task) = task::load(&tx, &id)? else {
                return Ok(Err(format!("Task {id} not found")));
            };
            if task.state.is_terminal() {
                return Ok(Ok(None));
            }
            let from = task.state;
            let now = now_ms();
            let (instance, agent) = card.split_once('/').unwrap_or((card.as_str(), ""));
            task.provider_instance = instance.to_string();
            task.provider_agent = agent.to_string();
            task.state = TaskState::Submitted;
            task.acknowledged_at = None;
            task.eta_seconds = None;
            task.updated_at = now as f64;
            tx.execute(
                "UPDATE tasks SET provider_instance = ?1, provider_agent = ?2, state = ?3,
                        acknowledged_at = NULL, eta_seconds = NULL, updated_at = ?4
                  WHERE id = ?5",
                params![instance, agent, TaskState::Submitted.as_str(), now, id],
            )?;
            let event = TaskEvent {
                task_id: id.clone(),
                from_state: Some(from),
                to_state: TaskState::Submitted,
                reason: change.reason.clone(),
                reason_kind: change.reason_kind,
                stage: change.stage,
                at: now as f64,
            };
            task::insert_event(&tx, &event)?;
            tx.commit()?;
            Ok(Ok(Some(TaskStateChanged { task, event })))
        })
        .await??;
    let Some(changed) = changed else {
        return Ok(None);
    };
    emit_all(bus, std::slice::from_ref(&changed));
    Ok(Some(changed.task))
}

/// The restart-resume `task_events` row (`from_state = to_state`)
pub async fn record_resume(db: &Db, bus: &EventBus, task: &TaskRecord) -> Result<(), String> {
    record_note(db, bus, task, Change::reason(RESUMED_REASON)).await
}

/// A `task_events` row that changes nothing but says why it waits (a hold
/// for approval, a place in line): `from_state = to_state`, with the
/// change's reason, kind and stage
pub async fn note(db: &Db, bus: &EventBus, task_id: &str, change: Change) -> Result<(), String> {
    let Some(task) = task::get_row(db, task_id).await? else {
        return Err(format!("Task {task_id} not found"));
    };
    record_note(db, bus, &task, change).await
}

async fn record_note(
    db: &Db,
    bus: &EventBus,
    task: &TaskRecord,
    change: Change,
) -> Result<(), String> {
    let event = TaskEvent {
        task_id: task.id.clone(),
        from_state: Some(task.state),
        to_state: task.state,
        reason: change.reason,
        reason_kind: change.reason_kind,
        stage: change.stage,
        at: now_ms() as f64,
    };
    let row = event.clone();
    db.call(move |conn| task::insert_event(conn, &row)).await?;
    events::emit(
        bus,
        &TaskStateChanged {
            task: task.clone(),
            event,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::services::a2a::task::tests::{record, store};
    use crate::services::a2a::task::{events_of, get_row};
    use crate::services::events::tests::RecordingSink;

    type Table = &'static [(TaskState, &'static [TaskState])];

    const LEGAL: Table = {
        use TaskState::*;
        &[
            (Submitted, &[Delivered, Cancelled, Abandoned]),
            (
                Delivered,
                &[Acknowledged, Rejected, Cancelled, Abandoned, Reassigned],
            ),
            (
                Acknowledged,
                &[
                    Working,
                    InputRequired,
                    Cancelled,
                    Failed,
                    TimedOut,
                    Reassigned,
                ],
            ),
            (
                Working,
                &[
                    Completed,
                    Failed,
                    InputRequired,
                    TimedOut,
                    Cancelled,
                    Reassigned,
                ],
            ),
            (
                InputRequired,
                &[Working, Failed, Cancelled, TimedOut, Reassigned],
            ),
        ]
    };

    fn expected(from: TaskState, to: TaskState) -> bool {
        LEGAL.iter().any(|(f, tos)| *f == from && tos.contains(&to))
    }

    #[test]
    fn the_table_matches_the_spec() {
        for from in TaskState::ALL {
            for to in TaskState::ALL {
                assert_eq!(is_legal(from, to), expected(from, to), "{from:?} → {to:?}");
            }
            if from.is_terminal() {
                assert!(TaskState::ALL.iter().all(|to| !is_legal(from, *to)));
            }
        }
    }

    #[tokio::test]
    async fn every_pair_applies_only_when_legal() {
        let sink = Arc::new(RecordingSink::default());
        let db = Db::in_memory();
        let bus = EventBus::new(db.clone(), sink.clone());
        for from in TaskState::ALL {
            for to in TaskState::ALL {
                let id = format!("{}-{}", from.as_str(), to.as_str());
                let mut row = record(&id);
                row.state = from;
                store(&db, &row).await;
                let outcome = apply(&db, &bus, &id, to, Change::reason("test")).await;
                let got = get_row(&db, &id).await.unwrap().unwrap();
                let events = events_of(&db, &id).await.unwrap();
                if expected(from, to) {
                    let changed = outcome.unwrap_or_else(|e| panic!("{from:?} → {to:?}: {e}"));
                    assert_eq!(changed.task.state, to);
                    assert_eq!(got.state, to);
                    assert_eq!(events.len(), 1, "one event per transition");
                    assert_eq!(events[0].from_state, Some(from));
                    assert_eq!(events[0].to_state, to);
                    assert_eq!(events[0].reason.as_deref(), Some("test"));
                    assert_eq!(got.acknowledged_at.is_some(), to == TaskState::Acknowledged);
                    assert_eq!(got.completed_at.is_some(), to.is_terminal());
                } else {
                    assert!(outcome.is_err(), "{from:?} → {to:?} must be refused");
                    assert_eq!(got.state, from, "nothing changes");
                    assert!(events.is_empty(), "no event for a refused transition");
                }
            }
        }
        let names = sink.names();
        assert!(names.contains(&task::TASK_STATE_CHANGED));
        assert!(names.contains(&TASK_TIMED_OUT));
    }

    #[tokio::test]
    async fn the_row_and_its_event_commit_together() {
        let db = Db::in_memory();
        let (bus, _) = crate::services::events::tests::bus();
        store(&db, &record("t")).await;
        // A failing statement after the step rolls both back
        let outcome = db
            .call(|conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let step =
                    apply_conn(&tx, "t", TaskState::Delivered, &Change::default(), now_ms())?;
                assert!(step.is_ok());
                tx.execute("INSERT INTO no_such_table VALUES (1)", [])?;
                tx.commit()
            })
            .await;
        assert!(outcome.is_err());
        assert_eq!(
            get_row(&db, "t").await.unwrap().unwrap().state,
            TaskState::Submitted
        );
        assert!(events_of(&db, "t").await.unwrap().is_empty());

        apply(&db, &bus, "t", TaskState::Delivered, Change::default())
            .await
            .unwrap();
        assert_eq!(events_of(&db, "t").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn advance_walks_the_shortest_path_in_one_transaction() {
        let (bus, sink) = crate::services::events::tests::bus();
        let db = bus.db.clone();
        store(&db, &record("t")).await;
        let done = advance(
            &db,
            &bus,
            "t",
            TaskState::Completed,
            Change {
                result: Some(serde_json::json!("ok")),
                ..Change::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(done.state, TaskState::Completed);
        assert_eq!(done.result, Some(serde_json::json!("ok")));
        assert!(done.acknowledged_at.is_some());
        let events = events_of(&db, "t").await.unwrap();
        let path: Vec<_> = events.iter().map(|e| e.to_state).collect();
        assert_eq!(
            path,
            [
                TaskState::Delivered,
                TaskState::Acknowledged,
                TaskState::Working,
                TaskState::Completed
            ]
        );
        assert_eq!(sink.names().len(), 4);
        // Terminal: nothing more
        assert!(
            advance(&db, &bus, "t", TaskState::Failed, Change::default())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(events_of(&db, "t").await.unwrap().len(), 4);
        assert_eq!(
            path_len(TaskState::Delivered, TaskState::Failed),
            Some(2),
            "via Acknowledged"
        );
        assert_eq!(path_len(TaskState::Working, TaskState::Acknowledged), None);
        assert_eq!(path_len(TaskState::Working, TaskState::Working), Some(0));
    }

    fn path_len(from: TaskState, to: TaskState) -> Option<usize> {
        path(from, to).map(|p| p.len())
    }
}
