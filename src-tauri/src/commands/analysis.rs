//! Run analysis commands (Tauri adapter for `services::run_analysis`).

use tauri::State;

pub use crate::services::run_analysis::AnalysisResponse;

use crate::services::run_analysis;
use crate::services::AppState;

/// Answers `question` from the run's own record, citing sources
#[tauri::command]
#[specta::specta]
pub async fn analyze_run(
    state: State<'_, AppState>,
    run_id: String,
    question: String,
) -> Result<AnalysisResponse, String> {
    run_analysis::analyze_run(&state, &run_id, &question).await
}

/// A short markdown summary of the run (cached)
#[tauri::command]
#[specta::specta]
pub async fn summarize_run(state: State<'_, AppState>, run_id: String) -> Result<String, String> {
    run_analysis::summarize_run(&state, &run_id).await
}
