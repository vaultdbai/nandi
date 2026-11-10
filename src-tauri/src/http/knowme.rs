//! Getting to know you routes (`/api/know-me`), mirroring
//! `commands/knowme.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::knowme::proposal::KnowMeProposal;
use crate::services::knowme::{self, KnowMeTodoAction};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `KnowMeStatus`
        .route("/know-me", get(status))
        // `{action: "later" | "skip"}`; returns `KnowMeStatus`
        .route("/know-me/todo", post(todo))
        // A `KnowMeProposal` (as edited on the card); confirms the approval
        .route("/know-me/proposals/{approval_id}/confirm", post(confirm))
}

#[derive(Deserialize)]
struct TodoBody {
    action: KnowMeTodoAction,
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(knowme::status(&state).await)
}

async fn todo(State(state): State<AppState>, Json(body): Json<TodoBody>) -> impl IntoResponse {
    ApiResult(knowme::todo(&state, body.action).await)
}

async fn confirm(
    State(state): State<AppState>,
    Path(approval_id): Path<String>,
    Json(proposal): Json<KnowMeProposal>,
) -> impl IntoResponse {
    ApiResult(knowme::confirm(&state, approval_id, proposal).await)
}
