//! `~/.nandi/local/logs/connectors.log`: one JSON line per connector tool
//! call. Like the other audit logs it records metadata only: argument
//! *names*, never their values or the tool's output. Who may use a
//! connector (grants, access requests, policy refusals) are decisions, kept
//! in the decisions file (`inbox::audit`), not here.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::utils::nandi_config;

pub const FILE_NAME: &str = "connectors.log";

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
    #[cfg(test)]
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

    /// Appends one entry (see `audit::append`).
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
        if let Some(path) = self.path() {
            crate::audit::append(path, &entry).await;
        }
    }
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
    }

    #[test]
    fn the_default_sink_never_writes_in_tests() {
        assert_eq!(AuditSink::default().path(), None);
    }
}
