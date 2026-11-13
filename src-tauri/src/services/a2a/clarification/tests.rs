//! Clarification end to end: policy routing, escalation through approvals,
//! denial, timeouts, round limits, and two instances over a mock relay.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::*;
use crate::model::{ApprovalDecision, RunStatus, RunSummary};
use crate::services::a2a::escalation::ESCALATION_TOOL;
use crate::services::a2a::task::TASK_STATE_CHANGED;
use crate::services::a2a::tool;
use crate::services::a2a::{Task, TaskResult};
use crate::services::domain::AgentId;
use crate::services::events::tests::RecordingSink;
use crate::services::relay::client_api::{Handler, StoringHandler};
use crate::services::relay::protocol::{TaskAssigned, TaskCompleted, TaskFailed};
use crate::services::relay::RelayStatus;

// ---------------------------------------------------------------------------
// A mock relay that only carries peer messages
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Hub {
    relays: Mutex<HashMap<String, Weak<PeerRelay>>>,
}

struct PeerRelay {
    instance: String,
    hub: Arc<Hub>,
    handlers: Mutex<Vec<Handler<PeerDelivery>>>,
    sent: Mutex<Vec<PeerMessage>>,
}

impl PeerRelay {
    fn join(hub: &Arc<Hub>, instance: &str) -> Arc<Self> {
        let relay = Arc::new(Self {
            instance: instance.to_string(),
            hub: hub.clone(),
            handlers: Mutex::default(),
            sent: Mutex::default(),
        });
        hub.relays
            .lock()
            .unwrap()
            .insert(instance.to_string(), Arc::downgrade(&relay));
        relay
    }

    fn deliver(&self, delivery: PeerDelivery) {
        for handler in self.handlers.lock().unwrap().iter() {
            handler(delivery.clone());
        }
    }

    fn sent(&self) -> Vec<PeerMessage> {
        self.sent.lock().unwrap().clone()
    }
}

impl RelayClient for PeerRelay {
    fn connect<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn register(&self, _: Vec<AgentCard>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn heartbeat(&self, _: HashMap<String, u32>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn query_capability<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn delegate_task<'a>(
        &'a self,
        _: &'a Task,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<crate::services::relay::client_api::Submitted, String>> {
        Box::pin(async { Err("not in these tests".to_string()) })
    }

    fn send_result(&self, _: TaskResult) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn on_task_assigned(&self, _: StoringHandler<TaskAssigned>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn on_task_completed(&self, _: Handler<TaskCompleted>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn on_task_failed(&self, _: Handler<TaskFailed>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn status(&self) -> RelayStatus {
        RelayStatus {
            connected: true,
            instance_id: self.instance.clone(),
            ..RelayStatus::default()
        }
    }

    fn send_peer(
        &self,
        target_instance: String,
        body: PeerMessage,
    ) -> BoxFuture<'_, Result<(), String>> {
        self.sent.lock().unwrap().push(body.clone());
        let target = self
            .hub
            .relays
            .lock()
            .unwrap()
            .get(&target_instance)
            .and_then(Weak::upgrade);
        // Offline: the relay would keep it for them
        if let Some(target) = target {
            target.deliver(PeerDelivery {
                from_instance: self.instance.clone(),
                target_instance,
                body,
            });
        }
        Box::pin(async { Ok(()) })
    }

    fn on_peer(&self, handler: Handler<PeerDelivery>) -> BoxFuture<'_, ()> {
        self.handlers.lock().unwrap().push(handler);
        Box::pin(async {})
    }
}

// ---------------------------------------------------------------------------
// Answerers
// ---------------------------------------------------------------------------

type AnswerFn = dyn Fn(&Question) -> Result<Option<String>, String> + Send + Sync;

/// Answers synchronously, and remembers what it was asked
struct Scripted {
    answer: Box<AnswerFn>,
    asked: Mutex<Vec<(String, Value)>>,
}

impl Scripted {
    fn new(
        answer: impl Fn(&Question) -> Result<Option<String>, String> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            answer: Box::new(answer),
            asked: Mutex::default(),
        })
    }

