//! The cross-user delegation lifecycle (wave final, Track B): a `tasks`
//! row per attempt, every transition in `task_events`, progress and status
//! traffic in `task_messages`.
//!
//! ```text
//! Submitted     → Delivered | Cancelled | Abandoned
//! Delivered     → Acknowledged | Rejected | Cancelled | Abandoned | Reassigned
//! Acknowledged  → Working | InputRequired | Cancelled | Failed | TimedOut | Reassigned
//! Working       → Completed | Failed | InputRequired | TimedOut | Cancelled | Reassigned
//! InputRequired → Working | Failed | Cancelled | TimedOut | Reassigned
//! Completed / Failed / Rejected / Cancelled / TimedOut / Abandoned /
//! Reassigned: terminal
//! ```
//!
//! Requester and provider each keep their own row for the same task ID
//! (the requester's has `requester_instance` = us, the provider's
//! `provider_instance` = us); the relay carries the transitions between
//! them (`relay::protocol::PeerMessage`) and decides the rest: it holds
//! work for someone offline, fails it over to the next candidate we
//! sealed it for, hands it on when its provider reassigns it, and ends it
//! at its due date (relay design steps 11 to 13). A task keeps its ID
//! through all of that; our requester row follows its current provider
//! (`task_state::hand_on`).
//!
//! Who writes what:
//! - `delegator` (requester): a row per cross-instance request,
//!   `Submitted` → `Delivered` when the relay hands it over, then the
//!   result. A delegation with a caller waiting on it is *live*
//!   ([`TaskService::is_live`]): the delegator finishes it.
//! - `executor` (provider): a row per inbound task, `TaskDelivered`, the
//!   slot queue, `AcknowledgeTask` with an ETA, `Working`, the result.
//! - [`TaskService`]: inbound `PeerMessage`s, the relay's task events,
//!   results nobody is waiting for (after a restart, an assignment), the
//!   outbox (`queue`), heartbeats and restart resume (`resume`).
//!
//! `task_messages` kinds starting with `_` are internal (the wire `Task` a
//! row was sent as, the provider's run) and not returned by
//! `get_task_messages`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, RwLock, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use tokio::sync::{broadcast::error::RecvError, mpsc, oneshot};

use super::delegator::{instance_of, member_name, A2aDeps};
use super::executor::Executor;
use super::task_state::{self, Change};
use super::{eta, Task, TaskStatus};
use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::relay::protocol::{
    PeerDelivery, PeerMessage, ResealRequest, TaskCompleted, TaskEvent as RelayTaskEvent,
    TaskFailed, TaskRefused, ALL_CAPABILITIES,
};
use crate::services::relay::{RelayClient, RelayStatus};
use crate::services::selection::SelectionService;
use crate::services::AppState;

// The wire shape lives in `nandi-protocol`
pub use nandi_protocol::{ReasonKind, TaskState};

/// `task://state_changed`
pub const TASK_STATE_CHANGED: &str = "task://state_changed";
/// `task://progress`
pub const TASK_PROGRESS: &str = "task://progress";
/// `task://timed_out`
pub const TASK_TIMED_OUT: &str = "task://timed_out";
/// `task://circuit_opened`
pub const TASK_CIRCUIT_OPENED: &str = "task://circuit_opened";

pub type TaskError = String;

/// Why a task row exists
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TaskSourceKind {
    /// An agent (or the user) delegated to a provider
    Direct,
    /// Our gateway dispatched an inbound request to a local target
    Gateway,
    /// A retry on the next provider after a failed attempt
    Failover,
    /// The provider handed the task to a teammate (wave EF)
    Reassign,
}

impl TaskSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Gateway => "gateway",
            Self::Failover => "failover",
            Self::Reassign => "reassign",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "gateway" => Self::Gateway,
            "failover" => Self::Failover,
            "reassign" => Self::Reassign,
            _ => Self::Direct,
        }
    }
}

/// One attempt at a delegated task (a `tasks` row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub requester_instance: String,
    /// Requester agent ID (or card ID for remote requesters)
    pub requester_agent: String,
    pub provider_instance: String,
    pub provider_agent: String,
    pub capability: String,
    pub input: Value,
    pub state: TaskState,
    pub eta_seconds: Option<u32>,
    /// Epoch ms
    pub deadline: Option<Timestamp>,
    /// Higher goes first in the offline queue
    pub priority: i32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub acknowledged_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub error: Option<String>,
    pub result: Option<Value>,
    /// The gateway task this one was dispatched for
    pub parent_task_id: Option<String>,
    pub source_kind: TaskSourceKind,
    /// 1-based; failover makes attempt `n + 1`
    pub attempt_number: u32,
    pub previous_attempt_id: Option<String>,
}

/// One transition (a `task_events` row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub task_id: String,
    pub from_state: Option<TaskState>,
    pub to_state: TaskState,
    /// Plain words for Details and Activity; the UI's message comes from
    /// `reason_kind`
    pub reason: Option<String>,
    /// Why it's waiting or was refused (work-model §2.3); `None` on
    /// transitions that carry no typed reason
    #[serde(default)]
    pub reason_kind: Option<ReasonKind>,
    /// Which part of handling it this happened in (work-model §1.2)
    #[serde(default)]
    pub stage: Option<TaskStage>,
    pub at: Timestamp,
}

/// Where in handling a task an event happened (the gateway's admit →
/// policy → approval → slot → route → dispatch → child → result)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TaskStage {
    Admit,
    Policy,
    Approval,
    Slot,
    Route,
    Dispatch,
    Child,
    Result,
}

impl TaskStage {
    pub const ALL: [TaskStage; 8] = [
        Self::Admit,
        Self::Policy,
        Self::Approval,
        Self::Slot,
        Self::Route,
        Self::Dispatch,
        Self::Child,
        Self::Result,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admit => "admit",
            Self::Policy => "policy",
            Self::Approval => "approval",
            Self::Slot => "slot",
            Self::Route => "route",
            Self::Dispatch => "dispatch",
            Self::Child => "child",
            Self::Result => "result",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.as_str() == value)
    }
}

/// A refusal with its kind: what the requester is told, and why in a form
/// the UI can match to a fix
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub kind: ReasonKind,
    pub stage: TaskStage,
    pub message: String,
}

