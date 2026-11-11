//! Trigger types: rules, fires, the events sources publish, and the
//! per-source configs (validated on every create/update).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use specta::Type;

use crate::model::{SourceType, Timestamp};
use crate::services::agents;
use crate::services::domain::calendar::CalendarSpec;
use crate::services::orchestrator::{PRIORITY_SCHEDULED, PRIORITY_WORKFLOW_NORMAL};

/// Fires per trigger per minute, unless the trigger sets a stricter
/// `rate_limit_seconds`. Enforced on webhook and device requests.
pub const DEFAULT_REQUESTS_PER_MINUTE: u32 = 60;
/// More than this many fires in `STORM_WINDOW` disables a trigger
pub const STORM_MAX_FIRES: usize = 5;
pub const STORM_WINDOW_MS: i64 = 10_000;
/// File events for one path within this window collapse into one fire
pub const DEFAULT_DEBOUNCE_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TriggerSourceType {
    Cron,
    Calendar,
    File,
    Webhook,
    Device,
    Event,
}

impl TriggerSourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cron => "cron",
            Self::Calendar => "calendar",
            Self::File => "file",
            Self::Webhook => "webhook",
            Self::Device => "device",
            Self::Event => "event",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "cron" => Self::Cron,
            "calendar" => Self::Calendar,
            "file" => Self::File,
            "webhook" => Self::Webhook,
            "device" => Self::Device,
            "event" => Self::Event,
            _ => return None,
        })
    }

    /// The `runs.source_type` of runs this source dispatches
    pub fn run_source(self) -> SourceType {
        match self {
            Self::Cron => SourceType::Cron,
            Self::Calendar => SourceType::Calendar,
            Self::File => SourceType::File,
            Self::Webhook => SourceType::Webhook,
            Self::Device => SourceType::Device,
            Self::Event => SourceType::Event,
        }
    }

    /// Queue priority when the trigger doesn't set one (lower runs sooner)
    pub fn default_priority(self) -> i32 {
        match self {
            Self::Cron | Self::Calendar => PRIORITY_SCHEDULED, // 80
            Self::File => 70,                                  // often time-sensitive
            Self::Webhook => 60,                               // external systems expect a response
            Self::Device => PRIORITY_WORKFLOW_NORMAL,          // 50: physical-world events
            Self::Event => 60,
        }
    }

    /// Whether requests must carry the trigger's secret
    pub fn has_secret(self) -> bool {
        matches!(self, Self::Webhook | Self::Device)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TriggerTargetType {
    Agent,
    Workflow,
}

impl TriggerTargetType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Workflow => "workflow",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "workflow" {
            Self::Workflow
        } else {
            Self::Agent
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TriggerFireStatus {
    /// A run was dispatched (queued behind the agent's current turn, if any)
    Queued,
    RateLimited,
    Error,
    /// Matched but deliberately not dispatched (e.g. its own run's event)
    Skipped,
}

impl TriggerFireStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::RateLimited => "rate_limited",
            Self::Error => "error",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "queued" => Self::Queued,
            "rate_limited" => Self::RateLimited,
            "skipped" => Self::Skipped,
            _ => Self::Error,
        }
    }
}

/// A trigger rule. In API responses `source_config` never holds the secret
/// (see `has_secret` and `SavedTrigger`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source_type: TriggerSourceType,
    pub source_config: Value,
    pub target_type: TriggerTargetType,
    pub target_id: String,
    pub input_template: Option<String>,
    pub priority: i32,
    pub rate_limit_seconds: Option<u32>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_fired_at: Option<Timestamp>,
    /// A webhook/device secret is set (it's only ever shown once)
    pub has_secret: bool,
}

impl Trigger {
    pub fn secret(&self) -> Option<&str> {
        self.source_config.get("secret").and_then(Value::as_str)
    }

    /// The shape clients see: the secret removed.
    pub fn public(&self) -> Trigger {
        let mut trigger = self.clone();
        trigger.has_secret = self.has_secret || self.secret().is_some();
        if let Some(config) = trigger.source_config.as_object_mut() {
            config.remove("secret");
        }
        trigger
    }
}

