//! Harness session routes (`/api/harness-sessions/...`), mirroring
//! `commands/harness_sessions.rs`.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::harness;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?agentId=&limit=`; returns `HarnessSessionSummary[]`
        .route("/harness-sessions", get(list))
        // GET returns `HarnessSessionSummary`; DELETE kills it (`null`)
        .route("/harness-sessions/{id}", get(get_one).delete(kill))
        // `?afterSeq=`; returns `HarnessEventRecord[]`
        .route("/harness-sessions/{id}/events", get(events))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    agent_id: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsQuery {
    after_seq: Option<u32>,
}

async fn list(State(state): State<AppState>, Query(query): Query<ListQuery>) -> impl IntoResponse {
    ApiResult(harness::list_harness_sessions(&state, query.agent_id, query.limit).await)
}

async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(harness::get_harness_session(&state, &id).await)
}

async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> impl IntoResponse {
    ApiResult(harness::get_harness_events(&state, &id, query.after_seq).await)
}

async fn kill(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(harness::kill_harness_session(&state, &id).await)
}
