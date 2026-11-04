//! Append-only JSON-lines audit logs under `~/.nandi/local/logs/`.
//!
//! Entries record *metadata* (who, what, how much) — never prompts, tool
//! output, API keys, or tokens.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::utils::nandi_config;

/// Serializes appends so concurrent entries never interleave.
static WRITE_LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Debug, Clone, Copy)]
pub enum AuditLog {
    /// `llm.log` — every LLM request, from the orchestrator or the proxy
    Llm,
    /// `egress.log` — every AWS call, fetch, and tunnel through the egress
    /// proxy (allowed or denied)
    Egress,
    /// `triggers.log` — every trigger fire and what it dispatched
    Triggers,
    /// `decisions.log` — every approval decision (who, or that it expired)
    /// and every call a policy refused (which policy)
    Decisions,
}

impl AuditLog {
    fn file_name(self) -> &'static str {
        match self {
            AuditLog::Llm => "llm.log",
            AuditLog::Egress => "egress.log",
            AuditLog::Triggers => "triggers.log",
            AuditLog::Decisions => "decisions.log",
        }
    }

    pub fn path(self) -> Result<std::path::PathBuf, String> {
        Ok(nandi_config::logs_dir()?.join(self.file_name()))
    }
}

// --- Reading ------------------------------------------------------------------
//
// `AuditEntry` is a frozen wave-1 contract: fields may be added, never
// renamed or removed.

/// The logs the viewer can read, by `AuditEntry::kind`.
pub const AUDIT_KINDS: [&str; 6] = [
    "llm",
    "egress",
    "triggers",
    "pilots",
    "connectors",
    "decisions",
];

/// The most entries one page may hold.
pub const MAX_AUDIT_PAGE: u32 = 500;

/// Bytes read from the end of a log per step while paging backwards.
const READ_CHUNK: u64 = 64 * 1024;

/// What a redacted value is replaced with.
pub const REDACTED: &str = "[redacted]";

/// One audit log line, normalized for the audit log viewer.
#[derive(Debug, Clone, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// ms since the Unix epoch
    pub timestamp: f64,
    /// `llm` | `egress` | `triggers` | `pilots` | `connectors`
    pub kind: String,
    pub agent_id: Option<String>,
    pub tool: Option<String>,
    /// Arguments with secrets and content removed
    pub args_redacted: Option<serde_json::Value>,
    pub outcome: String,
    pub duration_ms: Option<f64>,
}

/// Reads the audit logs (see `AuditLog::path`); writing stays with `record`.
#[derive(Debug, Default)]
pub struct AuditStore {
    /// `None` reads from `nandi_config::logs_dir()`
    dir: Option<PathBuf>,
}

impl AuditStore {
    pub fn new() -> Self {
        Self { dir: None }
    }

    /// Reads logs from `dir` instead of `~/.nandi` (tests).
    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    /// One page of the `kind` log, newest first, normalized and redacted.
    /// Lines that aren't JSON objects with a timestamp are skipped (and not
    /// counted toward `offset`). A missing log is empty.
    pub async fn list(
        &self,
        kind: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEntry>, String> {
        let file_name = log_file_name(kind)?;
        let dir = match &self.dir {
            Some(dir) => dir.clone(),
            None => nandi_config::logs_dir()?,
        };
        let path = dir.join(file_name);
        let kind = kind.to_string();
        let limit = limit.min(MAX_AUDIT_PAGE) as usize;
        tokio::task::spawn_blocking(move || read_page(&path, &kind, limit, offset as usize))
            .await
            .map_err(|e| format!("Failed to read the {file_name}: {e}"))?
    }
}

fn log_file_name(kind: &str) -> Result<&'static str, String> {
    match kind {
        "llm" => Ok(AuditLog::Llm.file_name()),
        "egress" => Ok(AuditLog::Egress.file_name()),
        "triggers" => Ok(AuditLog::Triggers.file_name()),
        "decisions" => Ok(AuditLog::Decisions.file_name()),
        // Written by the pilots track (wave 1)
        "pilots" => Ok(crate::services::pilots::audit::FILE_NAME),
        // Written by the connectors track (wave 1)
        "connectors" => Ok(crate::services::connectors::audit::FILE_NAME),
        _ => Err(format!(
            "Unknown audit log '{kind}': expected one of {}",
            AUDIT_KINDS.join(", ")
        )),
    }
}

