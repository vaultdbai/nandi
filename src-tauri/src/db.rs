//! SQLite persistence at `~/.nandi/state.db`.
//!
//! Two kinds of table live here:
//! - Runtime data, owned by SQLite: conversations, messages, runs, the
//!   agent queue, approvals, run events, workflow runs, trigger fires,
//!   agent health, and `local_settings`.
//! - The projection of the gold copy (`agents`, `workflows`, `triggers`,
//!   `settings`, `connectors`, plus `sync_state`): a rebuildable view of the
//!   repo files, written only by `services::projection`. Projected rows
//!   carry `source_path` and `projection_version`.
//!
//! The UI reconstructs everything from snapshots of these tables plus event
//! deltas.
//!
//! JSON columns are declared `TEXT`: a column declared `JSON` gets NUMERIC
//! affinity in SQLite, which would coerce a JSON value like `"123"`.

use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, Connection, OptionalExtension, Row, RowIndex};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{
    ApprovalStatus, AttachmentInfo, ChatMessage, PendingApproval, QueueEntry, RunEventRecord,
    RunStatus, RunSummary, SourceType, StudioConversationSummary, Timestamp, ToolCallStatus,
    WorkflowSummary, STUDIO_AGENT_ID,
};

mod approvals;
mod conversations;
mod messages;
mod migrations;
mod queue;
mod runs;
mod schema;
mod settings;

// One `db` module to its callers, as before the split
pub use self::{
    approvals::*, conversations::*, messages::*, queue::*, runs::*, schema::*, settings::*,
};

pub type DbConn = PooledConnection<SqliteConnectionManager>;

/// Pooled handle to the state database. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn ts(ms: i64) -> Timestamp {
    ms as f64
}

impl Db {
    /// Opens (creating if needed) the database and brings it to the latest
    /// schema, keeping a copy of it first when it upgrades.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        }
        let manager = SqliteConnectionManager::file(path).with_init(configure_connection);
        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
        let db = Self { pool };
        {
            let conn = db.conn()?;
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("Failed to enable WAL: {e}"))?;
            match init_db_file(&conn, path)? {
                Upgrade::Upgraded { from, to, backup } => log::info!(
                    "Upgraded the local database from version {from} to {to}; the copy from before is {}",
                    backup.map(|b| b.display().to_string()).unwrap_or_default()
                ),
                Upgrade::Created { to } => log::info!("Created the local database at version {to}"),
                Upgrade::UpToDate => {}
            }
        }
        Ok(db)
    }

    /// A private in-memory database (one connection, so every caller sees
    /// the same data).
    #[cfg(test)]
    pub fn in_memory() -> Self {
        let manager = SqliteConnectionManager::memory().with_init(configure_connection);
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        let db = Self { pool };
        db.init().unwrap();
        db
    }

    pub fn conn(&self) -> Result<DbConn, String> {
        self.pool
            .get()
            .map_err(|e| format!("Database unavailable: {e}"))
    }

    #[cfg(test)]
    fn init(&self) -> Result<(), String> {
        let conn = self.conn()?;
        init_db(&conn).map_err(|e| format!("Failed to initialize database: {e}"))
    }

    /// Runs blocking DB work off the async runtime.
    pub async fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|e| format!("Database unavailable: {e}"))?;
            f(&mut conn).map_err(|e| format!("Database error: {e}"))
        })
        .await
        .map_err(|e| format!("Database task failed: {e}"))?
    }
}

fn configure_connection(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
}

/// Reads a text column holding JSON, as a [`Value`] or a typed shape. Text
/// that isn't JSON, or isn't a `T`, fails the read: a bad row is reported,
/// never read as `null` or a default.
pub fn json_col<T: DeserializeOwned>(
    row: &Row,
    column: impl RowIndex + Copy,
) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|e| conversion_error(row, column, e))
}

/// [`json_col`] for a nullable column: `NULL` reads as `None`, text that
/// isn't JSON still fails the read.
pub fn opt_json_col<T: DeserializeOwned>(
    row: &Row,
    column: impl RowIndex + Copy,
) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = row.get(column)?;
    text.map(|text| serde_json::from_str(&text).map_err(|e| conversion_error(row, column, e)))
        .transpose()
}

