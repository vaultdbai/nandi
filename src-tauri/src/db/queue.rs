//! Each agent's queue of waiting turns, by priority then age.

use super::*;

// ---------------------------------------------------------------------------
// Agent queue
// ---------------------------------------------------------------------------

/// A queued request: everything needed to start its run later.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueRow {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub source_type: SourceType,
    pub source_id: Option<String>,
    pub source_label: Option<String>,
    pub priority: i32,
    pub message: String,
    pub message_id: String,
    pub attachments: Vec<AttachmentInfo>,
    pub created_at: i64,
}

impl QueueRow {
    fn to_entry(&self, position: u32) -> QueueEntry {
        QueueEntry {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            source_type: self.source_type,
            source_id: self.source_id.clone(),
            source_label: self.source_label.clone(),
            priority: self.priority,
            message: self.message.clone(),
            attachments: self.attachments.clone(),
            created_at: ts(self.created_at),
            position,
        }
    }
}

const QUEUE_COLUMNS: &str = "id, agent_id, run_id, source_type, source_id, source_label, priority,
     message, message_id, attachments, created_at";
/// Next to run first. `rowid` breaks ties within the same millisecond.
const QUEUE_ORDER: &str = "priority ASC, created_at ASC, rowid ASC";

fn queue_row(row: &Row) -> rusqlite::Result<QueueRow> {
    let attachments: Option<String> = row.get("attachments")?;
    Ok(QueueRow {
        id: row.get("id")?,
        agent_id: row.get("agent_id")?,
        run_id: row.get("run_id")?,
        source_type: text_enum(row, "source_type")?,
        source_id: row.get("source_id")?,
        source_label: row.get("source_label")?,
        priority: row.get("priority")?,
        message: row.get("message")?,
        message_id: row.get("message_id")?,
        attachments: attachments
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
        created_at: row.get("created_at")?,
    })
}

pub fn insert_queue_row(conn: &Connection, entry: &QueueRow) -> rusqlite::Result<()> {
    let attachments = (!entry.attachments.is_empty())
        .then(|| serde_json::to_string(&entry.attachments).unwrap_or_default());
    conn.execute(
        &format!(
            "INSERT INTO agent_queue ({QUEUE_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ),
        params![
            entry.id,
            entry.agent_id,
            entry.run_id,
            entry.source_type.as_str(),
            entry.source_id,
            entry.source_label,
            entry.priority,
            entry.message,
            entry.message_id,
            attachments,
            entry.created_at,
        ],
    )?;
    renumber_queue(conn, &entry.agent_id)
}

fn queue_rows(conn: &Connection, agent_id: &str) -> rusqlite::Result<Vec<QueueRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {QUEUE_COLUMNS} FROM agent_queue WHERE agent_id = ?1 ORDER BY {QUEUE_ORDER}"
    ))?;
    let rows = stmt.query_map([agent_id], queue_row)?;
    rows.collect()
}

/// The agent's queue, next to run first.
pub fn list_queue(conn: &Connection, agent_id: &str) -> rusqlite::Result<Vec<QueueEntry>> {
    Ok(queue_rows(conn, agent_id)?
        .iter()
        .enumerate()
        .map(|(position, row)| row.to_entry(position as u32))
        .collect())
}

pub fn queue_is_empty(conn: &Connection, agent_id: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM agent_queue WHERE agent_id = ?1 LIMIT 1")?;
    Ok(!stmt.exists([agent_id])?)
}

pub fn queued_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<QueueRow>> {
    conn.query_row(
        &format!("SELECT {QUEUE_COLUMNS} FROM agent_queue WHERE run_id = ?1"),
        [run_id],
        queue_row,
    )
    .optional()
}

/// Removes and returns the agent's next request: lowest priority value,
/// then oldest.
pub fn pop_queue(conn: &Connection, agent_id: &str) -> rusqlite::Result<Option<QueueRow>> {
    let next = conn
        .query_row(
            &format!(
                "SELECT {QUEUE_COLUMNS} FROM agent_queue WHERE agent_id = ?1
                  ORDER BY {QUEUE_ORDER} LIMIT 1"
            ),
            [agent_id],
            queue_row,
        )
        .optional()?;
    if let Some(row) = &next {
        delete_queue_row(conn, row)?;
    }
    Ok(next)
}

/// Removes a queued request. Returns it, or `None` if it isn't queued.
pub fn remove_queued(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<QueueRow>> {
    let row = queued_run(conn, run_id)?;
    if let Some(row) = &row {
        delete_queue_row(conn, row)?;
    }
    Ok(row)
}

fn delete_queue_row(conn: &Connection, row: &QueueRow) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM agent_queue WHERE id = ?1", [&row.id])?;
    conn.execute(
        "UPDATE runs SET queue_position = NULL WHERE id = ?1",
        [&row.run_id],
    )?;
    renumber_queue(conn, &row.agent_id)
}

/// Changes a queued request's priority. Returns whether it was queued.
pub fn set_queue_priority(
    conn: &Connection,
    run_id: &str,
    priority: i32,
) -> rusqlite::Result<bool> {
    let Some(row) = queued_run(conn, run_id)? else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE agent_queue SET priority = ?2 WHERE id = ?1",
        params![row.id, priority],
    )?;
    conn.execute(
        "UPDATE runs SET priority = ?2 WHERE id = ?1",
        params![run_id, priority],
    )?;
    renumber_queue(conn, &row.agent_id)?;
    Ok(true)
}

/// Mirrors each queued request's position onto its run.
fn renumber_queue(conn: &Connection, agent_id: &str) -> rusqlite::Result<()> {
    for (position, row) in queue_rows(conn, agent_id)?.iter().enumerate() {
        conn.execute(
            "UPDATE runs SET queue_position = ?2 WHERE id = ?1",
            params![row.run_id, position as i64],
        )?;
    }
    Ok(())
}

/// Marks a queued run as started now.
pub fn start_queued_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunSummary>> {
    conn.execute(
        "UPDATE runs SET status = 'running', started_at = ?2, queue_position = NULL WHERE id = ?1",
        params![run_id, now_ms()],
    )?;
    get_run(conn, run_id)
}
