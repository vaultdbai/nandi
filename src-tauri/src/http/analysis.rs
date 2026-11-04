//! Run analysis routes, mirroring `commands/analysis.rs`. Errors (including
//! malformed bodies) are `400 {"error": "..."}`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::run_analysis::{self, AnalysisResponse};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Body: `{"question": "..."}`; returns `AnalysisResponse`
        .route("/runs/{id}/analyze", post(analyze))
        // Returns the summary (markdown string)
        .route("/runs/{id}/summary", get(summary))
}

#[derive(Deserialize)]
struct AnalyzeBody {
    question: String,
}

async fn analyze(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    ApiResult(answer(&state, &id, &body).await)
}

async fn answer(state: &AppState, id: &str, body: &[u8]) -> Result<AnalysisResponse, String> {
    let body: AnalyzeBody =
        serde_json::from_slice(body).map_err(|e| format!("Invalid request body: {e}"))?;
    run_analysis::analyze_run(state, id, &body.question).await
}

async fn summary(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(run_analysis::summarize_run(&state, &id).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn call(state: &AppState, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = api()
            .with_state(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn post(uri: &str, body: &str) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn malformed_bodies_are_400_json() {
        let state = AppState::for_tests(vec![]);
        for body in ["", "{", "{}", r#"{"question": 3}"#] {
            let (status, json) = call(&state, post("/runs/r1/analyze", body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(
                json["error"]
                    .as_str()
                    .unwrap()
                    .contains("Invalid request body"),
                "{body}: {json}"
            );
        }
        state.shutdown();
    }

    #[tokio::test]
    async fn unknown_runs_are_400_json() {
        let state = AppState::for_tests(vec![]);
        let (status, json) =
            call(&state, post("/runs/nope/analyze", r#"{"question":"Why?"}"#)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("not found"));

        let request = Request::get("/runs/nope/summary")
            .body(Body::empty())
            .unwrap();
        let (status, json) = call(&state, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("not found"));
        state.shutdown();
    }
}