/// What clients send to create or update a trigger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInput {
    /// Defaults to a slug of the name (it's also the YAML file name)
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    pub source_type: TriggerSourceType,
    /// Cron: `{schedule}`. Calendar: `{times, days?, skipDates?, people?,
    /// rotate?, startDate?, byDay?, cover?}`. File: `{path, patterns?, events?, debounceMs?,
    /// recursive?}`. Webhook: `{}`. Device: `{deviceId, events?}`. Event:
    /// `{eventName, filter?}`. Secrets are generated, never sent.
    pub source_config: Value,
    pub target_type: TriggerTargetType,
    pub target_id: String,
    #[serde(default)]
    pub input_template: Option<String>,
    /// Defaults to the source type's priority
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub rate_limit_seconds: Option<u32>,
}

/// A saved trigger, plus its secret when one was just generated (shown once).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedTrigger {
    pub trigger: Trigger,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerFire {
    pub id: String,
    pub trigger_id: String,
    pub fired_at: Timestamp,
    pub payload: Value,
    /// The agent run, or the workflow run, it dispatched
    pub run_id: Option<String>,
    pub status: TriggerFireStatus,
    pub error: Option<String>,
    /// Where that run is now, read with the fire (never stored on it);
    /// `None` without a run
    #[serde(default)]
    pub run_status: Option<FireRunStatus>,
}

