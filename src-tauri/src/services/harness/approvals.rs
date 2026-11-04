//! Approvals for harness runs (Claude Code, Codex, Gemini CLI).
//!
//! A native run suspends on an approval and the orchestrator resumes it. A
//! harness run can't: the CLI is mid-turn, waiting for its
//! `session/request_permission` answer. So the run keeps going and one call
//! waits here instead:
//!
//! ```text
//! ACP permission request, kind in requires_approval
//!   ─▶ approval row + message (`approval_request`), tool_call_id `harness:<id>`
//!   ─▶ waits for: a decision (`decide`), the TTL, or the run ending
//!   ─▶ `approval_closed` ─▶ the CLI gets allow or reject
//! ```
//!
//! Decisions reach a waiter through [`decide`], never the orchestrator: its
//! `close_approval` would restart the run natively. An approval nobody waits
//! for any more (the app restarted mid-turn, so the CLI is gone) can't be
//! answered; its run is cancelled instead ([`close_stale`]).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::acp::{AskedCall, PermissionAsker};
use crate::db::{self, ApprovalRow, PlannedCall};
use crate::model::{ApprovalDecision, ApprovalStatus, APPROVAL_TTL_MS};
use crate::services::events::{EventBus, RunContext};
use crate::services::orchestrator::OrchestratorCommand;
use crate::services::AppState;

/// `approvals.tool_call_id` prefix of a harness run's approval
pub const CALL_PREFIX: &str = "harness:";

/// Approval id → the call waiting for its decision
static WAITING: LazyLock<Mutex<HashMap<String, oneshot::Sender<ApprovalStatus>>>> =
    LazyLock::new(Mutex::default);

fn waiting() -> std::sync::MutexGuard<'static, HashMap<String, oneshot::Sender<ApprovalStatus>>> {
    WAITING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether `approval` was asked by a harness run
pub fn is_harness(approval: &ApprovalRow) -> bool {
    approval.tool_call_id.starts_with(CALL_PREFIX)
}

/// Hands `decision` to the call waiting on `approval_id`. `false` when
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

/// Whether a call still waits on `approval_id` (it then expires itself)
pub fn is_waiting(approval_id: &str) -> bool {
    waiting().contains_key(approval_id)
}

/// An approval no call waits for: its CLI is gone, so the run can't go
/// on. Cancelling it expires the approval with it (`turn_done`).
pub async fn close_stale(state: &AppState, approval: &ApprovalRow) -> Result<(), String> {
    log::info!(
        "Approval {} waits on a harness run that's no longer running; cancelling run {}",
        approval.id,
        approval.run_id
    );
    state
        .orchestrators()
        .send(
            &approval.agent_id,
            OrchestratorCommand::Cancel {
                run_id: approval.run_id.clone(),
            },
        )
        .await
}

/// Asks the user, through the Inbox, on behalf of one harness run
pub struct RunApprover {
    bus: EventBus,
    ctx: RunContext,
    /// Cancelled when the run's session ends
    ended: CancellationToken,
    ttl: Duration,
}

impl RunApprover {
    pub fn new(bus: EventBus, ctx: RunContext, ended: CancellationToken) -> Self {
        Self {
            bus,
            ctx,
            ended,
            ttl: Duration::from_millis(APPROVAL_TTL_MS as u64),
        }
    }

    #[cfg(test)]
    fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }
}

