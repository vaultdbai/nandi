//! Tool catalog commands (Tauri adapter for `services::tools`).

use crate::services::tools;
use crate::services::tools::{ToolEntry, ToolUsageStats};

super::commands! {
    /// Built-in, connector, and pilot tools
    fn list_tools(state) -> Vec<ToolEntry> => tools::list_tools(&state).await;

    fn get_tool(state, tool_id: String) -> ToolEntry => tools::get_tool(&state, &tool_id).await;

    /// Disabled tools stay in the catalog
    fn set_tool_enabled(state, tool_id: String, enabled: bool) -> ToolEntry => tools::set_tool_enabled(&state, &tool_id, enabled).await;

    fn get_tool_usage_stats(state) -> Vec<ToolUsageStats> => tools::get_tool_usage_stats(&state).await;
}
