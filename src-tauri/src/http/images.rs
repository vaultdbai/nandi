//! Sandbox image routes (`/api/images/...`), mirroring `commands/images.rs`.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Router;

use super::routes::ApiResult;
use crate::services::images::{self, ImageStatus, SandboxImage};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `ImageStatus`
        .route("/images/{image}", get(status))
        // Starts a build; progress arrives as `images://build`
        .route("/images/{image}/build", post(build))
        // Starts a download of the pinned image; progress as `images://build`
        .route("/images/{image}/pull", post(pull))
}

fn parse(image: &str) -> Result<SandboxImage, String> {
    match image {
        "harness" => Ok(SandboxImage::Harness),
        "runtime" => Ok(SandboxImage::Runtime),
        other => Err(format!("Unknown image '{other}'")),
    }
}

async fn status(Path(image): Path<String>) -> ApiResult<ImageStatus> {
    ApiResult(match parse(&image) {
        Ok(image) => Ok(images::image_status(image).await),
        Err(e) => Err(e),
    })
}

async fn build(State(state): State<AppState>, Path(image): Path<String>) -> ApiResult<()> {
    ApiResult(parse(&image).and_then(|image| images::start_build(&state, image)))
}

async fn pull(State(state): State<AppState>, Path(image): Path<String>) -> ApiResult<()> {
    ApiResult(parse(&image).and_then(|image| images::start_download(&state, image)))
}
