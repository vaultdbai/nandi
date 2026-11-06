//! Sandbox image commands (Tauri adapter for `services::images`).

use tauri::State;

use crate::services::images::tools::{self, ToolBundle, ToolsStatus};
use crate::services::images::{self, ImageStatus, SandboxImage};
use crate::services::AppState;

/// Whether `image` is here and current, its download size, and how a
/// download (or the advanced local build) is going
#[tauri::command]
#[specta::specta]
pub async fn get_image_status(image: SandboxImage) -> Result<ImageStatus, String> {
    Ok(images::image_status(image).await)
}

/// The advanced option: builds `image` here in the background; output
/// arrives as `images://pull`. Nothing requires it
#[tauri::command]
#[specta::specta]
pub async fn build_image(state: State<'_, AppState>, image: SandboxImage) -> Result<(), String> {
    images::start_build(&state, image)
}

/// Downloads `image` (pinned to this app's sandbox version) in the
/// background; progress arrives as `images://pull`
#[tauri::command]
#[specta::specta]
pub async fn pull_image(state: State<'_, AppState>, image: SandboxImage) -> Result<(), String> {
    images::start_download(&state, image)
}

/// Whether a first-use tool bundle (Claude Code) is installed, and how its
/// install is going
#[tauri::command]
#[specta::specta]
pub async fn get_tools_status(bundle: ToolBundle) -> Result<ToolsStatus, String> {
    Ok(tools::status(bundle).await)
}

/// Gets a first-use tool bundle now (or again, after a failure) in the
/// background; output arrives as `images://tools`
#[tauri::command]
#[specta::specta]
pub async fn install_tools(bundle: ToolBundle) -> Result<(), String> {
    tools::start_install(bundle)
}
