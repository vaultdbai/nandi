//! Runs and their events.

use super::*;

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

const RUN_COLUMNS: &str =
    "id, agent_id, conversation_id, status, input, summary, started_at, ended_at,
     input_tokens, output_tokens, cost_usd, model,
     source_type, source_id, source_label, priority, queued_at,
     parent_run_id, compaction_summary";

fn run_row(row: &Row) -> rusqlite::Result<RunSummary> {
    Ok(RunSummary {
        id: row.get("id")?,
        agent_id: row.get("agent_id")?,
        conversation_id: row.get("conversation_id")?,
        status: text_enum(row, "status")?,
        input: row.get("input")?,
        summary: row.get("summary")?,
        started_at: ts(row.get("started_at")?),
        ended_at: row.get::<_, Option<i64>>("ended_at")?.map(ts),
        input_tokens: row.get("input_tokens")?,
        output_tokens: row.get("output_tokens")?,
        cost_usd: row.get("cost_usd")?,
        model: row.get("model")?,
        source_type: text_enum(row, "source_type")?,
        source_id: row.get("source_id")?,
        source_label: row.get("source_label")?,
        priority: row.get("priority")?,
        queued_at: row.get::<_, Option<i64>>("queued_at")?.map(ts),
        parent_run_id: row.get("parent_run_id")?,
        compaction_summary: row.get("compaction_summary")?,
    })
}

pub fn insert_run(conn: &Connection, run: &RunSummary) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO runs (id, agent_id, conversation_id, started_at, ended_at, status, input, summary,
                           source_type, source_id, source_label, priority, queued_at,
                           parent_run_id, compaction_summary)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            run.id,
            run.agent_id,
            run.conversation_id,
            run.started_at as i64,
            run.status.as_str(),
            run.input,
            run.source_type.as_str(),
            run.source_id,
            run.source_label,
            run.priority,
            run.queued_at.map(|t| t as i64),
            run.parent_run_id,
            run.compaction_summary,
        ],
    )?;
    Ok(())
}

/// The runs continuing `parent_run_id` after compaction, oldest first.
pub fn child_runs(conn: &Connection, parent_run_id: &str) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE parent_run_id = ?1 ORDER BY started_at, id"
    ))?;
    let rows = stmt.query_map([parent_run_id], run_row)?;
    rows.collect()
}

/// Ends `parent` as `compacted` and starts `child` (which must name it in
/// `parent_run_id`) with `messages`, in one transaction. Returns both runs
/// as updated.
pub fn start_compacted_child(
    conn: &mut Connection,
    child: &RunSummary,
    messages: &[MessageRow],
) -> rusqlite::Result<(RunSummary, RunSummary)> {
    let parent_id = child.parent_run_id.clone().unwrap_or_default();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let parent = set_run_status(&tx, &parent_id, RunStatus::Compacted, None)?
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    insert_run(&tx, child)?;
    for message in messages {
        insert_message(&tx, message)?;
    }
    let child = get_run(&tx, &child.id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    tx.commit()?;
    Ok((parent, child))
}

pub fn get_run(conn: &Connection, id: &str) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"),
        [id],
        run_row,
    )
    .optional()
}

/// Updates a run's status; terminal statuses also set `ended_at`. Returns
/// the updated run.
pub fn set_run_status(
    conn: &Connection,
    id: &str,
    status: RunStatus,
    summary: Option<&str>,
) -> rusqlite::Result<Option<RunSummary>> {
    let ended_at = status.is_finished().then(now_ms);
    conn.execute(
        "UPDATE runs SET status = ?2, ended_at = ?3, summary = COALESCE(?4, summary) WHERE id = ?1",
        params![id, status.as_str(), ended_at, summary],
    )?;
    get_run(conn, id)
}

/// Adds one model call's usage to the run's totals and records the model
/// (the latest call's). The cost stays NULL until a call with known
/// pricing is recorded.
pub fn add_run_usage(
    conn: &Connection,
    id: &str,
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    cost_usd: Option<f64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET input_tokens = input_tokens + ?2,
                         output_tokens = output_tokens + ?3,
                         cost_usd = CASE WHEN ?4 IS NULL THEN cost_usd
                                         ELSE COALESCE(cost_usd, 0) + ?4 END,
                         model = ?5
          WHERE id = ?1",
        params![id, input_tokens, output_tokens, cost_usd, model],
    )?;
    Ok(())
}

/// The conversation's running or approval-blocked run, if any.
pub fn active_run(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM runs
              WHERE conversation_id = ?1 AND status IN ('running', 'awaiting_approval')
              ORDER BY started_at DESC LIMIT 1"
        ),
        [conversation_id],
        run_row,
    )
    .optional()
}

