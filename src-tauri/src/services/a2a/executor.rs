//! The provider side: admits a delegated task, runs it as a turn on the
//! target agent's orchestrator (`SourceType::Agent`, delegation priority),
//! and wraps the turn's final reply as the `TaskResult`.
//!
//! A task is rejected, with a reason the requester sees, when:
//! - the target agent isn't on this instance or doesn't serve the capability
//! - it doesn't accept delegation (`accepts_delegation`, unless a
//!   per-capability override allows it) or an override denies it
//! - it already runs `max_delegation_concurrent` delegated tasks
//! - the requester is on another instance and the trust policy is `deny`,
//!   or `ask` with no override allowing the capability and the requester's
//!   instance not marked a trusted peer (`team::trust`). `ask` rejects
//!   instead of prompting: the owner allows the requester in the trust
//!   graph, and the next call goes through.
//!
//! Requests from this instance (its own agents, or the user) skip the
//! trust policy: they share an owner.
//!
//! Inbound tasks (from another instance) also get a provider `tasks` row
//! and the lifecycle messages (wave final, Track B): `TaskDelivered` on
//! arrival, `RejectTask` when refused, a wait in the slot queue while the
//! agent is full (the row stays `Delivered`), `AcknowledgeTask` with an ETA
//! from `eta_history`, `Working` once the turn is submitted (with a
//! `ProgressUpdate` when the task wants progress), then the result.
//! In-process tasks keep no row: the delegator's caller sees them end.
//!
//! Wave final merge:
//! - A task for `<instance>/gateway` (`gateway::GATEWAY_AGENT_ID`) isn't an
//!   agent turn: it goes to the gateway's handler (`set_gateway_handler`,
//!   registered by `gateway::init`), which applies its policy, routes it,
//!   and sends `RejectTask` itself when it refuses.
//! - Tasks from an instance whose delegations we paused, or from one of its
//!   agents we blocked (`team::trust`), are rejected.
//! - The turn's timeout doesn't run while the task waits for an answer to
//!   `ask_requester` (or its row is `InputRequired`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;

use super::delegator::A2aDeps;
use super::history::{self, Recorder};
use super::queue::SlotQueue;
use super::task::{self, ReasonKind, Refusal, TaskRecord, TaskStage, TaskState};
use super::task_state::{self, Change};
use super::{eta, DelegationDirection, Task, TaskResult, TaskStatus};
use crate::db::{self, role};
use crate::model::{RunStatus, RunSummary};
use crate::services::capability::workflow_provider;
use crate::services::chat;
use crate::services::domain::group::Decision;
use crate::services::domain::{Agent, AgentCard};
use crate::services::events::{AgentTurnDone, NandiEvent, WireEvent};
use crate::services::intake;
use crate::services::orchestrator::{OrchestratorCommand, RunSource};
use crate::services::relay::protocol::{PeerMessage, TaskAssigned};
use crate::services::relay::RelayClient;
use crate::services::team::trust as member_trust;
use crate::services::trust::{self, TrustPolicy};

/// While waiting for a turn, how often the DB is re-checked in case its
/// `agent://turn_done` was missed
const RECHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Runs a task addressed to `<instance>/gateway`: the gateway's result, or
/// `Err(reason)` when it refused it (having told a remote requester)
pub type GatewayHandler =
    Arc<dyn Fn(Task) -> BoxFuture<'static, Result<TaskResult, Refusal>> + Send + Sync>;

/// Gateway handlers by instance ID: the delegator's and the task service's
/// executors (built before `gateway::init`, without an `AppState`) find
/// their instance's gateway here
static GATEWAY_HANDLERS: LazyLock<Mutex<HashMap<String, GatewayHandler>>> =
    LazyLock::new(Mutex::default);

/// Makes `instance_id`'s executors hand gateway tasks to `handler`
/// (replacing any earlier one)
pub fn set_gateway_handler(instance_id: &str, handler: GatewayHandler) {
    GATEWAY_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(instance_id.to_string(), handler);
}

/// Turns running for delegated tasks: `<instance>/<task>` → (agent ID, run
/// ID). Lets a
/// cancelled gateway task stop the turn its child runs (in-process
/// children keep no `tasks` row to find it by).
static RUNNING_TURNS: LazyLock<Mutex<HashMap<String, (String, String)>>> =
    LazyLock::new(Mutex::default);

fn running_turns() -> std::sync::MutexGuard<'static, HashMap<String, (String, String)>> {
    RUNNING_TURNS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Removes a task's entry from [`RUNNING_TURNS`] when the turn ends
struct TurnEntry(String);

impl Drop for TurnEntry {
    fn drop(&mut self) {
        running_turns().remove(&self.0);
    }
}

/// Inbound tasks being handled right now, by `<instance>/<task>` (a task
/// keeps its ID when the relay moves it to another provider). Claimed
/// before the row check, so two deliveries of one task arriving together
/// can't both run it (both would see no row yet).
static HANDLING: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Mutex::default);

/// Releases a [`HANDLING`] claim when the delivery is done
struct Handling(String);

impl Handling {
    /// `None` when `instance` is already handling this task
    fn claim(instance: &str, task_id: &str) -> Option<Self> {
        let key = format!("{instance}/{task_id}");
        HANDLING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone())
            .then(|| Self(key))
    }
}

impl Drop for Handling {
    fn drop(&mut self) {
        HANDLING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// The child task ID the gateway runs a routed task under
/// What a gateway task's hand-off to a local agent adds to its ID
const GATEWAY_CHILD_SUFFIX: &str = "-gw";

pub fn gateway_child_id(task_id: &str) -> String {
    format!("{task_id}{GATEWAY_CHILD_SUFFIX}")
}

/// The gateway task a hand-off answers (the task itself when it isn't one)
pub fn gateway_parent(task_id: &str) -> &str {
    task_id
        .strip_suffix(GATEWAY_CHILD_SUFFIX)
        .unwrap_or(task_id)
}

fn gateway_handler(instance_id: &str) -> Option<GatewayHandler> {
    GATEWAY_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(instance_id)
        .cloned()
}

pub struct Executor {
    deps: A2aDeps,
    recorder: Recorder,
    /// Running delegated tasks per local agent ID
    load: Mutex<HashMap<String, u32>>,
    /// Inbound tasks waiting for a slot
    slots: SlotQueue,
}

/// Holds one unit of an agent's delegation load until dropped.
struct LoadGuard<'a> {
    executor: &'a Executor,
    agent_id: String,
}

impl Drop for LoadGuard<'_> {
    fn drop(&mut self) {
        let mut load = self.executor.lock_load();
        if let Some(count) = load.get_mut(&self.agent_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                load.remove(&self.agent_id);
            }
        }
    }
}

