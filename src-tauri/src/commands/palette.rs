//! Command palette commands (Tauri adapter for `services::palette`).

use serde_json::Value;
use tauri::State;

pub use crate::services::palette::{PaletteActionOutcome, PaletteResult};

use crate::services::palette;
use crate::services::AppState;

/// Ranked results across agents, workflows, runs, packages, tools, and
/// actions; `>`-prefixed queries search actions only
#[tauri::command]
#[specta::specta]
pub async fn search_palette(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<PaletteResult>, String> {
    palette::search_palette(&state, &query).await
}

/// Runs a built-in action by ID with optional arguments
#[tauri::command]
#[specta::specta]
pub async fn execute_palette_action(
    state: State<'_, AppState>,
    action_id: String,
    args: Value,
) -> Result<PaletteActionOutcome, String> {
    palette::execute_palette_action(&state, &action_id, args).await
}
