//! Git remote commands (Tauri adapter for `services::remote`). The
//! credential goes in once and never comes back out.

use tauri::State;

pub use crate::services::remote::{PullResult, PushResult, RemoteCredential, RemoteStatus};

use crate::services::remote;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_remote_status(state: State<'_, AppState>) -> Result<RemoteStatus, String> {
    remote::get_remote_status(&state).await
}

/// Pushes go to `<prefix>/<slug>` only (default prefix `nandi`)
#[tauri::command]
#[specta::specta]
pub async fn configure_remote(
    state: State<'_, AppState>,
    url: String,
    credential: Option<RemoteCredential>,
    prefix: Option<String>,
    slug: Option<String>,
) -> Result<RemoteStatus, String> {
    remote::configure_remote(&state, url, credential, prefix, slug).await
}

#[tauri::command]
#[specta::specta]
pub async fn clear_remote(state: State<'_, AppState>) -> Result<(), String> {
    remote::clear_remote(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn test_remote_connection(state: State<'_, AppState>) -> Result<(), String> {
    remote::test_remote_connection(&state).await
}

/// Never to main or master. `overwrite` force-pushes `<prefix>/<slug>`.
#[tauri::command]
#[specta::specta]
pub async fn push_to_remote(
    state: State<'_, AppState>,
    overwrite: bool,
) -> Result<PushResult, String> {
    remote::push_to_remote(&state, overwrite).await
}

#[tauri::command]
#[specta::specta]
pub async fn pull_from_remote(state: State<'_, AppState>) -> Result<PullResult, String> {
    remote::pull_from_remote(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_remote_branches(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    remote::list_remote_branches(&state).await
}
