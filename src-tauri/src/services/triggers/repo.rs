//! Triggers as files: `triggers/<id>.yaml` in the agents repo. The format
//! and its rules live in `domain::trigger`; reading and writing the files
//! is `repos::TriggerRepository`. This module maps between the engine's
//! `Trigger` row (with its secret) and the domain entity (without it).

use super::schema::Trigger;
#[cfg(test)]
use super::schema::{TriggerInput, TriggerSourceType, TriggerTargetType};
use crate::services::domain::trigger::{Target, TriggerDraft, TriggerSource};
use crate::services::domain::{self, TriggerId};

/// The rule as a domain entity, secret removed.
pub fn to_domain(trigger: &Trigger) -> Result<domain::Trigger, String> {
    let public = trigger.public();
    let config = serde_yaml::to_value(&public.source_config)
        .map_err(|e| format!("Invalid source config: {e}"))?;
    Ok(domain::Trigger::new(TriggerDraft {
        id: TriggerId::parse(public.id)?,
        name: public.name,
        enabled: public.enabled,
        source: TriggerSource::parse(public.source_type.as_str(), config)?,
        target: Target::parse(public.target_type.as_str(), &public.target_id)?,
        input_template: public.input_template,
        priority: Some(public.priority),
        rate_limit_seconds: public.rate_limit_seconds,
    })?)
}

/// What the engine builds a rule from (it keeps or generates the secret).
#[cfg(test)]
pub fn to_input(trigger: &domain::Trigger) -> TriggerInput {
    let source = trigger.source();
    TriggerInput {
        id: Some(trigger.id().to_string()),
        name: trigger.name().to_string(),
        enabled: Some(trigger.enabled()),
        source_type: source
            .type_name()
            .parse()
            .expect("a domain trigger source has a known type"),
        source_config: source.config_json(),
        target_type: trigger
            .target()
            .type_name()
            .parse()
            .expect("a domain trigger target has a known type"),
        target_id: trigger.target().id().to_string(),
        input_template: trigger.input_template().map(str::to_string),
        priority: Some(trigger.priority()),
        rate_limit_seconds: trigger.rate_limit_seconds(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    fn webhook_trigger() -> Trigger {
        Trigger {
            id: "hook".into(),
            name: "Hook".into(),
            enabled: false,
            source_type: TriggerSourceType::Webhook,
            source_config: json!({"secret": "never-in-git"}),
            target_type: TriggerTargetType::Workflow,
            target_id: "daily-report".into(),
            input_template: None,
            priority: 60,
            rate_limit_seconds: None,
            created_at: 0.0,
            updated_at: 0.0,
            last_fired_at: None,
            has_secret: true,
        }
    }

    #[test]
    fn parses_the_documented_format() {
        let input = to_input(&domain::Trigger::from_yaml(DOCUMENTED).unwrap());
        assert_eq!(input.id.as_deref(), Some("daily-digest"));
        assert_eq!(input.source_type, TriggerSourceType::Cron);
        assert_eq!(input.source_config["schedule"], "0 9 * * 1-5");
        assert_eq!(input.target_type, TriggerTargetType::Agent);
        assert_eq!(input.rate_limit_seconds, Some(3600));
        assert!(input.input_template.unwrap().starts_with("Find AI"));
    }

    #[test]
    fn round_trips_without_the_secret() {
        let yaml = to_domain(&webhook_trigger()).unwrap().to_yaml();
        assert!(!yaml.contains("never-in-git"), "{yaml}");
        let input = to_input(&domain::Trigger::from_yaml(&yaml).unwrap());
        assert_eq!(input.id.as_deref(), Some("hook"));
        assert_eq!(input.enabled, Some(false));
        assert_eq!(input.target_type, TriggerTargetType::Workflow);
        assert_eq!(input.source_config, json!({}));
    }
}
