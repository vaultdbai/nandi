//! Portable settings: `.nandi/settings.yaml`. Safe to commit: secrets are
//! referenced by keyring entry, never stored.
//!
//! ```yaml
//! default_model:
//!   provider: deepseek
//!   name: deepseek-flash
//! theme: dark
//! language: en
//! notifications:
//!   enabled: true
//! agent_defaults:
//!   requires_approval: [shell, write_file]
//! secret_refs:
//!   deepseek_api_key: nandi.llm.deepseek
//! marketplace:
//!   registries:                       # omitted: the default public registry
//!     - url: github.com/vaultdbai/nandi-registry
//!       kind: public
//!     - url: github.com/acme/nandi-community-acme
//!       kind: community
//!       auth: nandi.marketplace.acme   # keyring entry with a GitHub token
//! consent:                            # what the user approved per marketplace agent
//!   arxiv-researcher:
//!     tools: [shell, read_file]
//!     network: [export.arxiv.org]
//! delegation:
//!   preferred_providers:              # my pins: capability → provider card ID
//!     find-papers: inst-7f3a/papers
//! connector_grants:                   # saved grants (`grant.rs`); local ones are in settings.local.yaml
//!   - { agent: chief-of-staff, connector: outlook, access: read }
//! ```
//!
//! Machine-specific preferences (the quick pane shortcut) stay in the local
//! `preferences.json`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::agent::{model_ref_mapping, ModelRef, Provider};
use super::grant::Grants;
use super::{DomainError, DomainResult};

/// The model agents use when skill.md doesn't name one.
pub const DEFAULT_MODEL: &str = "claude:claude-sonnet-5";

/// A keyring entry name, e.g. `nandi.llm.deepseek`. Never a secret itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretRef(String);

impl SecretRef {
    pub fn parse(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let valid = value.starts_with("nandi.")
            && value.len() <= 128
            && value.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            });
        if !valid {
            return Err(DomainError::InvalidId {
                kind: "secret ref",
                value,
                reason:
                    "must start with 'nandi.' and use only lowercase letters, digits, '.', '_', '-'",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SecretRef {
    type Error = DomainError;
    fn try_from(value: String) -> DomainResult<Self> {
        Self::parse(value)
    }
}

impl From<SecretRef> for String {
    fn from(value: SecretRef) -> Self {
        value.0
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            other => Err(DomainError::invalid(
                "settings",
                "theme",
                format!("'{other}' is not system, light, or dark"),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationPrefs {
    pub enabled: bool,
    /// Notify when an agent needs approval
    pub approvals: bool,
    /// Notify when a run fails
    pub failures: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            approvals: true,
            failures: true,
        }
    }
}

/// The public registry every install starts with.
pub const DEFAULT_REGISTRY: &str = "github.com/vaultdbai/nandi-registry";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryKind {
    /// The curated public registry
    Public,
    /// Shared within a GitHub org or group of collaborators
    Community,
    /// The user's own
    Personal,
}

impl RegistryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Community => "community",
            Self::Personal => "personal",
        }
    }

    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "public" => Ok(Self::Public),
            "community" => Ok(Self::Community),
            "personal" => Ok(Self::Personal),
            other => Err(DomainError::invalid(
                "settings",
                "marketplace.registries.kind",
                format!("'{other}' is not public, community, or personal"),
            )),
        }
    }
}

/// A marketplace registry: a git repo with `index.json` at its root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConfig {
    /// `github.com/<owner>/<repo>`, an `https://` base URL, or a local
    /// `file://` folder
    pub url: String,
    pub kind: RegistryKind,
    /// Keyring entry holding a GitHub token for a private registry
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<SecretRef>,
}

impl RegistryConfig {
    pub fn new(url: &str, kind: RegistryKind, auth: Option<SecretRef>) -> DomainResult<Self> {
        let config = Self {
            url: normalize_registry_url(url),
            kind,
            auth,
        };
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> DomainResult<()> {
        let invalid = |reason: &str| {
            Err(DomainError::invalid(
                "settings",
                "marketplace.registries.url",
                format!("'{}' {reason}", self.url),
            ))
        };
        if self.url.is_empty() || self.url.chars().any(char::is_whitespace) {
            return invalid("must be a URL without spaces");
        }
        let rest = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, r)| r);
        if rest.split('/').next().unwrap_or_default().contains('@') {
            return invalid("must not contain credentials");
        }
        if self.url.contains("://") {
            let scheme_ok = ["https://", "http://", "file://"]
                .iter()
                .any(|scheme| self.url.starts_with(scheme));
            if !scheme_ok {
                return invalid("must be github.com/<owner>/<repo>, https://, or file://");
            }
        } else {
            let parts: Vec<&str> = self.url.split('/').collect();
            let github = parts.len() == 3
                && parts[0] == "github.com"
                && !parts[1].is_empty()
                && !parts[2].is_empty();
            if !github {
                return invalid("must be github.com/<owner>/<repo>");
            }
        }
        Ok(())
    }
}

