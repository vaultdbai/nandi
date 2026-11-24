//! The gold copy upgrade for a person coming from Nandi 0.1.0.
//!
//! 0.1.0 shipped the default agent as `chief-of-staff`. It is now Nandi
//! ([`DEFAULT_AGENT`]). At startup, before the scaffolding would seed a
//! fresh Nandi, [`upgrade`]:
//!
//! 1. Makes the Chief of Staff's folder Nandi's: `.nandi/agents/chief-of-staff/`
//!    moves to `.nandi/agents/<DEFAULT_AGENT>/` with every edit the person
//!    made. When both folders exist, the one the person edited is kept
//!    ([`Kept`]).
//! 2. Renames `chief-of-staff` to `nandi` in every definition file the
//!    person owns: grants in `settings.yaml`, triggers, workflows and other
//!    agents' frontmatter. Only YAML values that are exactly the old ID.
//! 3. Commits all of it once, [`COMMIT_MESSAGE`], so git history keeps the
//!    old files.
//!
//! A repo without the old folder has nothing to upgrade, so this runs once.
//! State.db's side of the rename is migration 006.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value};

use super::fs_atomic::content_hash;
use super::layout::{AGENT_FILE, INSTRUCTIONS_FILE};
use super::{RepoLayout, Repos};
use crate::services::builtin_agents::{default_agent, DEFAULT_AGENT};
use crate::services::repo_service::GitService;

/// The default agent's ID in Nandi 0.1.0
pub const OLD_ID: &str = "chief-of-staff";

/// The subject of the upgrade's commit
pub const COMMIT_MESSAGE: &str = "Upgrade from Nandi 0.1.0";

/// The files Nandi 0.1.0 seeded into the Chief of Staff's folder, with the
/// SHA-256 of each ([`text_hash`]), from the tag v0.1.0
pub const SHIPPED_IN_0_1_0: &[(&str, &str)] = &[
    (
        AGENT_FILE,
        "9ab35eff289774a988e7ec9a449198a5b5b3e41cdae2216a8bb45f97c46dad8d",
    ),
    (
        INSTRUCTIONS_FILE,
        "e8dd0b7bea4739d54ae8cb8d0718d3d94556c738d8ff69e26717e3a73c4c883f",
    ),
];

/// Which folder became Nandi's
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// Only the Chief of Staff's folder existed: it moved
    Moved,
    /// Nandi's folder was as this build seeds it: the Chief of Staff's
    /// replaced it
    ChiefOfStaffOverFreshNandi,
    /// The Chief of Staff's folder was as 0.1.0 seeded it: Nandi's stays
    NandiOverUneditedChiefOfStaff,
    /// Both were edited: Nandi's stays, the Chief of Staff's is in history
    NandiOverEditedChiefOfStaff,
}

impl Kept {
    /// What happened to the folders, for the commit message
    fn explain(self) -> String {
        let old = format!(".nandi/agents/{OLD_ID}/");
        let new = format!(".nandi/agents/{DEFAULT_AGENT}/");
        match self {
            Self::Moved => {
                format!("The Chief of Staff is now Nandi: {old} moved to {new} with your edits.")
            }
            Self::ChiefOfStaffOverFreshNandi => format!(
                "The Chief of Staff is now Nandi: {old} replaced the unedited {new} a newer                  Nandi had added, so your edits are Nandi's."
            ),
            Self::NandiOverUneditedChiefOfStaff => format!(
                "Both {old} and {new} existed. The Chief of Staff had no edits, so it was                  removed and Nandi's folder kept."
            ),
            Self::NandiOverEditedChiefOfStaff => format!(
                "Both {old} and {new} existed and both had edits. Nandi's folder was kept.                  The Chief of Staff's files are in the commit before this one."
            ),
        }
    }
}

/// What [`upgrade`] did
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgraded {
    pub kept: Kept,
    /// Definition files where `chief-of-staff` became `nandi`, relative
    pub renamed_in: Vec<String>,
    /// The commit, `None` when it changed nothing git didn't have
    pub commit: Option<String>,
}

impl Upgraded {
    fn commit_message(kept: Kept, renamed_in: &[String]) -> String {
        let mut message = format!("{COMMIT_MESSAGE}\n\n{}", kept.explain());
        if !renamed_in.is_empty() {
            message.push_str(&format!(
                "\n\nchief-of-staff is now nandi in: {}.",
                renamed_in.join(", ")
            ));
        }
        message
    }
}

/// Upgrades a gold copy Nandi 0.1.0 wrote, if it is one, and commits it.
pub fn upgrade(layout: &RepoLayout) -> Result<Option<Upgraded>, String> {
    let old_dir = layout.agent_dir(OLD_ID);
    if !old_dir.is_dir() {
        return Ok(None);
    }
    let new_dir = layout.agent_dir(DEFAULT_AGENT);
    let git = git2::Repository::open(&layout.root)
        .map_err(|e| format!("Couldn't open the gold copy's git history: {e}"))?;
    // Machine-local files (`work/`, logs) are never compared or committed
    let tracked = |path: &Path| {
        let Some(mut relative) = layout.relative(path) else {
            return false;
        };
        if path.is_dir() {
            relative.push('/'); // so `work/` matches the folder
        }
        !git.is_path_ignored(relative).unwrap_or(false)
    };
    let mut touched: Vec<PathBuf> = files_under(&old_dir, &tracked);
    touched.extend(files_under(&new_dir, &tracked));

    let kept = keep_one_folder(&old_dir, &new_dir, &tracked)?;
    touched.extend(files_under(&new_dir, &tracked));

    let mut renamed_in = Vec::new();
    for path in rename_in_definitions(layout)? {
        renamed_in.extend(layout.relative(&path));
        touched.push(path);
    }
    renamed_in.sort();

    let mut paths: Vec<String> = touched
        .iter()
        .filter_map(|path| layout.relative(path))
        .collect();
    paths.sort();
    paths.dedup();
    let message = Upgraded::commit_message(kept, &renamed_in);
    let commit = GitService::new(&layout.root).commit_files(&paths, &message, None)?;
    log::info!("{COMMIT_MESSAGE}: {kept:?}, renamed in {renamed_in:?}");
    Ok(Some(Upgraded {
        kept,
        renamed_in,
        commit,
    }))
}