    fn echo() -> Arc<Self> {
        Self::new(|q| Ok(Some(format!("agent: {}", q.topic))))
    }
}

impl Answerer for Scripted {
    fn answer<'a>(&'a self, call: AnswerCall<'a>) -> BoxFuture<'a, Result<Option<String>, String>> {
        self.asked
            .lock()
            .unwrap()
            .push((call.question.id.clone(), call.task_input.clone()));
        let answer = (self.answer)(call.question);
        Box::pin(async move { answer })
    }
}

/// Never answers
struct Silent;

impl Answerer for Silent {
    fn answer<'a>(&'a self, _: AnswerCall<'a>) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(futures_util::future::pending())
    }
}

// ---------------------------------------------------------------------------
// Two instances
// ---------------------------------------------------------------------------

struct Side {
    state: AppState,
    sink: Arc<RecordingSink>,
    service: Arc<ClarificationService>,
    relay: Arc<PeerRelay>,
    instance: String,
}

async fn side(hub: &Arc<Hub>, answerer: Arc<dyn Answerer>, log: ClarificationLog) -> Side {
    let sink = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![sink.clone()]);
    let instance = network::local_instance_id(state.db()).unwrap();
    let relay = PeerRelay::join(hub, &instance);
    let service = ClarificationService::new(ClarificationDeps {
        relay: Some(relay.clone()),
        answerer,
        log,
        provider_wait: Duration::from_secs(10),
        ..ClarificationDeps::from_state(&state).unwrap()
    });
    service.register().await;
    Side {
        state,
        sink,
        service,
        relay,
        instance,
    }
}

/// What the provider's `ask` returns
type Asked = Result<(u32, Vec<(Question, String)>), String>;

struct World {
    requester: Side,
    provider: Side,
    task_id: String,
    _log_dir: tempfile::TempDir,
}

impl World {
    fn turn(&self) -> DelegatedTurn {
        DelegatedTurn {
            task_id: self.task_id.clone(),
            requester_instance: self.requester.instance.clone(),
        }
    }

    /// The provider asks in the background
    fn ask(&self, questions: Vec<AskedQuestion>) -> tokio::task::JoinHandle<Asked> {
        let service = self.provider.service.clone();
        let turn = self.turn();
        tokio::spawn(async move { service.ask(&turn, questions).await })
    }

    async fn state(&self) -> String {
        task_state(&self.requester.state, &self.task_id).await
    }

