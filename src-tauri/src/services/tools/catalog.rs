//! The in-memory tool index, built from `tool_catalog`:
//! lookup by ID (or alias), by source, by tag.

use std::collections::HashMap;

use super::definition::{normalize_id, ToolDefinition, ToolEntry, ToolSource};
use crate::services::connectors::prefixed_name;
use crate::services::pilots::tools::tool_name as pilot_tool_name;

#[derive(Debug, Clone, Default)]
pub struct ToolCatalog {
    by_id: HashMap<String, ToolEntry>,
    /// Runtime tool name (`shell`, `gmail__send_email`,
    /// `pilot_opendesk__click`) -> catalog ID
    aliases: HashMap<String, String>,
}

/// The name the model sees for a tool, which the catalog also resolves:
/// the bare name for built-ins, the prefixed name for connector and pilot
/// tools. `None` when the runtime has no such name.
pub fn runtime_name(definition: &ToolDefinition) -> Option<String> {
    match &definition.source {
        ToolSource::Builtin => Some(definition.name.clone()),
        ToolSource::Connector { connector_id } => prefixed_name(connector_id, &definition.name),
        ToolSource::Pilot { pilot_id } => Some(pilot_tool_name(pilot_id, &definition.name)),
        ToolSource::Custom { .. } => None,
    }
}

impl ToolCatalog {
    pub fn from_entries(entries: Vec<ToolEntry>) -> Self {
        let mut catalog = Self::default();
        for entry in entries {
            if let Some(alias) = runtime_name(&entry.definition) {
                catalog
                    .aliases
                    .entry(alias)
                    .or_insert_with(|| entry.definition.id.clone());
            }
            catalog
                .by_id
                .entry(entry.definition.id.clone())
                .or_insert(entry);
        }
        catalog
    }

    /// By ID, a bare built-in name, or a runtime alias
    pub fn get(&self, id: &str) -> Option<&ToolEntry> {
        let id = id.trim();
        self.by_id
            .get(id)
            .or_else(|| self.by_id.get(&normalize_id(id)))
            .or_else(|| self.aliases.get(id).and_then(|id| self.by_id.get(id)))
    }

    /// Sorted by ID
    pub fn all(&self) -> Vec<&ToolEntry> {
        let mut all: Vec<_> = self.by_id.values().collect();
        all.sort_by(|a, b| a.definition.id.cmp(&b.definition.id));
        all
    }

    /// `source` is a `ToolSource::kind`; sorted by ID
    #[cfg(test)]
    pub fn by_source(&self, source: &str) -> Vec<&ToolDefinition> {
        self.definitions(|d| d.source.kind() == source)
    }

    /// Sorted by ID
    #[cfg(test)]
    pub fn by_tag(&self, tag: &str) -> Vec<&ToolDefinition> {
        self.definitions(|d| d.tags.iter().any(|t| t == tag))
    }

    /// The IDs in `required` the catalog doesn't have (disabled tools count
    /// as present: they exist, the user turned them off)
    pub fn missing<'a>(&self, required: &'a [String]) -> Vec<&'a str> {
        required
            .iter()
            .map(String::as_str)
            .filter(|id| !id.trim().is_empty() && self.get(id).is_none())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[cfg(test)]
    fn definitions(&self, keep: impl Fn(&ToolDefinition) -> bool) -> Vec<&ToolDefinition> {
        self.all()
            .into_iter()
            .map(|e| &e.definition)
            .filter(|d| keep(d))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::builtin::builtin_tools;
    use super::super::definition::LatencyClass;
    use super::*;

    fn entry(definition: ToolDefinition) -> ToolEntry {
        ToolEntry {
            definition,
            enabled: true,
            usage_count: 0,
        }
    }

    fn connector_tool(connector: &str, tool: &str) -> ToolDefinition {
        ToolDefinition {
            id: format!("{connector}.{tool}"),
            name: tool.to_string(),
            description: String::new(),
            source: ToolSource::Connector {
                connector_id: connector.to_string(),
            },
            input_schema: None,
            requires_approval: false,
            cost_estimate: None,
            latency_class: LatencyClass::Fast,
            tags: vec!["connector".into()],
        }
    }

    fn catalog() -> ToolCatalog {
        let mut entries: Vec<_> = builtin_tools().into_iter().map(entry).collect();
        entries.push(entry(connector_tool("gmail", "send_email")));
        ToolCatalog::from_entries(entries)
    }

    #[test]
    fn resolves_ids_bare_names_and_runtime_aliases() {
        let catalog = catalog();
        assert_eq!(
            catalog.get("builtin.shell").unwrap().definition.name,
            "shell"
        );
        assert_eq!(catalog.get("shell").unwrap().definition.id, "builtin.shell");
        assert_eq!(
            catalog.get("gmail__send_email").unwrap().definition.id,
            "gmail.send_email"
        );
        assert!(catalog.get("nope").is_none());
    }

    #[test]
    fn filters_by_source_and_tag() {
        let catalog = catalog();
        assert_eq!(catalog.by_source("connector")[0].id, "gmail.send_email");
        assert!(catalog
            .by_source("builtin")
            .iter()
            .all(|d| d.source == ToolSource::Builtin));
        assert!(catalog
            .by_tag("filesystem")
            .iter()
            .any(|d| d.id == "builtin.read_file"));
    }

    #[test]
    fn reports_only_the_missing_tools() {
        let required = vec![
            "shell".to_string(),
            "gmail.send_email".to_string(),
            "slack.post".to_string(),
        ];
        assert_eq!(catalog().missing(&required), ["slack.post"]);
    }
}
