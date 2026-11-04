//! Preferences commands (Tauri adapter for `services::preferences`).

use tauri::State;

use crate::services::{preferences, AppState};
use crate::types::AppPreferences;

#[tauri::command]
#[specta::specta]
pub async fn load_preferences(state: State<'_, AppState>) -> Result<AppPreferences, String> {
    preferences::load_preferences(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_preferences(
    state: State<'_, AppState>,
    preferences: AppPreferences,
) -> Result<(), String> {
    preferences::save_preferences(&state, preferences).await
}
