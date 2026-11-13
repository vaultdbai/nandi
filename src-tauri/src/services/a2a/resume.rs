//! Restart resume (wave final, Track B): on boot, tasks still Submitted,
//! Delivered, Acknowledged, or Working are picked up again (provider: rerun
//! the turn, recorded in `task_events`; requester: re-subscribe and flush).
//!
//! Every resumed task gets a `task_events` row with `from_state =
//! to_state` and the reason `task_state::RESUMED_REASON` (the state
//! machine has no self-transitions, so the row's state doesn't change).
//! Provider side: the interrupted run was failed at boot
//! (`EventBus::fail_interrupted_runs`); the task runs again as a new turn
//! from the wire task stored with it, and its result goes back through the
//! result outbox. A provider row without one can't be rerun and ends.
//! Requester side: results arrive through the handlers `listen`
//! registered; queued tasks flush.

use std::sync::Arc;

use super::result_outbox;
use super::task::{self, TaskService, TaskState};
use super::task_state::{self, Change};

impl TaskService {
    /// Resumes in-flight tasks; returns their IDs. Provider reruns go on in
    /// the background.
    pub async fn resume(self: &Arc<Self>) -> Result<Vec<String>, String> {
        let rows = self
            .db()
            .call(|conn| {
                task::select(
                    conn,
                    "state IN ('submitted', 'delivered', 'acknowledged', 'working')
                     ORDER BY priority DESC, created_at ASC",
                    &[],
                )
            })
            .await?;
        let mut resumed = Vec::new();
        for row in rows {
            task_state::record_resume(self.db(), self.bus(), &row).await?;
            resumed.push(row.id.clone());
            if row.is_requester(self.instance_id()) {
                continue;
            }
            let Some(request) = task::request_of(self.db(), &row.id).await? else {
                let to = match row.state {
                    TaskState::Submitted => TaskState::Cancelled,
                    TaskState::Delivered => TaskState::Rejected,
                    _ => TaskState::Failed,
                };
                task_state::advance(
                    self.db(),
                    self.bus(),
                    &row.id,
                    to,
                    Change::error("Couldn't resume after a restart: the request is missing"),
                )
                .await?;
                continue;
            };
            let service = self.clone();
            self.runtime().spawn(async move {
                let result = service.executor.execute_resumed(&request).await;
                result_outbox::send(service.db(), service.relay(), result).await;
            });
        }
        let flushed = self.flush().await;
        if !resumed.is_empty() {
            log::info!(
                "Resumed {} tasks after a restart ({} sent from the outbox)",
                resumed.len(),
                flushed.len()
            );
        }
        Ok(resumed)
    }
}
