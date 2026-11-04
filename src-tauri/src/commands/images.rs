//! Sandbox image commands (Tauri adapter for `services::images`).

use tauri::State;

use crate::services::images::{self, ImageStatus, SandboxImage};
use crate::services::AppState;

/// Whether `image` is built, Docker is running, and how a build is going
#[tauri::command]
#[specta::specta]
pub async fn get_image_status(image: SandboxImage) -> Result<ImageStatus, String> {
    Ok(images::image_status(image).await)
}

/// Builds `image` in the background; progress arrives as `images://build`
#[tauri::command]
#[specta::specta]
pub async fn build_image(state: State<'_, AppState>, image: SandboxImage) -> Result<(), String> {
    images::start_build(&state, image)
}

/// Downloads `image` (pinned to this app's sandbox version) in the
/// background; progress arrives as `images://build`
#[tauri::command]
#[specta::specta]
pub async fn pull_image(state: State<'_, AppState>, image: SandboxImage) -> Result<(), String> {
    images::start_download(&state, image)
}
