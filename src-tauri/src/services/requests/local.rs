//! Requests, phase 3 (task 14, work-model §4): a local ask's plan saved as
//! a Request with linked Steps (`requests`, `request_steps`) and carried
//! out here rather than in the webview. Steps start in order (each waits
//! for the ones it depends on), link the run, workflow run or task that
//! does them, and pick up again after a restart. Run data, never the gold
//! copy.
//!
//! The bookkeeping ([`new_request`], [`ready_steps`], [`settle`],
//! [`workflow_yaml`]) is pure; [`tick`] does the I/O.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::db;
use crate::model::RunStatus;
use crate::services::a2a::task::{self, TaskState};
use crate::services::ask::{ActionPlan, PlanStepKind};
use crate::services::events::{self, NandiEvent};
use crate::services::run_plan::{self, RunPlanStep, RunStepKind};
use crate::services::workflows::WorkflowRunStatus;
use crate::services::AppState;

use super::self_step;
use super::{
    humanize, Blocker, BlockerKind, BlockerWho, Direction, Request, RequestStatus, RequestStep,
    StepKind, StepLink, StepStatus,
};

/// `request://updated`: a local Request or one of its Steps changed
pub const REQUEST_UPDATED: &str = "request://updated";

/// How often a running Request looks at what its steps started
const POLL: Duration = Duration::from_secs(1);

/// Longest title, in characters
const MAX_TITLE_CHARS: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestUpdated {
    pub id: String,
}

impl NandiEvent for RequestUpdated {
    const NAME: &'static str = REQUEST_UPDATED;
}

/// One step of a local Request, as stored
#[derive(Debug, Clone, PartialEq)]
pub struct LocalStep {
    /// 0-based, the plan's order
    pub position: u32,
    pub kind: PlanStepKind,
    /// Agent, capability or workflow ID, or a member's instance
    pub target: String,
    pub label: String,
    /// Positions that must be done first
    pub depends_on: Vec<u32>,
    pub status: StepStatus,
    pub run_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub task_id: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

/// Who started a local Request
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RequestSource {
    /// The user, from the ask box or Studio
    #[default]
    Me,
    /// The Chief of Staff, from new mail
    Email,
    /// The Chief of Staff, from a calendar change
    Calendar,
    /// The Chief of Staff, in a chat
    ChiefOfStaff,
    /// A teammate's request to one of the user's agents (intake only)
    Teammate,
}

/// A local Request (an ask's plan), as stored
#[derive(Debug, Clone, PartialEq)]
pub struct LocalRequest {
    pub id: String,
    pub title: String,
    pub source: RequestSource,
    /// What was asked, word for word: each step's input
    pub ask_text: String,
    pub plan: ActionPlan,
    pub status: RequestStatus,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub ended_at: Option<i64>,
    pub steps: Vec<LocalStep>,
}

// ---------------------------------------------------------------------------
// Bookkeeping (pure)
// ---------------------------------------------------------------------------

/// The first non-empty line of the ask, shortened
pub fn title_for(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > MAX_TITLE_CHARS {
        let cut: String = line.chars().take(MAX_TITLE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_string()
    }
}

/// A new Request for `plan`: its steps pending, each after the one before
pub fn new_request(
    id: &str,
    text: &str,
    plan: &ActionPlan,
    source: RequestSource,
    now: i64,
) -> LocalRequest {
    let steps = plan
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| LocalStep {
            position: i as u32,
            kind: step.kind,
            target: step.target.clone(),
            label: if step.description.trim().is_empty() {
                humanize(&step.target)
            } else {
                step.description.trim().to_string()
            },
            depends_on: if i == 0 {
                Vec::new()
            } else {
                vec![i as u32 - 1]
            },
            status: StepStatus::Pending,
            run_id: None,
            workflow_run_id: None,
            task_id: None,
            error: None,
            started_at: None,
            ended_at: None,
        })
        .collect();
    LocalRequest {
        id: id.to_string(),
        title: title_for(text),
        source,
        ask_text: text.trim().to_string(),
        plan: plan.clone(),
        status: RequestStatus::Running,
        error: None,
        created_at: now,
        updated_at: now,
        ended_at: None,
        steps,
    }
}

/// Pending steps whose dependencies are all done, in order
pub fn ready_steps(steps: &[LocalStep]) -> Vec<u32> {
    steps
        .iter()
        .filter(|s| s.status == StepStatus::Pending)
        .filter(|s| {
            s.depends_on.iter().all(|d| {
                steps
                    .iter()
                    .find(|o| o.position == *d)
                    .is_some_and(|o| o.status == StepStatus::Done)
            })
        })
        .map(|s| s.position)
        .collect()
}

/// The Request's status from its steps. A failed step fails it, and the
/// steps still pending are skipped.
pub fn settle(request: &mut LocalRequest, now: i64) {
    if matches!(
        request.status,
        RequestStatus::Done | RequestStatus::Failed | RequestStatus::Cancelled
    ) {
        return;
    }
    let failed = request
        .steps
        .iter()
        .find(|s| s.status == StepStatus::Failed)
        .map(|s| {
            s.error
                .clone()
                .unwrap_or_else(|| format!("{} failed", s.label))
        });
    if let Some(error) = failed {
        for step in &mut request.steps {
            if step.status == StepStatus::Pending {
                step.status = StepStatus::Skipped;
            }
        }
        request.status = RequestStatus::Failed;
        request.error = Some(error);
        request.ended_at = Some(now);
        return;
    }
    let all_settled = request
        .steps
        .iter()
        .all(|s| matches!(s.status, StepStatus::Done | StepStatus::Skipped));
    request.status = if all_settled {
        request.ended_at = Some(now);
        RequestStatus::Done
    } else if request
        .steps
        .iter()
        .any(|s| s.status == StepStatus::Waiting)
    {
        RequestStatus::Waiting
    } else {
        RequestStatus::Running
    };
}

fn is_terminal(status: RequestStatus) -> bool {
    matches!(
        status,
        RequestStatus::Done | RequestStatus::Failed | RequestStatus::Cancelled
    )
}

/// A workflow ID from a title: letters, digits and hyphens
pub(crate) fn workflow_id_for(title: &str) -> String {
    let mut id = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let id: String = id.trim_matches('-').chars().take(60).collect();
    let id = id.trim_end_matches('-').to_string();
    if id.is_empty() {
        "saved-request".to_string()
    } else {
        id
    }
}

/// YAML for a double-quoted scalar
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// Part of an agent step's run plan, for the workflow draft
#[derive(Debug, PartialEq)]
enum RunPart {
    /// Steps the agent does, by title
    Agent(Vec<String>),
    /// A hand-over: the user's part
    Handover(String),
}

/// A run plan cut at its hand-overs. Pure.
fn run_parts(steps: &[RunPlanStep]) -> Vec<RunPart> {
    let mut parts = Vec::new();
    for step in steps {
        if step.kind == RunStepKind::Handover {
            parts.push(RunPart::Handover(step.title.clone()));
            continue;
        }
        match parts.last_mut() {
            Some(RunPart::Agent(titles)) => titles.push(step.title.clone()),
            _ => parts.push(RunPart::Agent(vec![step.title.clone()])),
        }
    }
    parts
}

fn agent_yaml(target: &str, input: &str) -> String {
    format!("    agent: {target}\n    input: {}\n", quoted(input))
}

fn manual_yaml(instructions: &str) -> String {
    format!(
        "    step_type: manual\n    instructions: {}\n",
        quoted(instructions)
    )
}

/// A workflow drafted from a Request's plan (Save as workflow); its steps
/// run in order, like the plan's. Agent steps stay agent steps, each given
/// the step and the ask; delegating, running another workflow or asking
/// someone become manual steps, which say what to do. When an agent step's
/// run plan (`run_plans`, by step position) handed something over (signing
/// in, paying), the step is cut there: the agent's parts stay agent steps
/// and each hand-over becomes a manual step for the user. The user confirms
/// it and the domain validates it. Pure.
pub fn workflow_yaml(request: &LocalRequest, run_plans: &HashMap<u32, Vec<RunPlanStep>>) -> String {
    let id = workflow_id_for(&request.title);
    let mut body = Vec::new();
    for step in &request.steps {
        match step.kind {
            PlanStepKind::LocalAgent => {
                let parts = run_plans
                    .get(&step.position)
                    .map(|steps| run_parts(steps))
                    .unwrap_or_default();
                if !parts.iter().any(|p| matches!(p, RunPart::Handover(_))) {
                    let input = format!("{}\n\nRequest: {}", step.label, request.ask_text);
                    body.push(agent_yaml(&step.target, &input));
                    continue;
                }
                for part in parts {
                    body.push(match part {
                        RunPart::Agent(titles) => {
                            let input = format!(
                                "{}: {}\n\nRequest: {}",
                                step.label,
                                titles.join("; "),
                                request.ask_text
                            );
                            agent_yaml(&step.target, &input)
                        }
                        RunPart::Handover(title) => {
                            manual_yaml(&format!("Your part of \"{}\": {title}", step.label))
                        }
                    });
                }
            }
            kind => {
                let instructions = match kind {
                    PlanStepKind::Capability => {
                        format!("Delegate \"{}\": {}", step.target, step.label)
                    }
                    PlanStepKind::Workflow => {
                        format!("Run the workflow \"{}\": {}", step.target, step.label)
                    }
                    PlanStepKind::Human => format!("Ask {}: {}", step.target, step.label),
                    _ => step.label.clone(),
                };
                body.push(manual_yaml(&instructions));
            }
        }
    }
    let mut out = format!("id: {id}\nname: {}\nsteps:\n", quoted(&request.title));
    for (i, step) in body.iter().enumerate() {
        out.push_str(&format!("  - id: step-{}\n{step}", i + 1));
    }
    out
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

fn enum_str<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn parse_enum<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    serde_json::from_value(Value::String(text.to_string())).ok()
}

fn step_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalStep> {
    let kind: String = row.get(1)?;
    let status: String = row.get(5)?;
    let depends: String = row.get(4)?;
    Ok(LocalStep {
        position: row.get(0)?,
        kind: parse_enum(&kind).unwrap_or(PlanStepKind::Manual),
        target: row.get(2)?,
        label: row.get(3)?,
        depends_on: serde_json::from_str(&depends).unwrap_or_default(),
        status: parse_enum(&status).unwrap_or(StepStatus::Pending),
        run_id: row.get(6)?,
        workflow_run_id: row.get(7)?,
        task_id: row.get(8)?,
        error: row.get(9)?,
        started_at: row.get(10)?,
        ended_at: row.get(11)?,
    })
}

/// A local Request with its steps, if `id` is one
pub fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<LocalRequest>> {
    let request = conn
        .query_row(
            "SELECT id, title, ask_text, plan, status, error, created_at, updated_at, ended_at,
                    source
               FROM requests WHERE id = ?1",
            [id],
            |row| {
                let plan: String = row.get(3)?;
                let status: String = row.get(4)?;
                let source: String = row.get(9)?;
                Ok(LocalRequest {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    source: parse_enum(&source).unwrap_or_default(),
                    ask_text: row.get(2)?,
                    plan: serde_json::from_str(&plan).unwrap_or_default(),
                    status: parse_enum(&status).unwrap_or(RequestStatus::Failed),
                    error: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    ended_at: row.get(8)?,
                    steps: Vec::new(),
                })
            },
        )
        .optional()?;
    let Some(mut request) = request else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT position, kind, target, label, depends_on, status, run_id, workflow_run_id,
                task_id, error, started_at, ended_at
           FROM request_steps WHERE request_id = ?1 ORDER BY position",
    )?;
    request.steps = stmt
        .query_map([id], step_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(request))
}

