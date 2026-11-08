//! Connector access routes (`/api/connector-grants`, `/api/access-requests`),
//! mirroring `commands/access.rs`.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::access::{self, AccessChoice};
use crate::services::domain::grant::{GrantAccess, GrantScope};
use crate::services::grants::GrantInfo;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // GET returns `GrantInfo[]`; POST body `{agentId, connectorId, access, scope}`
        .route("/connector-grants", get(list).post(grant))
        // Body `GrantInfo`
        .route("/connector-grants/revoke", post(revoke))
        // Body `{choice}`
        .route("/access-requests/{approval_id}", post(answer))
}

async fn list(State(state): State<AppState>) -> ApiResult<Vec<GrantInfo>> {
    ApiResult(Ok(state.grants().list()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrantBody {
    agent_id: String,
    connector_id: String,
    access: GrantAccess,
    scope: GrantScope,
}

async fn grant(State(state): State<AppState>, Json(body): Json<GrantBody>) -> ApiResult<()> {
    let who = crate::services::chat::decider(&state);
    ApiResult(
        access::grant_up_front(
            &state,
            &body.agent_id,
            &body.connector_id,
            body.access,
            body.scope,
            who,
        )
        .await,
    )
}

async fn revoke(State(state): State<AppState>, Json(grant): Json<GrantInfo>) -> ApiResult<()> {
    let who = crate::services::chat::decider(&state);
    ApiResult(access::revoke(&state, grant, who).await)
}

#[derive(Deserialize)]
struct AnswerBody {
    choice: AccessChoice,
    #[serde(default)]
    subdomains: bool,
}

async fn answer(
    State(state): State<AppState>,
    Path(approval_id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> ApiResult<()> {
    let who = crate::services::chat::decider(&state);
    ApiResult(access::answer(&state, &approval_id, body.choice, body.subdomains, who).await)
}
