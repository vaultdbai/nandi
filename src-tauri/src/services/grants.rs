//! Where connector grants are kept, and the only code that changes them.
//!
//! The model is in `domain::grant`. A grant is kept either on this computer
//! (`~/.nandi/settings.local.yaml`, never committed or backed up) or saved
//! in the repo's `settings.yaml` (the gold copy). Nothing expires. Every
//! grant and removal is a decision, recorded in the decisions file
//! (`inbox::audit`), and fires `grants://changed`.
//!
//! Grants are read again on every connector call, so removing one takes
//! effect on the agent's next call, even mid-run.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::domain::grant::{ConnectorGrant, GrantAccess, GrantScope, LocalSettings};
use crate::services::domain::EntityKind;
use crate::services::events::NandiEvent;
use crate::services::repo_store::RepoStore;
use crate::services::repos::fs_atomic::write_atomic;
use crate::services::repos::SingletonRepository;

/// Next to `config.json`, outside the repo
pub const LOCAL_FILE: &str = "settings.local.yaml";

/// One grant, as Settings › Connectors › Access lists it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GrantInfo {
    pub agent: String,
    pub connector: String,
    pub access: GrantAccess,
    pub scope: GrantScope,
    /// The mailboxes it covers; empty: every mailbox of the connector
    pub mailboxes: Vec<String>,
}

impl GrantInfo {
    fn of(grant: ConnectorGrant, scope: GrantScope) -> Self {
        Self {
            agent: grant.agent,
            connector: grant.connector,
            access: grant.access,
            scope,
            mailboxes: grant.mailboxes,
        }
    }
}

/// `grants://changed`: a grant was given or removed
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GrantsChanged {}

impl NandiEvent for GrantsChanged {
    const NAME: &'static str = "grants://changed";
}

/// The two grant files of one Nandi
#[derive(Clone)]
pub struct GrantStore {
    store: RepoStore,
    local: PathBuf,
}

impl GrantStore {
    pub fn new(store: &RepoStore, nandi_dir: &Path) -> Self {
        Self {
            store: store.clone(),
            local: nandi_dir.join(LOCAL_FILE),
        }
    }

    fn load_local(&self) -> Result<LocalSettings, String> {
        match std::fs::read_to_string(&self.local) {
            Ok(text) => LocalSettings::from_yaml(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LocalSettings::default()),
            Err(e) => Err(format!("Couldn't read {}: {e}", self.local.display())),
        }
    }

    fn save_local(&self, local: &LocalSettings) -> Result<(), String> {
        write_atomic(&self.local, local.to_yaml().as_bytes())
            .map_err(|e| format!("Couldn't write {}: {e}", self.local.display()))
    }

    /// Where `agent` holds `access` on `connector`, if it does (saved first)
    pub fn scope_of(
        &self,
        agent: &str,
        connector: &str,
        access: GrantAccess,
    ) -> Option<GrantScope> {
        let saved = self.store.read(|repo| {
            repo.settings()
                .connector_grants()
                .allows(agent, connector, access)
        });
        if saved {
            return Some(GrantScope::Saved);
        }
        match self.load_local() {
            Ok(local) if local.connector_grants.allows(agent, connector, access) => {
                Some(GrantScope::Local)
            }
            Ok(_) => None,
            Err(e) => {
                log::warn!("{e}");
                None
            }
        }
    }

    pub fn allows(&self, agent: &str, connector: &str, access: GrantAccess) -> bool {
        self.scope_of(agent, connector, access).is_some()
    }

    /// The mailboxes `agent`'s grants on `connector` cover, saved and local
    /// together. `None`: no grant; `Some([])`: every mailbox
    pub fn mailboxes(
        &self,
        agent: &str,
        connector: &str,
        access: GrantAccess,
    ) -> Option<Vec<String>> {
        let found: Vec<Vec<String>> = self
            .list()
            .into_iter()
            .filter(|g| g.agent == agent && g.connector == connector && g.access == access)
            .map(|g| g.mailboxes)
            .collect();
        if found.is_empty() {
            return None;
        }
        if found.iter().any(Vec::is_empty) {
            return Some(Vec::new());
        }
        Some(crate::services::domain::grant::normalize_mailboxes(
            &found.concat(),
        ))
    }