impl Refusal {
    pub fn new(kind: ReasonKind, stage: TaskStage, message: impl Into<String>) -> Self {
        Self {
            kind,
            stage,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Payload of `task://state_changed`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskStateChanged {
    pub task: TaskRecord,
    pub event: TaskEvent,
}

/// Payload of `task://progress`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub task_id: String,
    /// 0–100, when the provider knows
    pub percent: Option<u8>,
    pub note: Option<String>,
    /// Set while the provider holds or queues it (task 5)
    #[serde(default)]
    pub reason_kind: Option<ReasonKind>,
    /// 1-based place in the provider's line, while queued
    #[serde(default)]
    pub queue_position: Option<u32>,
    pub at: Timestamp,
}

/// The stage a provider's "still waiting" reason belongs to
pub fn waiting_stage(kind: ReasonKind) -> Option<TaskStage> {
    match kind {
        ReasonKind::NeedsTheirApproval | ReasonKind::NeedsMyApproval => Some(TaskStage::Approval),
        ReasonKind::Queued => Some(TaskStage::Slot),
        _ => None,
    }
}

/// From the last 100 completions of a capability on a provider agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EtaEstimate {
    pub median_seconds: u32,
    pub p95_seconds: u32,
    /// Below 5 the estimate is a low-confidence default
    pub sample_count: u32,
}

/// Progress, status, clarification, … traffic about a task (a
/// `task_messages` row)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskMessage {
    pub id: u32,
    /// `progress`, `status_query`, `status_response`, `clarification_request`,
    /// `clarification_response`, `note`, …
    pub kind: String,
    pub payload: Value,
    pub at: Timestamp,
}

/// `list_tasks` filter. `role`: `requester` (tasks we sent) or `provider`
/// (tasks we serve); `limit` 0 means the default (100).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskFilter {
    pub state: Option<String>,
    pub role: Option<String>,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub limit: u32,
}

/// What `query_task_status` answers (the provider's `StatusResponse`, or
/// the local row when the provider is unreachable)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusReport {
    pub task_id: String,
    pub state: TaskState,
    pub eta_seconds: Option<u32>,
    pub elapsed_seconds: u32,
    /// `false`: answered from the local row
    pub live: bool,
}

impl NandiEvent for TaskStateChanged {
    const NAME: &'static str = TASK_STATE_CHANGED;
}

impl NandiEvent for TaskProgress {
    const NAME: &'static str = TASK_PROGRESS;
}

/// The wire `Task` a row was sent as (requester) or arrived as (provider)
pub const REQUEST_KIND: &str = "_request";
/// The provider's turn: `{"agentId", "runId"}`
pub const RUN_KIND: &str = "_run";
pub const PROGRESS_KIND: &str = "progress";
/// The relay gave the task to the next candidate: `{from, to, toName,
/// cause, reasonKind}`
pub const FAILED_OVER_KIND: &str = "failed_over";
pub const STATUS_RESPONSE_KIND: &str = "status_response";

/// Most rows `list_tasks` returns
pub const MAX_LIST: u32 = 1_000;
/// `list_tasks` when the filter's `limit` is 0
pub const DEFAULT_LIST: u32 = 100;

/// The task asked for `ProgressUpdate`s (`"wants_progress": true`)
pub fn wants_progress(input: &Value) -> bool {
    input.get("wants_progress").and_then(Value::as_bool) == Some(true)
}

/// Offline-queue priority from the input (`"priority": n`), default 0
pub fn priority_of(input: &Value) -> i32 {
    input
        .get("priority")
        .and_then(Value::as_i64)
        .map_or(0, |p| p.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

/// The agent part of a card ID (`<instance>/<agent>`)
pub fn agent_of(card_id: &str) -> &str {
    card_id.split_once('/').map_or(card_id, |(_, agent)| agent)
}

impl TaskRecord {
    /// A `Submitted` row for a task we send to `provider_card`
    pub fn requester(
        id: &str,
        local_instance: &str,
        requester_agent: &str,
        provider_card: &str,
        capability: &str,
        input: &Value,
        deadline: Option<Timestamp>,
    ) -> Self {
        let now = now_ms() as f64;
        Self {
            id: id.to_string(),
            requester_instance: local_instance.to_string(),
            requester_agent: requester_agent.to_string(),
            provider_instance: instance_of(provider_card).to_string(),
            provider_agent: agent_of(provider_card).to_string(),
            capability: capability.to_string(),
            input: input.clone(),
            state: TaskState::Submitted,
            eta_seconds: None,
            deadline,
            priority: priority_of(input),
            created_at: now,
            updated_at: now,
            acknowledged_at: None,
            completed_at: None,
            error: None,
            result: None,
            parent_task_id: None,
            source_kind: TaskSourceKind::Direct,
            attempt_number: 1,
            previous_attempt_id: None,
        }
    }

    /// A `Submitted` row for a task the relay assigned us
    pub fn provider(task: &Task, local_instance: &str) -> Self {
        let mut record = Self::requester(
            &task.id,
            &task.requester.instance_id,
            &task.requester.id,
            &task.target,
            &task.capability,
            &task.input,
            task.deadline,
        );
        record.provider_instance = local_instance.to_string();
        record
    }

    /// `<provider_instance>/<provider_agent>`
    pub fn provider_card(&self) -> String {
        format!("{}/{}", self.provider_instance, self.provider_agent)
    }

    pub fn is_requester(&self, local_instance: &str) -> bool {
        self.requester_instance == local_instance
    }

    /// Seconds since it was acknowledged (else created), up to its end
    pub fn elapsed_seconds(&self, now: i64) -> u32 {
        let start = self.acknowledged_at.unwrap_or(self.created_at);
        let end = self.completed_at.unwrap_or(now as f64);
        ((end - start).max(0.0) / 1000.0) as u32
    }

    pub fn status_report(&self, live: bool) -> TaskStatusReport {
        TaskStatusReport {
            task_id: self.id.clone(),
            state: self.state,
            eta_seconds: self.eta_seconds,
            elapsed_seconds: self.elapsed_seconds(now_ms()),
            live,
        }
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

const COLUMNS: &str = "id, requester_instance, requester_agent, provider_instance, provider_agent,
    capability, input, state, eta_seconds, deadline, priority, created_at, updated_at,
    acknowledged_at, completed_at, error, result, parent_task_id, source_kind, attempt_number,
    previous_attempt_id";

fn json_of(text: Option<String>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(&t).ok())
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let state: String = row.get(7)?;
    let source_kind: String = row.get(18)?;
    let ms = |i: usize| -> rusqlite::Result<Option<f64>> {
        Ok(row.get::<_, Option<i64>>(i)?.map(|t| t as f64))
    };
    Ok(TaskRecord {
        id: row.get(0)?,
        requester_instance: row.get(1)?,
        requester_agent: row.get(2)?,
        provider_instance: row.get(3)?,
        provider_agent: row.get(4)?,
        capability: row.get(5)?,
        input: json_of(row.get(6)?).unwrap_or(Value::Null),
        state: TaskState::parse(&state).unwrap_or(TaskState::Failed),
        eta_seconds: row.get(8)?,
        deadline: ms(9)?,
        priority: row.get(10)?,
        created_at: row.get::<_, i64>(11)? as f64,
        updated_at: row.get::<_, i64>(12)? as f64,
        acknowledged_at: ms(13)?,
        completed_at: ms(14)?,
        error: row.get(15)?,
        result: json_of(row.get(16)?),
        parent_task_id: row.get(17)?,
        source_kind: TaskSourceKind::parse(&source_kind),
        attempt_number: row.get(19)?,
        previous_attempt_id: row.get(20)?,
    })
}

pub fn insert(conn: &Connection, t: &TaskRecord) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO tasks ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
             ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)"
        ),
        params![
            t.id,
            t.requester_instance,
            t.requester_agent,
            t.provider_instance,
            t.provider_agent,
            t.capability,
            t.input.to_string(),
            t.state.as_str(),
            t.eta_seconds,
            t.deadline.map(|d| d as i64),
            t.priority,
            t.created_at as i64,
            t.updated_at as i64,
            t.acknowledged_at.map(|d| d as i64),
            t.completed_at.map(|d| d as i64),
            t.error,
            t.result.as_ref().map(Value::to_string),
            t.parent_task_id,
            t.source_kind.as_str(),
            t.attempt_number,
            t.previous_attempt_id,
        ],
    )?;
    Ok(())
}

