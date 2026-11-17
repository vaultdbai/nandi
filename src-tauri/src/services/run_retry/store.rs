//! The retry columns of `runs` (`retry_of`, `attempt`; migration 007).

use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::model::{RunStatus, RunSummary, SourceType};
use crate::services::events::NandiEvent;
use crate::services::orchestrator::middleware::AgentTurnAborted;

/// Hops followed back to a first try, so a broken line can't loop
const MAX_HOPS: usize = 32;

/// A failed run, and where it stands in its line of attempts
#[derive(Debug, Clone, PartialEq)]
pub struct FailedAttempt {
    pub run: RunSummary,
    /// 1 for the first try
    pub attempt: u32,
    /// The first try's input: what every retry asks again (a retry's own
    /// input carries Nandi's note)
    pub first_input: String,
}

/// Records that `run_id` is attempt `attempt`, trying `retry_of` again
pub fn link(conn: &Connection, run_id: &str, retry_of: &str, attempt: u32) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET retry_of = ?2, attempt = ?3 WHERE id = ?1",
        params![run_id, retry_of, attempt],
    )?;
    Ok(())
}

/// The run that tries `run_id` again, if one does
pub fn retry_for(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunSummary>> {
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM runs WHERE retry_of = ?1 ORDER BY attempt DESC, started_at DESC LIMIT 1",
            [run_id],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => db::get_run(conn, &id),
        None => Ok(None),
    }
}

/// `run_id`'s attempt and the run it tries again
fn position(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<(u32, Option<String>)>> {
    conn.query_row(
        "SELECT attempt, retry_of FROM runs WHERE id = ?1",
        [run_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// `run_id` as a failed attempt; `None` unless it failed
pub fn failed_attempt(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<FailedAttempt>> {
    let Some(run) = db::get_run(conn, run_id)? else {
        return Ok(None);
    };
    if run.status != RunStatus::Failed {
        return Ok(None);
    }
    let Some((attempt, mut back)) = position(conn, run_id)? else {
        return Ok(None);
    };
    let mut first_input = run.input.clone();
    for _ in 0..MAX_HOPS {
        let Some(previous) = back else {
            break;
        };
        let Some(earlier) = db::get_run(conn, &previous)? else {
            break;
        };
        first_input = earlier.input;
        back = position(conn, &previous)?.and_then(|(_, of)| of);
    }
    Ok(Some(FailedAttempt {
        run,
        attempt,
        first_input,
    }))
}

/// Whether a middleware stopped `run_id`: a turn, cost or token cap, or
/// read-only. Those are the user's limits, never tried again on its own.
pub fn stopped_by_policy(conn: &Connection, run_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM run_events WHERE run_id = ?1 AND kind = ?2)",
        params![run_id, AgentTurnAborted::NAME],
        |row| row.get(0),
    )
}

/// Failed runs from `sources` that ended since `since_ms`, that no run
/// tries again yet and no middleware stopped, oldest first: what the
/// sweep may try again
pub fn open_failures(
    conn: &Connection,
    since_ms: i64,
    sources: &[SourceType],
) -> rusqlite::Result<Vec<FailedAttempt>> {
    failures(conn, since_ms, sources, false)
}

/// Failed runs from `sources` that ended since `since_ms` and that no run
/// tries again, whether or not a middleware stopped them: the latest
/// attempt of each line that failed
pub fn latest_failures(
    conn: &Connection,
    since_ms: i64,
    sources: &[SourceType],
) -> rusqlite::Result<Vec<FailedAttempt>> {
    failures(conn, since_ms, sources, true)
}

fn failures(
    conn: &Connection,
    since_ms: i64,
    sources: &[SourceType],
    stopped_too: bool,
) -> rusqlite::Result<Vec<FailedAttempt>> {
    // The names are the enum's own, never input
    let sources = sources
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM runs
          WHERE status = 'failed' AND ended_at >= ?1 AND source_type IN ({sources})
            AND NOT EXISTS (SELECT 1 FROM runs n WHERE n.retry_of = runs.id)
            AND (?3 OR NOT EXISTS (SELECT 1 FROM run_events e
                                    WHERE e.run_id = runs.id AND e.kind = ?2))
          ORDER BY ended_at"
    ))?;
    let ids = stmt
        .query_map(
            params![since_ms, AgentTurnAborted::NAME, stopped_too],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut failures = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(failed) = failed_attempt(conn, &id)? {
            failures.push(failed);
        }
    }
    Ok(failures)
}
