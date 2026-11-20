//! Long-lived per-agent orchestrators.
//!
//! Every agent (plus the Studio pseudo-agent) gets one task for the app's
//! lifetime, fed by an mpsc channel. Navigation never touches these tasks;
//! the UI only sends commands and reads snapshots + events.
//!
//! An agent works on one turn at a time. A request that arrives while it's
//! busy (or while others are already waiting) goes into its queue, a
//! SQLite table ordered by priority then age, so it survives restarts.
//! Whenever the agent goes idle, the orchestrator starts the next queued
//! request. Every change to the queue or current turn is announced as
//! `agent://queue_changed`.
//!
//! A run is a state machine persisted in SQLite, so it can be suspended for
//! approval (even across restarts) and resumed:
//!
//! ```text
//! queued ─▶ running ─▶ engine step (LLM: stream text, then tool calls) ─▶ each planned call:
//!   needs approval, none yet  ─▶ approval row + message ─▶ awaiting_approval (suspend)
//!   approved / no approval    ─▶ tool_call ─▶ MCP in the agent's container ─▶ tool_result
//!   rejected / expired        ─▶ skipped (the model is told on its next step)
//! a step with no tool calls ─▶ completed
//! ```
//!
//! A run suspended for approval is still the agent's current turn: queued
//! requests wait until it's approved, rejected, or stopped.
//!
//! Cancelling aborts the child task, which drops the in-flight LLM stream
//! (closing the HTTP connection) or tool call.
//!
//! The steps of a run execute in an abortable child task, so the
//! orchestrator keeps receiving commands (e.g. Cancel) while a run streams.

mod actor;
mod call_gate;
pub mod compaction;
mod executor;
mod llm_step;
pub mod middleware;
mod registry;
mod run_connectors;
pub mod run_plan;
pub mod runtime_router;
mod tool_calls;
pub mod tool_search;
pub mod verify_on_stop;

#[cfg(test)]
mod connector_tests;
#[cfg(test)]
mod middleware_tests;

#[cfg(test)]
pub(crate) use llm_step::offer_ask_requester;
pub use registry::Orchestrators;
pub(crate) use registry::{default_model, display_name};
// The parts of the run engine see each other as one module did
use self::{actor::*, executor::*, llm_step::*, run_connectors::*, tool_calls::*};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::db::{
    self, role, AssistantMeta, MessageRow, PlannedCall, QueueRow, ToolCallMeta, UserMeta,
};
use crate::engine::{chunk_text, CannedEngine, ToolOutcome, TurnEngine};
use crate::llm::{self, CallSource, LlmRequest, ToolSpec};
use crate::model::{
    ApprovalDecision, ApprovalStatus, AttachmentInfo, RunStatus, RunSummary, SourceType,
    ToolCallStatus, STUDIO_AGENT_ID,
};
use crate::services::a2a::{self, A2ADelegator};
use crate::services::agents::SkillConfig;
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::connectors::{self, ConnectorRegistry};
use crate::services::email;
use crate::services::events::{self, AgentWarning, EventBus, RunContext};
use crate::services::gateway::todos;
use crate::services::history;
use crate::services::inbox;
use crate::services::intake;
use crate::services::network;
use crate::services::pilots::{self, tools::PilotToolset, PilotConsentNeeded};
use crate::services::playbooks;
use crate::services::repo_store::RepoStore;
use crate::services::requests;
use crate::services::runtimes::AgentRuntimes;
use crate::services::secrets::Secrets;
use crate::services::studio;
use call_gate::{CallGate, Clearance, LiveChecks, Verdict};
use middleware::{
    AgentTurnAborted, ExitReason, MiddlewareDecision, ToolResultSummary, TurnContext,
    TurnMiddleware, TurnPolicy, TurnResult,
};