pub fn load(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<TaskRecord>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1"),
        [task_id],
        from_row,
    )
    .optional()
}

/// Rows matching a `WHERE` clause (no user input in `clause`)
pub fn select(
    conn: &Connection,
    clause: &str,
    args: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<TaskRecord>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM tasks WHERE {clause}"))?;
    let rows = stmt.query_map(args, from_row)?;
    rows.collect()
}

pub fn insert_event(conn: &Connection, event: &TaskEvent) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO task_events (task_id, from_state, to_state, reason, reason_kind, stage, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.task_id,
            event.from_state.map(TaskState::as_str),
            event.to_state.as_str(),
            event.reason,
            event.reason_kind.map(ReasonKind::as_str),
            event.stage.map(TaskStage::as_str),
            event.at as i64,
        ],
    )?;
    Ok(())
}

pub fn insert_message(
    conn: &Connection,
    task_id: &str,
    kind: &str,
    payload: &Value,
    at: i64,
) -> rusqlite::Result<TaskMessage> {
    conn.execute(
        "INSERT INTO task_messages (task_id, kind, payload, at) VALUES (?1, ?2, ?3, ?4)",
        params![task_id, kind, payload.to_string(), at],
    )?;
    Ok(TaskMessage {
        id: u32::try_from(conn.last_insert_rowid()).unwrap_or(u32::MAX),
        kind: kind.to_string(),
        payload: payload.clone(),
        at: at as f64,
    })
}

/// The newest `kind` message of a task
pub fn latest_message(
    conn: &Connection,
    task_id: &str,
    kind: &str,
) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        "SELECT payload FROM task_messages WHERE task_id = ?1 AND kind = ?2
          ORDER BY id DESC LIMIT 1",
        params![task_id, kind],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(json_of)
}

/// Inserts a new row (`None → Submitted` event, and the wire task as a
/// `_request` message) in one transaction, then `task://state_changed`.
pub async fn create(
    db: &Db,
    bus: &EventBus,
    record: &TaskRecord,
    request: Option<&Task>,
) -> Result<TaskRecord, String> {
    let row = record.clone();
    let request = request
        .map(|task| serde_json::to_value(task).map_err(|e| e.to_string()))
        .transpose()?;
    let event = TaskEvent {
        task_id: record.id.clone(),
        from_state: None,
        to_state: record.state,
        reason: None,
        reason_kind: None,
        stage: None,
        at: record.created_at,
    };
    let written = event.clone();
    db.call(move |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        insert(&tx, &row)?;
        insert_event(&tx, &written)?;
        if let Some(request) = &request {
            insert_message(&tx, &row.id, REQUEST_KIND, request, now_ms())?;
        }
        tx.commit()
    })
    .await?;
    events::emit(
        bus,
        &TaskStateChanged {
            task: record.clone(),
            event,
        },
    );
    Ok(record.clone())
}

pub async fn get_row(db: &Db, task_id: &str) -> Result<Option<TaskRecord>, String> {
    let id = task_id.to_string();
    db.call(move |conn| load(conn, &id)).await
}

/// The wire task stored with a row
pub async fn request_of(db: &Db, task_id: &str) -> Result<Option<Task>, String> {
    let id = task_id.to_string();
    let value = db
        .call(move |conn| latest_message(conn, &id, REQUEST_KIND))
        .await?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// Oldest first
pub async fn events_of(db: &Db, task_id: &str) -> Result<Vec<TaskEvent>, String> {
    let id = task_id.to_string();
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT task_id, from_state, to_state, reason, reason_kind, stage, at
               FROM task_events WHERE task_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([id], |row| {
            let from: Option<String> = row.get(1)?;
            let to: String = row.get(2)?;
            let kind: Option<String> = row.get(4)?;
            let stage: Option<String> = row.get(5)?;
            Ok(TaskEvent {
                task_id: row.get(0)?,
                from_state: from.as_deref().and_then(TaskState::parse),
                to_state: TaskState::parse(&to).unwrap_or(TaskState::Failed),
                reason: row.get(3)?,
                reason_kind: kind.as_deref().and_then(ReasonKind::parse),
                stage: stage.as_deref().and_then(TaskStage::parse),
                at: row.get::<_, i64>(6)? as f64,
            })
        })?;
        rows.collect()
    })
    .await
}

/// Oldest first, internal kinds left out
pub async fn messages_of(db: &Db, task_id: &str) -> Result<Vec<TaskMessage>, String> {
    let id = task_id.to_string();
    db.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, kind, payload, at FROM task_messages
              WHERE task_id = ?1 AND substr(kind, 1, 1) != '_' ORDER BY id",
        )?;
        let rows = stmt.query_map([id], |row| {
            Ok(TaskMessage {
                id: row.get(0)?,
                kind: row.get(1)?,
                payload: json_of(row.get(2)?).unwrap_or(Value::Null),
                at: row.get::<_, i64>(3)? as f64,
            })
        })?;
        rows.collect()
    })
    .await
}

pub async fn add_message(
    db: &Db,
    task_id: &str,
    kind: &str,
    payload: Value,
) -> Result<TaskMessage, String> {
    let (id, kind) = (task_id.to_string(), kind.to_string());
    db.call(move |conn| insert_message(conn, &id, &kind, &payload, now_ms()))
        .await
}

