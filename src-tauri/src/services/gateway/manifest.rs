//! NANDI.md (wave final, Track D): YAML frontmatter + Markdown body, parsed
//! into `GatewayManifest` and written back through the repo's atomic write
//! path. Reloaded (and re-registered) when the file watcher sees it change.
//!
//! The file uses snake_case keys (`display_name`, `user_slug`, …), unlike
//! the camelCase specta types, so it goes through its own serde shape.
//! Writes merge into the frontmatter already on disk: keys this version
//! doesn't know (identity fields from another track, a newer version's,
//! the user's own) are kept in place, and the body is written back exactly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value as Yaml};

use super::{
    Availability, AvailabilityStatus, CapabilityPreference, FederationConfig, GatewayManifest,
    GatewayPolicy, RelayConfig, RouteStrategy, RoutingConfig,
};
use crate::services::domain::agent_card::OwnerRef;
use crate::services::repos::fs_atomic;

/// `~/.nandi/repo/NANDI.md`, relative to the repo root
pub const MANIFEST_FILE: &str = "NANDI.md";

/// Written on first launch when NANDI.md is missing. `{{owner}}` is the
/// display name, empty until onboarding sets it.
pub const DEFAULT_TEMPLATE: &str = r#"---
name: "{{owner}}'s Nandi"
display_name: "{{owner}}"
description: "My Nandi agents."
owner:
  display_name: "{{owner}}"
# What you're responsible for, in a few words each (e.g. [Risk, SOX audit,
# billing system]). Published on your gateway card so people can find you.
responsibilities: []
# Whether you're around, shown on your profile. For example:
# availability:
#   status: away          # available, busy or away
#   until: 2026-11-10     # a date, or a time like 2026-11-10T09:00:00+01:00
#   delegate_to: bob-laptop-1a2b3c4d   # the instance ID of who covers
relay:
  enabled: false
  discoverable: false
policy:
  max_concurrent: 3
  rate_limit_per_hour: 30
  # Work someone assigns you through your gateway waits for your approval;
  # questions (ask-me) don't
  require_approval: [assign-me]
  expose: []
  hide: []
federation:
  enabled: false
  trusted_peers: []
routing:
  preferences: {}
---

<!-- Your Nandi's public profile: people on your relay read what follows,
and your gateway uses it to answer and route their requests. Say who you
are and what your Nandi can help with. (Comments like this one stay private.) -->

I'm {{owner}}.

## What I can help with

(describe the capabilities you want to expose; list the areas you own as
`responsibilities:` above, so other people's agents can find you by topic)

## What I can't help with

(describe what should be declined or routed elsewhere)
"#;

/// `<repo root>/NANDI.md`
pub fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_FILE)
}

/// NANDI.md split in two: the frontmatter as YAML (every key, known or not)
/// and the body exactly as written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ManifestDocument {
    pub frontmatter: Mapping,
    pub body: String,
}

/// Splits `text` at its `---` fences. Without an opening fence the whole
/// text is the body.
pub fn split(text: &str) -> Result<ManifestDocument, String> {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok(ManifestDocument {
            frontmatter: Mapping::new(),
            body: text.to_string(),
        });
    };
    // The closing fence: a line that is exactly `---`
    let mut offset = 0;
    let mut close = None;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            close = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let (yaml_end, body_start) = match close {
        Some(found) => found,
        None => return Err("NANDI.md: the frontmatter has no closing `---`".to_string()),
    };
    let yaml = &rest[..yaml_end];
    let frontmatter = if yaml.trim().is_empty() {
        Mapping::new()
    } else {
        match serde_yaml::from_str::<Yaml>(yaml)
            .map_err(|e| format!("NANDI.md: invalid frontmatter: {e}"))?
        {
            Yaml::Mapping(map) => map,
            Yaml::Null => Mapping::new(),
            _ => return Err("NANDI.md: the frontmatter must be a mapping".to_string()),
        }
    };
    Ok(ManifestDocument {
        frontmatter,
        body: rest[body_start..].to_string(),
    })
}

