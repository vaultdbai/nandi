//! An agent's delegation settings, kept in its skill.md (the gold copy):
//!
//! ```yaml
//! accepts_delegation: true          # inbound (CapabilitySet)
//! max_delegation_concurrent: 2
//! delegation_outbound: true         # may call `delegate_to_agent`
//! delegation_overrides:             # per-capability inbound exceptions
//!   summarize-paper: allow
//!   translate: deny
//! ```
//!
//! `delegation_outbound` and `delegation_overrides` are frontmatter keys
//! the domain keeps as unknown keys (written back unchanged), so this
//! module reads and writes them through the skill.md text. Listing
//! `delegate_to_agent` in `allowed-tools` also allows outbound calls.
//!
//! Overrides, per capability: `allow` takes the capability's tasks even
//! when `accepts_delegation` is off (an explicit `deny` trust rule still
//! wins); `deny` rejects them; `ask` defers to the agent's setting.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde_yaml::{Mapping, Value as Yaml};

use super::{AgentDelegationSettings, DelegationOverride};
use crate::services::domain::agent::split_frontmatter;
use crate::services::domain::capability::DELEGATION_TOOL;
use crate::services::domain::Agent;
use crate::services::trust::TrustPolicy;

pub const OUTBOUND_KEY: &str = "delegation_outbound";
pub const OVERRIDES_KEY: &str = "delegation_overrides";
const ALLOWED_TOOLS_KEY: &str = "allowed-tools";

/// The skill.md frontmatter as YAML (empty if it has none).
fn frontmatter(agent: &Agent) -> Mapping {
    let md = agent.to_skill_md();
    split_frontmatter(&md)
        .ok()
        .and_then(|(yaml, _)| yaml)
        .and_then(|yaml| serde_yaml::from_str::<Mapping>(yaml).ok())
        .unwrap_or_default()
}

/// Whether the agent may call `delegate_to_agent`.
pub fn outbound_allowed(agent: &Agent) -> bool {
    let listed = agent
        .capabilities()
        .tools()
        .allowed()
        .is_some_and(|tools| tools.iter().any(|t| t == DELEGATION_TOOL));
    listed || outbound_key(&frontmatter(agent))
}

fn outbound_key(fm: &Mapping) -> bool {
    fm.get(OUTBOUND_KEY)
        .and_then(Yaml::as_bool)
        .unwrap_or(false)
}

/// Per-capability inbound exceptions, by capability name.
pub fn overrides(agent: &Agent) -> BTreeMap<String, TrustPolicy> {
    overrides_of(&frontmatter(agent))
}

fn overrides_of(fm: &Mapping) -> BTreeMap<String, TrustPolicy> {
    let Some(Yaml::Mapping(map)) = fm.get(OVERRIDES_KEY) else {
        return BTreeMap::new();
    };
    map.iter()
        // Deliberate: skill.md is a hand-edited file. An override that isn't
        // allow, deny or ask is left out, so the capability keeps the
        // agent's own policy
        .filter_map(|(capability, policy)| {
            Some((
                capability.as_str()?.to_string(),
                policy.as_str()?.parse().ok()?,
            ))
        })
        .collect()
}

pub fn read(agent: &Agent) -> AgentDelegationSettings {
    let fm = frontmatter(agent);
    let capabilities = agent.capabilities();
    AgentDelegationSettings {
        allow_outbound: outbound_allowed(agent),
        accept_inbound: capabilities.accepts_delegation(),
        max_concurrent: capabilities.max_delegation_concurrent(),
        overrides: overrides_of(&fm)
            .into_iter()
            .map(|(capability, policy)| DelegationOverride { capability, policy })
            .collect(),
    }
}

