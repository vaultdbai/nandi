//! Team commands (Tauri adapter for `services::team`).

use crate::services::team;
use crate::services::team::{Team, TeamDetail};

super::commands! {
    fn list_teams(state) -> Vec<Team> => team::list_teams(&state).await;

    /// A team with its registry's agents and packs
    fn get_team(state, team_id: String) -> TeamDetail => team::get_team(&state, &team_id).await;

    /// `github_org` is the registry's git URL
    fn create_team(state, github_org: String) -> Team => team::create_team(&state, &github_org).await;

    /// Re-syncs a team that's already here; new teams come in through
    /// `create_team`
    fn join_team(state, team_id: String) -> () => team::join_team(&state, &team_id).await;

    fn leave_team(state, team_id: String) -> () => team::leave_team(&state, &team_id).await;

    fn sync_team_registry(state, team_id: String) -> () => team::sync_team_registry(&state, &team_id).await;

    fn share_agent(state, team_id: String, agent_id: String) -> team::ShareResult => team::share_agent(&state, &team_id, &agent_id).await;

    fn unshare_agent(state, team_id: String, agent_id: String) -> () => team::unshare_agent(&state, &team_id, &agent_id).await;
}
