//! `input_template` rendering: plain `{{key}}` substitution.
//!
//! - `{{payload.a.b}}` / `{{payload.items.0}}`: a path into the event
//! - `{{trigger.name}}`, `{{trigger.id}}`
//! - `{{fired_at}}`: ISO 8601
//! - `{{last_run.output}}`: the trigger's latest completed run's reply
//!
//! Missing keys become empty strings (and are reported, so the caller can
//! log them). Strings are inserted as-is, anything else as compact JSON.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z0-9_.\-]+)\s*\}\}").unwrap());

pub struct TemplateContext<'a> {
    pub payload: &'a Value,
    pub trigger_id: &'a str,
    pub trigger_name: &'a str,
    pub fired_at: &'a str,
    pub last_output: Option<&'a str>,
}

/// Renders `template`; returns the text and the keys that had no value.
pub fn render(template: &str, ctx: &TemplateContext) -> (String, Vec<String>) {
    let mut missing = Vec::new();
    let text = PLACEHOLDER
        .replace_all(template, |caps: &regex::Captures| {
            let key = &caps[1];
            match lookup(key, ctx) {
                Some(value) => value,
                None => {
                    missing.push(key.to_string());
                    String::new()
                }
            }
        })
        .into_owned();
    (text, missing)
}

fn lookup(key: &str, ctx: &TemplateContext) -> Option<String> {
    match key {
        "trigger.name" => return Some(ctx.trigger_name.to_string()),
        "trigger.id" => return Some(ctx.trigger_id.to_string()),
        "fired_at" => return Some(ctx.fired_at.to_string()),
        "last_run.output" => return ctx.last_output.map(str::to_string),
        "payload" => return Some(display(ctx.payload)),
        _ => {}
    }
    let path = key.strip_prefix("payload.")?;
    let mut value = ctx.payload;
    for part in path.split('.') {
        value = match value {
            Value::Object(map) => map.get(part)?,
            Value::Array(items) => items.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    (!value.is_null()).then(|| display(value))
}

fn display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The input sent when a trigger has no template.
pub fn default_input(trigger_name: &str, payload: &Value) -> String {
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_default();
    format!("Trigger \"{trigger_name}\" fired.\n\nEvent:\n```json\n{pretty}\n```")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(payload: &'a Value, last: Option<&'a str>) -> TemplateContext<'a> {
        TemplateContext {
            payload,
            trigger_id: "contracts",
            trigger_name: "New contracts",
            fired_at: "2026-10-06T09:00:00Z",
            last_output: last,
        }
    }

    #[test]
    fn substitutes_paths_and_builtins() {
        let payload = json!({
            "path": "/in/a.pdf",
            "size": 42,
            "meta": {"tags": ["x", "y"], "ok": true}
        });
        let (text, missing) = render(
            "{{trigger.name}}: {{ payload.path }} ({{payload.size}} B, {{payload.meta.tags.1}}, \
             {{payload.meta.ok}}) at {{fired_at}} after {{last_run.output}}",
            &ctx(&payload, Some("done")),
        );
        assert_eq!(
            text,
            "New contracts: /in/a.pdf (42 B, y, true) at 2026-10-06T09:00:00Z after done"
        );
        assert!(missing.is_empty());

        // Objects render with their keys in payload order
        let (text, _) = render("{{payload.meta}}", &ctx(&payload, None));
        assert_eq!(text, r#"{"tags":["x","y"],"ok":true}"#);
    }

    #[test]
    fn missing_keys_render_empty_and_are_reported() {
        let payload = json!({"a": null, "list": [1]});
        let (text, missing) = render(
            "[{{payload.nope}}][{{payload.a}}][{{payload.list.5}}][{{last_run.output}}][{{other}}] {{ not a key }}",
            &ctx(&payload, None),
        );
        assert_eq!(text, "[][][][][] {{ not a key }}");
        assert_eq!(
            missing,
            [
                "payload.nope",
                "payload.a",
                "payload.list.5",
                "last_run.output",
                "other"
            ]
        );
    }

    #[test]
    fn default_input_includes_the_event() {
        let text = default_input("Daily", &json!({"x": 1}));
        assert!(text.starts_with("Trigger \"Daily\" fired."));
        assert!(text.contains("\"x\": 1"));
    }
}