/// Parses NANDI.md. Unknown frontmatter keys are ignored here (and kept by
/// [`serialize`]). An `availability:` that doesn't hold up (an unknown
/// status, an unreadable `until`, a `delegate_to` that isn't an instance
/// ID) is an error, like any other invalid value.
pub fn parse(text: &str) -> Result<GatewayManifest, String> {
    let document = split(text)?;
    let mut frontmatter: Frontmatter = serde_yaml::from_value(Yaml::Mapping(document.frontmatter))
        .map_err(|e| format!("NANDI.md: {e}"))?;
    let availability = frontmatter
        .availability
        .take()
        .map(AvailabilityYaml::into_availability)
        .transpose()?
        .unwrap_or_default();
    Ok(GatewayManifest {
        availability,
        ..frontmatter.into_manifest(document.body)
    })
}

/// Writes `manifest` as NANDI.md. With `previous` (the file as it is now),
/// frontmatter keys this version doesn't know are kept, in place.
pub fn serialize(manifest: &GatewayManifest, previous: Option<&str>) -> Result<String, String> {
    let mut frontmatter = previous
        .and_then(|text| split(text).ok())
        .map(|document| document.frontmatter)
        .unwrap_or_default();
    let known = match serde_yaml::to_value(Frontmatter::from_manifest(manifest))
        .map_err(|e| format!("Failed to write NANDI.md: {e}"))?
    {
        Yaml::Mapping(map) => map,
        _ => Mapping::new(),
    };
    merge(&mut frontmatter, known);
    // Available again: the old `availability:` goes (the merge keeps keys
    // it isn't given)
    if AvailabilityYaml::from_availability(&manifest.availability).is_none() {
        frontmatter.remove("availability");
    }
    // An empty handle is written as no handle
    if manifest.owner.github_handle.is_empty() {
        if let Some(Yaml::Mapping(owner)) = frontmatter.get_mut("owner") {
            owner.remove("github_handle");
        }
    }
    let yaml = serde_yaml::to_string(&Yaml::Mapping(frontmatter))
        .map_err(|e| format!("Failed to write NANDI.md: {e}"))?;
    Ok(format!("---\n{yaml}---\n{}", manifest.body))
}

