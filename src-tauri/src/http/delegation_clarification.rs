//! Clarification routes (`/api/delegation/...`), mirroring
//! `commands/delegation_clarification.rs`.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::a2a::{clarification, escalation, ClarificationPolicy, ClarificationResponse};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Body `{"round", "answers": {questionId: answer}}`
        .route("/delegation/{task_id}/clarify", post(clarify))
        .route(
            "/delegation/{task_id}/escalate/{question_id}",
            post(escalate),
        )
        // Returns `PendingClarification[]`
        .route("/delegation/pending-clarifications", get(pending))
        // `ClarificationPolicy` (PUT returns the policy as saved)
        .route(
            "/agents/{agent_id}/clarification-policy",
            get(policy).put(set_policy),
        )
}

#[derive(Deserialize)]
struct ClarifyBody {
    #[serde(default)]
    round: u32,
    #[serde(default)]
    answers: HashMap<String, String>,
}

async fn clarify(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(body): Json<ClarifyBody>,
) -> impl IntoResponse {
    let response = ClarificationResponse {
        task_id,
        round: body.round,
        answers: body.answers,
    };
    ApiResult(clarification::respond_to_clarification(&state, response).await)
}

async fn escalate(
    State(state): State<AppState>,
    Path((task_id, question_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(escalation::escalate_clarification(&state, &task_id, &question_id).await)
}

async fn pending(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(clarification::get_pending_clarifications(&state).await)
}

async fn policy(State(state): State<AppState>, Path(agent_id): Path<String>) -> impl IntoResponse {
    ApiResult(clarification::get_clarification_policy(&state, &agent_id).await)
}

async fn set_policy(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(policy): Json<ClarificationPolicy>,
) -> impl IntoResponse {
    ApiResult(clarification::set_clarification_policy(&state, &agent_id, policy).await)
}
