//! Connector commands (Tauri adapter for `services::connectors`).

use crate::services::connectors;
use crate::services::connectors::{ConnectorInfo, ConnectorStatus};

super::commands! {
    fn list_connectors(state) -> Vec<ConnectorInfo> => connectors::list_connectors(&state).await;

    fn install_connector(state, id: String) -> () => connectors::install_connector(&state, &id).await;

    fn start_connector(state, id: String) -> () => connectors::start_connector(&state, &id).await;

    fn stop_connector(state, id: String) -> () => connectors::stop_connector(&state, &id).await;

    /// Replaces an installed connector's settings (never secrets) in
    /// `.nandi/connectors.yaml`; a running connector stops so it restarts with them
    fn configure_connector(state, id: String, config: std::collections::BTreeMap<String, serde_json::Value>) -> () => connectors::configure_connector(&state, &id, connectors::yaml_config(config)?).await;

    /// Saves one secret in the OS keyring (an empty value removes it).
    /// Values are never returned.
    fn set_connector_secret(state, id: String, key: String, value: String) -> () => connectors::set_connector_secret(&state, &id, &key, &value).await;

    fn connector_status(state, id: String) -> ConnectorStatus => connectors::connector_status(&state, &id).await;

    /// The connectors an agent uses, with its shared folders
    fn get_agent_connectors(state, agent_id: String) -> Vec<connectors::AgentConnectorChoice> => connectors::agent_connectors(&state, &agent_id);

    /// Replaces the connectors an agent uses (its skill.md `connectors:`)
    fn set_agent_connectors(state, agent_id: String, connectors: Vec<connectors::AgentConnectorChoice>) -> () => connectors::set_agent_connectors(&state, &agent_id, connectors).await;

    /// Which agents get each of a filesystem connector's shared folders
    fn list_folder_users(state, connector_id: String) -> Vec<connectors::FolderUsers> => connectors::folder_users(&state, &connector_id);

    /// Connectors in the registries (Library › Browse › Connectors)
    fn list_registry_connectors(state, refresh: bool) -> Vec<crate::services::marketplace::connectors::ConnectorEntry> => crate::services::marketplace::connectors::list(&state, refresh).await;

    /// What installing a registry connector gives, for the consent dialog
    fn get_connector_consent(state, registry_url: String, connector_id: String) -> crate::services::marketplace::connectors::ConnectorConsent => crate::services::marketplace::connectors::consent(&state, &registry_url, &connector_id).await;

    /// Installs (or updates) a registry connector after the dialog; the API
    /// key, if any, goes to the keyring
    fn install_registry_connector(state, registry_url: String, connector_id: String, api_key: Option<String>) -> () => crate::services::marketplace::connectors::install(&state, &registry_url, &connector_id, api_key).await;

    /// Removes an installed connector, its key and every grant on it
    fn remove_connector(state, id: String) -> () => connectors::remove_connector(&state, &id).await;

    /// Studio › New connector: a connector.md from an existing MCP server
    fn scaffold_connector(state, input: crate::services::studio::connector::ScaffoldConnectorInput) -> ConnectorInfo => crate::services::studio::connector::scaffold(&state, input).await;

    /// Publishes a connector made here to a registry
    fn publish_connector(state, input: crate::services::marketplace::connectors::PublishConnectorInput) -> crate::services::marketplace::connectors::PublishConnectorResult => crate::services::marketplace::connectors::publish(&state, input).await;
}