/// The status of the run a fire dispatched, agent or workflow run, so a
/// fire reads "Completed" rather than staying "Queued"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FireRunStatus {
    Queued,
    Running,
    AwaitingApproval,
    /// A workflow run at a manual step
    Waiting,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl FireRunStatus {
    /// `runs.status` or `workflow_runs.status` as stored
    pub fn parse(status: &str) -> Option<Self> {
        Some(match status {
            "queued" => Self::Queued,
            // A compacted run carries on in a child run
            "running" | "compacted" => Self::Running,
            "awaiting_approval" => Self::AwaitingApproval,
            "waiting" => Self::Waiting,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// A fire with its trigger's name, for the dashboard.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerFireSummary {
    pub fire: TriggerFire,
    pub trigger_name: String,
    pub source_type: TriggerSourceType,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerStats {
    pub enabled: u32,
    pub fired_today: u32,
}

/// Something a source saw, addressed to one trigger.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerEvent {
    pub trigger_id: String,
    pub source: TriggerSourceType,
    pub payload: Value,
    /// A manual test fire: allowed while disabled, never a storm
    pub test: bool,
    /// A backup taking over a calendar duty (`handover`): the same duty, so
    /// never rate limited. Only Nandi sets it, never a payload
    pub handover: bool,
}

// ---------------------------------------------------------------------------
// Per-source configs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronConfig {
    /// 5 fields (minute precision) or 6 with seconds first
    pub schedule: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEventKind {
    Create,
    Modify,
    Delete,
}

impl FileEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Modify => "modify",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileConfig {
    /// A folder; `~` is the home folder
    pub path: String,
    /// Glob patterns on the file name, e.g. `*.pdf`; empty matches all
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Defaults to create + modify
    #[serde(default)]
    pub events: Vec<FileEventKind>,
    #[serde(default, alias = "debounce_ms")]
    pub debounce_ms: Option<u64>,
    #[serde(default)]
    pub recursive: bool,
}

impl FileConfig {
    pub fn events(&self) -> Vec<FileEventKind> {
        if self.events.is_empty() {
            vec![FileEventKind::Create, FileEventKind::Modify]
        } else {
            self.events.clone()
        }
    }

    pub fn debounce(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.debounce_ms.unwrap_or(DEFAULT_DEBOUNCE_MS))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceConfig {
    #[serde(alias = "device_id")]
    pub device_id: String,
    /// Event names to fire on; empty fires on every event
    #[serde(default)]
    pub events: Vec<String>,
}

/// Internal events an `event` trigger can watch.
pub const EVENT_NAMES: &[&str] = &["agent.turn_done", "workflow.done", "workflow.failed"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventConfig {
    #[serde(alias = "event_name")]
    pub event_name: String,
    /// Payload fields that must match, e.g. `{agent_id, status}`
    #[serde(default)]
    pub filter: Map<String, Value>,
}

fn parse_config<T: for<'de> Deserialize<'de>>(kind: &str, config: &Value) -> Result<T, String> {
    serde_json::from_value(config.clone()).map_err(|e| format!("Invalid {kind} config: {e}"))
}

pub fn cron_config(config: &Value) -> Result<CronConfig, String> {
    parse_config("cron", config)
}

pub fn file_config(config: &Value) -> Result<FileConfig, String> {
    parse_config("file", config)
}

pub fn device_config(config: &Value) -> Result<DeviceConfig, String> {
    parse_config("device", config)
}

pub fn event_config(config: &Value) -> Result<EventConfig, String> {
    parse_config("event", config)
}

/// `~/x` → the home folder's `x`; a relative path is under the Nandi
/// home, as for shared folders (`connectors::expand_home`).
pub fn expand_home(path: &str) -> std::path::PathBuf {
    crate::services::connectors::expand_home(path)
}

/// Checks a source config and returns it normalized (secret kept as-is).
/// `check_paths` is off when loading from the repo: a missing folder is
/// then a warning at watch time, not a reason to drop the trigger.
pub fn validate_config(
    source_type: TriggerSourceType,
    config: &Value,
    check_paths: bool,
) -> Result<Value, String> {
    let mut config = match config {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        _ => return Err("Source config must be an object".to_string()),
    };
    match source_type {
        TriggerSourceType::Cron => {
            let cron = cron_config(&Value::Object(config.clone()))?;
            super::cron::parse_schedule(&cron.schedule)?;
        }
        TriggerSourceType::Calendar => {
            config.remove("secret");
            CalendarSpec::from_value(Value::Object(config.clone())).map_err(|e| e.to_string())?;
        }
        TriggerSourceType::File => {
            let file = file_config(&Value::Object(config.clone()))?;
            if file.path.trim().is_empty() {
                return Err("File triggers need a folder to watch".to_string());
            }
            for pattern in &file.patterns {
                glob::Pattern::new(pattern)
                    .map_err(|e| format!("Invalid pattern '{pattern}': {e}"))?;
            }
            let path = expand_home(&file.path);
            if check_paths && !path.is_dir() {
                return Err(format!("{} is not a folder", path.display()));
            }
        }
        TriggerSourceType::Webhook => {}
        TriggerSourceType::Device => {
            let device = device_config(&Value::Object(config.clone()))?;
            agents::validate_agent_id(&device.device_id)
                .map_err(|_| format!("Invalid device id '{}'", device.device_id))?;
        }
        TriggerSourceType::Event => {
            let event = event_config(&Value::Object(config.clone()))?;
            if !EVENT_NAMES.contains(&event.event_name.as_str()) {
                return Err(format!(
                    "Unknown event '{}' (expected one of: {})",
                    event.event_name,
                    EVENT_NAMES.join(", ")
                ));
            }
            if event.filter.values().any(|v| v.is_object() || v.is_array()) {
                return Err("Event filter values must be strings, numbers, or booleans".into());
            }
        }
    }
    if !source_type.has_secret() {
        config.remove("secret");
    }
    Ok(Value::Object(config))
}

/// A slug for an ID: lowercase letters, digits, and dashes.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "trigger".to_string()
    } else {
        slug.chars().take(48).collect()
    }
}

/// A fresh 256-bit secret (64 hex characters).
pub fn generate_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Constant-time comparison, so response timing doesn't leak the secret.
pub fn secrets_match(expected: &str, candidate: &str) -> bool {
    let (a, b) = (expected.as_bytes(), candidate.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_sources_to_priorities_and_run_sources() {
        assert_eq!(TriggerSourceType::Cron.default_priority(), 80);
        assert_eq!(TriggerSourceType::Calendar.default_priority(), 80);
        assert_eq!(TriggerSourceType::File.default_priority(), 70);
        assert_eq!(TriggerSourceType::Webhook.default_priority(), 60);
        assert_eq!(TriggerSourceType::Device.default_priority(), 50);
        assert_eq!(TriggerSourceType::Event.default_priority(), 60);
        assert_eq!(TriggerSourceType::Device.run_source(), SourceType::Device);
    }

    #[test]
    fn validates_configs_per_source() {
        let ok = |t, c: Value| validate_config(t, &c, true);
        assert!(ok(TriggerSourceType::Cron, json!({"schedule": "0 9 * * 1-5"})).is_ok());
        assert!(ok(TriggerSourceType::Cron, json!({"schedule": "nope"})).is_err());
        assert!(ok(
            TriggerSourceType::Calendar,
            json!({"times": ["09:00"], "people": ["a", "b"], "startDate": "2026-10-26"})
        )
        .is_ok());
        assert!(ok(TriggerSourceType::Calendar, json!({"times": ["9am"]})).is_err());
        assert!(ok(
            TriggerSourceType::File,
            json!({"path": "/definitely/not/here"})
        )
        .is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy();
        assert!(ok(
            TriggerSourceType::File,
            json!({"path": path, "patterns": ["*.pdf"]})
        )
        .is_ok());
        assert!(ok(
            TriggerSourceType::File,
            json!({"path": path, "patterns": ["[bad"]})
        )
        .is_err());
        assert!(validate_config(TriggerSourceType::File, &json!({"path": "/gone"}), false).is_ok());
        assert!(ok(TriggerSourceType::Device, json!({"deviceId": "printer-01"})).is_ok());
        assert!(ok(TriggerSourceType::Device, json!({"deviceId": "../x"})).is_err());
        assert!(ok(
            TriggerSourceType::Event,
            json!({"eventName": "agent.turn_done", "filter": {"agent_id": "a"}})
        )
        .is_ok());
        assert!(ok(TriggerSourceType::Event, json!({"eventName": "nope"})).is_err());
        // Only webhook and device configs keep a secret
        let cleaned = ok(
            TriggerSourceType::Cron,
            json!({"schedule": "* * * * *", "secret": "x"}),
        );
        assert_eq!(cleaned.unwrap().get("secret"), None);
    }

    #[test]
    fn hides_the_secret_from_clients() {
        let trigger = Trigger {
            id: "t".into(),
            name: "T".into(),
            enabled: true,
            source_type: TriggerSourceType::Webhook,
            source_config: json!({"secret": "s3cret"}),
            target_type: TriggerTargetType::Agent,
            target_id: "a".into(),
            input_template: None,
            priority: 60,
            rate_limit_seconds: None,
            created_at: 0.0,
            updated_at: 0.0,
            last_fired_at: None,
            has_secret: false,
        };
        let public = trigger.public();
        assert!(public.has_secret);
        assert_eq!(public.source_config, json!({}));
        assert!(!serde_json::to_string(&public).unwrap().contains("s3cret"));
    }

    #[test]
    fn slugs_and_secrets() {
        assert_eq!(slugify("Daily AI News Digest!"), "daily-ai-news-digest");
        assert_eq!(slugify("  ---  "), "trigger");
        let secret = generate_secret();
        assert_eq!(secret.len(), 64);
        assert!(secrets_match(&secret, &secret.clone()));
        assert!(!secrets_match(&secret, &secret[..63]));
        assert!(!secrets_match("abc", "abd"));
    }

    #[test]
    fn expands_the_home_folder() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_home("~/Downloads"), home.join("Downloads"));
        assert_eq!(expand_home("/tmp/x"), std::path::PathBuf::from("/tmp/x"));
        assert_eq!(expand_home("~other"), std::path::PathBuf::from("~other"));
    }
}
