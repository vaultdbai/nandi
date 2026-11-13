//! `~/.nandi/local/logs/pilots.log`: one JSON line per pilot tool call
//! (allowed, denied, or failed), with secret-looking arguments redacted.
//!
//! The line shape matches `audit::AuditEntry` (plus `pilotId`), so the
//! audit log viewer can read it like the other logs.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::utils::nandi_config;

pub const FILE_NAME: &str = "pilots.log";
pub const REDACTED: &str = "[redacted]";

static SECRET_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)password|token|secret|key").expect("valid regex"));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PilotAuditEntry {
    /// ms since the Unix epoch
    pub timestamp: f64,
    /// Always `pilots`
    pub kind: &'static str,
    pub agent_id: String,
    pub pilot_id: String,
    pub tool: String,
    pub args_redacted: Value,
    /// `ok` | `error` | `denied`
    pub outcome: String,
    pub duration_ms: Option<f64>,
}

impl PilotAuditEntry {
    pub fn new(agent_id: &str, pilot_id: &str, tool: &str, args: &Value) -> Self {
        Self {
            timestamp: crate::audit::now_ms() as f64,
            kind: "pilots",
            agent_id: agent_id.to_string(),
            pilot_id: pilot_id.to_string(),
            tool: tool.to_string(),
            args_redacted: redact(args),
            outcome: String::new(),
            duration_ms: None,
        }
    }
}

/// Replaces the value of every object key matching
/// `password|token|secret|key` (any case, at any depth), and any string
/// value that itself matches, with `[redacted]`.
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = if SECRET_KEY.is_match(k) {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact(v)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(s) if SECRET_KEY.is_match(s) => Value::String(REDACTED.to_string()),
        other => other.clone(),
    }
}

/// Appends `entry` (see `audit::append`).
pub async fn record(entry: &PilotAuditEntry) {
    // Tests must not write to the real ~/.nandi
    if cfg!(test) {
        return;
    }
    match nandi_config::logs_dir() {
        Ok(dir) => crate::audit::append(dir.join(FILE_NAME), entry).await,
        Err(e) => log::error!("Audit degraded: no folder for {FILE_NAME}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_secret_keys_at_any_depth() {
        let args = json!({
            "url": "https://example.com",
            "Password": "hunter2",
            "form": {"api_key": "k", "user": "me", "fields": [{"accessToken": "t"}]},
            "note": "my secret plan",
            "count": 3
        });
        assert_eq!(
            redact(&args),
            json!({
                "url": "https://example.com",
                "Password": REDACTED,
                "form": {"api_key": REDACTED, "user": "me", "fields": [{"accessToken": REDACTED}]},
                "note": REDACTED,
                "count": 3
            })
        );
    }

    #[test]
    fn entry_matches_the_audit_viewer_shape() {
        let entry = PilotAuditEntry::new("writer", "opendesk", "click", &json!({"token": "x"}));
        let line = serde_json::to_value(&entry).unwrap();
        assert_eq!(line["kind"], "pilots");
        assert_eq!(line["agentId"], "writer");
        assert_eq!(line["argsRedacted"]["token"], REDACTED);
        assert!(line.get("durationMs").is_some());
    }
}
