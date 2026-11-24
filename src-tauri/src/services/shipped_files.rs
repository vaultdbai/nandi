//! Updates to the files Nandi ships for its default agent: `agent.md`,
//! `instructions.md`, the playbooks and the default triggers.
//!
//! `.nandi/shipped.yaml` ([`ShippedManifest`]) records the SHA-256
//! ([`text_hash`]) of each shipped file as Nandi last wrote it. On every
//! start, [`update`] compares each file with the record and with this
//! build's version ([`decide`]):
//!
//! | The file in the gold copy           | What happens                          |
//! | ----------------------------------- | ------------------------------------- |
//! | Already this build's version        | Recorded                              |
//! | Unedited, and this build changed it | Replaced by the new version           |
//! | Edited, and this build changed it   | Kept; Nandi's page offers the update  |
//! | Deleted by the person               | Stays deleted                         |
//! | New in this build                   | Added                                 |
//!
//! The record lives in the gold copy, not in `state.db`, because it is
//! about the gold copy's files: it is committed with them, so it travels
//! with Backup and a restore, and a fresh `state.db` never loses it.
//!
//! A gold copy from Nandi 0.1.0 has no record. Its `agent.md` and
//! `instructions.md` are compared with [`SHIPPED_IN_0_1_0`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::builtin_agents::{default_agent, DEFAULT_AGENT, DEFAULT_TRIGGERS};
use crate::services::domain::EntityKind;
use crate::services::repo_service::GitService;
use crate::services::repo_store::RepoStore;
use crate::services::repos::from_0_1_0::{text_hash, SHIPPED_IN_0_1_0};
use crate::services::repos::fs_atomic::write_if_changed;
use crate::services::repos::RepoLayout;

/// The subject of the commit that updates unedited files at startup
pub const UPDATE_COMMIT: &str = "Update Nandi's shipped files";

/// One file Nandi ships into the gold copy
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedFile {
    /// Its path in the gold copy, relative to the root, with `/` separators
    pub path: String,
    /// This build's version
    pub content: &'static str,
    /// The entity it belongs to, re-projected when the file changes
    pub kind: EntityKind,
    pub id: String,
}

impl ShippedFile {
    /// The [`text_hash`] of this build's version
    pub fn hash(&self) -> String {
        text_hash(self.content.as_bytes())
    }
}

/// Every file this build ships for Nandi: its agent folder's files, then
/// its default triggers
pub fn shipped_files(layout: &RepoLayout) -> Vec<ShippedFile> {
    let in_agent = default_agent()
        .shipped_files()
        .into_iter()
        .map(|(name, content)| ShippedFile {
            path: agent_file(layout, &name),
            content,
            kind: EntityKind::Agent,
            id: DEFAULT_AGENT.to_string(),
        });
    let triggers = DEFAULT_TRIGGERS.iter().map(|(id, yaml)| ShippedFile {
        path: relative(layout, layout.trigger_path(id)),
        content: yaml,
        kind: EntityKind::Trigger,
        id: id.to_string(),
    });
    in_agent.chain(triggers).collect()
}

/// `name` in Nandi's folder, relative to the gold copy's root
fn agent_file(layout: &RepoLayout, name: &str) -> String {
    relative(layout, layout.agent_dir(DEFAULT_AGENT).join(name))
}

fn relative(layout: &RepoLayout, path: PathBuf) -> String {
    layout
        .relative(&path)
        .expect("shipped files live in the gold copy")
}

/// `.nandi/shipped.yaml`: what Nandi last wrote of its shipped files, and
/// the updates the person kept their own file over
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShippedManifest {
    /// Each shipped file by its path, with the [`text_hash`] of the version
    /// Nandi last wrote there
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// Each file the person chose Keep mine for, with the [`text_hash`] of
    /// the shipped version they kept theirs over. A newer version is
    /// offered again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kept: BTreeMap<String, String>,
}

