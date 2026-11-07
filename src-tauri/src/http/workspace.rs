//! Calendar and files routes (`/api/workspace`), mirroring
//! `commands/workspace.rs`. Tokens never come back out.

use axum::extract::{Path, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::EmailSignInPoll;
use crate::services::workspace::{self, AccountKind, WorkspaceStatus};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `WorkspaceStatus`
        .route("/workspace", get(status))
        // Returns `WorkspaceStatus` (the check runs in the background)
        .route("/workspace/calendar/check", post(check))
        // PUT body `{"minutes": n}`
        .route("/workspace/calendar/interval", put(interval))
        // Returns `DeviceFlowState`
        .route("/workspace/{kind}/sign-in", post(sign_in))
        // Returns `EmailSignInPoll`
        .route("/workspace/{kind}/sign-in/poll", post(poll))
        // Signs out; returns `WorkspaceStatus`
        .route("/workspace/{kind}", delete(disconnect))
}

fn kind(id: &str) -> Result<AccountKind, String> {
    AccountKind::of_id(id).ok_or_else(|| format!("Unknown account '{id}'"))
}

async fn status(State(state): State<AppState>) -> ApiResult<WorkspaceStatus> {
    ApiResult(Ok(workspace::status(&state).await))
}

async fn check(State(state): State<AppState>) -> ApiResult<WorkspaceStatus> {
    ApiResult(Ok(workspace::check_now(&state).await))
}

#[derive(Deserialize)]
struct IntervalBody {
    minutes: u32,
}

async fn interval(
    State(state): State<AppState>,
    Json(body): Json<IntervalBody>,
) -> ApiResult<WorkspaceStatus> {
    ApiResult(workspace::set_calendar_interval(&state, body.minutes).await)
}

async fn sign_in(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<DeviceFlowState> {
    ApiResult(match kind(&id) {
        Ok(k) => workspace::start_sign_in(&state, k).await,
        Err(e) => Err(e),
    })
}

async fn poll(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<EmailSignInPoll> {
    ApiResult(match kind(&id) {
        Ok(k) => Ok(workspace::poll_sign_in(&state, k).await),
        Err(e) => Err(e),
    })
}

async fn disconnect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<WorkspaceStatus> {
    ApiResult(match kind(&id) {
        Ok(k) => workspace::disconnect(&state, k).await,
        Err(e) => Err(e),
    })
}
