//! Roster, member profile, and trust commands (Tauri adapter for
//! `services::team::{roster, trust}`).

use tauri::State;

pub use crate::services::team::roster::{MemberProfile, TeamRoster};
pub use crate::services::team::trust::{TrustRelationship, TrustUpdate};

use crate::services::team::{roster, trust, trust_requests};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_team_roster(
    state: State<'_, AppState>,
    team_id: String,
) -> Result<TeamRoster, String> {
    roster::get_team_roster(&state, &team_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_member_profile(
    state: State<'_, AppState>,
    team_id: String,
    handle: String,
) -> Result<MemberProfile, String> {
    roster::get_member_profile(&state, &team_id, &handle).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_member_by_instance(
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<MemberProfile, String> {
    roster::get_member_by_instance(&state, &instance_id).await
}

/// "Trust everyone on my company's relay", and whether it applies
#[tauri::command]
#[specta::specta]
pub async fn get_relay_trust(state: State<'_, AppState>) -> Result<trust::RelayTrust, String> {
    Ok(trust::relay_trust(state.db()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_relay_trust(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<trust::RelayTrust, String> {
    trust::set_trusts_relay_members(state.db(), enabled)?;
    Ok(trust::relay_trust(state.db()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_trust_relationship(
    state: State<'_, AppState>,
    instance_id: String,
    update: TrustUpdate,
) -> Result<TrustRelationship, String> {
    trust::set_trust_relationship(&state, &instance_id, update).await
}

#[tauri::command]
#[specta::specta]
pub async fn block_agent_from_member(
    state: State<'_, AppState>,
    instance_id: String,
    agent_id: String,
) -> Result<TrustRelationship, String> {
    trust::block_agent_from_member(&state, &instance_id, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn unblock_agent_from_member(
    state: State<'_, AppState>,
    instance_id: String,
    agent_id: String,
) -> Result<TrustRelationship, String> {
    trust::unblock_agent_from_member(&state, &instance_id, &agent_id).await
}

/// People refused because this Nandi doesn't trust them yet
#[tauri::command]
#[specta::specta]
pub async fn list_trust_requests(
    state: State<'_, AppState>,
) -> Result<Vec<trust_requests::TrustRequest>, String> {
    trust_requests::list_trust_requests(&state).await
}

/// Trust them, or not now (`block`: refuse them from now on)
#[tauri::command]
#[specta::specta]
pub async fn respond_trust_request(
    state: State<'_, AppState>,
    instance_id: String,
    answer: trust_requests::TrustAnswer,
    block: bool,
) -> Result<(), String> {
    trust_requests::respond_trust_request(&state, &instance_id, answer, block).await
}

#[tauri::command]
#[specta::specta]
pub async fn pause_delegations_from_member(
    state: State<'_, AppState>,
    instance_id: String,
    paused: bool,
) -> Result<TrustRelationship, String> {
    trust::pause_delegations_from_member(&state, &instance_id, paused).await
}