#[async_trait]
impl PermissionAsker for RunApprover {
    async fn ask(&self, call: AskedCall) -> bool {
        let planned = PlannedCall {
            id: format!("{CALL_PREFIX}{}", call.tool_call_id),
            tool: call.tool,
            args: call.args,
            requires_approval: true,
        };
        let (sender, receiver) = oneshot::channel();
        let approval_id = match self.bus.approval_request(&self.ctx, &planned).await {
            Ok(id) => id,
            Err(e) => {
                log::warn!("Couldn't ask to approve a harness call: {e}");
                return false;
            }
        };
        waiting().insert(approval_id.clone(), sender);
        let status = tokio::select! {
            decided = receiver => decided.unwrap_or(ApprovalStatus::Expired),
            () = tokio::time::sleep(self.ttl) => ApprovalStatus::Expired,
            // `turn_done` expires the run's pending approvals
            () = self.ended.cancelled() => {
                waiting().remove(&approval_id);
                return false;
            }
        };
        waiting().remove(&approval_id);
        let id = approval_id.clone();
        let row = self
            .bus
            .db
            .call(move |conn| db::get_approval(conn, &id))
            .await;
        match row {
            Ok(Some(row)) if row.status == ApprovalStatus::Pending => {
                if let Err(e) = self.bus.approval_closed(&self.ctx, row, status).await {
                    log::warn!("Couldn't close approval {approval_id}: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("Couldn't read approval {approval_id}: {e}"),
        }
        status == ApprovalStatus::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RunStatus;
    use crate::services::events::tests::{bus, started};

    async fn new_approver(ttl: Duration) -> (RunApprover, CancellationToken) {
        let (bus, _sink) = bus();
        let ctx = started(&bus, &uuid::Uuid::new_v4().to_string()).await;
        let ended = CancellationToken::new();
        (
            RunApprover::new(bus, ctx, ended.clone()).with_ttl(ttl),
            ended,
        )
    }

    fn call(id: &str) -> AskedCall {
        AskedCall {
            tool_call_id: id.into(),
            tool: "shell".into(),
            args: serde_json::json!({"title": "Run git push"}),
        }
    }

    async fn pending(approver: &RunApprover) -> ApprovalRow {
        let run_id = approver.ctx.run_id.clone();
        for _ in 0..400 {
            let id = run_id.clone();
            let rows = approver
                .bus
                .db
                .call(move |conn| db::pending_approvals_for_run(conn, &id))
                .await
                .unwrap();
            if let Some(row) = rows.into_iter().next() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no approval was asked");
    }

    async fn run_status(approver: &RunApprover) -> RunStatus {
        let id = approver.ctx.run_id.clone();
        approver
            .bus
            .db
            .call(move |conn| db::get_run(conn, &id))
            .await
            .unwrap()
            .unwrap()
            .status
    }

    #[tokio::test]
    async fn an_approved_call_runs_and_the_run_goes_on() {
        let (approver, _ended) = new_approver(Duration::from_secs(60)).await;
        let approver = std::sync::Arc::new(approver);
        let asking = tokio::spawn({
            let approver = approver.clone();
            async move { approver.ask(call("t1")).await }
        });
        let row = pending(&approver).await;
        assert!(is_harness(&row));
        assert_eq!(row.tool, "shell");
        assert_eq!(run_status(&approver).await, RunStatus::AwaitingApproval);

        assert!(decide(&row.id, ApprovalDecision::Approved));
        assert!(asking.await.unwrap());
        assert_eq!(run_status(&approver).await, RunStatus::Running);
        assert!(!decide(&row.id, ApprovalDecision::Approved), "decided once");
    }

    #[tokio::test]
    async fn rejected_and_expired_calls_dont_run() {
        let (approver, _ended) = new_approver(Duration::from_secs(60)).await;
        let approver = std::sync::Arc::new(approver);
        let asking = tokio::spawn({
            let approver = approver.clone();
            async move { approver.ask(call("t2")).await }
        });
        let row = pending(&approver).await;
        assert!(decide(&row.id, ApprovalDecision::Rejected));
        assert!(!asking.await.unwrap());

        let (quick, _ended) = new_approver(Duration::from_millis(50)).await;
        assert!(!quick.ask(call("t3")).await, "nobody decided in time");
        let id = format!("{CALL_PREFIX}t3");
        let run_id = quick.ctx.run_id.clone();
        let rows = quick
            .bus
            .db
            .call(move |conn| db::pending_approvals_for_run(conn, &run_id))
            .await
            .unwrap();
        assert!(rows.iter().all(|r| r.tool_call_id != id), "expired");
    }

    #[tokio::test]
    async fn a_run_that_ends_stops_waiting() {
        let (approver, ended) = new_approver(Duration::from_secs(60)).await;
        let approver = std::sync::Arc::new(approver);
        let asking = tokio::spawn({
            let approver = approver.clone();
            async move { approver.ask(call("t4")).await }
        });
        let row = pending(&approver).await;
        ended.cancel();
        assert!(!asking.await.unwrap());
        assert!(!is_waiting(&row.id));
    }
}
