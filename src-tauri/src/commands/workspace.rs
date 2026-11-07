//! Calendar and files commands (Tauri adapter for `services::workspace`).

use tauri::State;

use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::EmailSignInPoll;
use crate::services::workspace::{self, AccountKind, WorkspaceStatus};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_status(state: State<'_, AppState>) -> Result<WorkspaceStatus, String> {
    Ok(workspace::status(&state).await)
}

/// Starts connecting a calendar or drive: only its own scopes are asked for
#[tauri::command]
#[specta::specta]
pub async fn start_workspace_sign_in(
    state: State<'_, AppState>,
    kind: AccountKind,
) -> Result<DeviceFlowState, String> {
    workspace::start_sign_in(&state, kind).await
}

#[tauri::command]
#[specta::specta]
pub async fn poll_workspace_sign_in(
    state: State<'_, AppState>,
    kind: AccountKind,
) -> Result<EmailSignInPoll, String> {
    Ok(workspace::poll_sign_in(&state, kind).await)
}

#[tauri::command]
#[specta::specta]
pub async fn disconnect_workspace(
    state: State<'_, AppState>,
    kind: AccountKind,
) -> Result<WorkspaceStatus, String> {
    workspace::disconnect(&state, kind).await
}

#[tauri::command]
#[specta::specta]
pub async fn check_calendar_now(state: State<'_, AppState>) -> Result<WorkspaceStatus, String> {
    Ok(workspace::check_now(&state).await)
}

#[tauri::command]
#[specta::specta]
pub async fn set_calendar_interval(
    state: State<'_, AppState>,
    minutes: u32,
) -> Result<WorkspaceStatus, String> {
    workspace::set_calendar_interval(&state, minutes).await
}
