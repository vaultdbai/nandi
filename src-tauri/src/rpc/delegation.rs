//! Delegation commands (Tauri adapter for `services::a2a`).

use crate::services::a2a;
use crate::services::a2a::{AgentDelegationSettings, TaskResult};
use crate::services::domain::AgentCard;
use serde_json::Value;

super::commands! {
    /// Requests that came back while nobody watched, not yet dismissed
    /// (Inbox items), newest first
    fn list_request_returns(state) -> Vec<a2a::returns::RequestReturn> => a2a::returns::list(state.db()).await;

    /// Takes a returned request out of the Inbox
    fn dismiss_request_return(state, task_id: String) -> () => a2a::returns::dismiss_request_return(&state, task_id).await;

    /// Local and remote agents serving `capability`
    fn list_available_providers(state, capability: String) -> Vec<AgentCard> => a2a::list_available_providers(&state, &capability).await;

    /// Starts a delegation; returns the task ID. `target` (a card ID) skips
    /// provider scoring.
    fn delegate_task(state, capability: String, input: Value, target: Option<String>) -> String => a2a::delegate_task(&state, &capability, input, target.as_deref()).await;

    /// Newest first, both directions
    fn list_delegation_history(state, limit: u32) -> Vec<TaskResult> => a2a::list_delegation_history(&state, limit).await;

    fn get_agent_delegation(state, agent_id: String) -> AgentDelegationSettings => a2a::get_agent_delegation(&state, &agent_id).await;

    fn set_agent_delegation(state, agent_id: String, settings: AgentDelegationSettings) -> () => a2a::set_agent_delegation(&state, &agent_id, settings).await;
}
