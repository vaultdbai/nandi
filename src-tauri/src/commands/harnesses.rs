//! Harness commands (Tauri adapter for `services::harness`): detection,
//! health, the end-to-end test, and the default harness.

use tauri::State;

pub use crate::services::harness::{HarnessDefault, HarnessHealth, HarnessInfo, HarnessTestResult};

use crate::services::harness;
use crate::services::AppState;

/// Every known harness with its last detected health
#[tauri::command]
#[specta::specta]
pub async fn list_harnesses(state: State<'_, AppState>) -> Result<Vec<HarnessInfo>, String> {
    harness::list_harnesses(&state).await
}

/// Probes the machine again (PATH, `--version`, config folders)
#[tauri::command]
#[specta::specta]
pub async fn detect_harnesses(state: State<'_, AppState>) -> Result<Vec<HarnessInfo>, String> {
    harness::detect_harnesses(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_harness_health(
    state: State<'_, AppState>,
    harness_id: String,
) -> Result<HarnessHealth, String> {
    harness::get_harness_health(&state, &harness_id).await
}

/// Starts the CLI over ACP, opens a session, and closes it
#[tauri::command]
#[specta::specta]
pub async fn test_harness(
    state: State<'_, AppState>,
    harness_id: String,
) -> Result<HarnessTestResult, String> {
    harness::test_harness(&state, &harness_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_harness_default(
    state: State<'_, AppState>,
    harness_id: String,
    model: Option<String>,
) -> Result<HarnessDefault, String> {
    harness::set_harness_default(&state, &harness_id, model).await
}

#[tauri::command]
#[specta::specta]
pub async fn clear_harness_default(state: State<'_, AppState>) -> Result<(), String> {
    harness::clear_harness_default(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_harness_default(
    state: State<'_, AppState>,
) -> Result<Option<HarnessDefault>, String> {
    harness::get_harness_default(&state).await
}