    /// Whether `agent` may use `mailbox` of `connector` with `access`
    pub fn covers(&self, agent: &str, connector: &str, access: GrantAccess, mailbox: &str) -> bool {
        let saved = self.store.read(|repo| {
            repo.settings()
                .connector_grants()
                .covers(agent, connector, access, mailbox)
        });
        saved
            || self.load_local().is_ok_and(|local| {
                local
                    .connector_grants
                    .covers(agent, connector, access, mailbox)
            })
    }

    /// Every grant, saved then local, each sorted by agent and connector
    pub fn list(&self) -> Vec<GrantInfo> {
        let mut out: Vec<GrantInfo> = self.store.read(|repo| {
            repo.settings()
                .connector_grants()
                .iter()
                .map(|g| GrantInfo::of(g.clone(), GrantScope::Saved))
                .collect()
        });
        match self.load_local() {
            Ok(local) => out.extend(
                local
                    .connector_grants
                    .iter()
                    .map(|g| GrantInfo::of(g.clone(), GrantScope::Local)),
            ),
            Err(e) => log::warn!("{e}"),
        }
        out
    }

    /// Sets which mailboxes `grant` covers (empty: every one). `false` when
    /// it already covered just those; an error when there is no such grant
    pub fn set_mailboxes(&self, grant: &GrantInfo, mailboxes: &[String]) -> Result<bool, String> {
        let missing = || format!("{} has no such grant on {}", grant.agent, grant.connector);
        let set = |grants: &mut crate::services::domain::grant::Grants| {
            grants
                .set_mailboxes(&grant.agent, &grant.connector, grant.access, mailboxes)
                .map_err(|e| e.to_string())?
                .ok_or_else(missing)
        };
        match grant.scope {
            GrantScope::Saved => {
                let mut settings = self.store.read(|repo| repo.settings().clone());
                if !set(settings.connector_grants_mut())? {
                    return Ok(false);
                }
                self.store.repos().settings.save(&settings)?;
                self.store.commit(EntityKind::Settings, "settings")?;
                Ok(true)
            }
            GrantScope::Local => {
                let mut local = self.load_local()?;
                if !set(&mut local.connector_grants)? {
                    return Ok(false);
                }
                self.save_local(&local)?;
                Ok(true)
            }
        }
    }

    /// `mailbox` of `connector` was signed out: grants naming it lose it,
    /// and a grant that named only it goes. Returns the grants removed and
    /// the grants narrowed, from both places
    pub fn drop_mailbox(
        &self,
        connector: &str,
        mailbox: &str,
    ) -> Result<(Vec<GrantInfo>, Vec<GrantInfo>), String> {
        let mut removed = Vec::new();
        let mut narrowed = Vec::new();
        let mut keep = |(gone, less): (Vec<ConnectorGrant>, Vec<ConnectorGrant>), scope| {
            removed.extend(gone.into_iter().map(|g| GrantInfo::of(g, scope)));
            narrowed.extend(less.into_iter().map(|g| GrantInfo::of(g, scope)));
        };
        let mut settings = self.store.read(|repo| repo.settings().clone());
        let saved = settings
            .connector_grants_mut()
            .drop_mailbox(connector, mailbox);
        if !saved.0.is_empty() || !saved.1.is_empty() {
            self.store.repos().settings.save(&settings)?;
            self.store.commit(EntityKind::Settings, "settings")?;
        }
        keep(saved, GrantScope::Saved);
        let mut local = self.load_local()?;
        let here = local.connector_grants.drop_mailbox(connector, mailbox);
        if !here.0.is_empty() || !here.1.is_empty() {
            self.save_local(&local)?;
        }
        keep(here, GrantScope::Local);
        Ok((removed, narrowed))
    }

    /// Gives `grant`, kept in `scope`. `false` when it was already there
    pub fn grant(&self, grant: ConnectorGrant, scope: GrantScope) -> Result<bool, String> {
        match scope {
            GrantScope::Saved => {
                let mut settings = self.store.read(|repo| repo.settings().clone());
                if !settings.connector_grants_mut().add(grant) {
                    return Ok(false);
                }
                self.store.repos().settings.save(&settings)?;
                self.store.commit(EntityKind::Settings, "settings")?;
                Ok(true)
            }
            GrantScope::Local => {
                let mut local = self.load_local()?;
                if !local.connector_grants.add(grant) {
                    return Ok(false);
                }
                self.save_local(&local)?;
                Ok(true)
            }
        }
    }