/// Newest first
pub async fn list_rows(
    db: &Db,
    local_instance: &str,
    filter: TaskFilter,
) -> Result<Vec<TaskRecord>, String> {
    let state = match filter.state.as_deref().filter(|s| !s.is_empty()) {
        Some(state) => {
            Some(TaskState::parse(state).ok_or_else(|| format!("Unknown task state '{state}'"))?)
        }
        None => None,
    };
    let role_column = match filter.role.as_deref().filter(|r| !r.is_empty()) {
        None => None,
        Some("requester") => Some("requester_instance"),
        Some("provider") => Some("provider_instance"),
        Some(other) => return Err(format!("Unknown task role '{other}'")),
    };
    let limit = match filter.limit {
        0 => DEFAULT_LIST,
        n => n.min(MAX_LIST),
    };
    let me = local_instance.to_string();
    let agent = filter.agent_id.filter(|a| !a.is_empty());
    db.call(move |conn| {
        let mut clauses = vec!["1 = 1".to_string()];
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(state) = state {
            args.push(Box::new(state.as_str()));
            clauses.push(format!("state = ?{}", args.len()));
        }
        if let Some(column) = role_column {
            args.push(Box::new(me.clone()));
            clauses.push(format!("{column} = ?{}", args.len()));
        }
        if let Some(agent) = agent {
            args.push(Box::new(agent));
            let n = args.len();
            clauses.push(format!("(requester_agent = ?{n} OR provider_agent = ?{n})"));
        }
        args.push(Box::new(limit));
        let clause = format!(
            "{} ORDER BY created_at DESC, rowid DESC LIMIT ?{}",
            clauses.join(" AND "),
            args.len()
        );
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        select(conn, &clause, &refs)
    })
    .await
}

/// Ends a requester row with what the provider (or the relay) answered:
/// the row walks to `Completed` / `Failed` (`Rejected` before it was
/// acknowledged) / `Cancelled` / `TimedOut`; completions go into
/// `eta_history` and close the provider's circuit, failures count against
/// it. `Ok(None)`: no such row, or it already ended.
pub async fn finish_requester(
    db: &Db,
    bus: &EventBus,
    task_id: &str,
    status: TaskStatus,
    output: Value,
    error: Option<String>,
) -> Result<Option<TaskRecord>, String> {
    let Some(task) = get_row(db, task_id).await? else {
        return Ok(None);
    };
    if task.state.is_terminal() {
        return Ok(None);
    }
    let error = || error.clone().unwrap_or_else(|| status.as_str().to_string());
    let (to, change) = match status {
        TaskStatus::Completed => (
            TaskState::Completed,
            Change {
                reason: Some("the provider sent its result".to_string()),
                result: Some(output),
                ..Change::default()
            },
        ),
        TaskStatus::Cancelled => (TaskState::Cancelled, Change::error(error())),
        TaskStatus::TimedOut => (TaskState::TimedOut, Change::error(error())),
        TaskStatus::Failed | TaskStatus::Running => {
            let to = if matches!(task.state, TaskState::Submitted | TaskState::Delivered) {
                TaskState::Rejected
            } else {
                TaskState::Failed
            };
            (to, Change::error(error()))
        }
    };
    let Some(ended) = task_state::advance(db, bus, task_id, to, change).await? else {
        return Ok(None);
    };
    // The ETA we show next time; the providers' track record and
    // circuits are the relay's
    if status == TaskStatus::Completed {
        let seconds = ((now_ms() as f64 - task.created_at).max(0.0) / 1000.0) as u32;
        let (capability, card) = (task.capability.clone(), task.provider_card());
        db.call(move |conn| eta::record(conn, &capability, &card, seconds, now_ms()))
            .await?;
    }
    Ok(Some(ended))
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// Background intervals; tests shorten them.
#[derive(Debug, Clone)]
pub struct TaskConfig {
    /// How often the outbox is flushed (also whenever the relay connects)
    pub sweep_interval: Duration,
    /// "still working" progress for tasks that want it
    pub heartbeat_interval: Duration,
    /// How long `query_task_status` waits for the provider
    pub status_timeout: Duration,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            sweep_interval: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(30),
            status_timeout: Duration::from_secs(5),
        }
    }
}

/// Whether an instance is reachable through the relay. The default asks
/// the relay for every card (presence); tests inject their own.
pub type Presence = Arc<dyn Fn(String) -> BoxFuture<'static, bool> + Send + Sync>;

// Short-lived (one per delivery): boxing buys nothing. The variants grew
// when serde_json's `preserve_order` (required by the ACP SDK) made JSON
// maps IndexMaps.
#[allow(clippy::large_enum_variant)]
enum Inbound {
    Peer(PeerDelivery),
    Completed(TaskCompleted),
    Failed(TaskFailed),
    Event(RelayTaskEvent),
    Refused(TaskRefused),
    Reseal(ResealRequest),
}

/// Services by instance ID, so a delegator built before `task::init` (and
/// without an `AppState`) can find its instance's service
static SERVICES: LazyLock<Mutex<HashMap<String, Weak<TaskService>>>> =
    LazyLock::new(Mutex::default);

/// The task service of a local instance, once `task::init` ran
pub fn service_for(instance_id: &str) -> Option<Arc<TaskService>> {
    SERVICES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(instance_id)
        .and_then(Weak::upgrade)
}

/// The task lifecycle service (Track B). No `AppState` inside: it holds
/// the delegation deps (DB, bus, relay, orchestrators) like the delegator.
pub struct TaskService {
    pub(super) deps: A2aDeps,
    /// Runs resumed provider tasks
    pub(super) executor: Arc<Executor>,
    pub(super) selection: Option<Arc<dyn SelectionService>>,
    pub(super) config: TaskConfig,
    runtime: tokio::runtime::Handle,
    presence: RwLock<Option<Presence>>,
    status_waiters: Mutex<HashMap<String, oneshot::Sender<Option<TaskStatusReport>>>>,
    live: Mutex<HashSet<String>>,
    /// One outbox flush at a time: the relay connecting and the sweep
    /// both flush, and two at once would send the same request twice
    pub(super) flushing: tokio::sync::Mutex<()>,
    inbound: mpsc::UnboundedSender<Inbound>,
}