/// The agent's current turn: its running or approval-blocked run, in any
/// of its conversations (its chat, or a delegated task's)
pub fn active_agent_run(conn: &Connection, agent_id: &str) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM runs
              WHERE agent_id = ?1 AND status IN ('running', 'awaiting_approval')
              ORDER BY started_at DESC LIMIT 1"
        ),
        [agent_id],
        run_row,
    )
    .optional()
}

/// Every running or approval-blocked run.
pub fn active_runs(conn: &Connection) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE status IN ('running', 'awaiting_approval')"
    ))?;
    let rows = stmt.query_map([], run_row)?;
    rows.collect()
}

/// Every running or approval-blocked run, and every run that ended at or
/// after `ended_since` (ms), Studio's left out: Ask's Live section.
pub fn live_runs(conn: &Connection, ended_since: i64) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs
          WHERE agent_id != ?2
            AND (status IN ('running', 'awaiting_approval')
                 OR (status IN ('completed', 'failed', 'cancelled') AND ended_at >= ?1))
          ORDER BY started_at DESC"
    ))?;
    let rows = stmt.query_map(params![ended_since, STUDIO_AGENT_ID], run_row)?;
    rows.collect()
}

/// Runs newest first, optionally for one agent. Studio runs are excluded
/// when no agent is given.
pub fn list_runs(
    conn: &Connection,
    agent_id: Option<&str>,
    limit: u32,
) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs
          WHERE (?1 IS NULL AND agent_id != ?3) OR agent_id = ?1
          ORDER BY started_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![agent_id, limit, STUDIO_AGENT_ID], run_row)?;
    rows.collect()
}

pub fn runs_since(conn: &Connection, since_ms: i64) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE started_at >= ?1 AND agent_id != ?2
          ORDER BY started_at DESC"
    ))?;
    let rows = stmt.query_map(params![since_ms, STUDIO_AGENT_ID], run_row)?;
    rows.collect()
}

/// Every run left `running`. At startup these are the runs a previous
/// process died in the middle of (see `EventBus::fail_interrupted_runs`).
pub fn running_runs(conn: &Connection) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE status = 'running'"
    ))?;
    let rows = stmt.query_map([], run_row)?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Run events
// ---------------------------------------------------------------------------

/// One `run_events` row, as stored and as the run trail keeps it
/// (`services::run_trail`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEventRow {
    pub id: i64,
    pub run_id: String,
    /// The event's name (`agent://tool_call`, ...)
    pub kind: String,
    pub payload: Value,
    /// Epoch ms
    pub created_at: i64,
}

/// Records an event of `run_id` and returns the row written
pub fn insert_run_event(
    conn: &Connection,
    run_id: &str,
    kind: &str,
    payload: &Value,
) -> rusqlite::Result<RunEventRow> {
    let created_at = now_ms();
    conn.execute(
        "INSERT INTO run_events (run_id, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![run_id, kind, payload.to_string(), created_at],
    )?;
    Ok(RunEventRow {
        id: conn.last_insert_rowid(),
        run_id: run_id.to_string(),
        kind: kind.to_string(),
        payload: payload.clone(),
        created_at,
    })
}

/// Puts back a row from the run trail with its own ID. A row already
/// there is kept.
pub fn restore_run_event(conn: &Connection, row: &RunEventRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO run_events (id, run_id, kind, payload, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            row.id,
            row.run_id,
            row.kind,
            row.payload.to_string(),
            row.created_at
        ],
    )?;
    Ok(())
}

/// Whether the tables the run trail rebuilds (`run_events`, `approvals`)
/// are both empty, as in a new `state.db`
pub fn run_trail_tables_empty(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM run_events) AND NOT EXISTS (SELECT 1 FROM approvals)",
        [],
        |row| row.get(0),
    )
}

/// The latest events across an agent's runs, oldest first.
pub fn list_agent_events(
    conn: &Connection,
    agent_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<RunEventRecord>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM (
            SELECT e.id, e.run_id, e.kind, e.payload, e.created_at
              FROM run_events e JOIN runs r ON r.id = e.run_id
             WHERE r.agent_id = ?1
             ORDER BY e.id DESC LIMIT ?2
         ) ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![agent_id, limit], |row| {
        Ok(RunEventRecord {
            id: row.get::<_, i64>(0)? as f64,
            run_id: row.get(1)?,
            kind: row.get(2)?,
            payload: json_col(row, 3)?,
            created_at: ts(row.get(4)?),
        })
    })?;
    rows.collect()
}
