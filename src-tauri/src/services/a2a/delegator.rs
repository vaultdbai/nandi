//! The requester side: finds providers (local cards plus the relay's),
//! ranks them (`ranking`), and runs the task: in-process for a local
//! provider (no relay needed), else through `RelayClient::delegate_task`,
//! waiting for the result up to the timeout (`TaskStatus::TimedOut` after
//! it). When the pick rejects the task, can't be reached, or times out, the
//! next ranked provider gets it (up to `MAX_ATTEMPTS`); a provider the
//! caller named is never swapped.
//!
//! Every delegation gets an outbound history row, `delegation://` events,
//! and audit log lines, however it ends. A failed attempt that was retried
//! keeps its own row (`<task_id>-attempt<n>`).
//!
//! Each cross-instance attempt also gets a requester `tasks` row (wave
//! final, Track B): `Submitted`, `Delivered` once the relay routes it, the
//! provider's lifecycle messages in between (handled by `TaskService`),
//! then the result. Retries on another instance are failover attempts
//! (`attempt_number`, `previous_attempt_id`, `source_kind = Failover`),
//! never on an instance that already failed this delegation or whose
//! circuit is open; with a `SelectionService` the rest is re-ranked before
//! each one. When the relay can't deliver to the last candidate, the task
//! stays `Submitted` in the offline queue (`queue`) and this waits for it
//! as usual. Once acknowledged, the wait stretches to the timeout sweep's
//! limit (3× the ETA); a timed-out wait ends the row too (`TimedOut`, or
//! `Cancelled` before an acknowledgement, telling the provider either
//! way), except a still-queued task, which the queue keeps.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;

use super::executor::{self, Executor};
use super::history::{self, AuditLog, Recorder};
use super::queue::{self, TIMEOUT_ETA_FACTOR};
use super::ranking::{self, RankData};
use super::task::{
    self, ReasonKind, TaskRecord, TaskService, TaskSourceKind, TaskStage, TaskState,
};
use super::task_state::{self, Change};
use super::{assignment, eta, failover};
use super::{A2ADelegator, DelegationRequest, Task, TaskResult, TaskStatus, DEFAULT_TIMEOUT_SECS};
use crate::db::{self, Db};
use crate::services::capability_index::CapabilityIndex;
use crate::services::domain::agent_card::{card_from_agent, OwnerRef};
use crate::services::domain::AgentCard;
use crate::services::events::{BroadcastSink, EventBus, NandiEvent, WireEvent};
use crate::services::gateway::assignments::GatewayAssignmentChanged;
use crate::services::gateway::holds::{GatewayHoldChanged, HoldStatus};
use crate::services::gateway::GATEWAY_AGENT_ID;
use crate::services::network;
use crate::services::orchestrator::Orchestrators;
use crate::services::relay::protocol::PeerMessage;
use crate::services::relay::unavailable::{self, RelayUnavailable};
use crate::services::relay::RelayClient;
use crate::services::repo_store::RepoStore;
use crate::services::team::trust as member_trust;
use crate::services::team::TeamManager;
use crate::services::AppState;

/// `local_settings` key overriding `DEFAULT_TIMEOUT_SECS`
pub const TIMEOUT_SETTING: &str = "delegation_timeout_secs";

/// Agent ID part of the user's requester card (`<instance>/_user`); never
/// a valid agent ID
pub const USER_REQUESTER: &str = "_user";

/// Providers one auto-picked delegation tries before giving up
pub const MAX_ATTEMPTS: usize = 3;

/// What delegation needs from the app. Built once by `a2a::init`; tests
/// build their own (e.g. with a mock relay or a short timeout).
#[derive(Clone)]
pub struct A2aDeps {
    pub db: Db,
    pub bus: EventBus,
    pub orchestrators: Orchestrators,
    pub broadcast: BroadcastSink,
    pub instance_id: String,
    pub index: Option<Arc<RwLock<CapabilityIndex>>>,
    pub relay: Option<Arc<dyn RelayClient>>,
    /// For ranking teammates first
    pub teams: Option<Arc<dyn TeamManager>>,
    pub audit: AuditLog,
    /// How long a delegated task may take
    pub timeout: Duration,
    /// How often an inbound task waiting for a slot looks again
    pub dequeue_interval: Duration,
}

impl A2aDeps {
    /// Who runs this instance: the index's owner, kept current from
    /// NANDI.md by the gateway; none before onboarding
    pub fn owner(&self) -> OwnerRef {
        self.index
            .as_ref()
            .map(|index| {
                index
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .owner()
                    .clone()
            })
            .unwrap_or_default()
    }

    pub fn from_state(state: &AppState) -> Result<Self, String> {
        let db = state.db().clone();
        let instance_id = network::local_instance_id(&db)?;
        let timeout = db
            .conn()
            .ok()
            .and_then(|conn| db::get_setting(&conn, TIMEOUT_SETTING).ok().flatten())
            .and_then(|value| value.as_u64())
            .filter(|secs| *secs > 0)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        Ok(Self {
            db,
            bus: state.bus().clone(),
            orchestrators: state.orchestrators().clone(),
            broadcast: state.broadcast().clone(),
            instance_id,
            index: state.capability_index().cloned(),
            relay: state.relay_client().cloned(),
            teams: state.team_manager().cloned(),
            audit: AuditLog::default_location(),
            timeout: Duration::from_secs(timeout),
            dequeue_interval: Duration::from_secs(queue::DEQUEUE_INTERVAL_SECS),
        })
    }

