//! Device source: `POST /api/triggers/device/{device_id}` with
//! `{event, data}` and a device trigger's secret in `X-Trigger-Secret`.
//! Devices (or an MCP server on the host relaying for them) can also use
//! a plain webhook trigger.
//!
//! Every enabled device trigger for that device whose secret matches and
//! whose `events` include the event (or is empty) fires. Device events are
//! accepted whatever the target agent's state: if its container is
//! stopped, the run waits in the agent's queue like any other.

use serde::Deserialize;
use serde_json::{json, Value};

use super::schema::{device_config, secrets_match, Trigger, TriggerSourceType};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DeviceEvent {
    pub event: String,
    #[serde(default)]
    pub data: Value,
}

/// Which outcome a device request has, before rate limiting.
#[derive(Debug, PartialEq)]
pub enum DeviceMatch<'a> {
    /// No enabled trigger for this device
    UnknownDevice,
    /// Triggers exist, but none has this secret
    WrongSecret,
    /// Authenticated: these triggers want the event (possibly none)
    Fire(Vec<&'a Trigger>),
}

pub fn match_triggers<'a>(
    triggers: impl IntoIterator<Item = &'a Trigger>,
    device_id: &str,
    secret: Option<&str>,
    event: &str,
) -> DeviceMatch<'a> {
    let for_device: Vec<&Trigger> = triggers
        .into_iter()
        .filter(|t| t.enabled && t.source_type == TriggerSourceType::Device)
        .filter(|t| device_config(&t.source_config).is_ok_and(|c| c.device_id == device_id))
        .collect();
    if for_device.is_empty() {
        return DeviceMatch::UnknownDevice;
    }
    let authorized: Vec<&Trigger> = for_device
        .into_iter()
        .filter(|t| matches!((t.secret(), secret), (Some(expected), Some(given)) if secrets_match(expected, given)))
        .collect();
    if authorized.is_empty() {
        return DeviceMatch::WrongSecret;
    }
    DeviceMatch::Fire(
        authorized
            .into_iter()
            .filter(|t| {
                device_config(&t.source_config)
                    .is_ok_and(|c| c.events.is_empty() || c.events.iter().any(|e| e == event))
            })
            .collect(),
    )
}

pub fn payload(device_id: &str, event: &DeviceEvent) -> Value {
    json!({ "device_id": device_id, "event": event.event, "data": event.data })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::triggers::schema::TriggerTargetType;

    fn device(id: &str, device_id: &str, events: &[&str], secret: &str) -> Trigger {
        Trigger {
            id: id.into(),
            name: id.into(),
            enabled: true,
            source_type: TriggerSourceType::Device,
            source_config: json!({"deviceId": device_id, "events": events, "secret": secret}),
            target_type: TriggerTargetType::Agent,
            target_id: "ops".into(),
            input_template: None,
            priority: 50,
            rate_limit_seconds: None,
            created_at: 0.0,
            updated_at: 0.0,
            last_fired_at: None,
            has_secret: true,
        }
    }

    #[test]
    fn fires_only_matching_events_with_the_right_secret() {
        let triggers = [
            device("jam", "printer-01", &["paper_jam"], "s1"),
            device("any", "printer-01", &[], "s1"),
            device("other-secret", "printer-01", &["paper_jam"], "s2"),
            device("other-device", "printer-02", &["paper_jam"], "s1"),
        ];
        let ids = |m: DeviceMatch| match m {
            DeviceMatch::Fire(ts) => ts.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            ids(match_triggers(
                &triggers,
                "printer-01",
                Some("s1"),
                "paper_jam"
            )),
            ["jam", "any"]
        );
        assert_eq!(
            ids(match_triggers(
                &triggers,
                "printer-01",
                Some("s1"),
                "ink_low"
            )),
            ["any"]
        );
        assert_eq!(
            match_triggers(&triggers, "printer-01", Some("nope"), "paper_jam"),
            DeviceMatch::WrongSecret
        );
        assert_eq!(
            match_triggers(&triggers, "printer-01", None, "paper_jam"),
            DeviceMatch::WrongSecret
        );
        assert_eq!(
            match_triggers(&triggers, "scanner", Some("s1"), "paper_jam"),
            DeviceMatch::UnknownDevice
        );
    }
}
