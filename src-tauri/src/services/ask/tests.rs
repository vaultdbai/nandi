use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use tokio::sync::mpsc;

use super::classifier::{self, Classification};
use super::context::{
    self, AgentBrief, CapabilityBrief, ContextSources, FocusedArtifact, GatewayContext,
    MemberBrief, ViewScope, WorkflowBrief,
};
use super::*;
use crate::model::{RunStatus, RunSummary, SourceType};
use crate::services::events::tests::RecordingSink;

/// Replies from a queue (the last one repeats) and records every call;
/// `stream` sends the reply in word-sized deltas
#[derive(Default)]
pub struct FakeAskLlm {
    replies: Mutex<VecDeque<Result<String, String>>>,
    pub calls: Mutex<Vec<(String, String)>>,
    /// No model can be called (no key, no usable trial)
    no_model: bool,
}

impl FakeAskLlm {
    pub fn replying(replies: &[&str]) -> Arc<Self> {
        let llm = Self::default();
        *llm.replies.lock().unwrap() = replies.iter().map(|r| Ok(r.to_string())).collect();
        Arc::new(llm)
    }

    /// Every call fails (no API key)
    pub fn failing() -> Arc<Self> {
        let llm = Self::default();
        llm.replies
            .lock()
            .unwrap()
            .push_back(Err("Add an API key".to_string()));
        Arc::new(llm)
    }

    /// Nothing can be called at all
    pub fn without_a_model() -> Arc<Self> {
        Arc::new(Self {
            no_model: true,
            ..Self::default()
        })
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }

    fn next(&self, system: &str, prompt: &str) -> Result<String, String> {
        self.calls
            .lock()
            .unwrap()
            .push((system.to_string(), prompt.to_string()));
        let mut replies = self.replies.lock().unwrap();
        if replies.len() > 1 {
            replies.pop_front().unwrap()
        } else {
            replies
                .front()
                .cloned()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }
}

impl AskLlm for FakeAskLlm {
    fn has_model(&self) -> bool {
        !self.no_model
    }

    fn complete<'a>(
        &'a self,
        system: &'a str,
        prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, AskError>> {
        let reply = self.next(system, prompt);
        Box::pin(async move { reply })
    }

    fn stream<'a>(
        &'a self,
        system: &'a str,
        prompt: &'a str,
        deltas: mpsc::Sender<String>,
    ) -> BoxFuture<'a, Result<String, AskError>> {
        let reply = self.next(system, prompt);
        Box::pin(async move {
            let text = reply?;
            for word in text.split_inclusive(' ') {
                let _ = deltas.send(word.to_string()).await;
            }
            Ok(text)
        })
    }
}

fn run(id: &str, agent: &str, status: RunStatus, started_at: f64) -> RunSummary {
    RunSummary {
        id: id.into(),
        agent_id: agent.into(),
        conversation_id: format!("c-{id}"),
        status,
        input: format!("input of {id}"),
        summary: matches!(status, RunStatus::Completed).then(|| format!("output of {id}")),
        started_at,
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: None,
        model: None,
        source_type: SourceType::User,
        source_id: None,
        source_label: None,
        priority: 100,
        queued_at: None,
        parent_run_id: None,
        compaction_summary: None,
    }
}

fn capability(id: &str, owned: bool, provided: bool, used: bool) -> CapabilityBrief {
    CapabilityBrief {
        id: id.into(),
        name: id.into(),
        version: "1.0.0".into(),
        description: format!("Does {id}"),
        owned,
        provided,
        used,
        online_providers: 1,
        cost_per_call_usd: None,
        sla_seconds: 300,
        approval_required: false,
        tags: Vec::new(),
    }
}

fn sources() -> ContextSources {
    ContextSources {
        runs: vec![
            run("r-old", "writer", RunStatus::Completed, 1.0),
            run("r-new", "writer", RunStatus::Completed, 5.0),
            run("r-failed", "writer", RunStatus::Failed, 4.0),
            run("r-live", "reviewer", RunStatus::Running, 6.0),
            run("r-wait", "writer", RunStatus::AwaitingApproval, 2.0),
        ],
        capabilities: vec![
            capability("code-review.python", true, true, false),
            capability("summarize", false, false, true),
        ],
        agents: vec![AgentBrief {
            id: "writer".into(),
            name: "Writer".into(),
            description: "Writes things".into(),
        }],
        workflows: vec![WorkflowBrief {
            id: "onboarding".into(),
            name: "Onboarding".into(),
            steps: vec!["writer".into(), "reviewer".into()],
        }],
        policies: Vec::new(),
        members: vec![MemberBrief {
            instance_id: "alice".into(),
            name: "Alice".into(),
            online: true,
            accepts_delegation: true,
            capabilities: vec!["code-review.python".into()],
        }],
        waiting: vec![context::WaitingBrief {
            kind: "manual_step".into(),
            id: "run-9".into(),
            title: "Onboarding Aayushi".into(),
            detail: "Add the laptop tag".into(),
        }],
    }
}