    pub fn store(&self) -> &RepoStore {
        self.orchestrators.store()
    }

    pub fn recorder(&self) -> Recorder {
        Recorder {
            db: self.db.clone(),
            bus: self.bus.clone(),
            audit: self.audit.clone(),
        }
    }

    fn relay_url(&self) -> String {
        self.relay
            .as_ref()
            .and_then(|relay| relay.status().relay_url)
            .unwrap_or_default()
    }
}

/// The instance part of a card ID (`<instance>/<agent>`)
pub fn instance_of(card_id: &str) -> &str {
    card_id.split_once('/').map_or("", |(instance, _)| instance)
}

type Waiter = oneshot::Sender<Result<TaskResult, String>>;

/// Remote tasks waiting for their result, by task ID.
#[derive(Default)]
struct Pending(Mutex<HashMap<String, Waiter>>);

impl Pending {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Waiter>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn add(&self, task_id: &str) -> oneshot::Receiver<Result<TaskResult, String>> {
        let (tx, rx) = oneshot::channel();
        self.lock().insert(task_id.to_string(), tx);
        rx
    }

    fn resolve(&self, task_id: &str, outcome: Result<TaskResult, String>) {
        if let Some(waiter) = self.lock().remove(task_id) {
            let _ = waiter.send(outcome);
        }
    }

    fn rekey(&self, from: &str, to: &str) {
        let mut pending = self.lock();
        if let Some(waiter) = pending.remove(from) {
            pending.insert(to.to_string(), waiter);
        }
    }

    fn forget(&self, task_id: &str) {
        self.lock().remove(task_id);
    }
}

pub struct Delegator {
    deps: A2aDeps,
    executor: Arc<Executor>,
    pending: Arc<Pending>,
    recorder: Recorder,
}

impl Delegator {
    pub fn new(deps: A2aDeps) -> Arc<Self> {
        Arc::new(Self {
            executor: Arc::new(Executor::new(deps.clone())),
            recorder: deps.recorder(),
            pending: Arc::default(),
            deps,
        })
    }

    pub fn deps(&self) -> &A2aDeps {
        &self.deps
    }

    pub fn executor(&self) -> &Arc<Executor> {
        &self.executor
    }

    /// Listens for results of our remote tasks, and runs the tasks the
    /// relay assigns to us. No-op without a relay client.
    pub async fn register_relay_handlers(self: Arc<Self>) {
        let Some(relay) = self.deps.relay.clone() else {
            return;
        };
        let pending = self.pending.clone();
        relay
            .on_task_completed(Box::new(move |completed| {
                pending.resolve(&completed.task_id, Ok(completed.result));
            }))
            .await;
        let pending = self.pending.clone();
        relay
            .on_task_failed(Box::new(move |failed| {
                pending.resolve(&failed.task_id, Err(failed.error));
            }))
            .await;
        executor::register(&self.executor, &relay).await;
    }

    fn endpoint(&self, agent_id: &str) -> String {
        let relay_url = self.deps.relay_url();
        if relay_url.is_empty() {
            String::new()
        } else {
            network::agent_endpoint(&relay_url, &self.deps.instance_id, agent_id)
        }
    }

