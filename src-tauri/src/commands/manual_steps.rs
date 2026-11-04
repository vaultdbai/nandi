//! Manual workflow step commands (Tauri adapter for
//! `services::workflows::manual_steps`).

use tauri::State;

pub use crate::services::workflows::manual_steps::{ManualStepCompletion, PendingManualStep};

use crate::services::workflows::manual_steps;
use crate::services::AppState;

/// Completes a manual step and resumes its workflow run
#[tauri::command]
#[specta::specta]
pub async fn complete_manual_step(
    state: State<'_, AppState>,
    run_id: String,
    step_id: String,
    completion: ManualStepCompletion,
) -> Result<(), String> {
    manual_steps::complete_manual_step(&state, &run_id, &step_id, completion).await
}

/// Manual steps waiting for someone, oldest first
#[tauri::command]
#[specta::specta]
pub async fn list_pending_manual_steps(
    state: State<'_, AppState>,
) -> Result<Vec<PendingManualStep>, String> {
    manual_steps::list_pending_manual_steps(&state).await
}