/// The comment at the top of `.nandi/shipped.yaml`
const MANIFEST_HEADER: &str =
    "# The files Nandi ships, by the SHA-256 of the version it last wrote.
# Nandi updates a file only while it still matches. Don't edit this file.
";

impl ShippedManifest {
    /// The record in the gold copy, `None` when there is none yet
    pub fn load(layout: &RepoLayout) -> Result<Option<Self>, String> {
        let path = layout.shipped_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
        };
        serde_yaml::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{} is malformed: {e}", path.display()))
    }

    /// The record, or for a gold copy without one, what Nandi 0.1.0 wrote
    pub fn load_or_0_1_0(layout: &RepoLayout) -> Result<Self, String> {
        Ok(Self::load(layout)?.unwrap_or_else(|| Self::from_0_1_0(layout)))
    }

    /// The files Nandi 0.1.0 seeded, now in Nandi's folder
    fn from_0_1_0(layout: &RepoLayout) -> Self {
        let files = SHIPPED_IN_0_1_0
            .iter()
            .map(|(name, hash)| (agent_file(layout, name), hash.to_string()))
            .collect();
        Self {
            files,
            kept: BTreeMap::new(),
        }
    }

    /// Writes the record. Returns whether the file changed.
    pub fn save(&self, layout: &RepoLayout) -> Result<bool, String> {
        let yaml = serde_yaml::to_string(self)
            .map_err(|e| format!("Couldn't write the shipped files record: {e}"))?;
        let path = layout.shipped_path();
        write_if_changed(&path, format!("{MANIFEST_HEADER}{yaml}").as_bytes())
            .map_err(|e| format!("Couldn't write {}: {e}", path.display()))
    }

    /// Nandi wrote `hash` at `path`; an update kept out before no longer
    /// applies
    fn wrote(&mut self, path: &str, hash: String) {
        self.files.insert(path.to_string(), hash);
        self.kept.remove(path);
    }

    /// What to do with `file`, given its [`text_hash`] in the gold copy
    fn decide(&self, file: &ShippedFile, on_disk: Option<&str>) -> Step {
        decide(
            self.files.get(&file.path).map(String::as_str),
            on_disk,
            &file.hash(),
            self.kept.get(&file.path).map(String::as_str),
        )
    }
}

/// What [`update`] does with one shipped file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// New in this build and not in the gold copy: write it
    Add,
    /// Unedited since Nandi wrote it, and this build changed it: write the
    /// new version
    Replace,
    /// Already this build's version: record it
    Record,
    /// Edited, and this build changed it: keep it and offer the update
    Offer,
    /// Nothing to do: the person deleted it, kept theirs over this version,
    /// or edited it while this build didn't change it
    Leave,
}

/// What to do with a shipped file, by [`text_hash`]:
/// - `written`: the version Nandi last wrote there, `None` if never
/// - `on_disk`: the file in the gold copy, `None` if it isn't there
/// - `shipped`: this build's version
/// - `kept`: the version the person chose Keep mine for, if any
pub fn decide(
    written: Option<&str>,
    on_disk: Option<&str>,
    shipped: &str,
    kept: Option<&str>,
) -> Step {
    match (written, on_disk) {
        (_, Some(disk)) if disk == shipped => Step::Record,
        (None, None) => Step::Add,
        (Some(_), None) => Step::Leave,
        (Some(written), Some(disk)) if disk == written => Step::Replace,
        (Some(written), Some(_)) if written == shipped => Step::Leave,
        (_, Some(_)) if kept == Some(shipped) => Step::Leave,
        (_, Some(_)) => Step::Offer,
    }
}

/// What [`update`] wrote
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Updated {
    /// Unedited files replaced by this build's version
    pub replaced: Vec<String>,
    /// Files new in this build
    pub added: Vec<String>,
    /// Whether `.nandi/shipped.yaml` changed
    pub recorded: bool,
}

