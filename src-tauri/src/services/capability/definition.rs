//! A capability definition: one file per capability at
//! `.nandi/capabilities/<id>.yaml` in the gold copy.
//!
//! ```yaml
//! ---
//! id: code-review.python
//! name: Python code review
//! description: Reviews a Python diff
//! version: 1.0.0
//! owner: { kind: user, id: alice, display_name: Alice, transferable: true }
//! policy: { visibility: network, sla_seconds: 600 }
//! required_tools: [shell]
//! tags: [code, python]
//! ---
//! Markdown body: what the capability does, for humans.
//! ```
//!
//! A plain YAML file (no `---` delimiters) parses too, with an empty body.
//! Pure: no I/O (`services::repos::capability_repo` reads and writes).
//!
//! Timestamps are epoch ms as `Timestamp` (f64), and large counters are
//! f64, because the bindings export no bigints.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::Timestamp;

// The wire shape lives in `nandi-protocol`
pub use nandi_protocol::ProviderKind;

/// Valid capability IDs: `^[a-z0-9][a-z0-9.-]*$`
pub const ID_PATTERN: &str = "^[a-z0-9][a-z0-9.-]*$";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDefinition {
    /// e.g. `code-review.python`
    pub id: String,
    pub name: String,
    pub description: String,
    /// semver
    pub version: String,
    pub owner: CapabilityOwner,
    #[serde(default)]
    pub policy: CapabilityPolicy,
    #[serde(default)]
    pub schemas: CapabilitySchemas,
    #[serde(default)]
    pub routing: RoutingPolicy,
    /// Tool IDs (`builtin.shell`, `gmail.send_email`; a bare `shell` means
    /// `builtin.shell`)
    #[serde(default)]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub optional_tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: Option<Timestamp>,
    pub updated_at: Option<Timestamp>,
    /// Markdown after the frontmatter
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityOwner {
    pub kind: OwnerKind,
    /// Handle, team slug, or org ID
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub transferable: bool,
    /// Transfers, oldest first
    #[serde(default)]
    pub history: Vec<OwnershipChange>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    #[default]
    User,
    Team,
    Org,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipChange {
    pub from: String,
    pub to: String,
    pub at: Timestamp,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CapabilityPolicy {
    pub visibility: Visibility,
    pub max_concurrent_per_provider: u32,
    pub total_max_concurrent: u32,
    pub sla_seconds: u32,
    pub approval_required: bool,
    pub cost: Option<CostSpec>,
    pub auto_reassign: ReassignPolicy,
}

impl Default for CapabilityPolicy {
    fn default() -> Self {
        Self {
            visibility: Visibility::Private,
            max_concurrent_per_provider: 1,
            total_max_concurrent: 0,
            sla_seconds: 0,
            approval_required: false,
            cost: None,
            auto_reassign: ReassignPolicy::Never,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Team,
    Network,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReassignPolicy {
    #[default]
    Never,
    SameCapability,
    AnyProvider,
}

/// What a call costs (capabilities and tools)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CostSpec {
    pub per_call_usd: Option<f64>,
    pub per_1k_tokens_usd: Option<f64>,
    pub note: Option<String>,
}

/// JSON Schemas
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CapabilitySchemas {
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct RoutingPolicy {
    pub strategy: CapabilityRouteStrategy,
    pub fallback: FallbackPolicy,
    /// Provider IDs (`<instance_id>/<provider_id>` or a bare instance ID)
    pub preferred_providers: Vec<String>,
    pub excluded_providers: Vec<String>,
    /// Hard requirement, on top of `required_tools`
    pub require_tools: Vec<String>,
}

/// Not `gateway::RouteStrategy`, which the gateway keeps
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRouteStrategy {
    #[default]
    LeastLoaded,
    RoundRobin,
    MostTrusted,
    Fastest,
    HighestSuccessRate,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    #[default]
    None,
    AnyOnline,
    EscalateToOwner,
}

/// Keys whose values are copied verbatim (JSON Schemas name their own
/// properties)
const VERBATIM_KEYS: &[&str] = &["input", "output"];

/// Frontmatter + body (or plain YAML) into a definition. Doesn't validate.
/// File keys are snake_case (`required_tools`, `display_name`); camelCase
/// is accepted too. Unknown keys are ignored.
pub fn parse(text: &str) -> Result<CapabilityDefinition, String> {
    let text = text.trim_start_matches('\u{feff}');
    let (yaml, body) = split(text)?;
    let value: serde_json::Value = if yaml.trim().is_empty() {
        serde_json::Value::Object(Default::default())
    } else {
        serde_yaml::from_str(yaml).map_err(|e| format!("Invalid capability YAML: {e}"))?
    };
    if !value.is_object() {
        return Err("A capability file must be a YAML mapping".to_string());
    }
    let mut value = rename_keys(value, &snake_to_camel, false);
    if let Some(map) = value.as_object_mut() {
        map.remove("body");
        // A file may leave its ID to its name (`capability_repo`), and the
        // description is optional
        for key in ["id", "description"] {
            map.entry(key)
                .or_insert_with(|| serde_json::Value::String(String::new()));
        }
    }
    let mut definition: CapabilityDefinition =
        serde_json::from_value(value).map_err(|e| format!("Invalid capability: {e}"))?;
    definition.body = body.trim().to_string();
    Ok(definition)
}

/// Back to frontmatter + body. Round-trips with `parse`.
pub fn serialize(definition: &CapabilityDefinition) -> Result<String, String> {
    let mut value = serde_json::to_value(definition)
        .map_err(|e| format!("Failed to encode the capability: {e}"))?;
    if let Some(map) = value.as_object_mut() {
        map.remove("body");
        // Omitted keys read back as their defaults
        map.retain(|_, v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()));
    }
    let value = rename_keys(value, &camel_to_snake, true);
    let yaml = serde_yaml::to_string(&value)
        .map_err(|e| format!("Failed to encode the capability: {e}"))?;
    let body = definition.body.trim();
    Ok(if body.is_empty() {
        format!("---\n{yaml}---\n")
    } else {
        format!("---\n{yaml}---\n\n{body}\n")
    })
}

/// ID matches `ID_PATTERN`, version is semver, name and owner are set,
/// `required_tools` is non-empty when `visibility` is `network`.
pub fn validate(definition: &CapabilityDefinition) -> Result<(), String> {
    if !valid_id(&definition.id) {
        return Err(format!(
            "Capability ID '{}' must match {ID_PATTERN}",
            definition.id
        ));
    }
    if semver::Version::parse(definition.version.trim()).is_err() {
        return Err(format!(
            "Capability '{}': version '{}' isn't semver (e.g. 1.0.0)",
            definition.id, definition.version
        ));
    }
    if definition.name.trim().is_empty() {
        return Err(format!("Capability '{}' needs a name", definition.id));
    }
    if definition.owner.id.trim().is_empty() {
        return Err(format!("Capability '{}' needs an owner", definition.id));
    }
    if definition.policy.visibility == Visibility::Network
        && definition
            .required_tools
            .iter()
            .all(|tool| tool.trim().is_empty())
    {
        return Err(format!(
            "Capability '{}' is visible on the network, so it must list its required_tools",
            definition.id
        ));
    }
    Ok(())
}

/// `^[a-z0-9][a-z0-9.-]*$`
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

/// `(frontmatter, body)`; without an opening `---` the whole text is YAML
fn split(text: &str) -> Result<(&str, &str), String> {
    let mut lines = text.split_inclusive('\n');
    match lines.next() {
        Some(first) if first.trim_end() == "---" => {}
        _ => return Ok((text, "")),
    }
    let start = text.find('\n').map_or(text.len(), |i| i + 1);
    let mut offset = start;
    for line in lines {
        if line.trim_end() == "---" {
            return Ok((&text[start..offset], &text[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err("The capability frontmatter has no closing `---`".to_string())
}

/// Renames object keys at every level except inside JSON Schemas. On the
/// way out (`integral`), whole-number floats (timestamps) become integers.
fn rename_keys(
    value: serde_json::Value,
    rename: &dyn Fn(&str) -> String,
    integral: bool,
) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let renamed = rename(&key);
                    let value = if VERBATIM_KEYS.contains(&renamed.as_str()) {
                        value
                    } else {
                        rename_keys(value, rename, integral)
                    };
                    (renamed, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| rename_keys(item, rename, integral))
                .collect(),
        ),
        Value::Number(n) if integral => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => Value::from(f as i64),
            _ => Value::Number(n),
        },
        other => other,
    }
}

fn snake_to_camel(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for c in key.chars() {
        if c == '_' || c == '-' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn camel_to_snake(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REVIEW: &str = "---
id: code-review.python
name: Python code review
description: Reviews a Python diff
version: 1.2.0
owner:
  kind: user
  id: alice
  display_name: Alice
  transferable: true
policy:
  visibility: network
  max_concurrent_per_provider: 2
  sla_seconds: 600
  auto_reassign: same_capability
schemas:
  input:
    type: object
    properties:
      python_version: { type: string }
routing:
  strategy: highest_success_rate
  preferred_providers: [inst-1/reviewer]
required_tools: [shell, read_file]
optional_tools: [github.create_review]
tags: [code, python]
created_at: 1760000000000
---

Reviews **Python** diffs.
";

    const PLAIN: &str = "id: summarize
name: Summarize
description: Summarizes text
version: 0.1.0
owner: { kind: team, id: research, display_name: Research }
";

    const ORG: &str = "---
id: org.invoice-triage
name: Invoice triage
description: Sorts invoices
version: 2.0.0-beta.1
owner:
  kind: org
  id: acme
  display_name: Acme
  history:
    - { from: bob, to: acme, at: 1760000000000, reason: Reorg }
policy: { visibility: team, approval_required: true, cost: { per_call_usd: 0.5 } }
---
";

    #[test]
    fn parses_frontmatter_plain_yaml_and_history() {
        let review = parse(REVIEW).unwrap();
        assert_eq!(review.id, "code-review.python");
        assert_eq!(review.owner.display_name, "Alice");
        assert!(review.owner.transferable);
        assert_eq!(review.policy.visibility, Visibility::Network);
        assert_eq!(review.policy.max_concurrent_per_provider, 2);
        assert_eq!(review.policy.auto_reassign, ReassignPolicy::SameCapability);
        assert_eq!(
            review.routing.strategy,
            CapabilityRouteStrategy::HighestSuccessRate
        );
        assert_eq!(review.required_tools, ["shell", "read_file"]);
        assert_eq!(review.created_at, Some(1_760_000_000_000.0));
        assert_eq!(review.body, "Reviews **Python** diffs.");
        // Schema property names are not renamed
        assert_eq!(
            review.schemas.input.unwrap()["properties"]["python_version"],
            json!({"type": "string"})
        );

        let plain = parse(PLAIN).unwrap();
        assert_eq!(plain.owner.kind, OwnerKind::Team);
        assert_eq!(plain.body, "");
        assert_eq!(plain.policy, CapabilityPolicy::default());

        let org = parse(ORG).unwrap();
        assert_eq!(org.owner.history.len(), 1);
        assert_eq!(org.owner.history[0].reason.as_deref(), Some("Reorg"));
        assert!(org.policy.approval_required);
        assert_eq!(org.policy.cost.unwrap().per_call_usd, Some(0.5));
    }

    #[test]
    fn round_trips() {
        for text in [REVIEW, PLAIN, ORG] {
            let parsed = parse(text).unwrap();
            let written = serialize(&parsed).unwrap();
            assert_eq!(parse(&written).unwrap(), parsed, "{written}");
            assert!(written.starts_with("---\n"), "{written}");
            assert!(!written.contains("requiredTools"), "{written}");
        }
        let written = serialize(&parse(REVIEW).unwrap()).unwrap();
        assert!(written.contains("created_at: 1760000000000\n"), "{written}");
        assert!(written.contains("python_version"), "{written}");
    }

    #[test]
    fn validates_ids_versions_owner_and_network_tools() {
        let good = parse(REVIEW).unwrap();
        validate(&good).unwrap();
        validate(&parse(PLAIN).unwrap()).unwrap();
        validate(&parse(ORG).unwrap()).unwrap();

        for bad in ["", "Code-Review", "-x", ".x", "a b", "a/b", "a_b"] {
            let def = CapabilityDefinition {
                id: bad.into(),
                ..good.clone()
            };
            assert!(validate(&def).is_err(), "{bad:?}");
        }
        for bad in ["", "1", "1.0", "v1.0.0", "latest"] {
            let def = CapabilityDefinition {
                version: bad.into(),
                ..good.clone()
            };
            assert!(validate(&def).unwrap_err().contains("semver"), "{bad:?}");
        }
        let mut no_tools = good.clone();
        no_tools.required_tools.clear();
        assert!(validate(&no_tools).unwrap_err().contains("required_tools"));
        no_tools.policy.visibility = Visibility::Team;
        validate(&no_tools).unwrap();

        let mut no_owner = good;
        no_owner.owner.id = " ".into();
        assert!(validate(&no_owner).unwrap_err().contains("owner"));
    }

    #[test]
    fn rejects_malformed_files() {
        assert!(parse("---\nid: x\n").unwrap_err().contains("closing"));
        assert!(parse("- a\n- b\n").is_err());
        assert!(parse("id: [unclosed\n").is_err());
    }
}