#[test]
fn the_context_says_what_waits_on_the_owner() {
    let text = context::describe(&context_for("work"));
    assert!(
        text.contains("Workflow \"Onboarding Aayushi\" needs them"),
        "{text}"
    );
    assert!(text.contains("Add the laptop tag"));
}

fn context_for(view: &str) -> GatewayContext {
    let view = ViewScope::parse(Some(view));
    let focused = view.artifact().map(|artifact| FocusedArtifact {
        artifact,
        content: Some("---\nname: X\n---\nBody".into()),
    });
    context::build(sources(), view, focused, &[])
}

// --- Context -----------------------------------------------------------

#[test]
fn views_parse_into_scopes() {
    assert_eq!(ViewScope::parse(None), ViewScope::Home);
    assert_eq!(ViewScope::parse(Some("home")), ViewScope::Home);
    assert_eq!(
        ViewScope::parse(Some("agent:writer")),
        ViewScope::Agent("writer".into())
    );
    assert_eq!(
        ViewScope::parse(Some("capability:code-review.python")),
        ViewScope::Capability("code-review.python".into())
    );
    assert_eq!(
        ViewScope::parse(Some("workflow:abc")),
        ViewScope::Workflow("abc".into())
    );
    assert_eq!(
        ViewScope::parse(Some("member:alice")),
        ViewScope::Member("alice".into())
    );
    assert_eq!(
        ViewScope::parse(Some("studio")),
        ViewScope::Other("studio".into())
    );
}

#[test]
fn a_home_context_has_no_focus_and_splits_active_from_recent_runs() {
    let context = context_for("home");
    assert_eq!(context.view, ViewScope::Home);
    assert!(context.focused.is_none());
    let active: Vec<&str> = context.active_runs.iter().map(|r| r.id.as_str()).collect();
    // Newest first
    assert_eq!(active, ["r-live", "r-wait"]);
    // Completed only, newest first
    let recent: Vec<&str> = context
        .recent_outputs
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(recent, ["r-new", "r-old"]);
    assert_eq!(
        context.recent_outputs[0].output.as_deref(),
        Some("output of r-new")
    );
    assert_eq!(context.capabilities.len(), 2);
    assert_eq!(context.agents.len(), 1);
    assert_eq!(context.workflows.len(), 1);
    assert_eq!(context.members.len(), 1);
}

#[test]
fn agent_capability_and_workflow_views_focus_their_artifact() {
    for (view, artifact) in [
        ("agent:writer", ArtifactRef::Agent("writer".into())),
        (
            "capability:code-review.python",
            ArtifactRef::Capability("code-review.python".into()),
        ),
        (
            "workflow:onboarding",
            ArtifactRef::Workflow("onboarding".into()),
        ),
    ] {
        let context = context_for(view);
        let focused = context.focused.expect(view);
        assert_eq!(focused.artifact, artifact, "{view}");
        assert!(focused.content.is_some());
        let described = context::describe(&context_for(view));
        assert!(described.contains("## The artifact in focus"), "{view}");
    }
}

#[test]
fn runs_the_user_was_looking_at_come_first() {
    let pinned = vec!["r-failed".to_string()];
    let context = context::build(sources(), ViewScope::Home, None, &pinned);
    assert_eq!(context.recent_outputs[0].id, "r-failed");
}

