//! Connector grants: "this agent may read (or write) with this connector".
//!
//! One model for every connector that reaches a person's data: installed
//! MCP connectors, mail, calendars and files. A grant is per agent and per
//! connector, with read and write kept apart, and it never expires: it lasts
//! until the person removes it. Grants are kept in one of two places:
//!
//! | Scope   | File                                   | Travels with the repo |
//! | ------- | -------------------------------------- | --------------------- |
//! | `local` | `~/.nandi/settings.local.yaml`         | no (this computer)    |
//! | `saved` | `.nandi/settings.yaml` (the gold copy) | yes (backed up)       |
//!
//! ```yaml
//! connector_grants:
//!   - { agent: chief-of-staff, connector: outlook, access: read }
//! ```

use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{EnumString, IntoStaticStr};

use super::{AgentId, DomainError, DomainResult};

/// Read or write access, kept apart: a write grant doesn't let an agent read
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Type,
    EnumString,
    IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum GrantAccess {
    Read,
    Write,
}

impl GrantAccess {
    /// The access's name
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Where a grant is kept
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum GrantScope {
    /// This computer only (`~/.nandi/settings.local.yaml`)
    Local,
    /// The repo's `settings.yaml`: backed up, and on a new computer too
    Saved,
}

impl GrantScope {
    /// The scope's name
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
pub struct ConnectorGrant {
    pub agent: String,
    /// A connector ID: `outlook`, `google-calendar`, an installed one, …
    pub connector: String,
    pub access: GrantAccess,
}

impl ConnectorGrant {
    pub fn new(agent: &str, connector: &str, access: GrantAccess) -> DomainResult<Self> {
        let grant = Self {
            agent: agent.to_string(),
            connector: connector.to_string(),
            access,
        };
        grant.check()?;
        Ok(grant)
    }

    fn check(&self) -> DomainResult<()> {
        AgentId::parse(self.agent.clone())?;
        let ok = !self.connector.is_empty()
            && self.connector.len() <= 64
            && self
                .connector
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if ok {
            Ok(())
        } else {
            Err(DomainError::invalid(
                format!("Grant for '{}'", self.agent),
                "connector",
                "must be a connector ID (lowercase letters, digits, '-', '_')",
            ))
        }
    }

    pub fn is(&self, agent: &str, connector: &str, access: GrantAccess) -> bool {
        self.agent == agent && self.connector == connector && self.access == access
    }
}

/// The grants of one scope, without duplicates, in a stable order
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grants(Vec<ConnectorGrant>);

impl Grants {
    pub fn new(grants: Vec<ConnectorGrant>) -> DomainResult<Self> {
        let mut out = Self::default();
        for grant in grants {
            grant.check()?;
            out.add(grant);
        }
        Ok(out)
    }

    pub fn allows(&self, agent: &str, connector: &str, access: GrantAccess) -> bool {
        self.0.iter().any(|g| g.is(agent, connector, access))
    }

    /// `false` when it was already there
    pub fn add(&mut self, grant: ConnectorGrant) -> bool {
        if self.0.contains(&grant) {
            return false;
        }
        self.0.push(grant);
        self.0.sort_by(|a, b| {
            (&a.agent, &a.connector, a.access).cmp(&(&b.agent, &b.connector, b.access))
        });
        true
    }

    /// `false` when there was nothing to remove
    pub fn remove(&mut self, agent: &str, connector: &str, access: GrantAccess) -> bool {
        let before = self.0.len();
        self.0.retain(|g| !g.is(agent, connector, access));
        before != self.0.len()
    }

    /// Every grant on `connector` (it was uninstalled or signed out)
    pub fn remove_connector(&mut self, connector: &str) -> Vec<ConnectorGrant> {
        let (gone, kept) = std::mem::take(&mut self.0)
            .into_iter()
            .partition(|g| g.connector == connector);
        self.0 = kept;
        gone
    }

    pub fn iter(&self) -> impl Iterator<Item = &ConnectorGrant> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `~/.nandi/settings.local.yaml`: settings for this computer only
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSettings {
    #[serde(default, skip_serializing_if = "Grants::is_empty")]
    pub connector_grants: Grants,
}

impl LocalSettings {
    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        let raw: Self = serde_yaml::from_str(content)
            .map_err(|e| DomainError::parse("settings.local.yaml", e))?;
        Ok(Self {
            connector_grants: Grants::new(raw.connector_grants.0)?,
        })
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(agent: &str, connector: &str, access: GrantAccess) -> ConnectorGrant {
        ConnectorGrant::new(agent, connector, access).unwrap()
    }

    #[test]
    fn grants_are_per_agent_connector_and_access() {
        let mut grants = Grants::default();
        assert!(grants.add(grant("chief-of-staff", "outlook", GrantAccess::Read)));
        assert!(!grants.add(grant("chief-of-staff", "outlook", GrantAccess::Read)));
        assert!(grants.allows("chief-of-staff", "outlook", GrantAccess::Read));
        assert!(!grants.allows("chief-of-staff", "outlook", GrantAccess::Write));
        assert!(!grants.allows("chief-of-staff", "gmail", GrantAccess::Read));
        assert!(!grants.allows("pr-reviewer", "outlook", GrantAccess::Read));
        assert!(grants.remove("chief-of-staff", "outlook", GrantAccess::Read));
        assert!(!grants.remove("chief-of-staff", "outlook", GrantAccess::Read));
        assert!(grants.is_empty());
    }

    #[test]
    fn removing_a_connector_takes_all_its_grants() {
        let mut grants = Grants::default();
        grants.add(grant("a", "tracker", GrantAccess::Read));
        grants.add(grant("b", "tracker", GrantAccess::Write));
        grants.add(grant("a", "outlook", GrantAccess::Read));
        assert_eq!(grants.remove_connector("tracker").len(), 2);
        assert_eq!(grants.iter().count(), 1);
    }

    #[test]
    fn local_settings_round_trip_and_refuse_bad_grants() {
        let yaml =
            "connector_grants:\n  - { agent: chief-of-staff, connector: outlook, access: read }\n";
        let local = LocalSettings::from_yaml(yaml).unwrap();
        assert!(local
            .connector_grants
            .allows("chief-of-staff", "outlook", GrantAccess::Read));
        assert_eq!(LocalSettings::from_yaml(&local.to_yaml()).unwrap(), local);
        assert_eq!(
            LocalSettings::from_yaml("").unwrap(),
            LocalSettings::default()
        );
        for bad in [
            "connector_grants:\n  - { agent: 'Bad Agent', connector: outlook, access: read }\n",
            "connector_grants:\n  - { agent: a, connector: 'Out Look', access: read }\n",
            "connector_grants:\n  - { agent: a, connector: outlook, access: admin }\n",
            "other: 1\n",
        ] {
            assert!(LocalSettings::from_yaml(bad).is_err(), "{bad}");
        }
    }
}
