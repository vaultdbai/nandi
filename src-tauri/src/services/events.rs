//! The event bus: every state change is written to SQLite (in one
//! transaction) *before* its event is emitted, so a missed event is never
//! lost — the next snapshot has it.
//!
//! Event names are global (`agent://token`, not `agent://{id}/token`); each
//! payload carries `agentId`/`conversationId`. The frontend registers its
//! listeners once at boot, and Tauri has no wildcard listeners. Every
//! payload is safe to apply twice: messages are upserted by ID and tokens
//! carry the offset they were appended at.
//!
//! The bus knows nothing about transports: it emits to an [`EventSink`].
//! The app fans out to the Tauri webview (see `commands::events`) and to
//! WebSocket subscribers ([`BroadcastSink`]).

use std::sync::Arc;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tokio::sync::broadcast;

use crate::db::{
    self, role, ApprovalMeta, ApprovalRow, AssistantMeta, Db, MessageRow, QueueRow, ToolCallMeta,
    UserMeta,
};
use crate::engine::ToolOutcome;
#[cfg(test)]
use crate::model::ApprovalDecision;
use crate::model::{
    ActivityAction, ActivityItem, ApprovalStatus, AttentionItem, AttentionKind, ChatMessage,
    QueueEntry, RunStatus, RunSummary, ToolCallStatus, STUDIO_AGENT_ID,
};

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

/// A typed event with a stable wire name. The Tauri adapter mirrors these
/// as `tauri_specta::Event`s so the TypeScript bindings stay typed.
pub trait NandiEvent: Serialize {
    const NAME: &'static str;
}

macro_rules! event {
    ($ty:ident, $name:literal) => {
        impl NandiEvent for $ty {
            const NAME: &'static str = $name;
        }
    };
}

/// A run started or changed status.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunUpdated {
    pub agent_id: String,
    pub conversation_id: String,
    pub run: RunSummary,
}
event!(AgentRunUpdated, "agent://run");

/// A user or error message was added.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageAdded {
    pub agent_id: String,
    pub conversation_id: String,
    pub message: ChatMessage,
}
event!(AgentMessageAdded, "agent://message");

/// `text` was appended to agent message `messageId` at `offset` (UTF-16
/// units). A chunk at offset 0 for an unknown message creates it.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentToken {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub offset: u32,
    pub text: String,
    pub created_at: f64,
}
event!(AgentToken, "agent://token");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolCall {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub message: ChatMessage,
}
event!(AgentToolCall, "agent://tool_call");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolResult {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub result: String,
    pub is_error: bool,
}
event!(AgentToolResult, "agent://tool_result");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentApprovalRequest {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub approval_id: String,
    pub message: ChatMessage,
}
event!(AgentApprovalRequest, "agent://approval_request");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentApprovalResolved {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub approval_id: String,
    pub status: ApprovalStatus,
}
event!(AgentApprovalResolved, "agent://approval_resolved");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnDone {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub status: RunStatus,
    /// The user message that started the run
    pub user_message_id: String,
    /// The last agent message, if any
    pub final_message_id: Option<String>,
    pub error: Option<String>,
}
event!(AgentTurnDone, "agent://turn_done");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum DashboardAttention {
    Upsert {
        item: AttentionItem,
    },
    #[serde(rename_all = "camelCase")]
    Remove {
        item_id: String,
    },
}
event!(DashboardAttention, "dashboard://attention");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DashboardActivity {
    pub item: ActivityItem,
}
event!(DashboardActivity, "dashboard://activity");

/// An agent was created or deleted; reload the agent list.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentsChanged {
    pub agent_id: String,
}
event!(AgentsChanged, "agent://agents_changed");

/// An agent's queue or current turn changed: a run was enqueued, started,
/// cancelled, reprioritized, or finished. Carries the whole state, so the
/// latest event always wins. Not recorded.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentQueueChanged {
    pub agent_id: String,
    /// Next to run first
    pub queue: Vec<QueueEntry>,
    /// The running or approval-blocked turn, if any
    pub current: Option<RunSummary>,
}
event!(AgentQueueChanged, "agent://queue_changed");

/// Something degraded that the user should know about, e.g. the agent's
/// container couldn't start so it's answering without tools. Not recorded.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentWarning {
    pub agent_id: String,
    pub message: String,
}
event!(AgentWarning, "agent://warning");

// ---------------------------------------------------------------------------
// Gold copy (`repo://`)
// ---------------------------------------------------------------------------

/// A repo entity was created, updated, or deleted. Sent as
/// `repo://<kind>.<action>` (see `DomainEvent::topic`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoChanged {
    /// agent | workflow | trigger | settings | connectors | manifest
    pub kind: String,
    pub id: String,
    /// created | updated | deleted
    pub action: String,
    pub source_path: String,
    /// user | watcher | git
    pub changed_by: String,
}

