//! Tests: runs through the turn middleware (pauses, plan mode, the run plan,
//! playbooks and the finish check)

use super::*;
use crate::db::ApprovalRow;
use crate::services::domain::{Agent, AgentId, EntityKind};
use crate::services::events::tests::RecordingSink;
use crate::services::repos::Repository;
use crate::services::AppState;
use futures_util::future::BoxFuture;
use middleware::{CostCapMiddleware, MiddlewareError, ReadOnlyMiddleware, TurnLimitMiddleware};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

fn deps(state: &AppState) -> RunDeps {
    RunDeps {
        bus: state.bus().clone(),
        secrets: state.secrets().clone(),
        runtimes: state.runtimes().clone(),
        store: state.store().clone(),
        connectors: None,
        harness: Default::default(),
    }
}

fn setup(script: Value) -> (AppState, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![sink.clone()]);
    let conn = state.bus().db.conn().unwrap();
    db::set_setting(&conn, db::ENGINE_KEY, &json!("canned")).unwrap();
    let mut script = script;
    script["tokenDelayMs"] = json!(0);
    script["studio"] = json!([]);
    db::set_setting(&conn, crate::engine::CANNED_SCRIPT_KEY, &script).unwrap();
    (state, sink)
}

fn save_agent(state: &AppState, id: &str, frontmatter: &str) {
    let agent = Agent::from_agent_md(
        AgentId::parse(id).unwrap(),
        &format!("---\nname: {id}\n{frontmatter}---\nHelp the user.\n"),
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, id).unwrap();
}

/// Persists a running run for `agent_id`, as `accept` does
async fn start(state: &AppState, agent_id: &str, run_id: &str) -> RunContext {
    let id = agent_id.to_string();
    let conversation = state
        .bus()
        .db
        .call(move |conn| db::ensure_agent_conversation(conn, &id))
        .await
        .unwrap();
    let ctx = RunContext {
        agent_id: agent_id.into(),
        agent_name: agent_id.into(),
        conversation_id: conversation.clone(),
        run_id: run_id.into(),
    };
    let run = RunSummary {
        id: run_id.into(),
        agent_id: agent_id.into(),
        conversation_id: conversation.clone(),
        status: RunStatus::Running,
        input: "plan the change".into(),
        summary: None,
        started_at: db::ts(db::now_ms()),
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: None,
        model: None,
        source_type: SourceType::User,
        source_id: None,
        source_label: None,
        priority: PRIORITY_USER,
        queued_at: None,
        parent_run_id: None,
        compaction_summary: None,
    };
    let message = MessageRow::new(
        format!("user-{run_id}"),
        &conversation,
        Some(run_id),
        role::USER,
        "plan the change",
        UserMeta::default(),
    );
    assert!(state
        .bus()
        .start_run(ctx.clone(), run, message)
        .await
        .unwrap());
    ctx
}

fn turn(agent_id: &str, run_id: &str) -> TurnContext {
    TurnContext {
        agent_id: agent_id.into(),
        run_id: run_id.into(),
        context_window: middleware::DEFAULT_CONTEXT_WINDOW,
        ..TurnContext::default()
    }
}

async fn drive(
    state: &AppState,
    agent_id: &str,
    run_id: &str,
    pipeline: &[Box<dyn TurnMiddleware>],
) -> RunEnd {
    drive_run(
        &deps(state),
        &DelegationSlot::default(),
        run_id,
        pipeline,
        turn(agent_id, run_id),
    )
    .await
    .unwrap()
}

async fn run_of(state: &AppState, run_id: &str) -> RunSummary {
    let id = run_id.to_string();
    state
        .bus()
        .db
        .call(move |c| db::get_run(c, &id))
        .await
        .unwrap()
        .unwrap()
}

async fn messages_of(state: &AppState, run_id: &str) -> Vec<MessageRow> {
    let id = run_id.to_string();
    state
        .bus()
        .db
        .call(move |c| db::list_run_messages(c, &id))
        .await
        .unwrap()
}

fn two_steps() -> Value {
    json!({
        "agent": [
            {
                "text": "Looking around.",
                "toolCalls": [{ "tool": "list_dir", "args": { "path": "." }, "result": "[]" }]
            },
            { "text": "All done." }
        ]
    })
}

#[tokio::test]
async fn the_turn_limit_aborts_the_run_cleanly() {
    let (state, sink) = setup(two_steps());
    start(&state, "writer", "r1").await;
    let pipeline: Vec<Box<dyn TurnMiddleware>> =
        vec![Box::new(TurnLimitMiddleware { max_turns: 1 })];

    assert_eq!(drive(&state, "writer", "r1", &pipeline).await, RunEnd::Done);

    let run = run_of(&state, "r1").await;
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.summary.unwrap().contains("limit of 1 turns"));
    let messages = messages_of(&state, "r1").await;
    let roles: Vec<_> = messages.iter().map(|m| m.role.as_str()).collect();
    // One step ran (with its tool call); the second never started
    assert_eq!(roles, ["user", "assistant", "tool_call", "error"]);
    assert!(messages[3].content.contains("max_turns"));

    let events = sink.take();
    let aborted: Vec<_> = events
        .iter()
        .filter(|(name, _)| *name == "agent://turn_aborted")
        .collect();
    assert_eq!(aborted.len(), 1);
    assert_eq!(aborted[0].1["middleware"], "turn_limit");
    assert_eq!(aborted[0].1["runId"], "r1");
    assert!(events.iter().any(|(name, _)| *name == "agent://turn_done"));
    state.shutdown();
}

