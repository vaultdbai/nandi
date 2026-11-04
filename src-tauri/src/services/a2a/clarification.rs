//! Bounded clarification (wave final, Track C). Provider side: the
//! `ask_requester` tool, offered only during a delegated turn, sends a
//! `ClarificationRequest` and blocks the turn until the answer or timeout.
//! Requester side: each question is classified against the requester
//! agent's `ClarificationPolicy` (escalate topic or `requires_human` →
//! the human; auto topic → the agent's LLM; anything else → the human) and
//! the answers go back as a `ClarificationResponse`. Every decision is
//! logged to `~/.nandi/local/logs/clarification.log` (JSONL).
//!
//! Decisions:
//! - One open round per task on the requester side. The response goes
//!   back once every question has an answer (auto answers wait for the
//!   escalated ones, so the provider gets one response per round).
//! - An auto answer the model can't give (no key, an error, `UNKNOWN`)
//!   escalates the question instead: the safe default.
//! - `!allow` answers every question with "Clarification not allowed" and
//!   fails the task; a round past `max_rounds` fails it and cancels the
//!   provider's task (`CancelTask`).
//! - The provider enforces the default `max_rounds` too, so a requester
//!   that never says "too many" still bounds the conversation.
//! - Tasks on this instance (local delegation) are answered in-process;
//!   others go through the relay (`RelayClient::send_peer`).
//! - The human answers through `respond_to_clarification`. Besides text,
//!   an answer may be `DENY_ANSWER` (cancel the task), `ALLOW_ONCE_ANSWER`
//!   (let the agent answer this one), or `ALLOW_ALWAYS_ANSWER` (also add
//!   the topic to the policy's `auto_topics`).
//! - Task state changes go through Track B's state machine
//!   (`task_state::advance`, via `transition`): the `tasks` row and its
//!   `task_events` rows in one transaction, then `task://state_changed`.
//!   Tasks without a row, or already ended, are left alone.
//! - Open rounds live in memory: a restart drops them (the provider's wait
//!   then times out).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::delegator::{instance_of, USER_REQUESTER};
use super::escalation;
use super::task::{self, TaskState};
use super::task_state::{self, Change};
use super::{
    ClarificationPolicy, ClarificationRequest, ClarificationResponse, PendingClarification,
    Question,
};
use crate::db::{self, Db};
use crate::llm::{self, CallSource, LlmRequest, Message};
use crate::model::{ApprovalStatus, SourceType};
use crate::services::domain::{Agent, AgentCard, EntityKind};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::llm_keys;
use crate::services::network;
use crate::services::relay::protocol::{PeerDelivery, PeerMessage};
use crate::services::relay::RelayClient;
use crate::services::repo_store::RepoStore;
use crate::services::repos::Repository;
use crate::services::secrets::Secrets;
use crate::services::AppState;
use crate::utils::nandi_config;

/// The provider-side tool name
pub const ASK_REQUESTER_TOOL: &str = "ask_requester";
/// `task://clarification_requested`
pub const CLARIFICATION_REQUESTED: &str = "task://clarification_requested";
/// `task://clarification_answered`
pub const CLARIFICATION_ANSWERED: &str = "task://clarification_answered";

/// skill.md key of the policy
pub const POLICY_KEY: &str = "clarification";
/// Sent for every question when the policy doesn't allow clarification
pub const NOT_ALLOWED_ANSWER: &str = "Clarification not allowed";
/// Human answers with a meaning (see the module docs)
pub const DENY_ANSWER: &str = "__nandi:deny__";
pub const ALLOW_ONCE_ANSWER: &str = "__nandi:allow_once__";
pub const ALLOW_ALWAYS_ANSWER: &str = "__nandi:allow_always__";
/// What the model replies when the task doesn't tell it the answer
const UNKNOWN_REPLY: &str = "UNKNOWN";

/// Questions per round
pub const MAX_QUESTIONS: usize = 10;
const MAX_QUESTION_CHARS: usize = 2000;
const MAX_TOPICS: usize = 50;
const MAX_ROUNDS_LIMIT: u32 = 10;
const MIN_TIMEOUT_SECS: u32 = 1;
const MAX_TIMEOUT_SECS: u32 = 7 * 24 * 3600;
/// How long a provider turn waits for an answer at most: the requester's
/// own timeout normally ends the wait first (`ClarificationTimeout`)
pub const DEFAULT_PROVIDER_WAIT: Duration = Duration::from_secs(30 * 60);
const MAX_ANSWER_TOKENS: u32 = 1024;
const LOG_FILE_NAME: &str = "clarification.log";

const ANSWER_PROMPT: &str = "You delegated a task to another agent, and it asks you a question \
before it goes on. Answer from the task you sent and your own instructions only, in one short \
paragraph. If they don't tell you the answer, reply with exactly UNKNOWN and nothing else: a \
person will answer instead. Never invent facts, credentials, or approvals.";

/// Where an incoming question goes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The requester agent's LLM answers
    Auto,
    /// A person answers (an `approvals` row)
    Escalate,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Escalate => "escalate",
        }
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// The policy as skill.md frontmatter (snake_case, every key optional)
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct PolicyYaml {
    allow: bool,
    max_rounds: u32,
    auto_topics: Vec<String>,
    escalate_topics: Vec<String>,
    timeout_seconds: u32,
}

impl Default for PolicyYaml {
    fn default() -> Self {
        ClarificationPolicy::default().into()
    }
}

impl From<ClarificationPolicy> for PolicyYaml {
    fn from(p: ClarificationPolicy) -> Self {
        Self {
            allow: p.allow,
            max_rounds: p.max_rounds,
            auto_topics: p.auto_topics,
            escalate_topics: p.escalate_topics,
            timeout_seconds: p.timeout_seconds,
        }
    }
}

impl From<PolicyYaml> for ClarificationPolicy {
    fn from(p: PolicyYaml) -> Self {
        Self {
            allow: p.allow,
            max_rounds: p.max_rounds,
            auto_topics: p.auto_topics,
            escalate_topics: p.escalate_topics,
            timeout_seconds: p.timeout_seconds,
        }
    }
}

