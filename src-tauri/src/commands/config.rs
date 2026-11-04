//! Repo config commands (Tauri adapter for `services::config`).

use tauri::State;

pub use crate::services::config::repo_config::RepoLlmConfigStatus;

use crate::services::config::repo_config;
use crate::services::AppState;

/// What `.nandi/llm.yaml` holds and what's in effect
#[tauri::command]
#[specta::specta]
pub async fn get_repo_llm_config(
    state: State<'_, AppState>,
) -> Result<RepoLlmConfigStatus, String> {
    repo_config::get_repo_llm_config(&state).await
}

/// Re-reads `.nandi/llm.yaml`
#[tauri::command]
#[specta::specta]
pub async fn reload_repo_llm_config(
    state: State<'_, AppState>,
) -> Result<RepoLlmConfigStatus, String> {
    repo_config::reload_repo_llm_config(&state).await
}