/// Reads `path` backwards in chunks, so a page near the end (the common
/// case) never reads the whole file.
fn read_page(
    path: &Path,
    kind: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<AuditEntry>, String> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Failed to open {}: {e}", path.display())),
    };
    let read_error = |e: std::io::Error| format!("Failed to read {}: {e}", path.display());
    let mut end = file.metadata().map_err(read_error)?.len();
    let mut entries = Vec::new();
    let mut skipped = 0;
    // The (partial) first line of the chunk read last
    let mut carry: Vec<u8> = Vec::new();

    while end > 0 && entries.len() < limit {
        let start = end.saturating_sub(READ_CHUNK);
        let mut chunk = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start)).map_err(read_error)?;
        file.read_exact(&mut chunk).map_err(read_error)?;
        chunk.extend_from_slice(&carry);

        let mut lines: Vec<&[u8]> = chunk.split(|b| *b == b'\n').collect();
        // Unless this chunk starts the file, its first line continues
        // in the next (earlier) chunk
        let first = if start > 0 { lines.remove(0) } else { &[][..] };
        for line in lines.into_iter().rev() {
            let Some(entry) = parse_line(kind, line) else {
                continue;
            };
            if skipped < offset {
                skipped += 1;
            } else {
                entries.push(entry);
                if entries.len() == limit {
                    break;
                }
            }
        }
        carry = first.to_vec();
        end = start;
    }
    Ok(entries)
}

fn parse_line(kind: &str, line: &[u8]) -> Option<AuditEntry> {
    let line = std::str::from_utf8(line).ok()?.trim();
    if line.is_empty() {
        return None;
    }
    match serde_json::from_str(line).ok()? {
        Value::Object(fields) => normalize(kind, fields),
        _ => None,
    }
}

/// Maps one log's own line shape onto `AuditEntry`. Fields that don't map
/// onto a column stay in `args_redacted`, redacted.
fn normalize(kind: &str, mut fields: Map<String, Value>) -> Option<AuditEntry> {
    let timestamp = take(&mut fields, &["ts", "timestamp"])?.as_f64()?;
    let mut agent_id = take_string(&mut fields, &["agent_id", "agentId"]);
    let duration_ms = take(&mut fields, &["duration_ms", "durationMs"]).and_then(|v| v.as_f64());

    let (tool, outcome) = match kind {
        "llm" => (
            take_string(&mut fields, &["model"]),
            take_string(&mut fields, &["status"]),
        ),
        "egress" => (
            take_string(&mut fields, &["kind"]),
            take(&mut fields, &["allowed"])
                .and_then(|v| v.as_bool())
                .map(|allowed| if allowed { "allowed" } else { "denied" }.to_string()),
        ),
        "triggers" => {
            // A fire that targets an agent is that agent's
            if agent_id.is_none() && fields.get("target_type") == Some(&json!("agent")) {
                agent_id = fields
                    .get("target_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            (
                take_string(&mut fields, &["source_type", "sourceType"]),
                take_string(&mut fields, &["status"]),
            )
        }
        // Pilot and connector calls show the tool's model-facing name
        "pilots" => {
            let tool = take_string(&mut fields, &["tool"]);
            let tool = match (take_string(&mut fields, &["pilot_id", "pilotId"]), tool) {
                (Some(pilot), Some(tool)) => {
                    Some(crate::services::pilots::tools::tool_name(&pilot, &tool))
                }
                (_, tool) => tool,
            };
            (tool, take_string(&mut fields, &["outcome"]))
        }
        "connectors" => {
            let tool = take_string(&mut fields, &["tool"]);
            let tool = match (
                take_string(&mut fields, &["connector_id", "connectorId"]),
                tool,
            ) {
                (Some(connector), Some(tool)) => Some(format!("{connector}__{tool}")),
                (_, tool) => tool,
            };
            (tool, take_string(&mut fields, &["outcome"]))
        }
        _ => (
            take_string(&mut fields, &["tool", "capability", "method"]),
            take_string(&mut fields, &["outcome", "status", "result"]),
        ),
    };

    // A line that carries its own args keeps them; otherwise the leftovers
    // are the details
    let args = match take(&mut fields, &["args_redacted", "argsRedacted", "args"]) {
        Some(args) => Some(args),
        None if fields.is_empty() => None,
        None => Some(Value::Object(fields)),
    };

    Some(AuditEntry {
        timestamp,
        kind: kind.to_string(),
        agent_id,
        tool,
        args_redacted: args.map(redact),
        outcome: outcome.unwrap_or_else(|| "unknown".to_string()),
        duration_ms,
    })
}

fn take(fields: &mut Map<String, Value>, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|key| fields.remove(*key))
        .filter(|value| !value.is_null())
}