/// How a delegated turn ended.
struct TurnOutcome {
    status: TaskStatus,
    output: Value,
    error: Option<String>,
    tokens_in: u32,
    tokens_out: u32,
}

impl TurnOutcome {
    fn failed(status: TaskStatus, error: String) -> Self {
        Self {
            status,
            output: Value::Null,
            error: Some(error),
            tokens_in: 0,
            tokens_out: 0,
        }
    }
}

impl Executor {
    pub fn new(deps: A2aDeps) -> Self {
        let recorder = deps.recorder();
        Self {
            deps,
            recorder,
            load: Mutex::default(),
            slots: SlotQueue::default(),
        }
    }

    fn lock_load(&self) -> std::sync::MutexGuard<'_, HashMap<String, u32>> {
        self.load.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Delegated tasks `agent_id` is running now
    pub fn load_of(&self, agent_id: &str) -> u32 {
        self.lock_load().get(agent_id).copied().unwrap_or(0)
    }

    fn acquire(&self, agent: &Agent) -> Result<LoadGuard<'_>, String> {
        let limit = agent.capabilities().delegation_limit();
        let mut load = self.lock_load();
        let count = load.entry(agent.id().to_string()).or_default();
        if *count >= limit {
            return Err(format!(
                "'{}' is busy: it already runs {count} of {limit} delegated tasks",
                agent.name()
            ));
        }
        *count += 1;
        Ok(LoadGuard {
            executor: self,
            agent_id: agent.id().to_string(),
        })
    }

    /// A slot on `agent`. In-process tasks fail at once when it's full (the
    /// delegator tries the next provider); inbound ones wait their turn in
    /// the slot queue (still `Delivered`) until the deadline, or until the
    /// requester cancels.
    async fn take_slot(
        &self,
        agent: &Agent,
        task: &Task,
        life: Option<&ProviderLife<'_>>,
    ) -> Result<LoadGuard<'_>, TurnOutcome> {
        let Some(life) = life else {
            return self
                .acquire(agent)
                .map_err(|e| TurnOutcome::failed(TaskStatus::Failed, e));
        };
        let agent_id = agent.id().to_string();
        let deadline = self.deadline(task);
        self.slots.join(
            &agent_id,
            &task.id,
            task::priority_of(&task.input),
            task.created_at as i64,
        );
        // The place in line last told (the requester sees it change)
        let mut told: Option<u32> = None;
        let outcome = loop {
            if self.slots.is_next(&agent_id, &task.id) {
                if let Ok(guard) = self.acquire(agent) {
                    break Ok(guard);
                }
            }
            if life.ended().await {
                break Err(TurnOutcome::failed(
                    TaskStatus::Cancelled,
                    "The task was cancelled while it waited".to_string(),
                ));
            }
            let position = self.slots.position(&agent_id, &task.id).map(|p| p as u32);
            if position.is_some() && position != told {
                told = position;
                let note = format!("Waiting for '{}' to be free", agent.name());
                life.waiting(
                    Change::reason(note).kind(ReasonKind::Queued, TaskStage::Slot),
                    position,
                )
                .await;
            }
            let now = Instant::now();
            if now >= deadline {
                let refusal = Refusal::new(
                    ReasonKind::TimedOut,
                    TaskStage::Slot,
                    format!("'{}' was still busy when time ran out", agent.name()),
                );
                life.reject(&refusal).await;
                break Err(TurnOutcome::failed(TaskStatus::Failed, refusal.message));
            }
            tokio::time::sleep((deadline - now).min(self.deps.dequeue_interval)).await;
        };
        self.slots.leave(&agent_id, &task.id);
        outcome
    }

    /// Whether this instance runs `task`, and on which agent.
    pub fn admit(&self, task: &Task) -> Result<Agent, Refusal> {
        use ReasonKind::*;
        let refuse = |kind, message: String| Refusal::new(kind, TaskStage::Admit, message);
        self.check_member_trust(task)?;
        let prefix = format!("{}/", self.deps.instance_id);
        let agent_id = task.target.strip_prefix(&prefix).ok_or_else(|| {
            refuse(
                Failed,
                "That agent isn't on this Nandi any more".to_string(),
            )
        })?;
        let agent = self.deps.store().agent(agent_id).map_err(|_| {
            refuse(
                Failed,
                format!("There's no agent '{agent_id}' on this Nandi any more"),
            )
        })?;
        let name = agent.name();
        let capability = task.capability.as_str();
        let override_policy = super::settings::overrides(&agent).get(capability).copied();
        if override_policy == Some(TrustPolicy::Deny) {
            return Err(refuse(
                NotAccepting,
                format!("'{name}' doesn't take '{capability}' requests"),
            ));
        }
        // Routed by this instance's gateway: NANDI.md's `expose`/`hide` and
        // caller policy already decided, so the agent needn't accept direct
        // delegation or serve the exact capability asked for
        let via_gateway = task.requester.instance_id == self.deps.instance_id
            && task.requester.agent_id() == crate::services::gateway::GATEWAY_AGENT_ID;
        if via_gateway {
            return Ok(agent);
        }
        if !agent.capabilities().accepts_delegation() && override_policy != Some(TrustPolicy::Allow)
        {
            return Err(refuse(
                NotAccepting,
                format!(
                    "'{name}' doesn't take requests from other Nandis. Its owner can turn \
                     that on in the agent's settings."
                ),
            ));
        }
        if !agent
            .capabilities()
            .serves()
            .iter()
            .any(|c| c.name == capability)
        {
            return Err(refuse(
                NotAccepting,
                format!("'{name}' doesn't offer '{capability}'"),
            ));
        }
        if task.requester.instance_id != self.deps.instance_id {
            let requester = &task.requester.id;
            let policy = trust::policy_for(&self.deps.db, requester, agent_id)
                .map_err(|e| refuse(Failed, e))?;
            match policy {
                TrustPolicy::Deny => {
                    return Err(refuse(
                        NotTrusted,
                        format!(
                            "'{name}' doesn't take requests from {}",
                            requester_name(task)
                        ),
                    ))
                }
                TrustPolicy::Allow => {}
                TrustPolicy::Ask if override_policy == Some(TrustPolicy::Allow) => {}
                // A member the owner marked trusted in Network, or anyone on
                // the company's relay
                TrustPolicy::Ask
                    if member_trust::is_trusted(&self.deps.db, &task.requester.instance_id) => {}
                // The owner approved this one task (`services::intake`)
                TrustPolicy::Ask if intake::is_approved_task(&self.deps.db, &task.id) => {}
                TrustPolicy::Ask => {
                    return Err(refuse(
                        NotTrusted,
                        format!(
                            "The owner of '{name}' needs to trust {} before it takes their \
                             requests.",
                            requester_name(task)
                        ),
                    ))
                }
            }
        }
        Ok(agent)
    }

    /// Another instance's requester we paused delegations with, or one of
    /// its agents we blocked (Track E)
    fn check_member_trust(&self, task: &Task) -> Result<(), Refusal> {
        let instance = &task.requester.instance_id;
        if *instance == self.deps.instance_id {
            return Ok(());
        }
        let who = requester_name(task);
        let refuse =
            |message: String| Refusal::new(ReasonKind::PausedOrBlocked, TaskStage::Admit, message);
        if member_trust::is_paused(&self.deps.db, instance) {
            return Err(refuse(format!("Requests from {who} are paused")));
        }
        if member_trust::is_instance_blocked(&self.deps.db, instance)
            || member_trust::is_blocked(&self.deps.db, instance, &task.requester.id)
        {
            return Err(refuse(format!("Requests from {who} are blocked")));
        }
        Ok(())
    }

    /// How `admit` sees the task before intake: admitted (trusted), held
    /// only because the requester isn't trusted yet (`ask`: the owner may
    /// approve it), or refused for another reason (`deny`, no such agent or
    /// capability), which `admit` reports itself
    fn standing(&self, task: &Task) -> Standing {
        match self.admit(task) {
            Ok(_) => Standing::Trusted,
            Err(refusal) if refusal.kind == ReasonKind::NotTrusted && !self.denied(task) => {
                Standing::Ask
            }
            Err(_) => Standing::Refused,
        }
    }

    /// The trust policy denies the requester this agent
    fn denied(&self, task: &Task) -> bool {
        let prefix = format!("{}/", self.deps.instance_id);
        task.target.strip_prefix(&prefix).is_some_and(|agent_id| {
            trust::policy_for(&self.deps.db, &task.requester.id, agent_id)
                .is_ok_and(|policy| policy == TrustPolicy::Deny)
        })
    }

    /// Intake for a task from another Nandi (`services::intake`): screened,
    /// then run at once (trusted and clean), refused (blocked, or the owner
    /// declined), or held (still `Delivered`) until the owner decides, the
    /// requester cancels, or its deadline passes. `None`: go on.
    async fn intake(&self, task: &Task, life: &ProviderLife<'_>) -> Option<Refusal> {
        if task.requester.instance_id == self.deps.instance_id
            || self.check_member_trust(task).is_err()
        {
            // Ours, or refused by `admit` with its own reason
            return None;
        }
        let refuse =
            |message: String| Refusal::new(ReasonKind::NotAccepting, TaskStage::Admit, message);
        let screen_only = self.is_gateway_task(task);
        let trusted = if screen_only {
            false
        } else {
            match self.standing(task) {
                Standing::Trusted => true,
                Standing::Ask => false,
                // `admit` refuses it with its own reason
                Standing::Refused => return None,
            }
        };
        // Groups I share with the requester (the gateway applies them to
        // its own tasks): the strictest of their policies and my levels
        let group = if screen_only {
            None
        } else {
            self.deps
                .groups
                .as_ref()
                .and_then(|rules| rules.decide(&task.requester.instance_id, &task.capability))
        };
        let group_say = match group.as_ref().map(|gate| &gate.decision) {
            None => intake::GroupSay::None,
            Some(Decision::Auto) => intake::GroupSay::Auto,
            Some(Decision::Ask(why)) => intake::GroupSay::Ask(why),
            Some(Decision::Refuse(why)) => {
                return Some(Refusal::new(
                    ReasonKind::NotAccepting,
                    TaskStage::Admit,
                    format!("Not accepted here. {why}"),
                ))
            }
        };
        let requester = requester_name(task);
        let agent = task.target.rsplit('/').next().unwrap_or(&task.target);
        let target = format!("{agent} · {}", task.capability);
        let text = match task.input.get("text").and_then(Value::as_str) {
            Some(text) => text.to_string(),
            None => task.input.to_string(),
        };
        let team_task = intake::TeamTask {
            task_id: &task.id,
            requester: &requester,
            target: &target,
            text: &text,
            instance_id: &task.requester.instance_id,
            trusted,
            // The gateway holds for the owner itself (Inbox)
            screen_only,
            group: group_say,
        };
        let deadline = self.deadline(task);
        let mut told = false;
        loop {
            match intake::gate_task(
                &self.deps.db,
                &self.deps.bus,
                &self.deps.decisions,
                &team_task,
            ) {
                Ok(intake::Gate::Go) => {
                    if let (Some(rules), Some(gate)) = (self.deps.groups.as_ref(), group.as_ref()) {
                        rules.went_ahead(gate, &task.requester.instance_id);
                    }
                    return None;
                }
                Ok(intake::Gate::Refuse(why)) => return Some(refuse(why)),
                Ok(intake::Gate::Declined) => {
                    return Some(Refusal::new(
                        ReasonKind::Declined,
                        TaskStage::Admit,
                        "The owner declined this request",
                    ))
                }
                Ok(intake::Gate::Wait(_)) => {}
                Err(e) => return Some(refuse(format!("This request couldn't be screened: {e}"))),
            }
            if !told {
                told = true;
                life.waiting(
                    Change::reason("Waiting for the owner to approve")
                        .kind(ReasonKind::NeedsTheirApproval, TaskStage::Admit),
                    None,
                )
                .await;
            }
            if life.ended().await {
                intake::drop_task(&self.deps.db, &self.deps.bus, &task.id);
                return Some(refuse("The task ended while it waited".to_string()));
            }
            let now = Instant::now();
            if now >= deadline {
                intake::drop_task(&self.deps.db, &self.deps.bus, &task.id);
                return Some(Refusal::new(
                    ReasonKind::TimedOut,
                    TaskStage::Admit,
                    "The owner didn't approve it in time",
                ));
            }
            tokio::time::sleep((deadline - now).min(self.deps.dequeue_interval)).await;
        }
    }

    /// Whether `task` is addressed to this instance's gateway
    fn is_gateway_task(&self, task: &Task) -> bool {
        task.target
            == AgentCard::card_id(
                &self.deps.instance_id,
                crate::services::gateway::GATEWAY_AGENT_ID,
            )
    }

    /// A gateway task: the gateway's result, or a failed one with its
    /// refusal (already sent to a remote requester by the gateway)
    async fn run_gateway(
        &self,
        task: &Task,
        mut result: TaskResult,
    ) -> (TaskResult, Option<Refusal>) {
        let direction = result.direction;
        let outcome = match gateway_handler(&self.deps.instance_id) {
            Some(handler) => handler(task.clone()).await,
            None => {
                let refusal = Refusal::new(
                    ReasonKind::Failed,
                    TaskStage::Admit,
                    "Sharing is turned off on this Nandi right now",
                );
                if task.requester.instance_id != self.deps.instance_id {
                    ProviderLife::new(&self.deps, task)
                        .send(PeerMessage::RejectTask {
                            task_id: task.id.clone(),
                            reason: refusal.message.clone(),
                            reason_kind: Some(refusal.kind),
                        })
                        .await;
                }
                Err(refusal)
            }
        };
        match outcome {
            Ok(mut done) => {
                done.direction = direction;
                (done, None)
            }
            Err(refusal) => {
                history::finish(
                    &mut result,
                    TaskStatus::Failed,
                    Some(refusal.message.clone()),
                );
                (result, Some(refusal))
            }
        }
    }

    /// The task waits for the requester's answer to `ask_requester`
    async fn awaiting_answer(&self, task: &Task) -> bool {
        if self
            .deps
            .orchestrators
            .clarifier()
            .is_some_and(|clarifier| clarifier.is_waiting(&task.id))
        {
            return true;
        }
        task::get_row(&self.deps.db, &task.id)
            .await
            .ok()
            .flatten()
            .is_some_and(|row| row.state == TaskState::InputRequired)
    }

    /// Runs `task` and returns its result. `inbound`: the task came from
    /// another instance, so this side records its own history row and
    /// `tasks` row.
    pub async fn execute(&self, task: &Task, inbound: bool) -> TaskResult {
        self.run_task(task, inbound, false).await
    }

    /// An inbound task whose `tasks` row survived a restart: runs it again
    /// from where the row is (see `resume`)
    pub async fn execute_resumed(&self, task: &Task) -> TaskResult {
        self.run_task(task, true, true).await
    }

    async fn run_task(&self, task: &Task, inbound: bool, resumed: bool) -> TaskResult {
        let owner = &self.deps.owner().github_handle;
        let requester_owner = &task.requester.owner.github_handle;
        let mut result = TaskResult {
            task_id: task.id.clone(),
            provider_agent: task.target.clone(),
            output: Value::Null,
            status: TaskStatus::Running,
            duration_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
            error: None,
            direction: if inbound {
                DelegationDirection::Inbound
            } else {
                DelegationDirection::Outbound
            },
            capability: task.capability.clone(),
            requester_agent: task.requester.id.clone(),
            provider_owner: (!owner.is_empty()).then(|| owner.clone()),
            requester_owner: (!requester_owner.is_empty()).then(|| requester_owner.clone()),
            input: task.input.clone(),
            started_at: db::now_ms() as f64,
            ended_at: None,
        };
        if inbound {
            self.recorder.started(&result).await;
        }
        let life = inbound.then(|| ProviderLife::new(&self.deps, task));
        if let Some(life) = &life {
            life.begin(resumed).await;
            // Screened, and held for the owner's approval unless trusted
            if let Some(refusal) = self.intake(task, life).await {
                life.reject(&refusal).await;
                history::finish(&mut result, TaskStatus::Failed, Some(refusal.message));
                self.recorder.finished(&result).await;
                return result;
            }
        }
        if self.is_gateway_task(task) {
            // The provider row and lifecycle like an agent task's; the
            // gateway moves it on as it routes (`gateway_dispatched`)
            let (result, refusal) = self.run_gateway(task, result).await;
            if let Some(life) = &life {
                if result.status == TaskStatus::Failed && life.untouched().await {
                    // Refused by its policy: the gateway told the requester
                    let change = match &refusal {
                        Some(refusal) => Change::refused(refusal),
                        None => Change::error(result.error.clone().unwrap_or_default()),
                    };
                    life.step(TaskState::Rejected, change).await;
                } else {
                    life.finish(&TurnOutcome {
                        status: result.status,
                        output: result.output.clone(),
                        error: result.error.clone(),
                        tokens_in: result.tokens_in,
                        tokens_out: result.tokens_out,
                    })
                    .await;
                }
            }
            if inbound {
                self.recorder.finished(&result).await;
            }
            return result;
        }

        // Wave A seam (Track F): a workflow provider runs the task instead
        let outcome = match workflow_provider::resolve(&self.deps, task).await {
            Some(target) => self.run_workflow(task, &target, life.as_ref()).await,
            None => self.run_agent(task, life.as_ref()).await,
        };
        if let Some(life) = &life {
            life.finish(&outcome).await;
        }
        result.output = outcome.output;
        result.tokens_in = outcome.tokens_in;
        result.tokens_out = outcome.tokens_out;
        history::finish(&mut result, outcome.status, outcome.error);

        if inbound {
            self.recorder.finished(&result).await;
        }
        result
    }

    /// The agent path: admit, wait for a slot, run the turn
    async fn run_agent(&self, task: &Task, life: Option<&ProviderLife<'_>>) -> TurnOutcome {
        match self.admit(task) {
            Ok(agent) => match self.take_slot(&agent, task, life).await {
                Ok(_guard) => {
                    if let Some(life) = life {
                        life.acknowledged(agent.id().as_str()).await;
                    }
                    self.run_turn(&agent, task, life)
                        .await
                        .unwrap_or_else(|e| TurnOutcome::failed(TaskStatus::Failed, e))
                }
                Err(outcome) => outcome,
            },
            Err(refusal) => {
                if let Some(life) = life {
                    life.reject(&refusal).await;
                }
                TurnOutcome::failed(TaskStatus::Failed, refusal.message)
            }
        }
    }

    /// The workflow path (wave A, Track F): the trust checks an agent
    /// gets, then `Acknowledged`, `Working`, a progress update per settled
    /// step (when the task wants progress), and the workflow's result,
    /// within the task's deadline
    async fn run_workflow(
        &self,
        task: &Task,
        target: &workflow_provider::WorkflowTarget,
        life: Option<&ProviderLife<'_>>,
    ) -> TurnOutcome {
        if let Err(refusal) = self.admit_workflow(task, &target.workflow_id) {
            if let Some(life) = life {
                life.reject(&refusal).await;
            }
            return TurnOutcome::failed(TaskStatus::Failed, refusal.message);
        }
        let row = match life {
            Some(life) => {
                life.acknowledged(&target.workflow_id).await;
                life.step(TaskState::Working, Change::reason("the workflow started"))
                    .await
            }
            None => None,
        };
        // Every settled step, live (task 5): stored on this side and sent to
        // the requester, not only as the breakdown at the end
        let progress = {
            let (relay, db) = (self.deps.relay.clone(), self.deps.db.clone());
            move |done: usize, total: usize, step: &str| {
                let Some(row) = row.clone() else {
                    return;
                };
                let percent = (done * 100 / total.max(1)).min(100) as u8;
                let note = format!("Step '{step}' done ({done}/{total})");
                let (relay, db) = (relay.clone(), db.clone());
                tokio::spawn(async move {
                    let sent =
                        task::send_progress(relay.as_ref(), &db, &row, Some(percent), Some(note))
                            .await;
                    if let Err(e) = sent {
                        log::debug!("Task {}: progress not sent: {e}", row.id);
                    }
                });
            }
        };
        let run = workflow_provider::run(&self.deps.db, task, target, progress);
        match tokio::time::timeout_at(self.deadline(task), run).await {
            Ok(done) => TurnOutcome {
                status: done.status,
                output: done.output,
                error: done.error,
                tokens_in: done.tokens_in,
                tokens_out: done.tokens_out,
            },
            Err(_) => TurnOutcome::failed(
                TaskStatus::TimedOut,
                format!(
                    "The workflow '{}' didn't finish in time",
                    target.workflow_id
                ),
            ),
        }
    }

    /// `admit` for a workflow provider: paused or blocked requesters are
    /// refused, and another instance's requester needs the workflow's
    /// trust policy (keyed by the workflow ID) or a trusted-peer mark
    fn admit_workflow(&self, task: &Task, workflow_id: &str) -> Result<(), Refusal> {
        self.check_member_trust(task)?;
        if task.requester.instance_id == self.deps.instance_id {
            return Ok(());
        }
        let requester = &task.requester.id;
        let not_trusted =
            |message: String| Refusal::new(ReasonKind::NotTrusted, TaskStage::Admit, message);
        let policy = trust::policy_for(&self.deps.db, requester, workflow_id)
            .map_err(|e| Refusal::new(ReasonKind::Failed, TaskStage::Admit, e))?;
        match policy {
            TrustPolicy::Allow => Ok(()),
            TrustPolicy::Ask
                if member_trust::is_trusted(&self.deps.db, &task.requester.instance_id) =>
            {
                Ok(())
            }
            // The owner approved this one task (`services::intake`)
            TrustPolicy::Ask if intake::is_approved_task(&self.deps.db, &task.id) => Ok(()),
            TrustPolicy::Deny => Err(not_trusted(format!(
                "The workflow '{workflow_id}' doesn't take requests from {}",
                requester_name(task)
            ))),
            TrustPolicy::Ask => Err(not_trusted(format!(
                "The owner of the workflow '{workflow_id}' needs to trust {} before it takes \
                 their requests.",
                requester_name(task)
            ))),
        }
    }

    fn deadline(&self, task: &Task) -> Instant {
        let now = db::now_ms() as f64;
        let wait = task
            .deadline
            .map(|deadline| Duration::from_millis((deadline - now).max(0.0) as u64))
            .unwrap_or(self.deps.timeout);
        Instant::now() + wait
    }

    async fn run_turn(
        &self,
        agent: &Agent,
        task: &Task,
        life: Option<&ProviderLife<'_>>,
    ) -> Result<TurnOutcome, String> {
        let mut deadline = self.deadline(task);
        let agent_id = agent.id().to_string();
        let label = format!(
            "Delegated by {} · {}",
            display(&task.requester.name, &task.requester.id),
            task.capability
        );
        // Its own conversation: the turn sees neither the owner's chats with
        // the agent nor other requesters' tasks (a resumed task finds its own)
        let (id, task_id, title) = (agent_id.clone(), task.id.clone(), label.clone());
        let conversation_id = self
            .deps
            .db
            .call(move |conn| db::ensure_task_conversation(conn, &id, &task_id, &title))
            .await?;
        // Before submitting, so the turn's end can't be missed
        let mut events = self.deps.broadcast.subscribe();
        let sent = chat::submit(
            &self.deps.orchestrators,
            &agent_id,
            conversation_id,
            prompt(task),
            vec![],
            None,
            RunSource::delegation(&task.requester.id, label),
        )
        .await?;
        let key = format!("{}/{}", self.deps.instance_id, task.id);
        running_turns().insert(key.clone(), (agent_id.clone(), sent.run_id.clone()));
        let _entry = TurnEntry(key);
        if let Some(life) = life {
            life.working(&agent_id, &sent.run_id).await;
        } else if let Some(gateway_task) = task.id.strip_suffix(GATEWAY_CHILD_SUFFIX) {
            // A gateway's hand-off to a local agent has no lifecycle of its
            // own: the run belongs to the gateway task, the one the person
            // sent, so the run shows who asked and what (RQ-27)
            let run = serde_json::json!({ "agentId": agent_id, "runId": sent.run_id });
            if let Err(e) =
                task::add_message(&self.deps.db, gateway_task, task::RUN_KIND, run).await
            {
                log::warn!("Task {gateway_task}: {e}");
            }
        }

        // The deadline moves on while the task waits for the requester's
        // answer (`ask_requester`): that time isn't the provider's
        let waiting = wait_for_run(&self.deps.db, &mut events, &sent.run_id);
        tokio::pin!(waiting);
        let mut checked = Instant::now();
        let ended = loop {
            let tick = deadline.min(Instant::now() + RECHECK_INTERVAL);
            tokio::select! {
                ended = &mut waiting => break Some(ended),
                _ = tokio::time::sleep_until(tick) => {}
            }
            let now = Instant::now();
            if self.awaiting_answer(task).await {
                deadline += now - checked;
            }
            checked = now;
            if now >= deadline {
                break None;
            }
        };
        let (run, reply) = match ended {
            Some(ended) => ended?,
            None => {
                self.stop(&agent_id, &sent.run_id).await;
                return Ok(TurnOutcome::failed(
                    TaskStatus::TimedOut,
                    format!("'{}' didn't finish in time", agent.name()),
                ));
            }
        };
        let (tokens_in, tokens_out) = (run.input_tokens, run.output_tokens);
        let mut outcome = match run.status {
            RunStatus::Completed => TurnOutcome {
                status: TaskStatus::Completed,
                output: parse_reply(&reply),
                error: None,
                tokens_in: 0,
                tokens_out: 0,
            },
            RunStatus::Cancelled => TurnOutcome::failed(
                TaskStatus::Cancelled,
                format!("'{}' stopped the task", agent.name()),
            ),
            _ => TurnOutcome::failed(
                TaskStatus::Failed,
                run.summary
                    .unwrap_or_else(|| format!("The turn ended as {}", run.status.as_str())),
            ),
        };
        (outcome.tokens_in, outcome.tokens_out) = (tokens_in, tokens_out);
        Ok(outcome)
    }

    /// Stops the turn running for `task_id` (or for the child its gateway
    /// routed it to); whether one was running
    pub(crate) async fn stop_task(&self, task_id: &str) -> bool {
        let turn = {
            let turns = running_turns();
            let me = &self.deps.instance_id;
            turns
                .get(&format!("{me}/{task_id}"))
                .or_else(|| turns.get(&format!("{me}/{}", gateway_child_id(task_id))))
                .cloned()
        };
        match turn {
            Some((agent_id, run_id)) => {
                self.stop(&agent_id, &run_id).await;
                true
            }
            None => false,
        }
    }

    /// Stops a turn (timed out, or its task cancelled), queued or running.
    pub(crate) async fn stop(&self, agent_id: &str, run_id: &str) {
        let id = run_id.to_string();
        let run = self
            .deps
            .db
            .call(move |conn| db::get_run(conn, &id))
            .await
            .ok()
            .flatten();
        let orchestrators = &self.deps.orchestrators;
        let stopped = match run.map(|r| r.status) {
            Some(RunStatus::Queued) => {
                let run_id = run_id.to_string();
                orchestrators
                    .request(agent_id, |reply| OrchestratorCommand::CancelQueued {
                        run_id,
                        reply,
                    })
                    .await
            }
            Some(status) if status.is_active() => {
                let run_id = run_id.to_string();
                orchestrators
                    .send(agent_id, OrchestratorCommand::Cancel { run_id })
                    .await
            }
            _ => Ok(()),
        };
        if let Err(e) = stopped {
            log::warn!("Failed to stop delegated run {run_id}: {e}");
        }
    }
}

