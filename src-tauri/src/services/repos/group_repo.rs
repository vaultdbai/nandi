//! Groups this Nandi is in (protocol 7): `.nandi/groups/<group-id>.md`,
//! the copy of each group's signed policy and members, and
//! `.nandi/groups/access.yaml`, my access level per group
//! (`domain::group`). Only `services::groups` writes them, from what the
//! relay says; they aren't projected into SQLite. Enforcement reads them
//! from disk for every request.

use std::path::PathBuf;
use std::sync::Arc;

use super::fs_atomic::{remove_if_exists, write_if_changed, TEMP_MARKER};
use super::git::GitIndex;
use super::{RepoError, RepoLayout, RepoResult};
use crate::services::domain::group::{check_group_id, GroupAccess, GroupCopy};

/// The access levels file in `.nandi/groups/`
pub const ACCESS_FILE: &str = "access.yaml";

#[derive(Clone)]
pub struct GroupRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

/// A copy that doesn't read, by its group ID (it blocks that group's work)
#[derive(Debug, Clone, PartialEq)]
pub struct BrokenCopy {
    pub group_id: String,
    pub error: String,
}

impl GroupRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    pub fn dir(&self) -> PathBuf {
        self.layout.groups_dir()
    }

    pub fn path(&self, group_id: &str) -> PathBuf {
        self.dir().join(format!("{group_id}.md"))
    }

    pub fn access_path(&self) -> PathBuf {
        self.dir().join(ACCESS_FILE)
    }

    /// One group's copy; `None` when there is none
    pub fn load(&self, group_id: &str) -> RepoResult<Option<GroupCopy>> {
        check_group_id(group_id).map_err(RepoError::Domain)?;
        let path = self.path(group_id);
        match std::fs::read_to_string(&path) {
            Ok(text) => GroupCopy::parse(group_id, &text)
                .map(Some)
                .map_err(RepoError::Domain),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    /// Every copy, sorted by group ID, with the ones that don't read kept
    /// apart. No folder: none.
    pub fn load_all(&self) -> RepoResult<(Vec<GroupCopy>, Vec<BrokenCopy>)> {
        let dir = self.dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()))
            }
            Err(e) => return Err(RepoError::io(&dir, e)),
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                (!name.contains(TEMP_MARKER))
                    .then(|| name.strip_suffix(".md").map(str::to_string))
                    .flatten()
            })
            .collect();
        ids.sort();
        let (mut copies, mut broken) = (Vec::new(), Vec::new());
        for id in ids {
            match self.load(&id) {
                Ok(Some(copy)) => copies.push(copy),
                Ok(None) => {}
                Err(e) => broken.push(BrokenCopy {
                    group_id: id,
                    error: e.to_string(),
                }),
            }
        }
        Ok((copies, broken))
    }

    /// Writes (and stages) a group's copy if it changed
    pub fn save(&self, copy: &GroupCopy) -> RepoResult<PathBuf> {
        check_group_id(&copy.group_id).map_err(RepoError::Domain)?;
        let path = self.path(&copy.group_id);
        self.write(&path, &copy.render())?;
        Ok(path)
    }

    /// Removes (and stages the removal of) a group's copy
    pub fn delete(&self, group_id: &str) -> RepoResult<()> {
        check_group_id(group_id).map_err(RepoError::Domain)?;
        let path = self.path(group_id);
        if remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_remove(&path);
            log::info!("Removed group copy {}", path.display());
        }
        Ok(())
    }

    /// My access levels (none set: every group is "Ask first")
    pub fn load_access(&self) -> RepoResult<GroupAccess> {
        let path = self.access_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => GroupAccess::parse(&text).map_err(RepoError::Domain),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GroupAccess::default()),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    pub fn save_access(&self, access: &GroupAccess) -> RepoResult<PathBuf> {
        let path = self.access_path();
        self.write(&path, &access.render())?;
        Ok(path)
    }

    fn write(&self, path: &std::path::Path, text: &str) -> RepoResult<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RepoError::io(dir, e))?;
        }
        if write_if_changed(path, text.as_bytes()).map_err(|e| RepoError::io(path, e))? {
            self.git.stage_add(path);
            log::info!("Saved {}", path.display());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::group::{AccessLevel, CopyMember};
    use crate::services::repos::Repos;
    use nandi_protocol::GroupRole;

    fn copy(id: &str) -> GroupCopy {
        GroupCopy {
            group_id: id.into(),
            name: "Finance".into(),
            version: 1,
            author: "alice-1".into(),
            signature: "sig".into(),
            updated_at: 1.0,
            members: vec![CopyMember {
                instance_id: "alice-1".into(),
                role: GroupRole::Admin,
            }],
            text: "---\nrequests:\n  approval: always\n---\nWhy.\n".into(),
        }
    }

    #[test]
    fn copies_are_written_read_back_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let groups = &repos.groups;
        assert_eq!(groups.load_all().unwrap(), (Vec::new(), Vec::new()));
        let path = groups.save(&copy("g1")).unwrap();
        assert!(path.ends_with(".nandi/groups/g1.md") || path.ends_with(".nandi\\groups\\g1.md"));
        groups.save(&copy("g2")).unwrap();
        std::fs::write(groups.path("broken"), "---\ngroup: other\n---\n").unwrap();
        groups.save_access(&GroupAccess::default()).unwrap();

        let (copies, broken) = groups.load_all().unwrap();
        assert_eq!(copies, vec![copy("g1"), copy("g2")]);
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].group_id, "broken");
        assert_eq!(groups.load("g1").unwrap(), Some(copy("g1")));

        groups.delete("g1").unwrap();
        assert_eq!(groups.load("g1").unwrap(), None);
        groups.delete("g1").unwrap();
        assert!(groups.load("../x").is_err());
    }

    #[test]
    fn access_levels_are_kept_in_one_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let groups = &repos.groups;
        assert_eq!(groups.load_access().unwrap(), GroupAccess::default());
        let mut access = GroupAccess::default();
        access.set("g1", AccessLevel::FullyAutomated).unwrap();
        groups.save_access(&access).unwrap();
        assert_eq!(
            groups.load_access().unwrap().level("g1"),
            AccessLevel::FullyAutomated
        );
        let text = std::fs::read_to_string(groups.access_path()).unwrap();
        assert!(text.contains("g1: fully-automated"), "{text}");
    }
}
