//! Stages gold-copy changes in git. Committing and pushing are explicit
//! ("Sync now"); every write is staged so `git status` shows exactly what
//! the app changed.
//!
//! Only the repo at the layout root is used, never one found further up,
//! so an unrelated repo around `~/.nandi` is left alone. Failures are logged:
//! the file on disk is what matters.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct GitIndex {
    root: PathBuf,
}

impl GitIndex {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn stage_add(&self, path: &Path) {
        self.update(path, |index, relative| index.add_path(relative));
    }

    pub fn stage_remove(&self, path: &Path) {
        self.update(path, |index, relative| index.remove_path(relative));
    }

    /// Stages everything under a folder (new, changed, and removed files).
    pub fn stage_add_dir(&self, dir: &Path) {
        self.update(dir, |index, relative| {
            let spec = relative.to_string_lossy().replace('\\', "/");
            index.remove_dir(relative, 0)?;
            index.add_all([spec.as_str()], git2::IndexAddOption::DEFAULT, None)
        });
    }

    /// Stages the removal of everything under a folder.
    pub fn stage_remove_dir(&self, dir: &Path) {
        self.update(dir, |index, relative| index.remove_dir(relative, 0));
    }

    fn update(
        &self,
        path: &Path,
        change: impl FnOnce(&mut git2::Index, &Path) -> Result<(), git2::Error>,
    ) {
        let Ok(repo) = git2::Repository::open(&self.root) else {
            return; // not a repo (e.g. not cloned yet)
        };
        let Some(workdir) = repo.workdir().map(PathBuf::from) else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&workdir) else {
            return; // outside the repo
        };
        let result = repo.index().and_then(|mut index| {
            change(&mut index, relative)?;
            index.write()
        });
        if let Err(e) = result {
            log::warn!("Failed to stage {}: {e}", relative.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(repo: &git2::Repository, path: &str) -> bool {
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        index.get_path(Path::new(path), 0).is_some()
    }

    #[test]
    fn stages_adds_and_removals_inside_the_repo_only() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let git = GitIndex::new(dir.path());
        let file = dir.path().join(".nandi/agents/a/skill.md");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x").unwrap();

        git.stage_add(&file);
        assert!(staged(&repo, ".nandi/agents/a/skill.md"));
        std::fs::remove_dir_all(dir.path().join(".nandi/agents/a")).unwrap();
        git.stage_remove_dir(&dir.path().join(".nandi/agents/a"));
        assert!(!staged(&repo, ".nandi/agents/a/skill.md"));

        // Outside the repo, or no repo at all: nothing happens
        let other = tempfile::tempdir().unwrap();
        git.stage_add(&other.path().join("x"));
        GitIndex::new(other.path()).stage_add(&other.path().join("x"));
    }
}
