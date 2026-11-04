//! Trigger commands (Tauri adapter for `services::triggers`).

use serde_json::Value;
use tauri::State;

use crate::services::triggers::schema::{SavedTrigger, Trigger, TriggerFire, TriggerInput};
use crate::services::{triggers, AppState};

#[tauri::command]
#[specta::specta]
pub async fn list_triggers(state: State<'_, AppState>) -> Result<Vec<Trigger>, String> {
    triggers::list_triggers(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_trigger(state: State<'_, AppState>, id: String) -> Result<Trigger, String> {
    triggers::get_trigger(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_trigger(
    state: State<'_, AppState>,
    input: TriggerInput,
) -> Result<SavedTrigger, String> {
    triggers::create_trigger(&state, input).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_trigger(
    state: State<'_, AppState>,
    id: String,
    input: TriggerInput,
) -> Result<SavedTrigger, String> {
    triggers::update_trigger(&state, id, input).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_trigger(state: State<'_, AppState>, id: String) -> Result<(), String> {
    triggers::delete_trigger(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_trigger_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    triggers::set_trigger_enabled(&state, id, enabled).await
}

#[tauri::command]
#[specta::specta]
pub async fn rotate_trigger_secret(
    state: State<'_, AppState>,
    id: String,
) -> Result<SavedTrigger, String> {
    triggers::rotate_trigger_secret(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn test_fire_trigger(
    state: State<'_, AppState>,
    id: String,
    payload: Option<Value>,
) -> Result<TriggerFire, String> {
    triggers::test_fire_trigger(&state, id, payload).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_trigger_fires(
    state: State<'_, AppState>,
    trigger_id: String,
    limit: Option<u32>,
) -> Result<Vec<TriggerFire>, String> {
    triggers::list_trigger_fires(&state, trigger_id, limit).await
}

/// Whether a calendar is valid and its next 5 runs with who is on duty
#[tauri::command]
#[specta::specta]
pub async fn preview_calendar(
    config: serde_json::Value,
) -> Result<triggers::calendar::CalendarPreview, String> {
    Ok(triggers::calendar::preview(
        &config,
        chrono::Local::now(),
        triggers::calendar::PREVIEW_COUNT,
    ))
}

/// Whether a cron schedule is valid and its next 3 runs (the scheduler's
/// own parser, so they match)
#[tauri::command]
#[specta::specta]
pub async fn preview_cron(schedule: String) -> Result<triggers::cron::CronPreview, String> {
    Ok(triggers::cron::preview(
        &schedule,
        chrono::Local::now(),
        triggers::cron::PREVIEW_COUNT,
    ))
}

/// Where webhook and device requests go on this machine, e.g.
/// `http://127.0.0.1:17463` (`None` until the web server is listening).
#[tauri::command]
#[specta::specta]
pub async fn get_web_origin(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state
        .http_port()
        .map(|port| format!("http://127.0.0.1:{port}")))
}