/// `https://github.com/o/r.git/` → `github.com/o/r`; other URLs lose a
/// trailing `/`.
pub fn normalize_registry_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let github = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    if let Some(path) = github.strip_prefix("github.com/") {
        let path = path.trim_end_matches(".git");
        return format!("github.com/{path}");
    }
    url.to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarketplaceSettings {
    /// `None`: just the default public registry
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registries: Option<Vec<RegistryConfig>>,
}

impl MarketplaceSettings {
    fn check(&self) -> DomainResult<()> {
        let Some(registries) = &self.registries else {
            return Ok(());
        };
        let mut seen = std::collections::HashSet::new();
        for registry in registries {
            registry.check()?;
            if !seen.insert(normalize_registry_url(&registry.url)) {
                return Err(DomainError::invalid(
                    "settings",
                    "marketplace.registries",
                    format!("'{}' is listed twice", registry.url),
                ));
            }
        }
        Ok(())
    }
}

/// What the user approved for a marketplace agent, by category. The items
/// are recorded (not just "yes"), so an update that asks for more needs
/// approval again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConsent {
    /// `*` means every runtime tool
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aws: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pilots: Vec<String>,
}

/// My delegation preferences. Personal: they follow me across my devices
/// but are never shared with a team.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DelegationPrefs {
    /// Capability → the card ID (`<instance>/<agent>`) to send it to first
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub preferred_providers: BTreeMap<String, String>,
}

impl DelegationPrefs {
    fn check(&self) -> DomainResult<()> {
        for (capability, card_id) in &self.preferred_providers {
            if capability.trim().is_empty() || capability.trim() != capability {
                return Err(DomainError::invalid(
                    "settings",
                    "delegation.preferred_providers",
                    format!("'{capability}' is not a capability name"),
                ));
            }
            let valid = card_id
                .split_once('/')
                .is_some_and(|(instance, agent)| !instance.is_empty() && !agent.is_empty());
            if !valid {
                return Err(DomainError::invalid(
                    "settings",
                    "delegation.preferred_providers",
                    format!("'{card_id}' is not a card ID (<instance>/<agent>)"),
                ));
            }
        }
        Ok(())
    }
}

/// Defaults new agents start with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    default_model: ModelRef,
    theme: Theme,
    /// Colour palette ID (e.g. "nandi", "aurora"); `None` is the default
    palette: Option<String>,
    language: Option<String>,
    notifications: NotificationPrefs,
    agent_defaults: AgentDefaults,
    /// Setting name → keyring entry. BTreeMap so the file's order is stable.
    secret_refs: BTreeMap<String, SecretRef>,
    marketplace: MarketplaceSettings,
    /// Agent ID → what the user approved for it
    consent: BTreeMap<String, AgentConsent>,
    delegation: DelegationPrefs,
    /// Saved connector grants: they travel with the repo
    connector_grants: Grants,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_model: ModelRef::new(Provider::Anthropic, "claude-sonnet-5")
                .expect("the default model is valid"),
            theme: Theme::default(),
            palette: None,
            language: None,
            notifications: NotificationPrefs::default(),
            agent_defaults: AgentDefaults::default(),
            secret_refs: BTreeMap::new(),
            marketplace: MarketplaceSettings::default(),
            consent: BTreeMap::new(),
            delegation: DelegationPrefs::default(),
            connector_grants: Grants::default(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    #[serde(default = "default_model", with = "model_ref_mapping")]
    default_model: ModelRef,
    #[serde(default)]
    theme: Theme,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    palette: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default)]
    notifications: NotificationPrefs,
    #[serde(default, skip_serializing_if = "is_default_agent_defaults")]
    agent_defaults: AgentDefaults,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    secret_refs: BTreeMap<String, SecretRef>,
    #[serde(default, skip_serializing_if = "is_default_marketplace")]
    marketplace: MarketplaceSettings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    consent: BTreeMap<String, AgentConsent>,
    #[serde(default, skip_serializing_if = "is_default_delegation")]
    delegation: DelegationPrefs,
    #[serde(default, skip_serializing_if = "Grants::is_empty")]
    connector_grants: Grants,
}

