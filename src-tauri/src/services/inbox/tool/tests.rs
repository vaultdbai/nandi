//! Tests for the decision tools of an Inbox item's chat (`inbox::tool`)

use std::time::Duration;

use serde_json::json;

use super::*;
use crate::db;
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::gateway::assignments::{self, AssignmentStatus, GatewayAssignment};
use crate::services::gateway::holds::{self, GatewayHold, HoldStatus};
use crate::services::inbox::thread;

fn hold(task_id: &str) -> GatewayHold {
    GatewayHold {
        task_id: task_id.into(),
        caller_instance: "desktop-sam".into(),
        caller_name: "Sam Lee".into(),
        capability: Some("ask-me".into()),
        summary: "Can you send the Q3 numbers?".into(),
        status: HoldStatus::Pending,
        created_at: 1.0,
        expires_at: (db::now_ms() + 600_000) as f64,
    }
}

async fn held(state: &AppState, task_id: &str) {
    let hold = hold(task_id);
    state
        .db()
        .call(move |conn| holds::open(conn, &hold))
        .await
        .unwrap();
}

async fn hold_status(state: &AppState, task_id: &str) -> HoldStatus {
    let id = task_id.to_string();
    state
        .db()
        .call(move |conn| holds::get(conn, &id))
        .await
        .unwrap()
        .unwrap()
        .status
}

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