    fn log_lines(&self) -> Vec<Value> {
        let path = self._log_dir.path().join("requester.log");
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn save_agent(state: &AppState, id: &str, frontmatter: &str) {
    let agent = Agent::from_skill_md(
        AgentId::parse(id).unwrap(),
        &format!("---\nname: {id}\n{frontmatter}---\nHelp.\n"),
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, id).unwrap();
}

/// A `tasks` row on the requester, as Track B will write it
async fn insert_task(state: &AppState, task_id: &str, requester: &str, provider_instance: &str) {
    // Requester-side: our own instance asked (Track B's resume fails any
    // in-flight row it takes for a provider row without a stored request)
    let me = network::local_instance_id(state.db()).unwrap();
    let (id, requester, provider_instance) = (
        task_id.to_string(),
        requester.to_string(),
        provider_instance.to_string(),
    );
    state
        .db()
        .call(move |conn| {
            let now = db::now_ms();
            conn.execute(
                "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                                    provider_agent, capability, input, state, created_at, updated_at)
                 VALUES (?1, ?7, ?2, ?3, ?4, 'summarize', ?5, 'working', ?6, ?6)",
                params![
                    id,
                    requester,
                    provider_instance,
                    format!("{provider_instance}/papers"),
                    json!({"text": "Attention is all you need"}).to_string(),
                    now,
                    me
                ],
            )
        })
        .await
        .unwrap();
}

async fn task_state(state: &AppState, task_id: &str) -> String {
    let id = task_id.to_string();
    state
        .db()
        .call(move |conn| {
            conn.query_row("SELECT state FROM tasks WHERE id = ?1", [id], |r| r.get(0))
        })
        .await
        .unwrap()
}

async fn task_events(state: &AppState, task_id: &str) -> Vec<String> {
    let id = task_id.to_string();
    state
        .db()
        .call(move |conn| {
            // Track B's startup resume may race the fixture's insert and
            // record a no-op `resumed after restart` row; it isn't a transition
            let mut stmt = conn.prepare(
                "SELECT to_state FROM task_events
                 WHERE task_id = ?1 AND COALESCE(reason, '') != ?2 ORDER BY id",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![id, crate::services::a2a::task_state::RESUMED_REASON],
                |r| r.get(0),
            )?;
            rows.collect()
        })
        .await
        .unwrap()
}

async fn approval_status(state: &AppState, approval_id: &str) -> ApprovalStatus {
    let id = approval_id.to_string();
    state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await
        .unwrap()
        .unwrap()
        .status
}

async fn world(policy_frontmatter: &str, answerer: Arc<dyn Answerer>) -> World {
    let hub = Arc::new(Hub::default());
    let log_dir = tempfile::tempdir().unwrap();
    let requester = side(
        &hub,
        answerer,
        ClarificationLog::at(log_dir.path().join("requester.log")),
    )
    .await;
    let provider = side(&hub, Scripted::echo(), ClarificationLog::default()).await;
    save_agent(&requester.state, "writer", policy_frontmatter);
    save_agent(
        &provider.state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let task_id = uuid::Uuid::new_v4().to_string();
    insert_task(&requester.state, &task_id, "writer", &provider.instance).await;
    World {
        requester,
        provider,
        task_id,
        _log_dir: log_dir,
    }
}

fn question(text: &str, topic: &str) -> AskedQuestion {
    AskedQuestion {
        question: text.into(),
        topic: topic.into(),
        requires_human: false,
    }
}

async fn wait_until<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn wait_for_pending(side: &Side, count: usize) -> Vec<PendingClarification> {
    wait_until("pending clarifications", || async move {
        side.service.pending().len() == count
    })
    .await;
    side.service.pending()
}

const AUTO_FORMAT: &str = "clarification:\n  auto_topics: [Format]\n  escalate_topics: [budget]\n";

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

fn q(topic: &str, requires_human: bool) -> Question {
    Question {
        id: "q".into(),
        question: "?".into(),
        topic: topic.into(),
        requires_human,
    }
}

#[test]
fn questions_route_by_policy_with_escalation_as_the_default() {
    let policy = ClarificationPolicy {
        auto_topics: vec!["Format".into(), "scope".into()],
        escalate_topics: vec!["budget".into(), "invoices".into()],
        ..ClarificationPolicy::default()
    };
    // Case-insensitive substring, either way round
    assert_eq!(classify(&policy, &q("format", false)), Route::Auto);
    assert_eq!(classify(&policy, &q("SCOPE of work", false)), Route::Auto);
    assert_eq!(classify(&policy, &q("Budget", false)), Route::Escalate);
    // Escalate topics win over auto topics
    assert_eq!(
        classify(&policy, &q("format of invoices", false)),
        Route::Escalate
    );
    // The provider asks for a person
    assert_eq!(classify(&policy, &q("format", true)), Route::Escalate);
    // Unknown and empty topics go to the human
    assert_eq!(
        classify_with_reason(&policy, &q("deadline", false)),
        (Route::Escalate, "default")
    );
    assert_eq!(classify(&policy, &q("  ", false)), Route::Escalate);
    assert_eq!(
        classify(&ClarificationPolicy::default(), &q("format", false)),
        Route::Escalate
    );
}

#[tokio::test]
async fn the_policy_lives_in_skill_md() {
    let state = AppState::for_tests(vec![]);
    save_agent(&state, "writer", "");
    assert_eq!(
        read_policy(state.store(), "writer").unwrap(),
        ClarificationPolicy::default()
    );

    let saved = write_policy(
        state.store(),
        "writer",
        ClarificationPolicy {
            max_rounds: 3,
            auto_topics: vec![" format ".into(), "FORMAT".into(), String::new()],
            escalate_topics: vec!["budget".into()],
            timeout_seconds: 60,
            ..ClarificationPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(
        saved.auto_topics,
        vec!["format"],
        "trimmed and de-duplicated"
    );
    assert_eq!(read_policy(state.store(), "writer").unwrap(), saved);
    let md = state.store().agent("writer").unwrap().to_skill_md();
    assert!(md.contains("clarification:"), "{md}");
    assert!(md.contains("max_rounds: 3"), "{md}");

    // Out of bounds
    let invalid = ClarificationPolicy {
        timeout_seconds: 0,
        ..ClarificationPolicy::default()
    };
    assert!(write_policy(state.store(), "writer", invalid).is_err());

    // The default removes the key
    write_policy(state.store(), "writer", ClarificationPolicy::default()).unwrap();
    let md = state.store().agent("writer").unwrap().to_skill_md();
    assert!(!md.contains("clarification:"), "{md}");
}

// ---------------------------------------------------------------------------
// Requester side
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_questions_are_answered_by_the_agent_at_once() {
    let answerer = Scripted::echo();
    let world = world(AUTO_FORMAT, answerer.clone()).await;

    let (round, answers) = world
        .ask(vec![question("Bullets or prose?", "format")])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(round, 1);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].0.id, "r1-q1");
    assert_eq!(answers[0].1, "agent: format");

    // Scoped context: the task input
    let asked = answerer.asked.lock().unwrap().clone();
    assert_eq!(asked[0].1["text"], "Attention is all you need");
    // Nobody was asked, so the task never left `working`
    assert_eq!(world.state().await, "working");
    assert!(task_events(&world.requester.state, &world.task_id)
        .await
        .is_empty());
    assert!(world.requester.service.pending().is_empty());

    // Logged just after the answers went out
    let w = &world;
    wait_until("the log", || async move { w.log_lines().len() == 4 }).await;
    let events: Vec<String> = world
        .log_lines()
        .iter()
        .map(|line| line["event"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(events, ["request", "auto", "auto_answered", "responded"]);
}

#[tokio::test]
async fn an_auto_answer_the_agent_cant_give_goes_to_the_human() {
    let world = world(AUTO_FORMAT, Scripted::new(|_| Ok(None))).await;
    let asking = world.ask(vec![question("Bullets or prose?", "format")]);
    let pending = wait_for_pending(&world.requester, 1).await;
    assert_eq!(pending[0].question.id, "r1-q1");
    assert!(pending[0].approval_id.is_some());
    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 1,
            answers: HashMap::from([("r1-q1".into(), "Prose".into())]),
        })
        .await
        .unwrap();
    let (_, answers) = asking.await.unwrap().unwrap();
    assert_eq!(answers[0].1, "Prose");
}

#[tokio::test]
async fn escalation_creates_an_approval_and_the_answer_resumes_the_task() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let asking = world.ask(vec![
        question("Bullets or prose?", "format"),
        question("How much may I spend?", "budget"),
    ]);

    let pending = wait_for_pending(&world.requester, 1).await;
    assert_eq!(pending[0].task_id, world.task_id);
    assert_eq!(pending[0].agent_id, "writer");
    assert_eq!(pending[0].question.topic, "budget");
    assert!(pending[0].deadline.is_some());
    let approval_id = pending[0].approval_id.clone().unwrap();

    let id = approval_id.clone();
    let approval = world
        .requester
        .state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.tool, ESCALATION_TOOL);
    assert_eq!(approval.agent_id, "writer");
    assert_eq!(approval.args["task_id"], world.task_id.as_str());
    assert_eq!(
        approval.args["question"]["question"],
        "How much may I spend?"
    );
    assert_eq!(approval.status, ApprovalStatus::Pending);
    // Not a run approval: the chat's list doesn't show it
    let listed = world
        .requester
        .state
        .db()
        .call(|conn| db::pending_approvals(conn, None))
        .await
        .unwrap();
    assert!(listed.is_empty());

    let w = &world;
    wait_until("input required", || async move {
        w.state().await == "inputRequired"
    })
    .await;
    // Events go out just after the state is written
    let w = &world;
    wait_until("the clarification event", || async move {
        w.requester.sink.names().contains(&CLARIFICATION_REQUESTED)
    })
    .await;
    assert!(world.requester.sink.names().contains(&TASK_STATE_CHANGED));
    // The auto answer waits for the human's
    assert!(world.provider.relay.sent().len() == 1);

    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 0,
            answers: HashMap::from([("r1-q2".into(), "  Up to 20 EUR ".into())]),
        })
        .await
        .unwrap();

    let (_, answers) = asking.await.unwrap().unwrap();
    let answers: HashMap<_, _> = answers.into_iter().map(|(q, a)| (q.topic, a)).collect();
    assert_eq!(answers["format"], "agent: format");
    assert_eq!(answers["budget"], "Up to 20 EUR");

    // Recorded just after the answers went out
    let w = &world;
    wait_until("working", || async move { w.state().await == "working" }).await;
    assert_eq!(
        task_events(&world.requester.state, &world.task_id).await,
        ["inputRequired", "working"]
    );
    assert_eq!(
        approval_status(&world.requester.state, &approval_id).await,
        ApprovalStatus::Approved
    );
    wait_until("answered", || async move {
        w.requester.sink.names().contains(&CLARIFICATION_ANSWERED)
    })
    .await;
    assert!(world.requester.service.pending().is_empty());
    assert!(world
        .requester
        .relay
        .sent()
        .iter()
        .any(|m| matches!(m, PeerMessage::ClarificationResponse(_))));
}

