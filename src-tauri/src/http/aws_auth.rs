//! AWS auth routes (`/api/aws/...`), mirroring `commands/aws_auth.rs`. No
//! route takes or returns key material.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::harness::aws_auth;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `AwsCliInfo`
        .route("/aws/cli", get(cli))
        // Returns `AwsProfile[]`
        .route("/aws/profiles", get(profiles))
        // Returns `AwsProfile`
        .route("/aws/profiles/{name}", get(profile))
        // Body `{ profile }`; returns `SsoLoginResult`
        .route("/aws/sso/login", post(start_sso))
        // Returns `SsoLoginResult`
        .route("/aws/sso/login/{id}", get(poll_sso))
        // Body `KeyProfileInput`; returns `AwsProfile`
        .route("/aws/profiles/keys", post(save_keys))
        // Body `SsoSetupInput`; returns `SsoSetup`
        .route("/aws/sso/setup", post(start_setup))
        // Returns `SsoSetup`; DELETE drops it
        .route("/aws/sso/setup/{id}", get(poll_setup).delete(cancel_setup))
        // Body `SsoSetupFinish`; returns `AwsProfile`
        .route("/aws/sso/setup/finish", post(finish_setup))
        // Body `{ profile, region }`; returns `CredentialStatus`
        .route("/aws/credentials/check", post(check))
        // Returns `BedrockApiKeyStatus`
        .route("/aws/bedrock-api-key", get(key_status))
}

#[derive(Deserialize)]
struct SsoBody {
    profile: String,
}

#[derive(Deserialize)]
struct CheckBody {
    profile: Option<String>,
    region: Option<String>,
}

async fn cli(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(aws_auth::detect_aws_cli(&state).await)
}

async fn profiles(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(aws_auth::list_aws_profiles(&state).await)
}

async fn profile(State(state): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    ApiResult(aws_auth::get_aws_profile(&state, &name).await)
}

async fn start_sso(State(state): State<AppState>, Json(body): Json<SsoBody>) -> impl IntoResponse {
    ApiResult(aws_auth::start_aws_sso_login(&state, &body.profile).await)
}

async fn poll_sso(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(aws_auth::poll_aws_sso_login(&state, &id).await)
}

async fn save_keys(
    State(state): State<AppState>,
    Json(input): Json<aws_auth::KeyProfileInput>,
) -> impl IntoResponse {
    ApiResult(aws_auth::save_aws_key_profile(&state, input).await)
}

async fn start_setup(
    State(state): State<AppState>,
    Json(input): Json<aws_auth::SsoSetupInput>,
) -> impl IntoResponse {
    ApiResult(aws_auth::start_aws_sso_setup(&state, input).await)
}

async fn poll_setup(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(aws_auth::poll_aws_sso_setup(&state, &id).await)
}

async fn finish_setup(
    State(state): State<AppState>,
    Json(finish): Json<aws_auth::SsoSetupFinish>,
) -> impl IntoResponse {
    ApiResult(aws_auth::finish_aws_sso_setup(&state, finish).await)
}

async fn cancel_setup(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(aws_auth::cancel_aws_sso_setup(&state, &id).await)
}

async fn check(State(state): State<AppState>, Json(body): Json<CheckBody>) -> impl IntoResponse {
    ApiResult(aws_auth::check_aws_credentials(&state, body.profile, body.region).await)
}

async fn key_status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(aws_auth::get_aws_bedrock_api_key_status(&state).await)
}
