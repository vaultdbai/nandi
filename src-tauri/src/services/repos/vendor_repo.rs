//! Marketplace files: vendored folders under `.nandi/agents/marketplace/`
//! and `.nandi/installed.yaml`. The only writer of either.
//!
//! A folder is replaced as a whole: the new files are written to a hidden
//! staging folder next to it (`.<name>.nandi-tmp-<uuid>`, which the watcher
//! and the agent locator ignore), then renamed into place. The watcher sees
//! the folder appear in one step instead of file by file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::fs_atomic::{
    content_hash, move_to_trash, remove_if_exists, write_if_changed, TEMP_MARKER,
};
use super::git::GitIndex;
use super::{RepoError, RepoLayout, RepoResult};
use crate::services::domain::installed::InstalledFile;
use crate::services::domain::{AgentId, DomainError, EntityKind};

/// A folder's files: `/`-separated relative path → contents.
pub type FileSet = BTreeMap<String, Vec<u8>>;

/// Largest folder the marketplace will vendor (all files together)
pub const MAX_FOLDER_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct VendorRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

/// A hash of every path and its contents, for "has this folder changed".
pub fn tree_hash(files: &FileSet) -> String {
    let mut listing = String::new();
    for (path, bytes) in files {
        listing.push_str(path);
        listing.push('\0');
        listing.push_str(&content_hash(bytes));
        listing.push('\n');
    }
    content_hash(listing.as_bytes())
}

/// Whether `relative` is safe to join onto a folder: plain names separated
/// by `/`, no `..`, no hidden names except a `.nandi` folder.
pub fn is_safe_relative(relative: &str) -> bool {
    !relative.is_empty()
        && !relative.starts_with('/')
        && relative.split('/').all(|part| {
            !part.is_empty()
                && part != ".."
                && (!part.starts_with('.') || part == ".nandi")
                && !part.contains(['\\', ':'])
                && !part.contains(TEMP_MARKER)
                && part.chars().all(|c| !c.is_control())
        })
}

fn staging_name(dir: &Path, tag: &str) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dir.with_file_name(format!(
        ".{name}{TEMP_MARKER}{tag}{}",
        uuid::Uuid::new_v4().simple()
    ))
}

