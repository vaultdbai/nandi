//! Pilot commands (Tauri adapter for `services::pilots`).

use tauri::State;

pub use crate::services::pilots::{AgentPilotPermission, PilotDefinition, PilotStatus};

use crate::services::pilots;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_pilots() -> Result<Vec<PilotDefinition>, String> {
    Ok(pilots::list_pilots())
}

#[tauri::command]
#[specta::specta]
pub async fn install_pilot(state: State<'_, AppState>, id: String) -> Result<(), String> {
    pilots::install_pilot(&state, &id).await
}

/// Returns the pilot's process ID
#[tauri::command]
#[specta::specta]
pub async fn start_pilot(state: State<'_, AppState>, id: String) -> Result<u32, String> {
    pilots::start_pilot(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_pilot(state: State<'_, AppState>, id: String) -> Result<(), String> {
    pilots::stop_pilot(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn pilot_status(state: State<'_, AppState>, id: String) -> Result<PilotStatus, String> {
    pilots::pilot_status(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn grant_pilot_capability(
    state: State<'_, AppState>,
    agent_id: String,
    pilot_id: String,
    capability: String,
) -> Result<(), String> {
    pilots::grant_pilot_capability(&state, &agent_id, &pilot_id, &capability).await
}

#[tauri::command]
#[specta::specta]
pub async fn revoke_pilot_capability(
    state: State<'_, AppState>,
    agent_id: String,
    pilot_id: String,
    capability: String,
) -> Result<(), String> {
    pilots::revoke_pilot_capability(&state, &agent_id, &pilot_id, &capability).await
}

/// Saves (or, empty, removes) a pilot secret the agent's skill.md names
#[tauri::command]
#[specta::specta]
pub async fn set_pilot_secret(
    state: State<'_, AppState>,
    agent_id: String,
    pilot_id: String,
    name: String,
    value: String,
) -> Result<(), String> {
    pilots::set_pilot_secret(&state, &agent_id, &pilot_id, &name, &value).await
}

/// Adds a site secret to the agent's browser: the name into skill.md, the
/// value into the keychain
#[tauri::command]
#[specta::specta]
pub async fn add_pilot_secret(
    state: State<'_, AppState>,
    agent_id: String,
    pilot_id: String,
    name: String,
    value: String,
) -> Result<(), String> {
    pilots::add_pilot_secret(&state, &agent_id, &pilot_id, &name, &value).await
}

/// "Sign out everywhere": clears the agent's browser profile. `false`: it
/// had none
#[tauri::command]
#[specta::specta]
pub async fn clear_browser_profile(
    state: State<'_, AppState>,
    agent_id: String,
    pilot_id: String,
) -> Result<bool, String> {
    pilots::clear_browser_profile(&state, &agent_id, &pilot_id).await
}

/// The pilots the agent's skill.md declares, and what the user granted each
#[tauri::command]
#[specta::specta]
pub async fn agent_pilot_permissions(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<AgentPilotPermission>, String> {
    pilots::agent_pilot_permissions(&state, &agent_id).await
}
