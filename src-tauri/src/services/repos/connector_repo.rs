//! Connector declarations: `.nandi/connectors.yaml`.

use std::path::PathBuf;
use std::sync::Arc;

use super::fs_atomic::write_if_changed;
use super::git::GitIndex;
use super::{RepoError, RepoLayout, RepoResult, SingletonRepository};
use crate::services::domain::ConnectorManifest;

#[derive(Clone)]
pub struct ConnectorRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

impl ConnectorRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }
}

impl SingletonRepository for ConnectorRepository {
    type Entity = ConnectorManifest;

    fn load(&self) -> RepoResult<ConnectorManifest> {
        let path = self.path();
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(ConnectorManifest::from_yaml(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConnectorManifest::default()),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    fn save(&self, manifest: &ConnectorManifest) -> RepoResult<PathBuf> {
        let path = self.path();
        if write_if_changed(&path, manifest.to_yaml().as_bytes())
            .map_err(|e| RepoError::io(&path, e))?
        {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    fn path(&self) -> PathBuf {
        self.layout.connectors_path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::Repos;

    #[test]
    fn round_trips_through_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        assert!(repos.connectors.load().unwrap().connectors().is_empty());
        let manifest =
            ConnectorManifest::from_yaml("connectors:\n  - {id: gh, kind: github}\n").unwrap();
        repos.connectors.save(&manifest).unwrap();
        assert_eq!(repos.connectors.load().unwrap(), manifest);
    }
}