fn names(specs: &[ToolSpec]) -> Vec<&str> {
    specs.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn every_decision_call_waits_for_the_user() {
    let call = |tool: &str| PlannedCall {
        id: tool.into(),
        tool: tool.into(),
        args: json!({}),
        requires_approval: false,
    };
    let mut calls = vec![
        call(APPROVE),
        call(REFUSE),
        call(ANSWER),
        call(DONE),
        call(REASSIGN),
        call("work_open"),
    ];
    gate(&mut calls);
    let waiting: Vec<bool> = calls.iter().map(|c| c.requires_approval).collect();
    assert_eq!(waiting, [true, true, true, true, true, false]);
}

#[test]
fn reassigning_finds_the_teammate_by_name_or_id() {
    use crate::services::a2a::reassign::ReassignCandidate;
    let candidate = |id: &str, name: &str| ReassignCandidate {
        instance_id: id.into(),
        display_name: name.into(),
        card_id: format!("{id}/gateway"),
    };
    let candidates = [
        candidate("desktop-bob", "Bob Park"),
        candidate("desktop-bea", "Bea Lin"),
    ];
    assert_eq!(
        pick(&candidates, "bob park").unwrap().instance_id,
        "desktop-bob"
    );
    assert_eq!(pick(&candidates, "Bob").unwrap().instance_id, "desktop-bob");
    assert_eq!(
        pick(&candidates, "desktop-bea").unwrap().instance_id,
        "desktop-bea"
    );
    let unclear = pick(&candidates, "B").unwrap_err();
    assert!(unclear.contains("Bob Park, Bea Lin"), "{unclear}");
    assert!(pick(&[], "Bob").unwrap_err().contains("Nobody online"));
}

#[tokio::test]
async fn decisions_are_offered_only_in_a_waiting_items_chat() {
    let state = AppState::for_tests(Vec::new());
    assert!(offered(&state, "c1").await.is_empty());
    assert!(offered(&state, "inbox:gatewayHold:missing")
        .await
        .is_empty());
    held(&state, "t1").await;
    let specs = offered(&state, "inbox:gatewayHold:t1").await;
    assert_eq!(names(&specs), [APPROVE, REFUSE, REASSIGN]);
    // Only an intake thread picks one of several queued items
    assert!(specs[0].input_schema["properties"].get("item").is_none());
    assert!(spec(DecisionTool::Approve, true).input_schema["properties"]
        .get("item")
        .is_some());

    crate::services::gateway::decide_gateway_hold(&state, "t1".into(), true, None)
        .await
        .unwrap();
    assert!(offered(&state, "inbox:gatewayHold:t1").await.is_empty());
}

#[tokio::test]
async fn a_decision_that_does_not_fit_the_item_changes_nothing() {
    let state = AppState::for_tests(Vec::new());
    held(&state, "t1").await;
    let thread = "inbox:gatewayHold:t1";
    let outcome = call(&state, thread, ANSWER, &json!({"text": "yes"})).await;
    assert!(outcome.is_error);
    assert!(outcome
        .result
        .contains("inbox_approve, inbox_refuse, inbox_reassign"));
    let outside = call(&state, "c1", APPROVE, &json!({})).await;
    assert!(outside.is_error);
    assert_eq!(hold_status(&state, "t1").await, HoldStatus::Pending);
    assert!(!decisions(&state).contains("gatewayHold"));
}

#[tokio::test]
async fn done_on_an_assignment_sends_the_note_back_and_is_recorded() {
    let state = AppState::for_tests(Vec::new());
    let assignment = GatewayAssignment {
        task_id: "t2".into(),
        caller_instance: "desktop-sam".into(),
        caller_name: "Sam Lee".into(),
        request: "Upload the SOC 1 evidence".into(),
        status: AssignmentStatus::Open,
        note: None,
        created_at: 1.0,
        due_at: None,
        closed_at: None,
        links: Vec::new(),
    };
    state
        .db()
        .call(move |conn| assignments::open(conn, &assignment))
        .await
        .unwrap();
    let thread = "inbox:assignment:t2";
    assert_eq!(
        names(&offered(&state, thread).await),
        [DONE, REFUSE, REASSIGN]
    );
    let outcome = call(
        &state,
        thread,
        DONE,
        &json!({"note": "Uploaded to the audit folder"}),
    )
    .await;
    assert!(!outcome.is_error, "{}", outcome.result);
    let closed = state
        .db()
        .call(|conn| assignments::get(conn, "t2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed.status, AssignmentStatus::Done);
    assert_eq!(closed.note.as_deref(), Some("Uploaded to the audit folder"));
    let text = decisions(&state);
    assert!(text.contains("- **Item:** assignment `t2`"), "{text}");
    assert!(text.contains("- **Decision:** done"), "{text}");
    assert!(
        text.contains("- **Reason:** Uploaded to the audit folder"),
        "{text}"
    );
    assert!(text.contains("- **Chat:** `inbox:assignment:t2`"), "{text}");
}

#[tokio::test]
async fn the_button_and_the_chat_record_the_same_decision() {
    let state = AppState::for_tests(Vec::new());
    held(&state, "t1").await;
    held(&state, "t2").await;
    // The button
    crate::services::gateway::decide_gateway_hold(
        &state,
        "t1".into(),
        false,
        Some("Not this quarter".into()),
    )
    .await
    .unwrap();
    // The chat
    let outcome = call(
        &state,
        "inbox:gatewayHold:t2",
        APPROVE,
        &json!({"note": "Only the summary, not the raw numbers"}),
    )
    .await;
    assert!(!outcome.is_error, "{}", outcome.result);
    assert_eq!(hold_status(&state, "t2").await, HoldStatus::Approved);

    let text = decisions(&state);
    let button = &text[text.find("`t1`").unwrap()..text.find("`t2`").unwrap()];
    assert!(button.contains("- **Decision:** declined"), "{text}");
    assert!(button.contains("- **Reason:** Not this quarter"), "{text}");
    assert!(!button.contains("**Chat:**"), "{text}");
    let chat = &text[text.find("`t2`").unwrap()..];
    assert!(chat.contains("- **Decision:** approved"), "{text}");
    assert!(chat.contains("- **Reason:** Only the summary"), "{text}");
    assert!(
        chat.contains("- **Chat:** `inbox:gatewayHold:t2`"),
        "{text}"
    );
    assert!(
        text.contains("(Sam Lee, ask-me: Can you send the Q3 numbers?)"),
        "{text}"
    );
}

#[tokio::test]
async fn a_returned_request_is_dismissed_from_its_chat() {
    let state = AppState::for_tests(Vec::new());
    state
        .db()
        .call(|conn| {
            conn.execute(
                "INSERT INTO request_returns (task_id, from_name, provider_agent, capability, \
                 outcome, summary, request, returned_at) \
                 VALUES ('t3', 'Bea Lin', 'desktop-bea/gateway', 'ask-me', 'answered', \
                 'Friday', 'When is the offsite?', ?1)",
                [db::now_ms() as f64],
            )
        })
        .await
        .unwrap();
    let thread = "inbox:requestReturn:t3";
    assert_eq!(names(&offered(&state, thread).await), [DONE]);
    let outcome = call(&state, thread, DONE, &json!({})).await;
    assert!(!outcome.is_error, "{}", outcome.result);
    assert!(crate::services::a2a::returns::list(state.db())
        .await
        .unwrap()
        .is_empty());
    assert!(decisions(&state).contains("- **Item:** requestReturn `t3`"));
}

async fn wait_for<F, Fut>(what: &str, mut check: F)
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

/// The Chief of Staff's pending approvals (the Inbox list leaves its chat
/// decisions out)
async fn chief_approvals(state: &AppState) -> Vec<crate::model::PendingApproval> {
    state
        .db()
        .call(|conn| db::pending_approvals(conn, Some(CHIEF_OF_STAFF)))
        .await
        .unwrap()
}

/// A Chief of Staff whose next turn proposes `tool` with `args`, never
/// marked as needing approval: the gate must make it wait anyway
async fn chief_proposes(state: &AppState, tool: &str, args: serde_json::Value) {
    crate::services::builtin_agents::seed(state.store().layout()).unwrap();
    state.store().load().unwrap();
    let script = json!({
        "agent": [
            { "text": "I'll propose it.", "toolCalls": [{
                "tool": tool, "args": args, "result": "", "requiresApproval": false
            }] },
            { "text": "Done." }
        ],
        "studio": [],
        "tokenDelayMs": 0
    });
    state
        .db()
        .call(move |conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
        })
        .await
        .unwrap();
}

/// The whole path: a group-forced hold (it is held like any other) is
/// decided in its chat only once the user confirms the proposal on its
/// card; then the same service call as the button declines it
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chat_decision_waits_for_the_users_confirm_then_calls_the_service() {
    let state = AppState::for_tests(Vec::new());
    held(&state, "t1").await;
    chief_proposes(&state, REFUSE, json!({"reason": "Ask Bea instead"})).await;
    let id = "inbox:gatewayHold:t1".to_string();
    thread::open(&state, id.clone()).await.unwrap();
    thread::send(&state, id.clone(), "Decline it, Bea owns this".into(), None)
        .await
        .unwrap();

    wait_for("the confirm card", || async {
        !chief_approvals(&state).await.is_empty()
    })
    .await;
    let approval = chief_approvals(&state).await.remove(0);
    assert_eq!(approval.tool, REFUSE);
    // Nothing is decided by the chat sentence, and the confirm isn't an
    // Inbox item of its own
    assert_eq!(hold_status(&state, "t1").await, HoldStatus::Pending);
    assert!(crate::services::chat::list_pending_approvals(&state)
        .await
        .unwrap()
        .is_empty());

    crate::services::chat::approve_action(
        &state,
        Some(CHIEF_OF_STAFF),
        approval.id,
        ApprovalDecision::Approved,
    )
    .await
    .unwrap();
    wait_for("the hold to be declined", || async {
        hold_status(&state, "t1").await == HoldStatus::Rejected
    })
    .await;
    // Recorded right after the hold is decided
    wait_for("the decision to be recorded", || async {
        decisions(&state).contains("gatewayHold")
    })
    .await;
    let text = decisions(&state);
    assert!(text.contains("- **Reason:** Ask Bea instead"), "{text}");
    assert!(
        text.contains("- **Chat:** `inbox:gatewayHold:t1`"),
        "{text}"
    );
    // The confirm itself isn't a second decision
    assert_eq!(text.matches("## ").count(), 1, "{text}");
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_confirm_decides_nothing() {
    let state = AppState::for_tests(Vec::new());
    held(&state, "t1").await;
    chief_proposes(&state, APPROVE, json!({})).await;
    let id = "inbox:gatewayHold:t1".to_string();
    thread::open(&state, id.clone()).await.unwrap();
    thread::send(&state, id, "Approve it".into(), None)
        .await
        .unwrap();
    wait_for("the confirm card", || async {
        !chief_approvals(&state).await.is_empty()
    })
    .await;
    let approval = chief_approvals(&state).await.remove(0);
    crate::services::chat::approve_action(&state, None, approval.id, ApprovalDecision::Rejected)
        .await
        .unwrap();
    wait_for("the turn to end", || async {
        chief_approvals(&state).await.is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(hold_status(&state, "t1").await, HoldStatus::Pending);
    assert!(!decisions(&state).contains("gatewayHold"));
    state.shutdown();
}
