//! Decisions page commands (Tauri adapter for `services::inbox::decisions`).

use tauri::State;

pub use crate::services::domain::decision::{DecisionFilter, DecisionRecord};

use crate::services::inbox::decisions;
use crate::services::AppState;

/// The months that have decisions (`2026-11`), newest first
#[tauri::command]
#[specta::specta]
pub async fn list_decision_months(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    decisions::months(&state)
}

/// One month's decisions, newest first, narrowed by the filter
#[tauri::command]
#[specta::specta]
pub async fn list_decisions(
    state: State<'_, AppState>,
    month: String,
    filter: Option<DecisionFilter>,
) -> Result<Vec<DecisionRecord>, String> {
    decisions::list(&state, month, filter).await
}