    /// This instance's cards serving `capability` (any capability when
    /// empty), with their current delegation load.
    pub fn local_cards(&self, capability: &str) -> Vec<AgentCard> {
        let indexed = match (&self.deps.index, capability.is_empty()) {
            (Some(index), false) => index
                .read()
                .map(|index| index.find_local(capability))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut cards = if indexed.is_empty() {
            // No index (or not built yet): derive them from the agents
            let instance = self.deps.instance_id.clone();
            self.deps.store().read(|repo| {
                repo.agents()
                    .filter(|agent| {
                        let serves = agent.capabilities().serves();
                        !serves.is_empty()
                            && (capability.is_empty()
                                || serves.iter().any(|c| c.name == capability))
                    })
                    .map(|agent| {
                        card_from_agent(agent, &instance, &self.endpoint(agent.id().as_str()))
                    })
                    .collect()
            })
        } else {
            indexed
        };
        let now = db::now_ms() as f64;
        for card in &mut cards {
            card.current_load = self.executor.load_of(card.agent_id());
            card.issued_at = now;
            if card.owner.github_handle.is_empty() {
                card.owner = self.owner();
            }
        }
        cards
    }

    /// Cards the relay knows serving `capability`, other instances only.
    async fn remote_cards(&self, capability: &str) -> Vec<AgentCard> {
        let Some(relay) = &self.deps.relay else {
            return Vec::new();
        };
        match relay.query_capability(capability).await {
            Ok(cards) => cards
                .into_iter()
                .filter(|card| card.instance_id != self.deps.instance_id)
                .collect(),
            Err(e) => {
                log::warn!("Couldn't ask the relay who serves '{capability}': {e}");
                Vec::new()
            }
        }
    }

    /// Local and remote providers, deduplicated by card ID (local first),
    /// without instances we paused delegations with or agents we blocked
    /// (Track E), so neither ranking nor failover ever picks them.
    pub async fn providers(&self, capability: &str) -> Vec<AgentCard> {
        let mut cards = self.local_cards(capability);
        let mut seen: HashSet<String> = cards.iter().map(|c| c.id.clone()).collect();
        for card in self.remote_cards(capability).await {
            if seen.insert(card.id.clone()) {
                cards.push(card);
            }
        }
        failover::without_untrusted(&self.deps.db, &self.deps.instance_id, cards)
    }

    /// The card a task is sent as: the local agent's, or the user's.
    fn requester_card(&self, agent_id: Option<&str>) -> Result<AgentCard, String> {
        let instance = &self.deps.instance_id;
        let now = db::now_ms() as f64;
        let card = match agent_id {
            // The gateway passing a task it routed to a local agent
            Some(GATEWAY_AGENT_ID) => AgentCard {
                id: AgentCard::card_id(instance, GATEWAY_AGENT_ID),
                name: "Gateway".to_string(),
                description: String::new(),
                instance_id: instance.clone(),
                endpoint: String::new(),
                capabilities: Vec::new(),
                accepts_delegation: false,
                max_concurrent: 0,
                current_load: 0,
                owner: OwnerRef::default(),
                signature: None,
                issued_at: now,
            },
            Some(agent_id) => {
                let agent = self.deps.store().agent(agent_id)?;
                card_from_agent(&agent, instance, &self.endpoint(agent_id))
            }
            None => AgentCard {
                id: AgentCard::card_id(instance, USER_REQUESTER),
                name: Some(self.owner().display_name)
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Nandi user".to_string()),
                description: String::new(),
                instance_id: instance.clone(),
                endpoint: String::new(),
                capabilities: Vec::new(),
                accepts_delegation: false,
                max_concurrent: 0,
                current_load: 0,
                owner: OwnerRef::default(),
                signature: None,
                issued_at: now,
            },
        };
        Ok(card.with_owner(self.owner()).with_issued_at(now))
    }

    fn owner(&self) -> OwnerRef {
        self.deps.owner()
    }

    /// Runs one delegation to its end. Failures (no provider, rejected,
    /// timed out) are a `TaskResult` with that status, recorded like any
    /// other; only an empty capability is an `Err`.
    pub async fn run(&self, request: DelegationRequest) -> Result<TaskResult, String> {
        let capability = request.capability.trim().to_string();
        if capability.is_empty() {
            return Err("Choose a capability to delegate".to_string());
        }
        let task_id = request
            .task_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut result = history::placeholder(
            &task_id,
            &capability,
            &request.input,
            request.target.as_deref(),
        );
        let requester = match self.requester_card(request.requester_agent.as_deref()) {
            Ok(card) => card,
            Err(e) => {
                history::finish(&mut result, TaskStatus::Failed, Some(e));
                self.recorder.finished(&result).await;
                return Ok(result);
            }
        };
        result.requester_agent = requester.id.clone();
        result.requester_owner = non_empty(&requester.owner.github_handle);

        let candidates = match self.candidates(&request, &capability, &requester).await {
            Ok(candidates) => candidates,
            Err(e) => {
                history::finish(&mut result, TaskStatus::Failed, Some(e));
                self.recorder.finished(&result).await;
                return Ok(result);
            }
        };

        let base_id = result.task_id.clone();
        let service = task::service_for(&self.deps.instance_id);
        let requester_agent = request
            .requester_agent
            .clone()
            .unwrap_or_else(|| USER_REQUESTER.to_string());
        let mut queue: VecDeque<_> = candidates.into();
        // Instances that failed this delegation (never the local one)
        let mut excluded: HashSet<String> = HashSet::new();
        let mut previous_remote: Option<TaskRecord> = None;
        let lasting = assignment::is_lasting(&capability);
        let mut attempt = 0;
        while let Some((target, card)) = queue.pop_front() {
            let target_instance = instance_of(&target).to_string();
            let local = target_instance == self.deps.instance_id;
            if excluded.contains(&target_instance) {
                continue;
            }
            // Whether another provider could take it after this one
            let more = !lasting
                && attempt + 1 < MAX_ATTEMPTS
                && queue.iter().any(|(next, _)| {
                    let instance = instance_of(next);
                    !excluded.contains(instance) && (local || instance != target_instance)
                });
            result.provider_agent = target.clone();
            result.provider_owner = card
                .as_ref()
                .and_then(|card| non_empty(&card.owner.github_handle));
            result.output = Value::Null;
            result.tokens_in = 0;
            result.tokens_out = 0;
            result.status = TaskStatus::Running;
            result.error = None;
            result.ended_at = None;
            result.started_at = db::now_ms() as f64;

            let now = db::now_ms() as f64;
            let task = Task {
                // A fresh ID on the wire, so a late reply from a provider we
                // gave up on can't answer the retry
                id: if attempt == 0 {
                    base_id.clone()
                } else {
                    format!("{base_id}-retry{attempt}")
                },
                requester: requester.clone(),
                target: target.clone(),
                capability: capability.clone(),
                input: request.input.clone(),
                created_at: now,
                // A request to another Nandi is due days out (an
                // assignment, a question, anything); a local one gets the wait
                deadline: Some(if lasting || !local {
                    assignment::request_due_at(&capability, &request.input, now as i64) as f64
                } else {
                    now + self.deps.timeout.as_millis() as f64
                }),
            };
            self.recorder.started(&result).await;

            let (status, error) = if local && lasting {
                self.assign_locally(&task, &mut result).await
            } else if local {
                let provided = self.executor.execute(&task, false).await;
                result.output = provided.output;
                result.tokens_in = provided.tokens_in;
                result.tokens_out = provided.tokens_out;
                (provided.status, provided.error)
            } else {
                let record = self
                    .attempt_record(&task, &requester_agent, previous_remote.as_ref())
                    .await;
                let outcome = self
                    .remote(&task, &mut result, !more, service.as_deref())
                    .await;
                previous_remote = Some(record);
                outcome
            };
            attempt += 1;
            // An assignment reached its person: it stays open (and so does
            // its history row) until they end it
            if status == TaskStatus::Running {
                return Ok(result);
            }
            let did_work =
                !result.output.is_null() || result.tokens_in > 0 || result.tokens_out > 0;
            let retry = ranking::retryable(status, did_work);
            // Wave A: the provider's own circuit (an open one keeps it out
            // of the capability-aware ranking)
            if let Some(selection) = service.as_ref().and_then(|s| s.selection.as_ref()) {
                let succeeded = status == TaskStatus::Completed;
                if succeeded || retry {
                    let provider = target
                        .strip_prefix(&format!("{target_instance}/"))
                        .unwrap_or(&target);
                    if let Err(e) = selection
                        .record_provider_outcome(&target_instance, provider, &capability, succeeded)
                        .await
                    {
                        log::debug!("Delegation {base_id}: provider circuit not updated: {e}");
                    }
                }
            }
            if retry && !local {
                excluded.insert(target_instance);
            }

            if more && retry {
                // Keep this attempt in the history, then hand the task on
                let mut failed = result.clone();
                failed.task_id = format!("{base_id}-attempt{attempt}");
                let reason = error.unwrap_or_else(|| status.as_str().to_string());
                history::finish(
                    &mut failed,
                    status,
                    Some(format!("{reason} (passed to the next provider)")),
                );
                self.recorder.finished(&failed).await;
                log::info!("Delegation {base_id}: '{target}' didn't take it ({reason}); trying the next provider");
                if let Some(selection) = service.as_ref().and_then(|s| s.selection.as_ref()) {
                    // Cardless entries (workflow providers) keep their turn
                    // after the re-ranked cards
                    let (carded, cardless): (Vec<_>, Vec<_>) =
                        queue.drain(..).partition(|(_, c)| c.is_some());
                    let cards: Vec<AgentCard> = carded.into_iter().filter_map(|(_, c)| c).collect();
                    let me = &self.deps.instance_id;
                    let ranked = failover::rank_next(
                        Some(selection),
                        &capability,
                        me,
                        &requester.id,
                        cards,
                        &excluded,
                    )
                    .await;
                    queue = ranked
                        .into_iter()
                        .map(|c| (c.id.clone(), Some(c)))
                        .chain(cardless)
                        .collect();
                }
                continue;
            }
            history::finish(&mut result, status, error);
            self.recorder.finished(&result).await;
            return Ok(result);
        }
        // Every candidate left was on an instance that already failed
        history::finish(
            &mut result,
            TaskStatus::Failed,
            Some(format!(
                "No other agent that serves '{capability}' could take it"
            )),
        );
        self.recorder.finished(&result).await;
        Ok(result)
    }

    /// Work assigned to this Nandi's own owner. Their gateway holds it for
    /// their approval and keeps it on their list until they end it, so it
    /// goes on in the background: `Running` once it's held or listed (or
    /// after [`assignment::ACCEPT_WINDOW`]), else the refusal that came
    /// first. The history row ends when the owner ends the assignment.
    async fn assign_locally(
        &self,
        task: &Task,
        result: &mut TaskResult,
    ) -> (TaskStatus, Option<String>) {
        let mut events = self.deps.broadcast.subscribe();
        let mut work = {
            let (executor, task) = (self.executor.clone(), task.clone());
            tokio::spawn(async move { executor.execute(&task, false).await })
        };
        let kept = tokio::time::timeout(assignment::ACCEPT_WINDOW, async {
            loop {
                tokio::select! {
                    provided = &mut work => return Err(provided),
                    event = events.recv() => match event {
                        Ok(event) => {
                            if let Some(state) = kept_state(&event, &task.id) {
                                return Ok(state);
                            }
                        }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => return Err((&mut work).await),
                    },
                }
            }
        })
        .await;
        let state = match kept {
            Ok(Ok(state)) => state,
            Err(_) => TaskState::Delivered,
            Ok(Err(provided)) => {
                let provided = provided_or_failed(task, provided);
                result.output = provided.output;
                result.tokens_in = provided.tokens_in;
                result.tokens_out = provided.tokens_out;
                return (provided.status, provided.error);
            }
        };
        result.output = json!({ "state": state.as_str(), "due": task.deadline });
        let (recorder, mut ended, task) = (self.recorder.clone(), result.clone(), task.clone());
        tokio::spawn(async move {
            let provided = provided_or_failed(&task, work.await);
            ended.output = provided.output;
            ended.tokens_in = provided.tokens_in;
            ended.tokens_out = provided.tokens_out;
            let status = match provided.status {
                // The owner ended it: whatever came back is final
                TaskStatus::Running => TaskStatus::Failed,
                status => status,
            };
            history::finish(&mut ended, status, provided.error);
            recorder.finished(&ended).await;
        });
        (TaskStatus::Running, None)
    }

    /// Ranking inputs: my pins, teammates, and the providers' history
    pub async fn rank_data(&self) -> RankData {
        RankData::gather(self.deps.store(), &self.deps.db, self.deps.teams.as_deref()).await
    }

    /// Who to try, in order (never empty): the named target alone, else
    /// the ranked providers (the run tries up to `MAX_ATTEMPTS`), without
    /// remote ones whose circuit is open.
    async fn candidates(
        &self,
        request: &DelegationRequest,
        capability: &str,
        requester: &AgentCard,
    ) -> Result<Vec<(String, Option<AgentCard>)>, String> {
        if let Some(target) = &request.target {
            if *target == requester.id {
                return Err("An agent can't delegate a task to itself".to_string());
            }
            let instance = instance_of(target);
            if instance != self.deps.instance_id {
                unavailable::configured(self.deps.relay.as_ref())?;
                let who = member_name(&self.deps.db, instance);
                if member_trust::is_paused(&self.deps.db, instance) {
                    return Err(format!("You paused requests with {who}"));
                }
                if member_trust::is_blocked(&self.deps.db, instance, target) {
                    return Err(format!("You blocked that agent of {who}"));
                }
            }
            let card = self
                .providers(capability)
                .await
                .into_iter()
                .find(|card| card.id == *target);
            return Ok(vec![(target.clone(), card)]);
        }
        let cards = self.providers(capability).await;
        let data = self.rank_data().await;
        let ctx = data.context(&self.deps.instance_id, &requester.id, capability);
        let mut ranked = Vec::new();
        for card in ranking::rank_providers(&cards, capability, &ctx) {
            let remote = card.instance_id != self.deps.instance_id;
            if remote
                && failover::is_circuit_open(&self.deps.db, &card.instance_id, capability)
                    .await
                    .unwrap_or(false)
            {
                continue;
            }
            ranked.push((card.id.clone(), Some(card.clone())));
        }
        // Wave A: providers without a required tool, or with an open
        // provider circuit, drop out; workflow providers join
        let selection = task::service_for(&self.deps.instance_id).and_then(|s| s.selection.clone());
        let ranked = failover::capability_order(
            selection.as_ref(),
            capability,
            &self.deps.instance_id,
            &HashSet::new(),
            ranked,
        )
        .await;
        if ranked.is_empty() {
            return Err(no_provider_reason(capability, &cards, &requester.id));
        }
        Ok(ranked)
    }

    /// The requester `tasks` row of a cross-instance attempt (with the wire
    /// task), linked to the previous remote attempt
    async fn attempt_record(
        &self,
        task: &Task,
        requester_agent: &str,
        previous: Option<&TaskRecord>,
    ) -> TaskRecord {
        let fresh = TaskRecord::requester(
            &task.id,
            &self.deps.instance_id,
            requester_agent,
            &task.target,
            &task.capability,
            &task.input,
            task.deadline,
        );
        let record = match previous {
            Some(previous) => {
                let mut next = failover::next_attempt(previous, &task.id, &task.target);
                next.source_kind = TaskSourceKind::Failover;
                next.deadline = task.deadline;
                next
            }
            None => fresh,
        };
        if let Err(e) = task::create(&self.deps.db, &self.deps.bus, &record, Some(task)).await {
            log::error!("Failed to record task {}: {e}", task.id);
        }
        record
    }

    /// Moves our row of `task_id` toward `to`, logging failures
    async fn step(&self, task_id: &str, to: TaskState, change: Change) -> Option<TaskRecord> {
        match task_state::advance(&self.deps.db, &self.deps.bus, task_id, to, change).await {
            Ok(moved) => moved,
            Err(e) => {
                log::warn!("Task {task_id}: {e}");
                None
            }
        }
    }

    /// Sends `task` through the relay and waits for its result. Fills in
    /// the output and tokens; returns the status and error. `last`: no
    /// other provider would get it, so an undeliverable task waits in the
    /// offline queue instead of failing.
    async fn remote(
        &self,
        task: &Task,
        result: &mut TaskResult,
        last: bool,
        service: Option<&TaskService>,
    ) -> (TaskStatus, Option<String>) {
        // Without a relay the request could never leave: say so now
        // instead of queueing it forever. A configured relay that is
        // reconnecting keeps it in the offline queue.
        let Ok(relay) = unavailable::configured(self.deps.relay.as_ref()) else {
            let error = RelayUnavailable.to_string();
            self.step(
                &task.id,
                TaskState::Abandoned,
                Change::error(error.clone()).kind(ReasonKind::Offline, TaskStage::Dispatch),
            )
            .await;
            return (TaskStatus::Failed, Some(error));
        };
        if let Some(service) = service {
            // A reassigned attempt's waiter moves to its successor
            let pending = Arc::downgrade(&self.pending);
            service.set_rekey_hook(|| {
                Box::new(move |from: &str, to: &str| {
                    if let Some(pending) = pending.upgrade() {
                        pending.rekey(from, to);
                    }
                })
            });
            service.mark_live(&task.id);
        }
        let mut followed = Vec::new();
        let outcome = self
            .remote_attempt(relay, task, result, last, &mut followed)
            .await;
        if let Some(service) = service {
            service.unmark_live(&task.id);
            for id in &followed {
                service.unmark_live(id);
            }
        }
        outcome
    }

    async fn remote_attempt(
        &self,
        relay: &Arc<dyn RelayClient>,
        task: &Task,
        result: &mut TaskResult,
        last: bool,
        followed: &mut Vec<String>,
    ) -> (TaskStatus, Option<String>) {
        // Before sending: the result may come back before `delegate_task` does
        let mut waiting = self.pending.add(&task.id);
        match relay.delegate_task(task).await {
            Ok(routed) => {
                if routed != task.id {
                    self.pending.rekey(&task.id, &routed);
                }
                self.step(
                    &task.id,
                    TaskState::Delivered,
                    Change::reason("routed by the relay").at(TaskStage::Dispatch),
                )
                .await;
            }
            Err(e) if !last => {
                self.pending.forget(&task.id);
                let error = format!("The network couldn't deliver the request: {e}");
                self.step(
                    &task.id,
                    TaskState::Abandoned,
                    Change::error(error.clone()).kind(ReasonKind::Offline, TaskStage::Dispatch),
                )
                .await;
                return (TaskStatus::Failed, Some(error));
            }
            // Queued: `TaskService::flush` sends it when the provider is back
            Err(e) => log::info!("Task {} waits in the offline queue: {e}", task.id),
        }

        let (db, bus) = (&self.deps.db, &self.deps.bus);
        let lasting = assignment::is_lasting(&task.capability);
        // A provider may hand the task on (wave EF): wait on the attempt
        // the requester's task service made for the new provider
        let mut current = task.clone();
        let waited = loop {
            match self.wait(&current.id, &mut waiting, lasting).await {
                Waited::Ended(row) if row.state == TaskState::Reassigned => {
                    let Some(next) = reassigned_to(db, &row.id).await else {
                        break Waited::Ended(row);
                    };
                    self.pending.rekey(&current.id, &next.id);
                    followed.push(next.id.clone());
                    current.target = next.provider_card();
                    current.id = next.id;
                }
                waited => break waited,
            }
        };
        // A result may reach the moved waiter before the poll above saw the
        // handoff: it belongs to the newest attempt
        if matches!(waited, Waited::Result(_)) {
            while let Some(next) = reassigned_to(db, &current.id).await {
                followed.push(next.id.clone());
                current.target = next.provider_card();
                current.id = next.id;
            }
        }
        if current.id != task.id {
            // Handed on: the provider that took it over is the one answering
            result.provider_agent = current.target.clone();
            result.provider_owner = None;
        }
        let task = &current;
        let (status, output, error) = match waited {
            Waited::Result(Ok(provided)) => {
                result.tokens_in = provided.tokens_in;
                result.tokens_out = provided.tokens_out;
                let status = match provided.status {
                    // A result is final, whatever the provider said
                    TaskStatus::Running => TaskStatus::Failed,
                    status => status,
                };
                (status, provided.output, provided.error)
            }
            Waited::Result(Err(e)) => (TaskStatus::Failed, Value::Null, Some(e)),
            Waited::Closed => (
                TaskStatus::Failed,
                Value::Null,
                Some("The relay connection closed before a result came".to_string()),
            ),
            // Ended here (cancelled, timed out or abandoned by a sweep,
            // rejected): already recorded
            Waited::Ended(row) => {
                self.pending.forget(&task.id);
                let error = row
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("The task ended as {}", row.state.as_str()));
                let status = match row.state {
                    TaskState::Completed => TaskStatus::Completed,
                    TaskState::Cancelled => TaskStatus::Cancelled,
                    TaskState::TimedOut => TaskStatus::TimedOut,
                    _ => TaskStatus::Failed,
                };
                if status == TaskStatus::Completed {
                    result.output = row.result.unwrap_or(Value::Null);
                    return (status, None);
                }
                return (status, Some(error));
            }
            Waited::TimedOut(row) => {
                self.pending.forget(&task.id);
                // No answer yet, nobody else to ask, and not due yet: it
                // stays open (queued for them, or with them) and the answer
                // comes later, to the task service (`on_result`)
                if last {
                    if let Some(row) = row.as_ref().filter(|row| still_due(row)) {
                        result.output = json!({
                            "state": row.state.as_str(),
                            "due": row.deadline,
                            "waiting": true,
                        });
                        return (TaskStatus::Running, None);
                    }
                }
                return self.gave_up(relay, task, row).await;
            }
            // The result comes later, to the task service (`on_result`)
            Waited::Accepted(row) => {
                self.pending.forget(&task.id);
                result.output = json!({
                    "state": row.state.as_str(),
                    "due": row.deadline,
                });
                return (TaskStatus::Running, None);
            }
        };
        result.output = output.clone();
        if let Err(e) =
            task::finish_requester(db, bus, &task.id, status, output, error.clone()).await
        {
            log::warn!("Task {}: {e}", task.id);
        }
        (status, error)
    }