#[tokio::test]
async fn denying_cancels_the_task_on_both_sides() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let asking = world.ask(vec![question("How much may I spend?", "budget")]);
    let pending = wait_for_pending(&world.requester, 1).await;
    let approval_id = pending[0].approval_id.clone().unwrap();

    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 1,
            answers: HashMap::from([("r1-q1".into(), DENY_ANSWER.into())]),
        })
        .await
        .unwrap();

    let error = asking.await.unwrap().unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert_eq!(world.state().await, "cancelled");
    assert_eq!(
        approval_status(&world.requester.state, &approval_id).await,
        ApprovalStatus::Rejected
    );
    assert!(world
        .requester
        .relay
        .sent()
        .iter()
        .any(|m| matches!(m, PeerMessage::CancelTask { .. })));
    // The round is over
    let late = world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 1,
            answers: HashMap::from([("r1-q1".into(), "ok".into())]),
        })
        .await;
    assert!(late.is_err());
}

#[tokio::test]
async fn approve_action_style_decisions_resolve_the_question() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let asking = world.ask(vec![question("How much may I spend?", "budget")]);
    let pending = wait_for_pending(&world.requester, 1).await;
    // Approved: the agent answers this one
    world
        .requester
        .service
        .resolve_approval(pending[0].approval_id.as_deref().unwrap(), true)
        .await
        .unwrap();
    let (_, answers) = asking.await.unwrap().unwrap();
    assert_eq!(answers[0].1, "agent: budget");
}