/// The agent's policy (the default when skill.md has none or it's invalid)
pub fn policy_of(agent: &Agent) -> ClarificationPolicy {
    agent
        .extra()
        .get(POLICY_KEY)
        .cloned()
        .and_then(|value| serde_yaml::from_value::<PolicyYaml>(value).ok())
        .map(ClarificationPolicy::from)
        .and_then(|policy| normalize(policy).ok())
        .unwrap_or_default()
}

fn clean_topics(topics: Vec<String>) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    for topic in topics {
        let topic = topic.trim().to_string();
        if !topic.is_empty() && !seen.iter().any(|t| t.eq_ignore_ascii_case(&topic)) {
            seen.push(topic);
        }
    }
    seen
}

/// Trims and de-duplicates topics; checks the bounds
pub fn normalize(policy: ClarificationPolicy) -> Result<ClarificationPolicy, String> {
    if policy.max_rounds > MAX_ROUNDS_LIMIT {
        return Err(format!(
            "At most {MAX_ROUNDS_LIMIT} rounds of questions are allowed"
        ));
    }
    if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&policy.timeout_seconds) {
        return Err(format!(
            "The timeout must be between {MIN_TIMEOUT_SECS} second and {} days",
            MAX_TIMEOUT_SECS / 86_400
        ));
    }
    let auto_topics = clean_topics(policy.auto_topics);
    let escalate_topics = clean_topics(policy.escalate_topics);
    if auto_topics.len() + escalate_topics.len() > MAX_TOPICS {
        return Err(format!("At most {MAX_TOPICS} topics are allowed"));
    }
    Ok(ClarificationPolicy {
        auto_topics,
        escalate_topics,
        ..policy
    })
}

/// The agent's policy, from its skill.md
pub fn read_policy(store: &RepoStore, agent_id: &str) -> Result<ClarificationPolicy, String> {
    Ok(policy_of(&store.agent(agent_id)?))
}

/// Writes the policy into the agent's skill.md (the gold copy), then
/// projects the change. The default policy removes the key.
pub fn write_policy(
    store: &RepoStore,
    agent_id: &str,
    policy: ClarificationPolicy,
) -> Result<ClarificationPolicy, String> {
    let policy = normalize(policy)?;
    let mut agent = store.agent(agent_id)?;
    let value = if policy == ClarificationPolicy::default() {
        None
    } else {
        Some(serde_yaml::to_value(PolicyYaml::from(policy.clone())).map_err(|e| e.to_string())?)
    };
    agent.set_extra(POLICY_KEY, value);
    store
        .repos()
        .agents
        .save(&agent)
        .map_err(|e| e.to_string())?;
    store.commit(EntityKind::Agent, agent_id)?;
    Ok(policy)
}

/// For Track F's policy editor (command and client method requested)
#[allow(dead_code)]
pub async fn get_clarification_policy(
    state: &AppState,
    agent_id: &str,
) -> Result<ClarificationPolicy, String> {
    read_policy(state.store(), agent_id)
}

/// For Track F's policy editor (command and client method requested)
#[allow(dead_code)]
pub async fn set_clarification_policy(
    state: &AppState,
    agent_id: &str,
    policy: ClarificationPolicy,
) -> Result<ClarificationPolicy, String> {
    write_policy(state.store(), agent_id, policy)
}

/// Case-insensitive substring match, either way round
fn topic_matches(topic: &str, topics: &[String]) -> bool {
    let topic = topic.trim().to_lowercase();
    if topic.is_empty() {
        return false;
    }
    topics
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .any(|t| t.contains(&topic) || topic.contains(&t))
}

/// Where a question goes, and why (for the log)
pub fn classify_with_reason(
    policy: &ClarificationPolicy,
    question: &Question,
) -> (Route, &'static str) {
    if question.requires_human {
        (Route::Escalate, "requires_human")
    } else if topic_matches(&question.topic, &policy.escalate_topics) {
        (Route::Escalate, "escalate_topic")
    } else if topic_matches(&question.topic, &policy.auto_topics) {
        (Route::Auto, "auto_topic")
    } else {
        (Route::Escalate, "default")
    }
}

pub fn classify(policy: &ClarificationPolicy, question: &Question) -> Route {
    classify_with_reason(policy, question).0
}

// ---------------------------------------------------------------------------
// The decision log
// ---------------------------------------------------------------------------

/// JSONL, one line per decision. Holds no answers or question text.
#[derive(Debug, Clone, Default)]
pub struct ClarificationLog {
    path: Option<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry<'a> {
    timestamp: i64,
    task_id: &'a str,
    round: u32,
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    question_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

/// Serializes writers, so lines never interleave
static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl ClarificationLog {
    #[allow(dead_code)] // Tests
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// `~/.nandi/local/logs/clarification.log` (none in tests)
    pub fn default_location() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        Self {
            path: nandi_config::logs_dir()
                .ok()
                .map(|dir| dir.join(LOG_FILE_NAME)),
        }
    }

    #[allow(dead_code)] // Tests
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Appends one entry; failures are logged, never returned
    pub async fn record(
        &self,
        task_id: &str,
        round: u32,
        event: &str,
        question_id: Option<&str>,
        detail: Option<&str>,
    ) {
        let Some(path) = &self.path else {
            return;
        };
        let entry = LogEntry {
            timestamp: db::now_ms(),
            task_id,
            round,
            event,
            question_id,
            detail,
        };
        if let Err(e) = append(path, &entry).await {
            log::error!("Failed to write the clarification log: {e}");
        }
    }
}

async fn append(path: &Path, entry: &LogEntry<'_>) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let _guard = WRITE_LOCK.lock().await;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| e.to_string())?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Auto answers
// ---------------------------------------------------------------------------

/// What the requester agent is asked
#[derive(Debug, Clone, Copy)]
pub struct AnswerCall<'a> {
    pub agent_id: &'a str,
    pub capability: &'a str,
    /// The original task input
    pub task_input: &'a Value,
    pub question: &'a Question,
}

/// Answers a question as the requester agent. `None`: it can't, so a
/// person should.
pub trait Answerer: Send + Sync {
    fn answer<'a>(&'a self, call: AnswerCall<'a>) -> BoxFuture<'a, Result<Option<String>, String>>;
}

/// The agent's own model (`llm::complete`), scoped to the task input and
/// the question
pub struct LlmAnswerer {
    pub secrets: Secrets,
    pub store: RepoStore,
}

