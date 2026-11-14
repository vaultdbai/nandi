//! Settings, agent state, the workflow list and the agent memory cache.

use super::*;

// ---------------------------------------------------------------------------
// Settings, agent state, workflows
// ---------------------------------------------------------------------------

/// Which turn engine runs agents: the LLM (default) or the canned script.
pub const ENGINE_KEY: &str = "engine";

pub fn uses_canned_engine(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(
        get_setting(conn, ENGINE_KEY)?.and_then(|v| v.as_str().map(|s| s == "canned"))
            == Some(true),
    )
}

/// A machine-local runtime setting (`local_settings`).
pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        "SELECT value FROM local_settings WHERE key = ?1",
        [key],
        |row| json_col(row, 0),
    )
    .optional()
}

/// Writes a machine-local runtime setting (`local_settings`).
pub fn set_setting(conn: &Connection, key: &str, value: &Value) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

/// Removes a machine-local runtime setting.
pub fn delete_setting(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM local_settings WHERE key = ?1", [key])?;
    Ok(())
}

pub fn set_agent_state(
    conn: &Connection,
    agent_id: &str,
    status: &str,
    error: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO agent_state (agent_id, status, error, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(agent_id) DO UPDATE SET status = excluded.status, error = excluded.error,
                                             updated_at = excluded.updated_at",
        params![agent_id, status, error, now_ms()],
    )?;
    Ok(())
}

/// Agents whose orchestrator is in the `error` state, with the error.
pub fn agents_in_error(conn: &Connection) -> rusqlite::Result<Vec<(String, Option<String>)>> {
    let mut stmt =
        conn.prepare("SELECT agent_id, error FROM agent_state WHERE status = 'error'")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

pub fn list_workflows(conn: &Connection) -> rusqlite::Result<Vec<WorkflowSummary>> {
    let mut stmt =
        conn.prepare("SELECT id, name, definition, created_at FROM workflows ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(WorkflowSummary {
            id: row.get(0)?,
            name: row.get(1)?,
            definition: json_col(row, 2)?,
            created_at: ts(row.get(3)?),
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Agent memory: a rebuildable cache of memory.md
// ---------------------------------------------------------------------------

/// Replaces the cached entries of `agent_id` with `entries`, in file order.
pub fn replace_agent_memory(
    conn: &mut Connection,
    agent_id: &str,
    entries: &[crate::services::agent_memory::MemoryEntry],
) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM agent_memory WHERE agent_id = ?1", [agent_id])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO agent_memory (agent_id, idx, content, created_at, source_run)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (idx, entry) in entries.iter().enumerate() {
            stmt.execute(params![
                agent_id,
                idx as i64,
                entry.content,
                entry.created_at as i64,
                entry.source_run,
            ])?;
        }
    }
    tx.commit()
}

/// The cached entries of `agent_id`, in file order.
#[cfg(test)]
pub fn list_agent_memory(
    conn: &Connection,
    agent_id: &str,
) -> rusqlite::Result<Vec<crate::services::agent_memory::MemoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT content, created_at, source_run FROM agent_memory
          WHERE agent_id = ?1 ORDER BY idx",
    )?;
    let rows = stmt.query_map([agent_id], |row| {
        Ok(crate::services::agent_memory::MemoryEntry {
            content: row.get(0)?,
            created_at: ts(row.get(1)?),
            source_run: row.get(2)?,
        })
    })?;
    rows.collect()
}
