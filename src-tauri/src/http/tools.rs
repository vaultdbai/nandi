//! Tool catalog routes (`/api/tools/...`), mirroring `commands/tools.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::tools;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `ToolEntry[]`
        .route("/tools", get(list))
        // Returns `ToolUsageStats[]` (static: matched before `{id}`)
        .route("/tools/usage", get(usage))
        // Returns `ToolEntry`
        .route("/tools/{id}", get(get_one))
        // Body `{ enabled: boolean }`; returns `ToolEntry`
        .route("/tools/{id}/enabled", put(set_enabled))
}

#[derive(Deserialize)]
struct EnabledBody {
    enabled: bool,
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(tools::list_tools(&state).await)
}

async fn usage(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(tools::get_tool_usage_stats(&state).await)
}

async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(tools::get_tool(&state, &id).await)
}

async fn set_enabled(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<EnabledBody>,
) -> impl IntoResponse {
    ApiResult(tools::set_tool_enabled(&state, &id, body.enabled).await)
}