impl Updated {
    /// Every file written, relative to the root, for the commit
    pub fn paths(&self, layout: &RepoLayout) -> Vec<String> {
        let mut paths: Vec<String> = self.replaced.iter().chain(&self.added).cloned().collect();
        if self.recorded {
            paths.push(relative(layout, layout.shipped_path()));
        }
        paths
    }

    /// The commit's message: the subject, then what changed
    pub fn commit_message(&self) -> String {
        let mut message = UPDATE_COMMIT.to_string();
        if !self.replaced.is_empty() {
            message.push_str(&format!(
                "\n\nUpdated, as you hadn't edited them: {}.",
                self.replaced.join(", ")
            ));
        }
        if !self.added.is_empty() {
            message.push_str(&format!("\n\nAdded: {}.", self.added.join(", ")));
        }
        message
    }
}

/// Brings Nandi's shipped files up to this build ([`decide`]) and writes
/// the record. Commits nothing: the caller does.
pub fn update(layout: &RepoLayout) -> Result<Updated, String> {
    update_files(layout, &shipped_files(layout))
}

/// [`update`] over a given set of shipped files
pub(crate) fn update_files(
    layout: &RepoLayout,
    shipped: &[ShippedFile],
) -> Result<Updated, String> {
    let existing = ShippedManifest::load(layout)?;
    let mut manifest = existing
        .clone()
        .unwrap_or_else(|| ShippedManifest::from_0_1_0(layout));
    let mut updated = Updated::default();
    for file in shipped {
        let on_disk = hash_on_disk(layout, &file.path);
        match manifest.decide(file, on_disk.as_deref()) {
            Step::Add => {
                write(layout, file)?;
                manifest.wrote(&file.path, file.hash());
                updated.added.push(file.path.clone());
            }
            Step::Replace => {
                write(layout, file)?;
                manifest.wrote(&file.path, file.hash());
                updated.replaced.push(file.path.clone());
            }
            Step::Record => manifest.wrote(&file.path, file.hash()),
            Step::Offer | Step::Leave => {}
        }
    }
    if existing.as_ref() != Some(&manifest) {
        updated.recorded = manifest.save(layout)?;
    }
    if !updated.replaced.is_empty() || !updated.added.is_empty() {
        log::info!(
            "Updated Nandi's shipped files: replaced {:?}, added {:?}",
            updated.replaced,
            updated.added
        );
    }
    Ok(updated)
}

/// An update Nandi's page offers: a shipped file the person edited, and
/// this build's version of it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShippedUpdate {
    /// The file, relative to the gold copy's root
    pub path: String,
    /// The person's version
    pub mine: String,
    /// This build's version
    pub shipped: String,
    /// A unified diff from theirs to this build's
    pub diff: String,
}

/// The updates to offer on Nandi's page ([`Step::Offer`])
pub fn offers(layout: &RepoLayout) -> Result<Vec<ShippedUpdate>, String> {
    offers_of(layout, &shipped_files(layout))
}

pub(crate) fn offers_of(
    layout: &RepoLayout,
    shipped: &[ShippedFile],
) -> Result<Vec<ShippedUpdate>, String> {
    let manifest = ShippedManifest::load_or_0_1_0(layout)?;
    shipped
        .iter()
        .filter(|file| {
            let on_disk = hash_on_disk(layout, &file.path);
            manifest.decide(file, on_disk.as_deref()) == Step::Offer
        })
        .map(|file| {
            let mine = std::fs::read_to_string(layout.root.join(&file.path))
                .map_err(|e| format!("Couldn't read {}: {e}", file.path))?;
            let diff = crate::services::ask::editor::unified_diff(&file.path, &mine, file.content)?;
            Ok(ShippedUpdate {
                path: file.path.clone(),
                mine,
                shipped: file.content.to_string(),
                diff,
            })
        })
        .collect()
}

