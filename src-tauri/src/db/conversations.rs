//! Conversations: an agent's, a check's, a task's and Studio's.

use super::*;

// ---------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------

/// Each agent has one long-lived conversation with a deterministic ID.
pub fn agent_conversation_id(agent_id: &str) -> String {
    format!("agent:{agent_id}")
}

pub fn ensure_agent_conversation(conn: &Connection, agent_id: &str) -> rusqlite::Result<String> {
    let id = agent_conversation_id(agent_id);
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, NULL)",
        params![id, agent_id, now_ms()],
    )?;
    Ok(id)
}

/// `email:<check id>`: one email check's own conversation, so its turn
/// sees neither the user's chats nor earlier checks' mail, and later
/// chats never see this mail in their history
pub fn email_conversation_id(check_id: &str) -> String {
    format!("email:{check_id}")
}

pub fn ensure_email_conversation(
    conn: &Connection,
    agent_id: &str,
    check_id: &str,
    title: &str,
) -> rusqlite::Result<String> {
    let id = email_conversation_id(check_id);
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, agent_id, now_ms(), title],
    )?;
    Ok(id)
}

/// `calendar:<check id>`: one calendar check's own conversation
pub fn ensure_calendar_conversation(
    conn: &Connection,
    agent_id: &str,
    check_id: &str,
    title: &str,
) -> rusqlite::Result<String> {
    let id = format!("calendar:{check_id}");
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, agent_id, now_ms(), title],
    )?;
    Ok(id)
}

/// `task:<task id>`: a delegated task's own conversation, so its turn sees
/// neither the owner's chats with the agent nor other requesters' tasks
pub fn task_conversation_id(task_id: &str) -> String {
    format!("task:{task_id}")
}

pub fn ensure_task_conversation(
    conn: &Connection,
    agent_id: &str,
    task_id: &str,
    title: &str,
) -> rusqlite::Result<String> {
    let id = task_conversation_id(task_id);
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, agent_id, now_ms(), title],
    )?;
    Ok(id)
}

pub fn insert_studio_conversation(
    conn: &Connection,
    id: &str,
    title: &str,
) -> rusqlite::Result<StudioConversationSummary> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, STUDIO_AGENT_ID, now, title],
    )?;
    Ok(StudioConversationSummary {
        id: id.to_string(),
        title: title.to_string(),
        created_at: ts(now),
        updated_at: ts(now),
        last_message: None,
    })
}

/// Titles a conversation that has none yet. Returns whether it did.
pub fn set_title_if_untitled(conn: &Connection, id: &str, title: &str) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE conversations SET title = ?2 WHERE id = ?1 AND COALESCE(title, '') = ''",
        params![id, title],
    )?;
    Ok(changed > 0)
}

pub fn conversation_agent(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT agent_id FROM conversations WHERE id = ?1",
        [conversation_id],
        |row| row.get(0),
    )
    .optional()
}

pub fn list_studio_conversations(
    conn: &Connection,
) -> rusqlite::Result<Vec<StudioConversationSummary>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, COALESCE(c.title, ''), c.created_at,
                COALESCE((SELECT MAX(created_at) FROM messages m WHERE m.conversation_id = c.id), c.created_at),
                (SELECT content FROM messages m
                  WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
                  ORDER BY m.created_at DESC, m.rowid DESC LIMIT 1)
           FROM conversations c
          WHERE c.agent_id = ?1
          ORDER BY 4 DESC",
    )?;
    let rows = stmt.query_map([STUDIO_AGENT_ID], |row| {
        Ok(StudioConversationSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            created_at: ts(row.get(2)?),
            updated_at: ts(row.get(3)?),
            last_message: row.get(4)?,
        })
    })?;
    rows.collect()
}
