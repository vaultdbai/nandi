//! Agent commands (Tauri adapter for `services::agents`).

use tauri::State;

use crate::services::agents::{self, AgentInfo, CreateAgentInput};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    agents::list_agents(&state).await
}

/// Each agent's setup and its last seven days; `today_start` is local
/// midnight, epoch ms
#[tauri::command]
#[specta::specta]
pub async fn list_agent_overviews(
    state: State<'_, AppState>,
    today_start: f64,
) -> Result<Vec<crate::services::agent_overview::AgentOverview>, String> {
    crate::services::agent_overview::list_agent_overviews(&state, today_start).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_agent(
    state: State<'_, AppState>,
    input: CreateAgentInput,
) -> Result<AgentInfo, String> {
    agents::create_agent(&state, input).await
}

#[tauri::command]
#[specta::specta]
pub async fn start_agent(state: State<'_, AppState>, agent_id: String) -> Result<String, String> {
    agents::start_agent(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_agent(state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    agents::stop_agent(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn restart_agent(state: State<'_, AppState>, agent_id: String) -> Result<String, String> {
    agents::restart_agent(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_agent(state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    agents::delete_agent(&state, &agent_id).await
}
