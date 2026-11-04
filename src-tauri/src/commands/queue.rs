//! Queue commands (Tauri adapter for `services::queue`).

use tauri::State;

use crate::model::{QueueEntry, RunSummary};
use crate::services::{queue, AppState};

#[tauri::command]
#[specta::specta]
pub async fn get_agent_queue(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<QueueEntry>, String> {
    queue::get_agent_queue(&state, agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_current_turn(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<RunSummary>, String> {
    queue::get_current_turn(&state, agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_queued_run(state: State<'_, AppState>, run_id: String) -> Result<(), String> {
    queue::cancel_queued_run(&state, run_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn reprioritize_queued_run(
    state: State<'_, AppState>,
    run_id: String,
    new_priority: i32,
) -> Result<(), String> {
    queue::reprioritize_queued_run(&state, run_id, new_priority).await
}
