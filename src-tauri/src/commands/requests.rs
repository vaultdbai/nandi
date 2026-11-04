//! Request commands (Tauri adapter for `services::requests`).

use tauri::State;

pub use crate::services::requests::Request;

use crate::services::ask::ActionPlan;
use crate::services::requests::{self, local};
use crate::services::AppState;

/// One Request with its Steps; `id` is any of its task IDs
#[tauri::command]
#[specta::specta]
pub async fn get_request(state: State<'_, AppState>, id: String) -> Result<Request, String> {
    requests::get_request(&state, id).await
}

/// The Request a run is part of, if any
#[tauri::command]
#[specta::specta]
pub async fn request_for_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<String>, String> {
    requests::request_for_run(&state, run_id).await
}

/// A failed outgoing Request tried again on the next provider (attempt
/// n + 1, linked to the last one)
#[tauri::command]
#[specta::specta]
pub async fn retry_task(state: State<'_, AppState>, id: String) -> Result<Request, String> {
    requests::retry_task(&state, id).await
}

/// Saves an ask's approved plan as a Request and starts it; returns its ID
#[tauri::command]
#[specta::specta]
pub async fn start_request(
    state: State<'_, AppState>,
    plan: ActionPlan,
    text: String,
) -> Result<String, String> {
    local::start_request(&state, plan, text).await
}

/// Marks a step of a local Request that waits on the user done
#[tauri::command]
#[specta::specta]
pub async fn complete_request_step(
    state: State<'_, AppState>,
    id: String,
    position: u32,
) -> Result<(), String> {
    local::complete_request_step(&state, id, position).await
}

/// Stops a local Request and what its steps started
#[tauri::command]
#[specta::specta]
pub async fn cancel_request(state: State<'_, AppState>, id: String) -> Result<(), String> {
    local::cancel_request(&state, id).await
}

/// Do this again: the same plan as a new Request; returns its ID
#[tauri::command]
#[specta::specta]
pub async fn rerun_request(state: State<'_, AppState>, id: String) -> Result<String, String> {
    local::rerun_request(&state, id).await
}

/// Save as workflow: a workflow file drafted from a Request's plan, for the
/// user to confirm (`createWorkflow` validates it)
#[tauri::command]
#[specta::specta]
pub async fn draft_workflow_from_request(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    local::draft_workflow_from_request(&state, id).await
}
