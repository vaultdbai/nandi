//! Agent-to-agent delegation across instances. A local
//! agent asks for a capability; the delegator picks a provider (local or
//! through the relay), sends a `Task`, and waits for its `TaskResult`. The
//! provider side runs delegated tasks as agent turns.
//!
//! - `delegator`: the requester side (`Delegator`), with fallback
//! - `route`: the network half of picking a provider (the relay's plan;
//!   the relay ranks, this module asks)
//! - `executor`: the provider side: checks, then runs the task as a turn
//!   (`SourceType::Agent`) on the target agent's orchestrator
//! - `history`: the `delegations` table, `delegation://` events, and the
//!   JSONL audit log (`~/.nandi/local/logs/delegations.log`)
//! - `settings`: per-agent delegation settings, kept in skill.md
//! - `tool`: the `delegate_to_agent` tool the orchestrator offers
//!
//! Decisions (see `docs/developer/a2a-delegation.md`):
//! - Local providers run in-process, so delegation works without a relay.
//! - Outbound: an agent gets `delegate_to_agent` when its skill.md sets
//!   `delegation_outbound: true` or lists the tool in `allowed-tools`.
//!   Each call needs approval unless skill.md declares `requires_approval`
//!   without it (the provider is only picked after approval, so local and
//!   cross-user calls are treated alike).
//! - Inbound: trust policy `ask` rejects the task with an error naming the
//!   requester; the owner allows it in the trust graph. A provider's user
//!   is never interrupted by a stranger's request.

pub mod assignment;
pub mod delegator;
pub mod executor;
pub mod history;
pub mod people;
pub mod returns;
pub mod route;
pub mod settings;
pub mod tool;
// Lifecycle, ETA, offline queue, restart resume, failover
pub mod eta;
pub mod failover;
pub mod queue;
pub mod result_outbox;
pub mod resume;
pub mod task;
pub mod task_state;
// Clarification and human escalation
pub mod clarification;
pub mod escalation;
// Supervisor reassignment
pub mod reassign;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::domain::{AgentCard, EntityKind};
use super::events::NandiEvent;
use super::repos::Repository;
use super::trust::TrustPolicy;
use super::AppState;
use crate::model::Timestamp;

pub use delegator::{A2aDeps, Delegator};
// The wire shapes live in `nandi-protocol`
pub use nandi_protocol::{
    ClarificationRequest, ClarificationResponse, DelegationDirection, Question, Task, TaskResult,
    TaskStatus,
};

/// Default wait for a delegated task's result: long enough for a person to
/// approve a held request and for a model to review a pull request
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Everything about one delegation a caller can choose.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DelegationRequest {
    /// Preassigned task ID (`delegate_task` returns it before the task ends)
    pub task_id: Option<String>,
    /// The local agent asking; `None` for the user (a "call" button)
    pub requester_agent: Option<String>,
    pub capability: String,
    pub input: Value,
    /// A provider card ID; `None` scores the providers
    pub target: Option<String>,
}

/// Picks a provider for a capability and runs a task on it.
pub trait A2ADelegator: Send + Sync {
    /// The best provider for `capability` (see `route`), falling back to
    /// the next ones when it rejects the task or times out.
    fn delegate<'a>(
        &'a self,
        capability: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<TaskResult, String>>;

    /// A specific provider (card ID), e.g. from a "call" button.
    fn delegate_to<'a>(
        &'a self,
        target: &'a str,
        capability: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<TaskResult, String>>;

    /// Local and remote cards serving `capability`
    fn list_providers<'a>(
        &'a self,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>>;

    /// `delegate` / `delegate_to` with a requester and a preassigned ID.
    /// The default ignores both.
    fn delegate_request(
        &self,
        request: DelegationRequest,
    ) -> BoxFuture<'_, Result<TaskResult, String>> {
        Box::pin(async move {
            match request.target {
                Some(target) => {
                    self.delegate_to(&target, &request.capability, request.input)
                        .await
                }
                None => self.delegate(&request.capability, request.input).await,
            }
        })
    }
}

/// An agent's delegation settings, shown in the agent's DelegationPanel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentDelegationSettings {
    /// The agent may call `delegate_to_agent` (default off)
    pub allow_outbound: bool,
    /// Remote agents may send it tasks: `accepts_delegation` (default off)
    pub accept_inbound: bool,
    pub max_concurrent: Option<u32>,
    pub overrides: Vec<DelegationOverride>,
}

/// A per-capability exception to the agent's inbound setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelegationOverride {
    pub capability: String,
    pub policy: TrustPolicy,
}

// ---------------------------------------------------------------------------
// Clarification
// ---------------------------------------------------------------------------

/// How a requester agent answers a provider's questions, kept in its
/// skill.md (`clarification:`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClarificationPolicy {
    /// Providers may ask at all
    pub allow: bool,
    /// Rounds of questions per task; one more fails the task
    pub max_rounds: u32,
    /// Topics the requester agent's LLM answers on its own
    pub auto_topics: Vec<String>,
    /// Topics that always go to the human
    pub escalate_topics: Vec<String>,
    /// Per round; no answer by then times the task out
    pub timeout_seconds: u32,
}

