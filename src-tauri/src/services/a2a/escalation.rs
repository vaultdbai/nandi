//! Human escalation of clarification questions (wave final, Track C): an
//! `approvals` row (`tool = "delegation_question"`), the task goes
//! `InputRequired`; the answer resumes it (`Working`), a denial cancels it,
//! no answer by the deadline times it out (see `clarification`).
//!
//! The rows are bookkeeping for the human's decision, not run approvals:
//! their `run_id` (`clarification:<task>`) names no run, so the chat's
//! pending-approval list (which joins `runs`) and the orchestrator's
//! resume path never see them. The UI answers through
//! `respond_to_clarification`; `resolve_approval` is the hook for
//! `approve_action` to forward a decision on one of these rows.

use serde_json::json;

use super::clarification::ClarificationService;
use super::Question;
use crate::db::{self, ApprovalRow, Db};
use crate::model::{ApprovalDecision, ApprovalStatus};
use crate::services::AppState;

/// The `approvals.tool` of an escalated question
pub const ESCALATION_TOOL: &str = "delegation_question";
/// `approvals.run_id` prefix of an escalated question
pub const RUN_PREFIX: &str = "clarification:";

/// Records the human's pending decision on one question
pub(super) async fn insert_approval(
    db: &Db,
    approval_id: &str,
    agent_id: &str,
    task_id: &str,
    round: u32,
    question: &Question,
) -> Result<(), String> {
    let row = ApprovalRow {
        id: approval_id.to_string(),
        agent_id: agent_id.to_string(),
        run_id: format!("{RUN_PREFIX}{task_id}"),
        message_id: String::new(),
        tool_call_id: format!("{RUN_PREFIX}{approval_id}"),
        tool: ESCALATION_TOOL.to_string(),
        args: json!({
            "task_id": task_id,
            "round": round,
            "question": question,
        }),
        status: ApprovalStatus::Pending,
        created_at: db::now_ms(),
        // The clarification's own deadline governs it
        expires_at: None,
        decided_by: None,
    };
    db.call(move |conn| db::insert_approval(conn, &row)).await
}

/// Resolves the still-pending ones among `ids`
pub(super) async fn close_approvals(db: &Db, ids: Vec<String>, status: ApprovalStatus) {
    if ids.is_empty() {
        return;
    }
    let closed = db
        .call(move |conn| {
            for id in ids {
                if let Some(row) = db::get_approval(conn, &id)? {
                    if row.status == ApprovalStatus::Pending {
                        db::set_approval_status(conn, &row, status)?;
                    }
                }
            }
            Ok(())
        })
        .await;
    if let Err(e) = closed {
        log::warn!("Failed to resolve clarification approvals: {e}");
    }
}

fn service(state: &AppState) -> Result<&std::sync::Arc<ClarificationService>, String> {
    state
        .clarifications()
        .ok_or_else(|| "Clarification is unavailable".to_string())
}

/// Sends one question of a task to the human, even if the policy would
/// have answered it automatically
pub async fn escalate_clarification(
    state: &AppState,
    task_id: &str,
    question_id: &str,
) -> Result<(), String> {
    service(state)?.escalate(task_id, question_id).await
}

/// A decision on an escalation row made through `approve_action`:
/// approved lets the requester agent answer, rejected cancels the task.
/// `chat::approve_action` forwards `ESCALATION_TOOL` rows here instead of
/// to the orchestrator.
pub async fn resolve_approval(
    state: &AppState,
    approval_id: &str,
    decision: ApprovalDecision,
) -> Result<(), String> {
    service(state)?
        .resolve_approval(approval_id, decision == ApprovalDecision::Approved)
        .await
}
