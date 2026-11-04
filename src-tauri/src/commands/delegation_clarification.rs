//! Clarification and escalation commands (Tauri adapter for
//! `services::a2a::{clarification, escalation}`).

use tauri::State;

pub use crate::services::a2a::{ClarificationPolicy, ClarificationResponse, PendingClarification};

use crate::services::a2a::{clarification, escalation};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn respond_to_clarification(
    state: State<'_, AppState>,
    response: ClarificationResponse,
) -> Result<(), String> {
    clarification::respond_to_clarification(&state, response).await
}

#[tauri::command]
#[specta::specta]
pub async fn escalate_clarification(
    state: State<'_, AppState>,
    task_id: String,
    question_id: String,
) -> Result<(), String> {
    escalation::escalate_clarification(&state, &task_id, &question_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_pending_clarifications(
    state: State<'_, AppState>,
) -> Result<Vec<PendingClarification>, String> {
    clarification::get_pending_clarifications(&state).await
}

/// The requester agent's clarification policy (skill.md `clarification:`)
#[tauri::command]
#[specta::specta]
pub async fn get_clarification_policy(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<ClarificationPolicy, String> {
    clarification::get_clarification_policy(&state, &agent_id).await
}

/// Returns the policy as saved (bounded, topics trimmed and de-duplicated)
#[tauri::command]
#[specta::specta]
pub async fn set_clarification_policy(
    state: State<'_, AppState>,
    agent_id: String,
    policy: ClarificationPolicy,
) -> Result<ClarificationPolicy, String> {
    clarification::set_clarification_policy(&state, &agent_id, policy).await
}
