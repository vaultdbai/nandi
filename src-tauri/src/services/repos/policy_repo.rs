//! Policies: `.nandi/policies/<id>.md`, Markdown with YAML frontmatter
//! (wave CD). Studio edits them; the ask box reads them as context; the
//! ones with `applies_to` are enforced (`domain::policy`). They aren't
//! projected into SQLite.

use std::path::PathBuf;
use std::sync::Arc;

use super::fs_atomic::write_if_changed;
use super::git::GitIndex;
use super::{RepoError, RepoLayout, RepoResult};
use crate::services::domain::policy::{PolicySet, PolicySummary};

#[derive(Clone)]
pub struct PolicyRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

impl PolicyRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    pub fn dir(&self) -> PathBuf {
        self.layout.policies_dir()
    }

    pub fn path(&self, id: &str) -> PathBuf {
        self.layout.policy_path(id)
    }

    /// `<id>.md` as it is on disk, `None` when there's no such file
    pub fn read(&self, id: &str) -> RepoResult<Option<String>> {
        let path = self.path(id);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    /// Every `<id>.md`, parsed, with the broken ones kept apart (they still
    /// block, `PolicySet::blocking`). No folder: no policies.
    pub fn load_all(&self) -> RepoResult<PolicySet> {
        let files = self.files()?;
        Ok(PolicySet::parse(
            files
                .iter()
                .map(|(id, content)| (id.as_str(), content.as_str())),
        ))
    }

    /// Every `<id>.md` by id, name and whether it parses, sorted by id
    pub fn list(&self) -> RepoResult<Vec<PolicySummary>> {
        Ok(self
            .files()?
            .iter()
            .map(|(id, content)| PolicySummary::of(id, content))
            .collect())
    }

    /// `(id, content)` of every `<id>.md`, sorted by id
    fn files(&self) -> RepoResult<Vec<(String, String)>> {
        let dir = self.dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(RepoError::io(&dir, e)),
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let content = std::fs::read_to_string(&path).map_err(|e| RepoError::io(&path, e))?;
            files.push((id, content));
        }
        files.sort();
        Ok(files)
    }

    /// Writes (and stages) the file if it changed; the caller validates
    pub fn save(&self, id: &str, text: &str) -> RepoResult<PathBuf> {
        let path = self.path(id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RepoError::io(dir, e))?;
        }
        if write_if_changed(&path, text.as_bytes()).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_add(&path);
            log::info!("Saved policy {}", path.display());
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::Repos;

    #[test]
    fn lists_every_policy_by_id_with_its_name_and_whether_it_parses() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        assert!(repos.policies.list().unwrap().is_empty(), "no folder");

        repos
            .policies
            .save(
                "no-shell",
                "---
name: No shell
applies_to: ['*']
tools: { deny: [shell] }
---
",
            )
            .unwrap();
        repos
            .policies
            .save(
                "broken",
                "---
tools: { nope: 1 }
---
",
            )
            .unwrap();
        repos
            .policies
            .save(
                "tone",
                "Be kind.
",
            )
            .unwrap();
        std::fs::write(repos.policies.dir().join("notes.txt"), "not a policy").unwrap();

        let listed: Vec<(String, String, bool)> = repos
            .policies
            .list()
            .unwrap()
            .into_iter()
            .map(|p| (p.id, p.name, p.valid))
            .collect();
        assert_eq!(
            listed,
            [
                ("broken".into(), "broken".into(), false),
                ("no-shell".into(), "No shell".into(), true),
                ("tone".into(), "tone".into(), true),
            ]
        );
    }
}