#[tokio::test]
async fn the_cost_cap_aborts_once_the_run_costs_too_much() {
    let (state, sink) = setup(two_steps());
    start(&state, "writer", "r1").await;
    state
        .bus()
        .db
        .call(|c| db::add_run_usage(c, "r1", "deepseek:x", 10, 10, Some(0.75)))
        .await
        .unwrap();
    let pipeline: Vec<Box<dyn TurnMiddleware>> =
        vec![Box::new(CostCapMiddleware { max_cost_usd: 0.5 })];

    drive(&state, "writer", "r1", &pipeline).await;
    let run = run_of(&state, "r1").await;
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.summary.unwrap().contains("$0.75"));
    assert!(sink.names().contains(&"agent://turn_aborted"));
    state.shutdown();
}

/// Pauses before step 0 until approved
struct PauseFirst;

impl TurnMiddleware for PauseFirst {
    fn name(&self) -> &'static str {
        "pause_first"
    }

    fn pre_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        Box::pin(async move {
            Ok(if ctx.turn_number == 0 {
                MiddlewareDecision::PauseForApproval("Start the expensive part?".into())
            } else {
                MiddlewareDecision::Continue
            })
        })
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        Box::pin(async { Ok(()) })
    }
}

async fn pause_approval(state: &AppState, run_id: &str) -> ApprovalRow {
    let (run, id) = (run_id.to_string(), pause_call_id(run_id, 0, "pause_first"));
    state
        .bus()
        .db
        .call(move |c| db::approval_for_call(c, &run, &id))
        .await
        .unwrap()
        .expect("an approval for the pause")
}

#[tokio::test]
async fn a_pause_suspends_for_approval_and_resumes_once_approved() {
    let (state, sink) = setup(two_steps());
    let ctx = start(&state, "writer", "r1").await;
    let pipeline: Vec<Box<dyn TurnMiddleware>> = vec![Box::new(PauseFirst)];

    drive(&state, "writer", "r1", &pipeline).await;
    assert_eq!(
        run_of(&state, "r1").await.status,
        RunStatus::AwaitingApproval
    );
    assert!(sink.names().contains(&"agent://approval_request"));
    let approval = pause_approval(&state, "r1").await;
    assert_eq!(approval.tool, PAUSE_TOOL);
    assert_eq!(approval.args["reason"], "Start the expensive part?");
    assert_eq!(approval.args["middleware"], "pause_first");
    assert_eq!(
        messages_of(&state, "r1")
            .await
            .iter()
            .filter(|m| m.role == role::ASSISTANT)
            .count(),
        0,
        "no step ran before the approval"
    );

    state
        .bus()
        .approval_resolved(&ctx, approval, ApprovalDecision::Approved)
        .await
        .unwrap();
    drive(&state, "writer", "r1", &pipeline).await;
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    state.shutdown();
}

/// The race the pilots hand-over test hit: the person answers in the
/// moment between the step writing its approval and the orchestrator
/// hearing that the step finished. The approval closes while the
/// executor is still busy, and the run resumes once it's free.
#[tokio::test]
async fn an_approval_answered_before_its_step_reports_back_still_resumes_the_run() {
    let (state, _sink) = setup(two_steps());
    save_agent(&state, "writer", "");
    start(&state, "writer", "r1").await;
    let pipeline: Vec<Box<dyn TurnMiddleware>> = vec![Box::new(PauseFirst)];
    drive(&state, "writer", "r1", &pipeline).await;
    let approval = pause_approval(&state, "r1").await;

    // The step that wrote the approval is still in flight
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let in_flight = InFlightSlot::default();
    let step = tokio::spawn(std::future::pending::<()>());
    *in_flight.lock() = Some(InFlight {
        run_id: "r1".into(),
        abort: step.abort_handle(),
    });
    let mut orchestrator = Orchestrator {
        agent_id: "writer".into(),
        rx,
        tx,
        bus: state.bus().clone(),
        deps: deps(&state),
        in_flight: in_flight.clone(),
        current: Some("r1".into()),
        delegation: DelegationSlot::default(),
    };
    orchestrator
        .close_approval(&approval.id, ApprovalStatus::Approved)
        .await
        .unwrap();
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Running);

    // Then the step reports back: the run carries on to the end
    orchestrator.step_finished("r1", StepOutcome::Ok).await;
    let reported = tokio::time::timeout(Duration::from_secs(10), orchestrator.rx.recv()).await;
    let Ok(Some(OrchestratorCommand::StepFinished { run_id, outcome })) = reported else {
        panic!("the resumed step never reported back");
    };
    orchestrator.step_finished(&run_id, outcome).await;
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    step.abort();
    state.shutdown();
}

#[tokio::test]
async fn a_rejected_pause_ends_the_run() {
    let (state, sink) = setup(two_steps());
    let ctx = start(&state, "writer", "r1").await;
    let pipeline: Vec<Box<dyn TurnMiddleware>> = vec![Box::new(PauseFirst)];

    drive(&state, "writer", "r1", &pipeline).await;
    let approval = pause_approval(&state, "r1").await;
    state
        .bus()
        .approval_resolved(&ctx, approval, ApprovalDecision::Rejected)
        .await
        .unwrap();
    drive(&state, "writer", "r1", &pipeline).await;
    let run = run_of(&state, "r1").await;
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.summary.unwrap().contains("not approved"));
    assert!(sink.names().contains(&"agent://turn_aborted"));
    state.shutdown();
}

