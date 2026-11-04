//! The trigger entity: `.nandi/triggers/<id>.yaml`, a rule that starts an
//! agent or workflow run when something happens.
//!
//! ```yaml
//! id: daily-digest
//! name: Daily AI News Digest
//! enabled: true
//! source:
//!   type: cron
//!   config:
//!     schedule: "0 9 * * 1-5"
//! target:
//!   type: agent
//!   id: research-agent
//! input_template: |
//!   Find AI infrastructure news from the last 24h and summarize the top 5.
//! priority: 80
//! rate_limit_seconds: 3600
//! ```
//!
//! Webhook and device triggers authenticate requests with a secret. The
//! file names only its keyring entry (`secretRef`, default
//! `nandi.trigger.<id>`); a `secret` key in a file is rejected.

use std::collections::BTreeMap;

use croner::parser::{CronParser, Seconds};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::calendar::CalendarSpec;
use super::ids::name_rule;
use super::settings::SecretRef;
use super::{check_len, AgentId, DomainError, DomainResult, TriggerId, WorkflowId};

pub const MAX_TEMPLATE_CHARS: usize = 100_000;
pub const MAX_PRIORITY: i32 = 1_000;
/// Internal events an `event` trigger can watch.
pub const EVENT_NAMES: &[&str] = &["agent.turn_done", "workflow.done", "workflow.failed"];

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronSpec {
    schedule: String,
}

impl CronSpec {
    /// 5 fields (minute precision) or 6 with seconds first.
    pub fn new(schedule: impl Into<String>) -> DomainResult<Self> {
        let schedule = schedule.into().trim().to_string();
        CronParser::builder()
            .seconds(Seconds::Optional)
            .build()
            .parse(&schedule)
            .map_err(|e| {
                DomainError::invalid("cron", "schedule", format!("'{schedule}' is invalid: {e}"))
            })?;
        Ok(Self { schedule })
    }