/// Leaves one folder, `new_dir`, as Nandi's: the person's edited one.
fn keep_one_folder(old_dir: &Path, new_dir: &Path, tracked: &Tracked) -> Result<Kept, String> {
    let kept = if !new_dir.exists() {
        Kept::Moved
    } else if !is_edited(new_dir, &shipped_now(), tracked) {
        Kept::ChiefOfStaffOverFreshNandi
    } else if !is_edited(old_dir, &shipped_in_0_1_0(), tracked) {
        Kept::NandiOverUneditedChiefOfStaff
    } else {
        Kept::NandiOverEditedChiefOfStaff
    };
    let failed = |e: std::io::Error| format!("Couldn't make the Chief of Staff Nandi: {e}");
    match kept {
        Kept::Moved => std::fs::rename(old_dir, new_dir).map_err(failed)?,
        Kept::ChiefOfStaffOverFreshNandi => {
            std::fs::remove_dir_all(new_dir).map_err(failed)?;
            std::fs::rename(old_dir, new_dir).map_err(failed)?;
        }
        Kept::NandiOverUneditedChiefOfStaff | Kept::NandiOverEditedChiefOfStaff => {
            // The commit's parent keeps the files
            std::fs::remove_dir_all(old_dir).map_err(failed)?;
        }
    }
    Ok(kept)
}

/// Renames `chief-of-staff` to `nandi` in each definition file that names
/// it, and returns the files written. A file that can't be read is left for
/// loading to report.
fn rename_in_definitions(layout: &RepoLayout) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for (format, path) in Repos::new(layout.clone()).definition_files() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains(OLD_ID) {
            continue;
        }
        match format.edit(&text, rename_agent) {
            Ok(Some(renamed)) => {
                super::fs_atomic::write_if_changed(&path, renamed.as_bytes())
                    .map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
                written.push(path);
            }
            Ok(None) => {}
            Err(e) => log::warn!("Not renaming the Chief of Staff in {}: {e}", path.display()),
        }
    }
    Ok(written)
}

/// Every YAML value that is exactly `chief-of-staff` becomes `nandi`.
/// Returns whether one did.
pub fn rename_agent(yaml: &mut Mapping) -> bool {
    yaml.values_mut()
        .map(rename_in_value)
        .fold(false, |any, renamed| any | renamed)
}

fn rename_in_value(value: &mut Value) -> bool {
    match value {
        Value::String(text) if text == OLD_ID => {
            *text = DEFAULT_AGENT.to_string();
            true
        }
        Value::Sequence(items) => items
            .iter_mut()
            .map(rename_in_value)
            .fold(false, |any, renamed| any | renamed),
        Value::Mapping(mapping) => rename_agent(mapping),
        Value::Tagged(tagged) => rename_in_value(&mut tagged.value),
        _ => false,
    }
}

/// The SHA-256 of a text file with its line endings as LF, so a file an
/// editor on Windows saved with CRLF still counts as unedited
pub fn text_hash(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    content_hash(text.as_bytes())
}

/// Whether a path is one git keeps (not ignored)
type Tracked<'a> = dyn Fn(&Path) -> bool + 'a;

/// Whether a folder differs from what was seeded into it: a file changed,
/// added or removed. `shipped` maps each seeded file to its [`text_hash`].
fn is_edited(dir: &Path, shipped: &BTreeMap<String, String>, tracked: &Tracked) -> bool {
    let found: BTreeMap<String, String> = files_under(dir, tracked)
        .into_iter()
        .filter_map(|path| {
            let relative = relative_to(dir, &path)?;
            let bytes = std::fs::read(&path).ok()?;
            Some((relative, text_hash(&bytes)))
        })
        .collect();
    &found != shipped
}

/// Nandi's files as this build seeds them
fn shipped_now() -> BTreeMap<String, String> {
    default_agent()
        .shipped_files()
        .into_iter()
        .map(|(path, content)| (path, text_hash(content.as_bytes())))
        .collect()
}

/// The Chief of Staff's files as 0.1.0 seeded them
pub fn shipped_in_0_1_0() -> BTreeMap<String, String> {
    SHIPPED_IN_0_1_0
        .iter()
        .map(|(path, hash)| (path.to_string(), hash.to_string()))
        .collect()
}

/// Every tracked file under `dir`, at any depth. Empty when it doesn't
/// exist.
fn files_under(dir: &Path, tracked: &Tracked) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !tracked(&path) {
            continue;
        }
        if path.is_dir() {
            files.extend(files_under(&path, tracked));
        } else {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// `path` inside `dir`, with `/` separators
fn relative_to(dir: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(dir).ok()?;
    Some(
        relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
#[path = "from_0_1_0_tests.rs"]
mod tests;