impl TaskService {
    /// Starts the inbound consumer (one at a time, in arrival order).
    /// [`listen`](Self::listen) subscribes it to the relay; [`start`](Self::start)
    /// starts resume and the background loops.
    pub fn new(
        deps: A2aDeps,
        selection: Option<Arc<dyn SelectionService>>,
        config: TaskConfig,
        runtime: &tokio::runtime::Handle,
    ) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let service = Arc::new(Self {
            executor: Arc::new(Executor::new(deps.clone())),
            deps,
            selection,
            config,
            runtime: runtime.clone(),
            presence: RwLock::default(),
            status_waiters: Mutex::default(),
            live: Mutex::default(),
            flushing: tokio::sync::Mutex::new(()),
            inbound: tx,
        });
        let weak = Arc::downgrade(&service);
        runtime.spawn(async move {
            while let Some(message) = rx.recv().await {
                let Some(service) = weak.upgrade() else {
                    break;
                };
                service.handle(message).await;
            }
        });
        service
    }

    pub fn db(&self) -> &Db {
        &self.deps.db
    }

    pub fn bus(&self) -> &EventBus {
        &self.deps.bus
    }

    pub fn instance_id(&self) -> &str {
        &self.deps.instance_id
    }

    pub fn relay(&self) -> Option<&Arc<dyn RelayClient>> {
        self.deps.relay.as_ref()
    }

    pub(super) fn runtime(&self) -> &tokio::runtime::Handle {
        &self.runtime
    }

    /// Makes [`service_for`] find this service (replacing any earlier one)
    pub fn register(self: &Arc<Self>) {
        SERVICES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.deps.instance_id.clone(), Arc::downgrade(self));
    }

    /// Subscribes to lifecycle traffic, results, the relay's task events,
    /// its refusals and its reseal requests. No-op without a relay client.
    pub async fn listen(&self) {
        let Some(relay) = self.relay() else {
            return;
        };
        let tx = self.inbound.clone();
        relay
            .on_peer(Box::new(move |delivery| {
                let _ = tx.send(Inbound::Peer(delivery));
            }))
            .await;
        let tx = self.inbound.clone();
        relay
            .on_task_completed(Box::new(move |completed| {
                let _ = tx.send(Inbound::Completed(completed));
            }))
            .await;
        let tx = self.inbound.clone();
        relay
            .on_task_failed(Box::new(move |failed| {
                let _ = tx.send(Inbound::Failed(failed));
            }))
            .await;
        let tx = self.inbound.clone();
        relay
            .on_task_event(Box::new(move |event| {
                let _ = tx.send(Inbound::Event(event));
            }))
            .await;
        let tx = self.inbound.clone();
        relay
            .on_task_refused(Box::new(move |refused| {
                let _ = tx.send(Inbound::Refused(refused));
            }))
            .await;
        let tx = self.inbound.clone();
        relay
            .on_reseal(Box::new(move |request| {
                let _ = tx.send(Inbound::Reseal(request));
            }))
            .await;
    }

    /// Restart resume, then the outbox flush (on a timer and whenever the
    /// relay connects), the result outbox flush (whenever it connects) and
    /// heartbeats. Each loop ends with the service.
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let sweep = self.config.sweep_interval;
        self.runtime.spawn(async move {
            if let Some(service) = weak.upgrade() {
                if let Err(e) = service.resume().await {
                    log::error!("Failed to resume tasks: {e}");
                }
            }
            let mut tick = tokio::time::interval(sweep);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(service) = weak.upgrade() else {
                    break;
                };
                service.flush().await;
            }
        });

        let weak = Arc::downgrade(self);
        let every = self.config.heartbeat_interval;
        self.runtime.spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(service) = weak.upgrade() else {
                    break;
                };
                if let Err(e) = service.heartbeat().await {
                    log::warn!("Task heartbeat failed: {e}");
                }
            }
        });

        let weak = Arc::downgrade(self);
        let mut events = self.deps.broadcast.subscribe();
        self.runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event)
                        if event.event == RelayStatus::NAME
                            && event.payload["connected"].as_bool() == Some(true) => {}
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
                let Some(service) = weak.upgrade() else {
                    break;
                };
                service.flush().await;
                service.flush_results().await;
            }
        });
    }

    pub async fn is_online(&self, instance: &str) -> bool {
        let custom = self
            .presence
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(presence) = custom {
            return presence(instance.to_string()).await;
        }
        let Some(relay) = self.relay() else {
            return false;
        };
        if !relay.status().connected {
            return false;
        }
        match relay.query_capability(ALL_CAPABILITIES).await {
            Ok(cards) => cards.iter().any(|card| card.instance_id == instance),
            // Can't tell: don't give up on it
            Err(_) => true,
        }
    }

    fn lock_live(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A caller is waiting on this task (the delegator finishes it)
    pub fn mark_live(&self, task_id: &str) {
        self.lock_live().insert(task_id.to_string());
    }

    pub fn unmark_live(&self, task_id: &str) {
        self.lock_live().remove(task_id);
    }

    pub fn is_live(&self, task_id: &str) -> bool {
        self.lock_live().contains(task_id)
    }

    // -- Rows ---------------------------------------------------------------

    pub async fn create(
        &self,
        record: &TaskRecord,
        request: Option<&Task>,
    ) -> Result<TaskRecord, String> {
        create(self.db(), self.bus(), record, request).await
    }

    pub async fn get(&self, task_id: &str) -> Result<TaskRecord, String> {
        get_row(self.db(), task_id)
            .await?
            .ok_or_else(|| format!("Task {task_id} not found"))
    }

    /// Sets one column (`column` is one of ours, never user input)
    async fn update<V>(&self, task_id: &str, column: &'static str, value: V) -> Result<(), String>
    where
        V: rusqlite::ToSql + Send + 'static,
    {
        let id = task_id.to_string();
        let updated = self
            .db()
            .call(move |conn| {
                conn.execute(
                    &format!("UPDATE tasks SET {column} = ?1, updated_at = ?2 WHERE id = ?3"),
                    params![value, now_ms(), id],
                )
            })
            .await?;
        if updated == 0 {
            return Err(format!("Task {task_id} not found"));
        }
        Ok(())
    }

    pub async fn set_eta(&self, task_id: &str, eta_seconds: u32) -> Result<(), String> {
        self.update(task_id, "eta_seconds", eta_seconds).await
    }

    // -- Messages -----------------------------------------------------------

    pub(super) async fn send(&self, target: &str, body: PeerMessage) -> Result<(), String> {
        let relay = crate::services::relay::unavailable::connected(self.relay())?;
        relay.send_peer(target.to_string(), body).await
    }

    /// Provider side: a `ProgressUpdate` to the requester, when the task
    /// asked for them. Returns whether it was sent.
    #[cfg(test)]
    pub async fn send_progress(
        &self,
        task_id: &str,
        percent: Option<u8>,
        note: Option<String>,
    ) -> Result<bool, String> {
        let task = self.get(task_id).await?;
        if task.is_requester(self.instance_id()) || !wants_progress(&task.input) {
            return Ok(false);
        }
        send_progress(self.relay(), self.db(), &task, percent, note).await
    }

    /// Requester side: asks the provider (the answer comes as a
    /// `StatusResponse`; see [`query_status`](Self::query_status)).
    pub async fn send_status_query(&self, task_id: &str) -> Result<(), String> {
        let task = self.get(task_id).await?;
        self.send(
            &task.provider_instance,
            PeerMessage::StatusQuery {
                task_id: task.id.clone(),
            },
        )
        .await
    }

    /// "still working" to the requesters of our `Working` tasks that want
    /// progress. Returns how many were sent.
    pub async fn heartbeat(&self) -> Result<u32, String> {
        let me = self.instance_id().to_string();
        let working = self
            .db()
            .call(move |conn| {
                select(
                    conn,
                    "provider_instance = ?1 AND requester_instance != ?1 AND state = 'working'",
                    &[&me],
                )
            })
            .await?;
        let mut sent = 0;
        for task in working.iter().filter(|t| wants_progress(&t.input)) {
            match send_progress(
                self.relay(),
                self.db(),
                task,
                None,
                Some("still working".to_string()),
            )
            .await
            {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => log::debug!("Heartbeat for task {} not sent: {e}", task.id),
            }
        }
        Ok(sent)
    }

    /// Cancels a task that hasn't ended, tells the other side, and (as the
    /// provider) stops its turn.
    pub async fn cancel(
        &self,
        task_id: &str,
        reason: Option<String>,
    ) -> Result<TaskRecord, String> {
        let task = self.get(task_id).await?;
        if task.state.is_terminal() {
            return Err(format!(
                "Task {task_id} already ended ({})",
                task.state.as_str()
            ));
        }
        let change = Change {
            reason: Some(reason.clone().unwrap_or_else(|| "cancelled".to_string())),
            ..Change::default()
        };
        let cancelled =
            task_state::apply(self.db(), self.bus(), task_id, TaskState::Cancelled, change)
                .await?
                .task;
        let requester = task.is_requester(self.instance_id());
        let other = if requester {
            &task.provider_instance
        } else {
            &task.requester_instance
        };
        if let Err(e) = self
            .send(
                other,
                PeerMessage::CancelTask {
                    task_id: task_id.to_string(),
                    reason,
                },
            )
            .await
        {
            log::info!("Couldn't tell {other} that task {task_id} was cancelled: {e}");
        }
        if !requester {
            self.stop_run(task_id).await;
        }
        Ok(cancelled)
    }

    /// The provider's answer to a `StatusQuery` (within the status
    /// timeout), else the local row (`live: false`). Our own provider rows
    /// are the live state.
    pub async fn query_status(&self, task_id: &str) -> Result<TaskStatusReport, String> {
        let task = self.get(task_id).await?;
        if !task.is_requester(self.instance_id()) {
            return Ok(task.status_report(true));
        }
        if task.state.is_terminal() {
            return Ok(task.status_report(false));
        }
        let (tx, rx) = oneshot::channel();
        self.status_waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task_id.to_string(), tx);
        let answered = match self.send_status_query(task_id).await {
            Ok(()) => tokio::time::timeout(self.config.status_timeout, rx)
                .await
                .ok()
                .and_then(Result::ok)
                .flatten(),
            Err(_) => None,
        };
        self.status_waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(task_id);
        match answered {
            Some(report) => {
                if let Some(eta) = report.eta_seconds.filter(|e| Some(*e) != task.eta_seconds) {
                    self.set_eta(task_id, eta).await?;
                }
                Ok(report)
            }
            None => Ok(task.status_report(false)),
        }
    }

    fn resolve_status(&self, task_id: &str, report: Option<TaskStatusReport>) {
        let waiter = self
            .status_waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(task_id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(report);
        }
    }

    /// Provider side: stops the turn running a task, if any
    pub(super) async fn stop_run(&self, task_id: &str) {
        let id = task_id.to_string();
        let run = self
            .db()
            .call(move |conn| latest_message(conn, &id, RUN_KIND))
            .await
            .ok()
            .flatten();
        if let Some((Some(agent), Some(run_id))) = run
            .as_ref()
            .map(|run| (run["agentId"].as_str(), run["runId"].as_str()))
        {
            return self.executor.stop(agent, run_id).await;
        }
        // A gateway task: the turn its routed child runs
        self.executor.stop_task(task_id).await;
    }

    // -- Inbound ------------------------------------------------------------

    async fn handle(self: &Arc<Self>, message: Inbound) {
        let outcome = match message {
            Inbound::Peer(delivery) => self.on_peer(delivery).await,
            Inbound::Completed(completed) => {
                let result = completed.result;
                self.on_result(
                    &completed.task_id,
                    result.status,
                    result.output,
                    result.error,
                )
                .await
            }
            Inbound::Failed(failed) => {
                self.on_result(
                    &failed.task_id,
                    TaskStatus::Failed,
                    Value::Null,
                    Some(failed.error),
                )
                .await
            }
            Inbound::Event(event) => self.on_event(event).await,
            Inbound::Refused(refused) => super::result_outbox::settle(self.db(), &refused).await,
            Inbound::Reseal(request) => super::reassign::reseal(self, request).await,
        };
        if let Err(e) = outcome {
            log::warn!("Task lifecycle: {e}");
        }
    }

    /// A result for a requester row nobody waits on (live ones are the
    /// delegator's)
    async fn on_result(
        self: &Arc<Self>,
        task_id: &str,
        status: TaskStatus,
        output: Value,
        error: Option<String>,
    ) -> Result<(), String> {
        if self.is_live(task_id) {
            return Ok(());
        }
        match get_row(self.db(), task_id).await? {
            Some(task) if task.is_requester(self.instance_id()) => {}
            _ => return Ok(()),
        }
        let ended = finish_requester(
            self.db(),
            self.bus(),
            task_id,
            status,
            output.clone(),
            error.clone(),
        )
        .await?;
        let Some(ended) = ended else {
            return Ok(());
        };
        // The requester's history row stayed open (an assignment, or a
        // request nobody had answered when the live wait ended)
        let _ = ended;
        super::history::close(self.db(), self.bus(), task_id, status, output, error).await?;
        Ok(())
    }

    /// What the relay did with one of our tasks. Its answers to our
    /// submissions (`accepted`, `refused`) are for whoever submitted.
    async fn on_event(self: &Arc<Self>, event: RelayTaskEvent) -> Result<(), String> {
        let task_id = event.task_id().to_string();
        let Some(task) = get_row(self.db(), &task_id).await? else {
            log::debug!("Task {task_id}: no such task here");
            return Ok(());
        };
        let requester = task.is_requester(self.instance_id());
        let (db, bus) = (self.db(), self.bus());
        match event {
            RelayTaskEvent::Accepted { .. } | RelayTaskEvent::Refused { .. } => {}
            RelayTaskEvent::FailedOver {
                from_instance,
                provider_instance,
                target,
                cause,
                reason_kind,
                ..
            } if requester => {
                let from = member_name(db, &from_instance);
                let to = member_name(db, &provider_instance);
                let mut change = Change::reason(format!(
                    "{from} couldn't take it; the relay gave it to {to}"
                ));
                change.reason_kind = reason_kind;
                change.stage = Some(TaskStage::Dispatch);
                task_state::hand_on(db, bus, &task_id, &target, change).await?;
                let payload = json!({
                    "from": from_instance,
                    "to": provider_instance,
                    "toName": to,
                    "cause": cause,
                    "reasonKind": reason_kind,
                });
                add_message(db, &task_id, FAILED_OVER_KIND, payload).await?;
            }
            RelayTaskEvent::Reassigned {
                from_instance,
                provider_instance,
                target,
                provider_name,
                by,
                note,
                ..
            } if requester => {
                super::reassign::moved(
                    self,
                    &task,
                    super::reassign::Move {
                        from_instance,
                        to_instance: provider_instance,
                        to_card: target,
                        to_name: provider_name,
                        by,
                        reason: note,
                    },
                )
                .await?;
            }
            RelayTaskEvent::Ended {
                state, reason_kind, ..
            } => {
                let error = match state {
                    TaskState::Abandoned => "Nobody took it up by its due date".to_string(),
                    _ if requester => "No result by its due date".to_string(),
                    _ => "The relay gave it to someone else".to_string(),
                };
                let mut change = Change::error(error.clone());
                change.reason_kind = reason_kind;
                if requester {
                    let ended = task_state::advance(db, bus, &task_id, state, change).await?;
                    if ended.is_some() && !self.is_live(&task_id) {
                        let status = if state == TaskState::TimedOut {
                            TaskStatus::TimedOut
                        } else {
                            TaskStatus::Failed
                        };
                        super::history::close(db, bus, &task_id, status, Value::Null, Some(error))
                            .await?;
                    }
                } else if task_state::advance(db, bus, &task_id, state, change)
                    .await?
                    .is_some()
                {
                    self.stop_run(&task_id).await;
                }
            }
            other => log::debug!("Task {task_id}: ignoring {other:?} on this side"),
        }
        Ok(())
    }

    async fn on_peer(self: &Arc<Self>, delivery: PeerDelivery) -> Result<(), String> {
        let task_id = delivery.body.task_id().to_string();
        let Some(task) = get_row(self.db(), &task_id).await? else {
            log::debug!("Task {task_id}: no such task here");
            return Ok(());
        };
        let requester = task.is_requester(self.instance_id());
        let peer = if requester {
            &task.provider_instance
        } else {
            &task.requester_instance
        };
        if delivery.from_instance != *peer {
            log::warn!(
                "Task {task_id}: ignoring a message from {}, not a party to it",
                delivery.from_instance
            );
            return Ok(());
        }
        let (db, bus) = (self.db(), self.bus());
        match delivery.body {
            PeerMessage::AcknowledgeTask {
                eta_seconds, note, ..
            } if requester => {
                let change = Change {
                    reason: Some(note.unwrap_or_else(|| "acknowledged by the provider".into())),
                    eta_seconds,
                    ..Change::default()
                };
                task_state::advance(db, bus, &task_id, TaskState::Acknowledged, change).await?;
            }
            PeerMessage::RejectTask {
                reason,
                reason_kind,
                ..
            } if requester => {
                let change = Change {
                    reason_kind,
                    ..Change::error(reason.clone())
                };
                if let Some(ended) =
                    task_state::advance(db, bus, &task_id, TaskState::Rejected, change).await?
                {
                    // Nobody waits on it (an assignment, held or listed):
                    // its history row is still open, and ending it puts
                    // "declined" (or why it was refused) in the Inbox
                    if !self.is_live(&task_id) {
                        super::history::close(
                            db,
                            bus,
                            &task_id,
                            TaskStatus::Failed,
                            Value::Null,
                            Some(reason),
                        )
                        .await?;
                    }
                    let _ = ended;
                }
            }
            PeerMessage::ProgressUpdate {
                percent,
                note,
                reason_kind,
                queue_position,
                ..
            } if requester => {
                match reason_kind {
                    // It waits at the provider (held for its owner, in
                    // line): it reached them, it isn't being worked on yet
                    Some(kind) => {
                        task_state::advance(
                            db,
                            bus,
                            &task_id,
                            TaskState::Delivered,
                            Change::reason("reached the provider"),
                        )
                        .await?;
                        let mut change =
                            Change::reason(note.clone().unwrap_or_else(|| "waiting".into()));
                        change.reason_kind = Some(kind);
                        change.stage = waiting_stage(kind);
                        task_state::note(db, bus, &task_id, change).await?;
                    }
                    // Progress means the provider is on it (there is no
                    // separate `Working` message)
                    None => {
                        task_state::advance(
                            db,
                            bus,
                            &task_id,
                            TaskState::Working,
                            Change::reason("the provider is working on it"),
                        )
                        .await?;
                    }
                }
                let payload = json!({
                    "percent": percent,
                    "note": note,
                    "reasonKind": reason_kind,
                    "queuePosition": queue_position,
                });
                let message = add_message(db, &task_id, PROGRESS_KIND, payload).await?;
                events::emit(
                    bus,
                    &TaskProgress {
                        task_id: task_id.clone(),
                        percent,
                        note,
                        reason_kind,
                        queue_position,
                        at: message.at,
                    },
                );
            }
            PeerMessage::StatusQuery { .. } if !requester => {
                let report = task.status_report(true);
                self.send(
                    peer,
                    PeerMessage::StatusResponse {
                        task_id: task_id.clone(),
                        state: report.state,
                        eta_seconds: report.eta_seconds,
                        elapsed_seconds: report.elapsed_seconds,
                    },
                )
                .await?;
            }
            PeerMessage::StatusResponse {
                state,
                eta_seconds,
                elapsed_seconds,
                ..
            } if requester => {
                let report = TaskStatusReport {
                    task_id: task_id.clone(),
                    state,
                    eta_seconds,
                    elapsed_seconds,
                    live: true,
                };
                let payload = serde_json::to_value(&report).unwrap_or_default();
                add_message(db, &task_id, STATUS_RESPONSE_KIND, payload).await?;
                self.resolve_status(&task_id, Some(report));
            }
            PeerMessage::CancelTask { reason, .. } => {
                let reason = reason.unwrap_or_else(|| {
                    if requester {
                        "cancelled by the provider".into()
                    } else {
                        "cancelled by the requester".into()
                    }
                });
                let cancelled = task_state::advance(
                    db,
                    bus,
                    &task_id,
                    TaskState::Cancelled,
                    Change::reason(reason),
                )
                .await?;
                if cancelled.is_some() && !requester {
                    self.stop_run(&task_id).await;
                }
            }
            PeerMessage::TaskDelivered { .. } if requester => {
                task_state::advance(
                    db,
                    bus,
                    &task_id,
                    TaskState::Delivered,
                    Change::reason("reached the provider"),
                )
                .await?;
            }
            // Track C's
            PeerMessage::ClarificationRequest(_)
            | PeerMessage::ClarificationResponse(_)
            | PeerMessage::ClarificationTimeout { .. } => {}
            other => log::debug!("Task {task_id}: ignoring {other:?} on this side"),
        }
        Ok(())
    }
}

