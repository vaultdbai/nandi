//! Work: long-running work in flight — workflow runs
//! and delegated tasks — as active / waiting / done items with their next
//! action. Reads `workflow_runs` and `tasks`; pausing and cancelling go
//! through the workflow engine.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::{RunStatus, RunSummary, Timestamp};
use crate::services::a2a::task::{self, ReasonKind, TaskFilter, TaskRecord, TaskState};
use crate::services::events;
use crate::services::events::NandiEvent;
use crate::services::requests::local;
use crate::services::triggers::schema::TriggerFire;
use crate::services::workflows::{
    WorkflowRun, WorkflowRunStatus, WorkflowStepRun, WorkflowStepStatus,
};
use crate::services::{AppState, WeakAppState};

pub type WorkError = String;

/// `work://item_updated`
pub const WORK_ITEM_UPDATED: &str = "work://item_updated";
/// `work://item_added`
pub const WORK_ITEM_ADDED: &str = "work://item_added";
/// `work://item_removed`
pub const WORK_ITEM_REMOVED: &str = "work://item_removed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WorkKind {
    Workflow,
    Task,
    Trigger,
    ManualStep,
    /// A local ask carried out as a Request (task 14)
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WorkState {
    Active,
    Waiting,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NextActionKind {
    Approval,
    ManualStep,
    Auto,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkProgress {
    pub completed_steps: u32,
    pub total_steps: u32,
    /// 0–100
    pub percent: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NextAction {
    /// English, with IDs: for Details only. The UI writes the message from
    /// `reason_kind` and `target` (task 5).
    pub description: String,
    pub kind: NextActionKind,
    /// Epoch ms
    pub deadline: Option<Timestamp>,
    /// Why it waits (work-model §2.3), when it waits on someone
    #[serde(default)]
    pub reason_kind: Option<ReasonKind>,
    /// What the fix acts on
    #[serde(default)]
    pub target: NextActionTarget,
}

/// The IDs a next action's fix needs (approve a step, open a run)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NextActionTarget {
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    /// Workflow run ID or task ID
    pub id: String,
    pub kind: WorkKind,
    pub title: String,
    pub state: WorkState,
    /// A task's provider: their name when this Nandi knows them, else
    /// their instance ID
    pub owner: Option<String>,
    pub progress: WorkProgress,
    pub next_action: Option<NextAction>,
    /// The workflow (for workflow runs)
    pub workflow_id: Option<String>,
    /// 1–5, once rated (delegated tasks)
    pub rating: Option<u32>,
    /// Epoch ms
    pub started_at: Timestamp,
    pub updated_at: Timestamp,
}

/// `work://item_updated` / `work://item_added`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemUpdated {
    pub item: WorkItem,
}

impl NandiEvent for WorkItemUpdated {
    const NAME: &'static str = WORK_ITEM_UPDATED;
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemAdded {
    pub item: WorkItem,
}

impl NandiEvent for WorkItemAdded {
    const NAME: &'static str = WORK_ITEM_ADDED;
}

/// `work://item_removed`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemRemoved {
    pub id: String,
}

impl NandiEvent for WorkItemRemoved {
    const NAME: &'static str = WORK_ITEM_REMOVED;
}

/// How far back done work is listed
const DONE_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// Trigger firings considered
const FIRES_LISTED: u32 = 100;

/// Agent runs started by trigger firings since `since`, as work items
async fn trigger_items(state: &AppState, since: i64) -> Vec<WorkItem> {
    state
        .db()
        .call(move |conn| {
            let fires =
                crate::services::triggers::store::agent_fires_since(conn, since, FIRES_LISTED)?;
            let mut items = Vec::new();
            for (fire, name) in fires {
                let Some(run_id) = fire.run_id.as_deref() else {
                    continue;
                };
                if let Some(run) = crate::db::get_run(conn, run_id)? {
                    items.push(from_trigger_fire(&fire, &name, &run));
                }
            }
            Ok(items)
        })
        .await
        .unwrap_or_else(|e| {
            log::warn!("Work: trigger firings not listed: {e}");
            Vec::new()
        })
}

/// A trigger firing that started agent run `run`: one work item named
/// after the trigger, in the run's state, opening that run
pub fn from_trigger_fire(fire: &TriggerFire, trigger_name: &str, run: &RunSummary) -> WorkItem {
    let state = match run.status {
        RunStatus::Queued | RunStatus::Running | RunStatus::Compacted => WorkState::Active,
        RunStatus::AwaitingApproval => WorkState::Waiting,
        RunStatus::Completed => WorkState::Done,
        RunStatus::Failed => WorkState::Failed,
        RunStatus::Cancelled => WorkState::Cancelled,
    };
    let finished = matches!(
        state,
        WorkState::Done | WorkState::Failed | WorkState::Cancelled
    );
    WorkItem {
        id: run.id.clone(),
        kind: WorkKind::Trigger,
        title: trigger_name.to_string(),
        state,
        owner: None,
        progress: WorkProgress {
            completed_steps: u32::from(finished),
            total_steps: 1,
            percent: if finished { 100 } else { 0 },
        },
        next_action: (run.status == RunStatus::AwaitingApproval).then(|| NextAction {
            description: format!("Approve a tool call for {}", run.agent_id),
            kind: NextActionKind::Approval,
            deadline: None,
            reason_kind: Some(ReasonKind::NeedsMyApproval),
            target: NextActionTarget {
                run_id: Some(run.id.clone()),
                agent_id: Some(run.agent_id.clone()),
                ..NextActionTarget::default()
            },
        }),
        workflow_id: None,
        rating: None,
        started_at: fire.fired_at,
        updated_at: run.ended_at.unwrap_or(run.started_at).max(fire.fired_at),
    }
}
/// Delegated tasks considered
const TASKS_LISTED: u32 = 100;

/// Work in flight: workflow runs and the tasks this instance delegated.
/// Holds the state weakly: the state owns it.
pub struct WorkService {
    state: WeakAppState,
}

impl WorkService {
    pub fn new(state: WeakAppState) -> Self {
        Self { state }
    }

    fn state(&self) -> Result<AppState, WorkError> {
        self.state
            .upgrade()
            .ok_or_else(|| "The app is shutting down".to_string())
    }

    /// Everything in flight plus what ended in the last day, newest first
    pub async fn list(&self, filter: Option<WorkState>) -> Result<Vec<WorkItem>, WorkError> {
        let state = self.state()?;
        let since = crate::db::now_ms() - DONE_WINDOW_MS;
        let runs = state.workflows().runs_in_flight_or_since(since).await?;
        let mut items: Vec<WorkItem> = runs
            .iter()
            .map(|run| from_run(run, &workflow_name(&state, &run.workflow_id)))
            .collect();
        // Trigger firings that ran an agent (a workflow firing is its run)
        items.extend(trigger_items(&state, since).await);
        let tasks = task::list_tasks(
            &state,
            TaskFilter {
                state: None,
                role: Some("requester".to_string()),
                agent_id: None,
                limit: TASKS_LISTED,
            },
        )
        .await
        .unwrap_or_else(|e| {
            log::warn!("Work: delegated tasks not listed: {e}");
            Vec::new()
        });
        items.extend(
            tasks
                .iter()
                // Gateway sub-tasks show under their parent
                .filter(|t| t.parent_task_id.is_none())
                .filter(|t| !t.state.is_terminal() || t.updated_at as i64 >= since)
                .map(from_task),
        );
        // Local asks are Requests; what their steps started shows in them
        let (requests, linked) = state
            .db()
            .call(move |conn| Ok((local::recent(conn, since)?, local::linked_ids(conn)?)))
            .await
            .unwrap_or_else(|e| {
                log::warn!("Work: requests not listed: {e}");
                Default::default()
            });
        items.retain(|item| !linked.contains(&item.id));
        items.extend(requests.iter().map(from_request));
        if let Some(filter) = filter {
            items.retain(|item| item.state == filter);
        }
        let rated = ratings(&state).await;
        let names = member_names(&state);
        for item in items.iter_mut().filter(|i| i.kind == WorkKind::Task) {
            item.rating = rated.get(&item.id).copied();
            // The person, not their instance ID
            if let Some(name) = item.owner.as_ref().and_then(|id| names.get(id)) {
                item.owner = Some(name.clone());
            }
        }
        items.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
        Ok(items)
    }

    pub async fn get(&self, id: &str) -> Result<WorkItem, WorkError> {
        let state = self.state()?;
        if let Some(run) = state.workflows().run(id).await? {
            return Ok(from_run(&run, &workflow_name(&state, &run.workflow_id)));
        }
        let mut item = task::get_task(&state, id)
            .await
            .map(|task| from_task(&task))
            .map_err(|_| format!("No work item '{id}'"))?;
        item.rating = ratings(&state).await.get(id).copied();
        Ok(item)
    }

    /// Rates a finished delegated task 1–5 (its requester evidence)
    pub async fn rate(&self, id: &str, rating: u32) -> Result<(), WorkError> {
        if !(1..=5).contains(&rating) {
            return Err("Ratings go from 1 to 5".to_string());
        }
        let state = self.state()?;
        let task_id = id.to_string();
        let rated = state
            .db()
            .call(move |conn| {
                conn.execute(
                    "UPDATE capability_evidence SET user_rating = ?1
                     WHERE task_id = ?2 AND role = 'requester'",
                    rusqlite::params![rating, task_id],
                )
            })
            .await?;
        if rated == 0 {
            return Err("Only finished delegated tasks can be rated".to_string());
        }
        if let Ok(item) = self.get(id).await {
            events::emit(state.bus(), &WorkItemUpdated { item });
        }
        Ok(())
    }

    pub async fn pause(&self, id: &str) -> Result<(), WorkError> {
        let state = self.state()?;
        self.require_run(&state, id, "paused").await?;
        let run = state.workflows().pause_run(id).await?;
        self.updated(&state, &run);
        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<(), WorkError> {
        let state = self.state()?;
        self.require_run(&state, id, "resumed").await?;
        let run = state.workflows().resume_run(id).await?;
        self.updated(&state, &run);
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<(), WorkError> {
        let state = self.state()?;
        if state.workflows().run(id).await?.is_some() {
            let run = state.workflows().cancel_run(id).await?;
            self.updated(&state, &run);
            return Ok(());
        }
        let task = task::cancel_task(&state, id, Some("Cancelled from Work".to_string())).await?;
        events::emit(
            state.bus(),
            &WorkItemUpdated {
                item: from_task(&task),
            },
        );
        Ok(())
    }

    /// Only workflow runs pause and resume
    async fn require_run(&self, state: &AppState, id: &str, verb: &str) -> Result<(), WorkError> {
        if state.workflows().run(id).await?.is_some() {
            return Ok(());
        }
        if task::get_task(state, id).await.is_ok() {
            return Err(format!(
                "Delegated tasks can't be {verb}; cancel it instead"
            ));
        }
        Err(format!("No work item '{id}'"))
    }

    fn updated(&self, state: &AppState, run: &WorkflowRun) {
        let item = from_run(run, &workflow_name(state, &run.workflow_id));
        events::emit(state.bus(), &WorkItemUpdated { item });
    }
}

/// Requester ratings by task ID
async fn ratings(state: &AppState) -> std::collections::HashMap<String, u32> {
    state
        .db()
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT task_id, user_rating FROM capability_evidence
                 WHERE role = 'requester' AND user_rating IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?;
            rows.collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()
        })
        .await
        .unwrap_or_default()
}

fn workflow_name(state: &AppState, workflow_id: &str) -> String {
    state
        .store()
        .read(|repo| repo.workflow(workflow_id).map(|w| w.name().to_string()))
        .unwrap_or_else(|| workflow_id.to_string())
}

/// Settled steps: done, or failed with `on_failure: continue`
fn settled(run: &WorkflowRun) -> u32 {
    run.steps
        .iter()
        .filter(|s| {
            s.status == WorkflowStepStatus::Done
                || (s.status == WorkflowStepStatus::Failed && s.continue_on_failure)
        })
        .count() as u32
}

fn progress(completed: u32, total: u32) -> WorkProgress {
    WorkProgress {
        completed_steps: completed,
        total_steps: total,
        percent: (completed * 100).checked_div(total).unwrap_or(0).min(100),
    }
}

/// A workflow run as a work item
pub fn from_run(run: &WorkflowRun, name: &str) -> WorkItem {
    let total = run.steps.len() as u32;
    let state = match run.status {
        WorkflowRunStatus::Running => WorkState::Active,
        WorkflowRunStatus::AwaitingApproval
        | WorkflowRunStatus::Paused
        | WorkflowRunStatus::Waiting => WorkState::Waiting,
        WorkflowRunStatus::Completed => WorkState::Done,
        WorkflowRunStatus::Failed => WorkState::Failed,
        WorkflowRunStatus::Cancelled => WorkState::Cancelled,
    };
    let next_action = match run.status {
        WorkflowRunStatus::Waiting => run
            .steps
            .iter()
            .find(|s| s.status == WorkflowStepStatus::AwaitingInput)
            .map(|s| NextAction {
                description: s
                    .instructions
                    .clone()
                    .unwrap_or_else(|| format!("Complete step '{}'", s.step_id)),
                kind: NextActionKind::ManualStep,
                deadline: s.deadline,
                reason_kind: None,
                target: step_target(run, s),
            }),
        WorkflowRunStatus::AwaitingApproval => run
            .steps
            .iter()
            .find(|s| s.status == WorkflowStepStatus::AwaitingApproval)
            .map(|s| NextAction {
                description: format!("Approve step '{}' for {}", s.step_id, s.agent_id),
                kind: NextActionKind::Approval,
                deadline: None,
                reason_kind: Some(ReasonKind::NeedsMyApproval),
                target: step_target(run, s),
            }),
        WorkflowRunStatus::Paused => Some(NextAction {
            description: "Paused: resume when you're ready".to_string(),
            kind: NextActionKind::ManualStep,
            deadline: None,
            reason_kind: None,
            target: NextActionTarget {
                run_id: Some(run.id.clone()),
                ..NextActionTarget::default()
            },
        }),
        WorkflowRunStatus::Running => run
            .steps
            .iter()
            .find(|s| {
                matches!(
                    s.status,
                    WorkflowStepStatus::Running | WorkflowStepStatus::Pending
                )
            })
            .map(|s| NextAction {
                description: if s.status == WorkflowStepStatus::Running {
                    format!("{} is working on '{}'", s.agent_id, s.step_id)
                } else {
                    format!("Next: '{}' on {}", s.step_id, s.agent_id)
                },
                kind: NextActionKind::Auto,
                deadline: None,
                reason_kind: None,
                target: step_target(run, s),
            }),
        _ => None,
    };
    let updated_at = run
        .steps
        .iter()
        .flat_map(|s| [s.started_at, s.ended_at])
        .flatten()
        .chain(run.ended_at)
        .fold(run.started_at, f64::max);
    WorkItem {
        id: run.id.clone(),
        kind: WorkKind::Workflow,
        title: name.to_string(),
        state,
        owner: None,
        progress: progress(settled(run), total),
        next_action,
        workflow_id: Some(run.workflow_id.clone()),
        rating: None,
        started_at: run.started_at,
        updated_at,
    }
}

/// The IDs a workflow step's fix needs
fn step_target(run: &WorkflowRun, step: &WorkflowStepRun) -> NextActionTarget {
    NextActionTarget {
        run_id: Some(run.id.clone()),
        step_id: Some(step.step_id.clone()),
        agent_id: Some(step.agent_id.clone()).filter(|a| !a.is_empty()),
        task_id: None,
    }
}

/// A local Request as a work item: its plan's progress, opening its page
pub fn from_request(request: &local::LocalRequest) -> WorkItem {
    use crate::services::requests::{RequestStatus, StepStatus};
    let state = match request.status {
        RequestStatus::Planning | RequestStatus::Running => WorkState::Active,
        RequestStatus::Waiting => WorkState::Waiting,
        RequestStatus::Done => WorkState::Done,
        RequestStatus::Failed => WorkState::Failed,
        RequestStatus::Cancelled => WorkState::Cancelled,
    };
    let done = request
        .steps
        .iter()
        .filter(|s| matches!(s.status, StepStatus::Done | StepStatus::Skipped))
        .count() as u32;
    WorkItem {
        id: request.id.clone(),
        kind: WorkKind::Request,
        title: request.title.clone(),
        state,
        owner: None,
        progress: progress(done, request.steps.len() as u32),
        next_action: None,
        workflow_id: None,
        rating: None,
        started_at: crate::db::ts(request.created_at),
        updated_at: crate::db::ts(request.updated_at),
    }
}

/// A delegated task as a work item
pub fn from_task(task: &TaskRecord) -> WorkItem {
    let state = match task.state {
        TaskState::Submitted
        | TaskState::Delivered
        | TaskState::Acknowledged
        | TaskState::Working => WorkState::Active,
        TaskState::InputRequired => WorkState::Waiting,
        TaskState::Completed => WorkState::Done,
        // Handed on: the next attempt is its own item
        TaskState::Cancelled | TaskState::Reassigned => WorkState::Cancelled,
        TaskState::Failed | TaskState::Rejected | TaskState::TimedOut | TaskState::Abandoned => {
            WorkState::Failed
        }
    };
    let target = NextActionTarget {
        task_id: Some(task.id.clone()),
        ..NextActionTarget::default()
    };
    let next_action = match task.state {
        TaskState::InputRequired => Some(NextAction {
            description: "Answer the provider's question".to_string(),
            kind: NextActionKind::ManualStep,
            deadline: task.deadline,
            reason_kind: Some(ReasonKind::NeedsMyAnswer),
            target,
        }),
        state if !state.is_terminal() => Some(NextAction {
            description: format!("Waiting on {}", task.provider_instance),
            kind: NextActionKind::Auto,
            deadline: task.deadline,
            reason_kind: None,
            target,
        }),
        _ => None,
    };
    WorkItem {
        id: task.id.clone(),
        kind: WorkKind::Task,
        title: task_title(task),
        state,
        owner: Some(task.provider_instance.clone()),
        progress: progress(u32::from(task.state == TaskState::Completed), 1),
        next_action,
        workflow_id: None,
        rating: None,
        started_at: task.created_at,
        updated_at: task.updated_at,
    }
}

/// Instance ID → display name, for the people this Nandi knows
fn member_names(state: &AppState) -> std::collections::HashMap<String, String> {
    crate::services::network::known_members(state.db())
        .into_iter()
        .filter(|m| !m.display_name.trim().is_empty())
        .map(|m| (m.instance_id, m.display_name))
        .collect()
}

/// Longest task title, in characters
const TASK_TITLE_CHARS: usize = 120;

/// What was asked ("Rotate the CI deploy keys…"), else the capability
fn task_title(task: &TaskRecord) -> String {
    let asked = crate::services::a2a::returns::text_of(&task.input);
    let asked = asked.split_whitespace().collect::<Vec<_>>().join(" ");
    if asked.is_empty() || task.input.is_null() {
        return task.capability.clone();
    }
    if asked.chars().count() <= TASK_TITLE_CHARS {
        return asked;
    }
    let mut short: String = asked.chars().take(TASK_TITLE_CHARS - 1).collect();
    short.push('…');
    short
}

pub async fn list_work(
    state: &AppState,
    state_filter: Option<WorkState>,
) -> Result<Vec<WorkItem>, WorkError> {
    state.work().list(state_filter).await
}

/// `work_preferences` key prefix of a dismissed failure (value: epoch ms)
const DISMISSED_PREFIX: &str = "dismissed:";
/// Dismissals older than the done window can't matter any more
const DISMISSED_KEPT_MS: i64 = 7 * DONE_WINDOW_MS;

/// Hides a failed item from the Inbox; it stays in Recent and history
pub async fn dismiss_work(state: &AppState, id: String) -> Result<(), WorkError> {
    if id.trim().is_empty() {
        return Err("Nothing to dismiss".to_string());
    }
    let now = crate::db::now_ms();
    let key = format!("{DISMISSED_PREFIX}{id}");
    state
        .db()
        .call(move |conn| {
            conn.execute(
                "INSERT INTO work_preferences (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, now.to_string()],
            )?;
            // Keep the table small
            conn.execute(
                "DELETE FROM work_preferences
                  WHERE key LIKE 'dismissed:%' AND CAST(value AS INTEGER) < ?1",
                [now - DISMISSED_KEPT_MS],
            )?;
            Ok(())
        })
        .await?;
    crate::services::inbox::audit::record(
        state,
        crate::services::inbox::audit::Decided::new("failed", id, "dismissed"),
    );
    Ok(())
}

/// IDs of dismissed failures
pub async fn list_dismissed_work(state: &AppState) -> Result<Vec<String>, WorkError> {
    state
        .db()
        .call(|conn| {
            let mut stmt =
                conn.prepare("SELECT key FROM work_preferences WHERE key LIKE 'dismissed:%'")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|key| key.map(|k| k[DISMISSED_PREFIX.len()..].to_string()))
                .collect()
        })
        .await
}

pub async fn get_work_item(state: &AppState, id: String) -> Result<WorkItem, WorkError> {
    state.work().get(&id).await
}

pub async fn pause_work(state: &AppState, id: String) -> Result<(), WorkError> {
    state.work().pause(&id).await
}

pub async fn resume_work(state: &AppState, id: String) -> Result<(), WorkError> {
    state.work().resume(&id).await
}

pub async fn rate_work(state: &AppState, id: String, rating: u32) -> Result<(), WorkError> {
    state.work().rate(&id, rating).await
}

pub async fn cancel_work(state: &AppState, id: String) -> Result<(), WorkError> {
    state.work().cancel(&id).await
}

#[cfg(test)]
mod tests;
