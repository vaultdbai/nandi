//! Global run commands (Tauri adapter for `services::runs`).

use tauri::State;

use crate::services::runs;
pub use crate::services::runs::{DateRange, RunFilters, RunPage, RunStats};
use crate::services::AppState;

/// Runs across every agent, newest first
#[tauri::command]
#[specta::specta]
pub async fn list_all_runs(
    state: State<'_, AppState>,
    filters: RunFilters,
    limit: u32,
    offset: u32,
) -> Result<RunPage, String> {
    runs::list_all_runs(state.db(), filters, limit, offset).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_run_stats(
    state: State<'_, AppState>,
    range: DateRange,
) -> Result<RunStats, String> {
    runs::get_run_stats(state.db(), range).await
}
