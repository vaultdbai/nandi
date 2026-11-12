//! Manual workflow step routes, mirroring `commands/manual_steps.rs`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use super::routes::ApiResult;
use crate::services::workflows::manual_steps::{self, ManualStepCompletion};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `PendingManualStep[]`
        .route("/workflows/manual-steps", get(pending))
        // Body `ManualStepCompletion`; an empty body means "no reply"
        .route(
            "/workflows/{run_id}/steps/{step_id}/complete",
            post(complete),
        )
}

async fn pending(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(manual_steps::list_pending_manual_steps(&state).await)
}

async fn complete(
    State(state): State<AppState>,
    Path((run_id, step_id)): Path<(String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    ApiResult(finish(&state, &run_id, &step_id, &body).await)
}

async fn finish(state: &AppState, run_id: &str, step_id: &str, body: &[u8]) -> Result<(), String> {
    let completion = completion_from(body)?;
    manual_steps::complete_manual_step(state, run_id, step_id, completion).await
}

/// The body is optional: an empty one (whatever the `Content-Type`) is a
/// completion with no reply and no attachments
fn completion_from(body: &[u8]) -> Result<ManualStepCompletion, String> {
    if body.trim_ascii().is_empty() {
        return Ok(ManualStepCompletion::default());
    }
    serde_json::from_slice(body).map_err(|e| format!("Invalid request body: {e}"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::db;

    #[test]
    fn an_empty_body_is_a_completion_with_no_reply() {
        assert_eq!(completion_from(b""), Ok(ManualStepCompletion::default()));
        assert_eq!(
            completion_from(
                b" 
"
            ),
            Ok(ManualStepCompletion::default())
        );
        assert_eq!(
            completion_from(br#"{"output":"LT-7"}"#),
            Ok(ManualStepCompletion {
                output: Some("LT-7".to_string()),
                attachment_ids: vec![],
            })
        );
        assert!(completion_from(b"{not json")
            .unwrap_err()
            .starts_with("Invalid request body"));
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
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

    /// The wave EF routes over HTTP, in one pass: the network directory and
    /// a member's trust, the repo LLM config, a workflow that waits on a
    /// manual step and resumes when it's completed, and reassigning a task
    /// this instance isn't serving (refused)
    #[tokio::test]
    async fn wave_ef_routes_work_over_http() {
        let state = AppState::for_tests(vec![]);
        let app = api()
            .merge(super::super::network::api())
            .merge(super::super::rpc::api())
            .merge(super::super::tasks::api())
            .with_state(state.clone());

        let (status, body) = call(&app, "GET", "/network/snapshot", "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["members"], json!([]));
        let (status, body) = call(
            &app,
            "PUT",
            "/network/members/bob-1/trust",
            r#"{"addedAsTrustedPeer":null,"delegationsPaused":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["delegationsPaused"], true);
        let (status, body) = call(&app, "GET", "/network/members/bob-1", "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["trust"]["delegationsPaused"], true);

        let (status, body) = call(&app, "POST", "/rpc/get_repo_llm_config", "{}").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["found"], false);
        let (status, body) = call(&app, "POST", "/rpc/reload_repo_llm_config", "{}").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["syncedAt"].is_number());

        // A workflow that waits on a person
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
            let (status, body) = call(&app, "GET", "/workflows/manual-steps", "").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            if body.as_array().is_some_and(|steps| !steps.is_empty()) {
                pending = body;
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

        let (status, body) = call(
            &app,
            "POST",
            &format!("/workflows/{run_id}/steps/welcome/complete"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "only the waiting step");
        assert_eq!(
            body["error"], "This step is not waiting for anyone",
            "an empty JSON body reaches the service"
        );
        let (status, body) = call(
            &app,
            "POST",
            &format!("/workflows/{run_id}/steps/laptop/complete"),
            r#"{"output":"LT-7","attachmentIds":[]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
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
        let (_, body) = call(&app, "GET", "/workflows/manual-steps", "").await;
        assert_eq!(body, json!([]));

        let (status, body) = call(
            &app,
            "POST",
            "/tasks/nope/reassign",
            r#"{"newProviderInstance":"bob-1"}"#,
        )
        .await;
        assert!(status.is_client_error(), "{status} {body}");
        state.shutdown();
    }
}