const CHANNEL_CAPACITY: usize = 64;
/// Restarts after a panic before an orchestrator is left down
const MAX_RESTARTS: u32 = 5;
const RESTART_DELAY: Duration = Duration::from_secs(1);
/// What a run that failed inside Nandi (a panic) tells the user
const INTERNAL_ERROR: &str = "The agent hit an internal error.";
// Steps per run are bounded by skill.md `max_turns` (TurnLimitMiddleware)

// Queue priorities: lower runs sooner, so interactive requests jump ahead
// of background work. Ties run oldest first.
pub const PRIORITY_USER: i32 = 10;
pub const PRIORITY_WORKFLOW_CRITICAL: i32 = 30;
pub const PRIORITY_WORKFLOW_NORMAL: i32 = 50;
pub const PRIORITY_AGENT_DELEGATION: i32 = 60;
#[allow(dead_code)] // For scheduled runs, once built
pub const PRIORITY_SCHEDULED: i32 = 80;
/// Also the `runs.priority` column default
#[allow(dead_code)]
pub const PRIORITY_BACKGROUND: i32 = 100;

/// Who asked for a turn, and how urgently.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSource {
    pub source_type: SourceType,
    /// The workflow run or delegating agent
    pub source_id: Option<String>,
    /// Human-readable, e.g. "Daily Report · step 2"
    pub source_label: Option<String>,
    pub priority: i32,
}

impl RunSource {
    /// A message typed by the user
    pub fn user() -> Self {
        Self {
            source_type: SourceType::User,
            source_id: None,
            source_label: None,
            priority: PRIORITY_USER,
        }
    }

    /// A workflow step. Critical steps gate later work, so they go first.
    pub fn workflow(workflow_run_id: &str, label: String, critical: bool) -> Self {
        Self {
            source_type: SourceType::Workflow,
            source_id: Some(workflow_run_id.to_string()),
            source_label: Some(label),
            priority: if critical {
                PRIORITY_WORKFLOW_CRITICAL
            } else {
                PRIORITY_WORKFLOW_NORMAL
            },
        }
    }

    /// A step of a local Request Nandi does itself
    /// (`requests::self_step`): `request_id` is the Request's. Like a
    /// normal workflow step, it waits behind the user's own messages.
    pub fn request_step(request_id: &str, label: String) -> Self {
        Self {
            source_type: SourceType::Request,
            source_id: Some(request_id.to_string()),
            source_label: Some(label),
            priority: PRIORITY_WORKFLOW_NORMAL,
        }
    }

    /// A task delegated by another agent (A2A): `requester` is its
    /// card ID.
    pub fn delegation(requester: &str, label: String) -> Self {
        Self {
            source_type: SourceType::Agent,
            source_id: Some(requester.to_string()),
            source_label: Some(label),
            priority: PRIORITY_AGENT_DELEGATION,
        }
    }
}

// --- A2A delegation -----------------------------------------------

/// The delegator the `delegate_to_agent` tool calls, set by `a2a::start`,
/// and the clarification service `ask_requester` calls,
/// set by `a2a::clarification::start`. Weak: `AppState` owns both, and
/// the delegator holds the orchestrators.
#[derive(Clone, Default)]
pub struct DelegationSlot {
    delegator: Arc<RwLock<Option<Weak<dyn A2ADelegator>>>>,
    clarification: a2a::tool::ClarificationSlot,
}

impl DelegationSlot {
    pub fn set(&self, delegator: &Arc<dyn A2ADelegator>) {
        *self.delegator.write().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::downgrade(delegator));
    }

    pub fn get(&self) -> Option<Arc<dyn A2ADelegator>> {
        self.delegator
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }

    pub fn set_clarifier(&self, service: &Arc<a2a::clarification::ClarificationService>) {
        self.clarification.set(service);
    }

    pub fn clarifier(&self) -> Option<Arc<a2a::clarification::ClarificationService>> {
        self.clarification.get()
    }
}

// -----------------------------------------------------------------------------

