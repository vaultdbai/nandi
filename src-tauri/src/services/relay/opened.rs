//! Person-to-person traffic as the app sends and receives it: opened, with
//! its content. Nothing here crosses the relay. The relay client seals
//! these for the recipient on the way out and opens them on the way in
//! (`sealing`); on the wire they're `nandi_protocol`'s `PeerMessage`,
//! `SealedTask` and `ResultReport`, with every note, reason, question,
//! answer, input, output and error in an envelope.

use serde::{Deserialize, Serialize};

use crate::services::a2a::{ClarificationRequest, ClarificationResponse, Task, TaskResult};
use nandi_protocol::{DutyDecision, FailoverCause, ReasonKind, TaskState};

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
    /// Provider → requester (live only)
    StatusResponse {
        task_id: String,
        state: TaskState,
        eta_seconds: Option<u32>,
        elapsed_seconds: u32,
    },
    /// Either side
    CancelTask {
        task_id: String,
        reason: Option<String>,
    },
    /// Provider → requester: the task reached the provider
    TaskDelivered { task_id: String },
    /// Provider → requester
    ClarificationRequest(ClarificationRequest),
    /// Requester → provider
    ClarificationResponse(ClarificationResponse),
    /// Requester → provider: nobody answered in time
    ClarificationTimeout { task_id: String },
}

impl PeerMessage {
    /// The task the message is about
    pub fn task_id(&self) -> &str {
        match self {
            Self::AcknowledgeTask { task_id, .. }
            | Self::RejectTask { task_id, .. }
            | Self::ProgressUpdate { task_id, .. }
            | Self::StatusQuery { task_id }
            | Self::StatusResponse { task_id, .. }
            | Self::CancelTask { task_id, .. }
            | Self::TaskDelivered { task_id }
            | Self::ClarificationTimeout { task_id } => task_id,
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

/// A `Peer` message as the receiving client opened it
#[derive(Debug, Clone, PartialEq)]
pub struct PeerDelivery {
    /// Who sent it
    pub from_instance: String,
    pub target_instance: String,
    pub body: PeerMessage,
}

/// A task for us to run, opened
#[derive(Debug, Clone, PartialEq)]
pub struct TaskAssigned {
    pub task_id: String,
    pub provider_instance: String,
    pub task: Task,
}

/// A provider's result, opened
#[derive(Debug, Clone, PartialEq)]
pub struct TaskCompleted {
    pub task_id: String,
    pub result: TaskResult,
}

/// A task of ours failed here, before a result (a request that didn't
/// open, a refusal to seal): the client's own words
#[derive(Debug, Clone, PartialEq)]
pub struct TaskFailed {
    pub task_id: String,
    pub error: String,
}

/// What the relay did with a task we're a party to (opened: a
/// reassignment's note is text)
#[derive(Debug, Clone, PartialEq)]
pub enum TaskEvent {
    /// Ours: the relay took it, with the provider or `held` for them
    Accepted {
        task_id: String,
        provider_instance: String,
        target: String,
        held: bool,
    },
    /// Ours: the relay wouldn't take it
    Refused { task_id: String, message: String },
    /// Ours: given to the next candidate we sealed it for
    FailedOver {
        task_id: String,
        from_instance: String,
        provider_instance: String,
        target: String,
        cause: FailoverCause,
        reason_kind: Option<ReasonKind>,
    },
    /// Ours: its provider (or a manager) handed it on
    Reassigned {
        task_id: String,
        from_instance: String,
        provider_instance: String,
        target: String,
        provider_name: Option<String>,
        by: String,
        note: Option<String>,
    },
    /// Either side: the relay ended it (past its due date, or given to
    /// someone else while we had it)
    Ended {
        task_id: String,
        state: TaskState,
        reason_kind: Option<ReasonKind>,
    },
}

impl TaskEvent {
    pub fn task_id(&self) -> &str {
        match self {
            Self::Accepted { task_id, .. }
            | Self::Refused { task_id, .. }
            | Self::FailedOver { task_id, .. }
            | Self::Reassigned { task_id, .. }
            | Self::Ended { task_id, .. } => task_id,
        }
    }
}

/// The relay asks us to seal `task_id`'s request for `target`: a
/// manager's move of a task we provide, or (as its requester) a move whose
/// provider no longer held the request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResealRequest {
    pub task_id: String,
    pub target: String,
    pub by: String,
}

/// Calendar duty traffic from the relay
#[derive(Debug, Clone, PartialEq)]
pub enum DutyDelivery {
    /// As a primary: a backup watches `duty_id`; how is it going?
    Query {
        duty_id: String,
        backup_instance: String,
    },
    /// As a backup: the relay's call on a duty we watch (a failed run's
    /// error, opened)
    Decision {
        duty_id: String,
        decision: DutyDecision,
        error: Option<String>,
    },
}