/// A `ProgressUpdate` for a provider row (recorded locally too)
pub(super) async fn send_progress(
    relay: Option<&Arc<dyn RelayClient>>,
    db: &Db,
    task: &TaskRecord,
    percent: Option<u8>,
    note: Option<String>,
) -> Result<bool, String> {
    let Some(relay) = relay else {
        return Ok(false);
    };
    relay
        .send_peer(
            task.requester_instance.clone(),
            PeerMessage::ProgressUpdate {
                task_id: task.id.clone(),
                percent,
                note: note.clone(),
                reason_kind: None,
                queue_position: None,
            },
        )
        .await?;
    add_message(
        db,
        &task.id,
        PROGRESS_KIND,
        json!({ "percent": percent, "note": note }),
    )
    .await?;
    Ok(true)
}

/// Builds the service, starts the timeout/abandon sweeps, the
/// offline-queue flusher, heartbeats, and restart resume. Must not block.
pub fn init(state: &AppState) -> Option<Arc<TaskService>> {
    let deps = match A2aDeps::from_state(state) {
        Ok(deps) => deps,
        Err(e) => {
            log::error!("The task lifecycle is unavailable: {e}");
            return None;
        }
    };
    let service = TaskService::new(
        deps,
        Some(state.selection().clone()),
        TaskConfig::default(),
        state.runtime(),
    );
    service.register();
    let starting = service.clone();
    state.runtime().spawn(async move {
        starting.listen().await;
        starting.start();
    });
    Some(service)
}

