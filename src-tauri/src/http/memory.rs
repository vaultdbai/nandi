//! Agent memory routes (wave final, Track C), mirroring
//! `commands/memory.rs`. Errors are `400 {"error": "..."}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::agent_memory;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `MemoryEntry[]`
        .route("/agents/{agent_id}/memory", get(load))
        // PUT body `{"content": "..."}`; DELETE removes the entry
        .route(
            "/agents/{agent_id}/memory/{index}",
            put(update).delete(delete),
        )
        // Returns `MemoryEntry[]` (the whole memory)
        .route("/agents/{agent_id}/memory/refresh", post(refresh))
        // GET returns `boolean`; PUT body `{"enabled": bool}`
        .route(
            "/agents/{agent_id}/memory-enabled",
            get(enabled).put(set_enabled),
        )
}

// Missing fields are a readable 400 from the service, not a bare 422
#[derive(Deserialize)]
struct UpdateBody {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct EnabledBody {
    #[serde(default)]
    enabled: Option<bool>,
}

async fn load(State(state): State<AppState>, Path(agent_id): Path<String>) -> impl IntoResponse {
    ApiResult(agent_memory::load_memory(&state, &agent_id).await)
}

async fn update(
    State(state): State<AppState>,
    Path((agent_id, index)): Path<(String, u32)>,
    Json(body): Json<UpdateBody>,
) -> impl IntoResponse {
    ApiResult(agent_memory::update_memory_entry(&state, &agent_id, index, body.content).await)
}

async fn delete(
    State(state): State<AppState>,
    Path((agent_id, index)): Path<(String, u32)>,
) -> impl IntoResponse {
    ApiResult(agent_memory::delete_memory(&state, &agent_id, index).await)
}

async fn refresh(State(state): State<AppState>, Path(agent_id): Path<String>) -> impl IntoResponse {
    ApiResult(agent_memory::refresh_from_run(&state, &agent_id, None).await)
}

async fn enabled(State(state): State<AppState>, Path(agent_id): Path<String>) -> impl IntoResponse {
    ApiResult(agent_memory::memory_enabled(&state, &agent_id).await)
}

async fn set_enabled(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<EnabledBody>,
) -> impl IntoResponse {
    let result = match body.enabled {
        Some(enabled) => agent_memory::set_memory_enabled(&state, &agent_id, enabled).await,
        None => Err("Missing 'enabled'".to_string()),
    };
    ApiResult(result)
}
