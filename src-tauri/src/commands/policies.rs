//! Policy commands (Tauri adapter for `services::policies`).

use tauri::State;

pub use crate::services::domain::policy::PolicySummary;
pub use crate::services::policies::AgentPolicies;

use crate::services::policies;
use crate::services::AppState;

/// The policies over an agent and what each changed, or why it can't run
#[tauri::command]
#[specta::specta]
pub async fn agent_policies(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<AgentPolicies, String> {
    policies::agent_policies(state.store(), &agent_id)
}

/// Every policy in the repo by id and name, broken ones included
#[tauri::command]
#[specta::specta]
pub async fn list_policies(state: State<'_, AppState>) -> Result<Vec<PolicySummary>, String> {
    policies::list(state.store())
}