fn service(state: &AppState) -> Result<&Arc<TaskService>, TaskError> {
    state
        .tasks()
        .ok_or_else(|| "The task lifecycle is unavailable".to_string())
}

pub async fn get_task(state: &AppState, task_id: &str) -> Result<TaskRecord, TaskError> {
    get_row(state.db(), task_id)
        .await?
        .ok_or_else(|| format!("Task {task_id} not found"))
}

pub async fn list_tasks(
    state: &AppState,
    filter: TaskFilter,
) -> Result<Vec<TaskRecord>, TaskError> {
    let me = crate::services::network::local_instance_id(state.db())?;
    list_rows(state.db(), &me, filter).await
}

pub async fn cancel_task(
    state: &AppState,
    task_id: &str,
    reason: Option<String>,
) -> Result<TaskRecord, TaskError> {
    service(state)?.cancel(task_id, reason).await
}

/// Sends `StatusQuery` and waits up to 5s for `StatusResponse`
pub async fn query_task_status(
    state: &AppState,
    task_id: &str,
) -> Result<TaskStatusReport, TaskError> {
    service(state)?.query_status(task_id).await
}

pub async fn get_task_events(state: &AppState, task_id: &str) -> Result<Vec<TaskEvent>, TaskError> {
    get_task(state, task_id).await?;
    events_of(state.db(), task_id).await
}