/// Writes the Request and every step (insert or replace)
pub fn save(conn: &mut Connection, request: &LocalRequest) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO requests (id, title, source, ask_text, plan, status, error,
                               created_at, updated_at, ended_at)
         VALUES (?1, ?2, ?10, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, status = excluded.status,
             ask_text = excluded.ask_text, error = excluded.error,
             updated_at = excluded.updated_at, ended_at = excluded.ended_at",
        params![
            request.id,
            request.title,
            request.ask_text,
            serde_json::to_string(&request.plan).unwrap_or_default(),
            enum_str(&request.status),
            request.error,
            request.created_at,
            request.updated_at,
            request.ended_at,
            enum_str(&request.source),
        ],
    )?;
    for step in &request.steps {
        tx.execute(
            "INSERT INTO request_steps (request_id, position, kind, target, label, depends_on,
                                        status, run_id, workflow_run_id, task_id, error,
                                        started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(request_id, position) DO UPDATE SET status = excluded.status,
                 run_id = excluded.run_id, workflow_run_id = excluded.workflow_run_id,
                 task_id = excluded.task_id, error = excluded.error,
                 started_at = excluded.started_at, ended_at = excluded.ended_at",
            params![
                request.id,
                step.position,
                enum_str(&step.kind),
                step.target,
                step.label,
                serde_json::to_string(&step.depends_on).unwrap_or_else(|_| "[]".into()),
                enum_str(&step.status),
                step.run_id,
                step.workflow_run_id,
                step.task_id,
                step.error,
                step.started_at,
                step.ended_at,
            ],
        )?;
    }
    tx.commit()
}

/// Requests still in progress (resumed at startup)
pub fn open_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM requests WHERE status IN ('planning', 'running', 'waiting')
          ORDER BY created_at",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

/// Requests open, or changed since `since` (epoch ms), newest first
pub fn recent(conn: &Connection, since: i64) -> rusqlite::Result<Vec<LocalRequest>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM requests
          WHERE status IN ('planning', 'running', 'waiting') OR updated_at >= ?1
          ORDER BY updated_at DESC LIMIT 200",
    )?;
    let ids = stmt
        .query_map([since], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(request) = load(conn, &id)? {
            out.push(request);
        }
    }
    Ok(out)
}

/// Every run, workflow run and task a local Request started (they show as
/// part of it, not on their own)
pub fn linked_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, workflow_run_id, task_id FROM request_steps
          WHERE run_id IS NOT NULL OR workflow_run_id IS NOT NULL OR task_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok([
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ])
    })?;
    let mut ids = HashSet::new();
    for row in rows {
        ids.extend(row?.into_iter().flatten());
    }
    Ok(ids)
}

/// The local Request a run, workflow run or task belongs to
pub fn request_of(conn: &Connection, linked_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT request_id FROM request_steps
          WHERE run_id = ?1 OR workflow_run_id = ?1 OR task_id = ?1 LIMIT 1",
        [linked_id],
        |row| row.get(0),
    )
    .optional()
}