/// A repo file failed to load or validate. The last good version (if any)
/// stays live; the projection is unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoValidationFailed {
    pub kind: String,
    pub id: Option<String>,
    pub source_path: Option<String>,
    pub error: String,
    pub changed_by: String,
}
event!(RepoValidationFailed, "repo://validation_failed");

/// The file watcher saw a repo file change and reloaded its entity (or
/// checked its policy).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoFileChanged {
    /// An entity kind (`RepoChanged::kind`), or policy
    pub kind: String,
    pub id: String,
    pub source_path: String,
    /// changed | unchanged | invalid
    pub result: String,
    pub changed_by: String,
}
event!(RepoFileChanged, "repo://file_changed");

impl crate::services::repo_store::EventPublisher for EventBus {
    fn changed(&self, event: &crate::services::domain::DomainEvent) {
        use crate::services::domain::EntityKind;
        let payload = RepoChanged {
            kind: event.kind.as_str().to_string(),
            id: event.id.clone(),
            action: event.action.as_str().to_string(),
            source_path: event.source_path.display().to_string(),
            changed_by: event.changed_by.as_str().to_string(),
        };
        match serde_json::to_value(&payload) {
            Ok(value) => self.sink.emit_json(event.topic(), value),
            Err(e) => log::error!("Failed to serialize {}: {e}", event.topic()),
        }
        // The list-reload events the UI already listens to
        match event.kind {
            EntityKind::Agent => emit(
                self,
                &AgentsChanged {
                    agent_id: event.id.clone(),
                },
            ),
            EntityKind::Workflow => emit(self, &crate::services::workflows::WorkflowsChanged {}),
            EntityKind::Trigger => emit(self, &crate::services::triggers::TriggersChanged {}),
            _ => {}
        }
    }

