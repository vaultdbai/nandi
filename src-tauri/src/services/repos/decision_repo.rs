//! The decisions made on the Inbox (`domain::decision`):
//! `.nandi/decisions/<yyyy-mm>.md`, one file per month, only ever
//! appended to. Written by `services::inbox::audit` for every decision,
//! from a button or an Ask chat; not projected into SQLite. Each write is
//! atomic and staged, like every gold-copy file.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::fs_atomic::write_if_changed;
use super::git::GitIndex;
use super::{RepoError, RepoLayout, RepoResult};
use crate::services::domain::decision::{is_month, month_header, DecisionEntry};
use crate::services::domain::DomainError;

/// One append at a time (read, add, write), across every repository clone
static APPEND_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct DecisionRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

impl DecisionRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    pub fn dir(&self) -> PathBuf {
        self.layout.decisions_dir()
    }

    /// `.nandi/decisions/<month>.md`
    pub fn path(&self, month: &str) -> RepoResult<PathBuf> {
        if !is_month(month) {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "'{month}' isn't a month (yyyy-mm)"
            ))));
        }
        Ok(self.dir().join(format!("{month}.md")))
    }

    /// Adds `entry` at the end of its month's file (made with its heading
    /// the first time) and stages it. Returns the file.
    pub fn append(&self, entry: &DecisionEntry) -> RepoResult<PathBuf> {
        let month = entry.month();
        let path = self.path(&month)?;
        let _guard = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => month_header(&month),
            Err(e) => return Err(RepoError::io(&path, e)),
        };
        if !text.is_empty() && !text.ends_with("\n\n") {
            text.push_str(if text.ends_with('\n') { "\n" } else { "\n\n" });
        }
        text.push_str(&entry.render());
        if write_if_changed(&path, text.as_bytes()).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    /// One month's file as written; `None` when nothing was decided then
    #[cfg(test)]
    pub fn read(&self, month: &str) -> RepoResult<Option<String>> {
        let path = self.path(month)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::Repos;

    fn entry(at: i64, decision: &str) -> DecisionEntry {
        DecisionEntry {
            at,
            decided_by: "Dev".into(),
            item_kind: "approval".into(),
            item_id: "a1".into(),
            subject: None,
            decision: decision.into(),
            note: None,
            conversation_id: None,
        }
    }

    #[test]
    fn entries_are_appended_to_their_months_file_and_staged() {
        let tmp = tempfile::tempdir().unwrap();
        let git = git2::Repository::init(tmp.path()).unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let decisions = &repos.decisions;
        assert_eq!(decisions.read("2026-11").unwrap(), None);

        // 2026-11-08 and 2026-11-09, then 2026-12-01
        let path = decisions
            .append(&entry(1_794_148_325_000, "approved"))
            .unwrap();
        decisions
            .append(&entry(1_794_234_725_000, "refused"))
            .unwrap();
        decisions.append(&entry(1_796_083_200_000, "done")).unwrap();
        assert!(path.ends_with("2026-11.md"));

        let november = decisions.read("2026-11").unwrap().unwrap();
        assert!(november.starts_with("# Decisions, 2026-11\n"), "{november}");
        let approved = november.find("**Decision:** approved").unwrap();
        let refused = november.find("**Decision:** refused").unwrap();
        assert!(approved < refused, "oldest first: {november}");
        assert!(!november.contains("done"), "{november}");
        assert!(decisions
            .read("2026-12")
            .unwrap()
            .unwrap()
            .contains("**Decision:** done"));

        let mut index = git.index().unwrap();
        index.read(true).unwrap();
        assert!(index
            .get_path(std::path::Path::new(".nandi/decisions/2026-11.md"), 0)
            .is_some());
        assert!(decisions.read("../x").is_err());
    }
}