pub async fn get_task_messages(
    state: &AppState,
    task_id: &str,
) -> Result<Vec<TaskMessage>, TaskError> {
    get_task(state, task_id).await?;
    messages_of(state.db(), task_id).await
}

/// `provider_agent` `None`: across every provider of the capability
pub async fn get_eta_estimate(
    state: &AppState,
    capability: &str,
    provider_agent: Option<String>,
) -> Result<EtaEstimate, TaskError> {
    let capability = capability.trim().to_string();
    if capability.is_empty() {
        return Err("Choose a capability".to_string());
    }
    state
        .db()
        .call(move |conn| eta::estimate(conn, &capability, provider_agent.as_deref()))
        .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A requester row `me → them/agent` (summarize)
    pub(crate) fn record(id: &str) -> TaskRecord {
        TaskRecord::requester(
            id,
            "me",
            "writer",
            "them/papers",
            "summarize",
            &json!({"text": "hi"}),
            None,
        )
    }

    pub(crate) async fn store(db: &Db, record: &TaskRecord) {
        let row = record.clone();
        db.call(move |conn| insert(conn, &row)).await.unwrap();
    }

    #[tokio::test]
    async fn rows_round_trip_and_filter() {
        let db = Db::in_memory();
        let mut sent = record("sent");
        sent.created_at = 1.0;
        sent.priority = 3;
        sent.deadline = Some(9.0);
        sent.result = Some(json!({"a": 1}));
        store(&db, &sent).await;
        let mut served = record("served");
        served.requester_instance = "them".into();
        served.requester_agent = "them/writer".into();
        served.provider_instance = "me".into();
        served.provider_agent = "papers".into();
        served.state = TaskState::Working;
        served.created_at = 2.0;
        store(&db, &served).await;

        assert_eq!(get_row(&db, "sent").await.unwrap().unwrap(), sent);
        let list = |filter: TaskFilter| {
            let db = db.clone();
            async move {
                list_rows(&db, "me", filter)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|t| t.id)
                    .collect::<Vec<_>>()
            }
        };
        assert_eq!(list(TaskFilter::default()).await, ["served", "sent"]);
        let role = |r: &str| TaskFilter {
            role: Some(r.into()),
            ..TaskFilter::default()
        };
        assert_eq!(list(role("requester")).await, ["sent"]);
        assert_eq!(list(role("provider")).await, ["served"]);
        let state = TaskFilter {
            state: Some("working".into()),
            ..TaskFilter::default()
        };
        assert_eq!(list(state).await, ["served"]);
        let agent = |a: &str| TaskFilter {
            agent_id: Some(a.into()),
            ..TaskFilter::default()
        };
        assert_eq!(list(agent("writer")).await, ["sent"]);
        assert_eq!(list(agent("papers")).await, ["served", "sent"]);
        let one = TaskFilter {
            limit: 1,
            ..TaskFilter::default()
        };
        assert_eq!(list(one).await.len(), 1);
        assert!(list_rows(&db, "me", role("boss")).await.is_err());
    }

    #[tokio::test]
    async fn internal_messages_are_left_out() {
        let db = Db::in_memory();
        let (bus, _) = crate::services::events::tests::bus();
        store(&db, &record("t")).await;
        add_message(&db, "t", RUN_KIND, json!({"runId": "r"}))
            .await
            .unwrap();
        add_message(&db, "t", PROGRESS_KIND, json!({"note": "half"}))
            .await
            .unwrap();
        let messages = messages_of(&db, "t").await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, PROGRESS_KIND);

        // `create` keeps the wire task
        let created = record("c");
        let task = Task {
            id: "c".into(),
            requester: crate::services::a2a::tests::user_card("me"),
            target: "them/papers".into(),
            capability: "summarize".into(),
            input: json!("x"),
            created_at: 1.0,
            deadline: None,
        };
        create(&db, &bus, &created, Some(&task)).await.unwrap();
        assert_eq!(request_of(&db, "c").await.unwrap(), Some(task));
        let events = events_of(&db, "c").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from_state, None);
        assert_eq!(events[0].to_state, TaskState::Submitted);
    }

    #[test]
    fn input_flags() {
        assert!(wants_progress(&json!({"wants_progress": true})));
        assert!(!wants_progress(&json!({"wants_progress": "yes"})));
        assert!(!wants_progress(&json!("text")));
        assert_eq!(priority_of(&json!({"priority": 7})), 7);
        assert_eq!(priority_of(&json!({})), 0);
    }
}
