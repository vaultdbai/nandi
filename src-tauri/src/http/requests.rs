//! Request routes (`/api/requests/...`), mirroring `commands/requests.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::ask::ActionPlan;
use crate::services::requests::local;
use crate::services::{requests, AppState};

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `Request` (any of its task IDs works)
        .route("/requests/{id}", get(detail))
        // Returns `Request` after a new attempt on the next provider
        .route("/requests/{id}/retry", post(retry))
        // Returns the Request ID a run is part of, or `null`
        .route("/runs/{id}/request", get(for_run))
        // Body `{ plan, text }`; returns the new Request's ID
        .route("/requests", post(start))
        // Returns `null` once the step waiting on the user is done
        .route(
            "/requests/{id}/steps/{position}/complete",
            post(complete_step),
        )
        // Returns `null` once the local Request is stopped
        .route("/requests/{id}/cancel", post(cancel))
        // Returns the new Request's ID (the same plan again)
        .route("/requests/{id}/rerun", post(rerun))
        // Returns the drafted workflow YAML
        .route("/requests/{id}/workflow-draft", post(draft_workflow))
}

#[derive(Deserialize)]
struct StartBody {
    plan: ActionPlan,
    text: String,
}

async fn start(State(state): State<AppState>, Json(body): Json<StartBody>) -> impl IntoResponse {
    ApiResult(local::start_request(&state, body.plan, body.text).await)
}

async fn complete_step(
    State(state): State<AppState>,
    Path((id, position)): Path<(String, u32)>,
) -> impl IntoResponse {
    ApiResult(local::complete_request_step(&state, id, position).await)
}

async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(local::cancel_request(&state, id).await)
}

async fn rerun(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(local::rerun_request(&state, id).await)
}

async fn draft_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(local::draft_workflow_from_request(&state, id).await)
}

async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(requests::get_request(&state, id).await)
}

async fn retry(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(requests::retry_task(&state, id).await)
}

async fn for_run(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(requests::request_for_run(&state, id).await)
}
