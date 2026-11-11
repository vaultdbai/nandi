//! Identity routes (`/api/identity/...`), mirroring `commands/identity.rs`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::{auth, AppState};

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `IdentityStatus`
        .route("/identity/status", get(status))
        // Returns `VerifiedIdentity | null`
        .route("/identity/verified", get(verified))
        // Body `{"name": "..."}`
        .route("/identity/display-name", put(display_name))
        // Body `{"slug": "..."}`
        .route("/identity/user-slug", put(user_slug))
        // Body `{"step": "model"}`
        .route("/identity/onboarding-step", put(onboarding_step))
        // Returns `GithubSignIn`; DELETE signs out
        .route(
            "/github/sign-in",
            get(github_status).delete(github_sign_out),
        )
        // Returns `DeviceFlowState`
        .route("/github/sign-in/start", post(github_start))
        // Returns `GithubSignIn`
        .route("/github/sign-in/poll", post(github_poll))
}

async fn github_status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(auth::github::status(&state)))
}

async fn github_start(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(auth::github::start(&state).await)
}

async fn github_poll(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(auth::github::poll(&state).await)
}

async fn github_sign_out(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(auth::github::sign_out(&state))
}

#[derive(Deserialize)]
struct StepBody {
    #[serde(default)]
    step: Option<auth::OnboardingStep>,
}

async fn onboarding_step(
    State(state): State<AppState>,
    Json(body): Json<StepBody>,
) -> impl IntoResponse {
    ApiResult(match body.step {
        Some(step) => auth::set_onboarding_step(&state, step).await,
        None => Err("Body needs a \"step\"".to_string()),
    })
}

#[derive(Deserialize)]
struct NameBody {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct SlugBody {
    #[serde(default)]
    slug: String,
}

async fn verified(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(crate::services::identity::identity(&state)))
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(auth::get_identity_status(&state).await)
}

async fn display_name(
    State(state): State<AppState>,
    Json(body): Json<NameBody>,
) -> impl IntoResponse {
    ApiResult(auth::set_display_name(&state, body.name).await)
}

async fn user_slug(State(state): State<AppState>, Json(body): Json<SlugBody>) -> impl IntoResponse {
    ApiResult(auth::set_user_slug(&state, body.slug).await)
}