/// The provider's `tasks` row of an inbound task, and the lifecycle
/// messages each step sends the requester. Failures are logged: the
/// lifecycle never fails the task itself.
/// A task from another Nandi as `admit` sees it, before intake
enum Standing {
    Trusted,
    /// Held only because the requester isn't trusted yet
    Ask,
    /// `admit` refuses it for another reason
    Refused,
}

struct ProviderLife<'a> {
    deps: &'a A2aDeps,
    task: &'a Task,
}

impl<'a> ProviderLife<'a> {
    fn new(deps: &'a A2aDeps, task: &'a Task) -> Self {
        Self { deps, task }
    }

    async fn send(&self, body: PeerMessage) {
        let Some(relay) = &self.deps.relay else {
            return;
        };
        let target = self.task.requester.instance_id.clone();
        if let Err(e) = relay.send_peer(target, body).await {
            log::debug!("Task {}: lifecycle message not sent: {e}", self.task.id);
        }
    }

    /// Moves the row toward `to`; the row when it moved
    async fn step(&self, to: TaskState, change: Change) -> Option<TaskRecord> {
        match task_state::advance(&self.deps.db, &self.deps.bus, &self.task.id, to, change).await {
            Ok(moved) => moved,
            Err(e) => {
                log::warn!("Task {}: {e}", self.task.id);
                None
            }
        }
    }