    /// Waits for the result, watching our row: it may end meanwhile (a
    /// cancel, a sweep), and once acknowledged the deadline stretches to
    /// `TIMEOUT_ETA_FACTOR` × the ETA. A `lasting` task (an assignment)
    /// never times out here: it's `Accepted` once the provider took it on,
    /// or after [`assignment::ACCEPT_WINDOW`] without a refusal.
    async fn wait(
        &self,
        task_id: &str,
        waiting: &mut oneshot::Receiver<Result<TaskResult, String>>,
        lasting: bool,
    ) -> Waited {
        let timeout = self.deps.timeout;
        let tick = (timeout / 4).clamp(Duration::from_millis(10), Duration::from_millis(500));
        let started = db::now_ms();
        let timeout_ms = timeout.as_millis() as i64;
        loop {
            tokio::select! {
                outcome = &mut *waiting => {
                    return match outcome {
                        Ok(result) => Waited::Result(result),
                        Err(_) => Waited::Closed,
                    };
                }
                _ = tokio::time::sleep(tick) => {}
            }
            let row = task::get_row(&self.deps.db, task_id).await.ok().flatten();
            if let Some(row) = row.as_ref().filter(|r| r.state.is_terminal()) {
                return Waited::Ended(row.clone());
            }
            if lasting {
                let Some(row) = row else { continue };
                let taken = matches!(
                    row.state,
                    TaskState::Acknowledged | TaskState::Working | TaskState::InputRequired
                );
                let window = assignment::ACCEPT_WINDOW.as_millis() as i64;
                if taken || db::now_ms() >= started + window {
                    return Waited::Accepted(row);
                }
                continue;
            }
            let mut deadline = started + timeout_ms;
            if let Some(row) = &row {
                if let (Some(acknowledged), true) = (
                    row.acknowledged_at,
                    matches!(
                        row.state,
                        TaskState::Acknowledged | TaskState::Working | TaskState::InputRequired
                    ),
                ) {
                    let eta = row.eta_seconds.unwrap_or(eta::DEFAULT_MEDIAN_SECONDS) as i64;
                    let limit = TIMEOUT_ETA_FACTOR as i64 * eta * 1000 + tick.as_millis() as i64;
                    deadline = deadline.max(acknowledged as i64 + limit);
                }
            }
            // Waiting for an answer to the provider's questions (escalated to
            // our human, perhaps): the clarification's own deadline ends it
            let answering = row
                .as_ref()
                .is_some_and(|r| r.state == TaskState::InputRequired);
            if !answering && db::now_ms() >= deadline {
                return Waited::TimedOut(row);
            }
        }
    }

