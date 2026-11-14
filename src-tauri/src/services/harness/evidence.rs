//! What a harness run leaves behind: a `harness_sessions` row, its
//! `harness_events`, and the run's usage and `runs.runtime`. Capability
//! evidence aggregates runs and tasks, not harness rows, so it covers
//! harness runs with no change: only the runtime column says which ran.

use rusqlite::{params, Connection, OptionalExtension};

use super::acp::{AcpSessionUpdate, AcpUsage};
use super::{HarnessEventRecord, HarnessSessionStatus, HarnessSessionSummary};
use crate::db::{self, now_ms, ts, Db};

/// Recorded events per session; later deltas are dropped (the run's own
/// messages keep the full text)
pub const MAX_EVENTS_PER_SESSION: u32 = 5_000;

/// A session row being opened
#[derive(Debug, Clone)]
pub struct SessionStart {
    pub id: String,
    pub adapter_id: String,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub acp_session_id: Option<String>,
    pub child_pid: Option<u32>,
    pub container_name: Option<String>,
    pub working_dir: String,
    pub model: Option<String>,
    pub provider: Option<String>,
}

pub async fn session_started(db: &Db, start: SessionStart) -> Result<(), String> {
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO harness_sessions (id, adapter_id, agent_id, run_id, acp_session_id,
                 status, child_pid, container_name, working_dir, model, provider, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                start.id,
                start.adapter_id,
                start.agent_id,
                start.run_id,
                start.acp_session_id,
                start.child_pid,
                start.container_name,
                start.working_dir,
                start.model,
                start.provider,
                now_ms(),
            ],
        )?;
        Ok(())
    })
    .await
}

/// Appends one update (bounded per session)
pub async fn record_event(
    db: &Db,
    session_id: &str,
    update: &AcpSessionUpdate,
) -> Result<(), String> {
    let (id, kind) = (session_id.to_string(), update.kind());
    let payload = serde_json::to_string(update).map_err(|e| e.to_string())?;
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO harness_events (session_id, seq, kind, payload, created_at)
             SELECT ?1, COALESCE(MAX(seq), 0) + 1, ?2, ?3, ?4
               FROM harness_events WHERE session_id = ?1
             HAVING COALESCE(MAX(seq), 0) < ?5",
            params![id, kind, payload, now_ms(), MAX_EVENTS_PER_SESSION],
        )?;
        Ok(())
    })
    .await
}

/// Closes the session row; a session already closed (killed) keeps its
/// status
pub async fn session_ended(
    db: &Db,
    session_id: &str,
    status: HarnessSessionStatus,
    stop_reason: Option<String>,
    error: Option<String>,
    usage: AcpUsage,
) -> Result<(), String> {
    let id = session_id.to_string();
    db.call(move |conn| {
        conn.execute(
            "UPDATE harness_sessions SET status = ?2, stop_reason = ?3, error = ?4,
                 input_tokens = ?5, output_tokens = ?6, ended_at = ?7
              WHERE id = ?1 AND status IN ('starting', 'running')",
            params![
                id,
                status.as_str(),
                stop_reason,
                error,
                usage.input_tokens,
                usage.output_tokens,
                now_ms()
            ],
        )?;
        Ok(())
    })
    .await
}

/// Marks sessions a previous app run left open (the CLIs died with it)
pub async fn close_orphaned_sessions(db: &Db) -> Result<usize, String> {
    db.call(|conn| {
        conn.execute(
            "UPDATE harness_sessions SET status = 'failed', ended_at = ?1,
                 error = COALESCE(error, 'Nandi stopped while the session ran')
              WHERE status IN ('starting', 'running')",
            [now_ms()],
        )
    })
    .await
}

/// Which runtime ran the run, and the turn's usage on the run's totals
pub async fn attribute_run(
    db: &Db,
    run_id: &str,
    runtime: &str,
    model: Option<&str>,
    usage: Option<AcpUsage>,
) -> Result<(), String> {
    let (id, runtime) = (run_id.to_string(), runtime.to_string());
    let model = model.map(str::to_string);
    db.call(move |conn| {
        conn.execute(
            "UPDATE runs SET runtime = ?2 WHERE id = ?1",
            params![id, runtime],
        )?;
        if let Some(usage) = usage {
            let model = model.unwrap_or_else(|| runtime.clone());
            db::add_run_usage(
                conn,
                &id,
                &model,
                usage.input_tokens,
                usage.output_tokens,
                usage.cost_usd,
            )?;
        }
        Ok(())
    })
    .await
}

const SESSION_COLUMNS: &str = "id, adapter_id, agent_id, run_id, status, child_pid,
    container_name, working_dir, model, provider, stop_reason, error, input_tokens,
    output_tokens, started_at, ended_at";

