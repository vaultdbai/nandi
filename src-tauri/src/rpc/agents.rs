//! Agent commands (`services::agents`, `services::agent_overview`).

use crate::services::agent_overview::{self, AgentOverview};
use crate::services::agents::{self, AgentInfo, CreateAgentInput};
use crate::services::shipped_files::ShippedUpdate;

super::commands! {
    fn list_agents(state) -> Vec<AgentInfo> => agents::list_agents(&state).await;

    /// Each agent's setup and its last seven days; `today_start` is local
    /// midnight, epoch ms
    fn list_agent_overviews(state, today_start: f64) -> Vec<AgentOverview> => agent_overview::list_agent_overviews(&state, today_start).await;

    fn create_agent(state, input: CreateAgentInput) -> AgentInfo => agents::create_agent(&state, input).await;

    /// Starts the agent's runtime container; returns its ID
    fn start_agent(state, agent_id: String) -> String => agents::start_agent(&state, &agent_id).await;

    fn stop_agent(state, agent_id: String) -> () => agents::stop_agent(&state, &agent_id).await;

    /// Restarts the agent's runtime container; returns its ID
    fn restart_agent(state, agent_id: String) -> String => agents::restart_agent(&state, &agent_id).await;

    fn delete_agent(state, agent_id: String) -> () => agents::delete_agent(&state, &agent_id).await;

    /// Puts Nandi, the default agent, back as it ships; its old files stay
    /// in the gold copy's git history
    fn reset_default_agent(state) -> () => agents::reset_default_agent(&state).await;

    /// The files Nandi ships that the person edited and this build
    /// changed, each with a diff from theirs
    fn default_agent_updates(state) -> Vec<ShippedUpdate> => agents::default_agent_updates(&state).await;

    /// Take update: commits this build's version of a shipped file over the
    /// person's; theirs stays in git history
    fn take_default_agent_update(state, path: String) -> () => agents::take_default_agent_update(&state, &path).await;

    /// Keep mine: the update to this file isn't offered again until a newer
    /// version ships
    fn keep_default_agent_file(state, path: String) -> () => agents::keep_default_agent_file(&state, &path).await;
}
