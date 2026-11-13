//! What happened to every delegation: the `delegations` table (one row per
//! task and side), `delegation://` events, and the JSONL audit log
//! (`~/.nandi/local/logs/delegations.log`). Machine-local state, never in
//! the gold copy.
//!
//! The audit log holds who called whom, for what, and how it ended; never
//! the task's input or output.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::{
    DelegationDirection, DelegationTaskCompleted, DelegationTaskFailed, DelegationTaskStarted,
    TaskResult, TaskStatus,
};
use crate::db::{now_ms, Db};
use crate::services::events::{self, EventBus};
use crate::utils::nandi_config;

/// Most rows `list` returns
pub const MAX_HISTORY: u32 = 1_000;

pub const LOG_FILE_NAME: &str = "delegations.log";

const COLUMNS: &str = "id, direction, capability, target_agent, requester_agent, provider_owner,
    requester_owner, input, output, status, started_at, ended_at, error, tokens_in, tokens_out";

/// A `Running` row for a task whose provider isn't picked yet.
pub fn placeholder(
    task_id: &str,
    capability: &str,
    input: &Value,
    target: Option<&str>,
) -> TaskResult {
    TaskResult {
        task_id: task_id.to_string(),
        provider_agent: target.unwrap_or_default().to_string(),
        output: Value::Null,
        status: TaskStatus::Running,
        duration_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
        error: None,
        direction: DelegationDirection::Outbound,
        capability: capability.to_string(),
        requester_agent: String::new(),
        provider_owner: None,
        requester_owner: None,
        input: input.clone(),
        started_at: now_ms() as f64,
        ended_at: None,
    }
}

/// Inserts or replaces the task's row.
pub async fn insert(db: &Db, result: &TaskResult) -> Result<(), String> {
    let row = result.clone();
    db.call(move |conn| upsert(conn, &row)).await
}

fn upsert(conn: &Connection, r: &TaskResult) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO delegations ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
               direction = excluded.direction, capability = excluded.capability,
               target_agent = excluded.target_agent, requester_agent = excluded.requester_agent,
               provider_owner = excluded.provider_owner, requester_owner = excluded.requester_owner,
               input = excluded.input, output = excluded.output, status = excluded.status,
               started_at = excluded.started_at, ended_at = excluded.ended_at,
               error = excluded.error, tokens_in = excluded.tokens_in,
               tokens_out = excluded.tokens_out"
        ),
        params![
            r.task_id,
            r.direction.as_str(),
            r.capability,
            r.provider_agent,
            r.requester_agent,
            r.provider_owner,
            r.requester_owner,
            r.input.to_string(),
            (!r.output.is_null()).then(|| r.output.to_string()),
            r.status.as_str(),
            r.started_at as i64,
            r.ended_at.map(|t| t as i64),
            r.error,
            r.tokens_in,
            r.tokens_out,
        ],
    )?;
    Ok(())
}

pub async fn get(db: &Db, task_id: &str) -> Result<Option<TaskResult>, String> {
    let id = task_id.to_string();
    db.call(move |conn| {
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM delegations WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()
    })
    .await
}

/// Newest first
pub async fn list(db: &Db, limit: u32) -> Result<Vec<TaskResult>, String> {
    db.call(move |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM delegations ORDER BY started_at DESC, rowid DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit], from_row)?;
        rows.collect()
    })
    .await
}