/// Take update: writes this build's version of `path` over the person's,
/// records it, and commits both. Their version stays in git history.
pub fn take(store: &RepoStore, git: &GitService, path: &str) -> Result<(), String> {
    take_of(store, git, &shipped_files(store.layout()), path)
}

pub(crate) fn take_of(
    store: &RepoStore,
    git: &GitService,
    shipped: &[ShippedFile],
    path: &str,
) -> Result<(), String> {
    let layout = store.layout();
    let (file, mut manifest) = offered(layout, shipped, path)?;
    // Their version first, so the history keeps it
    git.commit_files(
        std::slice::from_ref(&file.path),
        &keep_before_take(path),
        None,
    )?;
    write(layout, file)?;
    manifest.wrote(&file.path, file.hash());
    manifest.save(layout)?;
    store.commit(file.kind, &file.id)?;
    let paths = [file.path.clone(), relative(layout, layout.shipped_path())];
    git.commit_files(&paths, &format!("Take Nandi's update to {path}"), None)?;
    log::info!("Took Nandi's update to {path}");
    Ok(())
}

/// The commit that keeps the person's version before Take update
fn keep_before_take(path: &str) -> String {
    format!("Keep my {path} before taking Nandi's update")
}

/// Keep mine: records that the person keeps their `path` over this build's
/// version, so it isn't offered again until a newer one ships
pub fn keep_mine(store: &RepoStore, git: &GitService, path: &str) -> Result<(), String> {
    keep_mine_of(store.layout(), git, &shipped_files(store.layout()), path)
}

pub(crate) fn keep_mine_of(
    layout: &RepoLayout,
    git: &GitService,
    shipped: &[ShippedFile],
    path: &str,
) -> Result<(), String> {
    let (file, mut manifest) = offered(layout, shipped, path)?;
    manifest.kept.insert(file.path.clone(), file.hash());
    manifest.save(layout)?;
    let record = relative(layout, layout.shipped_path());
    git.commit_files(
        &[record],
        &format!("Keep my {path} over Nandi's update"),
        None,
    )?;
    log::info!("Kept the person's {path} over Nandi's update");
    Ok(())
}

/// The shipped file at `path` and the record, when an update to it is on
/// offer
fn offered<'a>(
    layout: &RepoLayout,
    shipped: &'a [ShippedFile],
    path: &str,
) -> Result<(&'a ShippedFile, ShippedManifest), String> {
    let manifest = ShippedManifest::load_or_0_1_0(layout)?;
    let file = shipped
        .iter()
        .find(|file| file.path == path)
        .ok_or_else(|| format!("{path} isn't one of Nandi's shipped files"))?;
    let on_disk = hash_on_disk(layout, path);
    if manifest.decide(file, on_disk.as_deref()) != Step::Offer {
        return Err(format!("There is no update to {path} on offer"));
    }
    Ok((file, manifest))
}

/// For a reset to defaults: writes every shipped file as this build ships
/// it, records each, and forgets every Keep mine. Commits nothing: the
/// reset does.
pub fn restore(layout: &RepoLayout) -> Result<(), String> {
    let mut manifest = ShippedManifest::load_or_0_1_0(layout)?;
    for file in shipped_files(layout) {
        write(layout, &file)?;
        manifest.wrote(&file.path, file.hash());
    }
    manifest.kept.clear();
    manifest.save(layout)?;
    Ok(())
}

/// The [`text_hash`] of `path` in the gold copy, `None` when it isn't there
fn hash_on_disk(layout: &RepoLayout, path: &str) -> Option<String> {
    std::fs::read(layout.root.join(path))
        .ok()
        .map(|bytes| text_hash(&bytes))
}

fn write(layout: &RepoLayout, file: &ShippedFile) -> Result<(), String> {
    let path = layout.root.join(&file.path);
    write_if_changed(&path, file.content.as_bytes())
        .map(|_| ())
        .map_err(|e| format!("Couldn't write {}: {e}", file.path))
}

#[cfg(test)]
#[path = "shipped_files_tests.rs"]
mod tests;