#[tokio::test]
async fn an_approval_nobody_decides_expires_and_the_run_goes_on() {
    let (state, sink) = setup(two_steps());
    let ctx = start(&state, "writer", "r1").await;
    let pipeline: Vec<Box<dyn TurnMiddleware>> = vec![Box::new(PauseFirst)];

    drive(&state, "writer", "r1", &pipeline).await;
    let approval = pause_approval(&state, "r1").await;
    let deadline = approval.expires_at.expect("a deadline");
    assert!(deadline - approval.created_at >= crate::model::APPROVAL_TTL_MS - 1000);
    // Not overdue until its deadline
    let before = state
        .bus()
        .db
        .call(move |c| db::overdue_approvals(c, deadline - 1))
        .await
        .unwrap();
    assert!(before.is_empty());
    let after = state
        .bus()
        .db
        .call(move |c| db::overdue_approvals(c, deadline))
        .await
        .unwrap();
    assert_eq!(after.len(), 1);

    state
        .bus()
        .approval_closed(&ctx, approval, ApprovalStatus::Expired)
        .await
        .unwrap();
    drive(&state, "writer", "r1", &pipeline).await;
    let closed = pause_approval(&state, "r1").await;
    assert_eq!(closed.status, ApprovalStatus::Expired);
    // An expired pause isn't an approval: the run stops there
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Failed);
    assert!(sink.names().contains(&"agent://approval_resolved"));
    state.shutdown();
}

/// Asks for one more step the first time a step would end the run
#[derive(Default)]
struct ContinueOnce(AtomicBool);

impl TurnMiddleware for ContinueOnce {
    fn name(&self) -> &'static str {
        "continue_once"
    }

    fn pre_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        Box::pin(async { Ok(MiddlewareDecision::Continue) })
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        Box::pin(async move {
            if result.exit_reason == ExitReason::Completed && !self.0.swap(true, Ordering::SeqCst) {
                result.continue_with = Some("Check your work before you finish.".into());
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn continue_with_keeps_the_loop_going_once() {
    let (state, _sink) = setup(json!({
        "agent": [{ "text": "First answer." }, { "text": "Checked: all good." }]
    }));
    start(&state, "writer", "r1").await;
    let pipeline: Vec<Box<dyn TurnMiddleware>> = vec![Box::new(ContinueOnce::default())];

    drive(&state, "writer", "r1", &pipeline).await;

    let run = run_of(&state, "r1").await;
    assert_eq!(run.status, RunStatus::Completed);
    assert!(run.summary.unwrap().contains("Checked"));
    let messages = messages_of(&state, "r1").await;
    let roles: Vec<_> = messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant"]);
    assert_eq!(messages[2].content, "Check your work before you finish.");
    state.shutdown();
}

/// Plan mode end to end: agent.md `middlewares: [read_only]`, the real
/// pipeline, the canned engine
#[tokio::test]
async fn plan_mode_rejects_write_tool_calls() {
    let (state, sink) = setup(json!({
        "agent": [
            {
                "text": "Let me look, then write.",
                "toolCalls": [
                    { "tool": "read_file", "args": { "path": "a.md" }, "result": "hello" },
                    {
                        "tool": "write_file",
                        "args": { "path": "a.md", "content": "bye" },
                        "result": "ok",
                        "requiresApproval": true
                    }
                ]
            },
            { "text": "Here's the plan instead." }
        ]
    }));
    save_agent(&state, "planner", "middlewares: [read_only]\n");
    start(&state, "planner", "r1").await;

    let end = execute_run(&deps(&state), &DelegationSlot::default(), "planner", "r1")
        .await
        .unwrap();
    assert_eq!(end, RunEnd::Done);
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    assert!(
        !sink.names().contains(&"agent://approval_request"),
        "a blocked call isn't offered for approval"
    );

    let calls: Vec<ToolCallMeta> = messages_of(&state, "r1")
        .await
        .into_iter()
        .filter(|m| m.role == role::TOOL_CALL)
        .map(|m| serde_json::from_value(m.metadata).unwrap())
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].tool, "read_file");
    assert!(!calls[0].is_error);
    assert_eq!(calls[0].result.as_deref(), Some("hello"));
    assert_eq!(calls[1].tool, "write_file");
    assert!(calls[1].is_error);
    assert!(calls[1]
        .result
        .as_deref()
        .unwrap()
        .contains("plan mode (read-only)"));
    state.shutdown();
}

/// `ask_requester` runs on the clarification service whatever
/// the engine, is never held for approval, and refuses outside a
/// delegated turn
#[tokio::test]
async fn ask_requester_is_never_gated_and_needs_a_delegated_turn() {
    let (state, sink) = setup(json!({
        "agent": [
            {
                "text": "Let me ask.",
                "toolCalls": [{
                    "tool": "ask_requester",
                    "args": { "questions": [{ "question": "Which year?", "topic": "scope" }] },
                    "result": "canned",
                    "requiresApproval": true
                }]
            },
            { "text": "Done." }
        ]
    }));
    save_agent(&state, "writer", "");
    start(&state, "writer", "r1").await;
    // `a2a::clarification::start` gave the orchestrators its service
    let clarifier = state.orchestrators().clarifier().expect("set by init");
    assert!(Arc::ptr_eq(&clarifier, state.clarifications().unwrap()));
    let slots = DelegationSlot::default();
    slots.set_clarifier(&clarifier);

    let end = execute_run(&deps(&state), &slots, "writer", "r1")
        .await
        .unwrap();
    assert_eq!(end, RunEnd::Done);
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    assert!(!sink.names().contains(&"agent://approval_request"));
    let calls: Vec<ToolCallMeta> = messages_of(&state, "r1")
        .await
        .into_iter()
        .filter(|m| m.role == role::TOOL_CALL)
        .map(|m| serde_json::from_value(m.metadata).unwrap())
        .collect();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].is_error);
    assert!(calls[0]
        .result
        .as_deref()
        .unwrap()
        .contains("only works while doing a delegated task"));

    // Not a delegated turn: not offered either
    let mut tools = Vec::new();
    offer_ask_requester(&slots, "r1", "writer", &mut tools).await;
    assert!(tools.is_empty());
    state.shutdown();
}

