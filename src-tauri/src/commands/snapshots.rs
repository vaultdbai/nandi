//! Snapshot commands (Tauri adapter for `services::snapshots`).

use tauri::State;

use crate::model::{StudioConversationSummary, WorkflowSummary};
use crate::services::snapshots::{
    self, AgentSnapshot, ConversationSnapshot, DashboardSnapshot, RunDetail,
};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_agent_snapshot(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<AgentSnapshot, String> {
    snapshots::get_agent_snapshot(&state, agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<ConversationSnapshot, String> {
    snapshots::get_conversation(&state, conversation_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_studio_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<StudioConversationSummary>, String> {
    snapshots::get_studio_conversations(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_workflows(state: State<'_, AppState>) -> Result<Vec<WorkflowSummary>, String> {
    snapshots::get_workflows(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_run_detail(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<RunDetail, String> {
    snapshots::get_run_detail(&state, run_id).await
}

/// The host path of a file a run touched, if it exists
#[tauri::command]
#[specta::specta]
pub async fn resolve_run_file(
    state: State<'_, AppState>,
    run_id: String,
    path: String,
) -> Result<Option<String>, String> {
    snapshots::run_file_path(&state, run_id, path).await
}

/// Opens a file a run touched with its default app, or shows it in the
/// file manager (`reveal`)
#[tauri::command]
#[specta::specta]
pub async fn open_run_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    path: String,
    reveal: bool,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let file = snapshots::run_file_path(&state, run_id, path)
        .await?
        .ok_or("That file isn't on this computer anymore")?;
    if reveal {
        app.opener().reveal_item_in_dir(&file)
    } else {
        app.opener().open_path(file, None::<&str>)
    }
    .map_err(|e| format!("Couldn't open the file: {e}"))
}

#[tauri::command]
#[specta::specta]
pub async fn get_dashboard_snapshot(
    state: State<'_, AppState>,
    day_start: f64,
) -> Result<DashboardSnapshot, String> {
    snapshots::get_dashboard_snapshot(&state, day_start).await
}