#[tokio::test]
async fn no_answer_by_the_deadline_times_the_task_out() {
    let world = world("clarification:\n  timeout_seconds: 1\n", Scripted::echo()).await;
    let asking = world.ask(vec![question("Which year?", "scope")]);
    let pending = wait_for_pending(&world.requester, 1).await;
    let approval_id = pending[0].approval_id.clone().unwrap();

    let error = asking.await.unwrap().unwrap_err();
    assert!(error.contains("didn't answer in time"), "{error}");
    assert_eq!(world.state().await, "timedOut");
    assert_eq!(
        task_events(&world.requester.state, &world.task_id).await,
        ["inputRequired", "timedOut"]
    );
    assert_eq!(
        approval_status(&world.requester.state, &approval_id).await,
        ApprovalStatus::Expired
    );
    assert!(world
        .requester
        .relay
        .sent()
        .iter()
        .any(|m| matches!(m, PeerMessage::ClarificationTimeout { .. })));
    assert!(world.requester.service.pending().is_empty());
}

#[tokio::test]
async fn rounds_past_the_limit_fail_the_task() {
    let world = world(
        "clarification:\n  max_rounds: 1\n  auto_topics: [format]\n",
        Scripted::echo(),
    )
    .await;
    world
        .ask(vec![question("Bullets?", "format")])
        .await
        .unwrap()
        .unwrap();
    // The provider may ask twice; the requester allows once
    let error = world
        .ask(vec![question("Headings?", "format")])
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.contains("Too many clarification rounds"), "{error}");
    assert_eq!(world.state().await, "failed");

    // The provider's own bound: no third round is even sent
    let sent = world.provider.relay.sent().len();
    let error = world
        .ask(vec![question("Colors?", "format")])
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.contains("already asked 2 rounds"), "{error}");
    assert_eq!(world.provider.relay.sent().len(), sent);
}