fn tool_calls_of(messages: &[MessageRow]) -> Vec<ToolCallMeta> {
    messages
        .iter()
        .filter(|m| m.role == role::TOOL_CALL)
        .map(|m| serde_json::from_value(m.metadata.clone()).unwrap())
        .collect()
}

async fn plan_of(state: &AppState, run_id: &str) -> Option<crate::services::run_plan::RunPlan> {
    crate::services::run_plan::tool::current(state.bus(), run_id).await
}

/// Plan first, end to end: the real pipeline, the canned engine
#[tokio::test]
async fn a_change_before_a_plan_is_blocked_and_runs_once_planned() {
    let (state, sink) = setup(json!({
        "agent": [
            {
                "text": "Reading, then writing.",
                "toolCalls": [
                    { "tool": "read_file", "args": { "path": "a.md" }, "result": "hello" },
                    { "tool": "write_file", "args": { "path": "a.md", "content": "bye" }, "result": "ok" }
                ]
            },
            {
                "text": "Planning.",
                "toolCalls": [
                    { "tool": "plan_set", "args": { "steps": [{ "title": "Rewrite a.md", "kind": "fill" }] }, "result": "" },
                    { "tool": "plan_update", "args": { "step": 1, "status": "doing" }, "result": "" },
                    { "tool": "write_file", "args": { "path": "a.md", "content": "bye" }, "result": "ok" },
                    { "tool": "plan_update", "args": { "step": 1, "status": "done", "result": "Rewritten" }, "result": "" }
                ]
            },
            { "text": "Done: a.md is rewritten." }
        ]
    }));
    save_agent(
        &state,
        "writer",
        "middlewares: [plan_first, finish_check]\n",
    );
    start(&state, "writer", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);

    let calls = tool_calls_of(&messages_of(&state, "r1").await);
    let tools: Vec<_> = calls.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(
        tools,
        [
            "read_file",
            "write_file",
            "plan_set",
            "plan_update",
            "write_file",
            "plan_update"
        ]
    );
    assert!(!calls[0].is_error, "reads never need a plan");
    assert!(calls[1].is_error);
    assert!(calls[1]
        .result
        .as_deref()
        .unwrap()
        .contains("make a plan first (`plan_set`)"));
    assert!(calls[2]
        .result
        .as_deref()
        .unwrap()
        .starts_with("Run plan (revision 1)"));
    assert_eq!(calls[4].result.as_deref(), Some("ok"));
    let plan = plan_of(&state, "r1").await.unwrap();
    assert_eq!(
        plan.steps[0].status,
        crate::services::run_plan::RunStepStatus::Done
    );
    assert_eq!(plan.steps[0].result.as_deref(), Some("Rewritten"));
    assert!(sink.names().contains(&"run://plan_updated"));
    // Everything was done: the finish check had nothing to ask
    assert!(!messages_of(&state, "r1")
        .await
        .iter()
        .any(|m| m.content.starts_with(run_plan::FINISH_CHECK_LEAD)));
    state.shutdown();
}