// ---------------------------------------------------------------------------
// Carrying it out
// ---------------------------------------------------------------------------

/// Requests with a runner, so a resume and a new step don't start two
fn runners() -> &'static Mutex<HashSet<String>> {
    static RUNNERS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNERS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// What a started step's run, workflow run or task says now
async fn observe(state: &AppState, step: &LocalStep) -> Option<(StepStatus, Option<String>)> {
    if let Some(run_id) = &step.run_id {
        let mut id = run_id.clone();
        // A compacted run goes on in a child run
        for _ in 0..8 {
            let lookup = id.clone();
            let run = state
                .db()
                .call(move |conn| {
                    let run = db::get_run(conn, &lookup)?;
                    let child: Option<String> = conn
                        .query_row(
                            "SELECT id FROM runs WHERE parent_run_id = ?1
                              ORDER BY started_at DESC LIMIT 1",
                            [&lookup],
                            |row| row.get(0),
                        )
                        .optional()?;
                    Ok((run, child))
                })
                .await
                .ok()?;
            let (Some(run), child) = run else {
                return None;
            };
            return Some(match run.status {
                RunStatus::Compacted => match child {
                    Some(child) => {
                        id = child;
                        continue;
                    }
                    None => (StepStatus::Running, None),
                },
                RunStatus::Queued | RunStatus::Running => (StepStatus::Running, None),
                RunStatus::AwaitingApproval => (StepStatus::Waiting, None),
                RunStatus::Completed => (StepStatus::Done, None),
                RunStatus::Failed => (
                    StepStatus::Failed,
                    Some(
                        run.summary
                            .unwrap_or_else(|| "The agent's run failed".into()),
                    ),
                ),
                RunStatus::Cancelled => (
                    StepStatus::Failed,
                    Some("The agent's run was cancelled".into()),
                ),
            });
        }
        return None;
    }
    if let Some(run_id) = &step.workflow_run_id {
        let run = state.workflows().run(run_id).await.ok()??;
        return Some(match run.status {
            WorkflowRunStatus::Running => (StepStatus::Running, None),
            WorkflowRunStatus::AwaitingApproval
            | WorkflowRunStatus::Waiting
            | WorkflowRunStatus::Paused => (StepStatus::Waiting, None),
            WorkflowRunStatus::Completed => (StepStatus::Done, None),
            WorkflowRunStatus::Failed => (
                StepStatus::Failed,
                Some(run.error.unwrap_or_else(|| "The workflow failed".into())),
            ),
            WorkflowRunStatus::Cancelled => (
                StepStatus::Failed,
                Some("The workflow was cancelled".into()),
            ),
        });
    }
    if let Some(task_id) = &step.task_id {
        let task = task::get_task(state, task_id).await.ok()?;
        return Some(match task.state {
            TaskState::InputRequired => (StepStatus::Waiting, None),
            TaskState::Completed => (StepStatus::Done, None),
            TaskState::Failed
            | TaskState::TimedOut
            | TaskState::Rejected
            | TaskState::Abandoned
            | TaskState::Cancelled => (
                StepStatus::Failed,
                Some(
                    task.error
                        .unwrap_or_else(|| "The delegation didn't finish".into()),
                ),
            ),
            // Handed to another provider: its new attempt is still this step
            _ => (StepStatus::Running, None),
        });
    }
    None
}

/// Starts one step: messages the agent, runs the workflow, or delegates
/// the capability, with the ask as the input. Asking a person and manual
/// steps wait for the user to mark them done.
async fn start(state: &AppState, request: &LocalRequest, step: &mut LocalStep, now: i64) {
    step.started_at = Some(now);
    let started = match step.kind {
        // The Chief of Staff's own step: in the task's thread or the
        // Request's conversation, never its main chat
        PlanStepKind::LocalAgent if self_step::is_self_step(step.kind, &step.target) => {
            self_step::start(state, request, step)
                .await
                .map(|run_id| step.run_id = Some(run_id))
        }
        PlanStepKind::LocalAgent => crate::services::chat::send_message(
            state,
            step.target.clone(),
            request.ask_text.clone(),
            Vec::new(),
            None,
        )
        .await
        .map(|sent| step.run_id = Some(sent.run_id)),
        PlanStepKind::Workflow => {
            crate::services::workflows::run_workflow(state, step.target.clone())
                .await
                .map(|run_id| step.workflow_run_id = Some(run_id))
        }
        PlanStepKind::Capability => crate::services::a2a::delegate_task(
            state,
            &step.target,
            json!({ "text": request.ask_text, "step": step.label }),
            None,
        )
        .await
        .map(|task_id| step.task_id = Some(task_id)),
        PlanStepKind::Human | PlanStepKind::Manual => {
            step.status = StepStatus::Waiting;
            return;
        }
    };
    match started {
        Ok(()) => step.status = StepStatus::Running,
        Err(e) => {
            step.status = StepStatus::Failed;
            step.error = Some(e);
            step.ended_at = Some(now);
        }
    }
}

/// One pass over a Request: follows started steps, starts the ready ones,
/// settles it. Returns whether it has ended.
pub async fn tick(state: &AppState, id: &str) -> Result<bool, String> {
    let lookup = id.to_string();
    let Some(mut request) = state.db().call(move |conn| load(conn, &lookup)).await? else {
        return Ok(true);
    };
    if is_terminal(request.status) {
        return Ok(true);
    }
    let before = request.clone();
    let now = db::now_ms();

    for i in 0..request.steps.len() {
        let step = &request.steps[i];
        let linked =
            step.run_id.is_some() || step.workflow_run_id.is_some() || step.task_id.is_some();
        if !linked || !matches!(step.status, StepStatus::Running | StepStatus::Waiting) {
            continue;
        }
        if let Some((status, error)) = observe(state, step).await {
            let step = &mut request.steps[i];
            if status != step.status {
                step.status = status;
                if matches!(status, StepStatus::Done | StepStatus::Failed) {
                    step.ended_at = Some(now);
                    step.error = error;
                }
            }
        }
    }
    // Start what's ready, until nothing more is (a step that only waits
    // on a person doesn't unblock the next one)
    if !request.steps.iter().any(|s| s.status == StepStatus::Failed) {
        for position in ready_steps(&request.steps) {
            let mut step = request.steps[position as usize].clone();
            start(state, &request, &mut step, now).await;
            request.steps[position as usize] = step;
        }
    }
    settle(&mut request, now);

    if request != before {
        request.updated_at = now;
        let saved = request.clone();
        state.db().call(move |conn| save(conn, &saved)).await?;
        events::emit(state.bus(), &RequestUpdated { id: id.to_string() });
        if request.status == RequestStatus::Done {
            // Recurring work: the Chief of Staff offers to save it
            super::recurrence::on_done(state, &request).await;
        }
    }
    Ok(is_terminal(request.status))
}

/// Carries the Request out in the background until it ends (one runner
/// per Request)
pub fn spawn_runner(state: &AppState, id: String) {
    {
        let mut running = runners().lock().unwrap_or_else(|e| e.into_inner());
        if !running.insert(id.clone()) {
            return;
        }
    }
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else {
                break;
            };
            match tick(&state, &id).await {
                Ok(true) => break,
                Ok(false) => {}
                Err(e) => log::warn!("Request {id}: {e}"),
            }
            drop(state);
            tokio::time::sleep(POLL).await;
        }
        runners()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    });
}

