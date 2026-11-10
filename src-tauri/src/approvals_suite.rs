//! Approvals end to end, and the one record of decisions
//! (`.nandi/decisions/<yyyy-mm>.md`), in-process over the HTTP API in the
//! offline suite's style: no relay, no model (the canned engine), nothing
//! reaches the internet. A broken approval or a missing decision entry
//! fails CI (`pnpm run rust:test`).
//!
//! - CH-08: a tool call waits for approval; approved, the run finishes
//! - CP-06: a call through a connector waits for access; refused, the run
//!   goes on without it
//! - PO-14: each decision is in the month's file: who, what, the decision;
//!   a policy's refusal is decided by that policy, and a tool it denies is
//!   refused before anyone is asked, even one that needs approval
//! - Inbox Ask chats: a decision made in the item's chat, once the person
//!   confirms it, names the chat
//! - RQ-32: an auto-approve rule's and the groups' approvals are theirs; a
//!   group that asks first holds the task until the person decides
//! - CH-10: a crashing orchestrator restarts, up to the cap, and a run
//!   waiting for approval survives it

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::offline_suite::{boot, call, canned, get, post, until};
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::intake::{self, Gate, GroupSay, TeamTask};
use crate::services::relay::BuiltInRelay;
use crate::services::AppState;

/// This month's decisions file
fn decisions(state: &AppState) -> String {
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    state
        .store()
        .repos()
        .decisions
        .read(&month)
        .unwrap()
        .unwrap_or_default()
}

/// The entry about `kind` `id`, from its heading to the next one
fn entry(state: &AppState, kind: &str, id: &str) -> String {
    let text = decisions(state);
    let item = format!("- **Item:** {kind} `{id}`");
    let Some(at) = text.find(&item) else {
        panic!("no entry for {kind} {id}:\n{text}");
    };
    let start = text[..at].rfind("## ").unwrap_or(0);
    let end = text[at..].find("\n## ").map_or(text.len(), |end| at + end);
    text[start..end].to_string()
}

