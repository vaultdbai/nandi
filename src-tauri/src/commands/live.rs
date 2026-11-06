//! Live UI commands (Tauri adapter for `services::live`).

use tauri::State;

pub use crate::services::live::{LiveChange, RefreshReport};

use crate::services::live;
use crate::services::AppState;

/// The latest file changes, newest first: who changed what
#[tauri::command]
#[specta::specta]
pub fn list_recent_changes(state: State<'_, AppState>) -> Result<Vec<LiveChange>, String> {
    Ok(state.live().feed().recent())
}

/// Re-reads every definition from disk (the Refresh button, F5)
#[tauri::command]
#[specta::specta]
pub async fn refresh_from_disk(state: State<'_, AppState>) -> Result<RefreshReport, String> {
    live::refresh_from_disk(&state).await
}