    /// New: the row (with the wire task), `Delivered`, `TaskDelivered`.
    /// Resumed: the row is already there.
    async fn begin(&self, resumed: bool) {
        if resumed {
            return;
        }
        let record = TaskRecord::provider(self.task, &self.deps.instance_id);
        if let Err(e) = task::create(&self.deps.db, &self.deps.bus, &record, Some(self.task)).await
        {
            log::warn!("Task {}: {e}", self.task.id);
            return;
        }
        self.step(
            TaskState::Delivered,
            Change::reason("assigned by the relay").at(TaskStage::Admit),
        )
        .await;
        self.send(PeerMessage::TaskDelivered {
            task_id: self.task.id.clone(),
        })
        .await;
    }

    async fn reject(&self, refusal: &Refusal) {
        if self
            .step(TaskState::Rejected, Change::refused(refusal))
            .await
            .is_some()
        {
            self.send(PeerMessage::RejectTask {
                task_id: self.task.id.clone(),
                reason: refusal.message.clone(),
                reason_kind: Some(refusal.kind),
            })
            .await;
            // The owner sees who wants in (Network › Sharing, Home's Inbox)
            if refusal.kind == ReasonKind::NotTrusted {
                crate::services::events::emit(
                    &self.deps.bus,
                    &crate::services::team::trust_requests::TrustRequestReceived {
                        instance_id: self.task.requester.instance_id.clone(),
                    },
                );
            }
        }
    }

