//! Decisions page routes (`/api/decisions`), mirroring
//! `commands/decisions.rs`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::domain::decision::DecisionFilter;
use crate::services::inbox::decisions;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns the months with decisions, newest first (`string[]`)
        .route("/decisions/months", get(months))
        // `?month=2026-11&kind=&decision=&search=`; returns `DecisionRecord[]`
        .route("/decisions", get(list))
}

#[derive(Deserialize)]
struct ListQuery {
    month: String,
    kind: Option<String>,
    decision: Option<String>,
    search: Option<String>,
}

async fn months(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(decisions::months(&state))
}

async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> impl IntoResponse {
    let filter = DecisionFilter {
        kind: q.kind,
        decision: q.decision,
        search: q.search,
    };
    ApiResult(decisions::list(&state, q.month, Some(filter)).await)
}
