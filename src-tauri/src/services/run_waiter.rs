//! Waiting for a run to settle.
//!
//! A run that reaches its context limit compacts: it ends as `compacted` and
//! goes on in a child run that names it in `runs.parent_run_id`
//! (`orchestrator::compaction`). So a run is settled only when the newest run
//! in that line has finished. Everything that waits on an agent run
//! (workflow steps, delegated tasks, a Request's steps) asks here.

use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::broadcast::{error::RecvError, Receiver};

use crate::db::{self, role, Db};
use crate::model::{RunStatus, RunSummary};
use crate::services::events::{AgentTurnDone, NandiEvent, WireEvent};

/// The newest run in `run_id`'s line: the run itself, or the latest child a
/// compaction left it going on in. `None` when there is no such run.
pub fn newest_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunSummary>> {
    let Some(mut run) = db::get_run(conn, run_id)? else {
        return Ok(None);
    };
    while run.status == RunStatus::Compacted {
        match db::child_runs(conn, &run.id)?.pop() {
            Some(child) => run = child,
            // The child starts in the same transaction; none yet is a race
            None => break,
        }
    }
    Ok(Some(run))
}

/// The last thing the agent said in `run_id`, or `""`
pub fn final_reply(conn: &Connection, run_id: &str) -> rusqlite::Result<String> {
    Ok(db::list_run_messages(conn, run_id)?
        .into_iter()
        .rev()
        .find(|m| m.role == role::ASSISTANT && !m.content.trim().is_empty())
        .map(|m| m.content)
        .unwrap_or_default())
}

/// Waits for runs to settle: wakes on `agent://turn_done`, and re-checks the
/// database after `recheck` of silence or when the events lag behind
pub struct RunWaiter {
    db: Db,
    events: Receiver<WireEvent>,
    recheck: Duration,
}

impl RunWaiter {
    /// `events` is subscribed before the run starts, so a turn that ends in
    /// between is never missed
    pub fn new(db: Db, events: Receiver<WireEvent>, recheck: Duration) -> Self {
        Self {
            db,
            events,
            recheck,
        }
    }

    /// The newest run in `run_id`'s line once it has finished, and its final
    /// reply
    pub async fn until_settled(&mut self, run_id: &str) -> Result<(RunSummary, String), String> {
        loop {
            let id = run_id.to_string();
            let (newest, settled) = self
                .db
                .call(move |conn| {
                    let Some(run) = newest_run(conn, &id)? else {
                        return Ok(None);
                    };
                    let settled = run.status.is_finished() && run.status != RunStatus::Compacted;
                    let reply = if settled {
                        Some(final_reply(conn, &run.id)?)
                    } else {
                        None
                    };
                    Ok(Some((run, reply)))
                })
                .await?
                .ok_or_else(|| format!("Run {run_id} not found"))?;
            if let Some(reply) = settled {
                return Ok((newest, reply));
            }
            self.turn_done(&newest.id).await?;
        }
    }

    /// Until `run_id`'s turn ends, `recheck` passes, or the events lag
    async fn turn_done(&mut self, run_id: &str) -> Result<(), String> {
        loop {
            match tokio::time::timeout(self.recheck, self.events.recv()).await {
                Ok(Ok(event)) => {
                    if event.event == AgentTurnDone::NAME && event.payload["runId"] == run_id {
                        return Ok(());
                    }
                }
                Ok(Err(RecvError::Lagged(_))) | Err(_) => return Ok(()),
                Ok(Err(RecvError::Closed)) => return Err("Event bus closed".to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::start_run;
    use crate::db::MessageRow;
    use crate::services::events::BroadcastSink;

    const RECHECK: Duration = Duration::from_millis(20);

    /// A waiter on `db`, and the events it hears (kept open: a closed bus
    /// ends the wait)
    fn waiter(db: &Db) -> (RunWaiter, BroadcastSink) {
        let events = BroadcastSink::new();
        let waiter = RunWaiter::new(db.clone(), events.subscribe(), RECHECK);
        (waiter, events)
    }

    /// Compacts `parent` into a new running child `child`
    fn compact(conn: &mut Connection, parent: &str, child: &str) {
        let mut run = db::get_run(conn, parent).unwrap().unwrap();
        run.id = child.to_string();
        run.status = RunStatus::Running;
        run.parent_run_id = Some(parent.to_string());
        db::start_compacted_child(conn, &run, &[]).unwrap();
    }

    /// Ends `run_id` as completed, its agent having said `reply`
    fn finish(conn: &Connection, run_id: &str, reply: &str) {
        let run = db::get_run(conn, run_id).unwrap().unwrap();
        let message = MessageRow::new(
            format!("{run_id}-reply"),
            &run.conversation_id,
            Some(run_id),
            role::ASSISTANT,
            reply,
            serde_json::json!({}),
        );
        db::insert_message(conn, &message).unwrap();
        db::set_run_status(conn, run_id, RunStatus::Completed, None).unwrap();
    }

    #[test]
    fn the_newest_run_follows_every_compaction() {
        let db = Db::in_memory();
        let mut conn = db.conn().unwrap();
        start_run(&conn, "writer", "r1");
        assert_eq!(newest_run(&conn, "r1").unwrap().unwrap().id, "r1");
        compact(&mut conn, "r1", "r2");
        compact(&mut conn, "r2", "r3");
        assert_eq!(newest_run(&conn, "r1").unwrap().unwrap().id, "r3");
        assert!(newest_run(&conn, "missing").unwrap().is_none());
    }

    #[tokio::test]
    async fn a_compacted_run_settles_when_its_child_finishes() {
        let db = Db::in_memory();
        {
            let mut conn = db.conn().unwrap();
            start_run(&conn, "writer", "r1");
            compact(&mut conn, "r1", "r2");
        }
        let (mut waiting, _events) = waiter(&db);
        let wait = waiting.until_settled("r1");
        tokio::pin!(wait);
        // Still going on in the child: not settled
        assert!(tokio::time::timeout(RECHECK * 5, &mut wait).await.is_err());

        finish(&db.conn().unwrap(), "r2", "all done");
        let (run, reply) = wait.await.unwrap();
        assert_eq!(run.id, "r2");
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(reply, "all done");
    }

    #[tokio::test]
    async fn an_unknown_run_is_an_error() {
        let db = Db::in_memory();
        let (mut waiting, _events) = waiter(&db);
        let error = waiting.until_settled("missing").await.unwrap_err();
        assert!(error.contains("missing"), "{error}");
    }
}