    /// Still waiting, and why (a hold, a place in line): a `task_events`
    /// note, and the requester told with the same kind
    async fn waiting(&self, change: Change, position: Option<u32>) {
        let (kind, note) = (change.reason_kind, change.reason.clone());
        if let Err(e) = task_state::note(&self.deps.db, &self.deps.bus, &self.task.id, change).await
        {
            log::warn!("Task {}: {e}", self.task.id);
        }
        // This side's Request page shows the place in line too
        let stored = serde_json::json!({
            "percent": null,
            "note": note,
            "reasonKind": kind,
            "queuePosition": position,
        });
        if let Err(e) =
            task::add_message(&self.deps.db, &self.task.id, task::PROGRESS_KIND, stored).await
        {
            log::debug!("Task {}: {e}", self.task.id);
        }
        self.send(PeerMessage::ProgressUpdate {
            task_id: self.task.id.clone(),
            percent: None,
            note,
            reason_kind: kind.map(ReasonKind::for_requester),
            queue_position: position,
        })
        .await;
    }

    /// Still `Delivered`: the gateway refused it before routing
    async fn untouched(&self) -> bool {
        task::get_row(&self.deps.db, &self.task.id)
            .await
            .ok()
            .flatten()
            .is_some_and(|row| row.state == TaskState::Delivered)
    }

