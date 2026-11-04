//! Triggers: `<triggers_dir>/<id>.yaml`. Secrets are never written here;
//! the files name their keyring entry.

use std::path::PathBuf;
use std::sync::Arc;

use super::fs_atomic::{remove_if_exists, write_if_changed};
use super::git::GitIndex;
use super::{
    file_stem, read_text, yaml_files, Loaded, RepoError, RepoLayout, RepoResult, Repository,
};
use crate::services::domain::{EntityKind, Trigger, TriggerId};

#[derive(Clone)]
pub struct TriggerRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

impl TriggerRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    pub fn path(&self, id: &str) -> PathBuf {
        self.layout.trigger_path(id)
    }

    fn read(&self, path: &std::path::Path) -> RepoResult<Trigger> {
        let stem = file_stem(path);
        let trigger = Trigger::from_yaml(&read_text(path, EntityKind::Trigger, &stem)?)?;
        if trigger.id().as_str() != stem {
            return Err(RepoError::IdMismatch {
                path: path.to_path_buf(),
                declared: trigger.id().to_string(),
            });
        }
        Ok(trigger)
    }
}

impl Repository for TriggerRepository {
    type Entity = Trigger;
    type Id = TriggerId;

    fn load(&self, id: &TriggerId) -> RepoResult<Trigger> {
        self.read(&self.path(id.as_str()))
    }

    fn load_all(&self) -> Loaded<Trigger> {
        let mut loaded = Loaded::default();
        for path in yaml_files(&self.layout.triggers_dir) {
            match self.read(&path) {
                Ok(trigger) => loaded.entities.push(trigger),
                Err(e) => {
                    log::warn!("Skipping trigger {}: {e}", path.display());
                    let id = Some(file_stem(&path));
                    loaded
                        .problems
                        .push(e.into_problem(EntityKind::Trigger, id, &path));
                }
            }
        }
        loaded
    }

    fn save(&self, trigger: &Trigger) -> RepoResult<PathBuf> {
        let path = self.path(trigger.id().as_str());
        if write_if_changed(&path, trigger.to_yaml().as_bytes())
            .map_err(|e| RepoError::io(&path, e))?
        {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    fn delete(&self, id: &TriggerId) -> RepoResult<()> {
        let path = self.path(id.as_str());
        if remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_remove(&path);
        }
        Ok(())
    }

    fn exists(&self, id: &TriggerId) -> bool {
        self.path(id.as_str()).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::Repos;

    #[test]
    fn saves_loads_and_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let yaml =
            "id: hook\nname: Hook\nsource: {type: webhook}\ntarget: {type: agent, id: writer}\n";
        let trigger = Trigger::from_yaml(yaml).unwrap();
        repos.triggers.save(&trigger).unwrap();
        assert_eq!(repos.triggers.load(trigger.id()).unwrap(), trigger);
        std::fs::write(repos.triggers.path("broken"), "id: [").unwrap();
        let loaded = repos.triggers.load_all();
        assert_eq!((loaded.entities.len(), loaded.problems.len()), (1, 1));
        repos.triggers.delete(trigger.id()).unwrap();
        repos.triggers.delete(trigger.id()).unwrap();
        assert!(!repos.triggers.exists(trigger.id()));
    }
}