    /// Removes one grant from `scope`. `false` when it wasn't there
    pub fn revoke(
        &self,
        agent: &str,
        connector: &str,
        access: GrantAccess,
        scope: GrantScope,
    ) -> Result<bool, String> {
        match scope {
            GrantScope::Saved => {
                let mut settings = self.store.read(|repo| repo.settings().clone());
                if !settings
                    .connector_grants_mut()
                    .remove(agent, connector, access)
                {
                    return Ok(false);
                }
                self.store.repos().settings.save(&settings)?;
                self.store.commit(EntityKind::Settings, "settings")?;
                Ok(true)
            }
            GrantScope::Local => {
                let mut local = self.load_local()?;
                if !local.connector_grants.remove(agent, connector, access) {
                    return Ok(false);
                }
                self.save_local(&local)?;
                Ok(true)
            }
        }
    }

    /// Removes every grant on `connector` from both places (it was
    /// uninstalled, or its account signed out)
    pub fn revoke_connector(&self, connector: &str) -> Result<Vec<GrantInfo>, String> {
        let mut gone = Vec::new();
        let mut settings = self.store.read(|repo| repo.settings().clone());
        let saved = settings.connector_grants_mut().remove_connector(connector);
        if !saved.is_empty() {
            self.store.repos().settings.save(&settings)?;
            self.store.commit(EntityKind::Settings, "settings")?;
        }
        let mut local = self.load_local()?;
        let local_gone = local.connector_grants.remove_connector(connector);
        if !local_gone.is_empty() {
            self.save_local(&local)?;
        }
        for (grants, scope) in [(saved, GrantScope::Saved), (local_gone, GrantScope::Local)] {
            gone.extend(grants.into_iter().map(|g| GrantInfo::of(g, scope)));
        }
        Ok(gone)
    }
}

/// The decisions file's kind for a grant given or removed
pub const GRANT_KIND: &str = "connectorGrant";

