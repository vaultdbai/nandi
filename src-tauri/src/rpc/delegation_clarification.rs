//! Clarification and escalation commands (Tauri adapter for
//! `services::a2a::{clarification, escalation}`).

use crate::services::a2a::{clarification, escalation};
use crate::services::a2a::{ClarificationPolicy, ClarificationResponse, PendingClarification};

super::commands! {
    fn respond_to_clarification(state, response: ClarificationResponse) -> () => clarification::respond_to_clarification(&state, response).await;

    fn escalate_clarification(state, task_id: String, question_id: String) -> () => escalation::escalate_clarification(&state, &task_id, &question_id).await;

    fn get_pending_clarifications(state) -> Vec<PendingClarification> => clarification::get_pending_clarifications(&state).await;

    /// The requester agent's clarification policy (skill.md `clarification:`)
    fn get_clarification_policy(state, agent_id: String) -> ClarificationPolicy => clarification::get_clarification_policy(&state, &agent_id).await;

    /// Returns the policy as saved (bounded, topics trimmed and de-duplicated)
    fn set_clarification_policy(state, agent_id: String, policy: ClarificationPolicy) -> ClarificationPolicy => clarification::set_clarification_policy(&state, &agent_id, policy).await;
}