/// Overlays `new` onto `base`: nested mappings merge, everything else
/// (scalars, lists) is replaced; keys only in `base` stay where they are.
fn merge(base: &mut Mapping, new: Mapping) {
    for (key, value) in new {
        match (base.get_mut(&key), value) {
            (Some(Yaml::Mapping(existing)), Yaml::Mapping(value)) => merge(existing, value),
            (Some(existing), value) => *existing = value,
            (None, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// NANDI.md under `root`, if it exists
pub fn load(root: &Path) -> Result<Option<GatewayManifest>, String> {
    let path = manifest_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to read {}: {e}", path.display())),
    }
}

/// Writes NANDI.md atomically (temp file + rename), keeping unknown
/// frontmatter keys. Returns the manifest as written.
pub fn save(root: &Path, manifest: &GatewayManifest) -> Result<GatewayManifest, String> {
    let path = manifest_path(root);
    let previous = std::fs::read_to_string(&path).ok();
    let text = serialize(manifest, previous.as_deref())?;
    fs_atomic::write_if_changed(&path, text.as_bytes())
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    parse(&text)
}

/// [`DEFAULT_TEMPLATE`] for `owner` (a display name, possibly empty)
pub fn render_default(owner: &str) -> String {
    let owner = owner.trim();
    // The frontmatter quotes it: escape what would end the string
    let quoted = owner.replace('\\', "\\\\").replace('"', "\\\"");
    let template = if owner.is_empty() {
        DEFAULT_TEMPLATE.replace("\"{{owner}}'s Nandi\"", "\"My Nandi\"")
    } else {
        DEFAULT_TEMPLATE.to_string()
    };
    let (frontmatter, body) = template
        .split_once("\n---\n")
        .unwrap_or((template.as_str(), ""));
    format!(
        "{}\n---\n{}",
        frontmatter.replace("{{owner}}", &quoted),
        body.replace("{{owner}}", owner)
    )
}

/// The body as others see it (the card, the router's and chat's prompts):
/// without `<!-- … -->` comments, which are notes to the owner
pub fn public_body(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// The template's greeting with the owner's name: "I'm ." (written before
/// onboarding knew the name) or "I'm <old>." becomes "I'm <name>."; any
/// other body is the owner's own and stays as it is
pub fn with_greeting(body: &str, old: &str, name: &str) -> String {
    let greeting = |who: &str| {
        format!(
            "
I'm {who}.
"
        )
    };
    [greeting(""), greeting(old.trim())]
        .iter()
        .find(|line| body.contains(line.as_str()))
        .map_or_else(
            || body.to_string(),
            |line| body.replacen(line.as_str(), &greeting(name.trim()), 1),
        )
}

/// Creates NANDI.md from the template, without an owner yet, if it's
/// missing. Returns whether it was created.
pub fn ensure_default(root: &Path) -> Result<bool, String> {
    let path = manifest_path(root);
    if path.exists() {
        return Ok(false);
    }
    fs_atomic::write_atomic(&path, render_default("").as_bytes())
        .map_err(|e| format!("Failed to create {}: {e}", path.display()))?;
    Ok(true)
}

/// NANDI.md's modification time, for change polling
pub fn modified(root: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(manifest_path(root))
        .and_then(|meta| meta.modified())
        .ok()
}

/// Longest responsibility kept, in characters
pub const MAX_RESPONSIBILITY_CHARS: usize = 100;
/// Most responsibilities kept
pub const MAX_RESPONSIBILITIES: usize = 50;

/// `responsibilities:` as published: trimmed, empties dropped, each cut to
/// [`MAX_RESPONSIBILITY_CHARS`], duplicates (ignoring case) dropped after
/// the first, at most [`MAX_RESPONSIBILITIES`]. Silent, like the rest of
/// the frontmatter: a hand-edited file always loads.
pub fn normalize_responsibilities(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|item| {
            let item = item.trim();
            match item.char_indices().nth(MAX_RESPONSIBILITY_CHARS) {
                Some((end, _)) => item[..end].trim_end().to_string(),
                None => item.to_string(),
            }
        })
        .filter(|item| !item.is_empty() && seen.insert(item.to_lowercase()))
        .take(MAX_RESPONSIBILITIES)
        .collect()
}

// ---------------------------------------------------------------------------
// The file's shape: snake_case keys
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Frontmatter {
    name: String,
    display_name: String,
    description: String,
    owner: OwnerYaml,
    user_slug: String,
    responsibilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    availability: Option<AvailabilityYaml>,
    relay: RelayYaml,
    policy: PolicyYaml,
    federation: FederationYaml,
    routing: RoutingYaml,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct OwnerYaml {
    display_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    github_handle: String,
}

/// `availability:` as written: `status` (available, busy or away), `until`
/// (a date or an RFC 3339 time), `delegate_to` (an instance ID)
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AvailabilityYaml {
    status: StatusYaml,
    #[serde(skip_serializing_if = "Option::is_none")]
    until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delegate_to: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StatusYaml {
    #[default]
    Available,
    Busy,
    Away,
}

impl AvailabilityYaml {
    /// Checked: `until` reads as a time, `delegate_to` is an instance ID.
    /// Available drops both.
    fn into_availability(self) -> Result<Availability, String> {
        let status = match self.status {
            StatusYaml::Available => return Ok(Availability::available()),
            StatusYaml::Busy => AvailabilityStatus::Busy,
            StatusYaml::Away => AvailabilityStatus::Away,
        };
        let until = self
            .until
            .as_deref()
            .map(str::trim)
            .filter(|until| !until.is_empty())
            .map(parse_until)
            .transpose()?;
        let delegate_to = self
            .delegate_to
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        if let Some(id) = &delegate_to {
            let valid = id.len() <= 128 && !id.contains('/') && !id.contains(char::is_whitespace);
            if !valid {
                return Err(format!(
                    "NANDI.md: availability.delegate_to '{id}' isn't an instance ID (copy it from their profile)"
                ));
            }
        }
        Ok(Availability {
            status,
            until,
            delegate_to,
        })
    }

    fn from_availability(availability: &Availability) -> Option<Self> {
        let status = match availability.status {
            AvailabilityStatus::Available => return None,
            AvailabilityStatus::Busy => StatusYaml::Busy,
            AvailabilityStatus::Away => StatusYaml::Away,
        };
        Some(Self {
            status,
            until: availability.until.map(format_until),
            delegate_to: availability.delegate_to.clone(),
        })
    }
}

/// `until` as epoch ms: a date is the start of that day here, a time is
/// RFC 3339
fn parse_until(text: &str) -> Result<f64, String> {
    use chrono::{Local, NaiveDate, TimeZone};
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let midnight = date.and_hms_opt(0, 0, 0).unwrap_or_default();
        return Local
            .from_local_datetime(&midnight)
            .earliest()
            .map(|at| at.timestamp_millis() as f64)
            .ok_or_else(|| format!("NANDI.md: availability.until {text} has no midnight here"));
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|at| at.timestamp_millis() as f64)
        .map_err(|_| {
            format!(
                "NANDI.md: availability.until '{text}' isn't a date (2026-11-10) or a time (2026-11-10T09:00:00+01:00)"
            )
        })
}

/// Epoch ms back as written: a date when it's midnight here, else a time
fn format_until(ms: f64) -> String {
    use chrono::{Local, TimeZone, Timelike};
    match Local.timestamp_millis_opt(ms as i64).single() {
        Some(at) if at.time().num_seconds_from_midnight() == 0 && at.nanosecond() == 0 => {
            at.format("%Y-%m-%d").to_string()
        }
        Some(at) => at.to_rfc3339(),
        None => String::new(),
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct RelayYaml {
    enabled: bool,
    discoverable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct PolicyYaml {
    max_concurrent: u32,
    rate_limit_per_hour: u32,
    require_approval: Vec<String>,
    expose: Vec<String>,
    hide: Vec<String>,
}

impl Default for PolicyYaml {
    fn default() -> Self {
        let policy = GatewayPolicy::default();
        Self {
            max_concurrent: policy.max_concurrent,
            rate_limit_per_hour: policy.rate_limit_per_hour,
            require_approval: policy.require_approval,
            expose: policy.expose,
            hide: policy.hide,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FederationYaml {
    enabled: bool,
    trusted_peers: Vec<OwnerYaml>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct RoutingYaml {
    /// Sorted, so writes are stable
    preferences: BTreeMap<String, PreferenceYaml>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct PreferenceYaml {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    preferred: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    excluded: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strategy: Option<StrategyYaml>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StrategyYaml {
    Fastest,
    Cheapest,
    #[serde(alias = "mostTrusted")]
    MostTrusted,
    #[serde(alias = "highestSuccessRate")]
    HighestSuccessRate,
}

impl From<StrategyYaml> for RouteStrategy {
    fn from(value: StrategyYaml) -> Self {
        match value {
            StrategyYaml::Fastest => Self::Fastest,
            StrategyYaml::Cheapest => Self::Cheapest,
            StrategyYaml::MostTrusted => Self::MostTrusted,
            StrategyYaml::HighestSuccessRate => Self::HighestSuccessRate,
        }
    }
}

impl From<RouteStrategy> for StrategyYaml {
    fn from(value: RouteStrategy) -> Self {
        match value {
            RouteStrategy::Fastest => Self::Fastest,
            RouteStrategy::Cheapest => Self::Cheapest,
            RouteStrategy::MostTrusted => Self::MostTrusted,
            RouteStrategy::HighestSuccessRate => Self::HighestSuccessRate,
        }
    }
}

impl OwnerYaml {
    fn from_owner(owner: &OwnerRef) -> Self {
        Self {
            display_name: owner.display_name.clone(),
            github_handle: owner.github_handle.clone(),
        }
    }

    fn into_owner(self) -> OwnerRef {
        OwnerRef {
            github_handle: self.github_handle,
            display_name: self.display_name,
        }
    }
}

impl Frontmatter {
    fn into_manifest(self, body: String) -> GatewayManifest {
        GatewayManifest {
            name: self.name,
            display_name: self.display_name,
            description: self.description,
            owner: self.owner.into_owner(),
            user_slug: self.user_slug,
            responsibilities: normalize_responsibilities(self.responsibilities),
            // Checked separately (`parse`): it can fail
            availability: Availability::available(),
            relay: RelayConfig {
                enabled: self.relay.enabled,
                discoverable: self.relay.discoverable,
            },
            policy: GatewayPolicy {
                max_concurrent: self.policy.max_concurrent,
                rate_limit_per_hour: self.policy.rate_limit_per_hour,
                require_approval: self.policy.require_approval,
                expose: self.policy.expose,
                hide: self.policy.hide,
            },
            federation: FederationConfig {
                enabled: self.federation.enabled,
                trusted_peers: self
                    .federation
                    .trusted_peers
                    .into_iter()
                    .map(OwnerYaml::into_owner)
                    .collect(),
            },
            routing: RoutingConfig {
                preferences: self
                    .routing
                    .preferences
                    .into_iter()
                    .map(|(capability, preference)| {
                        (
                            capability,
                            CapabilityPreference {
                                preferred: preference.preferred,
                                excluded: preference.excluded,
                                strategy: preference.strategy.map(Into::into),
                            },
                        )
                    })
                    .collect(),
            },
            body,
        }
    }

    fn from_manifest(manifest: &GatewayManifest) -> Self {
        Self {
            name: manifest.name.clone(),
            display_name: manifest.display_name.clone(),
            description: manifest.description.clone(),
            owner: OwnerYaml::from_owner(&manifest.owner),
            user_slug: manifest.user_slug.clone(),
            responsibilities: normalize_responsibilities(manifest.responsibilities.clone()),
            availability: AvailabilityYaml::from_availability(&manifest.availability),
            relay: RelayYaml {
                enabled: manifest.relay.enabled,
                discoverable: manifest.relay.discoverable,
            },
            policy: PolicyYaml {
                max_concurrent: manifest.policy.max_concurrent,
                rate_limit_per_hour: manifest.policy.rate_limit_per_hour,
                require_approval: manifest.policy.require_approval.clone(),
                expose: manifest.policy.expose.clone(),
                hide: manifest.policy.hide.clone(),
            },
            federation: FederationYaml {
                enabled: manifest.federation.enabled,
                trusted_peers: manifest
                    .federation
                    .trusted_peers
                    .iter()
                    .map(OwnerYaml::from_owner)
                    .collect(),
            },
            routing: RoutingYaml {
                preferences: manifest
                    .routing
                    .preferences
                    .iter()
                    .map(|(capability, preference)| {
                        (
                            capability.clone(),
                            PreferenceYaml {
                                preferred: preference.preferred.clone(),
                                excluded: preference.excluded.clone(),
                                strategy: preference.strategy.map(Into::into),
                            },
                        )
                    })
                    .collect(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn being_available_again_drops_the_old_availability() {
        let away = "---
name: X
availability:
  status: away
  delegate_to: bob-1a2b
---
Body
";
        let mut manifest = parse(away).unwrap();
        manifest.availability = Availability::available();
        let written = serialize(&manifest, Some(away)).unwrap();
        assert!(!written.contains("availability"), "{written}");
        assert!(parse(&written).unwrap().availability.is_available());
    }

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("nandi-gw-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn availability_is_read_checked_and_written_back() {
        let text = "---\nname: X\navailability:\n  status: away\n  until: 2026-11-10\n  delegate_to: bob-1a2b\n---\nBody\n";
        let manifest = parse(text).unwrap();
        assert_eq!(manifest.availability.status, AvailabilityStatus::Away);
        assert_eq!(
            manifest.availability.delegate_to.as_deref(),
            Some("bob-1a2b")
        );
        let until = manifest.availability.until.unwrap();
        assert_eq!(format_until(until), "2026-11-10");
        let written = serialize(&manifest, Some(text)).unwrap();
        assert!(written.contains("until: 2026-11-10"), "{written}");
        assert_eq!(parse(&written).unwrap().availability, manifest.availability);

        // A time, and busy with nobody covering
        let busy =
            parse("---\navailability:\n  status: busy\n  until: 2026-11-10T09:30:00Z\n---\n")
                .unwrap()
                .availability;
        assert_eq!(busy.until, Some(1_794_303_000_000.0));
        assert_eq!(busy.delegate_to, None);

        // Available drops the rest; none at all is available
        let back =
            parse("---\navailability:\n  status: available\n  until: 2026-11-10\n---\n").unwrap();
        assert_eq!(back.availability, Availability::available());
        assert!(parse("---\nname: X\n---\n")
            .unwrap()
            .availability
            .is_available());
        let written = serialize(&back, None).unwrap();
        assert!(!written.contains("availability"), "{written}");

        // Mistakes are errors, with what to fix
        for (text, says) in [
            ("status: vacation", "vacation"),
            ("status: away\n  until: next week", "isn't a date"),
            (
                "status: away\n  delegate_to: Bob Smith",
                "isn't an instance ID",
            ),
            ("status: away\n  covered_by: bob", "covered_by"),
        ] {
            let error = parse(&format!("---\navailability:\n  {text}\n---\n")).unwrap_err();
            assert!(error.contains(says), "{text}: {error}");
        }
    }

    #[test]
    fn parses_the_default_template() {
        let manifest = parse(&render_default("Ada")).unwrap();
        assert_eq!(manifest.name, "Ada's Nandi");
        assert_eq!(manifest.display_name, "Ada");
        assert_eq!(manifest.owner.display_name, "Ada");
        assert_eq!(manifest.owner.github_handle, "");
        // Assigned work waits for the owner; questions don't
        assert_eq!(
            manifest.policy,
            GatewayPolicy {
                require_approval: vec!["assign-me".into()],
                ..GatewayPolicy::default()
            }
        );
        assert!(!manifest.relay.enabled);
        assert!(manifest.body.contains("\nI'm Ada.\n"));
        assert!(manifest.body.ends_with("routed elsewhere)\n"));
        // The owner's notes stay in the file, out of what others see
        let public = public_body(&manifest.body);
        assert!(public.starts_with("I'm Ada."), "{public}");
        assert!(!public.contains("<!--") && !public.contains("public profile"));
    }

    #[test]
    fn comments_are_left_out_of_the_public_body() {
        assert_eq!(public_body("a <!-- x --> b"), "a  b");
        assert_eq!(public_body("<!--\nmany\nlines\n-->\nHi\n"), "Hi");
        assert_eq!(public_body("Hi <!-- never closed"), "Hi");
        assert_eq!(public_body("  plain  "), "plain");
    }

    #[test]
    fn the_greeting_follows_the_name_until_the_owner_rewrites_it() {
        let fresh = render_default("");
        let named = with_greeting(&fresh, "", "Alice");
        assert!(named.contains("\nI'm Alice.\n") && !named.contains("I'm ."));
        let renamed = with_greeting(&named, "Alice", "Alicia");
        assert!(renamed.contains("\nI'm Alicia.\n"));
        let own = "\nI review Rust code.\n";
        assert_eq!(with_greeting(own, "Alice", "Bob"), own);
    }

    #[test]
    fn round_trips_and_keeps_the_body_exactly() {
        let body = "\n# Hi\r\n\nTrailing spaces   \n---\nnot a fence for us\n\n";
        let text = format!("---\nname: X\n---\n{body}");
        let mut manifest = parse(&text).unwrap();
        assert_eq!(manifest.body, body);
        manifest.policy.rate_limit_per_hour = 7;
        manifest.policy.expose = vec!["summarize".into()];
        manifest.routing.preferences.insert(
            "summarize".into(),
            CapabilityPreference {
                preferred: vec!["i1".into()],
                excluded: vec![],
                strategy: Some(RouteStrategy::MostTrusted),
            },
        );
        manifest.federation.trusted_peers = vec![OwnerRef {
            github_handle: "grace".into(),
            display_name: "Grace".into(),
        }];
        let written = serialize(&manifest, Some(&text)).unwrap();
        assert!(written.contains("rate_limit_per_hour: 7"));
        assert!(written.contains("strategy: most_trusted"));
        let again = parse(&written).unwrap();
        assert_eq!(again, manifest);
        // Stable: writing it again changes nothing
        assert_eq!(serialize(&again, Some(&written)).unwrap(), written);
    }

    #[test]
    fn responsibilities_round_trip_normalized() {
        let text = "---\nname: X\nresponsibilities: [Risk, ' SOX audit ', '', risk, billing system]\n---\nBody\n";
        let manifest = parse(text).unwrap();
        assert_eq!(
            manifest.responsibilities,
            ["Risk", "SOX audit", "billing system"]
        );
        let written = serialize(&manifest, Some(text)).unwrap();
        let document = split(&written).unwrap();
        assert_eq!(
            document.frontmatter["responsibilities"],
            serde_yaml::from_str::<Yaml>("[Risk, SOX audit, billing system]").unwrap()
        );
        assert_eq!(parse(&written).unwrap(), manifest);
        assert_eq!(serialize(&manifest, Some(&written)).unwrap(), written);
        // The template starts with none
        assert!(parse(&render_default("Ada"))
            .unwrap()
            .responsibilities
            .is_empty());
        // Missing reads as none
        assert!(parse("---\nname: X\n---\n")
            .unwrap()
            .responsibilities
            .is_empty());
    }

    #[test]
    fn responsibilities_are_capped() {
        let long = "é".repeat(MAX_RESPONSIBILITY_CHARS + 20);
        let mut items: Vec<String> = (0..80).map(|i| format!("area {i}")).collect();
        items.insert(0, long);
        items.insert(1, "  ".into());
        let kept = normalize_responsibilities(items);
        assert_eq!(kept.len(), MAX_RESPONSIBILITIES);
        assert_eq!(kept[0].chars().count(), MAX_RESPONSIBILITY_CHARS);
        assert_eq!(kept[1], "area 0");
        // Written capped too
        let manifest = GatewayManifest {
            responsibilities: vec!["A".into(), "a".into(), " b ".into()],
            ..GatewayManifest::default()
        };
        assert_eq!(
            parse(&serialize(&manifest, None).unwrap())
                .unwrap()
                .responsibilities,
            ["A", "b"]
        );
    }

    #[test]
    fn keeps_unknown_keys() {
        let text = "---\nname: X\nfuture_key: [1, 2]\nowner:\n  display_name: Ada\n  avatar: a.png\npolicy:\n  max_concurrent: 2\n  custom: true\n---\nBody\n";
        let mut manifest = parse(text).unwrap();
        assert_eq!(manifest.policy.max_concurrent, 2);
        manifest.display_name = "Ada L".into();
        manifest.user_slug = "ada".into();
        let written = serialize(&manifest, Some(text)).unwrap();
        let document = split(&written).unwrap();
        let fm = &document.frontmatter;
        assert_eq!(
            fm["future_key"],
            serde_yaml::from_str::<Yaml>("[1, 2]").unwrap()
        );
        assert_eq!(fm["owner"]["avatar"], Yaml::from("a.png"));
        assert_eq!(fm["policy"]["custom"], Yaml::from(true));
        assert_eq!(fm["display_name"], Yaml::from("Ada L"));
        assert_eq!(fm["user_slug"], Yaml::from("ada"));
        // Unknown keys stay where they were
        let keys: Vec<&str> = fm.keys().filter_map(|k| k.as_str()).collect();
        assert_eq!(&keys[..3], ["name", "future_key", "owner"]);
        assert_eq!(document.body, "Body\n");
    }

    #[test]
    fn handles_missing_or_broken_frontmatter() {
        let plain = parse("Just a body\n").unwrap();
        assert_eq!(plain.body, "Just a body\n");
        assert_eq!(plain.policy.max_concurrent, 3);
        assert!(parse("---\nname: X\nno closing fence\n").is_err());
        assert!(parse("---\n- a list\n---\n").is_err());
        let crlf = parse("---\r\nname: X\r\n---\r\nBody\r\n").unwrap();
        assert_eq!(crlf.name, "X");
        assert_eq!(crlf.body, "Body\r\n");
    }

    #[test]
    fn creates_the_default_once() {
        let root = temp_root();
        assert!(ensure_default(&root).unwrap());
        let fresh = load(&root).unwrap().unwrap();
        assert_eq!(fresh.name, "My Nandi");
        assert_eq!(fresh.display_name, "");
        assert_eq!(fresh.owner.display_name, "");
        // Exists now: left alone
        let mut named = fresh;
        named.display_name = "Ada".into();
        save(&root, &named).unwrap();
        assert!(!ensure_default(&root).unwrap());
        assert_eq!(load(&root).unwrap().unwrap().display_name, "Ada");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_template_names_its_owner() {
        let manifest = parse(&render_default("octo\"cat")).unwrap();
        assert_eq!(manifest.display_name, "octo\"cat");
        assert_eq!(manifest.name, "octo\"cat's Nandi");
        assert!(manifest.body.contains("I'm octo\"cat."));
    }

    #[test]
    fn saves_atomically_and_keeps_the_body() {
        let root = temp_root();
        ensure_default(&root).unwrap();
        let mut manifest = load(&root).unwrap().unwrap();
        let body = manifest.body.clone();
        manifest.relay.enabled = true;
        let saved = save(&root, &manifest).unwrap();
        assert!(saved.relay.enabled);
        assert_eq!(saved.body, body);
        assert_eq!(load(&root).unwrap().unwrap(), saved);
        let leftovers = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(fs_atomic::TEMP_MARKER)
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(root);
    }
}