#[tokio::test]
async fn a_policy_that_forbids_questions_fails_the_task() {
    let world = world("clarification:\n  allow: false\n", Scripted::echo()).await;
    let (_, answers) = world
        .ask(vec![question("Bullets?", "format")])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answers[0].1, NOT_ALLOWED_ANSWER);
    let w = &world;
    wait_until("failed", || async move { w.state().await == "failed" }).await;
}

#[tokio::test]
async fn a_question_can_be_forced_to_the_human() {
    let world = world(AUTO_FORMAT, Arc::new(Silent)).await;
    let asking = world.ask(vec![question("Bullets?", "format")]);
    let w = &world;
    wait_until("the auto round", || async move {
        w.requester
            .service
            .escalate(&w.task_id, "r1-q1")
            .await
            .is_ok()
    })
    .await;
    let pending = wait_for_pending(&world.requester, 1).await;
    assert!(pending[0].approval_id.is_some());
    assert!(world
        .requester
        .service
        .escalate(&world.task_id, "nope")
        .await
        .is_err());
    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 1,
            answers: HashMap::from([("r1-q1".into(), "Bullets".into())]),
        })
        .await
        .unwrap();
    let (_, answers) = asking.await.unwrap().unwrap();
    assert_eq!(answers[0].1, "Bullets");
}

#[tokio::test]
async fn allow_always_lets_the_agent_answer_the_topic_from_now_on() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let asking = world.ask(vec![question("When is it due?", "deadline")]);
    wait_for_pending(&world.requester, 1).await;
    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 1,
            answers: HashMap::from([("r1-q1".into(), ALLOW_ALWAYS_ANSWER.into())]),
        })
        .await
        .unwrap();
    let (_, answers) = asking.await.unwrap().unwrap();
    assert_eq!(answers[0].1, "agent: deadline");
    let policy = read_policy(world.requester.state.store(), "writer").unwrap();
    assert_eq!(policy.auto_topics, vec!["Format", "deadline"]);
}

#[tokio::test]
async fn unknown_tasks_are_cancelled() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let service = world.provider.service.clone();
    let turn = DelegatedTurn {
        task_id: "not-a-task".into(),
        requester_instance: world.requester.instance.clone(),
    };
    let error = service
        .ask(&turn, vec![question("?", "format")])
        .await
        .unwrap_err();
    assert!(error.contains("doesn't know this task"), "{error}");
}

#[tokio::test]
async fn local_delegation_is_answered_in_process() {
    let hub = Arc::new(Hub::default());
    let local = side(&hub, Scripted::echo(), ClarificationLog::default()).await;
    save_agent(&local.state, "writer", AUTO_FORMAT);
    let task_id = uuid::Uuid::new_v4().to_string();
    insert_task(&local.state, &task_id, "writer", &local.instance).await;
    let turn = DelegatedTurn {
        task_id,
        requester_instance: local.instance.clone(),
    };
    let (_, answers) = local
        .service
        .ask(&turn, vec![question("Bullets?", "format")])
        .await
        .unwrap();
    assert_eq!(answers[0].1, "agent: format");
    assert!(local.relay.sent().is_empty(), "no relay needed");
}

// ---------------------------------------------------------------------------
// Two delegators, through the tool
// ---------------------------------------------------------------------------

