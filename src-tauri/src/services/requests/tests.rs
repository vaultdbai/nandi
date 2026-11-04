use std::collections::HashMap;

use serde_json::json;

use super::*;
use crate::model::SourceType;

const ME: &str = "desktop-me";

fn task(id: &str, requester: &str, provider: &str, state: TaskState) -> TaskRecord {
    TaskRecord {
        id: id.into(),
        requester_instance: requester.into(),
        requester_agent: format!("{requester}/writer"),
        provider_instance: provider.into(),
        provider_agent: "papers".into(),
        capability: "finance.risk".into(),
        input: json!({"text": "Summarize Q3 portfolio risk\nwith details"}),
        state,
        eta_seconds: None,
        deadline: None,
        priority: 0,
        created_at: 1_000.0,
        updated_at: 2_000.0,
        acknowledged_at: None,
        completed_at: None,
        error: None,
        result: None,
        parent_task_id: None,
        source_kind: task::TaskSourceKind::Direct,
        attempt_number: 1,
        previous_attempt_id: None,
    }
}

/// The gateway's acknowledgement, with its routing reason
fn routed(task_id: &str, reason: &str, at: f64) -> TaskEvent {
    TaskEvent {
        stage: Some(TaskStage::Route),
        ..event(task_id, TaskState::Acknowledged, Some(reason), at)
    }
}

fn event(task_id: &str, to: TaskState, reason: Option<&str>, at: f64) -> TaskEvent {
    TaskEvent {
        task_id: task_id.into(),
        from_state: None,
        to_state: to,
        reason: reason.map(str::to_string),
        reason_kind: None,
        stage: None,
        at,
    }
}

fn message(kind: &str, payload: Value) -> RawMessage {
    RawMessage {
        kind: kind.into(),
        payload,
        at: 1_500.0,
    }
}

fn bundle(task: TaskRecord, events: Vec<TaskEvent>, messages: Vec<RawMessage>) -> TaskBundle {
    TaskBundle {
        task,
        events,
        messages,
    }
}

fn run(id: &str, status: RunStatus, summary: Option<&str>) -> RunSummary {
    RunSummary {
        id: id.into(),
        agent_id: "papers".into(),
        conversation_id: "agent:papers".into(),
        status,
        input: "x".into(),
        summary: summary.map(str::to_string),
        started_at: 1_200.0,
        ended_at: None,
        input_tokens: 100,
        output_tokens: 20,
        cost_usd: Some(0.01),
        model: None,
        source_type: SourceType::Agent,
        source_id: None,
        source_label: None,
        priority: 60,
        queued_at: None,
        parent_run_id: None,
        compaction_summary: None,
    }
}

