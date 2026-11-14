//! Approvals a run waits on, and their decisions.

use super::*;

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRow {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub message_id: String,
    pub tool_call_id: String,
    pub tool: String,
    pub args: Value,
    pub status: ApprovalStatus,
    pub created_at: i64,
    /// Epoch ms; `None` never expires (escalated clarifications have their
    /// own deadline)
    pub expires_at: Option<i64>,
    /// Who approved or rejected it (`set_approval_decider`)
    pub decided_by: Option<String>,
}

const APPROVAL_COLUMNS: &str = "id, agent_id, run_id, message_id, tool_call_id, tool, args, \
     status, created_at, expires_at, decided_by";

fn approval_row(row: &Row) -> rusqlite::Result<ApprovalRow> {
    Ok(ApprovalRow {
        id: row.get("id")?,
        agent_id: row.get("agent_id")?,
        run_id: row.get("run_id")?,
        message_id: row.get("message_id")?,
        tool_call_id: row.get("tool_call_id")?,
        tool: row.get("tool")?,
        args: json_col(row, "args")?,
        status: text_enum(row, "status")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
        decided_by: row.get("decided_by")?,
    })
}

/// A closed approval as the run trail keeps it (`services::run_trail`):
/// the whole row, with who decided it and when it closed
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecidedApproval {
    #[serde(flatten)]
    pub approval: ApprovalRow,
    /// Epoch ms
    pub resolved_at: Option<i64>,
}

/// `id` as it stands, with when it closed. `None`: no such approval.
pub fn decided_approval(conn: &Connection, id: &str) -> rusqlite::Result<Option<DecidedApproval>> {
    conn.query_row(
        &format!("SELECT {APPROVAL_COLUMNS}, resolved_at FROM approvals WHERE id = ?1"),
        [id],
        |row| {
            Ok(DecidedApproval {
                approval: approval_row(row)?,
                resolved_at: row.get("resolved_at")?,
            })
        },
    )
    .optional()
}

/// Puts back a decision from the run trail. A later decision of the same
/// approval replaces an earlier one.
pub fn restore_approval(conn: &Connection, decided: &DecidedApproval) -> rusqlite::Result<()> {
    let approval = &decided.approval;
    conn.execute(
        "INSERT OR REPLACE INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool,
             args, status, created_at, resolved_at, expires_at, decided_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            approval.id,
            approval.agent_id,
            approval.run_id,
            approval.message_id,
            approval.tool_call_id,
            approval.tool,
            approval.args.to_string(),
            approval.status.as_str(),
            approval.created_at,
            decided.resolved_at,
            approval.expires_at,
            approval.decided_by,
        ],
    )?;
    Ok(())
}

/// Records who decided `id`, before the decision closes it
pub fn set_approval_decider(conn: &Connection, id: &str, who: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE approvals SET decided_by = ?2 WHERE id = ?1",
        params![id, who],
    )?;
    Ok(())
}

pub fn insert_approval(conn: &Connection, approval: &ApprovalRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool, args, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            approval.id,
            approval.agent_id,
            approval.run_id,
            approval.message_id,
            approval.tool_call_id,
            approval.tool,
            approval.args.to_string(),
            approval.status.as_str(),
            approval.created_at,
            approval.expires_at,
        ],
    )?;
    Ok(())
}

/// Pending approvals whose deadline has passed, oldest first
pub fn overdue_approvals(conn: &Connection, now_ms: i64) -> rusqlite::Result<Vec<ApprovalRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM approvals
          WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at <= ?1
          ORDER BY created_at"
    ))?;
    let rows = stmt.query_map([now_ms], approval_row)?;
    rows.collect()
}

pub fn get_approval(conn: &Connection, id: &str) -> rusqlite::Result<Option<ApprovalRow>> {
    conn.query_row(
        &format!("SELECT {APPROVAL_COLUMNS} FROM approvals WHERE id = ?1"),
        [id],
        approval_row,
    )
    .optional()
}

