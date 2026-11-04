//! Task lifecycle routes (`/api/tasks/...`, `/api/eta/...`), mirroring
//! `commands/tasks.rs`.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::a2a::reassign::{self, ReassignRequest};
use crate::services::a2a::task::{self, TaskFilter};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Query: `state`, `role`, `agentId`, `limit`; returns `TaskRecord[]`
        .route("/tasks", get(list))
        .route("/tasks/{id}", get(detail))
        // Body `{"reason"?}`; returns `TaskRecord`
        .route("/tasks/{id}/cancel", post(cancel))
        // Returns `TaskStatusReport`
        .route("/tasks/{id}/status", get(status))
        .route("/tasks/{id}/events", get(events))
        .route("/tasks/{id}/messages", get(messages))
        // Body `{"newProviderInstance", "reason"?}`; returns `TaskRecord`
        .route("/tasks/{id}/reassign", post(reassign))
        // Returns `ReassignCandidate[]`
        .route("/tasks/{id}/reassign-candidates", get(reassign_candidates))
        // Query: `providerAgent`; returns `EtaEstimate`
        .route("/eta/{capability}", get(eta))
}

#[derive(Deserialize, Default)]
struct CancelBody {
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EtaQuery {
    provider_agent: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    Query(filter): Query<TaskFilter>,
) -> impl IntoResponse {
    ApiResult(task::list_tasks(&state, filter).await)
}

async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(task::get_task(&state, &id).await)
}

async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<CancelBody>>,
) -> impl IntoResponse {
    let reason = body.and_then(|Json(body)| body.reason);
    ApiResult(task::cancel_task(&state, &id, reason).await)
}

async fn status(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(task::query_task_status(&state, &id).await)
}

async fn events(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(task::get_task_events(&state, &id).await)
}

async fn messages(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(task::get_task_messages(&state, &id).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReassignBody {
    new_provider_instance: String,
    reason: Option<String>,
}

async fn reassign(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(body): Json<ReassignBody>,
) -> impl IntoResponse {
    let request = ReassignRequest {
        task_id,
        new_provider_instance: body.new_provider_instance,
        reason: body.reason,
    };
    ApiResult(reassign::reassign_task(&state, request).await)
}

async fn reassign_candidates(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(reassign::reassign_candidates(&state, &id).await)
}

async fn eta(
    State(state): State<AppState>,
    Path(capability): Path<String>,
    Query(query): Query<EtaQuery>,
) -> impl IntoResponse {
    ApiResult(task::get_eta_estimate(&state, &capability, query.provider_agent).await)
}
