//! Ask box routes (`/api/ask...`), mirroring `commands/ask.rs`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::ask::{self, AskRequest};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Body `AskRequest`; returns `AskResponse`
        .route("/ask", post(answer))
        // Body `AskRequest`; returns the request ID (events follow)
        .route("/ask/stream", post(stream))
        // `?limit=`; returns `AskHistoryEntry[]`
        .route("/ask/history", get(history))
}

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
}

async fn answer(
    State(state): State<AppState>,
    Json(request): Json<AskRequest>,
) -> impl IntoResponse {
    ApiResult(ask::ask(&state, request).await)
}

async fn stream(
    State(state): State<AppState>,
    Json(request): Json<AskRequest>,
) -> impl IntoResponse {
    ApiResult(ask::ask_streaming(&state, request).await)
}

async fn history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    ApiResult(ask::list_ask_history(&state, query.limit).await)
}