    /// The wait ran out. A still-queued task stays in the queue; a sent one
    /// ends (`Cancelled` before the provider acknowledged it, else
    /// `TimedOut`) and the provider is told.
    async fn gave_up(
        &self,
        relay: &Arc<dyn RelayClient>,
        task: &Task,
        row: Option<TaskRecord>,
    ) -> (TaskStatus, Option<String>) {
        let mut error = format!(
            "No result from {} within {}s",
            member_name(&self.deps.db, instance_of(&task.target)),
            self.deps.timeout.as_secs_f32()
        );
        let state = row.as_ref().map(|r| r.state);
        let provider = instance_of(&task.target).to_string();
        let body = match state {
            Some(TaskState::Submitted) => {
                error.push_str(" (it stays queued until the provider is back online)");
                None
            }
            Some(TaskState::Delivered) => {
                let reason = "the requester stopped waiting".to_string();
                self.step(
                    &task.id,
                    TaskState::Cancelled,
                    Change::error(error.clone()).kind(ReasonKind::TimedOut, TaskStage::Result),
                )
                .await;
                Some(PeerMessage::CancelTask {
                    task_id: task.id.clone(),
                    reason: Some(reason),
                })
            }
            Some(_) => {
                self.step(
                    &task.id,
                    TaskState::TimedOut,
                    Change::error(error.clone()).kind(ReasonKind::TimedOut, TaskStage::Result),
                )
                .await;
                if let Err(e) = failover::record_failure(
                    &self.deps.db,
                    &self.deps.bus,
                    &provider,
                    &task.capability,
                )
                .await
                {
                    log::warn!("Task {}: {e}", task.id);
                }
                Some(PeerMessage::TaskTimedOut {
                    task_id: task.id.clone(),
                    reason: Some(error.clone()),
                })
            }
            None => None,
        };
        if let Some(body) = body {
            if let Err(e) = relay.send_peer(provider, body).await {
                log::debug!("Task {}: {e}", task.id);
            }
        }
        (TaskStatus::TimedOut, Some(error))
    }
}

