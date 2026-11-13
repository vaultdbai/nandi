//! Provider side: the result outbox. A finished task's result is kept here
//! before it goes to the relay, sent again whenever the relay connects, and
//! dropped only when the relay says it will never take it: a refusal
//! naming the task with a final code ([`is_final`]).
//!
//! The relay sends nothing back for a result it stored. It answers the same
//! result sent again with `task_ended`, so a resend on the next connect is
//! how the outbox learns the result got there. A dropped connection, or
//! quitting between finishing and sending, loses nothing.

use std::sync::Arc;

use nandi_protocol::{NOT_A_PARTY, TASK_ENDED, UNKNOWN_TASK};
use rusqlite::{params, Connection};

use super::task::TaskService;
use super::TaskResult;
use crate::db::{self, Db};
use crate::services::relay::protocol::TaskRefused;
use crate::services::relay::RelayClient;

/// Whether the relay will never take a result after refusing its task with
/// `code`. `task_ended` is also its answer to a result it already has.
pub fn is_final(code: &str) -> bool {
    [TASK_ENDED, NOT_A_PARTY, UNKNOWN_TASK].contains(&code)
}

/// Keeps `result` (replacing an older one for its task), then sends it.
/// One that doesn't go out now waits for the next connect.
pub async fn send(db: &Db, relay: Option<&Arc<dyn RelayClient>>, result: TaskResult) {
    let task_id = result.task_id.clone();
    let kept = result.clone();
    if let Err(e) = db.call(move |conn| keep(conn, &kept, db::now_ms())).await {
        log::error!("Task {task_id}: couldn't keep the result until the relay has it: {e}");
    }
    let Some(relay) = relay else {
        return;
    };
    if let Err(e) = relay.send_result(result).await {
        log::info!("Task {task_id}: the result waits for the relay ({e})");
    }
}

/// Drops the result of the refused task when the refusal is final
pub async fn settle(db: &Db, refused: &TaskRefused) -> Result<(), String> {
    if !is_final(&refused.code) {
        return Ok(());
    }
    let task_id = refused.task_id.clone();
    let cleared = db.call(move |conn| clear(conn, &task_id)).await?;
    if cleared {
        log::info!(
            "Task {}: the relay is done with its result ({})",
            refused.task_id,
            refused.code
        );
    }
    Ok(())
}

impl TaskService {
    /// Sends every kept result again, oldest first, once the relay is
    /// connected; how many went out
    pub async fn flush_results(&self) -> usize {
        let Some(relay) = self.relay().cloned() else {
            return 0;
        };
        if !relay.status().connected {
            return 0;
        }
        let results = match self.db().call(waiting).await {
            Ok(results) => results,
            Err(e) => {
                log::warn!("Couldn't read the result outbox: {e}");
                return 0;
            }
        };
        let mut sent = 0;
        for result in results {
            let task_id = result.task_id.clone();
            match relay.send_result(result).await {
                Ok(()) => sent += 1,
                Err(e) => log::info!("Task {task_id}: the result waits for the relay ({e})"),
            }
        }
        sent
    }
}

fn keep(conn: &Connection, result: &TaskResult, at: i64) -> rusqlite::Result<()> {
    let json = serde_json::to_string(result)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT OR REPLACE INTO result_outbox (task_id, result, kept_at) VALUES (?1, ?2, ?3)",
        params![result.task_id, json, at],
    )?;
    Ok(())
}

/// Whether there was a result to drop
fn clear(conn: &Connection, task_id: &str) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM result_outbox WHERE task_id = ?1",
        params![task_id],
    )?;
    Ok(n > 0)
}

/// The kept results, oldest first. One that no longer reads is skipped.
pub(super) fn waiting(conn: &mut Connection) -> rusqlite::Result<Vec<TaskResult>> {
    let mut stmt =
        conn.prepare("SELECT task_id, result FROM result_outbox ORDER BY kept_at, rowid")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut results = Vec::new();
    for row in rows {
        let (task_id, json) = row?;
        match serde_json::from_str(&json) {
            Ok(result) => results.push(result),
            Err(e) => log::warn!("Task {task_id}: a kept result doesn't read: {e}"),
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::services::a2a::{DelegationDirection, TaskStatus};

    fn result(task_id: &str, output: &str) -> TaskResult {
        TaskResult {
            task_id: task_id.into(),
            provider_agent: "me/papers".into(),
            output: json!(output),
            status: TaskStatus::Completed,
            duration_ms: 1,
            tokens_in: 0,
            tokens_out: 0,
            error: None,
            direction: DelegationDirection::Inbound,
            capability: "summarize".into(),
            requester_agent: "them/writer".into(),
            provider_owner: None,
            requester_owner: None,
            input: json!("q"),
            started_at: 1.0,
            ended_at: Some(2.0),
        }
    }

    fn refused(task_id: &str, code: &str) -> TaskRefused {
        TaskRefused {
            task_id: task_id.into(),
            code: code.into(),
            message: "No".into(),
        }
    }

    async fn kept(db: &Db) -> Vec<(String, serde_json::Value)> {
        db.call(waiting)
            .await
            .unwrap()
            .into_iter()
            .map(|result| (result.task_id, result.output))
            .collect()
    }

    #[tokio::test]
    async fn a_result_with_no_relay_is_kept_oldest_first_and_replaced_per_task() {
        let db = Db::in_memory();
        send(&db, None, result("b", "first")).await;
        send(&db, None, result("a", "only")).await;
        send(&db, None, result("b", "again")).await;
        assert_eq!(
            kept(&db).await,
            [("a".into(), json!("only")), ("b".into(), json!("again"))]
        );
    }

    #[tokio::test]
    async fn only_a_final_refusal_drops_the_result() {
        // (the relay's code, whether the result is dropped)
        let cases = [
            (TASK_ENDED, true),
            (NOT_A_PARTY, true),
            (UNKNOWN_TASK, true),
            ("unauthorized", false),
            ("ledger_failed", false),
            ("rate_limited", false),
        ];
        for (code, dropped) in cases {
            let db = Db::in_memory();
            send(&db, None, result("t1", "done")).await;
            settle(&db, &refused("t1", code)).await.unwrap();
            assert_eq!(kept(&db).await.is_empty(), dropped, "{code}");
        }
    }

    #[tokio::test]
    async fn a_refusal_drops_only_its_own_task() {
        let db = Db::in_memory();
        send(&db, None, result("t1", "done")).await;
        send(&db, None, result("t2", "done")).await;
        settle(&db, &refused("t1", TASK_ENDED)).await.unwrap();
        assert_eq!(kept(&db).await, [("t2".into(), json!("done"))]);
    }
}
