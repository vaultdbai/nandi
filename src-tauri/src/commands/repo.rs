//! Repo commands (Tauri adapter for `services::repo_service`).

use tauri::State;

use crate::services::projection::ProjectionStats;
use crate::services::repo_service::{self, RepoStatus};
use crate::services::AppState;

/// Wipes the local projection and rebuilds it from the repo files. Refuses
/// while a file is invalid unless `force` (which skips invalid files).
#[tauri::command]
#[specta::specta]
pub async fn repo_rebuild(
    state: State<'_, AppState>,
    force: Option<bool>,
) -> Result<ProjectionStats, String> {
    repo_service::rebuild(&state, force.unwrap_or(false)).await
}

/// Counts, problems (invalid files, broken references), and whether edits
/// apply live.
#[tauri::command]
#[specta::specta]
pub async fn repo_status(state: State<'_, AppState>) -> Result<RepoStatus, String> {
    repo_service::status(&state).await
}

// --- Agent editor + git (wave 1, Track B) ---------------------------------

pub use crate::services::repo_service::{AgentDefinitionInput, GitStatus};

use crate::services::agents::AgentInfo;

/// Writes the agent's files (gold copy) and returns the saved agent.
/// Creates the agent if it has no folder yet. Marketplace agents are
/// read-only until forked.
#[tauri::command]
#[specta::specta]
pub async fn repo_save_agent(
    state: State<'_, AppState>,
    agent_id: String,
    input: AgentDefinitionInput,
) -> Result<AgentInfo, String> {
    repo_service::save_agent(&state, &agent_id, input).await
}

/// Replaces the agent's `instructions.md` (the user's rules). For an
/// installed agent this is an override that updates keep.
#[tauri::command]
#[specta::specta]
pub async fn repo_save_agent_instructions(
    state: State<'_, AppState>,
    agent_id: String,
    instructions: String,
) -> Result<(), String> {
    repo_service::save_agent_instructions(&state, &agent_id, instructions).await
}

/// Back to an installed agent's shipped instructions; whether it had the
/// user's own
#[tauri::command]
#[specta::specta]
pub async fn repo_reset_agent_instructions(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<bool, String> {
    repo_service::reset_agent_instructions(&state, &agent_id).await
}

/// One entity as stored in its file. `kind`: `agent` | `workflow` |
/// `trigger` | `connector` | `settings`
#[tauri::command]
#[specta::specta]
pub async fn repo_load_entity(
    state: State<'_, AppState>,
    kind: String,
    id: String,
) -> Result<serde_json::Value, String> {
    repo_service::load_entity(&state, &kind, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn repo_git_status(state: State<'_, AppState>) -> Result<GitStatus, String> {
    repo_service::git_status(&state).await
}

/// Unified diff of one file against `HEAD`
#[tauri::command]
#[specta::specta]
pub async fn repo_git_diff(state: State<'_, AppState>, path: String) -> Result<String, String> {
    repo_service::git_diff(&state, &path).await
}

/// Stages everything and commits it; returns the commit hash. Never pushes
/// (that's "Sync now").
#[tauri::command]
#[specta::specta]
pub async fn repo_git_commit(
    state: State<'_, AppState>,
    message: String,
) -> Result<String, String> {
    repo_service::git_commit(&state, &message).await
}
