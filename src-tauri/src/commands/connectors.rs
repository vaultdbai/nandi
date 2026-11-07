//! Connector commands (Tauri adapter for `services::connectors`).

use tauri::State;

pub use crate::services::connectors::{ConnectorInfo, ConnectorStatus};

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

/// Connectors in the registries (Library › Browse › Connectors)
#[tauri::command]
#[specta::specta]
pub async fn list_registry_connectors(
    state: State<'_, AppState>,
    refresh: bool,
) -> Result<Vec<crate::services::marketplace::connectors::ConnectorEntry>, String> {
    crate::services::marketplace::connectors::list(&state, refresh).await
}

/// What installing a registry connector gives, for the consent dialog
#[tauri::command]
#[specta::specta]
pub async fn get_connector_consent(
    state: State<'_, AppState>,
    registry_url: String,
    connector_id: String,
) -> Result<crate::services::marketplace::connectors::ConnectorConsent, String> {
    crate::services::marketplace::connectors::consent(&state, &registry_url, &connector_id).await
}

/// Installs (or updates) a registry connector after the dialog; the API
/// key, if any, goes to the keyring
#[tauri::command]
#[specta::specta]
pub async fn install_registry_connector(
    state: State<'_, AppState>,
    registry_url: String,
    connector_id: String,
    api_key: Option<String>,
) -> Result<(), String> {
    crate::services::marketplace::connectors::install(&state, &registry_url, &connector_id, api_key)
        .await
}

/// Removes an installed connector, its key and every grant on it
#[tauri::command]
#[specta::specta]
pub async fn remove_connector(state: State<'_, AppState>, id: String) -> Result<(), String> {
    connectors::parse_id(&id)?;
    connectors::remove_connector(&state, &id).await
}

/// Studio › New connector: a connector.md from an existing MCP server
#[tauri::command]
#[specta::specta]
pub async fn scaffold_connector(
    state: State<'_, AppState>,
    input: crate::services::studio::connector::ScaffoldConnectorInput,
) -> Result<ConnectorInfo, String> {
    crate::services::studio::connector::scaffold(&state, input).await
}

/// Publishes a connector made here to a registry
#[tauri::command]
#[specta::specta]
pub async fn publish_connector(
    state: State<'_, AppState>,
    input: crate::services::marketplace::connectors::PublishConnectorInput,
) -> Result<crate::services::marketplace::connectors::PublishConnectorResult, String> {
    crate::services::marketplace::connectors::publish(&state, input).await
}