/// Reads one row of `table` with `read`. When a column fails to convert, the
/// error names the row by its `key_column` and the column by its name
/// (`tasks row 't1', column 'input': ...`), so the bad row can be found.
/// Other errors pass through unchanged.
pub fn named_row<T>(
    row: &Row,
    table: &'static str,
    key_column: usize,
    read: impl FnOnce(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    read(row).map_err(|error| match error {
        rusqlite::Error::FromSqlConversionFailure(index, kind, source) => {
            let bad_row = BadRow {
                table,
                key: row_key(row, key_column),
                column: row.as_ref().column_name(index).unwrap_or("?").to_string(),
                source,
            };
            rusqlite::Error::FromSqlConversionFailure(index, kind, Box::new(bad_row))
        }
        other => other,
    })
}

/// The text of a row's key column, for an error message
fn row_key(row: &Row, column: usize) -> String {
    match row.get_ref(column) {
        Ok(ValueRef::Text(text)) => String::from_utf8_lossy(text).into_owned(),
        Ok(ValueRef::Integer(number)) => number.to_string(),
        _ => "?".to_string(),
    }
}

/// A column of a named row that failed to convert
#[derive(Debug)]
struct BadRow {
    table: &'static str,
    key: String,
    column: String,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl std::fmt::Display for BadRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            table,
            key,
            column,
            source,
        } = self;
        write!(f, "{table} row '{key}', column '{column}': {source}")
    }
}

impl std::error::Error for BadRow {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Reads a text column holding an enum's stored name (`as_str`). An unknown
/// value fails the read: a bad row is reported, never silently remapped.
pub fn text_enum<T>(row: &Row, column: impl RowIndex + Copy) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let text: String = row.get(column)?;
    text.parse().map_err(|e| conversion_error(row, column, e))
}

/// [`text_enum`] for a nullable column: `NULL` reads as `None`, an unknown
/// value still fails the read.
pub fn opt_text_enum<T>(row: &Row, column: impl RowIndex + Copy) -> rusqlite::Result<Option<T>>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let text: Option<String> = row.get(column)?;
    text.map(|text| text.parse().map_err(|e| conversion_error(row, column, e)))
        .transpose()
}