    /// The requester (or a timeout) ended it while it waited
    async fn ended(&self) -> bool {
        task::get_row(&self.deps.db, &self.task.id)
            .await
            .ok()
            .flatten()
            .is_some_and(|row| row.state.is_terminal())
    }

    /// `Acknowledged` with the agent's median time for the capability
    async fn acknowledged(&self, agent_id: &str) {
        let card = format!("{}/{agent_id}", self.deps.instance_id);
        let capability = self.task.capability.clone();
        let estimate = self
            .deps
            .db
            .call(move |conn| eta::estimate(conn, &capability, Some(&card)))
            .await;
        let eta_seconds = estimate.ok().map(|e| e.median_seconds);
        let change = Change {
            reason: Some("accepted".to_string()),
            stage: Some(TaskStage::Child),
            eta_seconds,
            ..Change::default()
        };
        if self.step(TaskState::Acknowledged, change).await.is_some() {
            self.send(PeerMessage::AcknowledgeTask {
                task_id: self.task.id.clone(),
                eta_seconds,
                note: None,
            })
            .await;
        }
    }

    /// The turn is submitted: `Working`, and the first progress update
    async fn working(&self, agent_id: &str, run_id: &str) {
        let run = serde_json::json!({ "agentId": agent_id, "runId": run_id });
        if let Err(e) = task::add_message(&self.deps.db, &self.task.id, task::RUN_KIND, run).await {
            log::warn!("Task {}: {e}", self.task.id);
        }
        let Some(row) = self
            .step(
                TaskState::Working,
                Change::reason("the turn started").at(TaskStage::Child),
            )
            .await
        else {
            return;
        };
        // Always: it's how the requester learns the task is `Working`
        {
            let sent = task::send_progress(
                self.deps.relay.as_ref(),
                &self.deps.db,
                &row,
                Some(0),
                Some("started".to_string()),
            )
            .await;
            if let Err(e) = sent {
                log::debug!("Task {}: progress not sent: {e}", self.task.id);
            }
        }
    }