fn json(text: Option<String>) -> Value {
    text.and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<TaskResult> {
    let started_at: i64 = row.get(10)?;
    let ended_at: Option<i64> = row.get(11)?;
    let direction: String = row.get(1)?;
    let status: String = row.get(9)?;
    Ok(TaskResult {
        task_id: row.get(0)?,
        direction: DelegationDirection::parse(&direction),
        capability: row.get(2)?,
        provider_agent: row.get(3)?,
        requester_agent: row.get(4)?,
        provider_owner: row.get(5)?,
        requester_owner: row.get(6)?,
        input: json(row.get(7)?),
        output: json(row.get(8)?),
        status: TaskStatus::parse(&status),
        started_at: started_at as f64,
        ended_at: ended_at.map(|t| t as f64),
        duration_ms: ended_at.map_or(0, |end| (end - started_at).clamp(0, u32::MAX as i64) as u32),
        error: row.get(12)?,
        tokens_in: row.get(13)?,
        tokens_out: row.get(14)?,
    })
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// Where audit entries go. The default is
/// `~/.nandi/local/logs/delegations.log`, which tests never write to.
#[derive(Debug, Clone, Default)]
pub struct AuditLog {
    path: Option<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry<'a> {
    timestamp: i64,
    kind: &'static str,
    /// `started` | `finished`
    event: &'a str,
    task_id: &'a str,
    direction: &'static str,
    capability: &'a str,
    requester: &'a str,
    provider: &'a str,
    status: &'static str,
    duration_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// Serializes writers, so lines never interleave
static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl AuditLog {
    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// `~/.nandi/local/logs/delegations.log` (none in tests)
    pub fn default_location() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        Self {
            path: nandi_config::local_dir()
                .ok()
                .map(|dir| dir.join("logs").join(LOG_FILE_NAME)),
        }
    }

    /// Appends one entry. Failures are logged, not returned: an unwritable
    /// log must not fail the delegation.
    pub async fn record(&self, event: &str, result: &TaskResult) {
        let Some(path) = &self.path else {
            return;
        };
        let entry = Entry {
            timestamp: now_ms(),
            kind: "delegation",
            event,
            task_id: &result.task_id,
            direction: result.direction.as_str(),
            capability: &result.capability,
            requester: &result.requester_agent,
            provider: &result.provider_agent,
            status: result.status.as_str(),
            duration_ms: result.duration_ms,
            error: result.error.as_deref(),
        };
        if let Err(e) = append(path, &entry).await {
            log::error!("Failed to write the delegation audit log: {e}");
        }
    }
}

async fn append(path: &Path, entry: &Entry<'_>) -> Result<(), String> {
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let _guard = WRITE_LOCK.lock().await;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| e.to_string())?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    file.flush().await.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Recorder: row + event + audit, for both sides
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Recorder {
    pub db: Db,
    pub bus: EventBus,
    pub audit: AuditLog,
}

impl Recorder {
    pub async fn started(&self, result: &TaskResult) {
        if let Err(e) = insert(&self.db, result).await {
            log::error!("Failed to record delegation {}: {e}", result.task_id);
        }
        events::emit(
            &self.bus,
            &DelegationTaskStarted {
                task_id: result.task_id.clone(),
                direction: result.direction,
                capability: result.capability.clone(),
                requester_agent: result.requester_agent.clone(),
                target_agent: result.provider_agent.clone(),
            },
        );
        self.audit.record("started", result).await;
    }

    pub async fn finished(&self, result: &TaskResult) {
        if let Err(e) = insert(&self.db, result).await {
            log::error!("Failed to record delegation {}: {e}", result.task_id);
        }
        if result.status == TaskStatus::Completed {
            events::emit(
                &self.bus,
                &DelegationTaskCompleted {
                    result: result.clone(),
                },
            );
        } else {
            events::emit(
                &self.bus,
                &DelegationTaskFailed {
                    task_id: result.task_id.clone(),
                    status: result.status,
                    error: result
                        .error
                        .clone()
                        .unwrap_or_else(|| format!("The task ended as {}", result.status.as_str())),
                },
            );
        }
        self.audit.record("finished", result).await;
    }
}

/// Ends `result` now: status, error, and the duration since it started.
pub fn finish(result: &mut TaskResult, status: TaskStatus, error: Option<String>) {
    let now = now_ms();
    result.status = status;
    result.error = error;
    result.ended_at = Some(now as f64);
    result.duration_ms = (now - result.started_at as i64).clamp(0, u32::MAX as i64) as u32;
}

/// Ends the open history row of a task that outlived its caller (an
/// assignment): its result came later, to the task service. Emits what
/// [`Recorder::finished`] emits. No-op when the row isn't open.
pub async fn close(
    db: &Db,
    bus: &EventBus,
    task_id: &str,
    status: TaskStatus,
    output: Value,
    error: Option<String>,
) -> Result<(), String> {
    let Some(mut result) = get(db, task_id).await? else {
        return Ok(());
    };
    if result.status != TaskStatus::Running {
        return Ok(());
    }
    result.output = output;
    finish(&mut result, status, error);
    insert(db, &result).await?;
    // Ours, and it came back after the live wait: an Inbox item
    if result.direction == DelegationDirection::Outbound {
        let instance = result.provider_agent.split('/').next().unwrap_or_default();
        let name = crate::services::network::known_members(db)
            .into_iter()
            .find(|m| m.instance_id == instance)
            .map(|m| m.display_name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Someone on your network".to_string());
        let item = super::returns::from_result(&result, &name);
        let stored = item.clone();
        match db
            .call(move |conn| super::returns::insert(conn, &stored))
            .await
        {
            Ok(()) => events::emit(bus, &super::returns::RequestReturned { item }),
            Err(e) => log::warn!("Task {task_id}: couldn't record the return: {e}"),
        }
    }
    if status == TaskStatus::Completed {
        events::emit(bus, &DelegationTaskCompleted { result });
    } else {
        events::emit(
            bus,
            &DelegationTaskFailed {
                task_id: result.task_id.clone(),
                status,
                error: result
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("The task ended as {}", status.as_str())),
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn rows_round_trip_newest_first() {
        let db = Db::in_memory();
        let mut first = placeholder("t1", "summarize", &json!({"text": "a"}), Some("i/a"));
        first.started_at = 1_000.0;
        insert(&db, &first).await.unwrap();
        let mut second = placeholder("t2", "translate", &json!("b"), None);
        second.started_at = 2_000.0;
        second.direction = DelegationDirection::Inbound;
        insert(&db, &second).await.unwrap();

        first.output = json!({"summary": "x"});
        finish(&mut first, TaskStatus::Completed, None);
        insert(&db, &first).await.unwrap();

        let got = get(&db, "t1").await.unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Completed);
        assert_eq!(got.output, json!({"summary": "x"}));
        assert_eq!(got.input, json!({"text": "a"}));
        assert_eq!(got.provider_agent, "i/a");
        assert!(get(&db, "nope").await.unwrap().is_none());

        let rows = list(&db, 10).await.unwrap();
        let ids: Vec<_> = rows.iter().map(|r| r.task_id.as_str()).collect();
        assert_eq!(ids, ["t2", "t1"]);
        assert_eq!(rows[0].direction, DelegationDirection::Inbound);
        assert_eq!(list(&db, 1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_audit_log_leaves_out_input_and_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join(LOG_FILE_NAME);
        let log = AuditLog::at(path.clone());
        let mut result = placeholder("t1", "summarize", &json!({"secret": "s3"}), Some("i/a"));
        result.output = json!("private answer");
        log.record("started", &result).await;
        finish(&mut result, TaskStatus::Failed, Some("busy".into()));
        log.record("finished", &result).await;

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["event"], "started");
        assert_eq!(lines[1]["status"], "failed");
        assert_eq!(lines[1]["error"], "busy");
        assert_eq!(lines[1]["taskId"], "t1");
        assert!(!text.contains("s3") && !text.contains("private answer"));
    }
}
