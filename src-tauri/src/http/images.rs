//! Sandbox image routes (`/api/images/...`), mirroring `commands/images.rs`.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Router;

use super::routes::ApiResult;
use crate::services::images::tools::{self, ToolBundle, ToolsStatus};
use crate::services::images::{self, ImageStatus, SandboxImage};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `ImageStatus`
        .route("/images/{image}", get(status))
        // The advanced local build; output arrives as `images://pull`
        .route("/images/{image}/build", post(build))
        // Starts a download of the pinned image; progress as `images://pull`
        .route("/images/{image}/pull", post(pull))
        // Returns `ToolsStatus` of a first-use tool bundle
        .route("/tool-bundles/{bundle}", get(tools_status))
        // Starts its install; output arrives as `images://tools`
        .route("/tool-bundles/{bundle}/install", post(install_tools))
}

fn parse_bundle(bundle: &str) -> Result<ToolBundle, String> {
    match bundle {
        "claudeCode" => Ok(ToolBundle::ClaudeCode),
        other => Err(format!("Unknown tool bundle '{other}'")),
    }
}

async fn tools_status(Path(bundle): Path<String>) -> ApiResult<ToolsStatus> {
    ApiResult(match parse_bundle(&bundle) {
        Ok(bundle) => Ok(tools::status(bundle).await),
        Err(e) => Err(e),
    })
}

async fn install_tools(Path(bundle): Path<String>) -> ApiResult<()> {
    ApiResult(parse_bundle(&bundle).and_then(tools::start_install))
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