    pub fn schedule(&self) -> &str {
        &self.schedule
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEvent {
    Create,
    Modify,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSpec {
    /// A folder; `~` is the home folder. Whether it exists is checked when
    /// watching, not here (the domain does no I/O).
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// Empty: create + modify
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<FileEvent>,
    #[serde(
        default,
        alias = "debounce_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub debounce_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recursive: bool,
}

impl FileSpec {
    fn check(&self) -> DomainResult<()> {
        if self.path.trim().is_empty() {
            return Err(DomainError::invalid("file trigger", "path", "is required"));
        }
        for pattern in &self.patterns {
            glob::Pattern::new(pattern).map_err(|e| {
                DomainError::invalid("file trigger", "patterns", format!("'{pattern}': {e}"))
            })?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookSpec {
    #[serde(default, alias = "secret_ref", skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<SecretRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSpec {
    #[serde(alias = "device_id")]
    pub device_id: String,
    /// Empty: every event
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    #[serde(default, alias = "secret_ref", skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<SecretRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSpec {
    #[serde(alias = "event_name")]
    pub event_name: String,
    /// Payload fields that must match
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub filter: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerSource {
    Cron(CronSpec),
    Calendar(CalendarSpec),
    File(FileSpec),
    Webhook(WebhookSpec),
    Device(DeviceSpec),
    Event(EventSpec),
}

impl TriggerSource {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Cron(_) => "cron",
            Self::Calendar(_) => "calendar",
            Self::File(_) => "file",
            Self::Webhook(_) => "webhook",
            Self::Device(_) => "device",
            Self::Event(_) => "event",
        }
    }

    /// Queue priority when the trigger doesn't set one (lower runs sooner).
    pub fn default_priority(&self) -> i32 {
        match self {
            Self::Cron(_) | Self::Calendar(_) => 80,
            Self::File(_) => 70,
            Self::Webhook(_) | Self::Event(_) => 60,
            Self::Device(_) => 50,
        }
    }

    pub fn has_secret(&self) -> bool {
        matches!(self, Self::Webhook(_) | Self::Device(_))
    }

    /// Parses `source: {type, config}`.
    pub fn parse(source_type: &str, config: serde_yaml::Value) -> DomainResult<Self> {
        let config = match config {
            serde_yaml::Value::Null => serde_yaml::Value::Mapping(Default::default()),
            serde_yaml::Value::Mapping(map) => {
                if map.contains_key("secret") {
                    return Err(DomainError::invalid(
                        "trigger",
                        "source.config",
                        "must not contain a secret: secrets live in the keyring (secretRef)",
                    ));
                }
                serde_yaml::Value::Mapping(map)
            }
            _ => {
                return Err(DomainError::invalid(
                    "trigger",
                    "source.config",
                    "must be a mapping",
                ))
            }
        };
        fn spec<T: for<'de> Deserialize<'de>>(
            kind: &str,
            config: serde_yaml::Value,
        ) -> DomainResult<T> {
            serde_yaml::from_value(config)
                .map_err(|e| DomainError::parse(format!("{kind} config"), e))
        }
        let source = match source_type {
            "cron" => {
                #[derive(Deserialize)]
                struct Raw {
                    schedule: String,
                }
                Self::Cron(CronSpec::new(spec::<Raw>("cron", config)?.schedule)?)
            }
            "calendar" => {
                let json = serde_json::to_value(config)
                    .map_err(|e| DomainError::parse("calendar config", e))?;
                Self::Calendar(CalendarSpec::from_value(json)?)
            }
            "file" => Self::File(spec("file", config)?),
            "webhook" => Self::Webhook(spec("webhook", config)?),
            "device" => Self::Device(spec("device", config)?),
            "event" => Self::Event(spec("event", config)?),
            other => {
                return Err(DomainError::invalid(
                    "trigger",
                    "source.type",
                    format!("'{other}' is not one of cron, calendar, file, webhook, device, event"),
                ))
            }
        };
        source.check()?;
        Ok(source)
    }

    fn check(&self) -> DomainResult<()> {
        match self {
            Self::Cron(_) | Self::Calendar(_) | Self::Webhook(_) => Ok(()),
            Self::File(file) => file.check(),
            Self::Device(device) => name_rule(&device.device_id).map_err(|reason| {
                DomainError::invalid(
                    "device trigger",
                    "deviceId",
                    format!("'{}' {reason}", device.device_id),
                )
            }),
            Self::Event(event) => {
                if !EVENT_NAMES.contains(&event.event_name.as_str()) {
                    return Err(DomainError::invalid(
                        "event trigger",
                        "eventName",
                        format!(
                            "'{}' is unknown (expected one of: {})",
                            event.event_name,
                            EVENT_NAMES.join(", ")
                        ),
                    ));
                }
                if event.filter.values().any(|v| v.is_object() || v.is_array()) {
                    return Err(DomainError::invalid(
                        "event trigger",
                        "filter",
                        "values must be strings, numbers, or booleans",
                    ));
                }
                Ok(())
            }
        }
    }

    /// The config as JSON (camelCase keys), as the trigger engine reads it.
    pub fn config_json(&self) -> Value {
        let value = match self {
            Self::Cron(spec) => serde_json::to_value(spec),
            Self::Calendar(spec) => serde_json::to_value(spec),
            Self::File(spec) => serde_json::to_value(spec),
            Self::Webhook(spec) => serde_json::to_value(spec),
            Self::Device(spec) => serde_json::to_value(spec),
            Self::Event(spec) => serde_json::to_value(spec),
        };
        value.unwrap_or(Value::Null)
    }

    fn config_yaml(&self) -> serde_yaml::Value {
        serde_yaml::to_value(self.config_json()).unwrap_or(serde_yaml::Value::Null)
    }

    fn declared_secret_ref(&self) -> Option<&SecretRef> {
        match self {
            Self::Webhook(spec) => spec.secret_ref.as_ref(),
            Self::Device(spec) => spec.secret_ref.as_ref(),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Agent(AgentId),
    Workflow(WorkflowId),
}

impl Target {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Agent(_) => "agent",
            Self::Workflow(_) => "workflow",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Agent(id) => id.as_str(),
            Self::Workflow(id) => id.as_str(),
        }
    }

    pub fn parse(target_type: &str, id: &str) -> DomainResult<Self> {
        match target_type {
            "agent" => Ok(Self::Agent(AgentId::parse(id)?)),
            "workflow" => Ok(Self::Workflow(WorkflowId::parse(id)?)),
            other => Err(DomainError::invalid(
                "trigger",
                "target.type",
                format!("'{other}' is not agent or workflow"),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    id: TriggerId,
    name: String,
    enabled: bool,
    source: TriggerSource,
    target: Target,
    input_template: Option<String>,
    priority: i32,
    /// At most one fire per this many seconds
    rate_limit_seconds: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerFile {
    id: String,
    name: String,
    #[serde(default = "enabled_default")]
    enabled: bool,
    source: SourceSection,
    target: TargetSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate_limit_seconds: Option<u32>,
}

fn enabled_default() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSection {
    #[serde(rename = "type")]
    source_type: String,
    #[serde(default)]
    config: serde_yaml::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetSection {
    #[serde(rename = "type")]
    target_type: String,
    id: String,
}

/// Everything needed to create a trigger. `Trigger::new` validates it.
#[derive(Debug, Clone)]
pub struct TriggerDraft {
    pub id: TriggerId,
    pub name: String,
    pub enabled: bool,
    pub source: TriggerSource,
    pub target: Target,
    pub input_template: Option<String>,
    /// `None`: the source's default
    pub priority: Option<i32>,
    pub rate_limit_seconds: Option<u32>,
}

impl Trigger {
    pub fn new(draft: TriggerDraft) -> DomainResult<Self> {
        let trigger = Self {
            priority: draft
                .priority
                .unwrap_or_else(|| draft.source.default_priority()),
            id: draft.id,
            name: draft.name.trim().to_string(),
            enabled: draft.enabled,
            source: draft.source,
            target: draft.target,
            input_template: draft.input_template.filter(|t| !t.trim().is_empty()),
            rate_limit_seconds: draft.rate_limit_seconds.filter(|s| *s > 0),
        };
        trigger.validate()?;
        Ok(trigger)
    }

    pub fn validate(&self) -> DomainResult<()> {
        let label = format!("Trigger '{}'", self.id);
        if self.name.is_empty() {
            return Err(DomainError::invalid(label, "name", "is required"));
        }
        if !(0..=MAX_PRIORITY).contains(&self.priority) {
            return Err(DomainError::invalid(
                label,
                "priority",
                format!("must be between 0 and {MAX_PRIORITY}"),
            ));
        }
        if let Some(template) = &self.input_template {
            check_len(&label, "input_template", template, MAX_TEMPLATE_CHARS)?;
        }
        self.source.check()
    }

    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        let file: TriggerFile =
            serde_yaml::from_str(content).map_err(|e| DomainError::parse("trigger YAML", e))?;
        Self::new(TriggerDraft {
            id: TriggerId::parse(file.id)?,
            name: file.name,
            enabled: file.enabled,
            source: TriggerSource::parse(&file.source.source_type, file.source.config)?,
            target: Target::parse(&file.target.target_type, &file.target.id)?,
            input_template: file.input_template,
            priority: file.priority,
            rate_limit_seconds: file.rate_limit_seconds,
        })
    }

    pub fn to_yaml(&self) -> String {
        let file = TriggerFile {
            id: self.id.to_string(),
            name: self.name.clone(),
            enabled: self.enabled,
            source: SourceSection {
                source_type: self.source.type_name().to_string(),
                config: self.source.config_yaml(),
            },
            target: TargetSection {
                target_type: self.target.type_name().to_string(),
                id: self.target.id().to_string(),
            },
            input_template: self.input_template.clone(),
            priority: Some(self.priority),
            rate_limit_seconds: self.rate_limit_seconds,
        };
        serde_yaml::to_string(&file).unwrap_or_default()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The keyring entry holding this trigger's secret (webhook and device
    /// triggers only).
    pub fn secret_ref(&self) -> Option<SecretRef> {
        if !self.source.has_secret() {
            return None;
        }
        self.source
            .declared_secret_ref()
            .cloned()
            .or_else(|| SecretRef::parse(format!("nandi.trigger.{}", self.id)).ok())
    }

    pub fn id(&self) -> &TriggerId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn source(&self) -> &TriggerSource {
        &self.source
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn input_template(&self) -> Option<&str> {
        self.input_template.as_deref()
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    pub fn rate_limit_seconds(&self) -> Option<u32> {
        self.rate_limit_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENTED: &str = r#"
id: daily-digest
name: Daily AI News Digest
enabled: true
source:
  type: cron
  config:
    schedule: "0 9 * * 1-5"      # 9am weekdays
target:
  type: agent
  id: research-agent
input_template: |
  Find AI infrastructure news from the last 24h and summarize the top 5.
priority: 80
rate_limit_seconds: 3600
"#;

    #[test]
    fn parses_and_round_trips_the_documented_format() {
        let trigger = Trigger::from_yaml(DOCUMENTED).unwrap();
        assert_eq!(trigger.id().as_str(), "daily-digest");
        assert!(
            matches!(trigger.source(), TriggerSource::Cron(c) if c.schedule() == "0 9 * * 1-5")
        );
        assert_eq!(
            trigger.target(),
            &Target::Agent(AgentId::parse("research-agent").unwrap())
        );
        assert_eq!(trigger.rate_limit_seconds(), Some(3600));
        assert!(trigger.secret_ref().is_none());
        assert_eq!(Trigger::from_yaml(&trigger.to_yaml()).unwrap(), trigger);
    }

    #[test]
    fn webhooks_reference_their_secret() {
        let yaml =
            "id: hook\nname: Hook\nsource: {type: webhook}\ntarget: {type: workflow, id: daily}\n";
        let trigger = Trigger::from_yaml(yaml).unwrap();
        assert_eq!(trigger.secret_ref().unwrap().as_str(), "nandi.trigger.hook");
        assert_eq!(trigger.priority(), 60, "the source's default");
        let again = Trigger::from_yaml(&trigger.to_yaml()).unwrap();
        assert_eq!(again, trigger);

        let leaked = "id: hook\nname: Hook\nsource: {type: webhook, config: {secret: abc}}\ntarget: {type: agent, id: a}\n";
        let error = Trigger::from_yaml(leaked).unwrap_err().to_string();
        assert!(error.contains("must not contain a secret"), "{error}");
    }

    #[test]
    fn validate_catches_bad_rules() {
        for (yaml, expected) in [
            ("id: t\nname: T\nsource: {type: cron, config: {schedule: nope}}\ntarget: {type: agent, id: a}", "schedule"),
            ("id: t\nname: T\nsource: {type: cron, config: {schedule: '* * * * *'}}", "target"),
            ("id: t\nname: T\nsource: {type: cron, config: {schedule: '* * * * *'}}\ntarget: {type: agent, id: ''}", "agent id"),
            ("id: t\nname: T\nsource: {type: cron, config: {schedule: '* * * * *'}}\ntarget: {type: robot, id: a}", "target.type"),
            ("id: t\nname: T\nsource: {type: sms}\ntarget: {type: agent, id: a}", "source.type"),
            ("id: t\nname: T\nsource: {type: file, config: {path: /tmp, patterns: ['[bad']}}\ntarget: {type: agent, id: a}", "patterns"),
            ("id: t\nname: T\nsource: {type: device, config: {deviceId: ../x}}\ntarget: {type: agent, id: a}", "deviceId"),
            ("id: t\nname: T\nsource: {type: event, config: {eventName: nope}}\ntarget: {type: agent, id: a}", "unknown"),
            ("id: t\nname: T\nsource: {type: event, config: {eventName: workflow.done, filter: {a: [1]}}}\ntarget: {type: agent, id: a}", "filter"),
            ("id: t\nname: T\npriority: 5000\nsource: {type: webhook}\ntarget: {type: agent, id: a}", "priority"),
            ("id: t\nname: ' '\nsource: {type: webhook}\ntarget: {type: agent, id: a}", "name"),
            ("id: t\nname: T\nsource: {type: webhook}\ntarget: {type: agent, id: a}\ntypo: 1", "unknown field"),
        ] {
            let error = Trigger::from_yaml(yaml).unwrap_err().to_string();
            assert!(error.contains(expected), "{yaml}: {error}");
        }
    }

    #[test]
    fn exposes_configs_as_json() {
        let yaml = "id: f\nname: F\nsource:\n  type: file\n  config: {path: ~/in, patterns: ['*.pdf'], debounce_ms: 500}\ntarget: {type: agent, id: a}\n";
        let trigger = Trigger::from_yaml(yaml).unwrap();
        let config = trigger.source().config_json();
        assert_eq!(config["path"], "~/in");
        assert_eq!(config["debounceMs"], 500);
        assert!(config.get("recursive").is_none());
    }
}