impl Default for ClarificationPolicy {
    fn default() -> Self {
        Self {
            allow: true,
            max_rounds: 2,
            auto_topics: Vec::new(),
            escalate_topics: Vec::new(),
            timeout_seconds: 300,
        }
    }
}

/// A question waiting for the human (an escalation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingClarification {
    pub task_id: String,
    /// The requester agent the question is for
    pub agent_id: String,
    pub question: Question,
    /// Epoch ms
    pub deadline: Option<Timestamp>,
    /// The `approvals` row it waits on
    pub approval_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// `delegation://task_started`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelegationTaskStarted {
    pub task_id: String,
    pub direction: DelegationDirection,
    pub capability: String,
    pub requester_agent: String,
    pub target_agent: String,
}

impl NandiEvent for DelegationTaskStarted {
    const NAME: &'static str = "delegation://task_started";
}

/// `delegation://task_completed`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelegationTaskCompleted {
    pub result: TaskResult,
}

impl NandiEvent for DelegationTaskCompleted {
    const NAME: &'static str = "delegation://task_completed";
}

/// `delegation://task_failed` (failed, cancelled, or timed out)
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelegationTaskFailed {
    pub task_id: String,
    pub status: TaskStatus,
    pub error: String,
}

impl NandiEvent for DelegationTaskFailed {
    const NAME: &'static str = "delegation://task_failed";
}

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

/// Registers the provider-side handlers when a relay client exists, and
/// hands the delegator to the orchestrators' `delegate_to_agent` tool.
/// Doesn't block: handlers are registered in a spawned task.
pub fn start(state: &AppState, delegator: &Arc<Delegator>) {
    if delegator.deps().relay.is_some() {
        let registering = delegator.clone();
        state
            .runtime()
            .spawn(async move { registering.register_relay_handlers().await });
    }
    let delegator: Arc<dyn A2ADelegator> = delegator.clone();
    state.orchestrators().set_delegator(&delegator);
}

fn delegator(state: &AppState) -> Result<&Arc<Delegator>, String> {
    state
        .a2a_delegator()
        .ok_or_else(|| "Delegation is unavailable".to_string())
}

/// Every provider serving `capability`; an empty `capability` lists every
/// provider serving anything.
pub async fn list_available_providers(
    state: &AppState,
    capability: &str,
) -> Result<Vec<AgentCard>, String> {
    delegator(state)?.list_providers(capability.trim()).await
}

/// Starts a delegation in the background; returns the task ID. `target`
/// picks a provider instead of scoring. The user is the requester.
pub async fn delegate_task(
    state: &AppState,
    capability: &str,
    input: Value,
    target: Option<&str>,
) -> Result<String, String> {
    let capability = capability.trim();
    if capability.is_empty() {
        return Err("Choose a capability to delegate".to_string());
    }
    let delegator = delegator(state)?.clone();
    let task_id = uuid::Uuid::new_v4().to_string();
    let target = target.map(str::trim).filter(|t| !t.is_empty());
    // Someone else's agent: answered now, not after the background start
    if let Some(target) = target {
        let local = crate::services::network::local_instance_id(state.db())?;
        if delegator::instance_of(target) != local {
            crate::services::relay::unavailable::configured(state.relay_client())?;
        }
    }
    // Recorded now, so `get_task` finds it before a provider is picked
    let placeholder = history::placeholder(&task_id, capability, &input, target);
    history::insert(state.db(), &placeholder).await?;
    let request = DelegationRequest {
        task_id: Some(task_id.clone()),
        requester_agent: None,
        capability: capability.to_string(),
        input,
        target: target.map(str::to_string),
    };
    state.runtime().spawn(async move {
        if let Err(e) = delegator.delegate_request(request).await {
            log::warn!("Delegation failed: {e}");
        }
    });
    Ok(task_id)
}

/// Newest first, both directions
pub async fn list_delegation_history(
    state: &AppState,
    limit: u32,
) -> Result<Vec<TaskResult>, String> {
    history::list(state.db(), limit.clamp(1, history::MAX_HISTORY)).await
}

pub async fn get_agent_delegation(
    state: &AppState,
    agent_id: &str,
) -> Result<AgentDelegationSettings, String> {
    let agent = state.store().agent(agent_id)?;
    Ok(settings::read(&agent))
}

/// Writes the settings into the agent's skill.md (the gold copy), then
/// projects the change.
pub async fn set_agent_delegation(
    state: &AppState,
    agent_id: &str,
    settings: AgentDelegationSettings,
) -> Result<(), String> {
    let agent = state.store().agent(agent_id)?;
    let updated = settings::apply(&agent, &settings)?;
    state
        .store()
        .repos()
        .agents
        .save(&updated)
        .map_err(|e| e.to_string())?;
    state.store().commit(EntityKind::Agent, agent_id)?;
    Ok(())
}
