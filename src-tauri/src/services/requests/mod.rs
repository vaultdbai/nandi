//! Requests, phase 1 (work-model §4): one unit of work with its Steps,
//! projected from the task tables as they are today, with no schema change.
//! A Request is a root task (the incoming gateway task, or the first
//! attempt of an outgoing delegation) plus its gateway child
//! (`<id>-gw`, see `a2a::executor::gateway_child_id`), its failover
//! attempts (`previous_attempt_id`), their `_run` / `progress` /
//! `workflow_steps` messages and the child runs. Tasks are read directly,
//! not through `WorkItem`, which hides incoming and child tasks.
//!
//! [`project`] is pure; [`get_request`] loads what it needs.

pub mod blocker;
pub mod local;

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::db::{self, Db};
use crate::model::{RunStatus, RunSummary, Timestamp};
use crate::services::a2a::executor::gateway_child_id;
use crate::services::a2a::task::{
    self, agent_of, ReasonKind, TaskEvent, TaskRecord, TaskStage, TaskState, PROGRESS_KIND,
    RUN_KIND,
};
use crate::services::capability::workflow_provider::WORKFLOW_STEPS_KIND;
use crate::services::{network, AppState};

use blocker::{BlockerInput, BlockerMatch};
pub use blocker::{BlockerKind, BlockerWho, Direction};

/// Longest title, in characters
const MAX_TITLE_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RequestStatus {
    Planning,
    Running,
    Waiting,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    /// Receive / allow / route / deliver: drawn small, not numbered
    System,
    Agent,
    Workflow,
    /// Another Nandi does it
    Peer,
    /// An approval, manual step or answer
    Person,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Pending,
    Running,
    Waiting,
    Done,
    Failed,
    Skipped,
}

/// Which system step a row is; the UI names it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SystemStage {
    /// Incoming: the request arrived
    Received,
    /// Incoming: trust and policy let it through
    Allowed,
    /// Incoming: it waited for a free slot (only if it did)
    Queued,
    /// Incoming: it waited for the owner's approval (only if it did)
    Approval,
    /// Incoming: the gateway chose who does it
    Routed,
    /// Incoming: the result went back
    Delivered,
    /// Outgoing: this Nandi sent it
    Sent,
    /// Outgoing: it reached the provider's Nandi
    DeliveredTo,
    /// Outgoing: the provider took it on
    Accepted,
    /// Outgoing: the result came back
    Done,
}

/// What does a step's work, for the [›] links
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StepLink {
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub workflow_id: Option<String>,
    pub workflow_run_id: Option<String>,
}

/// One try at an outgoing task (failover makes more)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub task_id: String,
    pub attempt_number: u32,
    /// The provider's display name
    pub provider: String,
    pub state: TaskState,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    /// Why it ended badly (raw; shown under Details)
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestStep {
    pub id: String,
    pub kind: StepKind,
    /// Set on system steps
    pub stage: Option<SystemStage>,
    /// A name: the agent, workflow step, or member (plan steps)
    pub label: String,
    pub status: StepStatus,
    pub link: StepLink,
    /// Live note: "Step 'x' done (2/3)", the provider's progress
    pub progress: Option<String>,
    /// Plain detail from the system: the routing decision, the ETA
    pub detail: Option<String>,
    pub eta_seconds: Option<u32>,
    pub attempts: Vec<Attempt>,
    /// A workflow's steps
    pub children: Vec<RequestStep>,
    pub started_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
}

/// Why a Request is stuck, who acts, and what the fix button needs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub kind: BlockerKind,
    pub who: BlockerWho,
    /// It ended; nothing moves it on by itself
    pub terminal: bool,
    /// The step it belongs to
    pub step_id: Option<String>,
    /// Since when (epoch ms)
    pub since: Timestamp,
    pub deadline: Option<Timestamp>,
    /// The other side's display name
    pub member: Option<String>,
    /// The other side's device, for Trust / Resume
    pub member_instance: Option<String>,
    /// The local agent involved (Turn on, approvals)
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub run_id: Option<String>,
    pub approval_id: Option<String>,
    /// The task Cancel / Retry act on
    pub task_id: String,
    /// The raw reason, for Details only
    pub reason: Option<String>,
    /// 1-based place in line, while queued
    pub queue_position: Option<u32>,
}