/// Playbooks: `skill_get` reads the agent's own `skills/` on the host,
/// and needs no plan
#[tokio::test]
async fn skill_get_answers_from_the_agents_playbooks() {
    let (state, _sink) = setup(json!({
        "agent": [
            {
                "text": "Reading my playbook.",
                "toolCalls": [
                    { "tool": "skill_get", "args": { "name": "pay-a-bill" }, "result": "" },
                    { "tool": "skill_get", "args": { "name": "nope" }, "result": "" }
                ]
            },
            { "text": "Done." }
        ]
    }));
    save_agent(&state, "writer", "");
    let playbook = crate::services::domain::playbook::Playbook::new(
        "pay-a-bill",
        "Pay a bill online",
        "A bill is due",
        "1. Plan first.",
    )
    .unwrap();
    state
        .store()
        .save_agent_playbook(
            &AgentId::parse("writer").unwrap(),
            &crate::services::repo_service::PlaybookInput {
                name: playbook.name().into(),
                description: playbook.description().into(),
                when: playbook.when().into(),
                body: playbook.body().into(),
            },
            None,
        )
        .unwrap();
    start(&state, "writer", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    let calls = tool_calls_of(&messages_of(&state, "r1").await);
    assert_eq!(calls.len(), 2);
    assert!(!calls[0].is_error);
    assert_eq!(
        calls[0].result.as_deref(),
        Some(
            "# pay-a-bill
Pay a bill online

1. Plan first."
        )
    );
    assert!(calls[1].is_error);
    assert!(plan_of(&state, "r1").await.is_none());
    state.shutdown();
}

/// "Remember how I do this": a `skill_propose` call
fn propose_script(name: &str) -> Value {
    json!({
        "agent": [
            {
                "text": "I'll remember that.",
                "toolCalls": [{
                    "tool": "skill_propose",
                    "args": {
                        "name": name,
                        "description": "Pay the household bills",
                        "when": "A bill is due",
                        "body": "1. Pay from Chase, never the card.",
                        "reason": "You told me how you pay bills",
                        "current": { "body": "made up" }
                    },
                    "result": "canned"
                }]
            },
            { "text": "Saved once you approve." }
        ]
    })
}

async fn proposal_approval(state: &AppState, run_id: &str) -> ApprovalRow {
    let id = run_id.to_string();
    let pending = state
        .bus()
        .db
        .call(move |c| db::pending_approvals_for_run(c, &id))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one card");
    pending.into_iter().next().unwrap()
}

fn playbook_file(state: &AppState, agent: &str, name: &str) -> std::path::PathBuf {
    state
        .store()
        .repos()
        .agents
        .playbooks_dir(agent)
        .unwrap()
        .join(format!("{name}.md"))
}

/// A proposed playbook needs no run plan, waits for the user, and is
/// written (and committed) only once approved
#[tokio::test]
async fn a_proposed_playbook_is_written_once_approved() {
    let (state, sink) = setup(propose_script("pay-bills"));
    save_agent(&state, "writer", "");
    let ctx = start(&state, "writer", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    assert_eq!(
        run_of(&state, "r1").await.status,
        RunStatus::AwaitingApproval
    );
    assert!(sink.names().contains(&"agent://approval_request"));
    let approval = proposal_approval(&state, "r1").await;
    assert_eq!(approval.tool, "skill_propose");
    // A new name replaces nothing, whatever the model claimed
    assert!(approval.args.get("current").is_none());
    assert!(!playbook_file(&state, "writer", "pay-bills").exists());

    state
        .bus()
        .approval_closed(&ctx, approval, ApprovalStatus::Approved)
        .await
        .unwrap();
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    let calls = tool_calls_of(&messages_of(&state, "r1").await);
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].is_error, "{:?}", calls[0].result);
    let written = std::fs::read_to_string(playbook_file(&state, "writer", "pay-bills")).unwrap();
    assert!(written.contains("never the card"));
    let listed = state
        .store()
        .agent_playbooks(&AgentId::parse("writer").unwrap())
        .unwrap();
    assert_eq!(listed.playbooks.len(), 1);
    assert!(plan_of(&state, "r1").await.is_none(), "no plan needed");
    state.shutdown();
}

