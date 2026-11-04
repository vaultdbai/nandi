use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::services::events::tests::RecordingSink;

const HOUR: i64 = 60 * 60 * 1000;

/// A `workflow_runs` row with steps in the given statuses
async fn insert_run(state: &AppState, id: &str, status: &str, started_ago_ms: i64, steps: &[&str]) {
    let now = crate::db::now_ms();
    let steps: Vec<serde_json::Value> = steps
        .iter()
        .enumerate()
        .map(|(i, status)| {
            json!({
                "stepId": format!("s{}", i + 1),
                "agentId": "writer",
                "template": "do it",
                "requiresApproval": *status == "awaitingApproval",
                "status": status,
                "input": null,
                "output": null,
                "agentRunId": null,
                "error": null,
                "startedAt": null,
                "endedAt": null,
            })
        })
        .collect();
    let context = json!({ "steps": steps, "error": null }).to_string();
    let (id, status) = (id.to_string(), status.to_string());
    let ended =
        matches!(status.as_str(), "completed" | "failed").then_some(now - started_ago_ms + 1000);
    state
        .db()
        .call(move |conn| {
            conn.execute(
                "INSERT INTO workflow_runs (id, workflow_id, status, started_at, ended_at, context)
                 VALUES (?1, 'onboarding', ?2, ?3, ?4, ?5)",
                rusqlite::params![id, status, now - started_ago_ms, ended, context],
            )
            .map(|_| ())
        })
        .await
        .unwrap();
}

