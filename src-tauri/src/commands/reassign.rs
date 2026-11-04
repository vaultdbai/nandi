//! Task reassignment commands (Tauri adapter for `services::a2a::reassign`).

use tauri::State;

pub use crate::services::a2a::reassign::{ReassignCandidate, ReassignRequest};
pub use crate::services::a2a::task::TaskRecord;

use crate::services::a2a::reassign;
use crate::services::AppState;

/// Hands a task we're serving to a teammate; returns our (now
/// `reassigned`) row
#[tauri::command]
#[specta::specta]
pub async fn reassign_task(
    state: State<'_, AppState>,
    request: ReassignRequest,
) -> Result<TaskRecord, String> {
    reassign::reassign_task(&state, request).await
}

/// Online teammates who serve the task's capability
#[tauri::command]
#[specta::specta]
pub async fn list_reassign_candidates(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<ReassignCandidate>, String> {
    reassign::reassign_candidates(&state, &task_id).await
}
