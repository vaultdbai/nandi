//! Work routes (`/api/work...`), mirroring `commands/work.rs`.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::work::{self, WorkState};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?state=`; returns `WorkItem[]`
        .route("/work", get(list))
        // Returns the IDs of failures dismissed from the Inbox
        .route("/work/dismissed", get(dismissed))
        .route("/work/{id}/dismiss", post(dismiss))
        // Returns `WorkItem`
        .route("/work/{id}", get(item))
        .route("/work/{id}/pause", post(pause))
        .route("/work/{id}/resume", post(resume))
        .route("/work/{id}/cancel", post(cancel))
        // Body `{ rating }` (1–5)
        .route("/work/{id}/rating", post(rate))
}

#[derive(Deserialize)]
struct ListQuery {
    state: Option<WorkState>,
}

async fn list(State(state): State<AppState>, Query(query): Query<ListQuery>) -> impl IntoResponse {
    ApiResult(work::list_work(&state, query.state).await)
}

async fn dismissed(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(work::list_dismissed_work(&state).await)
}

async fn dismiss(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(work::dismiss_work(&state, id).await)
}

async fn item(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(work::get_work_item(&state, id).await)
}

async fn pause(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(work::pause_work(&state, id).await)
}

async fn resume(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(work::resume_work(&state, id).await)
}

async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(work::cancel_work(&state, id).await)
}

#[derive(Deserialize)]
struct RatingBody {
    rating: u32,
}

async fn rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RatingBody>,
) -> impl IntoResponse {
    ApiResult(work::rate_work(&state, id, body.rating).await)
}