impl VendorRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    // -- installed.yaml -----------------------------------------------------

    /// `.nandi/installed.yaml`; missing means nothing is installed.
    pub fn load_installed(&self) -> RepoResult<InstalledFile> {
        let path = self.layout.installed_path();
        match fs::read_to_string(&path) {
            Ok(content) => Ok(InstalledFile::from_yaml(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(InstalledFile::default()),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    /// Writes installed.yaml (removing it once nothing is installed) and
    /// stages it.
    pub fn save_installed(&self, file: &InstalledFile) -> RepoResult<()> {
        let path = self.layout.installed_path();
        let yaml = file.to_yaml();
        let changed = if yaml.is_empty() {
            remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))?
        } else {
            write_if_changed(&path, yaml.as_bytes()).map_err(|e| RepoError::io(&path, e))?
        };
        if changed {
            if path.exists() {
                self.git.stage_add(&path);
            } else {
                self.git.stage_remove(&path);
            }
        }
        Ok(())
    }

    // -- Folders ------------------------------------------------------------

    fn ensure_vendored(&self, dir: &Path) -> RepoResult<()> {
        if !self.layout.is_vendored(dir) || dir == self.layout.marketplace_dir() {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "{} is not a marketplace folder",
                dir.display()
            ))));
        }
        Ok(())
    }

    /// Writes `files` into a new hidden staging folder next to `dest`.
    fn stage(&self, dest: &Path, files: &FileSet) -> RepoResult<PathBuf> {
        let total: usize = files.values().map(Vec::len).sum();
        if total > MAX_FOLDER_BYTES {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "The folder is {} MB; the limit is {} MB",
                total / (1024 * 1024),
                MAX_FOLDER_BYTES / (1024 * 1024)
            ))));
        }
        let parent = dest.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| RepoError::io(parent, e))?;
        let staging = staging_name(dest, "");
        let result = (|| {
            for (relative, bytes) in files {
                if !is_safe_relative(relative) {
                    return Err(RepoError::Domain(DomainError::Rule(format!(
                        "Refusing to write '{relative}': not a plain relative path"
                    ))));
                }
                let path = staging.join(relative);
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir).map_err(|e| RepoError::io(dir, e))?;
                }
                fs::write(&path, bytes).map_err(|e| RepoError::io(&path, e))?;
            }
            fs::create_dir_all(&staging).map_err(|e| RepoError::io(&staging, e))?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
        Ok(staging)
    }

    /// Swaps a staged folder into `dest`, replacing what's there.
    fn swap_in(&self, staging: &Path, dest: &Path) -> RepoResult<()> {
        let old = dest.exists().then(|| staging_name(dest, "old-"));
        if let Some(old) = &old {
            if let Err(e) = fs::rename(dest, old) {
                let _ = fs::remove_dir_all(staging);
                return Err(RepoError::io(dest, e));
            }
        }
        if let Err(e) = fs::rename(staging, dest) {
            if let Some(old) = &old {
                let _ = fs::rename(old, dest);
            }
            let _ = fs::remove_dir_all(staging);
            return Err(RepoError::io(dest, e));
        }
        if let Some(old) = old {
            if let Err(e) = fs::remove_dir_all(&old) {
                log::warn!("Failed to remove {}: {e}", old.display());
            }
        }
        Ok(())
    }

    /// Replaces the vendored folder `dest` with exactly `files`, and stages
    /// it.
    pub fn put(&self, dest: &Path, files: &FileSet) -> RepoResult<()> {
        self.ensure_vendored(dest)?;
        let staging = self.stage(dest, files)?;
        self.swap_in(&staging, dest)?;
        self.git.stage_add_dir(dest);
        log::info!("Vendored {} files into {}", files.len(), dest.display());
        Ok(())
    }

    /// Moves a vendored folder to the trash and stages the removal. Empty
    /// publisher folders left behind are removed.
    pub fn remove(&self, dir: &Path) -> RepoResult<PathBuf> {
        self.ensure_vendored(dir)?;
        let target =
            move_to_trash(dir, &self.layout.trash_dir).map_err(|e| RepoError::io(dir, e))?;
        self.git.stage_remove_dir(dir);
        if let Some(parent) = dir.parent() {
            if parent != self.layout.marketplace_dir()
                && fs::read_dir(parent).is_ok_and(|mut e| e.next().is_none())
            {
                let _ = fs::remove_dir(parent);
            }
        }
        log::info!("Moved {} to {}", dir.display(), target.display());
        Ok(target)
    }

    /// Every regular file under `dir` (symlinks and staging files skipped).
    pub fn read(&self, dir: &Path) -> RepoResult<FileSet> {
        fn walk(base: &Path, dir: &Path, files: &mut FileSet) -> RepoResult<()> {
            let entries = fs::read_dir(dir).map_err(|e| RepoError::io(dir, e))?;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.contains(TEMP_MARKER) || name == ".git" {
                    continue;
                }
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    walk(base, &path, files)?;
                } else if meta.is_file() {
                    let relative = path
                        .strip_prefix(base)
                        .unwrap_or(&path)
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    let bytes = fs::read(&path).map_err(|e| RepoError::io(&path, e))?;
                    files.insert(relative, bytes);
                }
            }
            Ok(())
        }
        let mut files = FileSet::new();
        walk(dir, dir, &mut files)?;
        Ok(files)
    }

    /// Copies a vendored agent folder into a new local agent folder
    /// (`.nandi/agents/<id>/`) and stages it. Fails if that folder exists.
    pub fn copy_to_local(&self, src: &Path, id: &AgentId) -> RepoResult<PathBuf> {
        self.ensure_vendored(src)?;
        let dest = self.layout.agent_dir(id.as_str());
        if dest.exists() {
            return Err(RepoError::AlreadyExists {
                kind: EntityKind::Agent,
                id: id.to_string(),
            });
        }
        let files = self.read(src)?;
        let staging = self.stage(&dest, &files)?;
        if let Err(e) = fs::rename(&staging, &dest) {
            let _ = fs::remove_dir_all(&staging);
            return Err(RepoError::io(&dest, e));
        }
        self.git.stage_add_dir(&dest);
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::Repos;

    fn files(pairs: &[(&str, &str)]) -> FileSet {
        pairs
            .iter()
            .map(|(p, c)| (p.to_string(), c.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn replaces_folders_whole_and_stages_them() {
        let tmp = tempfile::tempdir().unwrap();
        git2::Repository::init(tmp.path()).unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let dest = repos.layout().vendored_agent_dir("alice", "arxiv");

        let v1 = files(&[("skill.md", "one"), ("workflows/a.yaml", "a")]);
        repos.vendor.put(&dest, &v1).unwrap();
        assert_eq!(repos.vendor.read(&dest).unwrap(), v1);
        let v2 = files(&[("skill.md", "two")]);
        repos.vendor.put(&dest, &v2).unwrap();
        assert_eq!(repos.vendor.read(&dest).unwrap(), v2, "old files are gone");
        assert_ne!(tree_hash(&v1), tree_hash(&v2));
        let leftovers: Vec<_> = fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no staging folders left");

        let index = git2::Repository::open(tmp.path()).unwrap().index().unwrap();
        assert!(index
            .get_path(
                Path::new(".nandi/agents/marketplace/alice/arxiv/skill.md"),
                0
            )
            .is_some());
        assert!(index
            .get_path(
                Path::new(".nandi/agents/marketplace/alice/arxiv/workflows/a.yaml"),
                0
            )
            .is_none());

        let local = repos
            .vendor
            .copy_to_local(&dest, &AgentId::parse("my-arxiv").unwrap())
            .unwrap();
        assert_eq!(fs::read_to_string(local.join("skill.md")).unwrap(), "two");
        repos.vendor.remove(&dest).unwrap();
        assert!(!dest.exists());
        assert!(
            !dest.parent().unwrap().exists(),
            "empty publisher folder removed"
        );
    }

    #[test]
    fn refuses_unsafe_paths_and_non_vendored_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let dest = repos.layout().pack_dir("p");
        for bad in ["../x", "/abs", "a/../b", ".git/config", "a\\b", "C:x"] {
            assert!(!is_safe_relative(bad), "{bad}");
            assert!(repos.vendor.put(&dest, &files(&[(bad, "x")])).is_err());
        }
        assert!(is_safe_relative(".nandi/manifest.yaml"));
        let local = repos.layout().agent_dir("mine");
        assert!(repos
            .vendor
            .put(&local, &files(&[("skill.md", "x")]))
            .is_err());
        assert!(repos
            .vendor
            .remove(&repos.layout().marketplace_dir())
            .is_err());
    }

    #[test]
    fn installed_file_round_trips_and_disappears_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        assert_eq!(
            repos.vendor.load_installed().unwrap(),
            InstalledFile::default()
        );
        let mut file = InstalledFile::default();
        file.upsert_agent(crate::services::domain::installed::InstalledAgent {
            id: AgentId::parse("arxiv").unwrap(),
            publisher: crate::services::domain::PublisherHandle::parse("alice").unwrap(),
            source: "github.com/alice/nandi-agents".into(),
            git_ref: "v1.0.0".into(),
            commit: "abc".into(),
            path: "agents/arxiv".into(),
            version: "1.0.0".into(),
            installed_at: "2026-10-06T00:00:00Z".into(),
            signature_verified: false,
            registry: None,
            tree_hash: "h".into(),
        })
        .unwrap();
        repos.vendor.save_installed(&file).unwrap();
        assert_eq!(repos.vendor.load_installed().unwrap(), file);
        repos
            .vendor
            .save_installed(&InstalledFile::default())
            .unwrap();
        assert!(!repos.layout().installed_path().exists());
    }
}