/// Whether `row` is open and its due date hasn't passed
fn still_due(row: &TaskRecord) -> bool {
    !row.state.is_terminal() && row.deadline.is_some_and(|due| due as i64 > db::now_ms())
}

/// How waiting for a remote result ended
/// The attempt the requester's task service made when the provider of
/// `task_id` reassigned it
async fn reassigned_to(db: &Db, task_id: &str) -> Option<TaskRecord> {
    let id = task_id.to_string();
    db.call(move |conn| {
        task::select(
            conn,
            "previous_attempt_id = ?1 AND source_kind = 'reassign'",
            &[&id],
        )
    })
    .await
    .ok()
    .and_then(|rows| rows.into_iter().next())
}

enum Waited {
    Result(Result<TaskResult, String>),
    /// The waiter was dropped (the relay connection went away)
    Closed,
    /// Our row ended without a result (a cancel, a sweep, a rejection)
    Ended(TaskRecord),
    /// The deadline passed; the row as of then
    TimedOut(Option<TaskRecord>),
    /// A lasting task (an assignment) is with its provider: the result
    /// comes later
    Accepted(TaskRecord),
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

impl A2ADelegator for Delegator {
    fn delegate<'a>(
        &'a self,
        capability: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(self.run(DelegationRequest {
            capability: capability.to_string(),
            input,
            ..Default::default()
        }))
    }