/// Saves an ask's plan as a Request and starts carrying it out
pub async fn start_request(
    state: &AppState,
    plan: ActionPlan,
    text: String,
) -> Result<String, String> {
    start_request_from(state, plan, text, RequestSource::Me).await
}

/// Like [`start_request`], for work the Chief of Staff started itself
pub async fn start_request_from(
    state: &AppState,
    plan: ActionPlan,
    text: String,
    source: RequestSource,
) -> Result<String, String> {
    let id = create_request(state, plan, text, source).await?;
    spawn_runner(state, id.clone());
    Ok(id)
}

/// Saves a plan as a Request without starting it, so the caller can link
/// it first (intake links its thread, where a Chief of Staff step runs);
/// [`spawn_runner`] starts it. Returns its ID.
pub async fn create_request(
    state: &AppState,
    plan: ActionPlan,
    text: String,
    source: RequestSource,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("There's nothing to do: the ask is empty".to_string());
    }
    if plan.rejection.is_some() || plan.steps.is_empty() {
        return Err("This plan has no steps to carry out".to_string());
    }
    self_step::check(&plan)?;
    let id = uuid::Uuid::new_v4().to_string();
    let request = new_request(&id, &text, &plan, source, db::now_ms());
    state.db().call(move |conn| save(conn, &request)).await?;
    events::emit(state.bus(), &RequestUpdated { id: id.clone() });
    Ok(id)
}

/// Marks a step that waits on the user (asking someone, a manual step) done
pub async fn complete_request_step(
    state: &AppState,
    id: String,
    position: u32,
) -> Result<(), String> {
    let lookup = id.clone();
    let mut request = state
        .db()
        .call(move |conn| load(conn, &lookup))
        .await?
        .ok_or_else(|| format!("Request {id} not found"))?;
    let step = request
        .steps
        .iter_mut()
        .find(|s| s.position == position)
        .ok_or("That step isn't part of this request")?;
    if !matches!(step.kind, PlanStepKind::Human | PlanStepKind::Manual)
        || step.status != StepStatus::Waiting
    {
        return Err("Only a step waiting on you can be marked done".to_string());
    }
    let now = db::now_ms();
    step.status = StepStatus::Done;
    step.ended_at = Some(now);
    request.updated_at = now;
    settle(&mut request, now);
    state.db().call(move |conn| save(conn, &request)).await?;
    events::emit(state.bus(), &RequestUpdated { id: id.clone() });
    spawn_runner(state, id);
    Ok(())
}

/// News for a Request, as stored (`request_updates`)
#[derive(Debug, Clone, PartialEq)]
pub struct RequestUpdate {
    pub request_id: String,
    pub at: i64,
    pub source: RequestSource,
    pub text: String,
}

/// The Request's input with `news` added, so the steps still to start get
/// it. Pure, for tests.
pub fn with_update(ask_text: &str, news: &str) -> String {
    format!("{}\n\nUpdate: {}", ask_text.trim_end(), news.trim())
}

pub fn save_update(conn: &Connection, update: &RequestUpdate) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO request_updates (request_id, at, source, text) VALUES (?1, ?2, ?3, ?4)",
        params![
            update.request_id,
            update.at,
            enum_str(&update.source),
            update.text
        ],
    )?;
    Ok(())
}

