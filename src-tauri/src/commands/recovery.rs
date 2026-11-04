//! Recovery commands (Tauri adapter for `services::recovery`).

use serde_json::Value;
use tauri::State;

use crate::services::{recovery, AppState};
use crate::types::RecoveryError;

#[tauri::command]
#[specta::specta]
pub async fn save_emergency_data(
    state: State<'_, AppState>,
    filename: String,
    data: Value,
) -> Result<(), RecoveryError> {
    recovery::save_emergency_data(&state, filename, data).await
}

#[tauri::command]
#[specta::specta]
pub async fn load_emergency_data(
    state: State<'_, AppState>,
    filename: String,
) -> Result<Value, RecoveryError> {
    recovery::load_emergency_data(&state, filename).await
}

#[tauri::command]
#[specta::specta]
pub async fn cleanup_old_recovery_files(state: State<'_, AppState>) -> Result<u32, RecoveryError> {
    recovery::cleanup_old_recovery_files(&state).await
}
