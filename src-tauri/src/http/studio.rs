//! Studio artifact routes (`/api/studio/...`), mirroring
//! `commands/studio.rs`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::ask::ArtifactRef;
use crate::services::studio::{self, ArtifactSave};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Body `{ artifact }`; returns `ArtifactLoad`
        .route("/studio/load", post(load))
        // Body `ArtifactSave`; returns `ArtifactSaveResult`
        .route("/studio/save", post(save))
        // Body `{ artifact, instruction }`; returns `EditPreview`
        .route("/studio/preview", post(preview))
        // Returns `PushTargetOption[]`
        .route("/studio/push-targets", get(push_targets))
        // Body `{ artifact, content }`; returns `ArtifactValidation`
        .route("/studio/validate", post(validate))
        // Body `{ artifact }` (`null`: nothing open)
        .route("/studio/open", post(open))
}

#[derive(Deserialize)]
struct OpenBody {
    artifact: Option<ArtifactRef>,
}

async fn open(State(state): State<AppState>, Json(body): Json<OpenBody>) -> impl IntoResponse {
    ApiResult(studio::set_open_artifact(&state, body.artifact))
}

#[derive(Deserialize)]
struct ValidateBody {
    artifact: ArtifactRef,
    content: String,
}

async fn validate(
    State(state): State<AppState>,
    Json(body): Json<ValidateBody>,
) -> impl IntoResponse {
    ApiResult(studio::validate_artifact(&state, body.artifact, body.content).await)
}

#[derive(Deserialize)]
struct LoadBody {
    artifact: ArtifactRef,
}

#[derive(Deserialize)]
struct PreviewBody {
    artifact: ArtifactRef,
    instruction: String,
}

async fn load(State(state): State<AppState>, Json(body): Json<LoadBody>) -> impl IntoResponse {
    ApiResult(studio::load_artifact(&state, body.artifact).await)
}

async fn save(State(state): State<AppState>, Json(body): Json<ArtifactSave>) -> impl IntoResponse {
    ApiResult(studio::save_artifact(&state, body).await)
}

async fn preview(
    State(state): State<AppState>,
    Json(body): Json<PreviewBody>,
) -> impl IntoResponse {
    ApiResult(studio::preview_edit(&state, body.artifact, body.instruction).await)
}

async fn push_targets(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(studio::list_push_targets(&state).await)
}