/// Updates added after `since` (epoch ms), oldest first
pub fn updates_since(conn: &Connection, since: i64) -> rusqlite::Result<Vec<RequestUpdate>> {
    let mut stmt = conn.prepare(
        "SELECT request_id, at, source, text FROM request_updates
          WHERE at > ?1 ORDER BY at LIMIT 200",
    )?;
    let rows = stmt.query_map([since], |row| {
        let source: String = row.get(2)?;
        Ok(RequestUpdate {
            request_id: row.get(0)?,
            at: row.get(1)?,
            source: parse_enum(&source).unwrap_or_default(),
            text: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Adds news to work in progress: the steps still to start get it with
/// their input, and an agent working on it now gets it in its chat.
/// Returns the agents told now.
pub async fn add_update(
    state: &AppState,
    id: String,
    news: String,
    source: RequestSource,
) -> Result<Vec<String>, String> {
    if news.trim().is_empty() {
        return Err("There's no news to add".to_string());
    }
    let lookup = id.clone();
    let mut request = state
        .db()
        .call(move |conn| load(conn, &lookup))
        .await?
        .ok_or_else(|| format!("Request {id} not found"))?;
    if is_terminal(request.status) {
        return Err(format!(
            "\"{}\" has ended; start new work for this instead",
            request.title
        ));
    }
    let now = db::now_ms();
    request.ask_text = with_update(&request.ask_text, &news);
    request.updated_at = now;
    let update = RequestUpdate {
        request_id: id.clone(),
        at: now,
        source,
        text: news.trim().to_string(),
    };
    let saved = request.clone();
    state
        .db()
        .call(move |conn| {
            save(conn, &saved)?;
            save_update(conn, &update)
        })
        .await?;
    let mut told = Vec::new();
    for step in &request.steps {
        if step.kind == PlanStepKind::LocalAgent
            && step.status == StepStatus::Running
            && !told.contains(&step.target)
        {
            let text = format!("Update for \"{}\": {}", request.title, news.trim());
            let sent = if self_step::is_self_step(step.kind, &step.target) {
                self_step::submit(state, &request, text).await.map(|_| ())
            } else {
                crate::services::chat::send_message(
                    state,
                    step.target.clone(),
                    text,
                    Vec::new(),
                    None,
                )
                .await
                .map(|_| ())
            };
            match sent {
                Ok(()) => told.push(step.target.clone()),
                Err(e) => log::warn!("Request {id}: couldn't tell {}: {e}", step.target),
            }
        }
    }
    events::emit(state.bus(), &RequestUpdated { id });
    Ok(told)
}

/// Stops a local Request: what its running steps started is cancelled,
/// and the steps not yet done are skipped
pub async fn cancel_request(state: &AppState, id: String) -> Result<(), String> {
    let lookup = id.clone();
    let mut request = state
        .db()
        .call(move |conn| load(conn, &lookup))
        .await?
        .ok_or_else(|| format!("Request {id} not found"))?;
    if is_terminal(request.status) {
        return Err("This request has already ended".to_string());
    }
    for step in request
        .steps
        .iter()
        .filter(|s| matches!(s.status, StepStatus::Running | StepStatus::Waiting))
    {
        // Best effort: the Request ends either way
        let stopped = if let Some(run_id) = &step.run_id {
            crate::services::chat::cancel_turn(state, step.target.clone(), run_id.clone()).await
        } else if let Some(run_id) = &step.workflow_run_id {
            state.workflows().cancel_run(run_id).await.map(|_| ())
        } else if let Some(task_id) = &step.task_id {
            task::cancel_task(state, task_id, Some("Request stopped".into()))
                .await
                .map(|_| ())
        } else {
            Ok(())
        };
        if let Err(e) = stopped {
            log::warn!("Request {id}: stopping step {}: {e}", step.position + 1);
        }
    }
    let now = db::now_ms();
    for step in &mut request.steps {
        if !matches!(step.status, StepStatus::Done | StepStatus::Failed) {
            step.status = StepStatus::Skipped;
            step.ended_at.get_or_insert(now);
        }
    }
    request.status = RequestStatus::Cancelled;
    request.updated_at = now;
    request.ended_at = Some(now);
    state.db().call(move |conn| save(conn, &request)).await?;
    events::emit(state.bus(), &RequestUpdated { id });
    Ok(())
}

/// Do this again: the same plan as a new Request
pub async fn rerun_request(state: &AppState, id: String) -> Result<String, String> {
    let lookup = id.clone();
    let request = state
        .db()
        .call(move |conn| load(conn, &lookup))
        .await?
        .ok_or_else(|| format!("Request {id} not found"))?;
    start_request_from(state, request.plan, request.ask_text, request.source).await
}

/// The run a step's run went on in: its latest compaction child, if any
fn latest_run(conn: &Connection, run_id: &str) -> rusqlite::Result<String> {
    let mut id = run_id.to_string();
    for _ in 0..8 {
        let child: Option<String> = conn
            .query_row(
                "SELECT id FROM runs WHERE parent_run_id = ?1 ORDER BY started_at DESC LIMIT 1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        match child {
            Some(child) => id = child,
            None => break,
        }
    }
    Ok(id)
}

/// The run plans of the Request's agent steps, by step position
pub fn run_plans(
    conn: &Connection,
    request: &LocalRequest,
) -> rusqlite::Result<HashMap<u32, Vec<RunPlanStep>>> {
    let mut plans = HashMap::new();
    for step in &request.steps {
        let Some(run_id) = &step.run_id else {
            continue;
        };
        let run_id = latest_run(conn, run_id)?;
        if let Some(plan) = run_plan::store::current(conn, &run_id)? {
            plans.insert(step.position, plan.steps);
        }
    }
    Ok(plans)
}

/// Save as workflow: a draft of the Request's plan as a workflow file, its
/// runs' hand-overs as manual steps
pub async fn draft_workflow_from_request(state: &AppState, id: String) -> Result<String, String> {
    let lookup = id.clone();
    let (request, plans) = state
        .db()
        .call(move |conn| {
            let Some(request) = load(conn, &lookup)? else {
                return Ok(None);
            };
            let plans = run_plans(conn, &request)?;
            Ok(Some((request, plans)))
        })
        .await?
        .ok_or_else(|| format!("Request {id} not found"))?;
    Ok(workflow_yaml(&request, &plans))
}

/// Picks up every Request that was in progress when the app stopped
pub async fn resume_all(state: &AppState) -> Result<(), String> {
    let ids = state.db().call(|conn| open_ids(conn)).await?;
    for id in ids {
        spawn_runner(state, id);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// As a Request
// ---------------------------------------------------------------------------

fn ts(ms: i64) -> f64 {
    db::ts(ms)
}

/// A local Request on the same page as everything else
pub async fn to_request(state: &AppState, local: LocalRequest) -> Request {
    let agents: std::collections::HashMap<String, String> = state.store().read(|repo| {
        repo.agents()
            .map(|a| (a.id().to_string(), a.name().to_string()))
            .collect()
    });
    let steps: Vec<RequestStep> = local
        .steps
        .iter()
        .map(|s| RequestStep {
            id: format!("step-{}", s.position + 1),
            kind: match s.kind {
                PlanStepKind::LocalAgent => StepKind::Agent,
                PlanStepKind::Workflow => StepKind::Workflow,
                PlanStepKind::Capability => StepKind::Peer,
                PlanStepKind::Human | PlanStepKind::Manual => StepKind::Person,
            },
            stage: None,
            label: s.label.clone(),
            status: s.status,
            link: StepLink {
                agent_id: (s.kind == PlanStepKind::LocalAgent).then(|| s.target.clone()),
                run_id: s.run_id.clone(),
                task_id: s.task_id.clone(),
                workflow_id: (s.kind == PlanStepKind::Workflow).then(|| s.target.clone()),
                workflow_run_id: s.workflow_run_id.clone(),
            },
            progress: None,
            detail: match s.kind {
                PlanStepKind::LocalAgent => agents.get(&s.target).cloned(),
                PlanStepKind::Human => Some(s.target.clone()),
                _ => Some(humanize(&s.target)),
            },
            eta_seconds: None,
            attempts: Vec::new(),
            children: Vec::new(),
            started_at: s.started_at.map(ts),
            ended_at: s.ended_at.map(ts),
        })
        .collect();

    // A step waiting on a tool approval: the banner approves it here
    let mut stuck: Option<Blocker> = None;
    for step in local
        .steps
        .iter()
        .filter(|s| s.status == StepStatus::Waiting)
    {
        let Some(run_id) = step.run_id.clone() else {
            continue;
        };
        let lookup = run_id.clone();
        let approvals = state
            .db()
            .call(move |conn| db::pending_approvals_for_run(conn, &lookup))
            .await
            .unwrap_or_default();
        if let Some(approval) = approvals.first() {
            stuck = Some(Blocker {
                kind: BlockerKind::NeedsMyApproval,
                who: BlockerWho::Me,
                terminal: false,
                step_id: Some(format!("step-{}", step.position + 1)),
                since: ts(approval.created_at),
                deadline: approval.expires_at.map(ts),
                member: None,
                member_instance: None,
                agent_id: Some(step.target.clone()),
                agent_name: agents.get(&step.target).cloned(),
                run_id: Some(run_id),
                approval_id: Some(approval.id.clone()),
                task_id: local.id.clone(),
                reason: None,
                queue_position: None,
            });
            break;
        }
    }
    if stuck.is_none() && local.status == RequestStatus::Failed {
        let step = local.steps.iter().find(|s| s.status == StepStatus::Failed);
        stuck = Some(Blocker {
            kind: BlockerKind::Failed,
            who: BlockerWho::Me,
            terminal: true,
            step_id: step.map(|s| format!("step-{}", s.position + 1)),
            since: ts(local.ended_at.unwrap_or(local.updated_at)),
            deadline: None,
            member: None,
            member_instance: None,
            agent_id: step
                .filter(|s| s.kind == PlanStepKind::LocalAgent)
                .map(|s| s.target.clone()),
            agent_name: step.and_then(|s| agents.get(&s.target).cloned()),
            run_id: step.and_then(|s| s.run_id.clone()),
            approval_id: None,
            task_id: local.id.clone(),
            reason: local.error.clone(),
            queue_position: None,
        });
    }

    let plan_steps = local.steps.len() as u32;
    let current_step = local
        .steps
        .iter()
        .find(|s| !matches!(s.status, StepStatus::Done | StepStatus::Skipped))
        .map_or(plan_steps, |s| s.position + 1);
    let task_ids = local
        .steps
        .iter()
        .filter_map(|s| s.task_id.clone())
        .collect();
    Request {
        id: local.id.clone(),
        title: local.title.clone(),
        input: Value::String(local.ask_text.clone()),
        capability: String::new(),
        direction: Direction::Local,
        counterpart: None,
        counterpart_instance: None,
        status: local.status,
        stuck,
        more_waiting: 0,
        steps,
        plan_steps,
        current_step,
        result: None,
        error: local.error.clone(),
        rating: None,
        eta_seconds: local.plan.estimated_duration_seconds,
        deadline: None,
        started_at: ts(local.created_at),
        updated_at: ts(local.updated_at),
        ended_at: local.ended_at.map(ts),
        task_ids,
        requester_instance: String::new(),
        provider_instance: String::new(),
        cost_usd: None,
        input_tokens: 0,
        output_tokens: 0,
        activity: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ask::PlanStep;

    fn plan(kinds: &[(PlanStepKind, &str)]) -> ActionPlan {
        ActionPlan {
            steps: kinds
                .iter()
                .map(|(kind, target)| PlanStep {
                    kind: *kind,
                    target: (*target).to_string(),
                    description: format!("Do {target}"),
                })
                .collect(),
            ..ActionPlan::default()
        }
    }

    #[test]
    fn a_new_request_runs_its_steps_in_order() {
        let request = new_request(
            "r1",
            "  Summarize the paper\nand more",
            &plan(&[
                (PlanStepKind::LocalAgent, "writer"),
                (PlanStepKind::Capability, "summarize"),
            ]),
            RequestSource::Me,
            10,
        );
        assert_eq!(request.title, "Summarize the paper");
        assert_eq!(request.status, RequestStatus::Running);
        assert_eq!(request.steps[0].depends_on, Vec::<u32>::new());
        assert_eq!(request.steps[1].depends_on, vec![0]);
        assert_eq!(ready_steps(&request.steps), vec![0]);
    }

    #[test]
    fn the_next_step_starts_when_the_one_before_is_done() {
        let mut request = new_request(
            "r1",
            "x",
            &plan(&[
                (PlanStepKind::LocalAgent, "a"),
                (PlanStepKind::LocalAgent, "b"),
            ]),
            RequestSource::Me,
            0,
        );
        request.steps[0].status = StepStatus::Running;
        assert!(ready_steps(&request.steps).is_empty());
        request.steps[0].status = StepStatus::Done;
        assert_eq!(ready_steps(&request.steps), vec![1]);
    }

    #[test]
    fn settles_from_its_steps() {
        let mut request = new_request(
            "r1",
            "x",
            &plan(&[(PlanStepKind::Manual, "m"), (PlanStepKind::LocalAgent, "a")]),
            RequestSource::Me,
            0,
        );
        request.steps[0].status = StepStatus::Waiting;
        settle(&mut request, 5);
        assert_eq!(request.status, RequestStatus::Waiting);
        request.steps[0].status = StepStatus::Done;
        request.steps[1].status = StepStatus::Done;
        settle(&mut request, 9);
        assert_eq!(request.status, RequestStatus::Done);
        assert_eq!(request.ended_at, Some(9));
    }

    #[test]
    fn a_failed_step_fails_it_and_skips_the_rest() {
        let mut request = new_request(
            "r1",
            "x",
            &plan(&[
                (PlanStepKind::LocalAgent, "a"),
                (PlanStepKind::LocalAgent, "b"),
            ]),
            RequestSource::Me,
            0,
        );
        request.steps[0].status = StepStatus::Failed;
        request.steps[0].error = Some("No model".into());
        settle(&mut request, 3);
        assert_eq!(request.status, RequestStatus::Failed);
        assert_eq!(request.error.as_deref(), Some("No model"));
        assert_eq!(request.steps[1].status, StepStatus::Skipped);
    }

    #[test]
    fn drafts_a_workflow_that_the_domain_accepts() {
        let request = new_request(
            "r1",
            "Weekly: summarize \"AI\" news",
            &plan(&[
                (PlanStepKind::LocalAgent, "researcher"),
                (PlanStepKind::Capability, "summarize"),
                (PlanStepKind::Human, "inst-priya"),
            ]),
            RequestSource::Me,
            0,
        );
        let yaml = workflow_yaml(&request, &HashMap::new());
        let workflow = crate::services::domain::Workflow::from_yaml(&yaml)
            .unwrap_or_else(|e| panic!("{e}\n{yaml}"));
        assert_eq!(workflow.id().as_str(), "weekly-summarize-ai-news");
        assert!(yaml.contains("agent: researcher"));
        assert!(yaml.contains("step_type: manual"));
    }

    fn run_step(title: &str, kind: RunStepKind) -> RunPlanStep {
        RunPlanStep {
            title: title.to_string(),
            kind,
            evidence: run_plan::RunStepEvidence::None,
            note: None,
            status: run_plan::RunStepStatus::Done,
            result: None,
            files: Vec::new(),
        }
    }

    #[test]
    fn a_run_plans_hand_overs_become_manual_steps_in_the_draft() {
        let request = new_request(
            "r1",
            "Pay the Acme bill",
            &plan(&[
                (PlanStepKind::LocalAgent, "chief-of-staff"),
                (PlanStepKind::LocalAgent, "writer"),
            ]),
            RequestSource::Email,
            0,
        );
        let run_plan = vec![
            run_step("Open acme-energy.com", RunStepKind::Navigate),
            run_step("Sign in", RunStepKind::Handover),
            run_step("Billing > Pay bill", RunStepKind::Navigate),
            run_step("Enter the amount", RunStepKind::Fill),
            run_step("Pay", RunStepKind::Handover),
            run_step(
                "Note the confirmation, close the browser",
                RunStepKind::Cleanup,
            ),
        ];
        let plans = HashMap::from([(0, run_plan)]);
        let yaml = workflow_yaml(&request, &plans);
        let workflow = crate::services::domain::Workflow::from_yaml(&yaml)
            .unwrap_or_else(|e| panic!("{e}\n{yaml}"));
        // agent, sign in, agent, pay, agent; then the writer's step as it was
        let manual: Vec<bool> = workflow.steps().iter().map(|s| s.is_manual()).collect();
        assert_eq!(manual, vec![false, true, false, true, false, false]);
        assert!(yaml.contains("Your part of \\\"Do chief-of-staff\\\": Sign in"));
        assert!(yaml.contains("Your part of \\\"Do chief-of-staff\\\": Pay"));
        assert!(yaml.contains("Do chief-of-staff: Billing > Pay bill; Enter the amount"));
        assert!(yaml.contains("  - id: step-6\n    agent: writer"));

        // A run plan with no hand-over leaves the step whole
        let plans = HashMap::from([(0, vec![run_step("Read it", RunStepKind::Read)])]);
        assert_eq!(
            workflow_yaml(&request, &plans),
            workflow_yaml(&request, &HashMap::new())
        );
    }

    #[test]
    fn stores_and_reloads_a_request() {
        let mut conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let mut request = new_request(
            "r1",
            "Do it",
            &plan(&[
                (PlanStepKind::Workflow, "daily"),
                (PlanStepKind::Manual, "m"),
            ]),
            RequestSource::Me,
            100,
        );
        save(&mut conn, &request).unwrap();
        request.steps[0].status = StepStatus::Running;
        request.steps[0].workflow_run_id = Some("wr1".into());
        save(&mut conn, &request).unwrap();
        let loaded = load(&conn, "r1").unwrap().unwrap();
        assert_eq!(loaded, request);
        assert_eq!(open_ids(&conn).unwrap(), vec!["r1".to_string()]);
        assert_eq!(request_of(&conn, "wr1").unwrap().as_deref(), Some("r1"));
        assert!(linked_ids(&conn).unwrap().contains("wr1"));
    }

    // -- With the app (in memory) ------------------------------------------

    use std::sync::Arc;

    use crate::services::events::tests::RecordingSink;

    async fn insert_workflow_run(state: &AppState, id: &str, status: &str) {
        let (id, status) = (id.to_string(), status.to_string());
        let context = json!({ "steps": [], "error": null }).to_string();
        state
            .db()
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO workflow_runs (id, workflow_id, status, started_at, context)
                     VALUES (?1, 'onboarding', ?2, ?3, ?4)",
                    params![id, status, db::now_ms(), context],
                )
                .map(|_| ())
            })
            .await
            .unwrap();
    }

    async fn stored(state: &AppState, id: &str) -> LocalRequest {
        let id = id.to_string();
        state
            .db()
            .call(move |conn| load(conn, &id))
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn an_ask_becomes_a_request_on_the_same_page() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let id = start_request(
            &state,
            plan(&[
                (PlanStepKind::Manual, "check"),
                (PlanStepKind::Human, "inst-b"),
            ]),
            "Check the numbers".into(),
        )
        .await
        .unwrap();
        tick(&state, &id).await.unwrap();
        let request = stored(&state, &id).await;
        assert_eq!(request.status, RequestStatus::Waiting);
        assert_eq!(request.steps[0].status, StepStatus::Waiting);
        assert_eq!(request.steps[1].status, StepStatus::Pending);
        assert!(recorder.names().contains(&REQUEST_UPDATED));

        complete_request_step(&state, id.clone(), 0).await.unwrap();
        tick(&state, &id).await.unwrap();
        assert_eq!(
            stored(&state, &id).await.steps[1].status,
            StepStatus::Waiting
        );
        // Only a step waiting on the user can be marked done
        assert!(complete_request_step(&state, id.clone(), 0).await.is_err());

        let page = crate::services::requests::get_request(&state, id.clone())
            .await
            .unwrap();
        assert_eq!(page.direction, Direction::Local);
        assert_eq!(page.title, "Check the numbers");
        assert_eq!(page.plan_steps, 2);
        assert_eq!(page.current_step, 2);
        assert_eq!(page.steps[0].status, StepStatus::Done);

        cancel_request(&state, id.clone()).await.unwrap();
        let request = stored(&state, &id).await;
        assert_eq!(request.status, RequestStatus::Cancelled);
        assert_eq!(request.steps[1].status, StepStatus::Skipped);
        assert!(cancel_request(&state, id).await.is_err());
    }

    #[tokio::test]
    async fn news_from_email_reaches_work_in_progress_and_the_digest() {
        use crate::services::requests::digest::{self, DigestKind};

        let state = AppState::for_tests(Vec::new());
        let id = start_request_from(
            &state,
            plan(&[(PlanStepKind::Manual, "pay")]),
            "Pay the power bill by Friday".into(),
            RequestSource::Email,
        )
        .await
        .unwrap();
        tick(&state, &id).await.unwrap();
        let request = stored(&state, &id).await;
        assert_eq!(request.source, RequestSource::Email);
        assert_eq!(request.status, RequestStatus::Waiting);

        let told = add_update(
            &state,
            id.clone(),
            "The due date moved to Monday".into(),
            RequestSource::Email,
        )
        .await
        .unwrap();
        assert!(told.is_empty(), "no agent is working on it");
        assert_eq!(
            stored(&state, &id).await.ask_text,
            "Pay the power bill by Friday\n\nUpdate: The due date moved to Monday"
        );

        let kinds: Vec<DigestKind> = digest::get(&state)
            .await
            .unwrap()
            .items
            .iter()
            .map(|i| i.kind)
            .collect();
        assert_eq!(kinds, vec![DigestKind::NeedsYou, DigestKind::Updated]);

        // Seen: only the work waiting on the user stays
        digest::mark_seen(&state).await.unwrap();
        let after = digest::get(&state).await.unwrap();
        assert!(after.since.is_some());
        assert_eq!(after.items.len(), 1);
        assert_eq!(after.items[0].kind, DigestKind::NeedsYou);

        // Ended work takes no news
        cancel_request(&state, id.clone()).await.unwrap();
        assert!(add_update(&state, id, "late".into(), RequestSource::Email)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_request_picks_up_after_a_restart() {
        let state = AppState::for_tests(vec![]);
        // As it was when the app stopped: step 1 done, step 2's workflow
        // run started (and since finished)
        let mut request = new_request(
            "r-restart",
            "Onboard Sam",
            &plan(&[
                (PlanStepKind::Manual, "prep"),
                (PlanStepKind::Workflow, "onboarding"),
            ]),
            RequestSource::Me,
            db::now_ms(),
        );
        request.steps[0].status = StepStatus::Done;
        request.steps[1].status = StepStatus::Running;
        request.steps[1].workflow_run_id = Some("wr-1".into());
        let saved = request.clone();
        state
            .db()
            .call(move |conn| save(conn, &saved))
            .await
            .unwrap();
        insert_workflow_run(&state, "wr-1", "completed").await;

        resume_all(&state).await.unwrap();
        for _ in 0..50 {
            if stored(&state, "r-restart").await.status == RequestStatus::Done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let request = stored(&state, "r-restart").await;
        assert_eq!(request.status, RequestStatus::Done);
        assert_eq!(request.steps[1].status, StepStatus::Done);

        // Home lists the Request, not the workflow run it started
        let work = crate::services::work::list_work(&state, None)
            .await
            .unwrap();
        assert!(work.iter().any(|w| w.id == "r-restart"));
        assert!(!work.iter().any(|w| w.id == "wr-1"));
        assert_eq!(
            crate::services::requests::request_for_run(&state, "wr-1".into())
                .await
                .unwrap()
                .as_deref(),
            Some("r-restart")
        );
    }

    // -- The Chief of Staff's own step ---------------------------------------

    use crate::services::builtin_agents::{self, CHIEF_OF_STAFF};
    use crate::services::intake;

    /// The Chief of Staff exists, and every agent answers with its input
    async fn chief_of_staff_answers(state: &AppState) {
        builtin_agents::seed(state.store().layout()).unwrap();
        state
            .store()
            .commit(crate::services::domain::EntityKind::Agent, CHIEF_OF_STAFF)
            .unwrap();
        state
            .db()
            .call(|conn| {
                db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
                db::set_setting(
                    conn,
                    crate::engine::CANNED_SCRIPT_KEY,
                    &json!({ "agent": [{ "text": "Done: [{{input}}]" }], "studio": [], "tokenDelayMs": 0 }),
                )
            })
            .await
            .unwrap();
    }

    fn clean() -> intake::screen::Screening {
        intake::screen::Screening {
            verdict: intake::screen::Verdict::Clean,
            reasons: Vec::new(),
            links: Vec::new(),
        }
    }

    async fn until_ended(state: &AppState, id: &str) -> LocalRequest {
        for _ in 0..100 {
            let request = stored(state, id).await;
            if is_terminal(request.status) {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("Request {id} didn't end: {:?}", stored(state, id).await);
    }

    /// The run's conversation and source
    async fn run_of(state: &AppState, run_id: &str) -> (String, crate::model::SourceType) {
        let id = run_id.to_string();
        state
            .db()
            .call(move |conn| {
                conn.query_row(
                    "SELECT conversation_id, source_type FROM runs WHERE id = ?1",
                    [&id],
                    |row| {
                        let source: String = row.get(1)?;
                        Ok((row.get(0)?, crate::model::SourceType::parse(&source)))
                    },
                )
            })
            .await
            .unwrap()
    }

    async fn main_chat_messages(state: &AppState) -> i64 {
        state
            .db()
            .call(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
                    [db::agent_conversation_id(CHIEF_OF_STAFF)],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_chief_of_staff_step_runs_in_the_task_thread_and_settles() {
        let state = AppState::for_tests(Vec::new());
        chief_of_staff_answers(&state).await;
        // Queued from mail with a plan that gives the Chief of Staff a step
        let item = intake::QueuedWork {
            id: "q1".into(),
            at: db::now_ms() as f64,
            source: RequestSource::Email,
            action: intake::IntakeAction::Start,
            request_id: None,
            request_title: None,
            text: "Pay the Acme Energy bill for October".into(),
            sender: None,
            screening: clean(),
            status: intake::IntakeStatus::Pending,
            result_request_id: None,
            thread_id: intake::thread::id_for("q1"),
            plan: Some(plan(&[(PlanStepKind::LocalAgent, CHIEF_OF_STAFF)])),
        };
        let saved = item.clone();
        state
            .db()
            .call(move |conn| {
                intake::save(conn, &saved)?;
                crate::services::inbox::thread::ensure(
                    conn,
                    &saved.thread_id,
                    "Pay the Acme Energy bill",
                )
            })
            .await
            .unwrap();

        let id = intake::approve(&state, "q1".into()).await.unwrap();
        let request = until_ended(&state, &id).await;
        assert_eq!(request.status, RequestStatus::Done, "{request:?}");
        assert_eq!(request.steps[0].status, StepStatus::Done);
        let run_id = request.steps[0].run_id.clone().expect("the step's run");
        let (conversation, source) = run_of(&state, &run_id).await;
        assert_eq!(conversation, item.thread_id);
        assert_eq!(source, crate::model::SourceType::Request);
        assert_eq!(main_chat_messages(&state).await, 0, "never its main chat");
        state.shutdown();
    }

    #[tokio::test]
    async fn a_chief_of_staff_step_without_a_thread_gets_the_requests_own_conversation() {
        let state = AppState::for_tests(Vec::new());
        chief_of_staff_answers(&state).await;
        let id = start_request(
            &state,
            plan(&[
                (PlanStepKind::LocalAgent, CHIEF_OF_STAFF),
                (PlanStepKind::Manual, "check"),
            ]),
            "Download the October statement from acme-bank.com".into(),
        )
        .await
        .unwrap();
        for _ in 0..100 {
            if stored(&state, &id).await.status == RequestStatus::Waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let request = stored(&state, &id).await;
        assert_eq!(request.steps[0].status, StepStatus::Done, "{request:?}");
        assert_eq!(request.steps[1].status, StepStatus::Waiting);
        let run_id = request.steps[0].run_id.clone().unwrap();
        let (conversation, _) = run_of(&state, &run_id).await;
        assert_eq!(conversation, self_step::conversation_id(&id));
        let owner = state
            .db()
            .call(move |conn| db::conversation_agent(conn, &conversation))
            .await
            .unwrap();
        assert_eq!(owner.as_deref(), Some(CHIEF_OF_STAFF));
        assert_eq!(main_chat_messages(&state).await, 0);

        // Two Chief of Staff steps in one plan are refused
        let twice = plan(&[
            (PlanStepKind::LocalAgent, CHIEF_OF_STAFF),
            (PlanStepKind::LocalAgent, CHIEF_OF_STAFF),
        ]);
        assert!(start_request(&state, twice, "Twice".into()).await.is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn work_a_chief_of_staff_step_starts_waits_for_the_user() {
        let state = AppState::for_tests(Vec::new());
        let before = state.db().call(|conn| open_ids(conn)).await.unwrap();
        let outcome = super::super::tool::call(
            &state,
            RequestSource::ChiefOfStaff,
            None,
            true,
            super::super::tool::START,
            &json!({ "what": "Chief of Staff: pay the Globex bill on globex.com" }),
        )
        .await;
        assert!(!outcome.is_error, "{}", outcome.result);
        assert!(outcome.result.contains("\"queued\""), "{}", outcome.result);
        let pending = state.db().call(|conn| intake::pending(conn)).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].text.contains("Globex"));
        // Nothing started on its own
        assert_eq!(
            state.db().call(|conn| open_ids(conn)).await.unwrap(),
            before
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn recurring_work_is_proposed_as_a_workflow_in_its_thread_and_on_home() {
        use crate::services::requests::digest;

        let state = AppState::for_tests(Vec::new());
        let bill = plan(&[(PlanStepKind::LocalAgent, CHIEF_OF_STAFF)]);
        // Last month's, done
        let mut earlier = new_request(
            "r-oct",
            "Pay the Acme Energy bill for October",
            &bill,
            RequestSource::Email,
            db::now_ms() - 1000,
        );
        earlier.status = RequestStatus::Done;
        earlier.ended_at = Some(earlier.created_at + 10);
        // This month's, through intake, its step's run finished
        let mut now = new_request(
            "r-nov",
            "Pay the Acme Energy bill for November",
            &bill,
            RequestSource::Email,
            db::now_ms(),
        );
        now.steps[0].status = StepStatus::Running;
        now.steps[0].run_id = Some("run-nov".into());
        let thread = intake::thread::id_for("q-nov");
        let item = intake::QueuedWork {
            id: "q-nov".into(),
            at: db::now_ms() as f64,
            source: RequestSource::Email,
            action: intake::IntakeAction::Start,
            request_id: None,
            request_title: None,
            text: now.ask_text.clone(),
            sender: None,
            screening: clean(),
            status: intake::IntakeStatus::Approved,
            result_request_id: Some("r-nov".into()),
            thread_id: thread.clone(),
            plan: Some(bill.clone()),
        };
        let (a, b, t) = (earlier.clone(), now.clone(), thread.clone());
        state
            .db()
            .call(move |conn| {
                save(conn, &a)?;
                save(conn, &b)?;
                intake::save(conn, &item)?;
                crate::services::inbox::thread::ensure(conn, &t, "Pay the Acme Energy bill")?;
                conn.execute(
                    "INSERT INTO runs (id, agent_id, conversation_id, status, input, started_at, source_type)
                     VALUES ('run-nov', ?1, ?2, 'completed', 'x', ?3, 'request')",
                    params![CHIEF_OF_STAFF, t, db::now_ms()],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        assert!(tick(&state, "r-nov").await.unwrap());
        assert_eq!(stored(&state, "r-nov").await.status, RequestStatus::Done);

        // A note in the thread, and the button on its card
        let id = thread.clone();
        let notes: Vec<String> = state
            .db()
            .call(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT content FROM messages WHERE conversation_id = ?1")?;
                let rows = stmt.query_map([id], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<String>>>()
            })
            .await
            .unwrap();
        assert!(
            notes.iter().any(|n| n.contains("Save as workflow")),
            "{notes:?}"
        );
        let loaded = intake::thread::get(&state, thread).await.unwrap();
        assert_eq!(loaded.workflow_proposals.len(), 1);
        assert_eq!(loaded.workflow_proposals[0].like_request_id, "r-oct");

        // On Home with the finished work
        let home = digest::get(&state).await.unwrap();
        let finished = home
            .items
            .iter()
            .find(|i| i.request_id == "r-nov")
            .expect("the finished work");
        assert!(finished.workflow_proposal.is_some());

        // Its draft, for the user to check and save
        let yaml = draft_workflow_from_request(&state, "r-nov".into())
            .await
            .unwrap();
        assert!(yaml.contains("agent: chief-of-staff"));
    }
}
