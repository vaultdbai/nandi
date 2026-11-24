//! The gold copy upgrade for a person coming from Nandi 0.2: an agent's
//! file is `agent.md` ([`AGENT_FILE`]), no longer `skill.md`.
//!
//! At startup, before anything else reads the agents, [`upgrade`]:
//!
//! 1. Renames `skill.md` to `agent.md` in every folder where an agent may
//!    live: the person's own agents and the installed copies of agents and
//!    packs (`agent_locator::folders`). The file's bytes don't change. A
//!    folder that already has an `agent.md` keeps it, and its `skill.md` is
//!    removed (git history keeps it).
//! 2. Re-records each installed copy in `installed.yaml`. Its tree hash
//!    names the files installed from the signed release. When the files on
//!    disk are exactly those files with only this rename, the new tree hash
//!    is recorded, so the copy still counts as unmodified. A copy changed
//!    in any other way keeps its old hash and still shows as modified: the
//!    upgrade never vouches for files nobody signed.
//! 3. Renames the agent file's entries in `shipped.yaml`, so an unedited
//!    Nandi still gets this build's updates and an edited one keeps them.
//! 4. Commits all of it once, [`COMMIT_MESSAGE`].
//!
//! Every step is idempotent, so a start cut short is finished by the next
//! one: a folder whose `skill.md` is still in the last commit is committed
//! again with the others. A repo with nothing to rename does nothing.

use std::path::Path;

use super::agent_locator;
use super::layout::AGENT_FILE;
use super::vendor_repo::{tree_hash, FileSet};
use super::{RepoLayout, Repos};
use crate::services::domain::installed::InstalledFile;
use crate::services::repo_service::GitService;
use crate::services::shipped_files::ShippedManifest;

/// An agent's file in Nandi 0.2 and before
pub const OLD_FILE: &str = "skill.md";

/// The subject of the upgrade's commit
pub const COMMIT_MESSAGE: &str = "Upgrade agents to agent.md";

/// What [`upgrade`] did
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Upgraded {
    /// Agent folders whose `skill.md` became `agent.md`, relative
    pub renamed: Vec<String>,
    /// Installed packs and agents re-recorded as unmodified
    pub rerecorded: Vec<String>,
    /// The commit, `None` when git already had all of it
    pub commit: Option<String>,
}

impl Upgraded {
    fn commit_message(&self) -> String {
        let mut message = format!(
            "{COMMIT_MESSAGE}\n\nAn agent's file is now {AGENT_FILE}. The text of each file is \
             unchanged."
        );
        if !self.renamed.is_empty() {
            message.push_str(&format!("\n\nRenamed in: {}.", self.renamed.join(", ")));
        }
        if !self.rerecorded.is_empty() {
            message.push_str(&format!(
                "\n\nStill as installed from their signed releases: {}.",
                self.rerecorded.join(", ")
            ));
        }
        message
    }
}

/// Upgrades a gold copy Nandi 0.2 wrote, if it is one, and commits it.
pub fn upgrade(layout: &RepoLayout) -> Result<Option<Upgraded>, String> {
    if !layout.agents_dir.is_dir() {
        return Ok(None);
    }
    let head = HeadFiles::open(&layout.root);
    let mut done = Upgraded::default();
    let mut paths: Vec<String> = Vec::new();
    for place in agent_locator::folders(layout) {
        let old = place.dir.join(OLD_FILE);
        let new = place.dir.join(AGENT_FILE);
        let renamed = rename(&old, &new)?;
        let (Some(old_rel), Some(new_rel)) = (layout.relative(&old), layout.relative(&new)) else {
            continue;
        };
        // Renamed now, or by a start that stopped before its commit
        if renamed || head.has(&old_rel) {
            if let Some(dir) = layout.relative(&place.dir) {
                done.renamed.push(dir);
            }
            paths.extend([old_rel, new_rel]);
        }
    }
    if paths.is_empty() {
        return Ok(None);
    }
    // A record that can't be read is left for loading to report: the
    // renamed agents still load, and their copies show as modified
    done.rerecorded = rerecord_installed(layout).unwrap_or_else(|e| {
        log::warn!("Not re-recording the installed copies: {e}");
        Vec::new()
    });
    rename_shipped(layout)?;
    for path in [layout.installed_path(), layout.shipped_path()] {
        paths.extend(layout.relative(&path));
    }
    done.commit =
        GitService::new(&layout.root).commit_files(&paths, &done.commit_message(), None)?;
    log::info!(
        "{COMMIT_MESSAGE}: renamed in {:?}, re-recorded {:?}",
        done.renamed,
        done.rerecorded
    );
    Ok(Some(done))
}