fn take_string(fields: &mut Map<String, Value>, keys: &[&str]) -> Option<String> {
    match take(fields, keys)? {
        Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

/// Field names whose (non-numeric) values are secrets.
fn secret_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)password|passwd|passphrase|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|^key$|authorization|cookie|credential",
        )
        .expect("valid regex")
    })
}

/// Secrets inside free text: `Bearer …`, `key=…`, and well-known key
/// shapes (OpenAI/Anthropic `sk-…`, AWS `AKIA…`, GitHub `ghp_…`).
fn secret_patterns() -> &'static [(Regex, &'static str)] {
    static RE: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RE.get_or_init(|| {
        [
            (r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]+", "$1 [redacted]"),
            (
                r#"(?i)\b(password|passwd|secret|token|api[_-]?key|access[_-]?key)(\s*[=:]\s*"?)[^\s"&,;]+"#,
                "$1$2[redacted]",
            ),
            (r"\bsk-[A-Za-z0-9_-]{16,}", REDACTED),
            (r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", REDACTED),
            (r"\bgh[pousr]_[A-Za-z0-9]{20,}", REDACTED),
        ]
        .into_iter()
        .map(|(pattern, replacement)| (Regex::new(pattern).expect("valid regex"), replacement))
        .collect()
    })
}

/// Replaces secrets in `value`: the values of secret-named fields (counts
/// like `input_tokens` stay), and secret-looking text anywhere.
pub fn redact(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let secret = secret_key().is_match(&key)
                        && !matches!(value, Value::Number(_) | Value::Bool(_) | Value::Null);
                    let value = if secret {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redact).collect()),
        Value::String(text) => Value::String(redact_text(&text)),
        other => other,
    }
}