fn is_default_delegation(value: &DelegationPrefs) -> bool {
    *value == DelegationPrefs::default()
}

fn is_default_marketplace(value: &MarketplaceSettings) -> bool {
    *value == MarketplaceSettings::default()
}

fn default_model() -> ModelRef {
    Settings::default().default_model
}

fn is_default_agent_defaults(value: &AgentDefaults) -> bool {
    *value == AgentDefaults::default()
}

impl Settings {
    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        let file: SettingsFile =
            serde_yaml::from_str(content).map_err(|e| DomainError::parse("settings.yaml", e))?;
        let settings = Self {
            default_model: file.default_model,
            theme: file.theme,
            palette: file.palette,
            language: file.language,
            notifications: file.notifications,
            agent_defaults: file.agent_defaults,
            secret_refs: file.secret_refs,
            marketplace: file.marketplace,
            consent: file.consent,
            delegation: file.delegation,
            connector_grants: Grants::new(file.connector_grants.iter().cloned().collect())?,
        };
        settings.check()?;
        Ok(settings)
    }

    fn check(&self) -> DomainResult<()> {
        self.marketplace.check()?;
        self.delegation.check()?;
        if let Some(id) = self
            .consent
            .keys()
            .find(|id| super::AgentId::parse(id.as_str()).is_err())
        {
            return Err(DomainError::invalid(
                "settings",
                "consent",
                format!("'{id}' is not an agent ID"),
            ));
        }
        if let Some(palette) = &self.palette {
            let valid = (1..=32).contains(&palette.len())
                && palette
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !valid {
                return Err(DomainError::invalid(
                    "settings",
                    "palette",
                    format!("'{palette}' is not a palette ID"),
                ));
            }
        }
        if let Some(language) = &self.language {
            let valid = (2..=16).contains(&language.len())
                && language
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                return Err(DomainError::invalid(
                    "settings",
                    "language",
                    format!("'{language}' is not a language code"),
                ));
            }
        }
        if let Some(key) = self.secret_refs.keys().find(|k| {
            k.is_empty()
                || !k
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }) {
            return Err(DomainError::invalid(
                "settings",
                "secret_refs",
                format!("key '{key}' must use lowercase letters, digits, and '_'"),
            ));
        }
        Ok(())
    }

    pub fn to_yaml(&self) -> String {
        let file = SettingsFile {
            default_model: self.default_model.clone(),
            theme: self.theme,
            palette: self.palette.clone(),
            language: self.language.clone(),
            notifications: self.notifications.clone(),
            agent_defaults: self.agent_defaults.clone(),
            secret_refs: self.secret_refs.clone(),
            marketplace: self.marketplace.clone(),
            consent: self.consent.clone(),
            delegation: self.delegation.clone(),
            connector_grants: self.connector_grants.clone(),
        };
        serde_yaml::to_string(&file).unwrap_or_default()
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn set_palette(&mut self, palette: Option<String>) -> DomainResult<()> {
        let previous = std::mem::replace(&mut self.palette, palette);
        self.check().inspect_err(|_| self.palette = previous)
    }

    pub fn set_language(&mut self, language: Option<String>) -> DomainResult<()> {
        let previous = std::mem::replace(&mut self.language, language);
        self.check().inspect_err(|_| self.language = previous)
    }

    pub fn set_default_model(&mut self, model: ModelRef) {
        self.default_model = model;
    }

    /// Points a setting at a keyring entry. The secret itself is written to
    /// the keyring by the caller; it never passes through here.
    pub fn set_secret_ref(&mut self, key: &str, secret_ref: SecretRef) -> DomainResult<()> {
        let previous = self.secret_refs.insert(key.to_string(), secret_ref);
        self.check().inspect_err(|_| match previous {
            Some(previous) => {
                self.secret_refs.insert(key.to_string(), previous);
            }
            None => {
                self.secret_refs.remove(key);
            }
        })
    }

    pub fn default_model(&self) -> &ModelRef {
        &self.default_model
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn palette(&self) -> Option<&str> {
        self.palette.as_deref()
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn notifications(&self) -> &NotificationPrefs {
        &self.notifications
    }

    pub fn agent_defaults(&self) -> &AgentDefaults {
        &self.agent_defaults
    }

    pub fn secret_refs(&self) -> &BTreeMap<String, SecretRef> {
        &self.secret_refs
    }

    /// The configured registries, or just the default public one.
    pub fn registries(&self) -> Vec<RegistryConfig> {
        self.marketplace.registries.clone().unwrap_or_else(|| {
            vec![RegistryConfig {
                url: DEFAULT_REGISTRY.to_string(),
                kind: RegistryKind::Public,
                auth: None,
            }]
        })
    }

    /// The saved connector grants
    pub fn connector_grants(&self) -> &Grants {
        &self.connector_grants
    }

    pub fn connector_grants_mut(&mut self) -> &mut Grants {
        &mut self.connector_grants
    }

    /// What the user approved for agent `id`, if anything.
    pub fn consent(&self, id: &str) -> Option<&AgentConsent> {
        self.consent.get(id)
    }

    /// Records (replaces) what the user approved for agent `id`; an empty
    /// consent forgets it.
    pub fn set_consent(&mut self, id: &str, consent: AgentConsent) -> DomainResult<()> {
        super::AgentId::parse(id)?;
        if consent == AgentConsent::default() {
            self.consent.remove(id);
        } else {
            self.consent.insert(id.to_string(), consent);
        }
        Ok(())
    }

    /// The card I pinned for `capability`, if any
    pub fn preferred_provider(&self, capability: &str) -> Option<&str> {
        self.delegation
            .preferred_providers
            .get(capability)
            .map(String::as_str)
    }

    /// Every pin, capability → card ID
    pub fn preferred_providers(&self) -> &BTreeMap<String, String> {
        &self.delegation.preferred_providers
    }

    /// Pins `card_id` for `capability`, or unpins it with `None`.
    pub fn set_preferred_provider(
        &mut self,
        capability: &str,
        card_id: Option<&str>,
    ) -> DomainResult<()> {
        let previous = match card_id {
            Some(card_id) => self
                .delegation
                .preferred_providers
                .insert(capability.to_string(), card_id.to_string()),
            None => self.delegation.preferred_providers.remove(capability),
        };
        self.check().inspect_err(|_| match previous {
            Some(previous) => {
                self.delegation
                    .preferred_providers
                    .insert(capability.to_string(), previous);
            }
            None => {
                self.delegation.preferred_providers.remove(capability);
            }
        })
    }

    /// Replaces the registry list (an empty list is kept: no registries).
    pub fn set_registries(&mut self, registries: Vec<RegistryConfig>) -> DomainResult<()> {
        let previous = self.marketplace.registries.replace(registries);
        self.check()
            .inspect_err(|_| self.marketplace.registries = previous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENTED: &str = r#"
default_model:
  provider: deepseek
  name: deepseek-chat
theme: dark
secret_refs:
  deepseek_api_key: nandi.llm.deepseek
  aws_access_key: nandi.aws.access_key
  aws_secret_key: nandi.aws.secret_key
"#;

    #[test]
    fn provider_pins_round_trip_and_are_validated() {
        let mut settings = Settings::default();
        settings
            .set_preferred_provider("find-papers", Some("inst-1/papers"))
            .unwrap();
        let yaml = settings.to_yaml();
        assert!(yaml.contains("preferred_providers"), "{yaml}");
        let back = Settings::from_yaml(&yaml).unwrap();
        assert_eq!(
            back.preferred_provider("find-papers"),
            Some("inst-1/papers")
        );

        assert!(settings
            .set_preferred_provider("find-papers", Some("no-slash"))
            .is_err());
        // A rejected pin leaves the old one
        assert_eq!(
            settings.preferred_provider("find-papers"),
            Some("inst-1/papers")
        );
        assert!(settings.set_preferred_provider(" ", Some("a/b")).is_err());

        settings
            .set_preferred_provider("find-papers", None)
            .unwrap();
        assert_eq!(settings.preferred_provider("find-papers"), None);
        assert!(!settings.to_yaml().contains("delegation"));
    }

    #[test]
    fn consent_round_trips_and_needs_agent_ids() {
        let mut settings = Settings::default();
        let consent = AgentConsent {
            tools: vec!["shell".into()],
            network: vec!["export.arxiv.org".into()],
            ..Default::default()
        };
        settings.set_consent("arxiv", consent.clone()).unwrap();
        let reloaded = Settings::from_yaml(&settings.to_yaml()).unwrap();
        assert_eq!(reloaded.consent("arxiv"), Some(&consent));
        assert!(settings.set_consent("Bad Id", consent).is_err());
        settings
            .set_consent("arxiv", AgentConsent::default())
            .unwrap();
        assert!(!settings.to_yaml().contains("consent"));
        assert!(Settings::from_yaml("consent:\n  '../x': {tools: [shell]}\n").is_err());
    }

    #[test]
    fn registries_default_validate_and_round_trip() {
        let mut settings = Settings::default();
        assert_eq!(settings.registries()[0].url, DEFAULT_REGISTRY);
        assert!(!settings.to_yaml().contains("marketplace"));

        let mut list = settings.registries();
        list.push(
            RegistryConfig::new(
                "https://github.com/acme/nandi-community-acme.git/",
                RegistryKind::Community,
                Some(SecretRef::parse("nandi.marketplace.acme").unwrap()),
            )
            .unwrap(),
        );
        assert_eq!(list[1].url, "github.com/acme/nandi-community-acme");
        settings.set_registries(list.clone()).unwrap();
        let reloaded = Settings::from_yaml(&settings.to_yaml()).unwrap();
        assert_eq!(reloaded.registries(), list);

        list.push(list[1].clone());
        assert!(settings.set_registries(list).is_err(), "duplicates");
        assert_eq!(settings.registries().len(), 2, "unchanged after a failure");
        for bad in [
            "https://user:tok@github.com/a/b",
            "github.com/only-owner",
            "ssh://git@github.com/a/b",
            "has space",
        ] {
            assert!(
                RegistryConfig::new(bad, RegistryKind::Personal, None).is_err(),
                "{bad}"
            );
        }
        assert!(RegistryConfig::new("file:///tmp/registry", RegistryKind::Personal, None).is_ok());
        settings.set_registries(Vec::new()).unwrap();
        assert!(Settings::from_yaml(&settings.to_yaml())
            .unwrap()
            .registries()
            .is_empty());
    }

    #[test]
    fn preserves_secret_refs_without_resolving_them() {
        let settings = Settings::from_yaml(DOCUMENTED).unwrap();
        assert_eq!(
            settings.default_model().to_string(),
            "deepseek:deepseek-chat"
        );
        assert_eq!(settings.theme(), Theme::Dark);
        assert_eq!(
            settings.secret_refs()["aws_secret_key"].as_str(),
            "nandi.aws.secret_key"
        );
        let yaml = settings.to_yaml();
        assert!(
            yaml.contains("deepseek_api_key: nandi.llm.deepseek"),
            "{yaml}"
        );
        assert!(yaml.contains("provider: deepseek"), "{yaml}");
        assert_eq!(Settings::from_yaml(&yaml).unwrap(), settings);
    }

    #[test]
    fn empty_file_is_defaults() {
        assert_eq!(Settings::from_yaml("").unwrap(), Settings::default());
        assert_eq!(
            Settings::default().default_model().to_string(),
            DEFAULT_MODEL
        );
    }

    #[test]
    fn rejects_secrets_and_bad_values() {
        for (yaml, expected) in [
            (
                "secret_refs:\n  deepseek_api_key: sk-live-abc123",
                "secret ref",
            ),
            ("deepseek_api_key: sk-live-abc123", "unknown field"),
            ("theme: neon", "theme"),
            ("language: 'not a code'", "language"),
            ("default_model: {provider: gemini, name: pro}", "provider"),
            ("secret_refs:\n  Bad-Key: nandi.x", "secret_refs"),
        ] {
            let error = Settings::from_yaml(yaml).unwrap_err().to_string();
            assert!(error.contains(expected), "{yaml}: {error}");
        }
    }

    #[test]
    fn mutations_keep_invariants() {
        let mut settings = Settings::default();
        assert!(settings.set_language(Some("x y".into())).is_err());
        assert!(settings.set_palette(Some("Not Valid".into())).is_err());
        assert_eq!(settings.palette(), None);
        settings.set_palette(Some("aurora".into())).unwrap();
        assert_eq!(settings.palette(), Some("aurora"));
        assert_eq!(settings.language(), None);
        settings
            .set_secret_ref(
                "openai_api_key",
                SecretRef::parse("nandi.llm.openai").unwrap(),
            )
            .unwrap();
        assert!(settings
            .set_secret_ref("BAD", SecretRef::parse("nandi.x").unwrap())
            .is_err());
        assert_eq!(settings.secret_refs().len(), 1);
    }
}