    fn delegate_to<'a>(
        &'a self,
        target: &'a str,
        capability: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(self.run(DelegationRequest {
            capability: capability.to_string(),
            input,
            target: Some(target.to_string()),
            ..Default::default()
        }))
    }

    fn list_providers<'a>(
        &'a self,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
        Box::pin(async move { Ok(self.providers(capability).await) })
    }

    fn delegate_request(
        &self,
        request: DelegationRequest,
    ) -> BoxFuture<'_, Result<TaskResult, String>> {
        Box::pin(self.run(request))
    }
}

/// Why no provider can take `capability`: the opt-in that's missing when
/// every agent that serves it has `accepts_delegation` off
/// Where the owner's gateway put local assignment `task_id`, if `event`
/// says: held for their approval (`Delivered`) or on their list
/// (`Acknowledged`)
fn kept_state(event: &WireEvent, task_id: &str) -> Option<TaskState> {
    if event.event == GatewayHoldChanged::NAME {
        let changed: GatewayHoldChanged = serde_json::from_value(event.payload.clone()).ok()?;
        (changed.hold.task_id == task_id && changed.hold.status == HoldStatus::Pending)
            .then_some(TaskState::Delivered)
    } else if event.event == GatewayAssignmentChanged::NAME {
        let changed: GatewayAssignmentChanged =
            serde_json::from_value(event.payload.clone()).ok()?;
        (changed.assignment.task_id == task_id).then_some(TaskState::Acknowledged)
    } else {
        None
    }
}