/// The provider's delegated run and its inbound delegation row
async fn delegated_run(world: &World) -> String {
    let provider = &world.provider;
    let requester_card = AgentCard::card_id(&world.requester.instance, "writer");
    let provider_card = AgentCard::card_id(&provider.instance, "papers");
    let run_id = uuid::Uuid::new_v4().to_string();
    let (run, task_id, requester, target) = (
        run_id.clone(),
        world.task_id.clone(),
        requester_card.clone(),
        provider_card.clone(),
    );
    provider
        .state
        .db()
        .call(move |conn| {
            let conversation_id = db::ensure_agent_conversation(conn, "papers")?;
            db::insert_run(
                conn,
                &RunSummary {
                    id: run,
                    agent_id: "papers".into(),
                    conversation_id,
                    status: RunStatus::Running,
                    input: "Summarize".into(),
                    summary: None,
                    started_at: db::now_ms() as f64,
                    ended_at: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: None,
                    model: None,
                    source_type: SourceType::Agent,
                    source_id: Some(requester.clone()),
                    source_label: None,
                    priority: 60,
                    queued_at: None,
                    parent_run_id: None,
                    compaction_summary: None,
                },
            )?;
            conn.execute(
                "INSERT INTO delegations (id, direction, capability, target_agent, requester_agent,
                                          input, status, started_at)
                 VALUES (?1, 'inbound', 'summarize', ?2, ?3, 'null', 'running', ?4)",
                params![task_id, target, requester, db::now_ms()],
            )
        })
        .await
        .unwrap();
    run_id
}

