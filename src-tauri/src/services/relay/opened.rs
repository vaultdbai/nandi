//! Person-to-person traffic as the app sends and receives it: opened, with
//! its content. Nothing here crosses the relay. The relay client seals
//! these for the recipient on the way out and opens them on the way in
//! (`sealing`); on the wire they're `nandi_protocol`'s `PeerMessage`,
//! `SealedTask` and `ResultReport`, with every note, reason, question,
//! answer, input, output and error in an envelope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::services::a2a::{ClarificationRequest, ClarificationResponse, Task, TaskResult};
use nandi_protocol::{Envelope, ReasonKind, TaskState};

/// Task lifecycle and clarification traffic between requester and
/// provider, tagged by `kind` (the opened form of the wire's
/// `PeerMessage`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PeerMessage {
    /// Provider → requester: accepted, with an estimate
    AcknowledgeTask {
        task_id: String,
        eta_seconds: Option<u32>,
        note: Option<String>,
    },
    /// Provider → requester: refused (policy, trust, capacity)
    RejectTask {
        task_id: String,
        reason: String,
        /// Why, as the requester's UI matches it to a fix
        reason_kind: Option<ReasonKind>,
    },
    /// Provider → requester, only when the task asked for progress (a hold
    /// or a place in line is always sent)
    ProgressUpdate {
        task_id: String,
        percent: Option<u8>,
        note: Option<String>,
        /// Why it waits, when it waits (held for approval, queued)
        reason_kind: Option<ReasonKind>,
        /// 1-based place in the provider's line, while queued
        queue_position: Option<u32>,
    },
    /// Requester → provider
    StatusQuery { task_id: String },
    /// Provider → requester
    StatusResponse {
        task_id: String,
        state: TaskState,
        eta_seconds: Option<u32>,
        elapsed_seconds: u32,
        /// Once it ended: its result, so a requester that was offline when
        /// it was sent still gets it (a lasting assignment)
        result: Option<Value>,
        error: Option<String>,
    },
    /// Either side
    CancelTask {
        task_id: String,
        reason: Option<String>,
    },
    /// Provider → requester: the task reached the provider
    TaskDelivered { task_id: String },
    /// Requester → provider: gave up waiting
    TaskTimedOut {
        task_id: String,
        reason: Option<String>,
    },
    /// Provider → requester
    ClarificationRequest(ClarificationRequest),
    /// Requester → provider
    ClarificationResponse(ClarificationResponse),
    /// Requester → provider: nobody answered in time
    ClarificationTimeout { task_id: String },
    /// Provider → requester: send this task to `new_provider_card` instead;
    /// the provider already let it go
    ReassignTask {
        task_id: String,
        new_provider_instance: String,
        new_provider_card: String,
        /// The new provider's display name, for the requester's thread
        new_provider_name: Option<String>,
        /// Who reassigned it (display name)
        by: String,
        reason: Option<String>,
        /// Received: the request, resealed by the provider for the new
        /// provider (the requester sends it on). Sent: leave it `None`;
        /// the relay client reseals the request it opened.
        request: Option<Box<Envelope>>,
    },
    /// A calendar trigger's backup → its primary: how is this duty going?
    /// (`duty_id` is `<trigger>/<date>/<time>`, see `domain::calendar`)
    DutyQuery { duty_id: String },
    /// Primary → backup
    DutyStatus { duty_id: String, state: DutyState },
}

impl PeerMessage {
    /// The task the message is about (a duty's ID for duty messages)
    pub fn task_id(&self) -> &str {
        match self {
            Self::AcknowledgeTask { task_id, .. }
            | Self::RejectTask { task_id, .. }
            | Self::ProgressUpdate { task_id, .. }
            | Self::StatusQuery { task_id }
            | Self::StatusResponse { task_id, .. }
            | Self::CancelTask { task_id, .. }
            | Self::TaskDelivered { task_id }
            | Self::TaskTimedOut { task_id, .. }
            | Self::ClarificationTimeout { task_id }
            | Self::ReassignTask { task_id, .. } => task_id,
            Self::DutyQuery { duty_id } | Self::DutyStatus { duty_id, .. } => duty_id,
            Self::ClarificationRequest(request) => &request.task_id,
            Self::ClarificationResponse(response) => &response.task_id,
        }
    }
}

/// How a calendar trigger's primary is doing with one scheduled fire, as
/// its own Nandi reports it (the opened form: a failure keeps its error)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DutyState {
    /// Not due yet on the primary's clock (epoch ms)
    Upcoming {
        due_at: i64,
    },
    /// Dispatched; its run hasn't finished (epoch ms)
    Running {
        since: i64,
    },
    Completed,
    Failed {
        error: Option<String>,
    },
    /// Due, but no run was started (trigger off, rate limited, or missing)
    NotRun,
}

/// A `Peer` (or `Undeliverable`) message as the receiving client opened it
#[derive(Debug, Clone, PartialEq)]
pub struct PeerDelivery {
    /// Who sent it (for `Undeliverable`: us)
    pub from_instance: String,
    pub target_instance: String,
    pub body: PeerMessage,
    /// The relay couldn't reach `target_instance`; `body` is ours, returned
    /// (its content can't be opened again, so it's left out)
    pub undeliverable: bool,
}

/// A task reached a provider, opened. The provider's handler runs it; the
/// requester's (no task) only learns where it went.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskAssigned {
    pub task_id: String,
    pub provider_instance: String,
    pub task: Option<Task>,
}

/// A provider's result, opened
#[derive(Debug, Clone, PartialEq)]
pub struct TaskCompleted {
    pub task_id: String,
    pub result: TaskResult,
}
