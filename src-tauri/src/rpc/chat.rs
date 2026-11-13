//! Chat commands (`services::chat`).

use crate::model::{ApprovalDecision, PendingApproval, StudioConversationSummary};
use crate::services::chat::{self, SendMessageResult};
use crate::services::pilots::handover::HandOverAnswer;

super::commands! {
    fn send_message(state, agent_id: String, text: String, attachments: Vec<String>, client_message_id: Option<String>) -> SendMessageResult => chat::send_message(&state, agent_id, text, attachments, client_message_id).await;

    fn send_studio_message(state, conversation_id: String, text: String, attachments: Vec<String>, client_message_id: Option<String>) -> SendMessageResult => chat::send_studio_message(&state, conversation_id, text, attachments, client_message_id).await;

    fn create_studio_conversation(state) -> StudioConversationSummary => chat::create_studio_conversation(&state).await;

    /// Refused unless the approval belongs to `agent_id`
    fn approve_action(state, agent_id: String, approval_id: String, decision: ApprovalDecision) -> () => chat::approve_action(&state, Some(&agent_id), approval_id, decision).await;

    /// Answers a browser hand-over, whichever agent's: done, skipped, or
    /// changed (with the text that went out)
    fn answer_hand_over(state, approval_id: String, answer: HandOverAnswer) -> () => chat::answer_hand_over(&state, approval_id, answer).await;

    /// Every pending tool approval, oldest first, with when each expires
    fn list_pending_approvals(state) -> Vec<PendingApproval> => chat::list_pending_approvals(&state).await;

    fn cancel_turn(state, agent_id: String, turn_id: String) -> () => chat::cancel_turn(&state, agent_id, turn_id).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    #[tokio::test]
    async fn studio_conversation_round_trip() {
        let state = AppState::for_tests(vec![]);
        let created = call(&state, "create_studio_conversation", json!({}));
        let id = created.await.unwrap()["id"].as_str().unwrap().to_string();

        let message = json!({ "conversationId": id, "text": "hello", "attachments": [], "clientMessageId": null });
        let sent = call(&state, "send_studio_message", message).await.unwrap();
        assert!(sent["runId"].is_string());

        let listed = call(&state, "get_studio_conversations", json!({}));
        assert_eq!(listed.await.unwrap()[0]["title"], "hello");
        state.shutdown();
    }
}
