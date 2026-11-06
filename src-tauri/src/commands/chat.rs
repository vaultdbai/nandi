//! Chat commands (Tauri adapter for `services::chat`).

use tauri::State;

use crate::model::{ApprovalDecision, StudioConversationSummary};
use crate::services::chat::{self, SendMessageResult};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn send_message(
    state: State<'_, AppState>,
    agent_id: String,
    text: String,
    attachments: Vec<String>,
    client_message_id: Option<String>,
) -> Result<SendMessageResult, String> {
    chat::send_message(&state, agent_id, text, attachments, client_message_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn send_studio_message(
    state: State<'_, AppState>,
    conversation_id: String,
    text: String,
    attachments: Vec<String>,
    client_message_id: Option<String>,
) -> Result<SendMessageResult, String> {
    chat::send_studio_message(
        &state,
        conversation_id,
        text,
        attachments,
        client_message_id,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn create_studio_conversation(
    state: State<'_, AppState>,
) -> Result<StudioConversationSummary, String> {
    chat::create_studio_conversation(&state).await
}

/// Refused unless the approval belongs to `agent_id`
#[tauri::command]
#[specta::specta]
pub async fn approve_action(
    state: State<'_, AppState>,
    agent_id: String,
    approval_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    chat::approve_action(&state, Some(&agent_id), approval_id, decision).await
}

/// Answers a browser hand-over, whichever agent's: done, skipped, or
/// changed (with the text that went out)
#[tauri::command]
#[specta::specta]
pub async fn answer_hand_over(
    state: State<'_, AppState>,
    approval_id: String,
    answer: crate::services::pilots::handover::HandOverAnswer,
) -> Result<(), String> {
    chat::answer_hand_over(&state, approval_id, answer).await
}

/// Every pending tool approval, oldest first, with when each expires
#[tauri::command]
#[specta::specta]
pub async fn list_pending_approvals(
    state: State<'_, AppState>,
) -> Result<Vec<crate::model::PendingApproval>, String> {
    chat::list_pending_approvals(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_turn(
    state: State<'_, AppState>,
    agent_id: String,
    turn_id: String,
) -> Result<(), String> {
    chat::cancel_turn(&state, agent_id, turn_id).await
}
