//! Connector declarations (`.nandi/connectors.yaml`) and connector packages
//! (`.nandi/connectors/<id>/connector.md`).

use std::path::PathBuf;
use std::sync::Arc;

use super::fs_atomic::{remove_if_exists, write_if_changed};
use super::git::GitIndex;
use super::layout::CONNECTOR_FILE;
use super::{RepoError, RepoLayout, RepoResult, SingletonRepository};
use crate::services::domain::connector_package::ConnectorPackage;
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

    /// The folder holding the package's `connector.md`
    pub fn package_dir(&self, id: &str) -> PathBuf {
        self.layout.connector_package_dir(id)
    }

    /// `connector.md` as it is on disk, `None` when there's none
    pub fn read_package(&self, id: &str) -> RepoResult<Option<String>> {
        let path = self.package_dir(id).join(CONNECTOR_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    /// The parsed package, `None` when the connector isn't one
    pub fn load_package(&self, id: &str) -> RepoResult<Option<ConnectorPackage>> {
        match self.read_package(id)? {
            Some(text) => Ok(Some(ConnectorPackage::parse(id, &text)?)),
            None => Ok(None),
        }
    }

    pub fn save_package(&self, package: &ConnectorPackage) -> RepoResult<PathBuf> {
        let path = self.package_dir(package.id.as_str()).join(CONNECTOR_FILE);
        if write_if_changed(&path, package.to_markdown().as_bytes())
            .map_err(|e| RepoError::io(&path, e))?
        {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    pub fn delete_package(&self, id: &str) -> RepoResult<bool> {
        let dir = self.package_dir(id);
        let path = dir.join(CONNECTOR_FILE);
        let removed = remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))?;
        if removed {
            self.git.stage_remove(&path);
            let _ = std::fs::remove_dir(&dir);
        }
        Ok(removed)
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

    #[test]
    fn packages_live_in_their_own_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        assert!(repos.connectors.load_package("fetch").unwrap().is_none());
        let package = ConnectorPackage::parse(
            "fetch",
            "---
name: Fetch
description: Read pages.
version: 1.0.0
run: uvx mcp-server-fetch==2025.4.7
hosts: [example.com]
tools:
  - {name: fetch, description: Fetch a page, access: read}
---
Read pages.
",
        )
        .unwrap();
        let path = repos.connectors.save_package(&package).unwrap();
        assert!(path.ends_with(".nandi/connectors/fetch/connector.md"));
        assert_eq!(
            repos.connectors.load_package("fetch").unwrap(),
            Some(package)
        );
        assert!(repos.connectors.delete_package("fetch").unwrap());
        assert!(repos.connectors.load_package("fetch").unwrap().is_none());
    }
}