    /// The row ends like the turn; completions go into `eta_history`
    async fn finish(&self, outcome: &TurnOutcome) {
        let error = || {
            outcome
                .error
                .clone()
                .unwrap_or_else(|| outcome.status.as_str().to_string())
        };
        let (to, change) = match outcome.status {
            TaskStatus::Completed => (
                TaskState::Completed,
                Change {
                    reason: Some("completed".to_string()),
                    stage: Some(TaskStage::Result),
                    result: Some(outcome.output.clone()),
                    ..Change::default()
                },
            ),
            TaskStatus::Cancelled => (
                TaskState::Cancelled,
                Change::error(error()).at(TaskStage::Result),
            ),
            TaskStatus::TimedOut => (
                TaskState::TimedOut,
                Change::error(error()).kind(ReasonKind::TimedOut, TaskStage::Result),
            ),
            TaskStatus::Failed | TaskStatus::Running => (
                TaskState::Failed,
                Change::error(error()).kind(ReasonKind::Failed, TaskStage::Result),
            ),
        };
        let Some(done) = self.step(to, change).await else {
            return;
        };
        if done.state == TaskState::Completed {
            let start = done.acknowledged_at.unwrap_or(done.created_at);
            let seconds = ((db::now_ms() as f64 - start).max(0.0) / 1000.0) as u32;
            let (capability, card) = (done.capability.clone(), done.provider_card());
            let recorded = self
                .deps
                .db
                .call(move |conn| eta::record(conn, &capability, &card, seconds, db::now_ms()))
                .await;
            if let Err(e) = recorded {
                log::warn!("Task {}: {e}", self.task.id);
            }
        }
    }
}

