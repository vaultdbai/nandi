//! Gateway commands (Tauri adapter for `services::gateway`).

use tauri::State;

pub use crate::services::gateway::{
    CallerPolicy, GatewayConfig, GatewayManifest, GatewayStatus, GatewayUsage,
};

use crate::services::gateway;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_gateway_status(state: State<'_, AppState>) -> Result<GatewayStatus, String> {
    gateway::get_gateway_status(&state).await
}

/// Requests waiting for the owner's approval, oldest first (the Inbox)
#[tauri::command]
#[specta::specta]
pub async fn list_gateway_holds(
    state: State<'_, AppState>,
) -> Result<Vec<gateway::holds::GatewayHold>, String> {
    gateway::list_gateway_holds(&state).await
}

/// Approve (routing goes on) or decline (it's rejected) a held request
#[tauri::command]
#[specta::specta]
pub async fn decide_gateway_hold(
    state: State<'_, AppState>,
    task_id: String,
    approve: bool,
) -> Result<gateway::holds::GatewayHold, String> {
    gateway::decide_gateway_hold(&state, task_id, approve).await
}

/// Work assigned to the owner that's still open, soonest due first (the
/// Inbox)
#[tauri::command]
#[specta::specta]
pub async fn list_gateway_assignments(
    state: State<'_, AppState>,
) -> Result<Vec<gateway::assignments::GatewayAssignment>, String> {
    gateway::list_gateway_assignments(&state).await
}

/// Mark an assignment Done (the requester gets the note) or Decline it
#[tauri::command]
#[specta::specta]
pub async fn close_gateway_assignment(
    state: State<'_, AppState>,
    task_id: String,
    done: bool,
    note: Option<String>,
) -> Result<gateway::assignments::GatewayAssignment, String> {
    gateway::close_gateway_assignment(&state, task_id, done, note).await
}

#[tauri::command]
#[specta::specta]
pub async fn make_gateway_reachable(state: State<'_, AppState>) -> Result<GatewayStatus, String> {
    gateway::make_gateway_reachable(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_gateway_config(state: State<'_, AppState>) -> Result<GatewayConfig, String> {
    gateway::get_gateway_config(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_gateway_config(
    state: State<'_, AppState>,
    config: GatewayConfig,
) -> Result<GatewayConfig, String> {
    gateway::set_gateway_config(&state, config).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_gateway_callers(state: State<'_, AppState>) -> Result<Vec<CallerPolicy>, String> {
    gateway::list_gateway_callers(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_caller_policy(
    state: State<'_, AppState>,
    policy: CallerPolicy,
) -> Result<CallerPolicy, String> {
    gateway::set_caller_policy(&state, policy).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_caller_policy(
    state: State<'_, AppState>,
    caller_instance: String,
) -> Result<(), String> {
    gateway::remove_caller_policy(&state, &caller_instance).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_gateway_usage(
    state: State<'_, AppState>,
    caller_instance: String,
) -> Result<GatewayUsage, String> {
    gateway::get_gateway_usage(&state, &caller_instance).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_gateway_manifest(state: State<'_, AppState>) -> Result<GatewayManifest, String> {
    gateway::get_gateway_manifest(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_gateway_manifest(
    state: State<'_, AppState>,
    manifest: GatewayManifest,
) -> Result<GatewayManifest, String> {
    gateway::set_gateway_manifest(&state, manifest).await
}

/// Ask the gateway about this instance (capabilities, queue, who to ask).
/// It answers; it never runs work.
#[tauri::command]
#[specta::specta]
pub async fn gateway_chat(state: State<'_, AppState>, message: String) -> Result<String, String> {
    gateway::gateway_chat(&state, &message).await
}