fn redact_text(text: &str) -> String {
    let mut text = text.to_string();
    for (pattern, replacement) in secret_patterns() {
        if pattern.is_match(&text) {
            text = pattern.replace_all(&text, *replacement).into_owned();
        }
    }
    text
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Appends `entry` as one JSON line. Failures are logged, not returned: an
/// unwritable audit log must not break the request being audited.
pub async fn record(log: AuditLog, entry: &impl Serialize) {
    // Tests must not write to the real ~/.nandi
    if cfg!(test) {
        return;
    }
    if let Err(e) = append(log, entry).await {
        log::error!("Failed to write audit log {}: {e}", log.file_name());
    }
}

async fn append(log: AuditLog, entry: &impl Serialize) -> Result<(), String> {
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');

    let dir = nandi_config::logs_dir()?;
    let _guard = WRITE_LOCK.lock().await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(log.file_name()))
        .await
        .map_err(|e| e.to_string())?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes `lines` as the `kind` log in a temporary folder.
    fn store_with(kind: &str, lines: &[String]) -> (tempfile::TempDir, AuditStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(dir.path().join(log_file_name(kind).unwrap()), text).unwrap();
        let store = AuditStore::with_dir(dir.path().to_path_buf());
        (dir, store)
    }

    fn llm_line(ts: u64) -> String {
        json!({
            "ts": ts,
            "source": "proxy",
            "agent_id": "writer",
            "model": "claude:claude-sonnet-5",
            "status": "ok",
            "input_tokens": 120,
            "output_tokens": 30,
            "duration_ms": 850,
            "cost_usd": 0.0042,
        })
        .to_string()
    }

    fn timestamps(entries: &[AuditEntry]) -> Vec<u64> {
        entries.iter().map(|e| e.timestamp as u64).collect()
    }

    #[tokio::test]
    async fn reads_jsonl_newest_first_and_pages() {
        let lines: Vec<String> = (1..=25).map(llm_line).collect();
        let (_dir, store) = store_with("llm", &lines);

        let first = store.list("llm", 10, 0).await.unwrap();
        assert_eq!(timestamps(&first), (16..=25).rev().collect::<Vec<_>>());

        let last = store.list("llm", 10, 20).await.unwrap();
        assert_eq!(timestamps(&last), [5, 4, 3, 2, 1]);

        assert!(store.list("llm", 10, 25).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pages_across_read_chunks() {
        // ~170 bytes a line: 2,000 lines span several 64 KiB chunks
        let lines: Vec<String> = (1..=2_000).map(llm_line).collect();
        let (_dir, store) = store_with("llm", &lines);

        let mut seen = Vec::new();
        for page in 0..5 {
            let entries = store
                .list("llm", MAX_AUDIT_PAGE, page * MAX_AUDIT_PAGE)
                .await
                .unwrap();
            seen.extend(timestamps(&entries));
        }
        assert_eq!(seen, (1..=2_000).rev().collect::<Vec<u64>>());
    }

    #[tokio::test]
    async fn skips_lines_that_arent_entries() {
        let lines = vec![
            llm_line(1),
            "not json".to_string(),
            String::new(),
            "[1, 2]".to_string(),
            json!({ "no": "timestamp" }).to_string(),
            llm_line(2),
        ];
        let (_dir, store) = store_with("llm", &lines);
        assert_eq!(store.list("llm", 10, 0).await.unwrap().len(), 2);
        // Skipped lines don't count toward the offset
        let second = store.list("llm", 10, 1).await.unwrap();
        assert_eq!(timestamps(&second), [1]);
    }

    #[tokio::test]
    async fn a_missing_log_is_empty_and_unknown_kinds_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = AuditStore::with_dir(dir.path().to_path_buf());
        for kind in AUDIT_KINDS {
            assert!(store.list(kind, 10, 0).await.unwrap().is_empty(), "{kind}");
        }
        let error = store.list("../secrets", 10, 0).await.unwrap_err();
        assert!(error.contains("Unknown audit log"), "{error}");
    }

    #[tokio::test]
    async fn normalizes_each_log_shape() {
        let (_dir, store) = store_with("llm", &[llm_line(7)]);
        let llm = &store.list("llm", 1, 0).await.unwrap()[0];
        assert_eq!(llm.kind, "llm");
        assert_eq!(llm.agent_id.as_deref(), Some("writer"));
        assert_eq!(llm.tool.as_deref(), Some("claude:claude-sonnet-5"));
        assert_eq!(llm.outcome, "ok");
        assert_eq!(llm.duration_ms, Some(850.0));
        // Token counts survive redaction (`token` is a secret-ish name)
        let args = llm.args_redacted.as_ref().unwrap();
        assert_eq!(args["input_tokens"], 120);
        assert_eq!(args["source"], "proxy");

        // `EgressLogEntry` is camelCase
        let egress = json!({
            "ts": 8, "agentId": "reader", "kind": "fetch",
            "target": "GET https://example.com/a", "allowed": false,
            "status": null, "durationMs": 12.5, "error": "not allowed",
        });
        let (_dir, store) = store_with("egress", &[egress.to_string()]);
        let egress = &store.list("egress", 1, 0).await.unwrap()[0];
        assert_eq!(egress.agent_id.as_deref(), Some("reader"));
        assert_eq!(egress.tool.as_deref(), Some("fetch"));
        assert_eq!(egress.outcome, "denied");
        assert_eq!(egress.duration_ms, Some(12.5));
        assert_eq!(
            egress.args_redacted.as_ref().unwrap()["target"],
            "GET https://example.com/a"
        );

        let fire = json!({
            "ts": 9, "trigger_id": "nightly", "fire_id": "f1", "source_type": "cron",
            "target_type": "agent", "target_id": "writer", "status": "queued",
            "run_id": "r1", "error": null,
        });
        let (_dir, store) = store_with("triggers", &[fire.to_string()]);
        let fire = &store.list("triggers", 1, 0).await.unwrap()[0];
        assert_eq!(fire.agent_id.as_deref(), Some("writer"));
        assert_eq!(fire.tool.as_deref(), Some("cron"));
        assert_eq!(fire.outcome, "queued");
        assert_eq!(
            fire.args_redacted.as_ref().unwrap()["trigger_id"],
            "nightly"
        );

        // Pilots: a generic shape with its own args
        let pilot = json!({
            "timestamp": 10, "agent_id": "writer", "tool": "screenshot",
            "args": { "display": 1, "api_key": "abc" }, "outcome": "ok",
        });
        let (_dir, store) = store_with("pilots", &[pilot.to_string()]);
        let pilot = &store.list("pilots", 1, 0).await.unwrap()[0];
        assert_eq!(pilot.tool.as_deref(), Some("screenshot"));
        assert_eq!(pilot.outcome, "ok");
        let args = pilot.args_redacted.as_ref().unwrap();
        assert_eq!(args["display"], 1);
        assert_eq!(args["api_key"], REDACTED);

        // The lines the pilot and connector writers produce
        let pilot = json!({
            "timestamp": 11.0, "kind": "pilots", "agentId": "writer",
            "pilotId": "opendesk", "tool": "click", "argsRedacted": { "x": 1 },
            "outcome": "denied", "durationMs": 3.0,
        });
        let (_dir, store) = store_with("pilots", &[pilot.to_string()]);
        let pilot = &store.list("pilots", 1, 0).await.unwrap()[0];
        assert_eq!(pilot.tool.as_deref(), Some("pilot_opendesk__click"));
        assert_eq!(pilot.outcome, "denied");

        let call = json!({
            "timestamp": 12, "kind": "connectors", "agentId": "writer",
            "connectorId": "filesystem", "tool": "read_file",
            "argsRedacted": { "path": "[redacted]" }, "outcome": "ok", "durationMs": 4,
        });
        let (_dir, store) = store_with("connectors", &[call.to_string()]);
        let call = &store.list("connectors", 1, 0).await.unwrap()[0];
        assert_eq!(call.kind, "connectors");
        assert_eq!(call.agent_id.as_deref(), Some("writer"));
        assert_eq!(call.tool.as_deref(), Some("filesystem__read_file"));
        assert_eq!(call.outcome, "ok");
        assert_eq!(call.duration_ms, Some(4.0));
        assert_eq!(call.args_redacted.as_ref().unwrap()["path"], REDACTED);
    }

    #[test]
    fn redacts_secret_fields_and_text() {
        let redacted = redact(json!({
            "password": "hunter2",
            "Authorization": "Bearer abc.def",
            "nested": { "client_secret": "s3cr3t", "accessKey": ["x"] },
            "input_tokens": 5,
            "has_token": true,
            "token": null,
            "url": "https://x.test/cb?token=abc123&page=2",
            "error": "401 for key sk-ant-0123456789abcdefghij",
            "header": "Authorization: Bearer eyJhbGciOi.payload",
            "aws": "used AKIAABCDEFGHIJKLMNOP",
            "gh": "ghp_abcdefghijklmnopqrstuvwxyz0123",
            "note": "password = hunter2",
            "plain": "nothing secret here",
        }));
        assert_eq!(redacted["password"], REDACTED);
        assert_eq!(redacted["Authorization"], REDACTED);
        assert_eq!(redacted["nested"]["client_secret"], REDACTED);
        assert_eq!(redacted["nested"]["accessKey"], REDACTED);
        assert_eq!(redacted["input_tokens"], 5);
        assert_eq!(redacted["has_token"], true);
        assert_eq!(redacted["token"], Value::Null);
        assert_eq!(redacted["url"], "https://x.test/cb?token=[redacted]&page=2");
        assert_eq!(redacted["error"], "401 for key [redacted]");
        assert_eq!(redacted["header"], "Authorization: Bearer [redacted]");
        assert_eq!(redacted["aws"], "used [redacted]");
        assert_eq!(redacted["gh"], REDACTED);
        assert_eq!(redacted["note"], "password = [redacted]");
        assert_eq!(redacted["plain"], "nothing secret here");
    }
}