fn session_row(row: &rusqlite::Row) -> rusqlite::Result<HarnessSessionSummary> {
    Ok(HarnessSessionSummary {
        id: row.get(0)?,
        adapter_id: row.get(1)?,
        agent_id: row.get(2)?,
        run_id: row.get(3)?,
        status: db::text_enum(row, 4)?,
        child_pid: row.get(5)?,
        container_name: row.get(6)?,
        working_dir: row.get(7)?,
        model: row.get(8)?,
        provider: row.get(9)?,
        stop_reason: row.get(10)?,
        error: row.get(11)?,
        input_tokens: row.get(12)?,
        output_tokens: row.get(13)?,
        started_at: ts(row.get(14)?),
        ended_at: row.get::<_, Option<i64>>(15)?.map(ts),
    })
}

pub fn list_sessions(
    conn: &Connection,
    agent_id: Option<&str>,
    limit: u32,
) -> rusqlite::Result<Vec<HarnessSessionSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLUMNS} FROM harness_sessions
          WHERE ?1 IS NULL OR agent_id = ?1
          ORDER BY started_at DESC LIMIT ?2"
    ))?;
    let rows = stmt
        .query_map(params![agent_id, limit], session_row)?
        .collect();
    rows
}

pub fn get_session(conn: &Connection, id: &str) -> rusqlite::Result<Option<HarnessSessionSummary>> {
    conn.query_row(
        &format!("SELECT {SESSION_COLUMNS} FROM harness_sessions WHERE id = ?1"),
        [id],
        session_row,
    )
    .optional()
}

pub fn list_events(
    conn: &Connection,
    session_id: &str,
    after_seq: u32,
) -> rusqlite::Result<Vec<HarnessEventRecord>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, seq, kind, payload, created_at FROM harness_events
          WHERE session_id = ?1 AND seq > ?2 ORDER BY seq",
    )?;
    let rows = stmt
        .query_map(params![session_id, after_seq], |row| {
            let payload: String = row.get(3)?;
            Ok(HarnessEventRecord {
                session_id: row.get(0)?,
                seq: row.get(1)?,
                kind: row.get(2)?,
                payload: serde_json::from_str(&payload).unwrap_or_default(),
                created_at: ts(row.get(4)?),
            })
        })?
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(id: &str) -> SessionStart {
        SessionStart {
            id: id.into(),
            adapter_id: "claude-code".into(),
            agent_id: Some("writer".into()),
            run_id: Some("r1".into()),
            acp_session_id: Some("acp-1".into()),
            child_pid: Some(42),
            container_name: Some("nandi-harness-writer".into()),
            working_dir: "/tmp/w".into(),
            model: Some("claude-sonnet-5".into()),
            provider: Some("bedrock".into()),
        }
    }

    #[tokio::test]
    async fn a_session_is_recorded_with_its_events() {
        let db = Db::in_memory();
        session_started(&db, start("s1")).await.unwrap();
        for text in ["a", "b"] {
            let update = AcpSessionUpdate::MessageDelta { text: text.into() };
            record_event(&db, "s1", &update).await.unwrap();
        }
        let usage = AcpUsage {
            input_tokens: 7,
            output_tokens: 3,
            ..AcpUsage::default()
        };
        session_ended(
            &db,
            "s1",
            HarnessSessionStatus::Finished,
            Some("end_turn".into()),
            None,
            usage,
        )
        .await
        .unwrap();
        // A later end (e.g. a kill racing completion) doesn't overwrite it
        session_ended(
            &db,
            "s1",
            HarnessSessionStatus::Killed,
            None,
            None,
            AcpUsage::default(),
        )
        .await
        .unwrap();

        let (session, events, after) = db
            .call(|conn| {
                Ok((
                    get_session(conn, "s1")?,
                    list_events(conn, "s1", 0)?,
                    list_events(conn, "s1", 1)?,
                ))
            })
            .await
            .unwrap();
        let session = session.unwrap();
        assert_eq!(session.status, HarnessSessionStatus::Finished);
        assert_eq!(session.input_tokens, 7);
        assert_eq!(session.stop_reason.as_deref(), Some("end_turn"));
        assert!(session.ended_at.is_some());
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[0].kind, "messageDelta");
        assert_eq!(events[1].payload["text"], "b");
        assert_eq!(after.len(), 1);
    }

    #[tokio::test]
    async fn orphaned_sessions_are_closed_on_startup() {
        let db = Db::in_memory();
        session_started(&db, start("s1")).await.unwrap();
        assert_eq!(close_orphaned_sessions(&db).await.unwrap(), 1);
        let session = db
            .call(|conn| get_session(conn, "s1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.status, HarnessSessionStatus::Failed);
    }

    #[tokio::test]
    async fn sessions_list_newest_first_and_by_agent() {
        let db = Db::in_memory();
        session_started(&db, start("s1")).await.unwrap();
        let mut other = start("s2");
        other.agent_id = Some("other".into());
        session_started(&db, other).await.unwrap();
        let (all, writer) = db
            .call(|conn| {
                Ok((
                    list_sessions(conn, None, 10)?,
                    list_sessions(conn, Some("writer"), 10)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(writer.len(), 1);
        assert_eq!(writer[0].id, "s1");
    }
}
