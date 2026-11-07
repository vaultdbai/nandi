//! Live UI routes (`/api/changes`), mirroring `commands/live.rs`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use super::routes::ApiResult;
use crate::services::live;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `LiveChange[]`, newest first
        .route("/changes", get(recent))
        // Re-reads every definition from disk; returns `RefreshReport`
        .route("/changes/refresh", post(refresh))
}

async fn recent(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(state.live().feed().recent()))
}

async fn refresh(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(live::refresh_from_disk(&state).await)
}
