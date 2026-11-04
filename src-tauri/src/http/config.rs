//! Repo config routes (`/api/config/...`), mirroring `commands/config.rs`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use super::routes::ApiResult;
use crate::services::config::repo_config;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `RepoLlmConfigStatus`
        .route("/config/llm", get(llm))
        .route("/config/llm/reload", post(reload))
}

async fn llm(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(repo_config::get_repo_llm_config(&state).await)
}

async fn reload(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(repo_config::reload_repo_llm_config(&state).await)
}
