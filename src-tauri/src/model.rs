//! Serializable domain types shared by the DB layer, events, and snapshots.
//!
//! These are the shapes the frontend sees (via tauri-specta). Timestamps are
//! milliseconds since the Unix epoch as `f64` (specta can't export `u64`).

use serde::{Deserialize, Serialize};
use specta::Type;

/// The pseudo-agent that owns Studio conversations. `@` can't appear in a
/// real agent ID, so it never collides with an agent folder.
pub const STUDIO_AGENT_ID: &str = "@studio";

pub use nandi_protocol::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    /// Waiting in the agent's queue behind its current turn
    Queued,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Cancelled,
    /// Ended by threshold compaction; a child run continues it
    Compacted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::AwaitingApproval => "awaiting_approval",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Compacted => "compacted",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "queued" => RunStatus::Queued,
            "running" => RunStatus::Running,
            "awaiting_approval" => RunStatus::AwaitingApproval,
            "completed" => RunStatus::Completed,
            "cancelled" => RunStatus::Cancelled,
            "compacted" => RunStatus::Compacted,
            _ => RunStatus::Failed,
        }
    }

    /// The agent's current turn: running or suspended for approval. A
    /// queued run is neither active nor finished.
    pub fn is_active(self) -> bool {
        matches!(self, RunStatus::Running | RunStatus::AwaitingApproval)
    }

    pub fn is_finished(self) -> bool {
        !self.is_active() && self != RunStatus::Queued
    }
}

/// Who asked for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SourceType {
    User,
    Workflow,
    /// Delegated by another agent (A2A)
    Agent,
    Scheduled,
    // Triggers (see `services::triggers`)
    Cron,
    Calendar,
    File,
    Webhook,
    Device,
    Event,
    /// The email check (`services::email`)
    Email,
    /// The calendar check (`services::workspace`)
    CalendarCheck,
}

impl SourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceType::User => "user",
            SourceType::Workflow => "workflow",
            SourceType::Agent => "agent",
            SourceType::Scheduled => "scheduled",
            SourceType::Cron => "cron",
            SourceType::Calendar => "calendar",
            SourceType::File => "file",
            SourceType::Webhook => "webhook",
            SourceType::Device => "device",
            SourceType::Event => "event",
            SourceType::Email => "email",
            SourceType::CalendarCheck => "calendar_check",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "workflow" => SourceType::Workflow,
            "agent" => SourceType::Agent,
            "scheduled" => SourceType::Scheduled,
            "cron" => SourceType::Cron,
            "calendar" => SourceType::Calendar,
            "file" => SourceType::File,
            "webhook" => SourceType::Webhook,
            "device" => SourceType::Device,
            "event" => SourceType::Event,
            "email" => SourceType::Email,
            "calendar_check" => SourceType::CalendarCheck,
            _ => SourceType::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    /// The run ended (e.g. was cancelled) before a decision
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalStatus::Pending => "pending",
            ApprovalStatus::Approved => "approved",
            ApprovalStatus::Rejected => "rejected",
            ApprovalStatus::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "pending" => ApprovalStatus::Pending,
            "approved" => ApprovalStatus::Approved,
            "rejected" => ApprovalStatus::Rejected,
            _ => ApprovalStatus::Expired,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentKind {
    File,
    Image,
}

/// A file attached to a user message, stored under `~/.nandi/attachments/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub id: String,
    pub name: String,
    pub kind: AttachmentKind,
    pub size_bytes: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ToolCallStatus {
    Running,
    Done,
}

/// One entry in a conversation, as rendered by the chat UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatMessage {
    #[serde(rename_all = "camelCase")]
    User {
        id: String,
        run_id: Option<String>,
        created_at: Timestamp,
        content: String,
        attachments: Vec<AttachmentInfo>,
    },
    /// Assistant text; grows as `agent://token` chunks arrive
    #[serde(rename_all = "camelCase")]
    Agent {
        id: String,
        run_id: Option<String>,
        created_at: Timestamp,
        content: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        id: String,
        run_id: Option<String>,
        created_at: Timestamp,
        tool: String,
        args: serde_json::Value,
        status: ToolCallStatus,
        result: Option<String>,
        is_error: bool,
    },
    /// The message ID is also the approval ID
    #[serde(rename_all = "camelCase")]
    Approval {
        id: String,
        run_id: Option<String>,
        created_at: Timestamp,
        tool: String,
        args: serde_json::Value,
        status: ApprovalStatus,
        /// When it expires if nobody decides
        expires_at: Option<Timestamp>,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        id: String,
        run_id: Option<String>,
        created_at: Timestamp,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub status: RunStatus,
    /// The user input that started the run
    pub input: String,
    /// Final agent text (truncated), once completed; the error, once failed
    pub summary: Option<String>,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    /// LLM usage summed over the run's model calls
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// `None` until a model with known pricing is called
    pub cost_usd: Option<f64>,
    /// The latest model call's model, e.g. `deepseek:deepseek-flash`
    pub model: Option<String>,
    pub source_type: SourceType,
    /// The workflow run or delegating agent, if any
    pub source_id: Option<String>,
    /// Human-readable source, e.g. "Daily Report · step 2"
    pub source_label: Option<String>,
    /// Lower runs sooner (see `orchestrator::PRIORITY_*`)
    pub priority: i32,
    /// When the request was queued, if it had to wait
    pub queued_at: Option<Timestamp>,
    /// The run this one continues after threshold compaction
    pub parent_run_id: Option<String>,
    /// The summary this run was started from (compaction)
    pub compaction_summary: Option<String>,
}

/// A run waiting for its agent's current turn to end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub source_type: SourceType,
    pub source_id: Option<String>,
    pub source_label: Option<String>,
    pub priority: i32,
    /// The input text
    pub message: String,
    pub attachments: Vec<AttachmentInfo>,
    pub created_at: Timestamp,
    /// 0 = next to run
    pub position: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub created_at: Timestamp,
    /// When it expires if nobody decides
    pub expires_at: Option<Timestamp>,
}

/// How long a tool approval waits for a decision before it expires and the
/// run goes on without that call (one hour)
pub const APPROVAL_TTL_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AttentionKind {
    Approval,
    Error,
    /// A workflow waits on a manual step (wave EF): `agent_name` is the
    /// workflow's name, `run_id` the workflow run
    ManualStep,
}

/// Something a person needs to act on, shown on the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    /// The approval ID, or the failed run's ID
    pub id: String,
    pub kind: AttentionKind,
    pub agent_id: String,
    pub agent_name: String,
    pub run_id: String,
    /// The chat message to scroll to when opened
    pub message_id: Option<String>,
    pub detail: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ActivityAction {
    Started,
    Completed,
    Failed,
    Cancelled,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// Stable per (run, action) so duplicate deltas are harmless
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub run_id: String,
    pub action: ActivityAction,
    pub subject: String,
    pub created_at: Timestamp,
}

/// A raw persisted event, for the Activity (debug) tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunEventRecord {
    pub id: f64,
    pub run_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StudioConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub definition: serde_json::Value,
    pub created_at: Timestamp,
}