/// The agent with `settings` written into its skill.md. The domain
/// re-validates the result (e.g. `max_concurrent` of 0 is rejected).
pub fn apply(agent: &Agent, settings: &AgentDelegationSettings) -> Result<Agent, String> {
    let md = agent.to_skill_md();
    let (yaml, body) = split_frontmatter(&md)?;
    let mut fm: Mapping = match yaml {
        Some(yaml) if !yaml.trim().is_empty() => {
            serde_yaml::from_str(yaml).map_err(|e| format!("Invalid skill.md: {e}"))?
        }
        _ => Mapping::new(),
    };
    let key = |k: &str| Yaml::String(k.to_string());

    if settings.accept_inbound {
        fm.insert(key("accepts_delegation"), Yaml::Bool(true));
    } else {
        fm.remove("accepts_delegation");
    }
    match settings.max_concurrent {
        Some(max) => {
            fm.insert(
                key("max_delegation_concurrent"),
                Yaml::Number(u64::from(max).into()),
            );
        }
        None => {
            fm.remove("max_delegation_concurrent");
        }
    }
    if settings.allow_outbound {
        fm.insert(key(OUTBOUND_KEY), Yaml::Bool(true));
    } else {
        fm.remove(OUTBOUND_KEY);
    }
    // An explicit tool list must agree with the toggle
    if let Some(Yaml::Sequence(tools)) = fm.get_mut(ALLOWED_TOOLS_KEY) {
        tools.retain(|t| t.as_str() != Some(DELEGATION_TOOL));
        if settings.allow_outbound {
            tools.push(key(DELEGATION_TOOL));
        }
    }

    let mut overrides = Mapping::new();
    for o in &settings.overrides {
        let capability = o.capability.trim();
        if capability.is_empty() || capability.chars().any(char::is_whitespace) {
            return Err(format!("'{capability}' isn't a valid capability name"));
        }
        overrides.insert(key(capability), key(o.policy.as_str()));
    }
    if overrides.is_empty() {
        fm.remove(OVERRIDES_KEY);
    } else {
        fm.insert(key(OVERRIDES_KEY), Yaml::Mapping(overrides));
    }

    let yaml = serde_yaml::to_string(&fm).map_err(|e| e.to_string())?;
    let body = body.trim();
    let md = if body.is_empty() {
        format!("---\n{yaml}---\n")
    } else {
        format!("---\n{yaml}---\n\n{body}\n")
    };
    let instructions = agent.instructions();
    let updated = Agent::from_files(
        agent.id().clone(),
        &md,
        (!instructions.is_empty()).then_some(instructions),
    )
    .map_err(|e| e.to_string())?;
    Ok(updated.with_timestamps(agent.created_at(), SystemTime::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::AgentId;

    fn agent(frontmatter: &str) -> Agent {
        Agent::from_skill_md(
            AgentId::parse("papers").unwrap(),
            &format!("---\nname: Papers\n{frontmatter}---\nFind papers.\n"),
        )
        .unwrap()
    }

    #[test]
    fn defaults_are_off() {
        let settings = read(&agent(""));
        assert_eq!(settings, AgentDelegationSettings::default());
    }

    #[test]
    fn settings_round_trip_through_skill_md() {
        let original = agent("serves: [{name: find-papers}]\ncustom: kept\n");
        let settings = AgentDelegationSettings {
            allow_outbound: true,
            accept_inbound: true,
            max_concurrent: Some(3),
            overrides: vec![DelegationOverride {
                capability: "find-papers".into(),
                policy: TrustPolicy::Allow,
            }],
        };
        let updated = apply(&original, &settings).unwrap();
        assert_eq!(read(&updated), settings);
        assert!(updated.capabilities().accepts_delegation());
        assert_eq!(updated.capabilities().delegation_limit(), 3);
        assert_eq!(updated.skill_body(), "Find papers.");
        let md = updated.to_skill_md();
        assert!(md.contains("custom: kept"), "{md}");
        assert!(md.contains("delegation_outbound: true"), "{md}");

        let off = apply(&updated, &AgentDelegationSettings::default()).unwrap();
        assert_eq!(read(&off), AgentDelegationSettings::default());
        assert!(!off.to_skill_md().contains("delegation_"));
        assert_eq!(off.capabilities().serves().len(), 1, "serves is kept");
    }

    #[test]
    fn an_explicit_tool_list_follows_the_outbound_toggle() {
        let listed = agent("allowed-tools: [shell, delegate_to_agent]\n");
        assert!(outbound_allowed(&listed));
        let off = apply(&listed, &AgentDelegationSettings::default()).unwrap();
        assert!(!outbound_allowed(&off));
        assert_eq!(off.capabilities().tools().allowed().unwrap(), ["shell"]);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let base = agent("");
        let zero = AgentDelegationSettings {
            max_concurrent: Some(0),
            ..Default::default()
        };
        assert!(apply(&base, &zero).is_err());
        let bad_name = AgentDelegationSettings {
            overrides: vec![DelegationOverride {
                capability: "two words".into(),
                policy: TrustPolicy::Deny,
            }],
            ..Default::default()
        };
        assert!(apply(&base, &bad_name).is_err());
    }
}
