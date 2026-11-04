//! Workflow builder routes, mirroring `commands/workflow_builder.rs`.
//! Errors (including malformed bodies) are `400 {"error": "..."}`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::workflow_graph;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `WorkflowGraph`
        .route("/workflows/{id}/graph", get(graph))
        // Body: `{"yaml": "..."}`; returns `Workflow`
        .route("/workflows/{id}/yaml", put(save_yaml))
}

async fn graph(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(workflow_graph::get_workflow_graph(&state, &id).await)
}

#[derive(Deserialize)]
struct YamlBody {
    yaml: String,
}

async fn save_yaml(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<YamlBody>, JsonRejection>,
) -> impl IntoResponse {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return ApiResult(Err(rejection.body_text())),
    };
    ApiResult(workflow_graph::save_workflow_yaml(&state, &id, &body.yaml).await)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn bad_bodies_and_unknown_workflows_are_400_json() {
        let state = AppState::for_tests(vec![]);
        let app = api().with_state(state.clone());
        for (method, uri, body) in [
            ("PUT", "/workflows/w1/yaml", "{}"),
            ("PUT", "/workflows/w1/yaml", "not json"),
            ("GET", "/workflows/w1/graph", ""),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{method} {uri}");
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(json["error"].is_string(), "{method} {uri}: {json}");
        }
        state.shutdown();
    }
}