/// A row of the Activity tab
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestActivity {
    pub at: Timestamp,
    pub task_id: String,
    /// `state` (a transition) or the message kind
    pub kind: String,
    pub from_state: Option<TaskState>,
    pub to_state: Option<TaskState>,
    pub reason: Option<String>,
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub id: String,
    pub title: String,
    /// What was asked, word for word: the text and every other input, as sent
    pub input: Value,
    pub capability: String,
    pub direction: Direction,
    /// The other side's display name (incoming: who asked; outgoing: who does it)
    pub counterpart: Option<String>,
    pub counterpart_instance: Option<String>,
    pub status: RequestStatus,
    pub stuck: Option<Blocker>,
    /// Other steps waiting besides the banner's ("+1 more waiting")
    pub more_waiting: u32,
    pub steps: Vec<RequestStep>,
    /// Plan steps (not system ones)
    pub plan_steps: u32,
    /// The plan step it's on, 1-based (0: none yet)
    pub current_step: u32,
    pub result: Option<Value>,
    pub error: Option<String>,
    /// 1–5, once the requester rated it
    pub rating: Option<u32>,
    pub eta_seconds: Option<u32>,
    pub deadline: Option<Timestamp>,
    pub started_at: Timestamp,
    pub updated_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    /// Every task row in it (root, child, attempts)
    pub task_ids: Vec<String>,
    pub requester_instance: String,
    pub provider_instance: String,
    /// Summed over the local child runs
    pub cost_usd: Option<f64>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub activity: Vec<RequestActivity>,
}

// ---------------------------------------------------------------------------
// Projection (pure)
// ---------------------------------------------------------------------------

/// A message of any kind (internal ones too)
#[derive(Debug, Clone, PartialEq)]
pub struct RawMessage {
    pub kind: String,
    pub payload: Value,
    pub at: Timestamp,
}

/// One task row with its history
#[derive(Debug, Clone, PartialEq)]
pub struct TaskBundle {
    pub task: TaskRecord,
    pub events: Vec<TaskEvent>,
    pub messages: Vec<RawMessage>,
}

impl TaskBundle {
    fn latest(&self, kind: &str) -> Option<&RawMessage> {
        self.messages.iter().rev().find(|m| m.kind == kind)
    }