/// Records a grant given (`granted`) or removed (`removed`) by this Nandi's
/// user in the decisions file, with `why` when it followed from something
/// else (a connector removed, an account signed out)
pub fn record(
    state: &crate::services::AppState,
    decision: &'static str,
    grant: &GrantInfo,
    why: Option<&str>,
) {
    use crate::services::inbox::audit::{self, Decided};
    audit::record(
        state,
        Decided::new(
            GRANT_KIND,
            format!("{}/{}", grant.agent, grant.connector),
            decision,
        )
        .subject(format!(
            "{}: {} with {}{} ({})",
            grant.agent,
            grant.access.as_str(),
            grant.connector,
            if grant.mailboxes.is_empty() {
                String::new()
            } else {
                format!(", only {}", grant.mailboxes.join(", "))
            },
            grant.scope.as_str()
        ))
        .note(why),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::services::repos::{RepoLayout, Repos};

    fn grant_store() -> (tempfile::TempDir, GrantStore) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        let store = RepoStore::sqlite(Repos::new(layout), Db::in_memory());
        store.load().unwrap();
        let grants = GrantStore::new(&store, tmp.path());
        (tmp, grants)
    }

    fn grant(agent: &str, connector: &str, access: GrantAccess) -> ConnectorGrant {
        ConnectorGrant::new(agent, connector, access).unwrap()
    }

    #[test]
    fn saved_grants_go_in_settings_yaml_and_local_ones_stay_out_of_the_repo() {
        let (tmp, grants) = grant_store();
        assert!(grants
            .grant(
                grant("coordinator", "outlook", GrantAccess::Read),
                GrantScope::Saved
            )
            .unwrap());
        assert!(grants
            .grant(
                grant("coordinator", "tracker", GrantAccess::Write),
                GrantScope::Local
            )
            .unwrap());
        let settings =
            std::fs::read_to_string(tmp.path().join("repo/.nandi/settings.yaml")).unwrap();
        assert!(settings.contains("connector: outlook"), "{settings}");
        assert!(!settings.contains("tracker"), "{settings}");
        let local = std::fs::read_to_string(tmp.path().join(LOCAL_FILE)).unwrap();
        assert!(local.contains("connector: tracker"), "{local}");

        assert_eq!(
            grants.scope_of("coordinator", "outlook", GrantAccess::Read),
            Some(GrantScope::Saved)
        );
        assert_eq!(
            grants.scope_of("coordinator", "tracker", GrantAccess::Write),
            Some(GrantScope::Local)
        );
        // Read and write are kept apart
        assert!(!grants.allows("coordinator", "tracker", GrantAccess::Read));
        assert_eq!(grants.list().len(), 2);

        assert!(grants
            .revoke(
                "coordinator",
                "tracker",
                GrantAccess::Write,
                GrantScope::Local
            )
            .unwrap());
        assert!(!grants.allows("coordinator", "tracker", GrantAccess::Write));
        assert!(!grants
            .revoke(
                "coordinator",
                "tracker",
                GrantAccess::Write,
                GrantScope::Local
            )
            .unwrap());
    }

    #[test]
    fn mailboxes_join_both_places_and_go_with_their_sign_out() {
        let (_tmp, grants) = grant_store();
        let read = GrantAccess::Read;
        let named = |agent: &str, list: &[&str]| {
            let list: Vec<String> = list.iter().map(ToString::to_string).collect();
            grant(agent, "gmail", read).for_mailboxes(&list).unwrap()
        };
        assert_eq!(grants.mailboxes("coordinator", "gmail", read), None);
        grants
            .grant(named("coordinator", &["a@x.com"]), GrantScope::Saved)
            .unwrap();
        grants
            .grant(named("coordinator", &["b@y.com"]), GrantScope::Local)
            .unwrap();
        assert_eq!(
            grants.mailboxes("coordinator", "gmail", read),
            Some(vec!["a@x.com".to_string(), "b@y.com".to_string()])
        );
        assert!(grants.covers("coordinator", "gmail", read, "B@y.com"));
        assert!(!grants.covers("coordinator", "gmail", read, "c@z.com"));

        // Narrowed in Settings, then widened again
        let saved = grants
            .list()
            .into_iter()
            .find(|g| g.scope == GrantScope::Saved)
            .unwrap();
        assert!(grants.set_mailboxes(&saved, &[]).unwrap());
        assert_eq!(grants.mailboxes("coordinator", "gmail", read), Some(vec![]));
        assert!(grants
            .set_mailboxes(&saved, &["a@x.com".to_string()])
            .unwrap());

        let (removed, narrowed) = grants.drop_mailbox("gmail", "a@x.com").unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].scope, GrantScope::Saved);
        assert!(narrowed.is_empty());
        assert_eq!(
            grants.mailboxes("coordinator", "gmail", read),
            Some(vec!["b@y.com".to_string()])
        );
        let gone = GrantInfo {
            agent: "coordinator".into(),
            connector: "outlook".into(),
            access: read,
            scope: GrantScope::Saved,
            mailboxes: vec![],
        };
        assert!(grants.set_mailboxes(&gone, &[]).is_err());
    }

    #[test]
    fn a_grant_on_every_mailbox_stays_when_one_signs_out() {
        let (tmp, grants) = grant_store();
        grants
            .grant(
                grant("coordinator", "outlook", GrantAccess::Read),
                GrantScope::Saved,
            )
            .unwrap();
        let file = tmp.path().join("repo/.nandi/settings.yaml");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("mailboxes"), "{text}");
        assert!(grants.covers("coordinator", "outlook", GrantAccess::Read, "a@x.com"));
        assert!(grants.covers("coordinator", "outlook", GrantAccess::Read, "b@y.com"));
        // Signing one mailbox out leaves a grant on every mailbox alone
        let (removed, narrowed) = grants.drop_mailbox("outlook", "a@x.com").unwrap();
        assert!(removed.is_empty() && narrowed.is_empty());
        assert!(grants.allows("coordinator", "outlook", GrantAccess::Read));
    }

    #[test]
    fn removing_a_connector_clears_both_places() {
        let (_tmp, grants) = grant_store();
        grants
            .grant(grant("a", "outlook", GrantAccess::Read), GrantScope::Saved)
            .unwrap();
        grants
            .grant(grant("b", "outlook", GrantAccess::Read), GrantScope::Local)
            .unwrap();
        grants
            .grant(grant("a", "gmail", GrantAccess::Read), GrantScope::Local)
            .unwrap();
        let gone = grants.revoke_connector("outlook").unwrap();
        assert_eq!(gone.len(), 2);
        assert_eq!(grants.list().len(), 1);
    }
}
