//! Email routes (`/api/email`), mirroring `commands/email.rs`. Tokens
//! never come back out.

use axum::extract::{Path, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::{
    self, BackfillAction, BackfillEstimate, BackfillRange, EmailSettingsInput, EmailSignInPoll,
    EmailStatus, MailProvider,
};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `EmailStatus`
        .route("/email", get(get_status))
        // PUT body `EmailSettingsInput`; returns `EmailStatus`
        .route("/email/settings", put(settings))
        // Returns `EmailStatus` (the check runs in the background)
        .route("/email/check", post(check))
        // Returns `DeviceFlowState`
        .route("/email/{provider}/sign-in", post(sign_in))
        // Returns `EmailSignInPoll`
        .route("/email/{provider}/sign-in/poll", post(poll))
        // PUT body `{"paused": bool}`; returns `EmailStatus`
        .route("/email/{provider}/paused", put(paused))
        // PUT body `{"action": "pause"|"resume"|"stop"|"restart"}`; returns `EmailStatus`
        .route("/email/{provider}/backfill", put(backfill))
        // POST body `{"range": BackfillRange}`; returns `BackfillEstimate[]`
        .route("/email/backfill/estimate", post(estimate))
        // Signs out; returns `EmailStatus`
        .route("/email/{provider}", delete(disconnect))
}

fn provider(id: &str) -> Result<MailProvider, String> {
    MailProvider::ALL
        .into_iter()
        .find(|p| p.id() == id)
        .ok_or_else(|| format!("Unknown mail provider '{id}'"))
}

async fn get_status(State(state): State<AppState>) -> ApiResult<EmailStatus> {
    ApiResult(Ok(email::status(&state).await))
}

async fn settings(
    State(state): State<AppState>,
    Json(body): Json<EmailSettingsInput>,
) -> ApiResult<EmailStatus> {
    ApiResult(email::set_settings(&state, body).await)
}

async fn check(State(state): State<AppState>) -> ApiResult<EmailStatus> {
    ApiResult(Ok(email::check_now(&state).await))
}

async fn sign_in(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<DeviceFlowState> {
    ApiResult(match provider(&id) {
        Ok(p) => email::start_sign_in(&state, p).await,
        Err(e) => Err(e),
    })
}

async fn poll(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<EmailSignInPoll> {
    ApiResult(match provider(&id) {
        Ok(p) => Ok(email::poll_sign_in(&state, p).await),
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct PausedBody {
    #[serde(default)]
    paused: bool,
}

async fn paused(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PausedBody>,
) -> ApiResult<EmailStatus> {
    ApiResult(match provider(&id) {
        Ok(p) => email::set_paused(&state, p, body.paused).await,
        Err(e) => Err(e),
    })
}

async fn disconnect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<EmailStatus> {
    ApiResult(match provider(&id) {
        Ok(p) => email::disconnect(&state, p).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct BackfillBody {
    action: BackfillAction,
}

async fn backfill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<BackfillBody>,
) -> ApiResult<EmailStatus> {
    ApiResult(match provider(&id) {
        Ok(p) => email::backfill::set_backfill(&state, p, body.action).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct EstimateBody {
    range: BackfillRange,
}

async fn estimate(
    State(state): State<AppState>,
    Json(body): Json<EstimateBody>,
) -> ApiResult<Vec<BackfillEstimate>> {
    ApiResult(Ok(email::backfill::estimate(&state, body.range).await))
}