/// The gateway accepted a task from another instance and routed it:
/// `Acknowledged` (with `note`, the routing reason, and an ETA: `eta_seconds`,
/// else the estimate), then
/// `Working` with a "started" progress update. No-op for local callers
/// (no provider row).
pub async fn gateway_dispatched(
    state: &crate::services::AppState,
    task: &Task,
    note: String,
    eta_seconds: Option<u32>,
) {
    if task.requester.instance_id
        == crate::services::network::local_instance_id(state.db()).unwrap_or_default()
    {
        return;
    }
    let deps = match A2aDeps::from_state(state) {
        Ok(deps) => deps,
        Err(e) => return log::warn!("Task {}: lifecycle unavailable: {e}", task.id),
    };
    let life = ProviderLife::new(&deps, task);
    let eta_seconds = match eta_seconds {
        Some(eta) => Some(eta),
        None => {
            let capability = task.capability.clone();
            let card = task.target.clone();
            let estimate = deps
                .db
                .call(move |conn| eta::estimate(conn, &capability, Some(&card)))
                .await;
            estimate.ok().map(|e| e.median_seconds)
        }
    };
    let change = Change {
        reason: Some(note.clone()),
        stage: Some(TaskStage::Route),
        eta_seconds,
        ..Change::default()
    };
    if life.step(TaskState::Acknowledged, change).await.is_some() {
        life.send(PeerMessage::AcknowledgeTask {
            task_id: task.id.clone(),
            eta_seconds,
            note: Some(note),
        })
        .await;
    }
    let Some(row) = life
        .step(
            TaskState::Working,
            Change::reason("routed; working on it").at(TaskStage::Dispatch),
        )
        .await
    else {
        return;
    };
    if let Err(e) = task::send_progress(
        deps.relay.as_ref(),
        &deps.db,
        &row,
        Some(0),
        Some("started".to_string()),
    )
    .await
    {
        log::debug!("Task {}: progress not sent: {e}", task.id);
    }
}

/// A gateway task waits (held for the owner's approval, or in line for a
/// slot): a `task_events` note with the change's kind and stage, and the
/// remote requester told the same (from its side). No-op for local callers.
pub async fn gateway_waiting(
    state: &crate::services::AppState,
    task: &Task,
    change: Change,
    position: Option<u32>,
) {
    if task.requester.instance_id
        == crate::services::network::local_instance_id(state.db()).unwrap_or_default()
    {
        return;
    }
    match A2aDeps::from_state(state) {
        Ok(deps) => {
            ProviderLife::new(&deps, task)
                .waiting(change, position)
                .await
        }
        Err(e) => log::warn!("Task {}: lifecycle unavailable: {e}", task.id),
    }
}

fn display<'a>(name: &'a str, fallback: &'a str) -> &'a str {
    if name.trim().is_empty() {
        fallback
    } else {
        name
    }
}

/// The message the provider agent gets.
fn prompt(task: &Task) -> String {
    let requester = display(&task.requester.name, &task.requester.id);
    let input = match &task.input {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => format!(
            "```json\n{}\n```",
            serde_json::to_string_pretty(other).unwrap_or_default()
        ),
    };
    format!(
        "{requester} ({}) delegated a `{}` task to you. Do it and reply with the result only.\n\n{input}",
        task.requester.id, task.capability
    )
}

/// JSON replies stay structured; anything else is text.
fn parse_reply(reply: &str) -> Value {
    serde_json::from_str(reply.trim()).unwrap_or_else(|_| Value::String(reply.to_string()))
}

/// Waits for a run to finish; returns it and its final reply.
async fn wait_for_run(
    db: &crate::db::Db,
    events: &mut tokio::sync::broadcast::Receiver<WireEvent>,
    run_id: &str,
) -> Result<(RunSummary, String), String> {
    loop {
        let id = run_id.to_string();
        let ended = db
            .call(move |conn| match db::get_run(conn, &id)? {
                Some(run) if run.status.is_finished() => {
                    let reply = db::list_run_messages(conn, &id)?
                        .into_iter()
                        .rev()
                        .find(|m| m.role == role::ASSISTANT && !m.content.trim().is_empty())
                        .map(|m| m.content)
                        .unwrap_or_default();
                    Ok(Some(Some((run, reply))))
                }
                Some(_) => Ok(Some(None)),
                None => Ok(None),
            })
            .await?
            .ok_or_else(|| format!("Run {run_id} not found"))?;
        if let Some(ended) = ended {
            return Ok(ended);
        }
        loop {
            match tokio::time::timeout(RECHECK_INTERVAL, events.recv()).await {
                Ok(Ok(event)) => {
                    if event.event == AgentTurnDone::NAME && event.payload["runId"] == run_id {
                        break;
                    }
                }
                Ok(Err(RecvError::Lagged(_))) | Err(_) => break,
                Ok(Err(RecvError::Closed)) => return Err("Event bus closed".to_string()),
            }
        }
    }
}

/// Runs the tasks the relay assigns to this instance and sends each result
/// back. Holds the executor weakly: the relay outlives neither.
pub async fn register(executor: &Arc<Executor>, relay: &Arc<dyn RelayClient>) {
    let executor: Weak<Executor> = Arc::downgrade(executor);
    let sender: Weak<dyn RelayClient> = Arc::downgrade(relay);
    let runtime = tokio::runtime::Handle::current();
    relay
        .on_task_assigned(Box::new(move |assigned: TaskAssigned| {
            let task = assigned.task;
            let (executor, sender) = (executor.clone(), sender.clone());
            runtime.spawn(async move {
                let Some(executor) = executor.upgrade() else {
                    return;
                };
                // Sent again (a delivery the relay sent before our ack): a
                // running task isn't run twice, a finished one answers again
                let Some(_handling) = Handling::claim(&executor.deps.instance_id, &task.id) else {
                    log::info!("Task {}: delivered twice; already on it", task.id);
                    return;
                };
                let db = executor.deps.db.clone();
                let result = match task::get_row(&db, &task.id).await.ok().flatten() {
                    Some(row) if !row.state.is_terminal() => return,
                    Some(_) => match history::get(&db, &task.id).await {
                        Ok(Some(done)) => done,
                        _ => return,
                    },
                    None => executor.execute(&task, true).await,
                };
                let Some(relay) = sender.upgrade() else {
                    return;
                };
                if let Err(e) = relay.send_result(result).await {
                    log::error!("Failed to send the result of task {}: {e}", task.id);
                }
            });
        }))
        .await;
}

/// How errors name a task's requester: its owner's display name, else the
/// card's name, else the card ID
fn requester_name(task: &Task) -> String {
    let requester = &task.requester;
    [requester.owner.display_name.trim(), requester.name.trim()]
        .into_iter()
        .find(|name| !name.is_empty())
        .map(|name| format!("'{name}'"))
        .unwrap_or_else(|| format!("'{}'", requester.id))
}
