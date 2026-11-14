//! Workflow runs in SQLite: the rows, their reads and writes, and moving a
//! run from one status to another.

use super::*;

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

pub(super) const RUN_COLUMNS: &str = "id, workflow_id, status, started_at, ended_at, context";

/// A bad column fails the read and names the run (`db::named_row`)
pub(super) fn run_row(row: &Row) -> rusqlite::Result<WorkflowRun> {
    db::named_row(row, "workflow_runs", 0, read_run_row)
}

fn read_run_row(row: &Row) -> rusqlite::Result<WorkflowRun> {
    let context: RunContext = db::json_col(row, 5)?;
    Ok(WorkflowRun {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        status: db::text_enum(row, 2)?,
        started_at: db::ts(row.get(3)?),
        ended_at: row.get::<_, Option<i64>>(4)?.map(db::ts),
        steps: context.steps,
        error: context.error,
        source: context.source,
    })
}

pub(super) fn insert_run(conn: &Connection, run: &WorkflowRun) -> rusqlite::Result<()> {
    conn.execute(
        &format!("INSERT INTO workflow_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
        params![
            run.id,
            run.workflow_id,
            run.status.as_str(),
            run.started_at as i64,
            run.ended_at.map(|t| t as i64),
            context_json(run),
        ],
    )?;
    Ok(())
}

/// The driver saves its in-memory copy after each step; a pause or cancel
/// made meanwhile (Work) wins over its `running`
pub(super) fn update_run(conn: &Connection, run: &WorkflowRun) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_runs SET
           status = CASE WHEN status IN ('paused', 'cancelled') AND ?2 = 'running'
                         THEN status ELSE ?2 END,
           ended_at = CASE WHEN status = 'cancelled' AND ?2 = 'running'
                           THEN ended_at ELSE ?3 END,
           context = ?4
         WHERE id = ?1",
        params![
            run.id,
            run.status.as_str(),
            run.ended_at.map(|t| t as i64),
            context_json(run),
        ],
    )?;
    Ok(())
}

fn context_json(run: &WorkflowRun) -> String {
    serde_json::to_string(&RunContext {
        steps: run.steps.clone(),
        error: run.error.clone(),
        source: run.source.clone(),
    })
    .unwrap_or_else(|_| "{}".to_string())
}

pub(super) fn get_run(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkflowRun>> {
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM workflow_runs WHERE id = ?1"),
        [id],
        run_row,
    )
    .optional()
}

pub(super) fn list_runs(
    conn: &Connection,
    workflow_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<WorkflowRun>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM workflow_runs WHERE workflow_id = ?1
         ORDER BY started_at DESC, rowid DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![workflow_id, limit], run_row)?;
    rows.collect()
}

pub(super) fn waiting_run_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM workflow_runs WHERE status = 'waiting'")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub(super) fn running_run_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM workflow_runs WHERE status = 'running'")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub(super) fn workflow_name(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT name FROM workflows WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .optional()
}

impl WorkflowEngine {
    /// Moves a run from one of `from` to `to` (atomically), returning it
    pub(super) async fn transition(
        &self,
        run_id: &str,
        from: &[WorkflowRunStatus],
        to: WorkflowRunStatus,
    ) -> Result<WorkflowRun, String> {
        let id = run_id.to_string();
        let from: Vec<&'static str> = from.iter().map(|s| s.as_str()).collect();
        let ended = matches!(to, WorkflowRunStatus::Cancelled).then(db::now_ms);
        self.db
            .call(move |conn| {
                let placeholders = (0..from.len())
                    .map(|i| format!("?{}", i + 4))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut values: Vec<rusqlite::types::Value> = vec![
                    id.clone().into(),
                    to.as_str().to_string().into(),
                    ended.into(),
                ];
                values.extend(from.iter().map(|s| s.to_string().into()));
                let changed = conn.execute(
                    &format!(
                        "UPDATE workflow_runs SET status = ?2, ended_at = COALESCE(?3, ended_at)
                         WHERE id = ?1 AND status IN ({placeholders})"
                    ),
                    rusqlite::params_from_iter(values),
                )?;
                let run = get_run(conn, &id)?;
                Ok((changed, run))
            })
            .await
            .and_then(|(changed, run)| {
                let run = run.ok_or_else(|| format!("Workflow run {run_id} not found"))?;
                if changed == 0 {
                    return Err(format!(
                        "This run is {}, so it can't be {}",
                        run.status.as_str().replace('_', " "),
                        match to {
                            WorkflowRunStatus::Paused => "paused",
                            WorkflowRunStatus::Running => "resumed",
                            _ => "cancelled",
                        }
                    ));
                }
                Ok(run)
            })
    }

    pub(super) async fn save(&self, run: &WorkflowRun) -> Result<(), String> {
        let run = run.clone();
        self.db.call(move |conn| update_run(conn, &run)).await
    }
}