/// A local run's result, or a failed one if its task panicked
fn provided_or_failed(
    task: &Task,
    joined: Result<TaskResult, tokio::task::JoinError>,
) -> TaskResult {
    joined.unwrap_or_else(|e| {
        let mut failed =
            history::placeholder(&task.id, &task.capability, &task.input, Some(&task.target));
        history::finish(
            &mut failed,
            TaskStatus::Failed,
            Some(format!("The assignment stopped unexpectedly: {e}")),
        );
        failed
    })
}

/// A member's display name, from the network roster (never a device ID)
fn member_name(db: &Db, instance: &str) -> String {
    crate::services::network::known_members(db)
        .into_iter()
        .find(|m| m.instance_id == instance)
        .map(|m| m.display_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "the other Nandi".to_string())
}

fn no_provider_reason(capability: &str, cards: &[AgentCard], requester: &str) -> String {
    let mut serving = cards
        .iter()
        .filter(|card| card.serves(capability) && card.id != requester)
        .peekable();
    if serving.peek().is_some() && serving.all(|card| !card.accepts_delegation) {
        format!(
            "The agents that offer '{capability}' don't take requests from other Nandis yet. \
             Their owner can turn that on in the agent's settings."
        )
    } else {
        format!("No agent that serves '{capability}' is online and accepts delegated tasks")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent_card::OwnerRef;

    fn card(id: &str, load: u32, issued_at: f64) -> AgentCard {
        let (instance, _) = id.split_once('/').unwrap();
        AgentCard {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            instance_id: instance.into(),
            endpoint: String::new(),
            capabilities: vec![crate::services::domain::capability::ServesCapability {
                name: "summarize".into(),
                description: String::new(),
                input_schema: None,
                output_schema: None,
            }],
            accepts_delegation: true,
            max_concurrent: 4,
            current_load: load,
            owner: OwnerRef::default(),
            signature: None,
            issued_at,
        }
    }

    fn pick(cards: &[AgentCard]) -> &str {
        let ctx = ranking::RankContext::bare("me", "me/requester");
        ranking::choose_provider(cards, "summarize", &ctx)
            .map(|c| c.id.as_str())
            .unwrap_or("none")
    }

    #[test]
    fn the_local_instance_wins_ties() {
        let cards = [card("them/a", 0, 5.0), card("me/b", 0, 1.0)];
        assert_eq!(pick(&cards), "me/b");
    }

    #[test]
    fn lower_load_breaks_ties() {
        let cards = [
            card("them/busy", 2, 9.0),
            card("them/idle", 0, 1.0),
            card("other/some", 1, 9.0),
        ];
        assert_eq!(pick(&cards), "them/idle");
    }

    #[test]
    fn fresher_heartbeat_breaks_ties() {
        let cards = [card("them/stale", 1, 100.0), card("other/fresh", 1, 200.0)];
        assert_eq!(pick(&cards), "other/fresh");
    }

    #[test]
    fn full_closed_wrong_and_self_are_skipped() {
        let mut full = card("me/full", 4, 1.0);
        full.max_concurrent = 4;
        let mut closed = card("me/closed", 0, 1.0);
        closed.accepts_delegation = false;
        let mut other = card("me/other", 0, 1.0);
        other.capabilities[0].name = "translate".into();
        let requester = card("me/requester", 0, 1.0);
        assert_eq!(pick(&[full, closed, other, requester]), "none");
    }
}
