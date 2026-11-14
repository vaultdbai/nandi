//! Messages and the metadata each role keeps on them.

use super::*;

// ---------------------------------------------------------------------------
// Message metadata (per role)
// ---------------------------------------------------------------------------

pub mod role {
    pub const USER: &str = "user";
    pub const ASSISTANT: &str = "assistant";
    pub const TOOL_CALL: &str = "tool_call";
    pub const APPROVAL: &str = "approval";
    pub const ERROR: &str = "error";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMeta {
    #[serde(default)]
    pub attachments: Vec<AttachmentInfo>,
}

/// A tool call the engine planned in an assistant step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedCall {
    pub id: String,
    pub tool: String,
    pub args: Value,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMeta {
    pub step: u32,
    #[serde(default)]
    pub planned_calls: Vec<PlannedCall>,
    /// The model's thinking for this step (replayed to DeepSeek)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallMeta {
    pub tool: String,
    pub args: Value,
    pub status: ToolCallStatus,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalMeta {
    pub tool: String,
    pub args: Value,
    pub status: ApprovalStatus,
    /// Epoch ms when a pending approval expires
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub metadata: Value,
}

impl MessageRow {
    pub fn new(
        id: impl Into<String>,
        conversation_id: &str,
        run_id: Option<&str>,
        role: &str,
        content: impl Into<String>,
        metadata: impl Serialize,
    ) -> Self {
        Self {
            id: id.into(),
            conversation_id: conversation_id.to_string(),
            run_id: run_id.map(str::to_string),
            role: role.to_string(),
            content: content.into(),
            created_at: now_ms(),
            metadata: serde_json::to_value(metadata).unwrap_or(Value::Null),
        }
    }

    /// A message Nandi added to keep a run going (`post_turn`'s
    /// `continue_with`, e.g. verify-on-stop). The model reads it as a user
    /// message, but the user didn't write it, so chats don't show it.
    pub fn is_continuation(&self) -> bool {
        self.role == role::USER && self.metadata["continuation"] == Value::Bool(true)
    }

    /// The UI shape of this row.
    pub fn to_chat(&self) -> ChatMessage {
        let id = self.id.clone();
        let run_id = self.run_id.clone();
        let created_at = ts(self.created_at);
        let meta = self.metadata.clone();
        match self.role.as_str() {
            role::USER => ChatMessage::User {
                id,
                run_id,
                created_at,
                content: self.content.clone(),
                attachments: serde_json::from_value::<UserMeta>(meta)
                    .unwrap_or_default()
                    .attachments,
            },
            role::TOOL_CALL => {
                let meta: ToolCallMeta =
                    serde_json::from_value(meta).unwrap_or_else(|_| ToolCallMeta {
                        tool: "unknown".into(),
                        args: Value::Null,
                        status: ToolCallStatus::Done,
                        result: None,
                        is_error: true,
                    });
                ChatMessage::ToolCall {
                    id,
                    run_id,
                    created_at,
                    tool: meta.tool,
                    args: meta.args,
                    status: meta.status,
                    result: meta.result,
                    is_error: meta.is_error,
                }
            }
            role::APPROVAL => {
                let meta: ApprovalMeta =
                    serde_json::from_value(meta).unwrap_or_else(|_| ApprovalMeta {
                        tool: "unknown".into(),
                        args: Value::Null,
                        status: ApprovalStatus::Expired,
                        expires_at: None,
                    });
                ChatMessage::Approval {
                    id,
                    run_id,
                    created_at,
                    tool: meta.tool,
                    args: meta.args,
                    status: meta.status,
                    expires_at: meta.expires_at.map(ts),
                }
            }
            role::ERROR => ChatMessage::Error {
                id,
                run_id,
                created_at,
                content: self.content.clone(),
            },
            _ => ChatMessage::Agent {
                id,
                run_id,
                created_at,
                content: self.content.clone(),
            },
        }
    }
}

const MESSAGE_COLUMNS: &str = "id, conversation_id, run_id, role, content, created_at, metadata";

fn message_row(row: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        run_id: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        metadata: json_col(row, 6)?,
    })
}

pub fn insert_message(conn: &Connection, message: &MessageRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO messages (id, conversation_id, run_id, role, content, created_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            message.id,
            message.conversation_id,
            message.run_id,
            message.role,
            message.content,
            message.created_at,
            message.metadata.to_string(),
        ],
    )?;
    Ok(())
}

pub fn get_message(conn: &Connection, id: &str) -> rusqlite::Result<Option<MessageRow>> {
    conn.query_row(
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        [id],
        message_row,
    )
    .optional()
}

/// Appends `text` to a message. Returns the offset it was appended at, in
/// UTF-16 code units (JavaScript string length), for idempotent deltas.
pub fn append_message_content(conn: &Connection, id: &str, text: &str) -> rusqlite::Result<usize> {
    let current: String =
        conn.query_row("SELECT content FROM messages WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    conn.execute(
        "UPDATE messages SET content = content || ?2 WHERE id = ?1",
        params![id, text],
    )?;
    Ok(current.encode_utf16().count())
}

pub fn update_message_metadata(
    conn: &Connection,
    id: &str,
    metadata: impl Serialize,
) -> rusqlite::Result<()> {
    let metadata = serde_json::to_value(metadata).unwrap_or(Value::Null);
    conn.execute(
        "UPDATE messages SET metadata = ?2 WHERE id = ?1",
        params![id, metadata.to_string()],
    )?;
    Ok(())
}

/// Metadata key on a message that compaction copied into a child run
/// (the original message's ID)
pub const COMPACTION_COPY_KEY: &str = "compactionCopyOf";
/// Metadata key on a child run's synthetic summary message (the parent
/// run's ID)
pub const COMPACTION_SUMMARY_KEY: &str = "compactionSummaryOf";

/// The latest `limit` messages of a conversation, oldest first. Messages
/// compaction wrote into a child run (copies and the summary) are left
/// out: the conversation already has the originals.
pub fn list_messages(
    conn: &Connection,
    conversation_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM (
            SELECT {MESSAGE_COLUMNS}, rowid AS seq FROM messages
             WHERE conversation_id = ?1
               AND metadata NOT LIKE '%\"{COMPACTION_COPY_KEY}\"%'
               AND metadata NOT LIKE '%\"{COMPACTION_SUMMARY_KEY}\"%'
            ORDER BY created_at DESC, rowid DESC LIMIT ?2
         ) ORDER BY created_at ASC, seq ASC"
    ))?;
    let rows = stmt.query_map(params![conversation_id, limit], message_row)?;
    rows.collect()
}

pub fn list_run_messages(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE run_id = ?1 ORDER BY created_at, rowid"
    ))?;
    let rows = stmt.query_map([run_id], message_row)?;
    rows.collect()
}
