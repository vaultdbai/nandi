//! Gateway routes (`/api/gateway/...`), mirroring `commands/gateway.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};

use super::routes::ApiResult;
use crate::services::gateway::{self, CallerPolicy, GatewayConfig, GatewayManifest};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `GatewayStatus`
        .route("/gateway/status", get(status))
        // Turns on relay.enabled + discoverable and re-registers; returns `GatewayStatus`
        .route("/gateway/reachable", axum::routing::post(make_reachable))
        // `GatewayConfig`
        .route("/gateway/config", get(config).put(set_config))
        // `GatewayManifest` (NANDI.md)
        .route("/gateway/manifest", get(manifest).put(set_manifest))
        // Returns `CallerPolicy[]`
        .route("/gateway/callers", get(callers))
        // PUT body `CallerPolicy`
        .route(
            "/gateway/callers/{caller_instance}",
            put(set_caller).delete(remove_caller),
        )
        // Returns `GatewayUsage`
        .route("/gateway/usage/{caller_instance}", get(usage))
        // Body `{"message": "..."}`; returns the answer (a string)
        .route("/gateway/chat", axum::routing::post(chat))
        // Returns `GatewayHold[]`: requests waiting for the owner's approval
        .route("/gateway/holds", get(holds))
        // Body `{"approve": true|false}`; returns the decided `GatewayHold`
        .route(
            "/gateway/holds/{task_id}/decide",
            axum::routing::post(decide_hold),
        )
        // Returns `GatewayAssignment[]`: work assigned to the owner, still open
        .route("/gateway/assignments", get(assignments))
        // Body `{"done": true|false, "note"?: "..."}`; returns the closed
        // `GatewayAssignment`
        .route(
            "/gateway/assignments/{task_id}/close",
            axum::routing::post(close_assignment),
        )
}

#[derive(serde::Deserialize)]
struct CloseBody {
    #[serde(default)]
    done: Option<bool>,
    #[serde(default)]
    note: Option<String>,
}

async fn assignments(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::list_gateway_assignments(&state).await)
}

async fn close_assignment(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(body): Json<CloseBody>,
) -> impl IntoResponse {
    ApiResult(match body.done {
        Some(done) => gateway::close_gateway_assignment(&state, task_id, done, body.note).await,
        None => Err("Body needs \"done\": true or false".to_string()),
    })
}

#[derive(serde::Deserialize)]
struct DecideBody {
    #[serde(default)]
    approve: Option<bool>,
}

async fn holds(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::list_gateway_holds(&state).await)
}

async fn decide_hold(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(body): Json<DecideBody>,
) -> impl IntoResponse {
    ApiResult(match body.approve {
        Some(approve) => gateway::decide_gateway_hold(&state, task_id, approve).await,
        None => Err("Body needs \"approve\": true or false".to_string()),
    })
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::get_gateway_status(&state).await)
}

async fn make_reachable(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::make_gateway_reachable(&state).await)
}

async fn config(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::get_gateway_config(&state).await)
}

async fn set_config(
    State(state): State<AppState>,
    Json(config): Json<GatewayConfig>,
) -> impl IntoResponse {
    ApiResult(gateway::set_gateway_config(&state, config).await)
}

async fn manifest(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::get_gateway_manifest(&state).await)
}

async fn set_manifest(
    State(state): State<AppState>,
    Json(manifest): Json<GatewayManifest>,
) -> impl IntoResponse {
    ApiResult(gateway::set_gateway_manifest(&state, manifest).await)
}

async fn callers(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::list_gateway_callers(&state).await)
}

async fn set_caller(
    State(state): State<AppState>,
    Path(caller_instance): Path<String>,
    Json(mut policy): Json<CallerPolicy>,
) -> impl IntoResponse {
    policy.caller_instance = caller_instance;
    ApiResult(gateway::set_caller_policy(&state, policy).await)
}

async fn remove_caller(
    State(state): State<AppState>,
    Path(caller_instance): Path<String>,
) -> impl IntoResponse {
    ApiResult(gateway::remove_caller_policy(&state, &caller_instance).await)
}

async fn usage(
    State(state): State<AppState>,
    Path(caller_instance): Path<String>,
) -> impl IntoResponse {
    ApiResult(gateway::get_gateway_usage(&state, &caller_instance).await)
}

#[derive(serde::Deserialize)]
struct ChatBody {
    #[serde(default)]
    message: String,
}

async fn chat(State(state): State<AppState>, Json(body): Json<ChatBody>) -> impl IntoResponse {
    ApiResult(gateway::gateway_chat(&state, &body.message).await)
}
