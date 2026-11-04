//! Capability commands (Tauri adapter for `services::capability_index`).

use tauri::State;

pub use crate::services::domain::AgentCard;

use crate::services::capability_index;
use crate::services::AppState;

/// Local agents serving `capability`
#[tauri::command]
#[specta::specta]
pub async fn find_agents_by_capability(
    state: State<'_, AppState>,
    capability: String,
) -> Result<Vec<AgentCard>, String> {
    capability_index::find_agents_by_capability(&state, &capability).await
}

/// Every capability a local agent serves
#[tauri::command]
#[specta::specta]
pub async fn list_declared_capabilities(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    capability_index::list_declared_capabilities(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_card(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<AgentCard, String> {
    capability_index::get_agent_card(&state, &agent_id).await
}
