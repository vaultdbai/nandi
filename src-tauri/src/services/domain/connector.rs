//! Connector declarations: `.nandi/connectors.yaml`. Declarations only, with
//! secrets referenced by keyring entry. Nothing consumes them at runtime
//! yet; the schema is here so repos can carry them forward-compatibly.
//!
//! ```yaml
//! connectors:
//!   - id: team-slack
//!     kind: slack
//!     config:
//!       workspace: acme
//!     secret_refs:
//!       bot_token: nandi.connector.team-slack
//! ```

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::settings::SecretRef;
use super::{ConnectorId, DomainError, DomainResult, EntityKind};

/// Config keys that look like credentials. They belong in `secret_refs`.
const SECRET_LIKE_KEYS: &[&str] = &[
    "secret",
    "token",
    "password",
    "api_key",
    "apikey",
    "private_key",
    "access_key",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connector {
    pub id: ConnectorId,
    /// e.g. `slack`, `github`, `postgres`
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_refs: BTreeMap<String, SecretRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorManifest {
    #[serde(default)]
    connectors: Vec<Connector>,
}

impl ConnectorManifest {
    pub fn new(connectors: Vec<Connector>) -> DomainResult<Self> {
        let manifest = Self { connectors };
        manifest.check()?;
        Ok(manifest)
    }

    fn check(&self) -> DomainResult<()> {
        let mut seen = HashSet::new();
        for connector in &self.connectors {
            if !seen.insert(connector.id.as_str()) {
                return Err(DomainError::Duplicate {
                    kind: EntityKind::Connectors,
                    id: connector.id.to_string(),
                });
            }
            let label = format!("Connector '{}'", connector.id);
            let kind_ok = !connector.kind.is_empty()
                && connector
                    .kind
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
            if !kind_ok {
                return Err(DomainError::invalid(
                    label,
                    "kind",
                    "must use lowercase letters, digits, '-', '_'",
                ));
            }
            if let Some(key) = connector.config.keys().find(|key| {
                let key = key.to_ascii_lowercase();
                SECRET_LIKE_KEYS.iter().any(|s| key.contains(s))
            }) {
                return Err(DomainError::invalid(
                    label,
                    "config",
                    format!("key '{key}' looks like a credential: put it in secret_refs"),
                ));
            }
        }
        Ok(())
    }

    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        let manifest: Self =
            serde_yaml::from_str(content).map_err(|e| DomainError::parse("connectors.yaml", e))?;
        manifest.check()?;
        Ok(manifest)
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_default()
    }

    pub fn connectors(&self) -> &[Connector] {
        &self.connectors
    }

    pub fn get(&self, id: &str) -> Option<&Connector> {
        self.connectors.iter().find(|c| c.id.as_str() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENTED: &str = r#"
connectors:
  - id: team-slack
    kind: slack
    config:
      workspace: acme
    secret_refs:
      bot_token: nandi.connector.team-slack
"#;

    #[test]
    fn parses_and_round_trips() {
        let manifest = ConnectorManifest::from_yaml(DOCUMENTED).unwrap();
        let slack = manifest.get("team-slack").unwrap();
        assert_eq!(slack.kind, "slack");
        assert_eq!(
            slack.secret_refs["bot_token"].as_str(),
            "nandi.connector.team-slack"
        );
        assert_eq!(
            ConnectorManifest::from_yaml(&manifest.to_yaml()).unwrap(),
            manifest
        );
        assert!(ConnectorManifest::from_yaml("")
            .unwrap()
            .connectors()
            .is_empty());
    }

    #[test]
    fn rejects_credentials_and_duplicates() {
        for (yaml, expected) in [
            (
                "connectors:\n  - {id: a, kind: slack, config: {bot_token: xoxb-1}}",
                "credential",
            ),
            (
                "connectors:\n  - {id: a, kind: slack}\n  - {id: a, kind: github}",
                "Duplicate",
            ),
            ("connectors:\n  - {id: a, kind: Slack!}", "kind"),
            (
                "connectors:\n  - {id: a, kind: slack, secret_refs: {t: plain-token}}",
                "secret ref",
            ),
        ] {
            let error = ConnectorManifest::from_yaml(yaml).unwrap_err().to_string();
            assert!(error.contains(expected), "{yaml}: {error}");
        }
    }
}
