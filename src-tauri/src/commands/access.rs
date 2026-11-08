//! Connector access commands (Tauri adapter for `services::access` and
//! `services::grants`): Settings › Connectors › Access, an agent's
//! Connectors panel, and the answer to an access request.

use tauri::State;

use crate::services::access::{self, AccessChoice};
use crate::services::domain::grant::{GrantAccess, GrantScope};
use crate::services::grants::GrantInfo;
use crate::services::AppState;

/// Every grant, saved and on this computer
#[tauri::command]
#[specta::specta]
pub async fn list_connector_grants(state: State<'_, AppState>) -> Result<Vec<GrantInfo>, String> {
    Ok(state.grants().list())
}

/// Gives a grant up front (it lasts until removed)
#[tauri::command]
#[specta::specta]
pub async fn grant_connector_access(
    state: State<'_, AppState>,
    agent_id: String,
    connector_id: String,
    access: GrantAccess,
    scope: GrantScope,
) -> Result<(), String> {
    let who = crate::services::chat::decider(&state);
    access::grant_up_front(&state, &agent_id, &connector_id, access, scope, who).await
}

/// Removes a grant; the agent's next call asks again
#[tauri::command]
#[specta::specta]
pub async fn revoke_connector_access(
    state: State<'_, AppState>,
    grant: GrantInfo,
) -> Result<(), String> {
    let who = crate::services::chat::decider(&state);
    access::revoke(&state, grant, who).await
}

/// The person's answer to a paused run's access request. `subdomains`: a
/// site request's grant covers the site's subdomains too
#[tauri::command]
#[specta::specta]
pub async fn answer_access_request(
    state: State<'_, AppState>,
    approval_id: String,
    choice: AccessChoice,
    subdomains: bool,
) -> Result<(), String> {
    let who = crate::services::chat::decider(&state);
    access::answer(&state, &approval_id, choice, subdomains, who).await
}
