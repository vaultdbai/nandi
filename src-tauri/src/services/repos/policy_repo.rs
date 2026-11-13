//! Policies: `.nandi/policies/<id>.md`, Markdown with YAML frontmatter.
//! Studio edits them; the ask box reads them as context; the
//! ones with `applies_to` are enforced (`domain::policy`). They aren't
//! projected into SQLite.
//!
//! Installed packs may ship policies for their own agents
//! (`.nandi/agents/marketplace/packs/<pack>/policies/<id>.md`, checked by
//! the installer). They apply like the others, as `<pack>/<id>`, and stay
//! read-only: they aren't listed for editing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::fs_atomic::{write_if_changed, TEMP_MARKER};
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

    /// Every `<id>.md` and every installed pack's policy, parsed, with the
    /// broken ones kept apart (they still block, `PolicySet::blocking`). No
    /// folder: no policies.
    pub fn load_all(&self) -> RepoResult<PolicySet> {
        let mut files = self.files()?;
        files.extend(self.pack_files()?);
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
        markdown_files(&self.dir(), "")
    }

    /// `(<pack>/<id>, content)` of every installed pack's policy
    fn pack_files(&self) -> RepoResult<Vec<(String, String)>> {
        let packs = self.layout.marketplace_dir().join(super::layout::PACKS_DIR);
        let Ok(entries) = std::fs::read_dir(&packs) else {
            return Ok(Vec::new());
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.contains(TEMP_MARKER) || !entry.path().is_dir() {
                continue;
            }
            let dir = entry.path().join(super::layout::POLICIES_DIR);
            files.extend(markdown_files(&dir, &format!("{name}/"))?);
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

/// `(<prefix><stem>, content)` of every `*.md` in `dir`, sorted; no folder:
/// none
fn markdown_files(dir: &Path, prefix: &str) -> RepoResult<Vec<(String, String)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(RepoError::io(dir, e)),
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let content = std::fs::read_to_string(&path).map_err(|e| RepoError::io(&path, e))?;
        files.push((format!("{prefix}{stem}"), content));
    }
    files.sort();
    Ok(files)
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

    #[test]
    fn installed_packs_policies_apply_but_are_not_listed_for_editing() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path(), tmp.path().join("trash"));
        let repos = Repos::new(layout.clone());
        let shipped = layout.pack_dir("social-pack").join("policies");
        std::fs::create_dir_all(&shipped).unwrap();
        std::fs::write(
            shipped.join("posting.md"),
            "---\napplies_to: [agent:social-manager]\nbrowser: { refuse: [{ controls: [post] }] }\n---\n",
        )
        .unwrap();
        // A half-written install isn't read
        let staging = layout
            .marketplace_dir()
            .join("packs")
            .join(format!(".x{TEMP_MARKER}1"))
            .join("policies");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("partial.md"), "---\napplies_to: [nope\n").unwrap();
        repos.policies.save("local", "Be kind.\n").unwrap();

        let set = repos.policies.load_all().unwrap();
        let ids: Vec<&str> = set.policies.iter().map(|p| p.id()).collect();
        assert_eq!(ids, ["local", "social-pack/posting"]);
        assert!(set.broken.is_empty());
        let listed: Vec<String> = repos
            .policies
            .list()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(listed, ["local"]);
    }
}
