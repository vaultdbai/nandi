//! Connector commands (Tauri adapter for `services::connectors`).

use tauri::State;

pub use crate::services::connectors::{ConnectorInfo, ConnectorStatus, DeviceFlowState};

use crate::services::connectors;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_connectors(state: State<'_, AppState>) -> Result<Vec<ConnectorInfo>, String> {
    connectors::list_connectors(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn install_connector(state: State<'_, AppState>, id: String) -> Result<(), String> {
    connectors::parse_id(&id)?;
    connectors::install_connector(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn start_connector(state: State<'_, AppState>, id: String) -> Result<(), String> {
    connectors::parse_id(&id)?;
    connectors::start_connector(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_connector(state: State<'_, AppState>, id: String) -> Result<(), String> {
    connectors::parse_id(&id)?;
    connectors::stop_connector(&state, &id).await
}

/// Replaces an installed connector's settings (never secrets) in
/// `.nandi/connectors.yaml`; a running connector stops so it restarts with them
#[tauri::command]
#[specta::specta]
pub async fn configure_connector(
    state: State<'_, AppState>,
    id: String,
    config: std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<(), String> {
    connectors::parse_id(&id)?;
    connectors::configure_connector(&state, &id, connectors::yaml_config(config)?).await
}

/// Saves one secret in the OS keyring (an empty value removes it).
/// Values are never returned.
#[tauri::command]
#[specta::specta]
pub async fn set_connector_secret(
    state: State<'_, AppState>,
    id: String,
    key: String,
    value: String,
) -> Result<(), String> {
    connectors::parse_id(&id)?;
    connectors::set_connector_secret(&state, &id, &key, &value).await
}

#[tauri::command]
#[specta::specta]
pub async fn connector_status(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectorStatus, String> {
    connectors::parse_id(&id)?;
    connectors::connector_status(&state, &id).await
}

/// Starts an OAuth2 sign-in. Device flow: show `userCode` and
/// `verificationUri`. PKCE (`userCode` empty): open `verificationUri`.
#[tauri::command]
#[specta::specta]
pub async fn start_oauth(
    state: State<'_, AppState>,
    connector_id: String,
) -> Result<DeviceFlowState, String> {
    connectors::parse_id(&connector_id)?;
    connectors::start_oauth(&state, &connector_id).await
}

/// Polls once; `true` once the user authorized
#[tauri::command]
#[specta::specta]
pub async fn poll_oauth(state: State<'_, AppState>, connector_id: String) -> Result<bool, String> {
    connectors::parse_id(&connector_id)?;
    connectors::poll_oauth(&state, &connector_id).await
}

/// The connectors an agent uses, with its shared folders
#[tauri::command]
#[specta::specta]
pub async fn get_agent_connectors(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<connectors::AgentConnectorChoice>, String> {
    connectors::agent_connectors(&state, &agent_id)
}

/// Replaces the connectors an agent uses (its skill.md `connectors:`)
#[tauri::command]
#[specta::specta]
pub async fn set_agent_connectors(
    state: State<'_, AppState>,
    agent_id: String,
    connectors: Vec<connectors::AgentConnectorChoice>,
) -> Result<(), String> {
    connectors::set_agent_connectors(&state, &agent_id, connectors).await
}

/// Which agents get each of a filesystem connector's shared folders
#[tauri::command]
#[specta::specta]
pub async fn list_folder_users(
    state: State<'_, AppState>,
    connector_id: String,
) -> Result<Vec<connectors::FolderUsers>, String> {
    connectors::parse_id(&connector_id)?;
    Ok(connectors::folder_users(&state, &connector_id))
}