fn conversion_error(
    row: &Row,
    column: impl RowIndex,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    let index = column.idx(row.as_ref()).unwrap_or_default();
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// Reads the single row `SELECT 'k1' AS id, ?1 AS doc, ?2 AS maybe`
    fn read_doc<T>(
        doc: &str,
        maybe: Option<&str>,
        read: impl FnOnce(&Row) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let conn = Connection::open_in_memory().unwrap();
        conn.query_row(
            "SELECT 'k1' AS id, ?1 AS doc, ?2 AS maybe",
            params![doc, maybe],
            |row| named_row(row, "docs", 0, read),
        )
    }

    #[test]
    fn json_columns_read_json_and_fail_on_anything_else() {
        let doc = |text: &str| read_doc(text, None, |row| json_col::<Value>(row, 1));
        assert_eq!(doc(r#"{"a": 1}"#).unwrap(), json!({"a": 1}));
        assert_eq!(doc("null").unwrap(), Value::Null);
        for bad in ["", "{oops", "nope"] {
            assert!(doc(bad).is_err(), "{bad:?}");
        }

        let maybe = |text: Option<&str>| read_doc("1", text, |row| opt_json_col::<Value>(row, 2));
        assert_eq!(maybe(None).unwrap(), None);
        assert_eq!(maybe(Some("[1]")).unwrap(), Some(json!([1])));
        assert!(maybe(Some("{oops")).is_err());

        // A typed read fails on JSON of the wrong shape
        assert!(read_doc(r#""text""#, None, |row| json_col::<u32>(row, 1)).is_err());
    }

    #[test]
    fn a_named_row_names_its_table_key_and_column_and_keeps_the_index() {
        let error = read_doc("{oops", None, |row| json_col::<Value>(row, 1)).unwrap_err();
        let rusqlite::Error::FromSqlConversionFailure(index, Type::Text, _) = &error else {
            panic!("not a conversion failure: {error:?}");
        };
        assert_eq!(*index, 1);
        assert!(
            error.to_string().contains("docs row 'k1', column 'doc': "),
            "{error}"
        );

        // Errors other than a failed conversion pass through unchanged
        let error = read_doc("1", None, |row| row.get::<_, String>(9)).unwrap_err();
        assert!(
            matches!(error, rusqlite::Error::InvalidColumnIndex(9)),
            "{error:?}"
        );
    }

    pub fn start_run(conn: &Connection, agent_id: &str, run_id: &str) -> String {
        let conversation_id = ensure_agent_conversation(conn, agent_id).unwrap();
        insert_run(
            conn,
            &RunSummary {
                id: run_id.into(),
                agent_id: agent_id.into(),
                conversation_id: conversation_id.clone(),
                status: RunStatus::Running,
                input: "hi".into(),
                summary: None,
                started_at: ts(now_ms()),
                ended_at: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                model: None,
                source_type: SourceType::User,
                source_id: None,
                source_label: None,
                priority: 100,
                queued_at: None,
                parent_run_id: None,
                compaction_summary: None,
            },
        )
        .unwrap();
        conversation_id
    }

    #[test]
    fn a_run_with_an_unknown_status_fails_to_read_instead_of_changing_meaning() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        start_run(&conn, "a", "r1");
        assert_eq!(
            get_run(&conn, "r1").unwrap().unwrap().status,
            RunStatus::Running
        );
        conn.execute("UPDATE runs SET status = 'finished' WHERE id = 'r1'", [])
            .unwrap();
        assert!(matches!(
            get_run(&conn, "r1"),
            Err(rusqlite::Error::FromSqlConversionFailure(3, Type::Text, _))
        ));
    }

    #[test]
    fn init_is_idempotent_and_seeds_settings() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        init_db(&conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(get_setting(&conn, crate::engine::CANNED_SCRIPT_KEY)
            .unwrap()
            .is_some());
    }

    #[test]
    fn appends_content_with_utf16_offsets() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = ensure_agent_conversation(&conn, "a").unwrap();
        insert_message(
            &conn,
            &MessageRow::new("m1", &conversation, None, role::ASSISTANT, "", json!({})),
        )
        .unwrap();

        assert_eq!(append_message_content(&conn, "m1", "😀 ").unwrap(), 0);
        // The emoji is 2 UTF-16 units
        assert_eq!(append_message_content(&conn, "m1", "hi").unwrap(), 3);
        assert_eq!(get_message(&conn, "m1").unwrap().unwrap().content, "😀 hi");
    }

    #[test]
    fn lists_latest_messages_in_order() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = ensure_agent_conversation(&conn, "a").unwrap();
        for i in 0..5 {
            insert_message(
                &conn,
                &MessageRow::new(
                    format!("m{i}"),
                    &conversation,
                    None,
                    role::USER,
                    format!("{i}"),
                    json!({}),
                ),
            )
            .unwrap();
        }

        let ids: Vec<_> = list_messages(&conn, &conversation, 3)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["m2", "m3", "m4"]);
    }

    #[test]
    fn only_nandis_continuations_are_continuations() {
        let added = json!({ "attachments": [], "continuation": true });
        let continuation =
            MessageRow::new("c1", "c", Some("r"), role::USER, "verify", added.clone());
        assert!(continuation.is_continuation());
        let typed = MessageRow::new("u1", "c", Some("r"), role::USER, "hey", json!({}));
        assert!(!typed.is_continuation());
        let reply = MessageRow::new("a1", "c", Some("r"), role::ASSISTANT, "ok", added);
        assert!(!reply.is_continuation(), "only user-role rows");
    }

    #[test]
    fn maps_rows_to_chat_messages() {
        let tool = MessageRow::new(
            "t1",
            "c",
            Some("r"),
            role::TOOL_CALL,
            "",
            ToolCallMeta {
                tool: "shell".into(),
                args: json!({"cmd": "ls"}),
                status: ToolCallStatus::Done,
                result: Some("ok".into()),
                is_error: false,
            },
        );
        let value = serde_json::to_value(tool.to_chat()).unwrap();
        assert_eq!(value["kind"], "toolCall");
        assert_eq!(value["runId"], "r");
        assert_eq!(value["isError"], false);

        let user = MessageRow::new("u1", "c", None, role::USER, "hey", json!({}));
        assert!(
            matches!(user.to_chat(), ChatMessage::User { attachments, .. } if attachments.is_empty())
        );
    }

    #[test]
    fn tracks_active_and_running_runs() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = start_run(&conn, "a", "r1");
        let running = |conn: &Connection| -> Vec<String> {
            running_runs(conn)
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect()
        };

        assert_eq!(active_run(&conn, &conversation).unwrap().unwrap().id, "r1");
        assert_eq!(running(&conn), ["r1"]);
        set_run_status(&conn, "r1", RunStatus::AwaitingApproval, None).unwrap();
        assert!(running(&conn).is_empty(), "awaiting runs survive restarts");

        set_run_status(&conn, "r1", RunStatus::Failed, None).unwrap();
        assert!(running(&conn).is_empty());
        assert!(active_run(&conn, &conversation).unwrap().is_none());
    }

    fn run_usage(conn: &Connection, id: &str) -> (u32, u32, Option<f64>, Option<String>) {
        let run = get_run(conn, id).unwrap().unwrap();
        (run.input_tokens, run.output_tokens, run.cost_usd, run.model)
    }

    #[test]
    fn sums_usage_across_model_calls() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        start_run(&conn, "a", "r1");
        assert_eq!(run_usage(&conn, "r1"), (0, 0, None, None));

        let model = "deepseek:deepseek-flash";
        add_run_usage(&conn, "r1", model, 100, 20, None).unwrap();
        assert_eq!(
            run_usage(&conn, "r1"),
            (100, 20, None, Some(model.to_string()))
        );
        add_run_usage(&conn, "r1", model, 50, 5, Some(0.25)).unwrap();
        add_run_usage(&conn, "r1", model, 10, 1, Some(0.5)).unwrap();
        assert_eq!(
            run_usage(&conn, "r1"),
            (160, 26, Some(0.75), Some(model.to_string()))
        );
    }

    fn enqueue(conn: &Connection, agent_id: &str, run_id: &str, priority: i32, created_at: i64) {
        let conversation_id = ensure_agent_conversation(conn, agent_id).unwrap();
        insert_run(
            conn,
            &RunSummary {
                id: run_id.into(),
                agent_id: agent_id.into(),
                conversation_id,
                status: RunStatus::Queued,
                input: run_id.into(),
                summary: None,
                started_at: ts(created_at),
                ended_at: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                model: None,
                source_type: SourceType::User,
                source_id: None,
                source_label: None,
                priority,
                queued_at: Some(ts(created_at)),
                parent_run_id: None,
                compaction_summary: None,
            },
        )
        .unwrap();
        insert_queue_row(
            conn,
            &QueueRow {
                id: format!("q-{run_id}"),
                agent_id: agent_id.into(),
                run_id: run_id.into(),
                source_type: SourceType::User,
                source_id: None,
                source_label: None,
                priority,
                message: run_id.into(),
                message_id: format!("m-{run_id}"),
                attachments: vec![],
                created_at,
            },
        )
        .unwrap();
    }

    fn queue_ids(conn: &Connection, agent_id: &str) -> Vec<String> {
        list_queue(conn, agent_id)
            .unwrap()
            .into_iter()
            .map(|e| e.run_id)
            .collect()
    }

    fn queue_position(conn: &Connection, run_id: &str) -> Option<i64> {
        conn.query_row(
            "SELECT queue_position FROM runs WHERE id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn queue_orders_by_priority_then_fifo() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        enqueue(&conn, "a", "bg-old", 50, 1);
        enqueue(&conn, "a", "user-old", 10, 2);
        enqueue(&conn, "a", "bg-new", 50, 3);
        enqueue(&conn, "a", "user-new", 10, 4);
        // Same priority and millisecond: insertion order
        enqueue(&conn, "a", "user-tie", 10, 4);
        enqueue(&conn, "other", "elsewhere", 1, 0);

        assert_eq!(
            queue_ids(&conn, "a"),
            ["user-old", "user-new", "user-tie", "bg-old", "bg-new"]
        );
        let entries = list_queue(&conn, "a").unwrap();
        assert_eq!(
            entries.iter().map(|e| e.position).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(queue_position(&conn, "bg-old"), Some(3));

        let popped: Vec<String> = std::iter::from_fn(|| pop_queue(&conn, "a").unwrap())
            .map(|row| row.run_id)
            .collect();
        assert_eq!(
            popped,
            ["user-old", "user-new", "user-tie", "bg-old", "bg-new"]
        );
        assert!(queue_is_empty(&conn, "a").unwrap());
        assert_eq!(queue_position(&conn, "bg-new"), None);
        assert_eq!(queue_ids(&conn, "other"), ["elsewhere"]);
    }

    #[test]
    fn removes_and_reprioritizes_queued_runs() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        enqueue(&conn, "a", "r1", 50, 1);
        enqueue(&conn, "a", "r2", 50, 2);
        enqueue(&conn, "a", "r3", 50, 3);

        let removed = remove_queued(&conn, "r2").unwrap().unwrap();
        assert_eq!(removed.message_id, "m-r2");
        assert!(remove_queued(&conn, "r2").unwrap().is_none());
        assert_eq!(queue_ids(&conn, "a"), ["r1", "r3"]);
        assert_eq!(queue_position(&conn, "r3"), Some(1));

        assert!(set_queue_priority(&conn, "r3", 5).unwrap());
        assert_eq!(queue_ids(&conn, "a"), ["r3", "r1"]);
        assert_eq!(get_run(&conn, "r3").unwrap().unwrap().priority, 5);
        assert_eq!(queue_position(&conn, "r1"), Some(1));
        assert!(!set_queue_priority(&conn, "missing", 1).unwrap());
    }

    #[test]
    fn queued_runs_are_neither_active_nor_ended() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        enqueue(&conn, "a", "r1", 10, 1);
        let run = get_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert!(run.ended_at.is_none());
        assert!(active_agent_run(&conn, "a").unwrap().is_none());
        assert!(running_runs(&conn).unwrap().is_empty());

        let started = start_queued_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(started.status, RunStatus::Running);
        assert!(started.started_at >= run.started_at);
        assert_eq!(started.queued_at, Some(ts(1)));
        assert_eq!(active_agent_run(&conn, "a").unwrap().unwrap().id, "r1");
    }

    #[test]
    fn approvals_mirror_status_onto_their_message() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = start_run(&conn, "a", "r1");
        insert_message(
            &conn,
            &MessageRow::new(
                "ap1",
                &conversation,
                Some("r1"),
                role::APPROVAL,
                "",
                ApprovalMeta {
                    tool: "shell".into(),
                    args: json!({}),
                    status: ApprovalStatus::Pending,
                    expires_at: Some(5_000),
                },
            ),
        )
        .unwrap();
        let approval = ApprovalRow {
            id: "ap1".into(),
            agent_id: "a".into(),
            run_id: "r1".into(),
            message_id: "ap1".into(),
            tool_call_id: "call1".into(),
            tool: "shell".into(),
            args: json!({}),
            status: ApprovalStatus::Pending,
            created_at: now_ms(),
            expires_at: Some(5_000),
            decided_by: None,
        };
        insert_approval(&conn, &approval).unwrap();

        let pending = &pending_approvals(&conn, Some("a")).unwrap()[0];
        assert_eq!(pending.conversation_id, conversation);
        assert_eq!(pending.expires_at, Some(5_000.0));
        assert_eq!(
            approval_for_call(&conn, "r1", "call1").unwrap().unwrap().id,
            "ap1"
        );
        // Another run's call with the same provider ID isn't approved by it
        assert!(approval_for_call(&conn, "r2", "call1").unwrap().is_none());
        // Overdue once its deadline has passed
        assert!(overdue_approvals(&conn, 4_999).unwrap().is_empty());
        assert_eq!(overdue_approvals(&conn, 5_000).unwrap()[0].id, "ap1");

        set_approval_decider(&conn, "ap1", "Ada").unwrap();
        assert_eq!(
            get_approval(&conn, "ap1")
                .unwrap()
                .unwrap()
                .decided_by
                .as_deref(),
            Some("Ada")
        );
        set_approval_status(&conn, &approval, ApprovalStatus::Approved).unwrap();
        assert!(pending_approvals(&conn, None).unwrap().is_empty());
        assert!(overdue_approvals(&conn, i64::MAX).unwrap().is_empty());
        let message = get_message(&conn, "ap1").unwrap().unwrap();
        // The deadline stays on the message, for "expired" vs "decided"
        assert!(matches!(
            message.to_chat(),
            ChatMessage::Approval {
                status: ApprovalStatus::Approved,
                expires_at: Some(5_000.0),
                ..
            }
        ));
    }

    #[test]
    fn summarizes_studio_conversations_newest_first() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        insert_studio_conversation(&conn, "s1", "First").unwrap();
        insert_studio_conversation(&conn, "s2", "Second").unwrap();
        let mut reply = MessageRow::new("m", "s1", None, role::ASSISTANT, "latest", json!({}));
        reply.created_at += 10_000;
        insert_message(&conn, &reply).unwrap();
        // Agent conversations aren't Studio conversations
        ensure_agent_conversation(&conn, "a").unwrap();

        let list = list_studio_conversations(&conn).unwrap();
        assert_eq!(
            list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["s1", "s2"]
        );
        assert_eq!(list[0].last_message.as_deref(), Some("latest"));
    }

    #[test]
    fn records_agent_state() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        set_agent_state(&conn, "a", "error", Some("panicked")).unwrap();
        set_agent_state(&conn, "b", "ok", None).unwrap();
        assert_eq!(
            agents_in_error(&conn).unwrap(),
            [("a".to_string(), Some("panicked".to_string()))]
        );
    }
}
