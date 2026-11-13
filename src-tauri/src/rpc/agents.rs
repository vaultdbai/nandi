//! Agent commands (`services::agents`, `services::agent_overview`).

use crate::services::agent_overview::{self, AgentOverview};
use crate::services::agents::{self, AgentInfo, CreateAgentInput};

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
}
