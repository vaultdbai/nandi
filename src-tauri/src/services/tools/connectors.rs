//! Connector tools (wave A, Track C): one definition per MCP tool of each
//! installed connector (`services::connectors`), `ToolSource::Connector`.
//! A running connector reports its live tools; a stopped one its static
//! list. Refreshed when the installs or the running set change.

use super::definition::{LatencyClass, ToolDefinition, ToolSource};
use crate::services::connectors::{self, ConnectorInfo};
use crate::services::AppState;

pub async fn connector_tools(state: &AppState) -> Vec<ToolDefinition> {
    match connectors::list_connectors(state).await {
        Ok(infos) => connector_definitions(&infos),
        Err(e) => {
            log::warn!("Tool catalog: connectors unavailable ({e})");
            Vec::new()
        }
    }
}

/// The tools of the installed connectors in `infos`
pub fn connector_definitions(infos: &[ConnectorInfo]) -> Vec<ToolDefinition> {
    infos
        .iter()
        .filter(|info| info.status.installed)
        .flat_map(|info| {
            info.tools.iter().map(|tool| ToolDefinition {
                id: format!("{}.{}", info.id, tool.name),
                name: tool.name.clone(),
                description: tool.description.clone(),
                source: ToolSource::Connector {
                    connector_id: info.id.clone(),
                },
                input_schema: None,
                requires_approval: tool.sensitive,
                cost_estimate: None,
                latency_class: LatencyClass::Fast,
                tags: vec!["connector".to_string(), info.kind.clone()],
            })
        })
        .collect()
}

/// What decides the connector tools without touching the keyring: which
/// connectors are installed and which are running. Cheap enough to poll.
pub fn fingerprint(state: &AppState) -> Vec<(String, bool)> {
    let manifest = state.store().read(|repo| repo.connectors().clone());
    let mut seen: Vec<(String, bool)> = manifest
        .connectors()
        .iter()
        .map(|decl| {
            let id = decl.id.as_str().to_string();
            let running = state.connectors().is_some_and(|r| r.is_running(&id));
            (id, running)
        })
        .collect();
    seen.sort();
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::connectors::{ConnectorStatus, ConnectorToolInfo};

    fn info(id: &str, installed: bool, tools: &[(&str, bool)]) -> ConnectorInfo {
        ConnectorInfo {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            kind: "mcp".to_string(),
            status: ConnectorStatus {
                installed,
                ..ConnectorStatus::default()
            },
            tools: tools
                .iter()
                .map(|(name, sensitive)| ConnectorToolInfo {
                    name: name.to_string(),
                    description: format!("{name} tool"),
                    sensitive: *sensitive,
                })
                .collect(),
            config: Default::default(),
            secrets: Vec::new(),
            setup_hint: None,
            package: None,
        }
    }

    #[test]
    fn only_installed_connectors_contribute_tools() {
        let infos = [
            info(
                "filesystem",
                true,
                &[("read_file", false), ("write_file", true)],
            ),
            info("web_search", false, &[("search", false)]),
        ];
        let tools = connector_definitions(&infos);
        let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["filesystem.read_file", "filesystem.write_file"]);
        assert_eq!(
            tools[1].source,
            ToolSource::Connector {
                connector_id: "filesystem".into()
            }
        );
        assert!(!tools[0].requires_approval && tools[1].requires_approval);
    }

    #[test]
    fn installing_a_connector_adds_its_tools() {
        let before = connector_definitions(&[info("web_search", false, &[("search", false)])]);
        let after = connector_definitions(&[info("web_search", true, &[("search", false)])]);
        assert!(before.is_empty());
        assert_eq!(after[0].id, "web_search.search");
    }
}