#[test]
fn the_description_lists_every_section() {
    let described = context::describe(&context_for("home"));
    for needle in [
        "## Where the owner is",
        "The home dashboard",
        "## Running now",
        "run `r-live` by `reviewer`",
        "## Recent outputs",
        "## Capabilities",
        "`code-review.python` v1.0.0",
        "owned, provided here",
        "## Agents",
        "## Workflows",
        "writer → reviewer",
        "## Policies",
        "(none)",
        "## Network members",
        "`alice` (Alice): online, takes requests",
    ] {
        assert!(described.contains(needle), "missing {needle}:\n{described}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_loader_reads_the_repo_for_each_kind_of_view() {
    let state = AppState::for_tests(vec![]);
    let layout = state.store().layout().clone();
    let skill = layout.skill_path("writer");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: Writer\ndescription: Writes\n---\nWrite.",
    )
    .unwrap();
    std::fs::create_dir_all(&layout.workflows_dir).unwrap();
    std::fs::write(
        layout.workflows_dir.join("daily.yaml"),
        "id: daily\nname: Daily\nsteps:\n  - id: one\n    agent: writer\n    input: hi\n",
    )
    .unwrap();
    state.store().load().unwrap();

    let home = context::load(&state, &AskContext::default()).await;
    assert_eq!(home.view, ViewScope::Home);
    assert!(home.focused.is_none());
    assert!(home
        .agents
        .iter()
        .any(|a| a.id == "writer" && a.name == "Writer"));
    assert!(home
        .workflows
        .iter()
        .any(|w| w.id == "daily" && w.steps == ["writer"]));
    assert!(home.policies.iter().any(|p| p.id == "gateway"));

    let agent = context::load(
        &state,
        &AskContext {
            current_view: Some("agent:writer".into()),
            ..AskContext::default()
        },
    )
    .await;
    let focused = agent.focused.unwrap();
    assert_eq!(focused.artifact, ArtifactRef::Agent("writer".into()));
    assert!(focused.content.unwrap().contains("name: Writer"));

    let workflow = context::load(
        &state,
        &AskContext {
            current_view: Some("workflow:daily".into()),
            ..AskContext::default()
        },
    )
    .await;
    assert!(workflow
        .focused
        .unwrap()
        .content
        .unwrap()
        .contains("id: daily"));

    // A capability with no file yet: in focus, but unread
    let capability = context::load(
        &state,
        &AskContext {
            current_view: Some("capability:code-review.python".into()),
            ..AskContext::default()
        },
    )
    .await;
    let focused = capability.focused.unwrap();
    assert_eq!(
        focused.artifact,
        ArtifactRef::Capability("code-review.python".into())
    );
    assert!(focused.content.is_none());
    state.shutdown();
}

// --- Classifier ----------------------------------------------------------

const FIXTURES: &[(&str, &str, AskKind)] = &[
    ("home", "What is running right now?", AskKind::Question),
    ("home", "How many runs failed this week?", AskKind::Question),
    (
        "home",
        "Who on my network can review Python?",
        AskKind::Question,
    ),
    (
        "agent:writer",
        "Is the writer agent busy?",
        AskKind::Question,
    ),
    ("home", "Review PR #42 in nandi", AskKind::Action),
    (
        "home",
        "Draft the Q4 metrics summary for me",
        AskKind::Action,
    ),
    ("home", "Ask Alice to review my Rust code", AskKind::Action),
    ("agent:writer", "Rename this agent to Scribe", AskKind::Edit),
    (
        "capability:code-review.python",
        "Change the SLA to 60 seconds",
        AskKind::Edit,
    ),
    (
        "home",
        "Add a step to the onboarding workflow that emails the new hire",
        AskKind::Edit,
    ),
];

#[tokio::test]
async fn ten_fixture_asks_classify_without_a_model() {
    let llm = FakeAskLlm::failing();
    for (view, text, kind) in FIXTURES {
        let classification = classifier::classify(llm.as_ref(), text, &context_for(view)).await;
        assert_eq!(classification.kind, *kind, "{text}");
        assert!(
            classification.confidence >= classifier::MIN_CONFIDENCE,
            "{text}"
        );
    }
}

#[tokio::test]
async fn ten_fixture_asks_classify_with_the_model() {
    for (view, text, kind) in FIXTURES {
        let reply = format!(
            "```json\n{{\"kind\": \"{}\", \"confidence\": 0.92, \"reason\": \"fixture\"}}\n```",
            match kind {
                AskKind::Question => "question",
                AskKind::Action => "action",
                AskKind::Edit => "edit",
            }
        );
        let llm = FakeAskLlm::replying(&[&reply]);
        let classification = classifier::classify(llm.as_ref(), text, &context_for(view)).await;
        assert_eq!(
            classification,
            Classification {
                kind: *kind,
                confidence: 0.92,
                reason: "fixture".into()
            }
        );
        let (system, prompt) = &llm.calls()[0];
        assert!(system.contains("Reply with JSON only"));
        assert!(prompt.contains(text));
        if view.starts_with("agent:") {
            assert!(prompt.contains("In focus: agent `writer`"));
        }
    }
}

#[tokio::test]
async fn an_unreadable_classification_falls_back_to_keywords() {
    let llm = FakeAskLlm::replying(&["{\"kind\": \"dance\"}"]);
    let classification =
        classifier::classify(llm.as_ref(), "What failed today?", &context_for("home")).await;
    assert_eq!(classification.kind, AskKind::Question);
    assert!(classifier::parse("no json here").is_err());
    assert_eq!(
        classifier::parse("Sure: {\"kind\":\"Edit\"}").unwrap().kind,
        AskKind::Edit
    );
}

// --- Answers and streaming ------------------------------------------------

const QUESTION: &str = r#"{"kind":"question","confidence":0.9,"reason":"asks"}"#;
const ANSWER: &str =
    "Run `r-live` is in progress for `reviewer`, and `writer` is idle.\nFOLLOWUPS: What failed? | Who can help?";

#[test]
fn answers_split_off_followups_and_cite_what_they_mention() {
    let answer = parse_answer(ANSWER, &context_for("home"));
    assert_eq!(
        answer.answer,
        "Run `r-live` is in progress for `reviewer`, and `writer` is idle."
    );
    assert_eq!(answer.followups, ["What failed?", "Who can help?"]);
    let sources: Vec<(&str, &str)> = answer
        .sources
        .iter()
        .map(|s| (s.kind.as_str(), s.id.as_str()))
        .collect();
    assert_eq!(sources, [("run", "r-live"), ("agent", "writer")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_question_is_answered_inline() {
    let state = AppState::for_tests(vec![]);
    let llm = FakeAskLlm::replying(&[QUESTION, ANSWER]);
    let service = AskService::new(state.downgrade(), llm.clone());
    let response = service
        .respond(
            &AskRequest {
                text: "What's running?".into(),
                ..AskRequest::default()
            },
            None,
        )
        .await
        .unwrap();
    let AskResponse::Question(answer) = response else {
        panic!("expected an answer, got {response:?}");
    };
    assert!(answer.answer.starts_with("Run `r-live`"));
    let calls = llm.calls();
    assert_eq!(calls.len(), 2, "classify, then answer");
    assert!(calls[1].0.contains("## Running now"));
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_model_the_help_guide_answers_without_calling_one() {
    let state = AppState::for_tests(vec![]);
    let llm = FakeAskLlm::without_a_model();
    let service = AskService::new(state.downgrade(), llm.clone());
    let response = service
        .respond(
            &AskRequest {
                text: "How do I install Docker?".into(),
                ..AskRequest::default()
            },
            None,
        )
        .await
        .unwrap();
    let AskResponse::Help(help) = response else {
        panic!("expected the help guide, got {response:?}");
    };
    assert_eq!(help.topic.unwrap().id, "install-docker");
    assert_eq!(help.reason, HelpReason::NoModel);
    assert!(llm.calls().is_empty(), "no model was tried");
    // Remembered like any ask
    let history = list_ask_history(&state, None).await.unwrap();
    assert!(matches!(history[0].response, AskResponse::Help(_)));
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_model_falls_back_to_the_help_guide() {
    let state = AppState::for_tests(vec![]);
    // The classifier reads the question; the answer call fails
    let llm = FakeAskLlm::default();
    llm.replies.lock().unwrap().extend([
        Ok(QUESTION.to_string()),
        Err("Can't reach the free trial right now.".to_string()),
    ]);
    let service = AskService::new(state.downgrade(), Arc::new(llm));
    let response = service
        .respond(
            &AskRequest {
                text: "What is an agent?".into(),
                ..AskRequest::default()
            },
            None,
        )
        .await
        .unwrap();
    let AskResponse::Help(help) = response else {
        panic!("expected the help guide, got {response:?}");
    };
    assert_eq!(help.topic.unwrap().id, "agents");
    assert_eq!(help.reason, HelpReason::ModelFailed);
    assert!(help.detail.unwrap().contains("free trial"));
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_low_confidence_ask_gets_a_question_back() {
    let state = AppState::for_tests(vec![]);
    let llm = FakeAskLlm::replying(&[r#"{"kind":"action","confidence":0.2}"#]);
    let service = AskService::new(state.downgrade(), llm);
    let response = service
        .respond(
            &AskRequest {
                text: "the thing".into(),
                ..AskRequest::default()
            },
            None,
        )
        .await
        .unwrap();
    assert!(matches!(response, AskResponse::Clarification(_)));
    assert!(service.respond(&AskRequest::default(), None).await.is_err());
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_emits_deltas_then_completed() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    let llm = FakeAskLlm::replying(&[QUESTION, ANSWER]);
    let service = Arc::new(AskService::new(state.downgrade(), llm));
    let request_id = spawn_streaming(
        service,
        state.bus().clone(),
        state.runtime(),
        AskRequest {
            text: "What's running?".into(),
            ..AskRequest::default()
        },
    );
    let mut events = Vec::new();
    for _ in 0..200 {
        events = recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.starts_with("ask://"))
            .cloned()
            .collect::<Vec<_>>();
        if events.iter().any(|(name, _)| *name == ASK_COMPLETED) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let names: Vec<&str> = events.iter().map(|(name, _)| *name).collect();
    let last = names.len() - 1;
    assert!(last > 1, "{names:?}");
    assert!(names[..last].iter().all(|n| *n == ASK_STREAMING_DELTA));
    assert_eq!(names[last], ASK_COMPLETED);
    let streamed: String = events[..last]
        .iter()
        .map(|(_, payload)| {
            assert_eq!(payload["requestId"], request_id);
            payload["delta"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(streamed, ANSWER);
    let completed = &events[last].1;
    assert_eq!(completed["requestId"], request_id);
    assert_eq!(completed["response"]["kind"], "question");
    assert!(completed["error"].is_null());
    state.shutdown();
}

// --- Planner ---------------------------------------------------------------

use super::editor::{self, ImpactFacts};
use super::planner;

fn plan_targets(plan: &ActionPlan) -> Vec<(PlanStepKind, &str)> {
    plan.steps
        .iter()
        .map(|s| (s.kind, s.target.as_str()))
        .collect()
}

#[tokio::test]
async fn five_fixture_asks_get_the_right_plan() {
    let llm = FakeAskLlm::failing();
    let home = context_for("home");

    // One capability: a delegation
    let one = planner::plan(llm.as_ref(), "Review my Python code", &home).await;
    assert_eq!(
        plan_targets(&one),
        [(PlanStepKind::Capability, "code-review.python")]
    );
    assert_eq!(one.estimated_duration_seconds, Some(300));
    assert!(one.rejection.is_none());
    assert!(one.steps[0].description.contains("provided here"));

    // Several: a workflow, in the order asked
    let two = planner::plan(
        llm.as_ref(),
        "Review my Python code and then summarize the findings",
        &home,
    )
    .await;
    assert_eq!(
        plan_targets(&two),
        [
            (PlanStepKind::Capability, "code-review.python"),
            (PlanStepKind::Capability, "summarize"),
        ]
    );
    assert_eq!(two.estimated_duration_seconds, Some(600));
    assert!(two.warnings.iter().any(|w| w.contains("as a new workflow")));

    // An existing workflow
    let workflow = planner::plan(
        llm.as_ref(),
        "Run the onboarding workflow for Aayushi",
        &home,
    )
    .await;
    assert_eq!(
        plan_targets(&workflow),
        [(PlanStepKind::Workflow, "onboarding")]
    );

    // A local agent
    let agent = planner::plan(llm.as_ref(), "Write the release notes", &home).await;
    assert_eq!(plan_targets(&agent), [(PlanStepKind::LocalAgent, "writer")]);

    // Out of scope
    let none = planner::plan(llm.as_ref(), "Book me a flight to Lisbon", &home).await;
    assert!(none.steps.is_empty());
    assert!(none.rejection.unwrap().contains("Nothing on this instance"));
}

#[tokio::test]
async fn the_model_plan_is_kept_to_what_exists() {
    let llm = FakeAskLlm::replying(&[r#"{"steps": [
        {"kind": "capability", "target": "summarize", "description": "Summarize the thread"},
        {"kind": "capability", "target": "made-up", "description": "Nope"},
        {"kind": "human", "target": "alice", "description": ""}
    ]}"#]);
    let plan = planner::plan(
        llm.as_ref(),
        "Summarize this and loop in Alice",
        &context_for("home"),
    )
    .await;
    assert_eq!(
        plan_targets(&plan),
        [
            (PlanStepKind::Capability, "summarize"),
            (PlanStepKind::Human, "alice"),
        ]
    );
    assert_eq!(plan.steps[0].description, "Summarize the thread");
    assert_eq!(plan.steps[1].description, "Ask Alice");

    let rejected = FakeAskLlm::replying(&[r#"{"steps": [], "rejection": "No one does taxes"}"#]);
    let plan = planner::plan(rejected.as_ref(), "Do my taxes", &context_for("home")).await;
    assert_eq!(plan.rejection.as_deref(), Some("No one does taxes"));
}

#[tokio::test]
async fn the_planner_may_give_the_chief_of_staff_one_step() {
    use crate::services::builtin_agents::CHIEF_OF_STAFF;

    let mut context = context_for("home");
    context.agents.push(AgentBrief {
        id: CHIEF_OF_STAFF.into(),
        name: "Chief of Staff".into(),
        description: "Your representative.".into(),
    });
    // Its line says what it can do itself
    let described = context::describe(&context);
    assert!(
        described.contains("Can do one step of a plan itself: a task on a website"),
        "{described}"
    );
    let llm = FakeAskLlm::replying(&[r#"{"steps": [
        {"kind": "localAgent", "target": "chief-of-staff", "description": "Pay the bill on acme-energy.com"},
        {"kind": "localAgent", "target": "writer", "description": "Write the note"},
        {"kind": "localAgent", "target": "chief-of-staff", "description": "File the receipt"}
    ]}"#]);
    let plan = planner::plan(llm.as_ref(), "Pay the Acme bill", &context).await;
    assert_eq!(
        plan_targets(&plan),
        [
            (PlanStepKind::LocalAgent, CHIEF_OF_STAFF),
            (PlanStepKind::LocalAgent, "writer"),
        ]
    );
    assert!(llm.calls()[0].0.contains("Never more than one step for it"));
}

#[test]
fn plans_warn_about_offline_providers_and_approval() {
    let mut context = context_for("home");
    context.capabilities[1].online_providers = 0;
    context.capabilities[1].approval_required = true;
    context.capabilities[1].cost_per_call_usd = Some(0.25);
    let plan = planner::estimate(
        vec![PlanStep {
            kind: PlanStepKind::Capability,
            target: "summarize".into(),
            description: String::new(),
        }],
        &context,
    );
    assert_eq!(plan.estimated_cost_usd, Some(0.25));
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.contains("No provider of summarize is online")));
    assert!(plan.warnings.iter().any(|w| w.contains("needs approval")));
}

// --- Edit previews -----------------------------------------------------------

const AGENT_BEFORE: &str = "---\nname: Writer\ndescription: Writes\n---\nWrite clearly.\n";
const WORKFLOW_BEFORE: &str = "id: onboarding\nname: Onboarding\nsteps:\n  - id: welcome\n    agent: writer\n    input: Welcome them\n";
const CAP_BEFORE: &str = "---\nid: code-review.python\nname: Python review\ndescription: Reviews Python\nversion: 1.2.3\nowner:\n  kind: user\n  id: ada\n  display_name: Ada\npolicy:\n  sla_seconds: 300\n---\n\nReview it.\n";

#[test]
fn agent_edits_produce_a_valid_diff() {
    let after = AGENT_BEFORE.replace("name: Writer", "name: Scribe");
    let preview = editor::build(
        &ArtifactRef::Agent("writer".into()),
        AGENT_BEFORE,
        &after,
        &ImpactFacts {
            workflows: vec!["Onboarding".into()],
            active_runs: 1,
            ..ImpactFacts::default()
        },
    )
    .unwrap();
    assert!(preview.diff.contains("--- a/agents/writer/skill.md"));
    assert!(preview.diff.contains("+++ b/agents/writer/skill.md"));
    assert!(preview.diff.contains("-name: Writer"));
    assert!(preview.diff.contains("+name: Scribe"));
    assert!(preview.diff.contains("@@"));
    assert_eq!(preview.version_bump, None);
    assert!(preview
        .impact
        .iter()
        .any(|i| i.contains("Workflow Onboarding")));
    assert!(preview.impact.iter().any(|i| i.contains("1 run in flight")));

    // Not an agent any more
    assert!(editor::build(
        &ArtifactRef::Agent("writer".into()),
        AGENT_BEFORE,
        "---\nname: [unclosed\n---\nBody\n",
        &ImpactFacts::default()
    )
    .is_err());
    // No change
    assert!(editor::build(
        &ArtifactRef::Agent("writer".into()),
        AGENT_BEFORE,
        AGENT_BEFORE,
        &ImpactFacts::default()
    )
    .unwrap_err()
    .contains("doesn't change anything"));
}

#[test]
fn workflow_edits_produce_a_valid_diff() {
    let after = format!(
        "{WORKFLOW_BEFORE}  - id: email\n    agent: writer\n    input: Email the new hire\n"
    );
    let preview = editor::build(
        &ArtifactRef::Workflow("onboarding".into()),
        WORKFLOW_BEFORE,
        &after,
        &ImpactFacts::default(),
    )
    .unwrap();
    assert!(preview.diff.contains("+  - id: email"));
    assert!(preview
        .impact
        .iter()
        .any(|i| i.contains("from 1 to 2 steps")));
    // The ID can't change
    let renamed = WORKFLOW_BEFORE.replace("id: onboarding", "id: welcome");
    assert!(editor::build(
        &ArtifactRef::Workflow("onboarding".into()),
        WORKFLOW_BEFORE,
        &renamed,
        &ImpactFacts::default()
    )
    .unwrap_err()
    .contains("ID can't change"));
}

#[test]
fn capability_edits_get_the_bump_they_need_and_name_subscribers() {
    let artifact = ArtifactRef::Capability("code-review.python".into());
    let facts = ImpactFacts {
        providers: vec!["Reviewer".into()],
        members: vec!["Alice".into()],
        used: true,
        ..ImpactFacts::default()
    };
    // An SLA change with the version untouched: bumped to 1.3.0
    let after = CAP_BEFORE.replace("sla_seconds: 300", "sla_seconds: 60");
    let preview = editor::build(&artifact, CAP_BEFORE, &after, &facts).unwrap();
    assert_eq!(preview.version_bump.as_deref(), Some("minor"));
    assert!(preview.after.contains("version: 1.3.0"));
    assert!(preview.diff.contains("+version: 1.3.0"));
    assert!(preview.diff.contains("+  sla_seconds: 60"));
    assert!(preview
        .impact
        .iter()
        .any(|i| i == "2 subscribers must pick up the new version: Reviewer, Alice"));
    assert!(preview
        .impact
        .iter()
        .any(|i| i.contains("This instance calls")));

    // A description tweak is a patch
    let patch = CAP_BEFORE.replace("Reviews Python\n", "Reviews Python code\n");
    let preview = editor::build(&artifact, CAP_BEFORE, &patch, &facts).unwrap();
    assert_eq!(preview.version_bump.as_deref(), Some("patch"));
    assert!(preview.after.contains("version: 1.2.4"));

    // Illegal: a tool requirement bumped as a patch
    let illegal = CAP_BEFORE
        .replace("version: 1.2.3", "version: 1.2.4")
        .replace("policy:\n", "required_tools: [git]\npolicy:\n");
    let error = editor::build(&artifact, CAP_BEFORE, &illegal, &facts).unwrap_err();
    assert!(error.contains("needs a major version bump"), "{error}");

    // A breaking change says so
    let major = CAP_BEFORE.replace("policy:\n", "required_tools: [git]\npolicy:\n");
    let preview = editor::build(&artifact, CAP_BEFORE, &major, &ImpactFacts::default()).unwrap();
    assert_eq!(preview.version_bump.as_deref(), Some("major"));
    assert!(preview.after.contains("version: 2.0.0"));
    assert!(preview.impact[0].starts_with("Breaking change"));
    assert!(preview.impact.iter().any(|i| i.contains("No subscribers")));
}

#[test]
fn edit_asks_find_their_artifact() {
    let home = context_for("home");
    assert_eq!(
        editor::find_artifact("Add a step to the onboarding workflow", &home),
        Some(ArtifactRef::Workflow("onboarding".into()))
    );
    assert_eq!(
        editor::find_artifact("Set code-review.python to a one minute SLA", &home),
        Some(ArtifactRef::Capability("code-review.python".into()))
    );
    assert_eq!(
        editor::find_artifact("Tighten NANDI.md", &home),
        Some(ArtifactRef::Gateway)
    );
    assert_eq!(editor::find_artifact("Make it nicer", &home), None);
    assert_eq!(
        editor::find_artifact("Make it nicer", &context_for("agent:writer")),
        Some(ArtifactRef::Agent("writer".into()))
    );
    assert_eq!(editor::strip_fences("```yaml\nid: x\n```"), "id: x");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edit_ask_previews_the_change_with_subscribers() {
    let state = AppState::for_tests(vec![]);
    let layout = state.store().layout().clone();
    let skill = layout.skill_path("reviewer");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: Reviewer\nallowed-tools: [shell]\nprovides: [code-review.python]\n---\nReview.",
    )
    .unwrap();
    std::fs::create_dir_all(layout.capabilities_dir()).unwrap();
    std::fs::write(layout.capability_path("code-review.python"), CAP_BEFORE).unwrap();
    state.store().load().unwrap();
    let capabilities = state.capabilities().unwrap().clone();
    capabilities.rebuild().await.unwrap();
    capabilities.refresh_local_providers().await.unwrap();

    let after = CAP_BEFORE.replace("sla_seconds: 300", "sla_seconds: 60");
    let fenced = format!("```yaml\n{after}```");
    let llm = FakeAskLlm::replying(&[
        r#"{"kind":"edit","confidence":0.95,"reason":"changes the SLA"}"#,
        &fenced,
    ]);
    let service = AskService::new(state.downgrade(), llm.clone());
    let response = service
        .respond(
            &AskRequest {
                text: "Change the SLA to 60 seconds".into(),
                attachments: Vec::new(),
                context: AskContext {
                    current_view: Some("capability:code-review.python".into()),
                    ..AskContext::default()
                },
            },
            None,
        )
        .await
        .unwrap();
    let AskResponse::Edit(preview) = response else {
        panic!("expected an edit preview, got {response:?}");
    };
    assert_eq!(
        preview.artifact,
        ArtifactRef::Capability("code-review.python".into())
    );
    assert_eq!(preview.before, CAP_BEFORE);
    assert!(preview.after.contains("version: 1.3.0"));
    assert!(
        preview.impact.iter().any(|i| i.contains("Reviewer")),
        "{:?}",
        preview.impact
    );
    // The edit prompt holds only that file
    let (system, prompt) = &llm.calls()[1];
    assert!(system.starts_with("You edit one file"));
    assert!(prompt.contains("sla_seconds: 300"));
    assert!(prompt.ends_with("Instruction: Change the SLA to 60 seconds"));
    // Nothing was written
    assert_eq!(
        std::fs::read_to_string(layout.capability_path("code-review.python")).unwrap(),
        CAP_BEFORE
    );
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_action_ask_gets_a_plan() {
    let state = AppState::for_tests(vec![]);
    let llm = FakeAskLlm::replying(&[
        r#"{"kind":"action","confidence":0.9}"#,
        r#"{"steps": [], "rejection": "Nothing here books flights"}"#,
    ]);
    let service = AskService::new(state.downgrade(), llm);
    let response = service
        .respond(
            &AskRequest {
                text: "Book a flight".into(),
                ..AskRequest::default()
            },
            None,
        )
        .await
        .unwrap();
    let AskResponse::Action(plan) = response else {
        panic!("expected a plan, got {response:?}");
    };
    assert_eq!(
        plan.rejection.as_deref(),
        Some("Nothing here books flights")
    );
    state.shutdown();
}

// --- History -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn asks_are_remembered_newest_first() {
    let state = AppState::for_tests(vec![]);
    let llm = FakeAskLlm::replying(&[QUESTION, ANSWER, r#"{"kind":"action","confidence":0.1}"#]);
    let service = AskService::new(state.downgrade(), llm);
    let first = AskRequest {
        text: "What's running?".into(),
        context: AskContext {
            current_view: Some("agent:writer".into()),
            ..AskContext::default()
        },
        ..AskRequest::default()
    };
    service.respond(&first, None).await.unwrap();
    service
        .respond(
            &AskRequest {
                text: "the thing".into(),
                ..AskRequest::default()
            },
            None,
        )
        .await
        .unwrap();
    // Failed asks aren't remembered
    assert!(service.respond(&AskRequest::default(), None).await.is_err());

    let history = list_ask_history(&state, None).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].request_text, "the thing");
    assert!(matches!(history[0].response, AskResponse::Clarification(_)));
    assert_eq!(history[1].request_text, "What's running?");
    assert!(matches!(history[1].response, AskResponse::Question(_)));
    assert_eq!(
        history[1].context.as_ref().unwrap().current_view.as_deref(),
        Some("agent:writer")
    );
    assert_eq!(list_ask_history(&state, Some(1)).await.unwrap().len(), 1);
    state.shutdown();
}