/// A request for a turn. The orchestrator persists it, as a run that
/// starts now or a queued one, and replies once that's durable.
#[derive(Debug)]
pub struct TurnRequest {
    pub run_id: String,
    pub conversation_id: String,
    /// The user message's ID (written when the run starts)
    pub message_id: String,
    pub text: String,
    /// The run's input: the text, or the attachment names if it's empty
    pub input: String,
    pub attachments: Vec<AttachmentInfo>,
    pub source: RunSource,
}

type Reply = oneshot::Sender<Result<(), String>>;

/// Commands an orchestrator task processes, one at a time.
#[derive(Debug)]
pub enum OrchestratorCommand {
    /// Start a turn now if the agent is idle, else queue it. Replies
    /// `Ok(true)` if it was queued, `Ok(false)` if it started.
    SendMessage {
        request: Box<TurnRequest>,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    /// Resolve an approval of the current turn
    Approve {
        approval_id: String,
        decision: ApprovalDecision,
    },
    /// Nobody decided in time: close the approval as expired and go on
    /// without that call
    ExpireApproval {
        approval_id: String,
    },
    /// Stop the current turn (never a queued run; see `CancelQueued`)
    Cancel {
        run_id: String,
    },
    /// Remove a queued run
    CancelQueued {
        run_id: String,
        reply: Reply,
    },
    /// Change a queued run's priority
    Reprioritize {
        run_id: String,
        priority: i32,
        reply: Reply,
    },
    Shutdown,
    /// Internal: the run's child task stopped (suspended, done, or failed)
    StepFinished {
        run_id: String,
        outcome: StepOutcome,
    },
    /// Tests: the orchestrator itself panics, as a bug in it would
    #[cfg(test)]
    Crash,
}

#[derive(Debug)]
pub enum StepOutcome {
    /// Completed or suspended for approval; state is already persisted
    Ok,
    /// A middleware (compaction) ended the run and continued the turn in
    /// this child run, which is already persisted as `running`
    Continued(String),
    Failed(String),
    Panicked(String),
}

/// What a run's child task needs. Cheap to clone.
#[derive(Clone)]
pub struct RunDeps {
    pub bus: EventBus,
    pub secrets: Secrets,
    pub runtimes: AgentRuntimes,
    /// Agents' definitions (skill.md, instructions.md)
    pub store: RepoStore,
    /// Runtime connectors; `None` in tests that don't need them
    pub connectors: Option<Arc<ConnectorRegistry>>,
    /// The app, for harness runs; set once the harness services
    /// exist, empty in tests that don't need them
    pub harness: HarnessSlot,
}

/// Late-bound: `RunDeps` is built before the harness services start
pub type HarnessSlot = Arc<std::sync::OnceLock<crate::services::WeakAppState>>;

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

#[cfg(test)]
mod prompt_tests {
    use super::today_prompt;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn the_prompt_says_the_day_and_offset() {
        let ist = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        let now = ist.with_ymd_and_hms(2026, 10, 23, 15, 4, 5).unwrap();
        assert_eq!(
            today_prompt(&now),
            "\n\nToday is Friday, 2026-10-23 (UTC+05:30)."
        );
    }
}

#[cfg(test)]
mod data_run_tests {
    use super::*;

    /// The email and calendar checks are fed untrusted text: they never
    /// get Nandi's browser, which only its chats may use
    /// (`agent_pilot_tools` is merged on the chat branch alone).
    #[test]
    fn a_check_started_run_never_gets_pilot_tools() {
        for tool in ["navigate", "click", "browser_hand_over"] {
            let name = pilots::tools::tool_name("browser", tool);
            for source in [SourceType::Email, SourceType::CalendarCheck] {
                assert!(!data_run_allows(source, &name), "{source:?} {name}");
            }
        }
    }

    /// Their work goes through the intake: no run plan there
    #[test]
    fn a_check_started_run_has_no_plan_tools() {
        for tool in crate::services::run_plan::tool::specs() {
            for source in [SourceType::Email, SourceType::CalendarCheck] {
                assert!(!data_run_allows(source, &tool.name), "{source:?}");
            }
        }
    }
}