    fn invalid(
        &self,
        problem: &crate::services::domain::Problem,
        changed_by: crate::services::domain::ChangedBy,
    ) {
        emit(
            self,
            &RepoValidationFailed {
                kind: problem.kind.as_str().to_string(),
                id: problem.id.clone(),
                source_path: problem
                    .source_path
                    .as_ref()
                    .map(|p| p.display().to_string()),
                error: problem.error.to_string(),
                changed_by: changed_by.as_str().to_string(),
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Sinks
// ---------------------------------------------------------------------------

/// Where events go: the Tauri webview, WebSocket clients, a recorder in tests.
pub trait EventSink: Send + Sync + 'static {
    fn emit_json(&self, name: &'static str, payload: Value);
}

/// Delivers every event to each of its sinks.
pub struct FanoutSink(Vec<Arc<dyn EventSink>>);

impl FanoutSink {
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self(sinks)
    }
}

impl EventSink for FanoutSink {
    fn emit_json(&self, name: &'static str, payload: Value) {
        for sink in &self.0 {
            sink.emit_json(name, payload.clone());
        }
    }
}

/// An event as sent to WebSocket clients: `{"event": name, "payload": …}`.
#[derive(Debug, Clone, Serialize)]
pub struct WireEvent {
    pub event: &'static str,
    pub payload: Value,
}

/// How many events a slow WebSocket client may fall behind before it is
/// told to resync from snapshots.
const BROADCAST_CAPACITY: usize = 1024;

/// Fans events out to WebSocket subscribers. With no subscribers, events
/// are simply dropped (the DB already has them).
#[derive(Clone)]
pub struct BroadcastSink(broadcast::Sender<WireEvent>);

impl BroadcastSink {
    pub fn new() -> Self {
        Self(broadcast::channel(BROADCAST_CAPACITY).0)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WireEvent> {
        self.0.subscribe()
    }
}

impl EventSink for BroadcastSink {
    fn emit_json(&self, name: &'static str, payload: Value) {
        // Err only means "no subscribers right now"
        let _ = self.0.send(WireEvent {
            event: name,
            payload,
        });
    }
}

/// Emits a one-off event that isn't tied to a run (and isn't recorded).
pub fn emit<E: NandiEvent>(bus: &EventBus, event: &E) {
    match serde_json::to_value(event) {
        Ok(value) => bus.sink.emit_json(E::NAME, value),
        Err(e) => log::error!("Failed to serialize {}: {e}", E::NAME),
    }
}

// ---------------------------------------------------------------------------
// Bus
// ---------------------------------------------------------------------------

/// Identifies the run an event belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct RunContext {
    pub agent_id: String,
    pub agent_name: String,
    pub conversation_id: String,
    pub run_id: String,
}

/// Events to emit once a transaction commits.
#[derive(Default)]
struct Outbox(Vec<(&'static str, Value)>);

impl Outbox {
    fn push<E: NandiEvent>(&mut self, event: &E) {
        match serde_json::to_value(event) {
            Ok(value) => self.0.push((E::NAME, value)),
            Err(e) => log::error!("Failed to serialize {}: {e}", E::NAME),
        }
    }

    /// Dashboard activity, except for Studio (it isn't an agent).
    fn activity(&mut self, item: ActivityItem) {
        if item.agent_id != STUDIO_AGENT_ID {
            self.push(&DashboardActivity { item });
        }
    }

    /// The agent's queue and current turn as of this transaction.
    fn queue_changed(&mut self, conn: &Connection, agent_id: &str) -> rusqlite::Result<()> {
        self.push(&AgentQueueChanged {
            agent_id: agent_id.to_string(),
            queue: db::list_queue(conn, agent_id)?,
            current: db::active_agent_run(conn, agent_id)?,
        });
        Ok(())
    }

    /// Queues the event and records it in `run_events` (same transaction).
    fn record<E: NandiEvent>(
        &mut self,
        conn: &Connection,
        run_id: &str,
        event: &E,
    ) -> rusqlite::Result<()> {
        if let Ok(value) = serde_json::to_value(event) {
            db::insert_run_event(conn, run_id, E::NAME, &value)?;
            self.0.push((E::NAME, value));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct EventBus {
    pub db: Db,
    sink: Arc<dyn EventSink>,
}

impl EventBus {
    pub fn new(db: Db, sink: Arc<dyn EventSink>) -> Self {
        Self { db, sink }
    }

    /// Runs `write` in a transaction, then emits what it queued.
    async fn commit<T, F>(&self, write: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &mut Outbox) -> rusqlite::Result<T> + Send + 'static,
    {
        let (value, outbox) = self
            .db
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let mut outbox = Outbox::default();
                let value = write(&tx, &mut outbox)?;
                tx.commit()?;
                Ok((value, outbox))
            })
            .await?;
        for (name, payload) in outbox.0 {
            self.sink.emit_json(name, payload);
        }
        Ok(value)
    }

    /// Records `event` in the run's log (`run_events`) and emits it, for
    /// events no other write carries (e.g. a harness's own permission
    /// decisions), so Run Detail can list them after the fact.
    pub async fn record<E: NandiEvent + Send + 'static>(
        &self,
        run_id: &str,
        event: E,
    ) -> Result<(), String> {
        let run_id = run_id.to_string();
        self.commit(move |conn, outbox| outbox.record(conn, &run_id, &event))
            .await
    }

    /// Records a run that starts right away, with its user message (called
    /// by the orchestrator when the agent is idle). Returns `false`, writing
    /// nothing, if the conversation already has an active run.
    pub async fn start_run(
        &self,
        ctx: RunContext,
        run: RunSummary,
        message: MessageRow,
    ) -> Result<bool, String> {
        self.commit(move |conn, outbox| {
            if db::active_run(conn, &run.conversation_id)?.is_some() {
                return Ok(false);
            }
            db::insert_run(conn, &run)?;
            db::insert_message(conn, &message)?;
            outbox.record(
                conn,
                &run.id,
                &AgentRunUpdated {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    run: run.clone(),
                },
            )?;
            outbox.record(
                conn,
                &run.id,
                &AgentMessageAdded {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    message: message.to_chat(),
                },
            )?;
            outbox.activity(activity(&ctx, ActivityAction::Started, &run.input));
            outbox.queue_changed(conn, &ctx.agent_id)?;
            Ok(true)
        })
        .await
    }

    /// Records a run that has to wait (status `queued`) and its queue entry.
    /// The user message is written when the run starts, so it lands in the
    /// conversation after the turn it waited for.
    pub async fn enqueue_run(&self, run: RunSummary, entry: QueueRow) -> Result<(), String> {
        self.commit(move |conn, outbox| {
            db::insert_run(conn, &run)?;
            db::insert_queue_row(conn, &entry)?;
            outbox.record(
                conn,
                &run.id,
                &AgentRunUpdated {
                    agent_id: run.agent_id.clone(),
                    conversation_id: run.conversation_id.clone(),
                    run: run.clone(),
                },
            )?;
            outbox.queue_changed(conn, &run.agent_id)
        })
        .await
    }

    /// Pops the agent's next queued request and starts its run: status
    /// `running`, user message written. Returns the run, or `None` if the
    /// queue is empty.
    pub async fn dequeue_next(
        &self,
        agent_id: &str,
        agent_name: &str,
    ) -> Result<Option<RunSummary>, String> {
        let (agent_id, agent_name) = (agent_id.to_string(), agent_name.to_string());
        self.commit(move |conn, outbox| {
            while let Some(entry) = db::pop_queue(conn, &agent_id)? {
                // A row whose run is gone or no longer queued is stale
                match db::get_run(conn, &entry.run_id)? {
                    Some(run) if run.status == RunStatus::Queued => {}
                    _ => continue,
                }
                let Some(run) = db::start_queued_run(conn, &entry.run_id)? else {
                    continue;
                };
                let ctx = RunContext {
                    agent_id: agent_id.clone(),
                    agent_name: agent_name.clone(),
                    conversation_id: run.conversation_id.clone(),
                    run_id: run.id.clone(),
                };
                let message = MessageRow::new(
                    &entry.message_id,
                    &run.conversation_id,
                    Some(&run.id),
                    role::USER,
                    entry.message,
                    UserMeta {
                        attachments: entry.attachments,
                    },
                );
                db::insert_message(conn, &message)?;
                outbox.record(conn, &run.id, &run_updated(&ctx, run.clone()))?;
                outbox.record(
                    conn,
                    &run.id,
                    &AgentMessageAdded {
                        agent_id: agent_id.clone(),
                        conversation_id: run.conversation_id.clone(),
                        message: message.to_chat(),
                    },
                )?;
                outbox.activity(activity(&ctx, ActivityAction::Started, &run.input));
                outbox.queue_changed(conn, &agent_id)?;
                return Ok(Some(run));
            }
            Ok(None)
        })
        .await
    }

    /// Removes a queued run and marks it cancelled. Returns `false` if it
    /// isn't queued (e.g. it already started).
    pub async fn cancel_queued(&self, agent_name: &str, run_id: &str) -> Result<bool, String> {
        let (agent_name, run_id) = (agent_name.to_string(), run_id.to_string());
        self.commit(move |conn, outbox| {
            let Some(entry) = db::remove_queued(conn, &run_id)? else {
                return Ok(false);
            };
            let Some(run) = db::set_run_status(conn, &run_id, RunStatus::Cancelled, None)? else {
                return Ok(false);
            };
            let ctx = RunContext {
                agent_id: run.agent_id.clone(),
                agent_name,
                conversation_id: run.conversation_id.clone(),
                run_id: run.id.clone(),
            };
            outbox.record(conn, &run.id, &run_updated(&ctx, run.clone()))?;
            // Workflows waiting on this run learn it ended
            outbox.record(
                conn,
                &run.id,
                &AgentTurnDone {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    run_id: run.id.clone(),
                    status: RunStatus::Cancelled,
                    user_message_id: entry.message_id,
                    final_message_id: None,
                    error: None,
                },
            )?;
            outbox.activity(activity(&ctx, ActivityAction::Cancelled, &run.input));
            outbox.queue_changed(conn, &ctx.agent_id)?;
            Ok(true)
        })
        .await
    }

    /// Changes a queued run's priority. Returns `false` if it isn't queued.
    pub async fn reprioritize(&self, run_id: &str, priority: i32) -> Result<bool, String> {
        let run_id = run_id.to_string();
        self.commit(move |conn, outbox| {
            if !db::set_queue_priority(conn, &run_id, priority)? {
                return Ok(false);
            }
            if let Some(run) = db::get_run(conn, &run_id)? {
                outbox.push(&AgentRunUpdated {
                    agent_id: run.agent_id.clone(),
                    conversation_id: run.conversation_id.clone(),
                    run: run.clone(),
                });
                outbox.queue_changed(conn, &run.agent_id)?;
            }
            Ok(true)
        })
        .await
    }

    /// Appends streamed agent text, creating the message on the first chunk.
    pub async fn token(
        &self,
        ctx: &RunContext,
        message_id: &str,
        step: u32,
        text: &str,
    ) -> Result<(), String> {
        let (ctx, message_id, text) = (ctx.clone(), message_id.to_string(), text.to_string());
        self.commit(move |conn, outbox| {
            let created_at = match db::get_message(conn, &message_id)? {
                Some(message) => message.created_at,
                None => {
                    let message = MessageRow::new(
                        &message_id,
                        &ctx.conversation_id,
                        Some(&ctx.run_id),
                        role::ASSISTANT,
                        "",
                        AssistantMeta {
                            step,
                            planned_calls: vec![],
                            reasoning: None,
                        },
                    );
                    db::insert_message(conn, &message)?;
                    message.created_at
                }
            };
            let offset = db::append_message_content(conn, &message_id, &text)?;
            outbox.push(&AgentToken {
                agent_id: ctx.agent_id,
                conversation_id: ctx.conversation_id,
                run_id: ctx.run_id,
                message_id,
                offset: offset as u32,
                text,
                created_at: db::ts(created_at),
            });
            Ok(())
        })
        .await
    }

    /// Records a step's plan (tool calls, reasoning) once its text has
    /// streamed. Creates the message if the step had no text. No event: the
    /// UI learns about the calls from `agent://tool_call`.
    pub async fn finish_step(
        &self,
        ctx: &RunContext,
        message_id: &str,
        meta: AssistantMeta,
    ) -> Result<(), String> {
        let (ctx, message_id) = (ctx.clone(), message_id.to_string());
        self.db
            .call(move |conn| {
                if db::get_message(conn, &message_id)?.is_some() {
                    db::update_message_metadata(conn, &message_id, meta)
                } else {
                    db::insert_message(
                        conn,
                        &MessageRow::new(
                            &message_id,
                            &ctx.conversation_id,
                            Some(&ctx.run_id),
                            role::ASSISTANT,
                            "",
                            meta,
                        ),
                    )
                }
            })
            .await
    }

    pub async fn tool_call(&self, ctx: &RunContext, call: &db::PlannedCall) -> Result<(), String> {
        let (ctx, call) = (ctx.clone(), call.clone());
        self.commit(move |conn, outbox| {
            let message = MessageRow::new(
                &call.id,
                &ctx.conversation_id,
                Some(&ctx.run_id),
                role::TOOL_CALL,
                "",
                ToolCallMeta {
                    tool: call.tool.clone(),
                    args: call.args.clone(),
                    status: ToolCallStatus::Running,
                    result: None,
                    is_error: false,
                },
            );
            db::insert_message(conn, &message)?;
            outbox.record(
                conn,
                &ctx.run_id,
                &AgentToolCall {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    run_id: ctx.run_id.clone(),
                    message: message.to_chat(),
                },
            )
        })
        .await
    }

    pub async fn tool_result(
        &self,
        ctx: &RunContext,
        call: &db::PlannedCall,
        outcome: &ToolOutcome,
    ) -> Result<(), String> {
        let (ctx, call, outcome) = (ctx.clone(), call.clone(), outcome.clone());
        self.commit(move |conn, outbox| {
            db::update_message_metadata(
                conn,
                &call.id,
                ToolCallMeta {
                    tool: call.tool.clone(),
                    args: call.args.clone(),
                    status: ToolCallStatus::Done,
                    result: Some(outcome.result.clone()),
                    is_error: outcome.is_error,
                },
            )?;
            outbox.record(
                conn,
                &ctx.run_id,
                &AgentToolResult {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    run_id: ctx.run_id.clone(),
                    message_id: call.id.clone(),
                    result: outcome.result,
                    is_error: outcome.is_error,
                },
            )
        })
        .await
    }

    /// Suspends the run on an approval: approval row + message + run status.
    /// Returns the approval's id.
    pub async fn approval_request(
        &self,
        ctx: &RunContext,
        call: &db::PlannedCall,
    ) -> Result<String, String> {
        let (ctx, call) = (ctx.clone(), call.clone());
        self.commit(move |conn, outbox| {
            let approval_id = uuid::Uuid::new_v4().to_string();
            let expires_at = Some(db::now_ms() + crate::model::APPROVAL_TTL_MS);
            let message = MessageRow::new(
                &approval_id,
                &ctx.conversation_id,
                Some(&ctx.run_id),
                role::APPROVAL,
                "",
                ApprovalMeta {
                    tool: call.tool.clone(),
                    args: call.args.clone(),
                    status: ApprovalStatus::Pending,
                    expires_at,
                },
            );
            db::insert_message(conn, &message)?;
            db::insert_approval(
                conn,
                &ApprovalRow {
                    id: approval_id.clone(),
                    agent_id: ctx.agent_id.clone(),
                    run_id: ctx.run_id.clone(),
                    message_id: approval_id.clone(),
                    tool_call_id: call.id.clone(),
                    tool: call.tool.clone(),
                    args: call.args.clone(),
                    status: ApprovalStatus::Pending,
                    created_at: message.created_at,
                    expires_at,
                    decided_by: None,
                },
            )?;
            let run = db::set_run_status(conn, &ctx.run_id, RunStatus::AwaitingApproval, None)?;

            outbox.record(
                conn,
                &ctx.run_id,
                &AgentApprovalRequest {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    run_id: ctx.run_id.clone(),
                    approval_id: approval_id.clone(),
                    message: message.to_chat(),
                },
            )?;
            if let Some(run) = run {
                outbox.record(conn, &ctx.run_id, &run_updated(&ctx, run))?;
            }
            outbox.push(&DashboardAttention::Upsert {
                item: approval_attention(
                    &ctx,
                    &approval_id,
                    &call.tool,
                    &call.args,
                    message.created_at,
                ),
            });
            Ok(approval_id)
        })
        .await
    }

    /// Resolves a pending approval and puts the run back to `running`.
    /// A decision as `approval_closed` takes it (tests decide this way; the
    /// orchestrator maps decisions itself)
    #[cfg(test)]
    pub async fn approval_resolved(
        &self,
        ctx: &RunContext,
        approval: ApprovalRow,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let status = match decision {
            ApprovalDecision::Approved => ApprovalStatus::Approved,
            ApprovalDecision::Rejected => ApprovalStatus::Rejected,
        };
        self.approval_closed(ctx, approval, status).await
    }

    /// Closes a pending approval as approved, rejected, or expired (nobody
    /// decided in time) and puts the run back to `running`; the model reads
    /// the outcome when it replans.
    pub async fn approval_closed(
        &self,
        ctx: &RunContext,
        approval: ApprovalRow,
        status: ApprovalStatus,
    ) -> Result<(), String> {
        let ctx = ctx.clone();
        let decision = serde_json::json!({
            "ts": db::now_ms(),
            "agent_id": ctx.agent_id,
            "run_id": ctx.run_id,
            "approval_id": approval.id,
            "tool": approval.tool,
            "outcome": status.as_str(),
            // Set by `chat::approve_action` before the decision closes it
            "decided_by": approval.decided_by,
        });
        let closed = self
            .commit(move |conn, outbox| {
                db::set_approval_status(conn, &approval, status)?;
                let run = db::set_run_status(conn, &ctx.run_id, RunStatus::Running, None)?;
                outbox.record(
                    conn,
                    &ctx.run_id,
                    &AgentApprovalResolved {
                        agent_id: ctx.agent_id.clone(),
                        conversation_id: ctx.conversation_id.clone(),
                        run_id: ctx.run_id.clone(),
                        approval_id: approval.id.clone(),
                        status,
                    },
                )?;
                if let Some(run) = run {
                    outbox.record(conn, &ctx.run_id, &run_updated(&ctx, run))?;
                }
                outbox.push(&DashboardAttention::Remove {
                    item_id: approval.id.clone(),
                });
                let action = match status {
                    ApprovalStatus::Approved => Some(ActivityAction::Approved),
                    ApprovalStatus::Rejected => Some(ActivityAction::Rejected),
                    _ => None,
                };
                if let Some(action) = action {
                    let mut item = activity(&ctx, action, &approval.tool);
                    item.id = format!("{}:{}", approval.id, status.as_str());
                    outbox.activity(item);
                }
                Ok(())
            })
            .await;
        if closed.is_ok() {
            crate::audit::record(crate::audit::AuditLog::Decisions, &decision).await;
        }
        closed
    }

    /// Ends a run. Failure adds an error message and an attention item;
    /// any approvals still pending expire.
    pub async fn turn_done(
        &self,
        ctx: &RunContext,
        status: RunStatus,
        error: Option<String>,
    ) -> Result<(), String> {
        let ctx = ctx.clone();
        self.commit(move |conn, outbox| {
            for approval in db::pending_approvals_for_run(conn, &ctx.run_id)? {
                db::set_approval_status(conn, &approval, ApprovalStatus::Expired)?;
                outbox.record(
                    conn,
                    &ctx.run_id,
                    &AgentApprovalResolved {
                        agent_id: ctx.agent_id.clone(),
                        conversation_id: ctx.conversation_id.clone(),
                        run_id: ctx.run_id.clone(),
                        approval_id: approval.id.clone(),
                        status: ApprovalStatus::Expired,
                    },
                )?;
                outbox.push(&DashboardAttention::Remove {
                    item_id: approval.id,
                });
            }

            let messages = db::list_run_messages(conn, &ctx.run_id)?;

            // Close tool calls the run was in the middle of
            for message in messages.iter().filter(|m| m.role == role::TOOL_CALL) {
                let Ok(meta) = serde_json::from_value::<ToolCallMeta>(message.metadata.clone())
                else {
                    continue;
                };
                if meta.status != ToolCallStatus::Running {
                    continue;
                }
                let result = "Stopped before it finished.".to_string();
                db::update_message_metadata(
                    conn,
                    &message.id,
                    ToolCallMeta {
                        status: ToolCallStatus::Done,
                        result: Some(result.clone()),
                        is_error: true,
                        ..meta
                    },
                )?;
                outbox.record(
                    conn,
                    &ctx.run_id,
                    &AgentToolResult {
                        agent_id: ctx.agent_id.clone(),
                        conversation_id: ctx.conversation_id.clone(),
                        run_id: ctx.run_id.clone(),
                        message_id: message.id.clone(),
                        result,
                        is_error: true,
                    },
                )?;
            }

            let user_message_id = messages
                .iter()
                .find(|m| m.role == role::USER)
                .map(|m| m.id.clone())
                .unwrap_or_default();
            let final_message = messages.iter().rev().find(|m| m.role == role::ASSISTANT);

            let summary = match (&error, final_message) {
                (Some(error), _) => Some(error.clone()),
                (None, Some(message)) if status == RunStatus::Completed => {
                    Some(truncate(&message.content, 280))
                }
                _ => None,
            };
            let run = db::set_run_status(conn, &ctx.run_id, status, summary.as_deref())?;

            if let Some(error) = &error {
                let message = MessageRow::new(
                    format!("error-{}", ctx.run_id),
                    &ctx.conversation_id,
                    Some(&ctx.run_id),
                    role::ERROR,
                    error.clone(),
                    serde_json::json!({}),
                );
                db::insert_message(conn, &message)?;
                outbox.record(
                    conn,
                    &ctx.run_id,
                    &AgentMessageAdded {
                        agent_id: ctx.agent_id.clone(),
                        conversation_id: ctx.conversation_id.clone(),
                        message: message.to_chat(),
                    },
                )?;
                if let Some(run) = &run {
                    outbox.push(&DashboardAttention::Upsert {
                        item: error_attention(&ctx, run, Some(message.id)),
                    });
                }
            }
            if let Some(run) = run.clone() {
                outbox.record(conn, &ctx.run_id, &run_updated(&ctx, run))?;
            }
            outbox.record(
                conn,
                &ctx.run_id,
                &AgentTurnDone {
                    agent_id: ctx.agent_id.clone(),
                    conversation_id: ctx.conversation_id.clone(),
                    run_id: ctx.run_id.clone(),
                    status,
                    user_message_id,
                    final_message_id: final_message.map(|m| m.id.clone()),
                    error: error.clone(),
                },
            )?;

            let action = match status {
                RunStatus::Completed => ActivityAction::Completed,
                RunStatus::Cancelled => ActivityAction::Cancelled,
                _ => ActivityAction::Failed,
            };
            let subject = run.as_ref().map(|r| r.input.clone()).unwrap_or_default();
            outbox.activity(activity(&ctx, action, &subject));
            outbox.queue_changed(conn, &ctx.agent_id)
        })
        .await
    }

    /// Re-announces pending approvals (e.g. after a restart). Idempotent.
    pub async fn reannounce_approvals(
        &self,
        agent_id: &str,
        agent_name: &str,
    ) -> Result<usize, String> {
        let (agent_id, agent_name) = (agent_id.to_string(), agent_name.to_string());
        self.commit(move |conn, outbox| {
            let pending = db::pending_approvals(conn, Some(&agent_id))?;
            for approval in &pending {
                let ctx = RunContext {
                    agent_id: agent_id.clone(),
                    agent_name: agent_name.clone(),
                    conversation_id: approval.conversation_id.clone(),
                    run_id: approval.run_id.clone(),
                };
                outbox.push(&DashboardAttention::Upsert {
                    item: approval_attention(
                        &ctx,
                        &approval.id,
                        &approval.tool,
                        &approval.args,
                        approval.created_at as i64,
                    ),
                });
            }
            Ok(pending.len())
        })
        .await
    }
}

fn run_updated(ctx: &RunContext, run: RunSummary) -> AgentRunUpdated {
    AgentRunUpdated {
        agent_id: ctx.agent_id.clone(),
        conversation_id: ctx.conversation_id.clone(),
        run,
    }
}

fn activity(ctx: &RunContext, action: ActivityAction, subject: &str) -> ActivityItem {
    let action_key = serde_json::to_value(action)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    ActivityItem {
        id: format!("{}:{action_key}", ctx.run_id),
        agent_id: ctx.agent_id.clone(),
        agent_name: ctx.agent_name.clone(),
        run_id: ctx.run_id.clone(),
        action,
        subject: truncate(subject, 120),
        created_at: db::ts(db::now_ms()),
    }
}

pub fn approval_attention(
    ctx: &RunContext,
    approval_id: &str,
    tool: &str,
    args: &Value,
    created_at: i64,
) -> AttentionItem {
    AttentionItem {
        id: approval_id.to_string(),
        kind: AttentionKind::Approval,
        agent_id: ctx.agent_id.clone(),
        agent_name: ctx.agent_name.clone(),
        run_id: ctx.run_id.clone(),
        message_id: Some(approval_id.to_string()),
        detail: format!("{tool}: {}", truncate(&args.to_string(), 200)),
        created_at: db::ts(created_at),
    }
}

pub fn error_attention(
    ctx: &RunContext,
    run: &RunSummary,
    message_id: Option<String>,
) -> AttentionItem {
    AttentionItem {
        id: run.id.clone(),
        kind: AttentionKind::Error,
        agent_id: ctx.agent_id.clone(),
        agent_name: ctx.agent_name.clone(),
        run_id: run.id.clone(),
        message_id,
        detail: run.summary.clone().unwrap_or_default(),
        created_at: run.ended_at.unwrap_or(run.started_at),
    }
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records emitted events for assertions.
    #[derive(Default)]
    pub struct RecordingSink(pub Mutex<Vec<(&'static str, Value)>>);

    impl EventSink for RecordingSink {
        fn emit_json(&self, name: &'static str, payload: Value) {
            self.0.lock().unwrap().push((name, payload));
        }
    }

    impl RecordingSink {
        pub fn names(&self) -> Vec<&'static str> {
            self.0.lock().unwrap().iter().map(|(n, _)| *n).collect()
        }

        pub fn take(&self) -> Vec<(&'static str, Value)> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    pub fn bus() -> (EventBus, Arc<RecordingSink>) {
        let sink = Arc::new(RecordingSink::default());
        (EventBus::new(Db::in_memory(), sink.clone()), sink)
    }

    pub fn ctx(run_id: &str) -> RunContext {
        RunContext {
            agent_id: "writer".into(),
            agent_name: "Writer".into(),
            conversation_id: db::agent_conversation_id("writer"),
            run_id: run_id.into(),
        }
    }

    pub async fn started(bus: &EventBus, run_id: &str) -> RunContext {
        let ctx = ctx(run_id);
        let conversation = bus
            .db
            .call(|conn| db::ensure_agent_conversation(conn, "writer"))
            .await
            .unwrap();
        let run = RunSummary {
            id: run_id.into(),
            agent_id: "writer".into(),
            conversation_id: conversation.clone(),
            status: RunStatus::Running,
            input: "draft".into(),
            summary: None,
            started_at: db::ts(db::now_ms()),
            ended_at: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
            model: None,
            source_type: crate::model::SourceType::User,
            source_id: None,
            source_label: None,
            priority: 10,
            queued_at: None,
            parent_run_id: None,
            compaction_summary: None,
        };
        let message = MessageRow::new(
            format!("user-{run_id}"),
            &conversation,
            Some(run_id),
            role::USER,
            "draft",
            serde_json::json!({}),
        );
        assert!(bus.start_run(ctx.clone(), run, message).await.unwrap());
        ctx
    }

    #[tokio::test]
    async fn writes_before_emitting_and_records_run_events() {
        let (bus, sink) = bus();
        let ctx = started(&bus, "r1").await;

        assert_eq!(
            sink.names(),
            [
                "agent://run",
                "agent://message",
                "dashboard://activity",
                "agent://queue_changed"
            ]
        );
        let kinds = bus
            .db
            .call(|conn| db::list_agent_events(conn, "writer", 10))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["agent://run", "agent://message"]);
        assert_eq!(ctx.run_id, "r1");
    }

    #[tokio::test]
    async fn tokens_create_then_append_with_offsets() {
        let (bus, sink) = bus();
        let ctx = started(&bus, "r1").await;
        sink.take();

        bus.token(&ctx, "m1", 0, "Hello ").await.unwrap();
        bus.token(&ctx, "m1", 0, "world").await.unwrap();

        let events = sink.take();
        assert_eq!(events[0].1["offset"], 0);
        assert_eq!(events[1].1["offset"], 6);
        let message = bus
            .db
            .call(|conn| db::get_message(conn, "m1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.content, "Hello world");
        assert_eq!(message.run_id.as_deref(), Some("r1"));
    }

    #[tokio::test]
    async fn approval_round_trip_updates_run_and_dashboard() {
        let (bus, sink) = bus();
        let ctx = started(&bus, "r1").await;
        let call = db::PlannedCall {
            id: "c1".into(),
            tool: "shell".into(),
            args: serde_json::json!({"cmd": "ls"}),
            requires_approval: true,
        };
        sink.take();

        bus.approval_request(&ctx, &call).await.unwrap();
        let events = sink.take();
        assert_eq!(
            events.iter().map(|e| e.0).collect::<Vec<_>>(),
            [
                "agent://approval_request",
                "agent://run",
                "dashboard://attention"
            ]
        );
        assert_eq!(events[1].1["run"]["status"], "awaitingApproval");
        assert_eq!(events[2].1["op"], "upsert");

        let approval = bus
            .db
            .call(|conn| db::approval_for_call(conn, "c1"))
            .await
            .unwrap()
            .unwrap();
        bus.approval_resolved(&ctx, approval.clone(), ApprovalDecision::Approved)
            .await
            .unwrap();
        let events = sink.take();
        assert_eq!(events[0].1["status"], "approved");
        assert_eq!(events[1].1["run"]["status"], "running");
        assert_eq!(
            events[2].1,
            serde_json::json!({"op": "remove", "itemId": approval.id})
        );
    }

    #[tokio::test]
    async fn failed_turn_adds_error_and_expires_approvals() {
        let (bus, sink) = bus();
        let ctx = started(&bus, "r1").await;
        let call = db::PlannedCall {
            id: "c1".into(),
            tool: "shell".into(),
            args: serde_json::json!({}),
            requires_approval: true,
        };
        bus.approval_request(&ctx, &call).await.unwrap();
        sink.take();

        bus.turn_done(&ctx, RunStatus::Cancelled, None)
            .await
            .unwrap();
        let names = sink.names();
        assert!(names.contains(&"agent://approval_resolved"));
        assert!(names.contains(&"agent://turn_done"));
        let pending = bus
            .db
            .call(|conn| db::pending_approvals(conn, None))
            .await
            .unwrap();
        assert!(pending.is_empty());

        let ctx2 = started(&bus, "r2").await;
        sink.take();
        bus.turn_done(&ctx2, RunStatus::Failed, Some("boom".into()))
            .await
            .unwrap();
        let events = sink.take();
        let done = events.iter().find(|e| e.0 == "agent://turn_done").unwrap();
        assert_eq!(done.1["status"], "failed");
        assert_eq!(done.1["userMessageId"], "user-r2");
        assert!(events
            .iter()
            .any(|e| e.0 == "dashboard://attention" && e.1["item"]["kind"] == "error"));
        let run = bus
            .db
            .call(|conn| db::get_run(conn, "r2"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.summary.as_deref(), Some("boom"));
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "hé…");
        assert_eq!(truncate("hi", 5), "hi");
    }
}
