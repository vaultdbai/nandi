//! Harness routes (`/api/harnesses/...`), mirroring `commands/harnesses.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::harness;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `HarnessInfo[]`
        .route("/harnesses", get(list))
        // Returns `HarnessInfo[]`
        .route("/harnesses/detect", post(detect))
        // GET returns `HarnessDefault | null`; PUT body `{ harnessId, model }`
        // returns `HarnessDefault`; DELETE returns `null` (static: matched
        // before `{id}`)
        .route(
            "/harnesses/default",
            get(get_default).put(set_default).delete(clear_default),
        )
        // Returns `HarnessHealth`
        .route("/harnesses/{id}/health", get(health))
        // Returns `HarnessTestResult`
        .route("/harnesses/{id}/test", post(test))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DefaultBody {
    harness_id: String,
    model: Option<String>,
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(harness::list_harnesses(&state).await)
}

async fn detect(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(harness::detect_harnesses(&state).await)
}

async fn get_default(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(harness::get_harness_default(&state).await)
}

async fn set_default(
    State(state): State<AppState>,
    Json(body): Json<DefaultBody>,
) -> impl IntoResponse {
    ApiResult(harness::set_harness_default(&state, &body.harness_id, body.model).await)
}

async fn clear_default(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(harness::clear_harness_default(&state).await)
}

async fn health(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(harness::get_harness_health(&state, &id).await)
}

async fn test(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(harness::test_harness(&state, &id).await)
}