    fn run_id(&self) -> Option<String> {
        self.latest(RUN_KIND)
            .and_then(|m| m.payload.get("runId"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    fn progress(&self) -> Option<String> {
        self.latest(PROGRESS_KIND)
            .and_then(|m| m.payload.get("note"))
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
    }

    /// The reason of the newest transition that has one
    fn last_reason(&self) -> Option<&str> {
        self.events.iter().rev().find_map(|e| e.reason.as_deref())
    }

    /// What the task says now (task 5): the kind on its newest event, if
    /// that event is about its current state (a waiting note, or the
    /// transition that ended it)
    fn current_kind(&self) -> Option<ReasonKind> {
        let last = self.events.last()?;
        (last.to_state == self.task.state)
            .then_some(last.reason_kind)
            .flatten()
    }

    /// It waits now at `stage` (a note with that stage is the newest event)
    fn waiting_at(&self, stage: TaskStage) -> bool {
        self.events.last().is_some_and(|e| {
            e.stage == Some(stage)
                && e.from_state == Some(e.to_state)
                && e.to_state == self.task.state
                && e.reason_kind.is_some()
                && !self.task.state.is_terminal()
        })
    }

    /// When it first reached `stage`, if it ever did
    fn stage_at(&self, stage: TaskStage) -> Option<Timestamp> {
        self.events
            .iter()
            .find(|e| e.stage == Some(stage))
            .map(|e| e.at)
    }

    /// The stage of the transition that rejected it
    fn rejected_at(&self) -> Option<TaskStage> {
        self.events
            .iter()
            .rev()
            .find(|e| e.to_state == TaskState::Rejected)
            .and_then(|e| e.stage)
    }

    /// Its place in the provider's line, from the newest progress message
    fn queue_position(&self) -> Option<u32> {
        self.latest(PROGRESS_KIND)
            .and_then(|m| m.payload.get("queuePosition"))
            .and_then(Value::as_u64)
            .map(|p| p as u32)
    }

    /// The workflow run the gateway started for it (`_workflow_run`)
    fn workflow_run_id(&self) -> Option<String> {
        self.latest(crate::services::gateway::router::WORKFLOW_RUN_KIND)
            .and_then(|m| m.payload.get("runId"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    fn reached(&self, state: TaskState) -> bool {
        self.task.state == state || self.events.iter().any(|e| e.to_state == state)
    }

    /// When it first reached `state`
    fn reached_at(&self, state: TaskState) -> Option<Timestamp> {
        self.events
            .iter()
            .find(|e| e.to_state == state)
            .map(|e| e.at)
    }

    /// The routing reason the gateway acknowledged with (stage `route`)
    fn routing(&self) -> Option<&str> {
        self.events
            .iter()
            .filter(|e| e.to_state == TaskState::Acknowledged && e.stage == Some(TaskStage::Route))
            .find_map(|e| e.reason.as_deref())
    }
}

/// Everything [`project`] reads
#[derive(Debug, Clone, Default)]
pub struct ProjectionInput {
    /// This Nandi's instance ID
    pub me: String,
    /// The root and its later attempts, oldest first (one row for incoming)
    pub attempts: Vec<TaskBundle>,
    /// The gateway's child task (its latest attempt)
    pub child: Option<TaskBundle>,
    pub runs: HashMap<String, RunSummary>,
    /// Instance ID → display name
    pub members: HashMap<String, String>,
    /// Local agent ID → name
    pub agents: HashMap<String, String>,
    /// Pending approval per run
    pub approvals: HashMap<String, String>,
    pub pending_clarification: bool,
    pub provider_online: Option<bool>,
    pub rating: Option<u32>,
    /// When the owner's approval of a held request runs out
    pub hold_expires_at: Option<Timestamp>,
}

/// The first line of the ask, or the capability in words
pub fn title_of(input: &Value, capability: &str) -> String {
    let text = match input {
        Value::String(text) => Some(text.as_str()),
        Value::Object(fields) => [
            "text",
            "request",
            "natural_language_request",
            "prompt",
            "query",
            "message",
            "input",
        ]
        .iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str)),
        _ => None,
    };
    let line = text
        .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(|line| {
            if line.chars().count() > MAX_TITLE_CHARS {
                let cut: String = line.chars().take(MAX_TITLE_CHARS - 1).collect();
                format!("{}…", cut.trim_end())
            } else {
                line.to_string()
            }
        });
    line.unwrap_or_else(|| humanize(capability))
}

/// `finance.risk-review` → `Finance risk review`
pub fn humanize(id: &str) -> String {
    let words: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => id.to_string(),
    }
}

fn step_status_of(state: TaskState) -> StepStatus {
    match state {
        TaskState::Submitted | TaskState::Delivered => StepStatus::Pending,
        TaskState::Acknowledged | TaskState::Working => StepStatus::Running,
        TaskState::InputRequired => StepStatus::Waiting,
        TaskState::Completed => StepStatus::Done,
        TaskState::Failed | TaskState::TimedOut | TaskState::Rejected | TaskState::Abandoned => {
            StepStatus::Failed
        }
        TaskState::Cancelled | TaskState::Reassigned => StepStatus::Skipped,
    }
}

fn run_step_status(run: &RunSummary) -> StepStatus {
    match run.status {
        RunStatus::Queued => StepStatus::Pending,
        RunStatus::Running | RunStatus::Compacted => StepStatus::Running,
        RunStatus::AwaitingApproval => StepStatus::Waiting,
        RunStatus::Completed => StepStatus::Done,
        RunStatus::Failed => StepStatus::Failed,
        RunStatus::Cancelled => StepStatus::Skipped,
    }
}

fn system(id: &str, stage: SystemStage, status: StepStatus, at: Option<Timestamp>) -> RequestStep {
    RequestStep {
        id: id.to_string(),
        kind: StepKind::System,
        stage: Some(stage),
        label: String::new(),
        status,
        link: StepLink::default(),
        progress: None,
        detail: None,
        eta_seconds: None,
        attempts: Vec::new(),
        children: Vec::new(),
        started_at: at,
        ended_at: at,
    }
}

impl ProjectionInput {
    fn member(&self, instance: &str) -> String {
        self.members
            .get(instance)
            .cloned()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| instance.to_string())
    }

    fn agent_name(&self, card_or_id: &str) -> String {
        let id = agent_of(card_or_id);
        self.agents.get(id).cloned().unwrap_or_else(|| humanize(id))
    }

    /// An agent step for the run a task's turn left (`_run`)
    fn agent_step(&self, id: &str, bundle: &TaskBundle) -> RequestStep {
        let run_id = bundle.run_id();
        let run = run_id.as_ref().and_then(|r| self.runs.get(r));
        RequestStep {
            id: id.to_string(),
            kind: StepKind::Agent,
            stage: None,
            label: self.agent_name(&bundle.task.provider_agent),
            status: run
                .map(run_step_status)
                .unwrap_or_else(|| step_status_of(bundle.task.state)),
            link: StepLink {
                agent_id: Some(agent_of(&bundle.task.provider_agent).to_string()),
                run_id: run_id.clone(),
                task_id: Some(bundle.task.id.clone()),
                ..Default::default()
            },
            progress: bundle.progress(),
            detail: None,
            eta_seconds: bundle.task.eta_seconds,
            attempts: Vec::new(),
            children: self.workflow_children(bundle),
            started_at: run.map(|r| r.started_at).or(Some(bundle.task.created_at)),
            ended_at: run.and_then(|r| r.ended_at).or(bundle.task.completed_at),
        }
    }

    /// A workflow's steps, from the `workflow_steps` breakdown (written when
    /// the run ends)
    fn workflow_children(&self, bundle: &TaskBundle) -> Vec<RequestStep> {
        let Some(breakdown) = bundle.latest(WORKFLOW_STEPS_KIND) else {
            return Vec::new();
        };
        let run_id = breakdown.payload["runId"].as_str().map(str::to_string);
        let workflow_id = breakdown.payload["workflowId"].as_str().map(str::to_string);
        breakdown.payload["steps"]
            .as_array()
            .map(|steps| {
                steps
                    .iter()
                    .enumerate()
                    .map(|(n, step)| {
                        let step_id = step["stepId"].as_str().unwrap_or_default();
                        RequestStep {
                            id: format!("wf-{n}"),
                            kind: StepKind::Agent,
                            stage: None,
                            label: humanize(step_id),
                            status: match step["status"].as_str() {
                                Some("done" | "completed") => StepStatus::Done,
                                Some("failed" | "rejected") => StepStatus::Failed,
                                Some("running") => StepStatus::Running,
                                Some("skipped") => StepStatus::Skipped,
                                Some(s) if s.starts_with("awaiting") => StepStatus::Waiting,
                                _ => StepStatus::Pending,
                            },
                            link: StepLink {
                                agent_id: step["agentId"].as_str().map(str::to_string),
                                workflow_id: workflow_id.clone(),
                                workflow_run_id: run_id.clone(),
                                ..Default::default()
                            },
                            progress: None,
                            detail: None,
                            eta_seconds: None,
                            attempts: Vec::new(),
                            children: Vec::new(),
                            started_at: None,
                            ended_at: None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn attempts(&self) -> Vec<Attempt> {
        self.attempts
            .iter()
            .map(|a| Attempt {
                task_id: a.task.id.clone(),
                attempt_number: a.task.attempt_number,
                provider: self.member(&a.task.provider_instance),
                state: a.task.state,
                started_at: a.task.created_at,
                ended_at: a.task.completed_at,
                error: a.task.error.clone(),
            })
            .collect()
    }
}

fn direction_of(task: &TaskRecord, me: &str) -> Direction {
    match (task.requester_instance == me, task.provider_instance == me) {
        (true, false) => Direction::Outgoing,
        (false, true) => Direction::Incoming,
        _ => Direction::Local,
    }
}

/// The Request a set of tasks makes up
pub fn project(input: &ProjectionInput) -> Option<Request> {
    let root = input.attempts.first()?;
    let latest = input.attempts.last()?;
    let direction = direction_of(&root.task, &input.me);
    let mut steps = Vec::new();

    // The run that did the work, for the blocker and costs
    let worker = match direction {
        Direction::Incoming => input.child.as_ref().unwrap_or(root),
        _ => latest,
    };
    let worker_run = worker.run_id().and_then(|id| input.runs.get(&id));

    let (counterpart_instance, plan_step_id) = match direction {
        Direction::Incoming => {
            let ended = root.task.state.is_terminal();
            // Refused by trust or policy; a declined hold, or a wait for a
            // slot that ran out, got past Allowed first
            let rejected_late = matches!(
                root.rejected_at(),
                Some(TaskStage::Approval | TaskStage::Slot)
            );
            let refused = root.task.state == TaskState::Rejected
                && root.task.acknowledged_at.is_none()
                && !rejected_late;
            steps.push(system(
                "received",
                SystemStage::Received,
                StepStatus::Done,
                Some(root.task.created_at),
            ));
            let queued_at = root.stage_at(TaskStage::Slot);
            let held_at = root.stage_at(TaskStage::Approval);
            let allowed_at = root
                .reached_at(TaskState::Acknowledged)
                .or_else(|| root.reached_at(TaskState::Working))
                .or(held_at)
                .or(queued_at);
            steps.push(system(
                "allowed",
                SystemStage::Allowed,
                if refused {
                    StepStatus::Failed
                } else if allowed_at.is_some() || ended {
                    StepStatus::Done
                } else {
                    StepStatus::Running
                },
                allowed_at,
            ));
            // Only when it happened: waited for the owner, then for a slot
            for (id, stage, system_stage, at) in [
                (
                    "approval",
                    TaskStage::Approval,
                    SystemStage::Approval,
                    held_at,
                ),
                ("queued", TaskStage::Slot, SystemStage::Queued, queued_at),
            ] {
                if at.is_none() {
                    continue;
                }
                let status = if root.waiting_at(stage) {
                    StepStatus::Waiting
                } else if root.task.state == TaskState::Rejected
                    && root.rejected_at() == Some(stage)
                {
                    StepStatus::Failed
                } else if ended && root.task.acknowledged_at.is_none() {
                    StepStatus::Skipped
                } else {
                    StepStatus::Done
                };
                steps.push(system(id, system_stage, status, at));
            }
            let mut plan_id = None;
            if !refused && !rejected_late {
                if let Some(routing) = root.routing() {
                    let mut routed = system(
                        "routed",
                        SystemStage::Routed,
                        StepStatus::Done,
                        root.reached_at(TaskState::Acknowledged),
                    );
                    routed.detail = Some(routing.to_string());
                    steps.push(routed);
                }
                let plan = match &input.child {
                    Some(child) if child.task.provider_instance == input.me => {
                        Some(input.agent_step("step-1", child))
                    }
                    // Passed on to another Nandi's gateway
                    Some(child) => Some(RequestStep {
                        kind: StepKind::Peer,
                        label: input.member(&child.task.provider_instance),
                        attempts: vec![Attempt {
                            task_id: child.task.id.clone(),
                            attempt_number: child.task.attempt_number,
                            provider: input.member(&child.task.provider_instance),
                            state: child.task.state,
                            started_at: child.task.created_at,
                            ended_at: child.task.completed_at,
                            error: child.task.error.clone(),
                        }],
                        ..input.agent_step("step-1", child)
                    }),
                    None if root.run_id().is_some() => Some(input.agent_step("step-1", root)),
                    None => root
                        .routing()
                        .and_then(|r| r.strip_prefix("Running the workflow "))
                        .map(|rest| RequestStep {
                            kind: StepKind::Workflow,
                            label: rest.split(':').next().unwrap_or(rest).trim().to_string(),
                            status: step_status_of(root.task.state),
                            children: input.workflow_children(root),
                            // The run, recorded as it started (task 5); its
                            // steps' progress live, stored on this side
                            link: StepLink {
                                task_id: Some(root.task.id.clone()),
                                workflow_run_id: root.workflow_run_id(),
                                ..Default::default()
                            },
                            progress: root.progress(),
                            ..input.agent_step("step-1", root)
                        }),
                };
                if let Some(plan) = plan {
                    plan_id = Some(plan.id.clone());
                    steps.push(plan);
                }
                steps.push(system(
                    "delivered",
                    SystemStage::Delivered,
                    match root.task.state {
                        TaskState::Completed => StepStatus::Done,
                        s if s.is_terminal() => StepStatus::Failed,
                        _ => StepStatus::Pending,
                    },
                    root.task.completed_at,
                ));
            }
            (
                root.task.requester_instance.clone(),
                plan_id.or(Some("allowed".to_string())),
            )
        }
        Direction::Outgoing => {
            steps.push(system(
                "sent",
                SystemStage::Sent,
                StepStatus::Done,
                Some(root.task.created_at),
            ));
            let delivered_at = latest.reached_at(TaskState::Delivered);
            let delivered = latest.reached(TaskState::Delivered)
                || latest.task.acknowledged_at.is_some()
                || latest.task.state == TaskState::Completed;
            let mut delivered_to = system(
                "delivered-to",
                SystemStage::DeliveredTo,
                if delivered {
                    StepStatus::Done
                } else if latest.task.state.is_terminal() {
                    StepStatus::Failed
                } else {
                    StepStatus::Waiting
                },
                delivered_at,
            );
            delivered_to.label = input.member(&latest.task.provider_instance);
            steps.push(delivered_to);
            // Held for their owner, or in their line (task 5)
            let waiting_there = latest.task.acknowledged_at.is_none()
                && !latest.task.state.is_terminal()
                && matches!(
                    latest.current_kind(),
                    Some(ReasonKind::NeedsTheirApproval | ReasonKind::Queued)
                );
            let mut accepted = system(
                "accepted",
                SystemStage::Accepted,
                if latest.task.acknowledged_at.is_some() {
                    StepStatus::Done
                } else if latest.task.state == TaskState::Rejected {
                    StepStatus::Failed
                } else if waiting_there {
                    StepStatus::Waiting
                } else {
                    StepStatus::Pending
                },
                latest.task.acknowledged_at,
            );
            accepted.eta_seconds = latest.task.eta_seconds;
            steps.push(accepted);
            // The provider's internals collapse into one Working step
            steps.push(RequestStep {
                id: "step-1".to_string(),
                kind: StepKind::Peer,
                stage: None,
                label: input.member(&latest.task.provider_instance),
                status: step_status_of(latest.task.state),
                link: StepLink {
                    task_id: Some(latest.task.id.clone()),
                    ..Default::default()
                },
                progress: latest.progress(),
                detail: None,
                eta_seconds: latest.task.eta_seconds,
                attempts: input.attempts(),
                children: input.workflow_children(latest),
                started_at: latest.task.acknowledged_at.or(Some(root.task.created_at)),
                ended_at: latest.task.completed_at,
            });
            steps.push(system(
                "done",
                SystemStage::Done,
                match latest.task.state {
                    TaskState::Completed => StepStatus::Done,
                    s if s.is_terminal() => StepStatus::Failed,
                    _ => StepStatus::Pending,
                },
                latest.task.completed_at,
            ));
            (
                latest.task.provider_instance.clone(),
                Some("step-1".to_string()),
            )
        }
        Direction::Local => {
            steps.push(input.agent_step("step-1", latest));
            (String::new(), Some("step-1".to_string()))
        }
    };

    let effective = match direction {
        Direction::Outgoing | Direction::Local => latest,
        Direction::Incoming => root,
    };
    let reason = effective
        .task
        .error
        .as_deref()
        .or_else(|| effective.last_reason());
    let matched: Option<BlockerMatch> = blocker::blocker(&BlockerInput {
        direction: Some(direction),
        state: Some(effective.task.state),
        reason,
        run_status: worker_run.map(|r| r.status),
        run_summary: worker_run.and_then(|r| r.summary.as_deref()),
        workflow_step_awaiting_approval: false,
        pending_clarification: input.pending_clarification,
        provider_offline: direction == Direction::Outgoing && input.provider_online == Some(false),
        reason_kind: effective.current_kind(),
    });
    // A gateway hold or a place in line on this side (incoming)
    let held_here = direction == Direction::Incoming && root.waiting_at(TaskStage::Approval);
    let queued_here = direction == Direction::Incoming && root.waiting_at(TaskStage::Slot);

    let counterpart =
        (!counterpart_instance.is_empty()).then(|| input.member(&counterpart_instance));
    let local_agent = match direction {
        Direction::Incoming => Some(
            input
                .child
                .as_ref()
                .map(|c| c.task.provider_agent.clone())
                .unwrap_or_else(|| root.task.provider_agent.clone()),
        ),
        _ => Some(latest.task.provider_agent.clone()),
    }
    .map(|a| agent_of(&a).to_string())
    .filter(|a| !a.is_empty() && a != crate::services::gateway::GATEWAY_AGENT_ID);

    let stuck = matched.map(|m| {
        let step_id = match m.kind {
            BlockerKind::NotTrusted | BlockerKind::NotAccepting | BlockerKind::PausedOrBlocked => {
                Some(
                    if direction == Direction::Outgoing {
                        "delivered-to"
                    } else {
                        "allowed"
                    }
                    .to_string(),
                )
            }
            BlockerKind::Offline => Some("delivered-to".to_string()),
            BlockerKind::NeedsMyApproval if held_here => Some("approval".to_string()),
            BlockerKind::Queued if queued_here => Some("queued".to_string()),
            BlockerKind::NeedsTheirApproval | BlockerKind::Queued
                if direction == Direction::Outgoing =>
            {
                Some("accepted".to_string())
            }
            BlockerKind::Declined if direction == Direction::Incoming => {
                Some("approval".to_string())
            }
            BlockerKind::Declined => Some("accepted".to_string()),
            _ => plan_step_id.clone(),
        };
        let run_id = worker_run.map(|r| r.id.clone());
        Blocker {
            kind: m.kind,
            who: m.who,
            terminal: m.terminal,
            step_id,
            since: effective
                .events
                .last()
                .map(|e| e.at)
                .unwrap_or(effective.task.updated_at),
            deadline: match m.kind {
                BlockerKind::NeedsMyAnswer => effective.task.deadline,
                BlockerKind::NeedsMyApproval if held_here => input.hold_expires_at,
                _ => None,
            },
            member: counterpart.clone(),
            member_instance: (!counterpart_instance.is_empty())
                .then(|| counterpart_instance.clone()),
            agent_name: local_agent.as_ref().map(|a| input.agent_name(a)),
            agent_id: local_agent.clone(),
            approval_id: run_id
                .as_ref()
                .and_then(|r| input.approvals.get(r).cloned()),
            run_id,
            task_id: effective.task.id.clone(),
            reason: reason.map(str::to_string),
            queue_position: (m.kind == BlockerKind::Queued)
                .then(|| effective.queue_position())
                .flatten(),
        }
    });

    let status = match effective.task.state {
        TaskState::Completed => RequestStatus::Done,
        TaskState::Cancelled => RequestStatus::Cancelled,
        s if s.is_terminal() => RequestStatus::Failed,
        _ if stuck.as_ref().is_some_and(|b| !b.terminal) => RequestStatus::Waiting,
        TaskState::InputRequired => RequestStatus::Waiting,
        _ => RequestStatus::Running,
    };

    let plan: Vec<&RequestStep> = steps
        .iter()
        .filter(|s| s.kind != StepKind::System)
        .collect();
    let plan_steps = plan.len() as u32;
    let current_step = plan
        .iter()
        .position(|s| s.status != StepStatus::Done)
        .map(|i| i as u32 + 1)
        .unwrap_or(plan_steps);
    let waiting = steps
        .iter()
        .filter(|s| s.status == StepStatus::Waiting)
        .count() as u32;
    // The banner shows one; the rest say so on their rows
    let more_waiting = if stuck.is_some() {
        waiting.saturating_sub(1)
    } else {
        0
    };

    let mut bundles: Vec<&TaskBundle> = input.attempts.iter().collect();
    if let Some(child) = &input.child {
        bundles.push(child);
    }
    let mut activity: Vec<RequestActivity> = bundles
        .iter()
        .flat_map(|b| {
            b.events
                .iter()
                .map(|e| RequestActivity {
                    at: e.at,
                    task_id: e.task_id.clone(),
                    kind: "state".to_string(),
                    from_state: e.from_state,
                    to_state: Some(e.to_state),
                    reason: e.reason.clone(),
                    payload: None,
                })
                .chain(
                    b.messages
                        .iter()
                        .filter(|m| m.kind != task::REQUEST_KIND)
                        .map(|m| RequestActivity {
                            at: m.at,
                            task_id: b.task.id.clone(),
                            kind: m.kind.clone(),
                            from_state: None,
                            to_state: None,
                            reason: None,
                            payload: Some(m.payload.clone()),
                        }),
                )
        })
        .collect();
    activity.sort_by(|a, b| a.at.total_cmp(&b.at));

    let local_runs: Vec<&RunSummary> = bundles
        .iter()
        .filter_map(|b| b.run_id())
        .filter_map(|id| input.runs.get(&id))
        .collect();
    let costs: Vec<f64> = local_runs.iter().filter_map(|r| r.cost_usd).collect();

    Some(Request {
        id: root.task.id.clone(),
        title: title_of(&root.task.input, &root.task.capability),
        input: root.task.input.clone(),
        capability: root.task.capability.clone(),
        direction,
        counterpart,
        counterpart_instance: (!counterpart_instance.is_empty()).then_some(counterpart_instance),
        status,
        stuck,
        more_waiting,
        steps,
        plan_steps,
        current_step,
        result: effective.task.result.clone(),
        error: effective.task.error.clone(),
        rating: input.rating,
        eta_seconds: effective.task.eta_seconds,
        deadline: effective.task.deadline,
        started_at: root.task.created_at,
        updated_at: bundles
            .iter()
            .map(|b| b.task.updated_at)
            .fold(root.task.updated_at, f64::max),
        ended_at: effective.task.state.is_terminal().then(|| {
            effective
                .task
                .completed_at
                .unwrap_or(effective.task.updated_at)
        }),
        task_ids: bundles.iter().map(|b| b.task.id.clone()).collect(),
        requester_instance: root.task.requester_instance.clone(),
        provider_instance: effective.task.provider_instance.clone(),
        cost_usd: (!costs.is_empty()).then(|| costs.iter().sum()),
        input_tokens: local_runs.iter().map(|r| r.input_tokens).sum(),
        output_tokens: local_runs.iter().map(|r| r.output_tokens).sum(),
        activity,
    })
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn messages_all(conn: &Connection, task_id: &str) -> rusqlite::Result<Vec<RawMessage>> {
    let mut stmt =
        conn.prepare("SELECT kind, payload, at FROM task_messages WHERE task_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([task_id], |row| {
        let payload: String = row.get(1)?;
        Ok(RawMessage {
            kind: row.get(0)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            at: row.get::<_, i64>(2)? as f64,
        })
    })?;
    rows.collect()
}

fn events_all(conn: &Connection, task_id: &str) -> rusqlite::Result<Vec<TaskEvent>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, from_state, to_state, reason, reason_kind, stage, at FROM task_events
          WHERE task_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([task_id], |row| {
        let from: Option<String> = row.get(1)?;
        let to: String = row.get(2)?;
        let kind: Option<String> = row.get(4)?;
        let stage: Option<String> = row.get(5)?;
        Ok(TaskEvent {
            task_id: row.get(0)?,
            from_state: from.as_deref().and_then(TaskState::parse),
            to_state: TaskState::parse(&to).unwrap_or(TaskState::Failed),
            reason: row.get(3)?,
            reason_kind: kind.as_deref().and_then(task::ReasonKind::parse),
            stage: stage.as_deref().and_then(task::TaskStage::parse),
            at: row.get::<_, i64>(6)? as f64,
        })
    })?;
    rows.collect()
}

fn bundle(conn: &Connection, task: TaskRecord) -> rusqlite::Result<TaskBundle> {
    Ok(TaskBundle {
        events: events_all(conn, &task.id)?,
        messages: messages_all(conn, &task.id)?,
        task,
    })
}

/// The root a task ID belongs to: its gateway task (for `<id>-gw`) and the
/// first attempt of its failover chain
pub fn root_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let Some(mut task) = task::load(conn, id)? else {
        return Ok(None);
    };
    // A gateway child belongs to the gateway task
    if let Some(parent) = task
        .parent_task_id
        .clone()
        .or_else(|| task.id.strip_suffix("-gw").map(str::to_string))
    {
        if let Some(parent) = task::load(conn, &parent)? {
            task = parent;
        }
    }
    while let Some(previous) = task.previous_attempt_id.clone() {
        match task::load(conn, &previous)? {
            Some(earlier) => task = earlier,
            None => break,
        }
    }
    Ok(Some(task.id))
}

/// The chain starting at `root`, oldest first
fn chain(conn: &Connection, root: TaskRecord) -> rusqlite::Result<Vec<TaskRecord>> {
    let mut chain = vec![root];
    for _ in 0..16 {
        let last = &chain[chain.len() - 1].id;
        let next = task::select(
            conn,
            "previous_attempt_id = ?1 ORDER BY created_at",
            &[last],
        )?;
        match next.into_iter().next() {
            Some(next) => chain.push(next),
            None => break,
        }
    }
    Ok(chain)
}

fn rating_of(conn: &Connection, ids: &[String]) -> Option<u32> {
    ids.iter().find_map(|id| {
        conn.query_row(
            "SELECT user_rating FROM capability_evidence WHERE task_id = ?1 AND user_rating IS NOT NULL",
            params![id],
            |row| row.get::<_, u32>(0),
        )
        .optional()
        .ok()
        .flatten()
    })
}

/// Everything about the Request `id` (any of its task IDs)
pub async fn load(db: &Db, id: &str) -> Result<Option<ProjectionInput>, String> {
    let id = id.to_string();
    let me = network::local_instance_id(db)?;
    let members: HashMap<String, String> = network::known_members(db)
        .into_iter()
        .map(|m| (m.instance_id, m.display_name))
        .collect();
    db.call(move |conn| {
        let Some(root) = root_id(conn, &id)? else {
            return Ok(None);
        };
        let Some(root) = task::load(conn, &root)? else {
            return Ok(None);
        };
        let child_root = task::load(conn, &gateway_child_id(&root.id))?;
        let attempts = chain(conn, root)?
            .into_iter()
            .map(|t| bundle(conn, t))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let child = match child_root {
            Some(child) => {
                let latest = chain(conn, child)?.pop();
                latest.map(|t| bundle(conn, t)).transpose()?
            }
            None => None,
        };
        let mut runs = HashMap::new();
        let mut approvals = HashMap::new();
        for b in attempts.iter().chain(child.iter()) {
            if let Some(run_id) = b.run_id() {
                if let Some(run) = db::get_run(conn, &run_id)? {
                    if let Some(approval) = db::pending_approvals_for_run(conn, &run_id)?
                        .into_iter()
                        .next()
                    {
                        approvals.insert(run_id.clone(), approval.id);
                    }
                    runs.insert(run_id, run);
                }
            }
        }
        let ids: Vec<String> = attempts.iter().map(|b| b.task.id.clone()).collect();
        let hold_expires_at = match attempts.first() {
            Some(root) => crate::services::gateway::holds::get(conn, &root.task.id)?
                .filter(|hold| hold.status == crate::services::gateway::holds::HoldStatus::Pending)
                .map(|hold| hold.expires_at),
            None => None,
        };
        Ok(Some(ProjectionInput {
            me,
            rating: rating_of(conn, &ids),
            attempts,
            child,
            runs,
            members,
            agents: HashMap::new(),
            approvals,
            pending_clarification: false,
            provider_online: None,
            hold_expires_at,
        }))
    })
    .await
}

pub async fn get_request(state: &AppState, id: String) -> Result<Request, String> {
    // A local ask (task 14) is stored as itself
    let lookup = id.clone();
    if let Some(request) = state
        .db()
        .call(move |conn| local::load(conn, &lookup))
        .await?
    {
        return Ok(local::to_request(state, request).await);
    }
    let Some(mut input) = load(state.db(), &id).await? else {
        return Err(format!("Request {id} not found"));
    };
    input.agents = state.store().read(|repo| {
        repo.agents()
            .map(|a| (a.id().to_string(), a.name().to_string()))
            .collect()
    });
    let task_ids: Vec<String> = input.attempts.iter().map(|b| b.task.id.clone()).collect();
    if let Some(clarifications) = state.clarifications() {
        input.pending_clarification = clarifications
            .pending()
            .iter()
            .any(|c| task_ids.contains(&c.task_id));
    }
    if let (Some(latest), Some(tasks)) = (input.attempts.last(), state.tasks()) {
        if direction_of(&latest.task, &input.me) == Direction::Outgoing
            && latest.task.state == TaskState::Submitted
        {
            input.provider_online = Some(tasks.is_online(&latest.task.provider_instance).await);
        }
    }
    project(&input).ok_or_else(|| format!("Request {id} not found"))
}

/// The Request a run belongs to (a delegated turn's `_run`, or a local
/// ask's step), if any
pub async fn request_for_run(state: &AppState, run_id: String) -> Result<Option<String>, String> {
    state
        .db()
        .call(move |conn| {
            let task_id: Option<String> = conn
                .query_row(
                    "SELECT task_id FROM task_messages
                      WHERE kind = ?1 AND json_extract(payload, '$.runId') = ?2
                      ORDER BY id DESC LIMIT 1",
                    params![RUN_KIND, run_id],
                    |row| row.get(0),
                )
                .optional()?;
            match task_id {
                Some(task_id) => root_id(conn, &task_id),
                // A step of a local ask (task 14)
                None => local::request_of(conn, &run_id),
            }
        })
        .await
}

/// A new attempt for a failed outgoing task, on the next provider, linked
/// through `previous_attempt_id` (failover). Errors when there's none.
pub async fn retry_task(state: &AppState, id: String) -> Result<Request, String> {
    let tasks = state
        .tasks()
        .ok_or_else(|| "The task lifecycle is unavailable".to_string())?
        .clone();
    let input = load(state.db(), &id)
        .await?
        .ok_or_else(|| format!("Request {id} not found"))?;
    let latest = input
        .attempts
        .last()
        .map(|b| b.task.clone())
        .ok_or_else(|| format!("Request {id} not found"))?;
    match tasks.failover(&latest).await? {
        Some(_) => get_request(state, id).await,
        None => Err("There's no other provider to try right now".to_string()),
    }
}

#[cfg(test)]
mod tests;
