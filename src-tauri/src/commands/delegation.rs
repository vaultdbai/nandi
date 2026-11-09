//! Delegation commands (Tauri adapter for `services::a2a`).

use serde_json::Value;
use tauri::State;

pub use crate::services::a2a::{AgentDelegationSettings, TaskResult};

use crate::services::a2a;
use crate::services::domain::AgentCard;
use crate::services::AppState;

/// Requests that came back while nobody watched, not yet dismissed
/// (Inbox items), newest first
#[tauri::command]
#[specta::specta]
pub async fn list_request_returns(
    state: State<'_, AppState>,
) -> Result<Vec<a2a::returns::RequestReturn>, String> {
    a2a::returns::list(state.db()).await
}

/// Takes a returned request out of the Inbox
#[tauri::command]
#[specta::specta]
pub async fn dismiss_request_return(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    a2a::returns::dismiss_request_return(&state, task_id).await
}

/// Local and remote agents serving `capability`
#[tauri::command]
#[specta::specta]
pub async fn list_available_providers(
    state: State<'_, AppState>,
    capability: String,
) -> Result<Vec<AgentCard>, String> {
    a2a::list_available_providers(&state, &capability).await
}

/// Starts a delegation; returns the task ID. `target` (a card ID) skips
/// provider scoring.
#[tauri::command]
#[specta::specta]
pub async fn delegate_task(
    state: State<'_, AppState>,
    capability: String,
    input: Value,
    target: Option<String>,
) -> Result<String, String> {
    a2a::delegate_task(&state, &capability, input, target.as_deref()).await
}

/// Newest first, both directions
#[tauri::command]
#[specta::specta]
pub async fn list_delegation_history(
    state: State<'_, AppState>,
    limit: u32,
) -> Result<Vec<TaskResult>, String> {
    a2a::list_delegation_history(&state, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_delegation(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<AgentDelegationSettings, String> {
    a2a::get_agent_delegation(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_agent_delegation(
    state: State<'_, AppState>,
    agent_id: String,
    settings: AgentDelegationSettings,
) -> Result<(), String> {
    a2a::set_agent_delegation(&state, &agent_id, settings).await
}