async fn state() -> (AppState, Arc<RecordingSink>) {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    insert_run(
        &state,
        "active",
        "running",
        HOUR,
        &["done", "running", "pending"],
    )
    .await;
    insert_run(
        &state,
        "waiting",
        "awaiting_approval",
        HOUR,
        &["done", "awaitingApproval"],
    )
    .await;
    insert_run(&state, "done", "completed", 2 * HOUR, &["done", "done"]).await;
    insert_run(&state, "old", "completed", 48 * HOUR, &["done"]).await;
    insert_run(&state, "broken", "failed", HOUR, &["failed"]).await;
    (state, recorder)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_lists_active_waiting_and_recent_done() {
    let (state, _) = state().await;
    let items = list_work(&state, None).await.unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert!(!ids.contains(&"old"), "done more than a day ago: {ids:?}");
    for id in ["active", "waiting", "done", "broken"] {
        assert!(ids.contains(&id), "{id} missing: {ids:?}");
    }

    let active = items.iter().find(|i| i.id == "active").unwrap();
    assert_eq!(active.kind, WorkKind::Workflow);
    assert_eq!(active.state, WorkState::Active);
    assert_eq!(active.title, "onboarding");
    assert_eq!(active.workflow_id.as_deref(), Some("onboarding"));
    assert_eq!(
        active.progress,
        WorkProgress {
            completed_steps: 1,
            total_steps: 3,
            percent: 33
        }
    );
    let next = active.next_action.as_ref().unwrap();
    assert_eq!(next.kind, NextActionKind::Auto);
    assert!(next.description.contains("writer is working on 's2'"));

    let waiting = items.iter().find(|i| i.id == "waiting").unwrap();
    assert_eq!(waiting.state, WorkState::Waiting);
    let next = waiting.next_action.as_ref().unwrap();
    assert_eq!(next.kind, NextActionKind::Approval);
    assert!(next.description.contains("Approve step 's2'"));
    // Typed (task 5): the UI writes the copy and approves this step inline
    assert_eq!(
        next.reason_kind,
        Some(crate::services::a2a::task::ReasonKind::NeedsMyApproval)
    );
    assert_eq!(next.target.run_id.as_deref(), Some("waiting"));
    assert_eq!(next.target.step_id.as_deref(), Some("s2"));

    assert_eq!(
        items.iter().find(|i| i.id == "done").unwrap().state,
        WorkState::Done
    );
    assert_eq!(
        items.iter().find(|i| i.id == "broken").unwrap().state,
        WorkState::Failed
    );

    let only_waiting = list_work(&state, Some(WorkState::Waiting)).await.unwrap();
    assert_eq!(only_waiting.len(), 1);
    assert_eq!(
        get_work_item(&state, "done".into()).await.unwrap().state,
        WorkState::Done
    );
    assert!(get_work_item(&state, "nope".into()).await.is_err());
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_pauses_resumes_and_cancels_workflow_runs() {
    let (state, recorder) = state().await;

    pause_work(&state, "active".into()).await.unwrap();
    let paused = get_work_item(&state, "active".into()).await.unwrap();
    assert_eq!(paused.state, WorkState::Waiting);
    assert_eq!(paused.next_action.unwrap().kind, NextActionKind::ManualStep);
    assert!(recorder.names().contains(&WORK_ITEM_UPDATED));
    // Twice: refused
    assert!(pause_work(&state, "active".into())
        .await
        .unwrap_err()
        .contains("can't be paused"));

    resume_work(&state, "active".into()).await.unwrap();
    let run = state.workflows().run("active").await.unwrap().unwrap();
    assert_ne!(run.status, WorkflowRunStatus::Paused);

    cancel_work(&state, "waiting".into()).await.unwrap();
    let cancelled = get_work_item(&state, "waiting".into()).await.unwrap();
    assert_eq!(cancelled.state, WorkState::Cancelled);
    assert!(cancelled.next_action.is_none());
    assert!(cancel_work(&state, "waiting".into())
        .await
        .unwrap_err()
        .contains("can't be cancelled"));
    assert!(cancel_work(&state, "done".into()).await.is_err());
    assert!(pause_work(&state, "nope".into()).await.is_err());
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_delegations_can_be_rated() {
    let (state, _) = state().await;
    assert!(rate_work(&state, "done".into(), 4)
        .await
        .unwrap_err()
        .contains("Only finished delegated tasks"));
    state
        .db()
        .call(|conn| {
            conn.execute(
                "INSERT INTO capability_evidence (id, capability_id, task_id, role, status, observed_at)
                 VALUES ('e1', 'review', 't1', 'requester', 'completed', 1)",
                [],
            )
            .map(|_| ())
        })
        .await
        .unwrap();
    assert!(rate_work(&state, "t1".into(), 9).await.is_err());
    rate_work(&state, "t1".into(), 4).await.unwrap();
    assert_eq!(ratings(&state).await.get("t1"), Some(&4));
    state.shutdown();
}

#[tokio::test]
async fn dismissed_failures_are_remembered() {
    let state = AppState::for_tests(vec![]);
    assert!(list_dismissed_work(&state).await.unwrap().is_empty());
    assert!(dismiss_work(&state, "  ".into()).await.is_err());
    dismiss_work(&state, "run-1".into()).await.unwrap();
    dismiss_work(&state, "run-1".into()).await.unwrap();
    dismiss_work(&state, "t-2".into()).await.unwrap();
    let mut dismissed = list_dismissed_work(&state).await.unwrap();
    dismissed.sort();
    assert_eq!(dismissed, ["run-1", "t-2"]);
    state.shutdown();
}

fn agent_run(status: RunStatus) -> RunSummary {
    RunSummary {
        id: "run-9".into(),
        agent_id: "writer".into(),
        conversation_id: "agent:writer".into(),
        status,
        input: "Morning check".into(),
        summary: None,
        started_at: 2_000.0,
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: None,
        model: None,
        source_type: crate::model::SourceType::User,
        source_id: Some("morning".into()),
        source_label: Some("Morning check".into()),
        priority: 80,
        queued_at: None,
        parent_run_id: None,
        compaction_summary: None,
    }
}

fn fire() -> TriggerFire {
    TriggerFire {
        id: "f1".into(),
        trigger_id: "morning".into(),
        fired_at: 1_000.0,
        payload: json!({}),
        run_id: Some("run-9".into()),
        status: crate::services::triggers::schema::TriggerFireStatus::Queued,
        error: None,
        run_status: None,
    }
}

#[test]
fn a_trigger_firing_is_a_work_item_in_its_runs_state() {
    for (status, expected) in [
        (RunStatus::Queued, WorkState::Active),
        (RunStatus::Running, WorkState::Active),
        (RunStatus::Compacted, WorkState::Active),
        (RunStatus::AwaitingApproval, WorkState::Waiting),
        (RunStatus::Completed, WorkState::Done),
        (RunStatus::Failed, WorkState::Failed),
        (RunStatus::Cancelled, WorkState::Cancelled),
    ] {
        let item = from_trigger_fire(&fire(), "Morning check", &agent_run(status));
        assert_eq!(item.state, expected, "{status:?}");
        assert_eq!(item.kind, WorkKind::Trigger);
        // Named after the trigger, opening the run it started
        assert_eq!(
            (item.id.as_str(), item.title.as_str()),
            ("run-9", "Morning check")
        );
        assert_eq!(item.started_at, 1_000.0);
        assert_eq!(
            item.next_action.is_some(),
            status == RunStatus::AwaitingApproval,
            "{status:?}"
        );
    }
    let done = from_trigger_fire(&fire(), "Morning check", &agent_run(RunStatus::Completed));
    assert_eq!(done.progress.percent, 100);
}

fn task_record(input: serde_json::Value) -> crate::services::a2a::task::TaskRecord {
    use crate::services::a2a::task::{TaskRecord, TaskSourceKind, TaskState};
    TaskRecord {
        id: "t1".into(),
        requester_instance: "me".into(),
        requester_agent: "chief-of-staff".into(),
        provider_instance: "i-sam".into(),
        provider_agent: "i-sam/gateway".into(),
        capability: "assign-me".into(),
        input,
        state: TaskState::Delivered,
        eta_seconds: None,
        deadline: None,
        priority: 0,
        created_at: 1_000.0,
        updated_at: 1_000.0,
        acknowledged_at: None,
        completed_at: None,
        error: None,
        result: None,
        parent_task_id: None,
        source_kind: TaskSourceKind::Direct,
        attempt_number: 1,
        previous_attempt_id: None,
    }
}

#[test]
fn a_task_is_titled_with_what_was_asked() {
    let asked = from_task(&task_record(json!({"text": "Rotate the CI
 deploy keys", "due": "Fri"})));
    assert_eq!(asked.title, "Rotate the CI deploy keys");
    let long = from_task(&task_record(json!({"text": "x ".repeat(200)})));
    assert_eq!(long.title.chars().count(), 120);
    assert!(long.title.ends_with('…'));
    // Nothing readable: the capability
    assert_eq!(
        from_task(&task_record(serde_json::Value::Null)).title,
        "assign-me"
    );
}
