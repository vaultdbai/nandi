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
//!   - { agent: coordinator, connector: outlook, access: read }
//!   - { agent: coordinator, connector: gmail, access: read, mailboxes: [me@work.com] }
//! ```
//!
//! A mail grant covers every mailbox of its provider, ones connected later
//! too, unless it names `mailboxes`: then only those. Grants written before
//! mailboxes could be named have none, so they cover every mailbox as they
//! always did. A grant's last named mailbox going takes the grant with it:
//! it never widens to every mailbox.

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
    /// The mailboxes it covers, by address (lowercase, sorted). Empty:
    /// every mailbox of the connector
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mailboxes: Vec<String>,
}

/// Addresses as a grant keeps them: trimmed, lowercase, sorted, once each
pub fn normalize_mailboxes(mailboxes: &[String]) -> Vec<String> {
    let mut out: Vec<String> = mailboxes
        .iter()
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| !m.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

impl ConnectorGrant {
    pub fn new(agent: &str, connector: &str, access: GrantAccess) -> DomainResult<Self> {
        let grant = Self {
            agent: agent.to_string(),
            connector: connector.to_string(),
            access,
            mailboxes: Vec::new(),
        };
        grant.check()?;
        Ok(grant)
    }

    /// The grant covering only `mailboxes` (empty: every mailbox)
    pub fn for_mailboxes(self, mailboxes: &[String]) -> DomainResult<Self> {
        let grant = Self {
            mailboxes: normalize_mailboxes(mailboxes),
            ..self
        };
        grant.check()?;
        Ok(grant)
    }

    /// Whether it lets its agent use `mailbox`
    pub fn covers(&self, mailbox: &str) -> bool {
        self.mailboxes.is_empty()
            || self
                .mailboxes
                .iter()
                .any(|m| m.eq_ignore_ascii_case(mailbox.trim()))
    }

    fn check(&self) -> DomainResult<()> {
        AgentId::parse(self.agent.clone())?;
        let ok = !self.connector.is_empty()
            && self.connector.len() <= 64
            && self
                .connector
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !ok {
            return Err(DomainError::invalid(
                format!("Grant for '{}'", self.agent),
                "connector",
                "must be a connector ID (lowercase letters, digits, '-', '_')",
            ));
        }
        let address = |m: &String| {
            m.len() <= 254
                && m.split_once('@')
                    .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
                && !m.chars().any(char::is_whitespace)
                && *m == m.to_ascii_lowercase()
        };
        if self.mailboxes.iter().all(address) {
            Ok(())
        } else {
            Err(DomainError::invalid(
                format!("Grant for '{}'", self.agent),
                "mailboxes",
                "must be email addresses",
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
        self.find(agent, connector, access).is_some()
    }

    /// The grant of `agent` on `connector` for `access`, if there is one
    pub fn find(
        &self,
        agent: &str,
        connector: &str,
        access: GrantAccess,
    ) -> Option<&ConnectorGrant> {
        self.0.iter().find(|g| g.is(agent, connector, access))
    }

    /// Whether `agent` may use `mailbox` of `connector` with `access`
    pub fn covers(&self, agent: &str, connector: &str, access: GrantAccess, mailbox: &str) -> bool {
        self.find(agent, connector, access)
            .is_some_and(|g| g.covers(mailbox))
    }

    /// Gives `grant`. A grant already there for the same agent, connector
    /// and access takes its mailboxes too (every mailbox when either covers
    /// every one). `false` when nothing changed
    pub fn add(&mut self, grant: ConnectorGrant) -> bool {
        if let Some(known) = self
            .0
            .iter_mut()
            .find(|g| g.is(&grant.agent, &grant.connector, grant.access))
        {
            let merged = if known.mailboxes.is_empty() || grant.mailboxes.is_empty() {
                Vec::new()
            } else {
                normalize_mailboxes(&[known.mailboxes.clone(), grant.mailboxes].concat())
            };
            if merged == known.mailboxes {
                return false;
            }
            known.mailboxes = merged;
            return true;
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

    /// Sets which mailboxes a grant covers (empty: every one). `None`: there
    /// is no such grant; else whether it changed
    pub fn set_mailboxes(
        &mut self,
        agent: &str,
        connector: &str,
        access: GrantAccess,
        mailboxes: &[String],
    ) -> DomainResult<Option<bool>> {
        let Some(grant) = self.0.iter_mut().find(|g| g.is(agent, connector, access)) else {
            return Ok(None);
        };
        let next = grant.clone().for_mailboxes(mailboxes)?;
        let changed = next.mailboxes != grant.mailboxes;
        *grant = next;
        Ok(Some(changed))
    }

    /// `mailbox` of `connector` was signed out: grants naming it lose it.
    /// Returns the grants removed (it was the last one they named) and the
    /// grants that still name others. Grants covering every mailbox stay
    pub fn drop_mailbox(
        &mut self,
        connector: &str,
        mailbox: &str,
    ) -> (Vec<ConnectorGrant>, Vec<ConnectorGrant>) {
        let gone = |m: &String| m.eq_ignore_ascii_case(mailbox.trim());
        let mut removed = Vec::new();
        let mut narrowed = Vec::new();
        let mut kept = Vec::new();
        for mut grant in std::mem::take(&mut self.0) {
            if grant.connector != connector || !grant.mailboxes.iter().any(gone) {
                kept.push(grant);
                continue;
            }
            grant.mailboxes.retain(|m| !gone(m));
            if grant.mailboxes.is_empty() {
                removed.push(grant);
            } else {
                narrowed.push(grant.clone());
                kept.push(grant);
            }
        }
        self.0 = kept;
        (removed, narrowed)
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
        assert!(grants.add(grant("coordinator", "outlook", GrantAccess::Read)));
        assert!(!grants.add(grant("coordinator", "outlook", GrantAccess::Read)));
        assert!(grants.allows("coordinator", "outlook", GrantAccess::Read));
        assert!(!grants.allows("coordinator", "outlook", GrantAccess::Write));
        assert!(!grants.allows("coordinator", "gmail", GrantAccess::Read));
        assert!(!grants.allows("pr-reviewer", "outlook", GrantAccess::Read));
        assert!(grants.remove("coordinator", "outlook", GrantAccess::Read));
        assert!(!grants.remove("coordinator", "outlook", GrantAccess::Read));
        assert!(grants.is_empty());
    }

    fn addresses(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    fn named(agent: &str, connector: &str, list: &[&str]) -> ConnectorGrant {
        grant(agent, connector, GrantAccess::Read)
            .for_mailboxes(&addresses(list))
            .unwrap()
    }

    #[test]
    fn a_grant_covers_every_mailbox_unless_it_names_some() {
        let mut grants = Grants::default();
        grants.add(grant("coordinator", "gmail", GrantAccess::Read));
        assert!(grants.covers("coordinator", "gmail", GrantAccess::Read, "any@one.com"));
        let work = named("pr-reviewer", "gmail", &[" Me@Work.com "]);
        assert_eq!(work.mailboxes, ["me@work.com"]);
        grants.add(work);
        let read = GrantAccess::Read;
        assert!(grants.covers("pr-reviewer", "gmail", read, "ME@work.com"));
        assert!(!grants.covers("pr-reviewer", "gmail", read, "me@home.com"));
        assert!(!grants.covers("pr-reviewer", "outlook", read, "me@work.com"));
    }

    #[test]
    fn adding_merges_mailboxes_and_every_mailbox_wins() {
        let mut grants = Grants::default();
        let mailboxes = |grants: &Grants| {
            grants
                .find("coordinator", "gmail", GrantAccess::Read)
                .unwrap()
                .mailboxes
                .clone()
        };
        assert!(grants.add(named("coordinator", "gmail", &["a@x.com"])));
        assert!(!grants.add(named("coordinator", "gmail", &["a@x.com"])));
        assert!(grants.add(named("coordinator", "gmail", &["b@y.com"])));
        assert_eq!(mailboxes(&grants), ["a@x.com", "b@y.com"]);
        assert!(grants.add(grant("coordinator", "gmail", GrantAccess::Read)));
        assert!(mailboxes(&grants).is_empty());
        // Every mailbox stays every mailbox
        assert!(!grants.add(named("coordinator", "gmail", &["c@z.com"])));
        assert_eq!(grants.iter().count(), 1);
    }

    #[test]
    fn narrowing_and_widening_a_grant() {
        let mut grants = Grants::default();
        let read = GrantAccess::Read;
        grants.add(grant("coordinator", "outlook", read));
        let only_a = addresses(&["a@x.com"]);
        let set = |grants: &mut Grants, connector: &str, list: &[String]| {
            grants
                .set_mailboxes("coordinator", connector, read, list)
                .unwrap()
        };
        assert_eq!(set(&mut grants, "outlook", &only_a), Some(true));
        assert_eq!(set(&mut grants, "outlook", &only_a), Some(false));
        assert!(!grants.covers("coordinator", "outlook", read, "b@x.com"));
        assert_eq!(set(&mut grants, "outlook", &[]), Some(true));
        assert!(grants.covers("coordinator", "outlook", read, "b@x.com"));
        assert_eq!(set(&mut grants, "gmail", &[]), None);
        let bad = addresses(&["not an address"]);
        assert!(grants
            .set_mailboxes("coordinator", "outlook", read, &bad)
            .is_err());
    }

    #[test]
    fn a_signed_out_mailbox_leaves_its_grants_and_never_widens_them() {
        let mut grants = Grants::default();
        grants.add(named("coordinator", "gmail", &["a@x.com"]));
        grants.add(named("pr-reviewer", "gmail", &["a@x.com", "b@y.com"]));
        grants.add(grant("triager", "gmail", GrantAccess::Read));
        let (removed, narrowed) = grants.drop_mailbox("gmail", "A@x.com");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].agent, "coordinator");
        assert_eq!(narrowed.len(), 1);
        assert_eq!(narrowed[0].mailboxes, ["b@y.com"]);
        // The grant went: it doesn't become "every mailbox"
        assert!(!grants.allows("coordinator", "gmail", GrantAccess::Read));
        assert!(grants.covers("triager", "gmail", GrantAccess::Read, "a@x.com"));
    }

    #[test]
    fn grants_saved_before_mailboxes_cover_every_mailbox() {
        // Nandi 0.2 wrote grants without `mailboxes`: they load unchanged
        // and cover every mailbox, as before
        let yaml =
            "connector_grants:\n  - { agent: coordinator, connector: gmail, access: read }\n";
        let local = LocalSettings::from_yaml(yaml).unwrap();
        assert!(local.connector_grants.covers(
            "coordinator",
            "gmail",
            GrantAccess::Read,
            "second@gmail.com"
        ));
        // Written back the same way: nothing to rewrite on disk
        assert!(
            !local.to_yaml().contains("mailboxes"),
            "{}",
            local.to_yaml()
        );
        let named = "connector_grants:\n  - { agent: coordinator, connector: gmail, access: read, mailboxes: [a@x.com] }\n";
        let local = LocalSettings::from_yaml(named).unwrap();
        assert_eq!(LocalSettings::from_yaml(&local.to_yaml()).unwrap(), local);
        let bad = "connector_grants:\n  - { agent: coordinator, connector: gmail, access: read, mailboxes: [Not An Address] }\n";
        assert!(LocalSettings::from_yaml(bad).is_err());
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
            "connector_grants:\n  - { agent: coordinator, connector: outlook, access: read }\n";
        let local = LocalSettings::from_yaml(yaml).unwrap();
        assert!(local
            .connector_grants
            .allows("coordinator", "outlook", GrantAccess::Read));
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
