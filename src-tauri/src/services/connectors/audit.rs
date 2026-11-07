//! `~/.nandi/local/logs/connectors.log`: one JSON line per connector tool
//! call. Like the other audit logs it records metadata only: argument
//! *names*, never their values or the tool's output.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::utils::nandi_config;

pub const FILE_NAME: &str = "connectors.log";

/// Serializes appends so concurrent entries never interleave.
static WRITE_LOCK: Mutex<()> = Mutex::const_new(());

/// Shaped like `audit::AuditEntry`, so the audit log viewer can read it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry<'a> {
    /// ms since the Unix epoch
    pub timestamp: u64,
    pub kind: &'static str,
    pub agent_id: &'a str,
    pub connector_id: &'a str,
    pub tool: &'a str,
    pub args_redacted: Value,
    /// `ok` | `error` | `unavailable`
    pub outcome: &'a str,
    pub duration_ms: u64,
}

/// One grant decision or refusal, so "who let this agent read my mail, and
/// when" has an answer: `granted`, `removed`, `allowed_once`, `refused`
/// (the person said no), `forbidden` (a policy), `revoked_in_run` (a call
/// after its grant was removed)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessEntry {
    pub timestamp: u64,
    pub kind: &'static str,
    pub event: &'static str,
    pub agent_id: String,
    pub connector_id: String,
    /// `read` or `write`
    pub access: &'static str,
    /// `local` or `saved`, for grants
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<&'static str>,
    /// Who decided (the local user's name), or the policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl AccessEntry {
    pub fn new(event: &'static str, agent: &str, connector: &str, access: &'static str) -> Self {
        Self {
            timestamp: crate::audit::now_ms(),
            kind: "connector_access",
            event,
            agent_id: agent.to_string(),
            connector_id: connector.to_string(),
            access,
            scope: None,
            by: None,
            run_id: None,
        }
    }
}

/// Where entries go. The default is `~/.nandi/local/logs/connectors.log`,
/// which tests never write to.
#[derive(Debug, Clone, Default)]
pub struct AuditSink {
    path: Option<PathBuf>,
}

/// `{"path": "a.txt", "content": "..."}` -> `{"path": "[redacted]", "content": "[redacted]"}`
pub fn redact(args: &Value) -> Value {
    match args {
        Value::Object(map) => Value::Object(
            map.keys()
                .map(|key| (key.clone(), Value::String("[redacted]".into())))
                .collect(),
        ),
        Value::Null => Value::Null,
        _ => Value::String("[redacted]".into()),
    }
}

impl AuditSink {
    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.path {
            return Some(path.clone());
        }
        if cfg!(test) {
            return None;
        }
        nandi_config::logs_dir().ok().map(|dir| dir.join(FILE_NAME))
    }

    /// Appends one entry. Failures are logged, not returned: an unwritable
    /// audit log must not break the call being audited.
    pub async fn record(
        &self,
        agent_id: &str,
        connector_id: &str,
        tool: &str,
        args: &Value,
        outcome: &str,
        duration_ms: u64,
    ) {
        let entry = Entry {
            timestamp: crate::audit::now_ms(),
            kind: "connectors",
            agent_id,
            connector_id,
            tool,
            args_redacted: redact(args),
            outcome,
            duration_ms,
        };
        let Some(path) = self.path() else {
            return;
        };
        if let Err(e) = append(&path, &entry).await {
            log::error!("Failed to write the connector audit log: {e}");
        }
    }
}

impl AuditSink {
    /// Appends one access entry (failures are logged, as for calls)
    pub async fn record_access(&self, entry: &AccessEntry) {
        let Some(path) = self.path() else {
            return;
        };
        if let Err(e) = append(&path, entry).await {
            log::error!("Failed to write the connector audit log: {e}");
        }
    }
}

async fn append(path: &std::path::Path, entry: &impl Serialize) -> Result<(), String> {
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
    // tokio writes in the background; make sure the line is on disk
    file.flush().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn appends_json_lines_without_argument_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join(FILE_NAME);
        let sink = AuditSink::at(path.clone());
        let args = json!({"path": "secret-plans.txt", "content": "top secret"});
        sink.record("agent-1", "filesystem", "write_file", &args, "ok", 3)
            .await;
        sink.record("agent-1", "filesystem", "read_file", &json!({}), "error", 1)
            .await;

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("top secret") && !text.contains("secret-plans"));
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "connectors");
        assert_eq!(lines[0]["agentId"], "agent-1");
        assert_eq!(lines[0]["tool"], "write_file");
        assert_eq!(lines[0]["argsRedacted"]["content"], "[redacted]");
        assert_eq!(lines[1]["outcome"], "error");

        let mut granted = AccessEntry::new("granted", "chief-of-staff", "outlook", "read");
        granted.scope = Some("saved");
        granted.by = Some("Dev".into());
        sink.record_access(&granted).await;
        let text = std::fs::read_to_string(&path).unwrap();
        let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(last["kind"], "connector_access");
        assert_eq!(last["event"], "granted");
        assert_eq!(last["scope"], "saved");
        assert_eq!(last["by"], "Dev");
    }

    #[test]
    fn the_default_sink_never_writes_in_tests() {
        assert_eq!(AuditSink::default().path(), None);
    }
}
