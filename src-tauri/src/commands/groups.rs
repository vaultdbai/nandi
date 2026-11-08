//! Group commands (Tauri adapter for `services::groups`): Network › Groups

use tauri::State;

pub use crate::services::domain::group::AccessLevel;
pub use crate::services::groups::{GroupView, GroupsOverview, InviteRequest};
pub use nandi_protocol::{GroupInvite, GroupRole};

use crate::services::groups::Groups;
use crate::services::AppState;

/// Every group on the relay and the invites for me (`connected: false`
/// without a relay)
#[tauri::command]
#[specta::specta]
pub async fn list_groups(state: State<'_, AppState>) -> Result<GroupsOverview, String> {
    Groups::of(&state)?.overview().await
}

/// One group in full (members only)
#[tauri::command]
#[specta::specta]
pub async fn get_group(state: State<'_, AppState>, group_id: String) -> Result<GroupView, String> {
    Groups::of(&state)?.get(&group_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_group(
    state: State<'_, AppState>,
    name: String,
    description: String,
    policy: String,
) -> Result<GroupView, String> {
    Groups::of(&state)?
        .create(&name, &description, &policy)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_group(
    state: State<'_, AppState>,
    group_id: String,
    name: String,
    description: String,
) -> Result<GroupView, String> {
    Groups::of(&state)?
        .update(&group_id, &name, &description)
        .await
}

/// Signs the next version of the policy and sends it (admins)
#[tauri::command]
#[specta::specta]
pub async fn set_group_policy(
    state: State<'_, AppState>,
    group_id: String,
    policy: String,
) -> Result<GroupView, String> {
    Groups::of(&state)?.set_policy(&group_id, &policy).await
}

#[tauri::command]
#[specta::specta]
pub async fn invite_to_group(
    state: State<'_, AppState>,
    group_id: String,
    invite: InviteRequest,
) -> Result<GroupInvite, String> {
    Groups::of(&state)?.invite(&group_id, invite).await
}

#[tauri::command]
#[specta::specta]
pub async fn revoke_group_invite(state: State<'_, AppState>, code: String) -> Result<(), String> {
    Groups::of(&state)?.revoke_invite(&code).await
}

#[tauri::command]
#[specta::specta]
pub async fn join_group(state: State<'_, AppState>, code: String) -> Result<GroupView, String> {
    Groups::of(&state)?.join(&code).await
}

#[tauri::command]
#[specta::specta]
pub async fn leave_group(state: State<'_, AppState>, group_id: String) -> Result<(), String> {
    Groups::of(&state)?.leave(&group_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_group_member(
    state: State<'_, AppState>,
    group_id: String,
    instance_id: String,
) -> Result<GroupView, String> {
    Groups::of(&state)?
        .remove_member(&group_id, &instance_id)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_group_role(
    state: State<'_, AppState>,
    group_id: String,
    instance_id: String,
    role: GroupRole,
) -> Result<GroupView, String> {
    Groups::of(&state)?
        .set_role(&group_id, &instance_id, role)
        .await
}

/// My access level for a group ("Ask first" when I never picked one)
#[tauri::command]
#[specta::specta]
pub async fn get_group_access(
    state: State<'_, AppState>,
    group_id: String,
) -> Result<AccessLevel, String> {
    Groups::of(&state)?.access(&group_id)
}

/// Picks my access level for a group (works offline)
#[tauri::command]
#[specta::specta]
pub async fn set_group_access(
    state: State<'_, AppState>,
    group_id: String,
    level: AccessLevel,
) -> Result<AccessLevel, String> {
    Groups::of(&state)?.set_access(&group_id, level)
}
