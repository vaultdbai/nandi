//! A tool in the unified catalog.
//!
//! IDs are dotted: `builtin.<name>` (`builtin.shell`),
//! `<connector_id>.<tool>` (`gmail.send_email`), `<pilot_id>.<tool>`. A
//! bare name in a capability's `required_tools` (`shell`) means
//! `builtin.<name>`. The runtime's own tool names (connector-prefixed,
//! pilot-prefixed) are aliases the catalog resolves.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::capability::definition::CostSpec;

/// The prefix of built-in tool IDs
pub const BUILTIN_PREFIX: &str = "builtin";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: ToolSource,
    pub input_schema: Option<serde_json::Value>,
    pub requires_approval: bool,
    pub cost_estimate: Option<CostSpec>,
    pub latency_class: LatencyClass,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolSource {
    /// shell, read_file, ...
    Builtin,
    Connector {
        connector_id: String,
    },
    Pilot {
        pilot_id: String,
    },
    /// User-provided MCP servers
    Custom {
        mcp_server: String,
    },
}

impl ToolSource {
    /// `tool_catalog.source`
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Connector { .. } => "connector",
            Self::Pilot { .. } => "pilot",
            Self::Custom { .. } => "custom",
        }
    }

    /// `tool_catalog.source_id`
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Builtin => None,
            Self::Connector { connector_id } => Some(connector_id),
            Self::Pilot { pilot_id } => Some(pilot_id),
            Self::Custom { mcp_server } => Some(mcp_server),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    Instant,
    #[default]
    Fast,
    Slow,
    Long,
}

/// A catalog entry as the UI sees it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolEntry {
    pub definition: ToolDefinition,
    pub enabled: bool,
    /// Calls recorded in the last 30 days
    pub usage_count: u32,
}

/// Per-tool usage
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolUsageStats {
    pub tool_id: String,
    pub calls: u32,
    pub failures: u32,
    pub last_used_at: Option<crate::model::Timestamp>,
}

/// `builtin.shell` for `shell`; dotted IDs unchanged
pub fn normalize_id(id: &str) -> String {
    let id = id.trim();
    if id.contains('.') {
        id.to_string()
    } else {
        format!("{BUILTIN_PREFIX}.{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_names_are_builtins() {
        assert_eq!(normalize_id("shell"), "builtin.shell");
        assert_eq!(normalize_id("gmail.send_email"), "gmail.send_email");
    }
}
