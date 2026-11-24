//! Ask each time: one approval card per release, while the request waits
//! in the egress proxy.
//!
//! ```text
//! proxy request with a key ─▶ approval row + card (tool `lock_box_key`,
//!   tool_call_id `lockbox:<id>`), naming the key, the agent and the host
//!   ─▶ waits for: a decision (`decide`), the wait running out, or the
//!      approval closing without one (the run ended)
//!   ─▶ approved: the key is released; anything else: nothing is
//! ```
//!
//! Like a harness call, the run isn't suspended by the orchestrator: the
//! request waits here, and decisions reach it through [`decide`]
//! (`chat::approve_action`). The card never holds the key's value.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

use crate::db::{self, ApprovalRow, PlannedCall};
use crate::model::RunSummary;
use crate::model::{ApprovalDecision, ApprovalStatus};
use crate::services::events::RunContext;
use crate::services::AppState;

/// `approvals.tool_call_id` prefix of a key release's approval
pub const CALL_PREFIX: &str = "lockbox:";
/// How often a waiting request checks its approval is still open
const CHECK_EVERY: Duration = Duration::from_secs(2);

/// Approval id → the request waiting for its decision
static WAITING: LazyLock<Mutex<HashMap<String, oneshot::Sender<ApprovalStatus>>>> =
    LazyLock::new(Mutex::default);

fn waiting() -> std::sync::MutexGuard<'static, HashMap<String, oneshot::Sender<ApprovalStatus>>> {
    WAITING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether `approval` asks to release a key
pub fn is_key_approval(approval: &ApprovalRow) -> bool {
    approval.tool_call_id.starts_with(CALL_PREFIX)
}

/// Hands `decision` to the request waiting on `approval_id`. `false` when
/// nothing waits for it any more.
pub fn decide(approval_id: &str, decision: ApprovalDecision) -> bool {
    let status = match decision {
        ApprovalDecision::Approved => ApprovalStatus::Approved,
        ApprovalDecision::Rejected => ApprovalStatus::Rejected,
    };
    waiting()
        .remove(approval_id)
        .is_some_and(|waiter| waiter.send(status).is_ok())
}

/// Whether a request still waits on `approval_id` (it then expires itself)
pub fn is_waiting(approval_id: &str) -> bool {
    waiting().contains_key(approval_id)
}

async fn context(state: &AppState, run: &RunSummary) -> RunContext {
    let agent_name = state
        .store()
        .agent(&run.agent_id)
        .map(|a| a.name().to_string())
        .unwrap_or_else(|_| run.agent_id.clone());
    RunContext {
        agent_id: run.agent_id.clone(),
        agent_name,
        conversation_id: run.conversation_id.clone(),
        run_id: run.id.clone(),
    }
}

/// An approval no request waits for (the app restarted while it waited):
/// it closes as expired, and nothing is released
pub async fn close_stale(state: &AppState, approval: &ApprovalRow) -> Result<(), String> {
    let id = approval.run_id.clone();
    let run = state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await?
        .ok_or("This key request's run is gone")?;
    let ctx = context(state, &run).await;
    state
        .bus()
        .approval_closed(&ctx, approval.clone(), ApprovalStatus::Expired)
        .await
}

/// The person's answer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Approved, by whom
    Approved(String),
    /// Declined, by whom (or nobody, when it expired)
    Declined(Option<String>),
}

/// Asks to release a key in `run`, with `args` on the card, and waits up
/// to `wait` for the person
pub async fn ask(state: &AppState, run: &RunSummary, args: Value, wait: Duration) -> Answer {
    let ctx = context(state, run).await;
    let planned = PlannedCall {
        id: format!("{CALL_PREFIX}{}", uuid::Uuid::new_v4().simple()),
        tool: super::KEY_TOOL.to_string(),
        args,
        requires_approval: true,
    };
    let (sender, mut receiver) = oneshot::channel();
    let approval_id = match state.bus().approval_request(&ctx, &planned).await {
        Ok(id) => id,
        Err(e) => {
            log::warn!("Couldn't ask to release a key: {e}");
            return Answer::Declined(None);
        }
    };
    waiting().insert(approval_id.clone(), sender);
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    let mut check = tokio::time::interval(CHECK_EVERY);
    let status = loop {
        tokio::select! {
            decided = &mut receiver => break decided.unwrap_or(ApprovalStatus::Expired),
            () = &mut deadline => break ApprovalStatus::Expired,
            _ = check.tick() => {
                // The run ended and closed it (`turn_done`)
                if !still_pending(state, &approval_id).await {
                    break ApprovalStatus::Expired;
                }
            }
        }
    };
    waiting().remove(&approval_id);
    let id = approval_id.clone();
    let row = state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await;
    let decided_by = match row {
        Ok(Some(row)) => {
            if row.status == ApprovalStatus::Pending {
                if let Err(e) = state.bus().approval_closed(&ctx, row.clone(), status).await {
                    log::warn!("Couldn't close approval {approval_id}: {e}");
                }
            }
            row.decided_by
        }
        Ok(None) => None,
        Err(e) => {
            log::warn!("Couldn't read approval {approval_id}: {e}");
            None
        }
    };
    match status {
        ApprovalStatus::Approved => {
            Answer::Approved(decided_by.unwrap_or_else(|| crate::services::chat::decider(state)))
        }
        _ => Answer::Declined(decided_by),
    }
}

/// Whether the person declined (or let expire) a card for `key` at `host`
/// earlier in `run_id`
pub async fn declined_in_run(state: &AppState, run_id: &str, key: &str, host: &str) -> bool {
    let run = run_id.to_string();
    let rows = state
        .db()
        .call(move |conn| db::run_approvals_for_tool(conn, &run, super::KEY_TOOL))
        .await
        .unwrap_or_default();
    rows.iter().any(|row| {
        matches!(
            row.status,
            ApprovalStatus::Rejected | ApprovalStatus::Expired
        ) && row.args["key"] == key
            && row.args["host"] == host
    })
}

async fn still_pending(state: &AppState, approval_id: &str) -> bool {
    let id = approval_id.to_string();
    state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await
        .ok()
        .flatten()
        .is_some_and(|row| row.status == ApprovalStatus::Pending)
}
