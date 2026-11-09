//! Delegation routes, mirroring `commands/delegation.rs`. Errors are
//! `400 {"error": "..."}`.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use super::routes::ApiResult;
use crate::services::a2a::{self, AgentDelegationSettings};
use crate::services::AppState;

async fn returns(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(a2a::returns::list(state.db()).await)
}

async fn dismiss_return(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(a2a::returns::dismiss_request_return(&state, task_id).await)
}

/// History rows returned when `?limit=` is omitted
const DEFAULT_HISTORY_LIMIT: u32 = 200;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?capability=`; returns `AgentCard[]` (every provider if empty)
        .route("/delegation/providers", get(providers))
        // Body: `{"capability", "input", "target"?}`; returns the task ID
        .route("/delegation/tasks", post(delegate))
        // `?limit=`; returns `TaskResult[]`, newest first
        .route("/delegation/history", get(history))
        // Requests that came back while nobody watched: `RequestReturn[]`
        .route("/delegation/returns", get(returns))
        .route(
            "/delegation/returns/{task_id}/dismiss",
            post(dismiss_return),
        )
        // GET returns `AgentDelegationSettings`; PUT body is one
        .route(
            "/agents/{agent_id}/delegation",
            get(agent_settings).put(set_agent_settings),
        )
}

#[derive(Deserialize)]
struct ProvidersQuery {
    #[serde(default)]
    capability: String,
}

async fn providers(
    State(state): State<AppState>,
    Query(query): Query<ProvidersQuery>,
) -> impl IntoResponse {
    ApiResult(a2a::list_available_providers(&state, &query.capability).await)
}

/// Parses a JSON body, so a malformed one is a `400 {"error"}` like any
/// other error (axum's own rejection is plain text)
fn parse<T: serde::de::DeserializeOwned>(body: Value) -> Result<T, String> {
    serde_json::from_value(body).map_err(|e| format!("Invalid request body: {e}"))
}

#[derive(Deserialize)]
struct DelegateBody {
    capability: String,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    target: Option<String>,
}

async fn delegate(State(state): State<AppState>, Json(body): Json<Value>) -> impl IntoResponse {
    let body: DelegateBody = match parse(body) {
        Ok(body) => body,
        Err(e) => return ApiResult(Err(e)),
    };
    ApiResult(
        a2a::delegate_task(&state, &body.capability, body.input, body.target.as_deref()).await,
    )
}

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
}

async fn history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(DEFAULT_HISTORY_LIMIT);
    ApiResult(a2a::list_delegation_history(&state, limit).await)
}

async fn agent_settings(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(a2a::get_agent_delegation(&state, &agent_id).await)
}

async fn set_agent_settings(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let settings: AgentDelegationSettings = match parse(body) {
        Ok(settings) => settings,
        Err(e) => return ApiResult(Err(e)),
    };
    ApiResult(a2a::set_agent_delegation(&state, &agent_id, settings).await)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::services::AppState;

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let app = super::api().with_state(state.clone());
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn routes_answer_json() {
        let state = AppState::for_tests(vec![]);
        let (status, body) = send(&state, "GET", "/delegation/providers?capability=x", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!([]));

        let (status, body) = send(&state, "GET", "/delegation/history", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!([]));

        let (status, body) = send(
            &state,
            "POST",
            "/delegation/tasks",
            r#"{"capability": "summarize", "input": {"text": "hi"}}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_string(), "the task ID");

        let (status, _) = send(&state, "GET", "/agents/nope/delegation", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Malformed bodies are JSON errors too
        let (status, body) = send(&state, "POST", "/delegation/tasks", "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("Invalid request body"));
        let (status, body) = send(&state, "PUT", "/agents/a/delegation", "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].is_string());
        state.shutdown();
    }
}