#[tokio::test]
async fn two_delegators_exchange_two_rounds_and_complete() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let run_id = delegated_run(&world).await;
    let provider = world.provider.service.clone();

    assert!(tool::ask_requester_offered(Some(&provider), &run_id, "papers").await);
    assert!(!tool::ask_requester_offered(Some(&provider), "no-run", "papers").await);
    assert_eq!(tool::ask_requester_spec().name, ASK_REQUESTER_TOOL);

    // Round 1: the requester agent answers
    let outcome = tool::ask_requester(
        Some(provider.clone()),
        &run_id,
        "papers",
        &json!({"questions": [{"question": "Bullets or prose?", "topic": "format"}]}),
    )
    .await;
    assert!(!outcome.is_error, "{}", outcome.result);
    let result: Value = serde_json::from_str(&outcome.result).unwrap();
    assert_eq!(result["round"], 1);
    assert_eq!(result["answers"][0]["answer"], "agent: format");

    // Round 2: a person answers
    let asking = {
        let provider = provider.clone();
        let run_id = run_id.clone();
        tokio::spawn(async move {
            tool::ask_requester(
                Some(provider),
                &run_id,
                "papers",
                &json!({"questions": [
                    {"question": "May I buy the PDF?", "topic": "budget", "requires_human": true}
                ]}),
            )
            .await
        })
    };
    let pending = wait_for_pending(&world.requester, 1).await;
    assert_eq!(pending[0].question.id, "r2-q1");
    assert!(pending[0].question.requires_human);
    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 2,
            answers: HashMap::from([("r2-q1".into(), "Yes, up to 30 EUR".into())]),
        })
        .await
        .unwrap();
    let outcome = asking.await.unwrap();
    assert!(!outcome.is_error, "{}", outcome.result);
    let result: Value = serde_json::from_str(&outcome.result).unwrap();
    assert_eq!(result["round"], 2);
    assert_eq!(result["answers"][0]["answer"], "Yes, up to 30 EUR");

    // The provider finishes; the requester records it
    let w = &world;
    wait_until("working", || async move { w.state().await == "working" }).await;
    assert!(transition(
        world.requester.state.db(),
        world.requester.state.bus(),
        &world.task_id,
        TaskState::Completed,
        "done",
    )
    .await
    .unwrap());
    assert_eq!(
        task_events(&world.requester.state, &world.task_id).await,
        ["inputRequired", "working", "completed"]
    );
    // Both rounds are in the task's messages
    let task_id = world.task_id.clone();
    let kinds: Vec<String> = world
        .requester
        .state
        .db()
        .call(move |conn| {
            let mut stmt =
                conn.prepare("SELECT kind FROM task_messages WHERE task_id = ?1 ORDER BY id")?;
            let rows = stmt.query_map([task_id], |r| r.get(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    assert_eq!(
        kinds,
        [
            "clarification_request",
            "clarification_response",
            "clarification_request",
            "clarification_response"
        ]
    );

    // Not a delegated turn: no tool
    let outcome = tool::ask_requester(Some(provider), "no-run", "papers", &json!({})).await;
    assert!(outcome.is_error);
}

// ---------------------------------------------------------------------------
// Merge wiring: the orchestrator and the provider's timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_orchestrator_offers_ask_requester_on_delegated_turns_only() {
    use crate::services::orchestrator::{offer_ask_requester, tool_search, DelegationSlot};
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    let run_id = delegated_run(&world).await;
    let slots = DelegationSlot::default();
    slots.set_clarifier(&world.provider.service);

    // Many tools: deferred loading hides most, never `ask_requester`
    let many: Vec<crate::llm::ToolSpec> = (0..tool_search::DEFERRED_LOAD_THRESHOLD + 5)
        .map(|i| crate::llm::ToolSpec {
            name: format!("tool_{i}"),
            description: "Does a thing".into(),
            input_schema: json!({"type": "object"}),
        })
        .collect();
    let mut tools = tool_search::step_tools(&run_id, many);
    offer_ask_requester(&slots, &run_id, "papers", &mut tools).await;
    offer_ask_requester(&slots, &run_id, "papers", &mut tools).await;
    let offered = tools
        .iter()
        .filter(|t| t.name == ASK_REQUESTER_TOOL)
        .count();
    assert_eq!(offered, 1, "offered once");

    // A turn that isn't delegated, or no service: not offered
    let mut tools = Vec::new();
    offer_ask_requester(&slots, "no-run", "papers", &mut tools).await;
    offer_ask_requester(&DelegationSlot::default(), &run_id, "papers", &mut tools).await;
    assert!(tools.is_empty());
}

#[tokio::test]
async fn the_provider_knows_a_task_is_waiting_for_an_answer() {
    let world = world(AUTO_FORMAT, Scripted::echo()).await;
    assert!(!world.provider.service.is_waiting(&world.task_id));
    let asking = world.ask(vec![question("How much may I spend?", "budget")]);
    wait_for_pending(&world.requester, 1).await;
    assert!(world.provider.service.is_waiting(&world.task_id));
    world
        .requester
        .service
        .respond(ClarificationResponse {
            task_id: world.task_id.clone(),
            round: 1,
            answers: HashMap::from([("r1-q1".into(), "Up to 30 EUR".into())]),
        })
        .await
        .unwrap();
    asking.await.unwrap().unwrap();
    assert!(!world.provider.service.is_waiting(&world.task_id));
}

#[tokio::test]
async fn approve_action_resolves_escalations_through_the_clarification_service() {
    // The state's own service (set by `init`); a local task needs no relay
    let state = AppState::for_tests(vec![]);
    let me = network::local_instance_id(state.db()).unwrap();
    save_agent(&state, "writer", AUTO_FORMAT);
    let task_id = uuid::Uuid::new_v4().to_string();
    insert_task(&state, &task_id, "writer", &me).await;
    let service = state.clarifications().unwrap().clone();
    let turn = DelegatedTurn {
        task_id: task_id.clone(),
        requester_instance: me,
    };
    let asking = {
        let service = service.clone();
        tokio::spawn(async move {
            service
                .ask(&turn, vec![question("How much may I spend?", "budget")])
                .await
        })
    };
    wait_until("the escalation", || {
        let service = service.clone();
        async move { service.pending().len() == 1 }
    })
    .await;
    let approval_id = service.pending()[0].approval_id.clone().unwrap();

    // Through the chat's approval path, not the orchestrator: a rejection
    // denies the question and cancels the task
    crate::services::chat::approve_action(
        &state,
        None,
        approval_id.clone(),
        ApprovalDecision::Rejected,
    )
    .await
    .unwrap();
    let asked = asking.await.unwrap();
    assert!(asked.unwrap_err().contains("cancelled"));
    assert_ne!(
        approval_status(&state, &approval_id).await,
        ApprovalStatus::Pending
    );
    assert_eq!(task_state(&state, &task_id).await, "cancelled");
    state.shutdown();
}