fn answer_prompt(call: &AnswerCall<'_>) -> String {
    let input = match call.task_input {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    format!(
        "You delegated a `{}` task with this input:\n\n{input}\n\nThe agent doing it asks \
         (topic: {}):\n\n{}",
        call.capability, call.question.topic, call.question.question
    )
}

impl Answerer for LlmAnswerer {
    fn answer<'a>(&'a self, call: AnswerCall<'a>) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let skill = self.store.skill(call.agent_id)?;
            llm_keys::usable_model(&self.secrets, &skill.model)?;
            let system = if skill.instructions.is_empty() {
                ANSWER_PROMPT.to_string()
            } else {
                format!("{}\n\n---\n\n{ANSWER_PROMPT}", skill.instructions)
            };
            let request = LlmRequest {
                model: skill.model.clone(),
                system: Some(system),
                messages: vec![Message::User {
                    content: answer_prompt(&call),
                }],
                tools: Vec::new(),
                max_tokens: Some(MAX_ANSWER_TOKENS),
            };
            let response = llm::complete(
                &self.secrets,
                &request,
                Some(call.agent_id),
                CallSource::Orchestrator,
            )
            .await?;
            let answer = response.content.trim();
            Ok((!answer.is_empty() && answer != UNKNOWN_REPLY).then(|| answer.to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// Task state (Track B's state machine)
// ---------------------------------------------------------------------------

/// Moves a task's row toward `to` along the legal path
/// (`task_state::advance`, each step recorded and announced), unless it has
/// no row, is already there, or has ended. Returns whether it moved.
pub async fn transition(
    db: &Db,
    bus: &EventBus,
    task_id: &str,
    to: TaskState,
    reason: &str,
) -> Result<bool, String> {
    if task::get_row(db, task_id).await?.is_none() {
        return Ok(false);
    }
    let change = if matches!(
        to,
        TaskState::Failed | TaskState::Cancelled | TaskState::TimedOut
    ) {
        Change::error(reason)
    } else {
        Change::reason(reason)
    };
    Ok(task_state::advance(db, bus, task_id, to, change)
        .await?
        .is_some())
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Payload of `task://clarification_requested`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClarificationRequested {
    task_id: String,
    agent_id: String,
    round: u32,
    max_rounds: u32,
    questions: Vec<Question>,
    deadline: f64,
}

impl NandiEvent for ClarificationRequested {
    const NAME: &'static str = CLARIFICATION_REQUESTED;
}

/// Payload of `task://clarification_answered`; `outcome`: `answered`,
/// `timedOut`, `denied`, `failed`, or `cancelled`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClarificationAnswered {
    task_id: String,
    round: u32,
    outcome: &'static str,
}

impl NandiEvent for ClarificationAnswered {
    const NAME: &'static str = CLARIFICATION_ANSWERED;
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// What the service needs; tests replace the relay, answerer, and log.
pub struct ClarificationDeps {
    pub db: Db,
    pub bus: EventBus,
    pub store: RepoStore,
    pub instance_id: String,
    pub relay: Option<Arc<dyn RelayClient>>,
    pub answerer: Arc<dyn Answerer>,
    pub log: ClarificationLog,
    pub runtime: tokio::runtime::Handle,
    /// Provider side: the longest an `ask_requester` call waits
    pub provider_wait: Duration,
}

impl ClarificationDeps {
    pub fn from_state(state: &AppState) -> Result<Self, String> {
        let db = state.db().clone();
        let instance_id = network::local_instance_id(&db)?;
        Ok(Self {
            db,
            bus: state.bus().clone(),
            store: state.store().clone(),
            instance_id,
            relay: state.relay_client().cloned(),
            answerer: Arc::new(LlmAnswerer {
                secrets: state.secrets().clone(),
                store: state.store().clone(),
            }),
            log: ClarificationLog::default_location(),
            runtime: state.runtime().clone(),
            provider_wait: DEFAULT_PROVIDER_WAIT,
        })
    }
}

/// What the requester knows about a task it delegated
#[derive(Debug, Clone)]
struct TaskContext {
    /// The local requester agent; `None` for the user
    agent_id: Option<String>,
    capability: String,
    input: Value,
    /// The instance the task went to (empty if unknown)
    provider_instance: String,
}

/// One open round of questions (requester side)
#[derive(Debug, Clone)]
struct Round {
    task_id: String,
    round: u32,
    provider_instance: String,
    ctx: TaskContext,
    policy: ClarificationPolicy,
    questions: Vec<Question>,
    routes: HashMap<String, Route>,
    answers: HashMap<String, String>,
    /// Question ID → approval ID
    approvals: HashMap<String, String>,
    /// Epoch ms
    deadline: i64,
    /// Some question went to the human (the task went `InputRequired`)
    escalated: bool,
}

impl Round {
    fn question(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|q| q.id == id)
    }

    fn complete(&self) -> bool {
        self.questions
            .iter()
            .all(|q| self.answers.contains_key(&q.id))
    }

    fn agent_label(&self) -> String {
        self.ctx
            .agent_id
            .clone()
            .unwrap_or_else(|| USER_REQUESTER.to_string())
    }
}

/// How a provider's wait ended
#[derive(Debug)]
enum Reply {
    Answered(HashMap<String, String>),
    TimedOut,
    Cancelled(String),
    Undeliverable,
}

struct Waiting {
    round: u32,
    reply: oneshot::Sender<Reply>,
}

/// A question as the provider agent asks it
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskedQuestion {
    pub question: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default, alias = "requires_human")]
    pub requires_human: bool,
}

/// The delegated task a provider turn serves
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedTurn {
    pub task_id: String,
    pub requester_instance: String,
}

/// Track C: per-task rounds, pending escalations, the log.
pub struct ClarificationService {
    me: Weak<ClarificationService>,
    db: Db,
    bus: EventBus,
    store: RepoStore,
    instance_id: String,
    relay: Option<Arc<dyn RelayClient>>,
    answerer: Arc<dyn Answerer>,
    log: ClarificationLog,
    runtime: tokio::runtime::Handle,
    provider_wait: Duration,
    /// Requester side: the open round per task
    rounds: Mutex<HashMap<String, Round>>,
    /// Provider side: rounds asked per task
    asked: Mutex<HashMap<String, u32>>,
    /// Provider side: the turn waiting per task
    waiting: Mutex<HashMap<String, Waiting>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// `<instance>/<agent>` or a bare agent ID → the local agent, if it's ours
fn local_agent(id: &str, instance_id: &str) -> Option<String> {
    let agent = match id.split_once('/') {
        Some((instance, agent)) if instance == instance_id => agent,
        Some(_) => return None,
        None => id,
    };
    (!agent.is_empty() && agent != USER_REQUESTER).then(|| agent.to_string())
}

impl ClarificationService {
    pub fn new(deps: ClarificationDeps) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            db: deps.db,
            bus: deps.bus,
            store: deps.store,
            instance_id: deps.instance_id,
            relay: deps.relay,
            answerer: deps.answerer,
            log: deps.log,
            runtime: deps.runtime,
            provider_wait: deps.provider_wait,
            rounds: Mutex::default(),
            asked: Mutex::default(),
            waiting: Mutex::default(),
        })
    }

    /// Spawns `work` on the service's runtime with a strong reference
    fn spawn<F, Fut>(&self, work: F)
    where
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        if let Some(me) = self.me.upgrade() {
            self.runtime.spawn(async move { work(me).await });
        }
    }

    /// Listens for clarification traffic on the relay. Holds the service
    /// weakly: `AppState` owns it.
    pub async fn register(&self) {
        let Some(relay) = &self.relay else {
            return;
        };
        let me = self.me.clone();
        let runtime = self.runtime.clone();
        relay
            .on_peer(Box::new(move |delivery: PeerDelivery| {
                let Some(service) = me.upgrade() else {
                    return;
                };
                runtime.spawn(async move { service.handle_peer(delivery).await });
            }))
            .await;
    }

    /// Sends to another instance, or handles it here when it's ours
    async fn send(&self, target_instance: &str, body: PeerMessage) -> Result<(), String> {
        if target_instance == self.instance_id {
            let delivery = PeerDelivery {
                from_instance: self.instance_id.clone(),
                target_instance: target_instance.to_string(),
                body,
                undeliverable: false,
            };
            self.spawn(|me| async move { me.handle_peer(delivery).await });
            return Ok(());
        }
        let relay = crate::services::relay::unavailable::connected(self.relay.as_ref())?;
        relay.send_peer(target_instance.to_string(), body).await
    }

    async fn record_message(&self, task_id: &str, kind: &'static str, payload: Value) {
        let id = task_id.to_string();
        let written = self
            .db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO task_messages (task_id, kind, payload, at) VALUES (?1, ?2, ?3, ?4)",
                    params![id, kind, payload.to_string(), db::now_ms()],
                )
            })
            .await;
        if let Err(e) = written {
            log::warn!("Failed to record a {kind} for task {task_id}: {e}");
        }
    }

    async fn set_state(&self, task_id: &str, to: TaskState, reason: &str) {
        if let Err(e) = transition(&self.db, &self.bus, task_id, to, reason).await {
            log::warn!("Failed to move task {task_id} to {}: {e}", to.as_str());
        }
    }

    /// Every clarification message that arrives (either side). Boxed:
    /// answering a local task sends to ourselves, which recurses.
    pub fn handle_peer(&self, delivery: PeerDelivery) -> BoxFuture<'_, ()> {
        Box::pin(self.handle_peer_now(delivery))
    }

    async fn handle_peer_now(&self, delivery: PeerDelivery) {
        let PeerDelivery {
            from_instance,
            body,
            undeliverable,
            ..
        } = delivery;
        match body {
            PeerMessage::ClarificationRequest(request) if !undeliverable => {
                self.on_request(&from_instance, request).await
            }
            PeerMessage::ClarificationRequest(request) => {
                self.wake(&request.task_id, None, Reply::Undeliverable)
            }
            PeerMessage::ClarificationResponse(response) if !undeliverable => {
                let round = response.round;
                self.wake(
                    &response.task_id,
                    Some(round),
                    Reply::Answered(response.answers),
                )
            }
            PeerMessage::ClarificationTimeout { task_id } if !undeliverable => {
                self.wake(&task_id, None, Reply::TimedOut)
            }
            PeerMessage::CancelTask { task_id, reason } if !undeliverable => {
                let reason = reason.unwrap_or_else(|| "The task was cancelled".to_string());
                self.wake(&task_id, None, Reply::Cancelled(reason));
                // The provider gave up: nothing left to answer
                let open = lock(&self.rounds).remove(&task_id);
                if let Some(round) = open {
                    self.close(round, ApprovalStatus::Expired, "cancelled")
                        .await;
                }
            }
            PeerMessage::ClarificationResponse(response) => log::warn!(
                "The answers for task {} couldn't reach the provider",
                response.task_id
            ),
            _ => {}
        }
    }

    // --- Requester side -----------------------------------------------------

    async fn task_context(&self, task_id: &str) -> Result<Option<TaskContext>, String> {
        type Row = (String, String, String, String, String);
        fn read(row: &rusqlite::Row) -> rusqlite::Result<Row> {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        }
        let id = task_id.to_string();
        let found = self
            .db
            .call(move |conn| {
                let task = conn
                    .query_row(
                        "SELECT requester_agent, capability, input, provider_instance, provider_agent
                           FROM tasks WHERE id = ?1",
                        [&id],
                        read,
                    )
                    .optional()?;
                if task.is_some() {
                    return Ok(task);
                }
                conn.query_row(
                    "SELECT requester_agent, capability, input, '', target_agent
                       FROM delegations WHERE id = ?1 AND direction = 'outbound'",
                    [&id],
                    read,
                )
                .optional()
            })
            .await?;
        Ok(found.map(
            |(requester, capability, input, provider_instance, provider_agent)| TaskContext {
                agent_id: local_agent(&requester, &self.instance_id),
                capability,
                input: serde_json::from_str(&input).unwrap_or(Value::Null),
                provider_instance: if provider_instance.is_empty() {
                    instance_of(&provider_agent).to_string()
                } else {
                    provider_instance
                },
            },
        ))
    }

    async fn on_request(&self, from_instance: &str, request: ClarificationRequest) {
        let task_id = request.task_id.clone();
        let round_no = request.round;
        let count = request.questions.len().to_string();
        self.log
            .record(&task_id, round_no, "request", None, Some(&count))
            .await;
        let ctx = match self.task_context(&task_id).await {
            Ok(Some(ctx)) => ctx,
            Ok(None) => {
                self.log
                    .record(&task_id, round_no, "unknown_task", None, None)
                    .await;
                let _ = self
                    .send(
                        from_instance,
                        PeerMessage::CancelTask {
                            task_id,
                            reason: Some("The requester doesn't know this task".to_string()),
                        },
                    )
                    .await;
                return;
            }
            Err(e) => {
                log::error!("Failed to look up task {task_id}: {e}");
                return;
            }
        };
        if !ctx.provider_instance.is_empty() && ctx.provider_instance != from_instance {
            log::warn!(
                "Ignoring clarification for task {task_id} from {from_instance}: it went to {}",
                ctx.provider_instance
            );
            self.log
                .record(
                    &task_id,
                    round_no,
                    "wrong_sender",
                    None,
                    Some(from_instance),
                )
                .await;
            return;
        }
        let policy = ctx
            .agent_id
            .as_deref()
            .and_then(|agent| read_policy(&self.store, agent).ok())
            .unwrap_or_default();
        self.record_message(
            &task_id,
            "clarification_request",
            serde_json::to_value(&request).unwrap_or(Value::Null),
        )
        .await;

        if !policy.allow {
            self.log
                .record(&task_id, round_no, "not_allowed", None, None)
                .await;
            let answers = request
                .questions
                .iter()
                .map(|q| (q.id.clone(), NOT_ALLOWED_ANSWER.to_string()))
                .collect();
            let response = ClarificationResponse {
                task_id: task_id.clone(),
                round: round_no,
                answers,
            };
            if let Err(e) = self
                .send(from_instance, PeerMessage::ClarificationResponse(response))
                .await
            {
                log::warn!("Failed to answer the clarification for task {task_id}: {e}");
            }
            self.set_state(&task_id, TaskState::Failed, NOT_ALLOWED_ANSWER)
                .await;
            self.emit_answered(&task_id, round_no, "failed");
            return;
        }
        if round_no == 0 || round_no > policy.max_rounds {
            let reason = format!(
                "Too many clarification rounds (round {round_no}, at most {})",
                policy.max_rounds
            );
            self.log
                .record(&task_id, round_no, "max_rounds", None, Some(&reason))
                .await;
            self.set_state(&task_id, TaskState::Failed, &reason).await;
            let _ = self
                .send(
                    from_instance,
                    PeerMessage::CancelTask {
                        task_id: task_id.clone(),
                        reason: Some(reason),
                    },
                )
                .await;
            self.emit_answered(&task_id, round_no, "failed");
            return;
        }

        let mut routes = HashMap::new();
        for question in &request.questions {
            let (route, reason) = match ctx.agent_id {
                // The user asked: there's no agent to answer
                None => (Route::Escalate, "no_agent"),
                Some(_) => classify_with_reason(&policy, question),
            };
            routes.insert(question.id.clone(), route);
            self.log
                .record(
                    &task_id,
                    round_no,
                    route.as_str(),
                    Some(&question.id),
                    Some(reason),
                )
                .await;
        }
        let deadline = db::now_ms() + i64::from(policy.timeout_seconds) * 1000;
        let round = Round {
            task_id: task_id.clone(),
            round: round_no,
            provider_instance: from_instance.to_string(),
            ctx,
            policy: policy.clone(),
            questions: request.questions.clone(),
            routes: routes.clone(),
            answers: HashMap::new(),
            approvals: HashMap::new(),
            deadline,
            escalated: false,
        };
        let replaced = lock(&self.rounds).insert(task_id.clone(), round);
        if let Some(old) = replaced {
            escalation::close_approvals(
                &self.db,
                old.approvals.into_values().collect(),
                ApprovalStatus::Expired,
            )
            .await;
        }

        let escalated: Vec<String> = request
            .questions
            .iter()
            .filter(|q| routes.get(&q.id) == Some(&Route::Escalate))
            .map(|q| q.id.clone())
            .collect();
        if !escalated.is_empty() {
            self.open_escalations(&task_id, round_no, escalated).await;
        }
        for question in request.questions {
            if routes.get(&question.id) == Some(&Route::Auto) {
                self.spawn_auto_answer(&task_id, round_no, question);
            }
        }
        let timeout = Duration::from_secs(u64::from(policy.timeout_seconds));
        let (timer_task, timer_round) = (task_id.clone(), round_no);
        self.spawn(move |me| async move {
            tokio::time::sleep(timeout).await;
            me.expire(&timer_task, timer_round).await;
        });
        self.try_complete(&task_id).await;
    }

    fn spawn_auto_answer(&self, task_id: &str, round: u32, question: Question) {
        let task_id = task_id.to_string();
        self.spawn(move |me| async move {
            let ctx = {
                let rounds = lock(&me.rounds);
                match rounds.get(&task_id) {
                    Some(r) if r.round == round => r.ctx.clone(),
                    _ => return,
                }
            };
            let Some(agent_id) = ctx.agent_id.clone() else {
                return;
            };
            let result = me
                .answerer
                .answer(AnswerCall {
                    agent_id: &agent_id,
                    capability: &ctx.capability,
                    task_input: &ctx.input,
                    question: &question,
                })
                .await;
            me.auto_answered(&task_id, round, &question.id, result)
                .await;
        });
    }

    async fn auto_answered(
        &self,
        task_id: &str,
        round: u32,
        question_id: &str,
        result: Result<Option<String>, String>,
    ) {
        let failure = {
            let mut rounds = lock(&self.rounds);
            let Some(open) = rounds.get_mut(task_id) else {
                return;
            };
            // Escalated meanwhile, or another round: the answer is stale
            if open.round != round
                || open.routes.get(question_id) != Some(&Route::Auto)
                || open.answers.contains_key(question_id)
            {
                return;
            }
            match result {
                Ok(Some(answer)) if !answer.trim().is_empty() => {
                    open.answers
                        .insert(question_id.to_string(), answer.trim().to_string());
                    None
                }
                Ok(_) => Some("the agent couldn't answer".to_string()),
                Err(e) => Some(e),
            }
        };
        match failure {
            None => {
                self.log
                    .record(task_id, round, "auto_answered", Some(question_id), None)
                    .await
            }
            Some(reason) => {
                self.log
                    .record(
                        task_id,
                        round,
                        "auto_failed",
                        Some(question_id),
                        Some(&reason),
                    )
                    .await;
                self.open_escalations(task_id, round, vec![question_id.to_string()])
                    .await;
            }
        }
        self.try_complete(task_id).await;
    }

    /// Sends questions of the open round to the human: an approval each,
    /// the task `InputRequired`, `task://clarification_requested`
    async fn open_escalations(&self, task_id: &str, round: u32, question_ids: Vec<String>) {
        let (opened, agent_id, deadline, max_rounds) = {
            let mut rounds = lock(&self.rounds);
            let Some(open) = rounds.get_mut(task_id).filter(|r| r.round == round) else {
                return;
            };
            let mut opened = Vec::new();
            for id in question_ids {
                let Some(question) = open.question(&id).cloned() else {
                    continue;
                };
                open.routes.insert(id.clone(), Route::Escalate);
                // A forced escalation drops an auto answer not yet sent
                open.answers.remove(&id);
                if !open.approvals.contains_key(&id) {
                    let approval_id = uuid::Uuid::new_v4().to_string();
                    open.approvals.insert(id.clone(), approval_id.clone());
                    opened.push((approval_id, question));
                }
            }
            open.escalated = true;
            (
                opened,
                open.agent_label(),
                open.deadline,
                open.policy.max_rounds,
            )
        };
        if opened.is_empty() {
            return;
        }
        for (approval_id, question) in &opened {
            if let Err(e) = escalation::insert_approval(
                &self.db,
                approval_id,
                &agent_id,
                task_id,
                round,
                question,
            )
            .await
            {
                log::error!("Failed to ask about task {task_id}: {e}");
            }
            self.log
                .record(task_id, round, "escalated", Some(&question.id), None)
                .await;
        }
        self.set_state(
            task_id,
            TaskState::InputRequired,
            "Waiting for a person to answer the provider's questions",
        )
        .await;
        events::emit(
            &self.bus,
            &ClarificationRequested {
                task_id: task_id.to_string(),
                agent_id,
                round,
                max_rounds,
                questions: opened.into_iter().map(|(_, q)| q).collect(),
                deadline: db::ts(deadline),
            },
        );
    }

    /// Sends the round's answers once every question has one
    async fn try_complete(&self, task_id: &str) {
        let done = {
            let mut rounds = lock(&self.rounds);
            match rounds.get(task_id) {
                Some(open) if open.complete() => rounds.remove(task_id),
                _ => None,
            }
        };
        let Some(done) = done else {
            return;
        };
        let response = ClarificationResponse {
            task_id: task_id.to_string(),
            round: done.round,
            answers: done.answers.clone(),
        };
        self.record_message(
            task_id,
            "clarification_response",
            serde_json::to_value(&response).unwrap_or(Value::Null),
        )
        .await;
        // Recorded first, so the provider never resumes ahead of our state
        if done.escalated {
            self.set_state(task_id, TaskState::Working, "The questions were answered")
                .await;
        }
        escalation::close_approvals(
            &self.db,
            done.approvals.into_values().collect(),
            ApprovalStatus::Approved,
        )
        .await;
        self.log
            .record(task_id, done.round, "responded", None, None)
            .await;
        if let Err(e) = self
            .send(
                &done.provider_instance,
                PeerMessage::ClarificationResponse(response),
            )
            .await
        {
            log::warn!("Failed to send the answers for task {task_id}: {e}");
        }
        self.emit_answered(task_id, done.round, "answered");
    }

    fn emit_answered(&self, task_id: &str, round: u32, outcome: &'static str) {
        events::emit(
            &self.bus,
            &ClarificationAnswered {
                task_id: task_id.to_string(),
                round,
                outcome,
            },
        );
    }

    /// Resolves an ended round's approvals and tells the UI
    async fn close(&self, round: Round, status: ApprovalStatus, outcome: &'static str) {
        escalation::close_approvals(&self.db, round.approvals.into_values().collect(), status)
            .await;
        self.log
            .record(&round.task_id, round.round, outcome, None, None)
            .await;
        self.emit_answered(&round.task_id, round.round, outcome);
    }

    /// The round's deadline passed: the task times out
    async fn expire(&self, task_id: &str, round: u32) {
        let expired = {
            let mut rounds = lock(&self.rounds);
            match rounds.get(task_id) {
                Some(open) if open.round == round => rounds.remove(task_id),
                _ => None,
            }
        };
        let Some(expired) = expired else {
            return;
        };
        let reason = format!(
            "Nobody answered the provider's questions within {}s",
            expired.policy.timeout_seconds
        );
        self.set_state(task_id, TaskState::TimedOut, &reason).await;
        self.record_message(task_id, "clarification_timeout", json!({ "round": round }))
            .await;
        let provider_instance = expired.provider_instance.clone();
        self.close(expired, ApprovalStatus::Expired, "timedOut")
            .await;
        if let Err(e) = self
            .send(
                &provider_instance,
                PeerMessage::ClarificationTimeout {
                    task_id: task_id.to_string(),
                },
            )
            .await
        {
            log::warn!("Failed to tell the provider task {task_id} timed out: {e}");
        }
    }

    /// The human declined: the task is cancelled on both sides
    pub async fn deny(&self, task_id: &str) -> Result<(), String> {
        let denied = lock(&self.rounds)
            .remove(task_id)
            .ok_or("There is no open clarification for this task")?;
        let reason = "The requester declined to answer the provider's questions".to_string();
        self.set_state(task_id, TaskState::Cancelled, &reason).await;
        let provider_instance = denied.provider_instance.clone();
        self.close(denied, ApprovalStatus::Rejected, "denied").await;
        if let Err(e) = self
            .send(
                &provider_instance,
                PeerMessage::CancelTask {
                    task_id: task_id.to_string(),
                    reason: Some(reason),
                },
            )
            .await
        {
            log::warn!("Failed to cancel task {task_id} at the provider: {e}");
        }
        Ok(())
    }

    /// The human answers (see the module docs for the special answers)
    pub async fn respond(&self, response: ClarificationResponse) -> Result<(), String> {
        enum Action {
            Answer(String, String),
            Auto { question: Question, always: bool },
        }
        let task_id = response.task_id.clone();
        let (round_no, actions, agent_id, deny) = {
            let rounds = lock(&self.rounds);
            let open = rounds
                .get(&task_id)
                .ok_or("There is no open clarification for this task")?;
            if response.round != 0 && response.round != open.round {
                return Err("This round of questions is already over".to_string());
            }
            if response.answers.is_empty() {
                return Err("Answer at least one question".to_string());
            }
            let mut actions = Vec::new();
            let mut deny = false;
            for (id, answer) in &response.answers {
                let question = open
                    .question(id)
                    .ok_or_else(|| format!("Unknown question '{id}'"))?;
                match answer.trim() {
                    DENY_ANSWER => deny = true,
                    ALLOW_ONCE_ANSWER | ALLOW_ALWAYS_ANSWER if open.ctx.agent_id.is_none() => {
                        return Err(
                            "No agent can answer this on its own: type an answer".to_string()
                        )
                    }
                    ALLOW_ONCE_ANSWER | ALLOW_ALWAYS_ANSWER => actions.push(Action::Auto {
                        question: question.clone(),
                        always: answer.trim() == ALLOW_ALWAYS_ANSWER,
                    }),
                    "" => return Err("Type an answer first".to_string()),
                    text => actions.push(Action::Answer(id.clone(), text.to_string())),
                }
            }
            (open.round, actions, open.ctx.agent_id.clone(), deny)
        };
        if deny {
            return self.deny(&task_id).await;
        }

        let mut resolved = Vec::new();
        let mut auto = Vec::new();
        let mut learn = Vec::new();
        {
            let mut rounds = lock(&self.rounds);
            let open = rounds
                .get_mut(&task_id)
                .filter(|r| r.round == round_no)
                .ok_or("This round of questions is already over")?;
            for action in actions {
                match action {
                    Action::Answer(id, text) => {
                        open.routes.insert(id.clone(), Route::Escalate);
                        open.answers.insert(id.clone(), text);
                        resolved.extend(open.approvals.remove(&id));
                    }
                    Action::Auto { question, always } => {
                        open.routes.insert(question.id.clone(), Route::Auto);
                        open.answers.remove(&question.id);
                        resolved.extend(open.approvals.remove(&question.id));
                        if always {
                            learn.push(question.topic.clone());
                        }
                        auto.push(question);
                    }
                }
            }
        }
        escalation::close_approvals(&self.db, resolved, ApprovalStatus::Approved).await;
        if let (Some(agent_id), false) = (&agent_id, learn.is_empty()) {
            if let Err(e) = self.learn_topics(agent_id, learn) {
                log::warn!("Failed to update the clarification policy of '{agent_id}': {e}");
            }
        }
        for id in response.answers.keys() {
            self.log
                .record(&task_id, round_no, "human_answered", Some(id), None)
                .await;
        }
        for question in auto {
            self.spawn_auto_answer(&task_id, round_no, question);
        }
        self.try_complete(&task_id).await;
        Ok(())
    }

    /// "Allow always": the agent answers these topics from now on
    fn learn_topics(&self, agent_id: &str, topics: Vec<String>) -> Result<(), String> {
        let mut policy = read_policy(&self.store, agent_id)?;
        policy
            .auto_topics
            .extend(topics.into_iter().filter(|t| !t.trim().is_empty()));
        write_policy(&self.store, agent_id, policy).map(|_| ())
    }

    /// Sends a question to the human even if the agent would answer it
    pub async fn escalate(&self, task_id: &str, question_id: &str) -> Result<(), String> {
        let round = {
            let rounds = lock(&self.rounds);
            let open = rounds
                .get(task_id)
                .ok_or("There is no open clarification for this task")?;
            if open.question(question_id).is_none() {
                return Err(format!("Unknown question '{question_id}'"));
            }
            if open.approvals.contains_key(question_id) {
                return Ok(());
            }
            open.round
        };
        self.log
            .record(task_id, round, "forced_escalation", Some(question_id), None)
            .await;
        self.open_escalations(task_id, round, vec![question_id.to_string()])
            .await;
        Ok(())
    }

    /// An escalation approval was decided elsewhere (e.g. `approve_action`):
    /// approved lets the agent answer it, rejected cancels the task
    pub async fn resolve_approval(&self, approval_id: &str, approved: bool) -> Result<(), String> {
        let (task_id, round, question_id) = lock(&self.rounds)
            .values()
            .find_map(|r| {
                r.approvals
                    .iter()
                    .find(|(_, id)| id.as_str() == approval_id)
                    .map(|(q, _)| (r.task_id.clone(), r.round, q.clone()))
            })
            .ok_or("This question is no longer waiting for an answer")?;
        let answer = if approved {
            ALLOW_ONCE_ANSWER
        } else {
            DENY_ANSWER
        };
        self.respond(ClarificationResponse {
            task_id,
            round,
            answers: HashMap::from([(question_id, answer.to_string())]),
        })
        .await
    }

    /// Questions waiting for the human, soonest deadline first
    pub fn pending(&self) -> Vec<PendingClarification> {
        let rounds = lock(&self.rounds);
        let mut pending: Vec<PendingClarification> = rounds
            .values()
            .flat_map(|r| {
                r.questions
                    .iter()
                    .filter(|q| {
                        r.routes.get(&q.id) == Some(&Route::Escalate)
                            && !r.answers.contains_key(&q.id)
                    })
                    .map(|q| PendingClarification {
                        task_id: r.task_id.clone(),
                        agent_id: r.agent_label(),
                        question: q.clone(),
                        deadline: Some(db::ts(r.deadline)),
                        approval_id: r.approvals.get(&q.id).cloned(),
                    })
            })
            .collect();
        pending.sort_by(|a, b| {
            a.deadline
                .partial_cmp(&b.deadline)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.question.id.cmp(&b.question.id))
        });
        pending
    }

    // --- Provider side ------------------------------------------------------

    /// The delegated task a provider run serves, if it is a delegated turn
    pub async fn delegated_turn(
        &self,
        run_id: &str,
        agent_id: &str,
    ) -> Result<Option<DelegatedTurn>, String> {
        let id = run_id.to_string();
        let provider_card = AgentCard::card_id(&self.instance_id, agent_id);
        let agent = agent_id.to_string();
        self.db
            .call(move |conn| {
                let Some(run) = db::get_run(conn, &id)? else {
                    return Ok(None);
                };
                let Some(requester) = run
                    .source_id
                    .filter(|_| run.source_type == SourceType::Agent)
                else {
                    return Ok(None);
                };
                let task_id: Option<String> = conn
                    .query_row(
                        "SELECT id FROM delegations
                          WHERE target_agent = ?1 AND requester_agent = ?2 AND status = 'running'
                          ORDER BY started_at DESC LIMIT 1",
                        params![provider_card, requester],
                        |row| row.get(0),
                    )
                    .optional()?;
                let task_id = match task_id {
                    Some(id) => Some(id),
                    None => conn
                        .query_row(
                            "SELECT id FROM tasks
                              WHERE provider_agent IN (?1, ?2) AND requester_agent = ?3
                                AND state IN ('delivered', 'acknowledged', 'working', 'inputRequired')
                              ORDER BY created_at DESC LIMIT 1",
                            params![provider_card, agent, requester],
                            |row| row.get(0),
                        )
                        .optional()?,
                };
                Ok(task_id.map(|task_id| DelegatedTurn {
                    task_id,
                    requester_instance: instance_of(&requester).to_string(),
                }))
            })
            .await
    }

    /// Whether a turn of ours is blocked in `ask_requester` on `task_id`
    /// (the provider side's delegation timeout doesn't run meanwhile)
    pub fn is_waiting(&self, task_id: &str) -> bool {
        lock(&self.waiting).contains_key(task_id)
    }

    /// Wakes the turn waiting on `task_id` (for `round`, if given)
    fn wake(&self, task_id: &str, round: Option<u32>, reply: Reply) {
        let waiting = {
            let mut waiting = lock(&self.waiting);
            match waiting.get(task_id) {
                Some(w) if round.is_none_or(|r| r == w.round) => waiting.remove(task_id),
                _ => None,
            }
        };
        if let Some(waiting) = waiting {
            let _ = waiting.reply.send(reply);
        }
    }

    /// Provider side: asks the requester and waits for the answers.
    /// Returns the round and `(question, answer)` pairs in the order asked.
    pub async fn ask(
        &self,
        turn: &DelegatedTurn,
        asked: Vec<AskedQuestion>,
    ) -> Result<(u32, Vec<(Question, String)>), String> {
        let task_id = &turn.task_id;
        if asked.is_empty() {
            return Err("Ask at least one question".to_string());
        }
        if asked.len() > MAX_QUESTIONS {
            return Err(format!("Ask at most {MAX_QUESTIONS} questions at a time"));
        }
        if asked.iter().any(|q| q.question.trim().is_empty()) {
            return Err("Every question needs text".to_string());
        }
        if asked.iter().any(|q| q.question.len() > MAX_QUESTION_CHARS) {
            return Err(format!(
                "Keep each question under {MAX_QUESTION_CHARS} characters"
            ));
        }
        let max_rounds = ClarificationPolicy::default().max_rounds;
        let round = {
            let mut rounds = lock(&self.asked);
            let count = rounds.entry(task_id.clone()).or_default();
            if *count >= max_rounds {
                return Err(format!(
                    "You already asked {max_rounds} rounds of questions for this task: finish \
                     it with what you know"
                ));
            }
            *count += 1;
            *count
        };
        let questions: Vec<Question> = asked
            .into_iter()
            .enumerate()
            .map(|(i, q)| Question {
                id: format!("r{round}-q{}", i + 1),
                question: q.question.trim().to_string(),
                topic: q.topic.trim().to_string(),
                requires_human: q.requires_human,
            })
            .collect();
        let (tx, rx) = oneshot::channel();
        lock(&self.waiting).insert(task_id.clone(), Waiting { round, reply: tx });
        let request = ClarificationRequest {
            task_id: task_id.clone(),
            round,
            questions: questions.clone(),
        };
        let count = questions.len().to_string();
        self.log
            .record(task_id, round, "asked", None, Some(&count))
            .await;
        if let Err(e) = self
            .send(
                &turn.requester_instance,
                PeerMessage::ClarificationRequest(request),
            )
            .await
        {
            lock(&self.waiting).remove(task_id);
            return Err(format!("Couldn't reach the requester: {e}"));
        }
        let reply = tokio::time::timeout(self.provider_wait, rx).await;
        lock(&self.waiting).remove(task_id);
        let answers = match reply {
            Ok(Ok(Reply::Answered(answers))) => answers,
            Ok(Ok(Reply::TimedOut)) | Err(_) => {
                return Err("The requester didn't answer in time".to_string())
            }
            Ok(Ok(Reply::Cancelled(reason))) => {
                return Err(format!("The requester cancelled the task: {reason}"))
            }
            Ok(Ok(Reply::Undeliverable)) => {
                return Err("The requester is offline right now".to_string())
            }
            Ok(Err(_)) => return Err("Clarification stopped".to_string()),
        };
        self.log
            .record(task_id, round, "answered", None, None)
            .await;
        Ok((
            round,
            questions
                .into_iter()
                .map(|q| {
                    let answer = answers
                        .get(&q.id)
                        .cloned()
                        .unwrap_or_else(|| "(no answer)".to_string());
                    (q, answer)
                })
                .collect(),
        ))
    }
}

fn service(state: &AppState) -> Result<&Arc<ClarificationService>, String> {
    state
        .clarifications()
        .ok_or_else(|| "Clarification is unavailable".to_string())
}

/// The human (or a UI on their behalf) answers a round directly
pub async fn respond_to_clarification(
    state: &AppState,
    response: ClarificationResponse,
) -> Result<(), String> {
    service(state)?.respond(response).await
}

/// Questions waiting for the human
pub async fn get_pending_clarifications(
    state: &AppState,
) -> Result<Vec<PendingClarification>, String> {
    Ok(service(state)?.pending())
}

/// Builds the service and listens on the relay (in a spawned task).
/// Must not block.
pub fn init(state: &AppState) -> Option<Arc<ClarificationService>> {
    let deps = match ClarificationDeps::from_state(state) {
        Ok(deps) => deps,
        Err(e) => {
            log::error!("Clarification is unavailable: {e}");
            return None;
        }
    };
    let service = ClarificationService::new(deps);
    state.orchestrators().set_clarifier(&service);
    let registering = service.clone();
    state
        .runtime()
        .spawn(async move { registering.register().await });
    Some(service)
}

#[cfg(test)]
mod tests;