/// The approval of tool call `tool_call_id` in run `run_id`. A provider's
/// call IDs are only unique within its answer (a local server may number
/// every answer's calls `call_0`, `call_1`), so never one of another run.
pub fn approval_for_call(
    conn: &Connection,
    run_id: &str,
    tool_call_id: &str,
) -> rusqlite::Result<Option<ApprovalRow>> {
    conn.query_row(
        &format!(
            "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ?1 AND tool_call_id = ?2"
        ),
        [run_id, tool_call_id],
        approval_row,
    )
    .optional()
}

/// Sets an approval's status and mirrors it onto its chat message.
pub fn set_approval_status(
    conn: &Connection,
    approval: &ApprovalRow,
    status: ApprovalStatus,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE approvals SET status = ?2, resolved_at = ?3 WHERE id = ?1",
        params![approval.id, status.as_str(), now_ms()],
    )?;
    update_message_metadata(
        conn,
        &approval.message_id,
        ApprovalMeta {
            tool: approval.tool.clone(),
            args: approval.args.clone(),
            status,
            expires_at: approval.expires_at,
        },
    )
}

/// Pending approvals, oldest first, optionally for one agent.
pub fn pending_approvals(
    conn: &Connection,
    agent_id: Option<&str>,
) -> rusqlite::Result<Vec<PendingApproval>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.agent_id, a.run_id, r.conversation_id, a.tool, a.args, a.created_at, a.expires_at
           FROM approvals a JOIN runs r ON r.id = a.run_id
          WHERE a.status = 'pending' AND (?1 IS NULL OR a.agent_id = ?1)
          ORDER BY a.created_at",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok(PendingApproval {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            run_id: row.get(2)?,
            conversation_id: row.get(3)?,
            tool: row.get(4)?,
            args: json_col(row, 5)?,
            created_at: ts(row.get(6)?),
            expires_at: row.get::<_, Option<i64>>(7)?.map(ts),
        })
    })?;
    rows.collect()
}

/// Every approval in a conversation, by the tool call it gates.
pub fn approvals_by_call(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<std::collections::HashMap<String, ApprovalStatus>> {
    let mut stmt = conn.prepare(
        "SELECT a.tool_call_id, a.status FROM approvals a JOIN runs r ON r.id = a.run_id
          WHERE r.conversation_id = ?1",
    )?;
    let rows = stmt.query_map([conversation_id], |row| {
        Ok((
            row.get::<_, String>("tool_call_id")?,
            text_enum(row, "status")?,
        ))
    })?;
    rows.collect()
}

/// Every approval of `run_id` for one synthetic tool (connector access
/// requests), oldest first
pub fn run_approvals_for_tool(
    conn: &Connection,
    run_id: &str,
    tool: &str,
) -> rusqlite::Result<Vec<ApprovalRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ?1 AND tool = ?2 ORDER BY created_at"
    ))?;
    let rows = stmt.query_map(params![run_id, tool], approval_row)?;
    rows.collect()
}

/// Replaces an approval's arguments (an access request records the choice
/// made) and mirrors them onto its chat message
pub fn set_approval_args(
    conn: &Connection,
    approval: &ApprovalRow,
    args: &Value,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE approvals SET args = ?2 WHERE id = ?1",
        params![approval.id, args.to_string()],
    )?;
    update_message_metadata(
        conn,
        &approval.message_id,
        ApprovalMeta {
            tool: approval.tool.clone(),
            args: args.clone(),
            status: approval.status,
            expires_at: approval.expires_at,
        },
    )
}

pub fn pending_approvals_for_run(
    conn: &Connection,
    run_id: &str,
) -> rusqlite::Result<Vec<ApprovalRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ?1 AND status = 'pending'"
    ))?;
    let rows = stmt.query_map([run_id], approval_row)?;
    rows.collect()
}