fn members() -> HashMap<String, String> {
    [
        ("priya-laptop", "Priya's Nandi"),
        ("sam-desktop", "Sam's Nandi"),
        ("ann-pc", "Ann's Nandi"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

fn input(attempts: Vec<TaskBundle>) -> ProjectionInput {
    ProjectionInput {
        me: ME.into(),
        attempts,
        members: members(),
        agents: [("papers".to_string(), "Portfolio Analyst".to_string())].into(),
        ..Default::default()
    }
}

fn stage(request: &Request, stage: SystemStage) -> &RequestStep {
    request
        .steps
        .iter()
        .find(|s| s.stage == Some(stage))
        .unwrap_or_else(|| panic!("no {stage:?} step in {:?}", request.steps))
}

#[test]
fn an_outgoing_failover_shows_every_attempt_and_why_it_moved_on() {
    let mut first = task("t1", ME, "sam-desktop", TaskState::TimedOut);
    first.error = Some("'Papers' didn't finish in time".into());
    let mut second = task("t2", ME, "ann-pc", TaskState::Rejected);
    second.attempt_number = 2;
    second.previous_attempt_id = Some("t1".into());
    second.error = Some("Provider requires trust. (…)".into());
    let mut third = task("t3", ME, "priya-laptop", TaskState::Working);
    third.attempt_number = 3;
    third.previous_attempt_id = Some("t2".into());
    third.acknowledged_at = Some(1_800.0);
    third.eta_seconds = Some(240);
    let request = project(&input(vec![
        bundle(first, vec![], vec![]),
        bundle(second, vec![], vec![]),
        bundle(
            third,
            vec![event(
                "t3",
                TaskState::Delivered,
                Some("routed by the relay"),
                1_700.0,
            )],
            vec![message(PROGRESS_KIND, json!({"note": "Scoring positions"}))],
        ),
    ]))
    .unwrap();

    assert_eq!(request.id, "t1");
    assert_eq!(request.direction, Direction::Outgoing);
    assert_eq!(request.title, "Summarize Q3 portfolio risk");
    assert_eq!(request.counterpart.as_deref(), Some("Priya's Nandi"));
    assert_eq!(request.status, RequestStatus::Running);
    assert!(request.stuck.is_none());
    let peer = request
        .steps
        .iter()
        .find(|s| s.kind == StepKind::Peer)
        .unwrap();
    assert_eq!(peer.label, "Priya's Nandi");
    assert_eq!(peer.progress.as_deref(), Some("Scoring positions"));
    let providers: Vec<&str> = peer.attempts.iter().map(|a| a.provider.as_str()).collect();
    assert_eq!(providers, ["Sam's Nandi", "Ann's Nandi", "Priya's Nandi"]);
    assert_eq!(
        peer.attempts[0].error.as_deref(),
        Some("'Papers' didn't finish in time")
    );
    assert_eq!(
        stage(&request, SystemStage::DeliveredTo).status,
        StepStatus::Done
    );
    assert_eq!(
        stage(&request, SystemStage::Accepted).eta_seconds,
        Some(240)
    );
    assert_eq!((request.plan_steps, request.current_step), (1, 1));
    assert_eq!(request.task_ids, ["t1", "t2", "t3"]);
}

#[test]
fn an_incoming_gateway_task_routed_to_a_workflow_shows_its_steps() {
    let mut root = task("g1", "priya-laptop", ME, TaskState::Completed);
    root.acknowledged_at = Some(1_100.0);
    root.completed_at = Some(1_900.0);
    root.result = Some(json!("Q3 risk is concentrated in two positions"));
    let request = project(&input(vec![bundle(
        root,
        vec![
            event("g1", TaskState::Submitted, None, 1_000.0),
            routed(
                "g1",
                "Running the workflow Risk Review: matches finance.risk",
                1_100.0,
            ),
            event(
                "g1",
                TaskState::Working,
                Some("routed; working on it"),
                1_101.0,
            ),
            event("g1", TaskState::Completed, None, 1_900.0),
        ],
        vec![message(
            WORKFLOW_STEPS_KIND,
            json!({"workflowId": "risk-review", "runId": "wr1", "status": "completed", "steps": [
                {"stepId": "fetch", "agentId": "papers", "status": "done"},
                {"stepId": "score-positions", "agentId": "papers", "status": "done"}
            ]}),
        )],
    )]))
    .unwrap();

    assert_eq!(request.direction, Direction::Incoming);
    assert_eq!(request.status, RequestStatus::Done);
    let stages: Vec<Option<SystemStage>> = request.steps.iter().map(|s| s.stage).collect();
    assert_eq!(
        stages,
        [
            Some(SystemStage::Received),
            Some(SystemStage::Allowed),
            Some(SystemStage::Routed),
            None,
            Some(SystemStage::Delivered)
        ]
    );
    let routed = stage(&request, SystemStage::Routed);
    assert!(routed.detail.as_deref().unwrap().contains("Risk Review"));
    let workflow = &request.steps[3];
    assert_eq!(
        (workflow.kind, workflow.label.as_str()),
        (StepKind::Workflow, "Risk Review")
    );
    let children: Vec<&str> = workflow.children.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(children, ["Fetch", "Score positions"]);
    assert_eq!(
        workflow.children[0].link.workflow_run_id.as_deref(),
        Some("wr1")
    );
    assert_eq!(
        stage(&request, SystemStage::Delivered).status,
        StepStatus::Done
    );
}

#[test]
fn an_incoming_task_waiting_on_my_approval_points_at_the_run() {
    let mut root = task("g2", "priya-laptop", ME, TaskState::Working);
    root.acknowledged_at = Some(1_100.0);
    let mut child = task("g2-gw", ME, ME, TaskState::Working);
    child.requester_agent = "gateway".into();
    let mut input = input(vec![bundle(
        root,
        vec![routed(
            "g2",
            "Routed to Portfolio Analyst: serves it",
            1_100.0,
        )],
        vec![],
    )]);
    input.child = Some(bundle(
        child,
        vec![],
        vec![message(
            RUN_KIND,
            json!({"agentId": "papers", "runId": "r1"}),
        )],
    ));
    input
        .runs
        .insert("r1".into(), run("r1", RunStatus::AwaitingApproval, None));
    input.approvals.insert("r1".into(), "ap1".into());

    let request = project(&input).unwrap();
    assert_eq!(request.status, RequestStatus::Waiting);
    let stuck = request.stuck.clone().unwrap();
    assert_eq!(
        (stuck.kind, stuck.who, stuck.terminal),
        (BlockerKind::NeedsMyApproval, BlockerWho::Me, false)
    );
    assert_eq!(stuck.step_id.as_deref(), Some("step-1"));
    assert_eq!(stuck.approval_id.as_deref(), Some("ap1"));
    assert_eq!(stuck.agent_name.as_deref(), Some("Portfolio Analyst"));
    let step = request.steps.iter().find(|s| s.id == "step-1").unwrap();
    assert_eq!(
        (step.kind, step.status),
        (StepKind::Agent, StepStatus::Waiting)
    );
    assert_eq!(step.label, "Portfolio Analyst");
    assert_eq!(step.link.run_id.as_deref(), Some("r1"));
    assert_eq!((request.input_tokens, request.cost_usd), (100, Some(0.01)));
}

#[test]
fn a_trust_rejection_names_the_member_on_both_sides() {
    let reason = "Provider requires trust. (The owner of 'Papers' needs to add 'priya' to their trusted peers.)";
    let mut incoming = task("g3", "priya-laptop", ME, TaskState::Rejected);
    incoming.error = Some(reason.into());
    let request = project(&input(vec![bundle(incoming, vec![], vec![])])).unwrap();
    let stuck = request.stuck.clone().unwrap();
    assert_eq!(
        (stuck.kind, stuck.who),
        (BlockerKind::NotTrusted, BlockerWho::Me)
    );
    assert_eq!(stuck.member.as_deref(), Some("Priya's Nandi"));
    assert_eq!(stuck.member_instance.as_deref(), Some("priya-laptop"));
    assert_eq!(stuck.step_id.as_deref(), Some("allowed"));
    // Rejected at the door: only Received ✓ and Allowed ✕
    assert_eq!(request.steps.len(), 2);
    assert_eq!(
        stage(&request, SystemStage::Allowed).status,
        StepStatus::Failed
    );
    assert_eq!(request.status, RequestStatus::Failed);

    let mut outgoing = task("t4", ME, "priya-laptop", TaskState::Rejected);
    outgoing.error = Some(reason.into());
    let request = project(&input(vec![bundle(outgoing, vec![], vec![])])).unwrap();
    let stuck = request.stuck.unwrap();
    assert_eq!(
        (stuck.kind, stuck.who),
        (BlockerKind::NotTrusted, BlockerWho::Them)
    );
    assert_eq!(stuck.step_id.as_deref(), Some("delivered-to"));
    assert_eq!(stuck.member.as_deref(), Some("Priya's Nandi"));
}

#[test]
fn an_undelivered_task_to_an_offline_provider_waits() {
    let mut input = input(vec![bundle(
        task("t5", ME, "priya-laptop", TaskState::Submitted),
        vec![],
        vec![],
    )]);
    input.provider_online = Some(false);
    let request = project(&input).unwrap();
    assert_eq!(request.status, RequestStatus::Waiting);
    let stuck = request.stuck.clone().unwrap();
    assert_eq!(
        (stuck.kind, stuck.who),
        (BlockerKind::Offline, BlockerWho::Nobody)
    );
    assert_eq!(
        stage(&request, SystemStage::DeliveredTo).status,
        StepStatus::Waiting
    );
}

#[test]
fn unknown_members_fall_back_to_their_device_name() {
    let request = project(&input(vec![bundle(
        task("t6", ME, "zed-box", TaskState::Working),
        vec![],
        vec![],
    )]))
    .unwrap();
    assert_eq!(request.counterpart.as_deref(), Some("zed-box"));
}

#[test]
fn titles_are_the_first_line_or_the_capability_in_words() {
    assert_eq!(
        title_of(&json!("  \nFind papers\nmore"), "x"),
        "Find papers"
    );
    assert_eq!(
        title_of(&json!({"request": "Translate this"}), "x"),
        "Translate this"
    );
    assert_eq!(
        title_of(&json!({}), "code-review.python"),
        "Code review python"
    );
    let long = "word ".repeat(40);
    let title = title_of(&json!(long), "x");
    assert!(
        title.chars().count() <= MAX_TITLE_CHARS && title.ends_with('…'),
        "{title}"
    );
}

#[tokio::test]
async fn any_task_id_resolves_to_its_request() {
    let db = Db::in_memory();
    let root = task("root", "priya-laptop", ME, TaskState::Working);
    let child = task("root-gw", ME, ME, TaskState::Working);
    let mut retry = task("root-gw-2", ME, ME, TaskState::Working);
    retry.previous_attempt_id = Some("root-gw".into());
    retry.attempt_number = 2;
    db.call(move |conn| {
        task::insert(conn, &root)?;
        task::insert(conn, &child)?;
        task::insert(conn, &retry)?;
        task::insert_message(
            conn,
            "root-gw",
            RUN_KIND,
            &json!({"agentId": "papers", "runId": "r9"}),
            1,
        )?;
        Ok(())
    })
    .await
    .unwrap();
    for id in ["root", "root-gw"] {
        let found = db.call(move |conn| root_id(conn, id)).await.unwrap();
        assert_eq!(found.as_deref(), Some("root"), "{id}");
    }
    let missing = db.call(|conn| root_id(conn, "nope")).await.unwrap();
    assert_eq!(missing, None);
}