/// Waits for `kind` `id`'s entry (a decision is recorded right after it
/// takes effect)
async fn recorded(state: &AppState, kind: &str, id: &str) -> String {
    let item = format!("- **Item:** {kind} `{id}`");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !decisions(state).contains(&item) {
        assert!(
            Instant::now() < deadline,
            "never recorded: {kind} {id}\n{}",
            decisions(state)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    entry(state, kind, id)
}

fn you(state: &AppState) -> String {
    format!(
        "- **Decided by:** {}",
        crate::services::chat::decider(state)
    )
}

/// Sends a chat message and waits for the run to settle (done or paused)
async fn run(router: &axum::Router, agent: &str, text: &str) -> (String, Value) {
    let sent = post(
        router,
        &format!("agents/{agent}/messages"),
        json!({ "text": text }),
    )
    .await;
    let run_id = sent["runId"].as_str().expect("a run ID").to_string();
    let detail = until(router, &format!("runs/{run_id}"), |body| {
        body["run"]["status"] != "running" && body["run"]["status"] != "queued"
    })
    .await;
    (run_id, detail)
}

async fn finished(router: &axum::Router, run_id: &str) -> Value {
    until(router, &format!("runs/{run_id}"), |body| {
        body["run"]["status"] == "completed" || body["run"]["status"] == "failed"
    })
    .await
}

/// The pending approval of `agent`'s `tool`
async fn pending(router: &axum::Router, agent: &str, tool: &str) -> Value {
    let list = until(router, "approvals/pending", |body| {
        body.as_array().is_some_and(|list| {
            list.iter()
                .any(|a| a["agentId"] == agent && a["tool"] == tool)
        })
    })
    .await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|a| a["agentId"] == agent && a["tool"] == tool)
        .cloned()
        .unwrap()
}

/// The canned engine's default script, with no delays: `list_dir`, then
/// `shell` (which needs approval), then the answer
async fn shell_script(state: &AppState) {
    let mut script = crate::engine::default_script();
    script["tokenDelayMs"] = json!(0);
    canned(state, script).await;
}

/// An agent that calls the issue tracker's `list_issues`
async fn tracker_script(state: &AppState) {
    canned(
        state,
        json!({
            "agent": [
                {
                    "text": "Let me look at the open issues.",
                    "toolCalls": [{
                        "tool": "tracker__list_issues",
                        "args": { "project": "web" },
                        "result": "[{\"id\": 7, \"title\": \"Login is slow\"}]"
                    }]
                },
                { "text": "Done." }
            ],
            "studio": [],
            "tokenDelayMs": 0
        }),
    )
    .await;
}

/// The issue tracker connector, and agent `triager` using it
async fn triager_with_tracker(router: &axum::Router) {
    post(
        router,
        "connectors/scaffold",
        json!({
            "id": "tracker", "name": "Issue tracker", "description": "Issues.",
            "run": "uvx tracker-mcp==1.0", "hosts": ["api.tracker.example"],
            "tools": [{ "name": "list_issues", "description": "List issues", "access": "read" }],
        }),
    )
    .await;
    post(
        router,
        "agents",
        json!({ "id": "triager", "name": "Triager" }),
    )
    .await;
    let (status, body) = call(
        router,
        "PUT",
        "agents/triager/connectors",
        json!({ "connectors": [{ "id": "tracker" }] }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
}

fn write_policy(state: &AppState, id: &str, text: &str) {
    let layout = state.store().layout();
    std::fs::create_dir_all(layout.policies_dir()).unwrap();
    std::fs::write(layout.policy_path(id), text).unwrap();
}

/// CH-08, CP-06, PO-14: a tool call approved, a connector call refused, a
/// policy's refusals; each on the record
#[tokio::test]
async fn approvals_and_refusals_are_on_the_record() {
    let (state, router) = boot(BuiltInRelay::default());

    // CH-08: the shell call waits; approved, the run finishes
    shell_script(&state).await;
    post(
        &router,
        "agents",
        json!({ "id": "approver", "name": "Approver" }),
    )
    .await;
    let (run_id, detail) = run(&router, "approver", "How long is the report?").await;
    assert_eq!(detail["run"]["status"], "awaitingApproval", "{detail}");
    let approval = pending(&router, "approver", "shell").await;
    let id = approval["id"].as_str().unwrap().to_string();
    post(
        &router,
        "agents/approver/approve",
        json!({ "approvalId": id, "decision": "approved" }),
    )
    .await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    let approved = recorded(&state, "approval", &id).await;
    assert!(approved.contains(&you(&state)), "{approved}");
    assert!(
        approved.contains(&format!("- **Item:** approval `{id}` (approver: shell)")),
        "{approved}"
    );
    assert!(
        approved.contains("- **Decision:** approved\n"),
        "{approved}"
    );
    // A second decision on it is refused, and not recorded
    let (status, body) = call(
        &router,
        "POST",
        "agents/approver/approve",
        json!({ "approvalId": id, "decision": "rejected" }),
    )
    .await;
    assert_ne!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(decisions(&state).matches(&format!("`{id}`")).count(), 1);

    // CP-06: the connector call waits for access; refused, the run goes
    // on without it
    tracker_script(&state).await;
    triager_with_tracker(&router).await;
    let (run_id, detail) = run(&router, "triager", "What's open?").await;
    assert_eq!(detail["run"]["status"], "awaitingApproval", "{detail}");
    let request = pending(&router, "triager", "connector_access").await;
    let request_id = request["id"].as_str().unwrap().to_string();
    post(
        &router,
        &format!("access-requests/{request_id}"),
        json!({ "choice": "deny" }),
    )
    .await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    assert!(
        detail
            .to_string()
            .contains("the person didn't let this agent read with Issue tracker"),
        "the model got the refusal: {detail}"
    );
    let refused = recorded(&state, "approval", &request_id).await;
    assert!(refused.contains(&you(&state)), "{refused}");
    assert!(
        refused.contains("(triager: read with Issue tracker)"),
        "{refused}"
    );
    assert!(
        refused.contains("- **Decision:** refused (deny)"),
        "{refused}"
    );

    // PO-14: a policy that forbids the connector refuses it unasked, and
    // the refusal is the policy's
    write_policy(
        &state,
        "no-tracker",
        "---\napplies_to: ['agent:triager']\nconnectors: { forbid: [tracker] }\n---\nNo issue tracker.\n",
    );
    let (run_id, _) = run(&router, "triager", "Again?").await;
    let detail = finished(&router, &run_id).await;
    assert!(
        detail.to_string().contains("the policy 'no-tracker'"),
        "{detail}"
    );
    let by_policy = recorded(&state, "policy", &run_id).await;
    assert!(
        by_policy.contains("- **Decided by:** policy no-tracker"),
        "{by_policy}"
    );
    assert!(
        by_policy.contains("(triager: read with Issue tracker)"),
        "{by_policy}"
    );
    assert!(by_policy.contains("- **Decision:** refused"), "{by_policy}");

    // A policy that denies `shell` (hidden from the model): a call naming
    // it anyway is refused, and the refusal is the policy's
    canned(
        &state,
        json!({
            "agent": [
                { "text": "Counting.", "toolCalls": [{
                    "tool": "shell",
                    "args": { "cmd": "wc -w report.md" },
                    "result": "1284 report.md"
                }] },
                { "text": "Done." }
            ],
            "studio": [],
            "tokenDelayMs": 0
        }),
    )
    .await;
    write_policy(
        &state,
        "no-shell",
        "---\napplies_to: ['agent:approver']\ntools: { deny: [shell] }\n---\nNo shell.\n",
    );
    let (run_id, detail) = run(&router, "approver", "And now?").await;
    assert_eq!(
        detail["run"]["status"], "completed",
        "never asked: {detail}"
    );
    let calls = state
        .db()
        .call(move |conn| crate::db::list_run_messages(conn, &run_id))
        .await
        .unwrap();
    let shell_call = calls
        .iter()
        .filter(|m| m.role == crate::db::role::TOOL_CALL)
        .find(|m| m.metadata["tool"] == "shell")
        .map(|m| m.id.clone())
        .expect("the model tried shell");
    let denied = recorded(&state, "policy", &shell_call).await;
    assert!(
        denied.contains("- **Decided by:** policy no-shell"),
        "{denied}"
    );
    assert!(denied.contains("(approver: tool shell)"), "{denied}");
    assert!(denied.contains("- **Decision:** refused"), "{denied}");
    state.shutdown();
}

/// PO-14, CH-08: a tool a policy denies that also needs approval (shell,
/// by default), named by the model anyway, is refused by the policy
/// before anyone is asked: no approval row, one decision, the policy's
#[tokio::test]
async fn a_denied_tool_is_refused_without_asking_first() {
    let (state, router) = boot(BuiltInRelay::default());
    // As a real model's call is planned: shell is in the approval list
    canned(
        &state,
        json!({
            "agent": [
                { "text": "Counting.", "toolCalls": [{
                    "tool": "shell",
                    "args": { "cmd": "wc -w report.md" },
                    "result": "1284 report.md",
                    "requiresApproval": true
                }] },
                { "text": "Done." }
            ],
            "studio": [],
            "tokenDelayMs": 0
        }),
    )
    .await;
    post(
        &router,
        "agents",
        json!({ "id": "counter", "name": "Counter" }),
    )
    .await;
    write_policy(
        &state,
        "no-shell",
        "---
applies_to: ['agent:counter']
tools: { deny: [shell] }
---
No shell.
",
    );
    let (run_id, detail) = run(&router, "counter", "How long is the report?").await;
    assert_eq!(
        detail["run"]["status"], "completed",
        "never asked: {detail}"
    );
    assert!(
        detail
            .to_string()
            .contains("Tool 'shell' is not available: a policy doesn't allow it"),
        "the model got the refusal: {detail}"
    );
    let calls = state
        .db()
        .call(move |conn| crate::db::list_run_messages(conn, &run_id))
        .await
        .unwrap();
    let shell_call = calls
        .iter()
        .filter(|m| m.role == crate::db::role::TOOL_CALL)
        .find(|m| m.metadata["tool"] == "shell")
        .map(|m| m.id.clone())
        .expect("the model tried shell");
    let id = shell_call.clone();
    let approval = state
        .db()
        .call(move |conn| crate::db::approval_for_call(conn, &id))
        .await
        .unwrap();
    assert!(approval.is_none(), "nobody was asked: {approval:?}");
    let pending = get(&router, "approvals/pending").await;
    assert_eq!(pending, json!([]), "no approval card");
    let denied = recorded(&state, "policy", &shell_call).await;
    assert!(
        denied.contains("- **Decided by:** policy no-shell"),
        "{denied}"
    );
    assert!(denied.contains("(counter: tool shell)"), "{denied}");
    assert!(denied.contains("- **Decision:** refused"), "{denied}");
    assert_eq!(
        decisions(&state)
            .matches(&format!("`{shell_call}`"))
            .count(),
        1,
        "one decision: {}",
        decisions(&state)
    );
    state.shutdown();
}

/// An Inbox item decided in its Ask chat: the Chief of Staff proposes,
/// nothing happens until the person confirms, then the approval is
/// decided and recorded with the chat
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_decision_in_an_inbox_chat_is_recorded_with_the_chat() {
    let (state, router) = boot(BuiltInRelay::default());
    crate::services::builtin_agents::seed(state.store().layout()).unwrap();
    state.store().load().unwrap();
    shell_script(&state).await;
    post(
        &router,
        "agents",
        json!({ "id": "approver", "name": "Approver" }),
    )
    .await;
    let (run_id, _) = run(&router, "approver", "How long is the report?").await;
    let approval = pending(&router, "approver", "shell").await;
    let id = approval["id"].as_str().unwrap().to_string();

    // The Chief of Staff's turn in the item's chat proposes the approval,
    // never marked as needing a confirm: the gate makes it wait anyway
    canned(
        &state,
        json!({
            "agent": [
                { "text": "I'll approve it.", "toolCalls": [{
                    "tool": crate::services::inbox::tool::APPROVE,
                    "args": { "note": "Only this once" },
                    "result": "",
                    "requiresApproval": false
                }] },
                { "text": "Done." }
            ],
            "studio": [],
            "tokenDelayMs": 0
        }),
    )
    .await;
    let thread = format!("inbox%3Aapproval%3A{id}");
    post(&router, &format!("inbox/threads/{thread}"), Value::Null).await;
    post(
        &router,
        &format!("inbox/threads/{thread}/messages"),
        json!({ "text": "Approve it, only this once" }),
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let confirm = loop {
        let mine = state
            .db()
            .call(|conn| crate::db::pending_approvals(conn, Some(CHIEF_OF_STAFF)))
            .await
            .unwrap();
        if let Some(confirm) = mine.into_iter().next() {
            break confirm;
        }
        assert!(Instant::now() < deadline, "no confirm card");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // The sentence decided nothing, and the confirm isn't an Inbox item
    assert!(!decisions(&state).contains(&format!("`{id}`")));
    let listed = get(&router, "approvals/pending").await;
    assert!(
        !listed.to_string().contains(&confirm.id),
        "the confirm stays in its chat: {listed}"
    );

    post(
        &router,
        &format!("agents/{CHIEF_OF_STAFF}/approve"),
        json!({ "approvalId": confirm.id, "decision": "approved" }),
    )
    .await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    let decided = recorded(&state, "approval", &id).await;
    assert!(decided.contains("- **Decision:** approved"), "{decided}");
    assert!(decided.contains(&you(&state)), "{decided}");
    assert!(
        decided.contains(&format!("- **Chat:** `inbox:approval:{id}`")),
        "{decided}"
    );
    assert!(
        decided.contains("- **Reason:** Only this once"),
        "{decided}"
    );
    // The confirm itself isn't a second decision
    assert!(
        !decisions(&state).contains(&format!("`{}`", confirm.id)),
        "{}",
        decisions(&state)
    );
    state.shutdown();
}

fn team_task<'a>(task_id: &'a str, group: GroupSay<'a>) -> TeamTask<'a> {
    TeamTask {
        task_id,
        requester: "Lena Park",
        target: "Writer · review-pr",
        text: "Review PR 42 by Friday",
        instance_id: "inst-lena",
        trusted: false,
        screen_only: false,
        group,
    }
}

fn gate(state: &AppState, task: &TeamTask) -> Gate {
    intake::gate_task(
        state.db(),
        state.bus(),
        &state.store().repos().decisions,
        task,
    )
    .unwrap()
}

/// The queue item of a teammate's task
async fn item_of(state: &AppState, task_id: &str) -> String {
    let task_id = task_id.to_string();
    state
        .db()
        .call(move |conn| intake::for_task(conn, &task_id))
        .await
        .unwrap()
        .expect("queued")
        .id
}

/// RQ-32 and group policies: a rule's approval is the rule's, the groups'
/// is theirs, and a group that asks first holds the task (whatever the
/// rule says) until the person approves it
#[tokio::test]
async fn auto_approvals_are_recorded_as_such() {
    let (state, router) = boot(BuiltInRelay::default());
    let rule = intake::rules::AutoApproveRule {
        id: "r1".into(),
        matcher: intake::rules::RuleMatch::Teammate {
            instance_id: "inst-lena".into(),
            name: "Lena Park".into(),
        },
        actions: vec![crate::services::intake::IntakeAction::Run],
    };
    let (status, body) = call(
        &router,
        "PUT",
        "chief-of-staff/auto-approve",
        serde_json::to_value(vec![rule]).unwrap(),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    // The rule lets it through: decided by the rule
    assert_eq!(gate(&state, &team_task("task-1", GroupSay::None)), Gate::Go);
    let item = item_of(&state, "task-1").await;
    let by_rule = recorded(&state, "intake", &item).await;
    assert!(
        by_rule.contains("- **Decided by:** auto-approve rule (requests from Lena Park)"),
        "{by_rule}"
    );
    assert!(by_rule.contains("- **Decision:** approved"), "{by_rule}");

    // The groups' policies let it run: decided by them
    assert_eq!(gate(&state, &team_task("task-2", GroupSay::Auto)), Gate::Go);
    let item = item_of(&state, "task-2").await;
    let by_groups = recorded(&state, "intake", &item).await;
    assert!(
        by_groups.contains(&format!("- **Decided by:** {}", intake::GROUPS_DECIDED)),
        "{by_groups}"
    );

    // A group that asks first holds it, rule or not, until the person
    // approves; then it's the person's decision
    let ask = GroupSay::Ask("Finance: the owner approves every request");
    let Gate::Wait(item) = gate(&state, &team_task("task-3", ask)) else {
        panic!("a group that asks first holds the task");
    };
    assert!(!decisions(&state).contains(&format!("`{item}`")));
    post(
        &router,
        &format!("chief-of-staff/queue/{item}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(gate(&state, &team_task("task-3", ask)), Gate::Go);
    let by_you = recorded(&state, "intake", &item).await;
    assert!(by_you.contains(&you(&state)), "{by_you}");
    assert!(!by_you.contains("auto-approve"), "{by_you}");
    state.shutdown();
}

/// CH-10: a crashing orchestrator is restarted up to the cap, then left
/// down; a run waiting for approval survives every restart, and the next
/// command for the agent starts it again to finish the run
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crashing_orchestrator_restarts_up_to_the_cap() {
    let (state, router) = boot(BuiltInRelay::default());
    shell_script(&state).await;
    post(
        &router,
        "agents",
        json!({ "id": "crasher", "name": "Crasher" }),
    )
    .await;
    let (run_id, _) = run(&router, "crasher", "How long is the report?").await;
    let approval = pending(&router, "crasher", "shell").await;
    let orchestrators = state.orchestrators();

    let up = |deadline: Duration| async move {
        let until = Instant::now() + deadline;
        while !orchestrators.is_running("crasher") {
            if Instant::now() > until {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    };
    // Five crashes: restarted each time (1 s later)
    for crash in 1..=5 {
        assert!(orchestrators.crash("crasher"), "crash {crash}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(up(Duration::from_secs(5)).await, "restart {crash}");
    }
    // The sixth: left down, the agent marked as errored
    assert!(orchestrators.crash("crasher"));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!orchestrators.is_running("crasher"), "past the cap");
    let in_error = state
        .db()
        .call(|conn| crate::db::agents_in_error(conn))
        .await
        .unwrap();
    assert!(
        format!("{in_error:?}").contains("Orchestrator crashed"),
        "{in_error:?}"
    );
    // The approval waited through it all; deciding it starts the agent
    // again and the run finishes
    let still = pending(&router, "crasher", "shell").await;
    assert_eq!(still["id"], approval["id"]);
    post(
        &router,
        "agents/crasher/approve",
        json!({ "approvalId": approval["id"], "decision": "approved" }),
    )
    .await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    state.shutdown();
}