#[tokio::test]
async fn a_rejected_proposal_writes_nothing() {
    let (state, _) = setup(propose_script("pay-bills"));
    save_agent(&state, "writer", "");
    let ctx = start(&state, "writer", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    let approval = proposal_approval(&state, "r1").await;
    state
        .bus()
        .approval_closed(&ctx, approval, ApprovalStatus::Rejected)
        .await
        .unwrap();
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    assert!(!playbook_file(&state, "writer", "pay-bills").exists());
    state.shutdown();
}

/// Changing a playbook: the card carries the current one, from the file
#[tokio::test]
async fn a_proposal_for_an_existing_name_shows_what_it_replaces() {
    let (state, _) = setup(propose_script("pay-bills"));
    save_agent(&state, "writer", "");
    state
        .store()
        .save_agent_playbook(
            &AgentId::parse("writer").unwrap(),
            &crate::services::repo_service::PlaybookInput {
                name: "pay-bills".into(),
                description: "Pay bills".into(),
                when: "A bill is due".into(),
                body: "1. Pay by card.".into(),
            },
            None,
        )
        .unwrap();
    start(&state, "writer", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    let approval = proposal_approval(&state, "r1").await;
    assert_eq!(approval.args["current"]["body"], "1. Pay by card.");
    state.shutdown();
}

/// Checked before the user is asked: a broken proposal is the model's
/// to fix, and a check-started run never proposes
#[tokio::test]
async fn a_proposal_that_breaks_the_rules_never_reaches_the_user() {
    let (state, sink) = setup(propose_script("Pay Bills"));
    save_agent(&state, "writer", "");
    start(&state, "writer", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "writer", "r1")
        .await
        .unwrap();
    assert!(!sink.names().contains(&"agent://approval_request"));
    let calls = tool_calls_of(&messages_of(&state, "r1").await);
    assert!(calls[0].is_error);
    assert!(calls[0].result.as_deref().unwrap().contains("kebab-case"));
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    for source in [SourceType::Email, SourceType::CalendarCheck] {
        assert!(!data_run_allows(
            source,
            crate::services::playbooks::PROPOSE
        ));
    }
    state.shutdown();
}

#[tokio::test]
async fn a_risky_call_needs_a_doing_step_that_covers_it() {
    let delegate = json!({ "target": "lena", "input": { "text": "Please review" } });
    let (state, _) = setup(json!({
        "agent": [
            {
                "text": "Planning.",
                "toolCalls": [
                    { "tool": "plan_set", "args": { "steps": [
                        { "title": "Read the brief", "kind": "read" },
                        { "title": "Ask Lena", "kind": "delegate" }
                    ] }, "result": "" },
                    { "tool": "plan_update", "args": { "step": 1, "status": "doing" }, "result": "" },
                    { "tool": "delegate_to_agent", "args": delegate.clone(), "result": "sent" },
                    { "tool": "plan_update", "args": { "step": 1, "status": "done" }, "result": "" },
                    { "tool": "plan_update", "args": { "step": 2, "status": "doing" }, "result": "" },
                    { "tool": "delegate_to_agent", "args": delegate, "result": "sent" },
                    { "tool": "plan_update", "args": { "step": 2, "status": "done" }, "result": "" }
                ]
            },
            { "text": "Asked Lena." }
        ]
    }));
    save_agent(&state, "asker", "middlewares: [plan_first]\n");
    start(&state, "asker", "r1").await;
    execute_run(&deps(&state), &DelegationSlot::default(), "asker", "r1")
        .await
        .unwrap();
    let calls = tool_calls_of(&messages_of(&state, "r1").await);
    let delegations: Vec<String> = calls
        .iter()
        .filter(|c| c.tool == "delegate_to_agent")
        .map(|c| c.result.clone().unwrap_or_default())
        .collect();
    assert_eq!(delegations.len(), 2);
    assert!(
        delegations[0].contains("starts work or delegates it"),
        "{}",
        delegations[0]
    );
    assert!(delegations[0].contains("(1, Read the brief) is a `read` step"));
    assert!(delegations[0].contains("`delegate` or `approval`"));
    assert!(!delegations[1].starts_with("Blocked"), "{}", delegations[1]);
    state.shutdown();
}

#[tokio::test]
async fn the_finish_check_fires_once_per_run() {
    let (state, _) = setup(json!({
        "agent": [
            {
                "text": "Planning.",
                "toolCalls": [
                    { "tool": "plan_set", "args": { "steps": [
                        { "title": "Open the site", "kind": "navigate", "evidence": "screenshot" },
                        { "title": "Close the browser", "kind": "cleanup" }
                    ] }, "result": "" },
                    { "tool": "plan_update", "args": { "step": 1, "status": "done" }, "result": "" }
                ]
            },
            { "text": "All done." },
            { "text": "Really done." },
            { "text": "Never reached." }
        ]
    }));
    save_agent(&state, "browser-user", "");
    start(&state, "browser-user", "r1").await;
    execute_run(
        &deps(&state),
        &DelegationSlot::default(),
        "browser-user",
        "r1",
    )
    .await
    .unwrap();
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    let messages = messages_of(&state, "r1").await;
    let checks: Vec<_> = messages
        .iter()
        .filter(|m| m.role == role::USER && m.content.starts_with(run_plan::FINISH_CHECK_LEAD))
        .collect();
    assert_eq!(checks.len(), 1, "fires once");
    assert!(
        checks[0].content.contains("Step 2 is still open"),
        "{}",
        checks[0].content
    );
    assert!(checks[0].content.contains("Step 1 needs a screenshot"));
    let replies: Vec<_> = messages
        .iter()
        .filter(|m| m.role == role::ASSISTANT)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(replies, ["Planning.", "All done.", "Really done."]);
    // A run resumed later sees it fired (it's in the run's messages)
    let mut turn = turn("browser-user", "r1");
    refresh_turn(&mut turn, &run_of(&state, "r1").await, &messages, 3);
    assert!(turn.finish_check_fired);
    state.shutdown();
}

#[tokio::test]
async fn an_open_browser_gets_the_finish_check_and_is_closed_at_run_end() {
    let (state, _) = setup(json!({ "agent": [{ "text": "Found it." }, { "text": "Closed." }] }));
    save_agent(&state, "surfer", "");
    start(&state, "surfer", "r1").await;
    pilots::browser::set_pages_open("surfer", "browser-headless", true);
    execute_run(&deps(&state), &DelegationSlot::default(), "surfer", "r1")
        .await
        .unwrap();
    let messages = messages_of(&state, "r1").await;
    let check = messages
        .iter()
        .find(|m| m.content.starts_with(run_plan::FINISH_CHECK_LEAD))
        .expect("the finish check fired");
    assert!(
        check.content.contains("close it (`browser_close`)"),
        "{}",
        check.content
    );
    // Nandi closed the pages when the run ended
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    assert!(!pilots::browser::has_open_pages("surfer"));
    state.shutdown();
}

/// A video left playing: the visible browser stays open after the run, and
/// the sites it was allowed once stay allowed until that window closes
#[tokio::test]
async fn a_visible_browser_left_open_keeps_its_window_and_sites() {
    use crate::services::egress::capabilities::Capabilities;
    let (state, _) =
        setup(json!({ "agent": [{ "text": "Playing." }, { "text": "Left it open." }] }));
    save_agent(&state, "viewer", "");
    start(&state, "viewer", "r-left-open").await;
    pilots::browser::set_pages_open("viewer", "browser", true);
    pilots::sites::allow_for_run("viewer", "r-left-open", &["googlevideo.com".into()]);
    execute_run(
        &deps(&state),
        &DelegationSlot::default(),
        "viewer",
        "r-left-open",
    )
    .await
    .unwrap();
    let streams = || {
        pilots::sites::with_run_sites("viewer", Capabilities::default())
            .allows_tunnel("googlevideo.com", 443)
    };
    assert_eq!(
        run_of(&state, "r-left-open").await.status,
        RunStatus::Completed
    );
    assert!(pilots::browser::window_open("viewer"), "the window stays");
    assert!(streams(), "the video keeps streaming");
    pilots::browser::set_pages_open("viewer", "browser", false);
    assert!(!streams(), "closing the window takes the sites back");
    state.shutdown();
}

#[tokio::test]
async fn screenshots_are_named_per_step_linked_and_refused_during_a_handover() {
    let agent = "plan-shot-tester";
    let shot = |name: &str| {
        json!({
            "tool": "pilot_browser__browser_take_screenshot",
            "args": { "filename": name, "type": "png" },
            "result": "Took the screenshot"
        })
    };
    let (state, _) = setup(json!({
        "agent": [
            {
                "text": "Planning.",
                "toolCalls": [
                    { "tool": "plan_set", "args": { "steps": [
                        { "title": "Open the site", "kind": "navigate", "evidence": "screenshot" },
                        { "title": "Sign in", "kind": "handover" }
                    ] }, "result": "" },
                    { "tool": "plan_update", "args": { "step": 1, "status": "doing" }, "result": "" },
                    shot("browser/r-shot/1-1.png"),
                    { "tool": "plan_update", "args": { "step": 1, "status": "done" }, "result": "" },
                    { "tool": "plan_update", "args": { "step": 2, "status": "doing" }, "result": "" },
                    shot("browser/r-shot/2-1.png"),
                    { "tool": "plan_update", "args": { "step": 2, "status": "done" }, "result": "" }
                ]
            },
            { "text": "Signed in." }
        ]
    }));
    save_agent(&state, agent, "middlewares: [plan_first]\n");
    start(&state, agent, "r-shot").await;
    execute_run(&deps(&state), &DelegationSlot::default(), agent, "r-shot")
        .await
        .unwrap();
    let calls = tool_calls_of(&messages_of(&state, "r-shot").await);
    let shots: Vec<_> = calls
        .iter()
        .filter(|c| c.tool.ends_with("browser_take_screenshot"))
        .collect();
    assert_eq!(shots.len(), 2);
    assert_eq!(shots[0].args["filename"], "browser/r-shot/1-1.png");
    assert!(!shots[0].is_error, "{:?}", shots[0].result);
    assert!(shots[1].is_error);
    assert!(shots[1]
        .result
        .as_deref()
        .unwrap()
        .contains("no screenshots during a handover"));
    let plan = plan_of(&state, "r-shot").await.unwrap();
    assert_eq!(plan.steps[0].files, ["browser/r-shot/1-1.png"]);
    assert!(plan.steps[1].files.is_empty());
    if let Ok(work) = crate::utils::nandi_config::work_dir() {
        let _ = std::fs::remove_dir_all(work.join(agent));
    }
    state.shutdown();
}

#[tokio::test]
async fn the_default_pipeline_has_the_run_plan_middlewares() {
    let state = AppState::for_tests(vec![]);
    save_agent(&state, "helper", "");
    save_agent(&state, "opted-out", "middlewares: [verify_on_stop]\n");
    let deps = deps(&state);
    let names = |agent: &str| -> Vec<&'static str> {
        run_pipeline(&deps, agent)
            .1
            .iter()
            .map(|m| m.name())
            .collect()
    };
    let helper = names("helper");
    let at = |name| helper.iter().position(|n| *n == name).unwrap();
    assert!(at(middleware::FINISH_CHECK) < at(middleware::VERIFY_ON_STOP));
    assert!(helper.contains(&middleware::PLAN_FIRST));
    let opted_out = names("opted-out");
    assert!(!opted_out.contains(&middleware::PLAN_FIRST));
    assert!(!opted_out.contains(&middleware::FINISH_CHECK));
    state.shutdown();
}

#[tokio::test]
async fn the_default_pipeline_leaves_a_run_unchanged() {
    let (state, sink) = setup(two_steps());
    save_agent(&state, "helper", "");
    start(&state, "helper", "r1").await;

    execute_run(&deps(&state), &DelegationSlot::default(), "helper", "r1")
        .await
        .unwrap();
    assert_eq!(run_of(&state, "r1").await.status, RunStatus::Completed);
    assert!(!sink.names().contains(&"agent://turn_aborted"));
    let runtime: Option<String> = state
        .bus()
        .db
        .call(|c| c.query_row("SELECT runtime FROM runs WHERE id = 'r1'", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(runtime.as_deref(), Some("native"));
    state.shutdown();
}

#[tokio::test]
async fn studio_and_undefined_agents_get_the_builtin_limits_only() {
    let state = AppState::for_tests(vec![]);
    save_agent(
        &state,
        "planner",
        "middlewares: [read_only]\nmax_turns: 7\n",
    );
    let deps = deps(&state);

    let names = |agent: &str| -> Vec<&'static str> {
        run_pipeline(&deps, agent)
            .1
            .iter()
            .map(|m| m.name())
            .collect()
    };
    assert_eq!(names(STUDIO_AGENT_ID), middleware::BUILTIN_MIDDLEWARES);
    assert_eq!(names("nobody"), middleware::BUILTIN_MIDDLEWARES);
    let planner = names("planner");
    assert!(planner.starts_with(&middleware::BUILTIN_MIDDLEWARES));
    assert_eq!(planner.last(), Some(&middleware::READ_ONLY));
    assert!(!planner.contains(&middleware::COMPACTION));
    assert_eq!(run_pipeline(&deps, "planner").0.max_turns, 7);
    state.shutdown();
}

#[test]
fn system_extras_are_appended_in_order() {
    let system = with_system_extras(
        "Base.".into(),
        &[
            "<agent_memory>x</agent_memory>".into(),
            "  ".into(),
            "More.".into(),
        ],
    );
    assert_eq!(system, "Base.\n\n<agent_memory>x</agent_memory>\n\nMore.");
    assert_eq!(with_system_extras("Base.".into(), &[]), "Base.");
}

#[test]
fn the_turn_context_is_rebuilt_from_the_run() {
    let mut turn = turn("writer", "r1");
    turn.system_extras.push("stale".into());
    let assistant = MessageRow::new(
        "a1",
        "c",
        Some("r1"),
        role::ASSISTANT,
        "Writing.",
        AssistantMeta {
            step: 0,
            planned_calls: vec![PlannedCall {
                id: "call-1".into(),
                tool: "write_file".into(),
                args: json!({ "path": "notes.md" }),
                requires_approval: false,
            }],
            reasoning: None,
        },
    );
    let call = MessageRow::new(
        "call-1",
        "c",
        Some("r1"),
        role::TOOL_CALL,
        "",
        ToolCallMeta {
            tool: "write_file".into(),
            args: json!({ "path": "notes.md" }),
            status: ToolCallStatus::Done,
            result: Some("ok".into()),
            is_error: false,
        },
    );
    let run = RunSummary {
        id: "r1".into(),
        agent_id: "writer".into(),
        conversation_id: "c".into(),
        status: RunStatus::Running,
        input: String::new(),
        summary: None,
        started_at: db::ts(0),
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: Some(1.25),
        model: None,
        source_type: SourceType::User,
        source_id: None,
        source_label: None,
        priority: PRIORITY_USER,
        queued_at: None,
        parent_run_id: None,
        compaction_summary: None,
    };
    refresh_turn(&mut turn, &run, &[assistant, call], 1);
    assert_eq!(turn.turn_number, 1);
    assert_eq!(turn.accumulated_cost, 1.25);
    assert!(turn.token_estimate > 0);
    assert!(turn.system_extras.is_empty());
    assert_eq!(turn.files_touched, ["notes.md"]);
    assert_eq!(turn.last_tool_results.len(), 1);
    assert_eq!(turn.last_tool_results[0].turn, 0);
    assert_eq!(turn.last_tool_results[0].tool, "write_file");
}

#[test]
fn read_only_blocks_writes_but_not_reads() {
    let pipeline: Vec<Box<dyn TurnMiddleware>> = vec![Box::new(ReadOnlyMiddleware::default())];
    assert_eq!(
        middleware::blocking_middleware(&pipeline, "write_file"),
        Some(middleware::READ_ONLY)
    );
    assert_eq!(
        middleware::blocking_middleware(&pipeline, "read_file"),
        None
    );
}

fn lineage_run(id: &str, parent: Option<&str>) -> RunSummary {
    RunSummary {
        id: id.into(),
        agent_id: "writer".into(),
        conversation_id: "c".into(),
        status: RunStatus::Running,
        input: "task".into(),
        summary: None,
        started_at: db::ts(0),
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: None,
        model: None,
        source_type: SourceType::User,
        source_id: None,
        source_label: None,
        priority: PRIORITY_USER,
        queued_at: None,
        parent_run_id: parent.map(str::to_string),
        compaction_summary: parent.map(|_| "the summary".to_string()),
    }
}

fn row(id: &str, run: Option<&str>, role: &str, content: &str, at: i64, meta: Value) -> MessageRow {
    let mut row = MessageRow::new(id, "c", run, role, content, meta);
    row.created_at = at;
    row
}

#[test]
fn a_compacted_run_sees_its_summary_and_kept_turns_not_the_originals() {
    let db = crate::db::Db::in_memory();
    let mut conn = db.conn().unwrap();
    conn.execute(
        "INSERT INTO conversations (id, agent_id, created_at) VALUES ('c', 'writer', 1)",
        [],
    )
    .unwrap();
    // An earlier, unrelated turn in the conversation
    db::insert_run(&conn, &lineage_run("old", None)).unwrap();
    for m in [
        row(
            "old-u",
            Some("old"),
            role::USER,
            "earlier question",
            10,
            json!({}),
        ),
        row(
            "old-a",
            Some("old"),
            role::ASSISTANT,
            "earlier answer",
            11,
            json!({}),
        ),
    ] {
        db::insert_message(&conn, &m).unwrap();
    }
    // The parent run's originals
    db::insert_run(&conn, &lineage_run("p", None)).unwrap();
    for m in [
        row("p-u", Some("p"), role::USER, "do the task", 20, json!({})),
        row(
            "p-a1",
            Some("p"),
            role::ASSISTANT,
            "dropped step",
            21,
            json!({}),
        ),
        row(
            "p-a2",
            Some("p"),
            role::ASSISTANT,
            "kept step",
            22,
            json!({}),
        ),
    ] {
        db::insert_message(&conn, &m).unwrap();
    }
    // The child: summary + a copy of the kept step
    let child = lineage_run("child", Some("p"));
    let messages = [
        row(
            "s",
            Some("child"),
            role::ASSISTANT,
            "the summary",
            21,
            json!({ db::COMPACTION_SUMMARY_KEY: "p" }),
        ),
        row(
            "k",
            Some("child"),
            role::ASSISTANT,
            "kept step",
            22,
            json!({ db::COMPACTION_COPY_KEY: "p-a2" }),
        ),
    ];
    db::start_compacted_child(&mut conn, &child, &messages).unwrap();

    let rows = history_rows(&conn, "c", "child").unwrap();
    let ids: Vec<&str> = rows.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["old-u", "old-a", "s", "k"]);
    let summary = &rows[2];
    assert_eq!(summary.role, role::USER);
    assert!(summary.content.ends_with("the summary"));

    let history = history::build(&rows, &HashMap::new());
    assert_eq!(history.len(), 4, "{history:?}");

    // A run without a parent keeps the plain conversation window
    let plain = history_rows(&conn, "c", "old").unwrap();
    assert!(plain.iter().any(|m| m.id == "p-a1"));
}
