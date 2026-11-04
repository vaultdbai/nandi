//! Tool catalog commands (Tauri adapter for `services::tools`).

use tauri::State;

pub use crate::services::tools::{ToolEntry, ToolUsageStats};

use crate::services::tools;
use crate::services::AppState;

/// Built-in, connector, and pilot tools
#[tauri::command]
#[specta::specta]
pub async fn list_tools(state: State<'_, AppState>) -> Result<Vec<ToolEntry>, String> {
    tools::list_tools(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_tool(state: State<'_, AppState>, tool_id: String) -> Result<ToolEntry, String> {
    tools::get_tool(&state, &tool_id).await
}

/// Disabled tools stay in the catalog
#[tauri::command]
#[specta::specta]
pub async fn set_tool_enabled(
    state: State<'_, AppState>,
    tool_id: String,
    enabled: bool,
) -> Result<ToolEntry, String> {
    tools::set_tool_enabled(&state, &tool_id, enabled).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_tool_usage_stats(
    state: State<'_, AppState>,
) -> Result<Vec<ToolUsageStats>, String> {
    tools::get_tool_usage_stats(&state).await
}
