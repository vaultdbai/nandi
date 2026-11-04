//! Work commands (Tauri adapter for `services::work`).

use tauri::State;

pub use crate::services::work::{WorkItem, WorkState};

use crate::services::work;
use crate::services::AppState;

/// Workflow runs and tasks, newest first; `state_filter` `None` for all
#[tauri::command]
#[specta::specta]
pub async fn list_work(
    state: State<'_, AppState>,
    state_filter: Option<WorkState>,
) -> Result<Vec<WorkItem>, String> {
    work::list_work(&state, state_filter).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_work_item(state: State<'_, AppState>, id: String) -> Result<WorkItem, String> {
    work::get_work_item(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn pause_work(state: State<'_, AppState>, id: String) -> Result<(), String> {
    work::pause_work(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn resume_work(state: State<'_, AppState>, id: String) -> Result<(), String> {
    work::resume_work(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_work(state: State<'_, AppState>, id: String) -> Result<(), String> {
    work::cancel_work(&state, id).await
}

/// Rates a finished delegated task 1–5
#[tauri::command]
#[specta::specta]
pub async fn rate_work(state: State<'_, AppState>, id: String, rating: u32) -> Result<(), String> {
    work::rate_work(&state, id, rating).await
}

/// Hides a failed item from the Inbox (it stays in history)
#[tauri::command]
#[specta::specta]
pub async fn dismiss_work(state: State<'_, AppState>, id: String) -> Result<(), String> {
    work::dismiss_work(&state, id).await
}

/// IDs of failures dismissed from the Inbox
#[tauri::command]
#[specta::specta]
pub async fn list_dismissed_work(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    work::list_dismissed_work(&state).await
}