/// `skill.md` becomes `agent.md`. Returns whether it changed anything.
fn rename(old: &Path, new: &Path) -> Result<bool, String> {
    if !old.is_file() {
        return Ok(false);
    }
    let failed = |e: std::io::Error| format!("Couldn't rename {}: {e}", old.display());
    if new.is_file() {
        log::warn!(
            "{} has both {OLD_FILE} and {AGENT_FILE}: keeping {AGENT_FILE}",
            old.parent().unwrap_or(old).display()
        );
        std::fs::remove_file(old).map_err(failed)?;
    } else {
        std::fs::rename(old, new).map_err(failed)?;
    }
    Ok(true)
}

/// Records the new tree hash of each installed copy whose files are
/// exactly the ones it was installed with, but for the rename. Returns the
/// IDs re-recorded.
fn rerecord_installed(layout: &RepoLayout) -> Result<Vec<String>, String> {
    let vendor = Repos::new(layout.clone()).vendor;
    let mut installed: InstalledFile = vendor.load_installed().map_err(String::from)?;
    let mut rerecorded = Vec::new();
    for pack in &mut installed.packs {
        let dir = layout.pack_dir(pack.id.as_str());
        if let Some(hash) = renamed_hash(
            &vendor.read(&dir).unwrap_or_default(),
            &pack.tree_hash,
            pack_agent_file,
        ) {
            pack.tree_hash = hash;
            rerecorded.push(pack.id.to_string());
        }
    }
    for agent in &mut installed.agents {
        let dir = layout.vendored_agent_dir(agent.publisher.as_str(), agent.id.as_str());
        if let Some(hash) = renamed_hash(
            &vendor.read(&dir).unwrap_or_default(),
            &agent.tree_hash,
            |path| path == AGENT_FILE,
        ) {
            agent.tree_hash = hash;
            rerecorded.push(agent.id.to_string());
        }
    }
    if !rerecorded.is_empty() {
        vendor.save_installed(&installed).map_err(String::from)?;
    }
    Ok(rerecorded)
}

/// `agents/<id>/agent.md` in a pack
fn pack_agent_file(path: &str) -> bool {
    path.strip_prefix("agents/")
        .and_then(|rest| rest.strip_suffix(&format!("/{AGENT_FILE}")))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

/// The tree hash of `files` when they differ from the recorded hash only
/// by the rename: the agent files (`is_agent_file`) named `skill.md` again
/// hash to `recorded`. `None` when the record already matches, or when
/// anything else changed.
pub fn renamed_hash(
    files: &FileSet,
    recorded: &str,
    is_agent_file: impl Fn(&str) -> bool,
) -> Option<String> {
    let now = tree_hash(files);
    if now == recorded {
        return None;
    }
    let as_installed: FileSet = files
        .iter()
        .map(|(path, bytes)| {
            let path = match path.strip_suffix(AGENT_FILE) {
                Some(dir) if is_agent_file(path) => format!("{dir}{OLD_FILE}"),
                _ => path.clone(),
            };
            (path, bytes.clone())
        })
        .collect();
    (tree_hash(&as_installed) == recorded).then_some(now)
}

/// `.../skill.md` becomes `.../agent.md` in `shipped.yaml`'s record. A
/// gold copy without one (from Nandi 0.1.0) has nothing to rename.
fn rename_shipped(layout: &RepoLayout) -> Result<(), String> {
    let Some(mut manifest) = ShippedManifest::load(layout)? else {
        return Ok(());
    };
    let before = manifest.clone();
    for record in [&mut manifest.files, &mut manifest.kept] {
        let old: Vec<String> = record
            .keys()
            .filter(|path| is_old_agent_file(path))
            .cloned()
            .collect();
        for path in old {
            let hash = record.remove(&path).expect("listed above");
            let new = format!("{}{AGENT_FILE}", path.trim_end_matches(OLD_FILE));
            record.entry(new).or_insert(hash);
        }
    }
    if manifest != before {
        manifest.save(layout)?;
    }
    Ok(())
}

/// `.nandi/agents/<id>/skill.md`, not a playbook named `skill.md`
fn is_old_agent_file(path: &str) -> bool {
    path.strip_suffix(OLD_FILE)
        .is_some_and(|dir| dir.ends_with('/') && !dir.ends_with("/skills/"))
}

/// The paths in the gold copy's last commit, to finish a start that
/// stopped before it committed
struct HeadFiles(Option<git2::Repository>);

impl HeadFiles {
    fn open(root: &Path) -> Self {
        Self(git2::Repository::open(root).ok())
    }

    fn has(&self, relative: &str) -> bool {
        let Some(repo) = &self.0 else {
            return false;
        };
        repo.head()
            .and_then(|head| head.peel_to_tree())
            .and_then(|tree| tree.get_path(Path::new(relative)))
            .is_ok()
    }
}

#[cfg(test)]
#[path = "from_0_2_tests.rs"]
mod tests;
