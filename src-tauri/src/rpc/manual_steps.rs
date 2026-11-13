//! Manual workflow step commands (Tauri adapter for
//! `services::workflows::manual_steps`).

use crate::services::workflows::manual_steps;
use crate::services::workflows::manual_steps::{ManualStepCompletion, PendingManualStep};

super::commands! {
    /// Completes a manual step and resumes its workflow run
    fn complete_manual_step(state, run_id: String, step_id: String, completion: ManualStepCompletion) -> () => manual_steps::complete_manual_step(&state, &run_id, &step_id, completion).await;

    /// Manual steps waiting for someone, oldest first
    fn list_pending_manual_steps(state) -> Vec<PendingManualStep> => manual_steps::list_pending_manual_steps(&state).await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{json, Value};

    use crate::db;
    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    const ONBOARDING: &str = r#"
id: onboarding
name: Onboarding Aayushi
steps:
  - id: accounts
    agent: helper
    input: "Create the accounts"
  - id: laptop
    step_type: manual
    instructions: "Tag the laptop for {{accounts.output}}"
  - id: welcome
    agent: helper
    input: "Welcome; laptop {{laptop.output}}"
"#;

    /// A workflow waits on a manual step and resumes when it's completed;
    /// only the waiting step can be completed
    #[tokio::test]
    async fn a_manual_step_waits_for_its_completion() {
        let state = AppState::for_tests(vec![]);
        state
            .db()
            .call(|conn| {
                db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
                db::set_setting(
                    conn,
                    crate::engine::CANNED_SCRIPT_KEY,
                    &json!({ "agent": [{ "text": "[{{input}}]" }], "studio": [], "tokenDelayMs": 0 }),
                )
            })
            .await
            .unwrap();
        let engine = state.workflows().clone();
        engine.create_workflow(ONBOARDING).await.unwrap();
        let run_id = engine.run_workflow("onboarding").await.unwrap();
        let mut pending = json!([]);
        for _ in 0..500 {
            let steps = call(&state, "list_pending_manual_steps", json!({}))
                .await
                .unwrap();
            if steps.as_array().is_some_and(|steps| !steps.is_empty()) {
                pending = steps;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(pending[0]["runId"], json!(run_id));
        assert_eq!(pending[0]["workflowName"], "Onboarding Aayushi");
        assert_eq!(
            pending[0]["instructions"],
            "Tag the laptop for [Create the accounts]"
        );

        let not_waiting = json!({ "runId": run_id, "stepId": "welcome", "completion": {} });
        assert_eq!(
            call(&state, "complete_manual_step", not_waiting).await,
            Err("This step is not waiting for anyone".to_string())
        );
        let laptop = json!({
            "runId": run_id,
            "stepId": "laptop",
            "completion": { "output": "LT-7", "attachmentIds": [] },
        });
        assert_eq!(
            call(&state, "complete_manual_step", laptop).await,
            Ok(Value::Null)
        );
        let mut done = None;
        for _ in 0..500 {
            let run = engine.run(&run_id).await.unwrap().unwrap();
            if run.status == crate::services::workflows::WorkflowRunStatus::Completed {
                done = Some(run);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let done = done.expect("the run resumed and finished");
        assert_eq!(
            done.steps[2].output.as_deref(),
            Some("[Welcome; laptop LT-7]")
        );
        assert_eq!(
            call(&state, "list_pending_manual_steps", json!({})).await,
            Ok(json!([]))
        );
        state.shutdown();
    }
}
