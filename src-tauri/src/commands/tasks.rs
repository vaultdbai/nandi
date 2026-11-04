//! Task lifecycle commands (Tauri adapter for `services::a2a::task`).

use tauri::State;

pub use crate::services::a2a::task::{
    EtaEstimate, TaskEvent, TaskFilter, TaskMessage, TaskRecord, TaskStatusReport,
};

use crate::services::a2a::task;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_task(state: State<'_, AppState>, task_id: String) -> Result<TaskRecord, String> {
    task::get_task(&state, &task_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_tasks(
    state: State<'_, AppState>,
    filter: Option<TaskFilter>,
) -> Result<Vec<TaskRecord>, String> {
    task::list_tasks(&state, filter.unwrap_or_default()).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_task(
    state: State<'_, AppState>,
    task_id: String,
    reason: Option<String>,
) -> Result<TaskRecord, String> {
    task::cancel_task(&state, &task_id, reason).await
}

#[tauri::command]
#[specta::specta]
pub async fn query_task_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskStatusReport, String> {
    task::query_task_status(&state, &task_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_task_events(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<TaskEvent>, String> {
    task::get_task_events(&state, &task_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_task_messages(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<TaskMessage>, String> {
    task::get_task_messages(&state, &task_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_eta_estimate(
    state: State<'_, AppState>,
    capability: String,
    provider_agent: Option<String>,
) -> Result<EtaEstimate, String> {
    task::get_eta_estimate(&state, &capability, provider_agent).await
}
