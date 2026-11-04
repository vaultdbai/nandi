//! Team commands (Tauri adapter for `services::team`).

use tauri::State;

pub use crate::services::team::{Team, TeamDetail};

use crate::services::team;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_teams(state: State<'_, AppState>) -> Result<Vec<Team>, String> {
    team::list_teams(&state).await
}

/// A team with its registry's agents and packs
#[tauri::command]
#[specta::specta]
pub async fn get_team(state: State<'_, AppState>, team_id: String) -> Result<TeamDetail, String> {
    team::get_team(&state, &team_id).await
}

/// `github_org` is the registry's git URL
#[tauri::command]
#[specta::specta]
pub async fn create_team(state: State<'_, AppState>, github_org: String) -> Result<Team, String> {
    team::create_team(&state, &github_org).await
}

/// Re-syncs a team that's already here; new teams come in through
/// `create_team`
#[tauri::command]
#[specta::specta]
pub async fn join_team(state: State<'_, AppState>, team_id: String) -> Result<(), String> {
    team::join_team(&state, &team_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn leave_team(state: State<'_, AppState>, team_id: String) -> Result<(), String> {
    team::leave_team(&state, &team_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn sync_team_registry(state: State<'_, AppState>, team_id: String) -> Result<(), String> {
    team::sync_team_registry(&state, &team_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn share_agent(
    state: State<'_, AppState>,
    team_id: String,
    agent_id: String,
) -> Result<team::ShareResult, String> {
    team::share_agent(&state, &team_id, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn unshare_agent(
    state: State<'_, AppState>,
    team_id: String,
    agent_id: String,
) -> Result<(), String> {
    team::unshare_agent(&state, &team_id, &agent_id).await
}
