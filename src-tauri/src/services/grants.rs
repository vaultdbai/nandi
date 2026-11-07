//! Where connector grants are kept, and the only code that changes them.
//!
//! The model is in `domain::grant`. A grant is kept either on this computer
//! (`~/.nandi/settings.local.yaml`, never committed or backed up) or saved
//! in the repo's `settings.yaml` (the gold copy). Nothing expires. Every
//! grant and removal goes to the connector audit log
//! (`~/.nandi/local/logs/connectors.log`) and fires `grants://changed`.
//!
//! Grants are read again on every connector call, so removing one takes
//! effect on the agent's next call, even mid-run.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::connectors::audit::{AccessEntry, AuditSink};
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

    /// Every grant, saved then local, each sorted by agent and connector
    pub fn list(&self) -> Vec<GrantInfo> {
        let info = |grant: &ConnectorGrant, scope| GrantInfo {
            agent: grant.agent.clone(),
            connector: grant.connector.clone(),
            access: grant.access,
            scope,
        };
        let mut out: Vec<GrantInfo> = self.store.read(|repo| {
            repo.settings()
                .connector_grants()
                .iter()
                .map(|g| info(g, GrantScope::Saved))
                .collect()
        });
        match self.load_local() {
            Ok(local) => out.extend(
                local
                    .connector_grants
                    .iter()
                    .map(|g| info(g, GrantScope::Local)),
            ),
            Err(e) => log::warn!("{e}"),
        }
        out
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
            gone.extend(grants.into_iter().map(|g| GrantInfo {
                agent: g.agent,
                connector: g.connector,
                access: g.access,
                scope,
            }));
        }
        Ok(gone)
    }
}

/// Records a grant event in the audit log
pub async fn audit(
    sink: &AuditSink,
    event: &'static str,
    grant: &GrantInfo,
    by: Option<String>,
    run_id: Option<String>,
) {
    let mut entry = AccessEntry::new(event, &grant.agent, &grant.connector, grant.access.as_str());
    entry.scope = Some(grant.scope.as_str());
    entry.by = by;
    entry.run_id = run_id;
    sink.record_access(&entry).await;
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
                grant("chief-of-staff", "outlook", GrantAccess::Read),
                GrantScope::Saved
            )
            .unwrap());
        assert!(grants
            .grant(
                grant("chief-of-staff", "tracker", GrantAccess::Write),
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
            grants.scope_of("chief-of-staff", "outlook", GrantAccess::Read),
            Some(GrantScope::Saved)
        );
        assert_eq!(
            grants.scope_of("chief-of-staff", "tracker", GrantAccess::Write),
            Some(GrantScope::Local)
        );
        // Read and write are kept apart
        assert!(!grants.allows("chief-of-staff", "tracker", GrantAccess::Read));
        assert_eq!(grants.list().len(), 2);

        assert!(grants
            .revoke(
                "chief-of-staff",
                "tracker",
                GrantAccess::Write,
                GrantScope::Local
            )
            .unwrap());
        assert!(!grants.allows("chief-of-staff", "tracker", GrantAccess::Write));
        assert!(!grants
            .revoke(
                "chief-of-staff",
                "tracker",
                GrantAccess::Write,
                GrantScope::Local
            )
            .unwrap());
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
