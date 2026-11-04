//! Git remote routes (`/api/remote/...`), mirroring `commands/remote.rs`.
//! The credential goes in (PUT config) and never comes back out.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::remote::{self, RemoteCredential};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `RemoteStatus`
        .route("/remote/status", get(status))
        // Body `{"url", "credential"?, "prefix"?, "slug"?}`; returns `RemoteStatus`
        .route("/remote/config", put(configure))
        .route("/remote", axum::routing::delete(clear))
        .route("/remote/test", post(test))
        // Body `{"overwrite": bool}` (optional); returns `PushResult`
        .route("/remote/push", post(push))
        // Returns `PullResult`
        .route("/remote/pull", post(pull))
        // Returns `string[]`
        .route("/remote/branches", get(branches))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigureBody {
    #[serde(default)]
    url: String,
    credential: Option<RemoteCredential>,
    prefix: Option<String>,
    slug: Option<String>,
}

#[derive(Deserialize, Default)]
struct PushBody {
    #[serde(default)]
    overwrite: bool,
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(remote::get_remote_status(&state).await)
}

async fn configure(
    State(state): State<AppState>,
    Json(body): Json<ConfigureBody>,
) -> impl IntoResponse {
    ApiResult(
        remote::configure_remote(&state, body.url, body.credential, body.prefix, body.slug).await,
    )
}

async fn clear(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(remote::clear_remote(&state).await)
}

async fn test(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(remote::test_remote_connection(&state).await)
}

async fn push(State(state): State<AppState>, body: Option<Json<PushBody>>) -> impl IntoResponse {
    let overwrite = body.map(|Json(body)| body.overwrite).unwrap_or_default();
    ApiResult(remote::push_to_remote(&state, overwrite).await)
}

async fn pull(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(remote::pull_from_remote(&state).await)
}

async fn branches(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(remote::list_remote_branches(&state).await)
}
