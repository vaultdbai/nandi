//! Group commands (Tauri adapter for `services::groups`): Network › Groups

use crate::services::domain::group::AccessLevel;
use crate::services::groups::Groups;
use crate::services::groups::{GroupView, GroupsOverview, InviteRequest};
use nandi_protocol::{GroupInvite, GroupRole};

super::commands! {
    /// Every group on the relay and the invites for me (`connected: false`
    /// without a relay)
    fn list_groups(state) -> GroupsOverview => Groups::of(&state)?.overview().await;

    /// One group in full (members only)
    fn get_group(state, group_id: String) -> GroupView => Groups::of(&state)?.get(&group_id).await;

    fn create_group(state, name: String, description: String, policy: String) -> GroupView => Groups::of(&state)?.create(&name, &description, &policy).await;

    fn update_group(state, group_id: String, name: String, description: String) -> GroupView => Groups::of(&state)?.update(&group_id, &name, &description).await;

    /// Signs the next version of the policy and sends it (admins)
    fn set_group_policy(state, group_id: String, policy: String) -> GroupView => Groups::of(&state)?.set_policy(&group_id, &policy).await;

    fn invite_to_group(state, group_id: String, invite: InviteRequest) -> GroupInvite => Groups::of(&state)?.invite(&group_id, invite).await;

    fn revoke_group_invite(state, code: String) -> () => Groups::of(&state)?.revoke_invite(&code).await;

    fn join_group(state, code: String) -> GroupView => Groups::of(&state)?.join(&code).await;

    fn leave_group(state, group_id: String) -> () => Groups::of(&state)?.leave(&group_id).await;

    fn remove_group_member(state, group_id: String, instance_id: String) -> GroupView => Groups::of(&state)?.remove_member(&group_id, &instance_id).await;

    fn set_group_role(state, group_id: String, instance_id: String, role: GroupRole) -> GroupView => Groups::of(&state)?.set_role(&group_id, &instance_id, role).await;

    /// My access level for a group ("Ask first" when I never picked one)
    fn get_group_access(state, group_id: String) -> AccessLevel => Groups::of(&state)?.access(&group_id);

    /// Picks my access level for a group (works offline)
    fn set_group_access(state, group_id: String, level: AccessLevel) -> AccessLevel => Groups::of(&state)?.set_access(&group_id, level);
}
