//! Live UI: every change to a file the app shows, whoever made it, as one
//! typed event (`live://changed`) and a short list of recent changes.
//!
//! Sources:
//!
//! | What changed                                 | Seen by                         | Who                                          |
//! | -------------------------------------------- | ------------------------------- | -------------------------------------------- |
//! | agents, workflows, triggers, settings.yaml, connectors.yaml, manifest.yaml | the store (any write) | the store's `ChangedBy`: you, or outside Nandi |
//! | policies, installed.yaml, NANDI.md           | the repo watcher                | you (`own_writes`), else outside Nandi       |
//! | My notes, agents' work folders               | the folder watcher (`folders`)  | you, else the run going at the time (`author`), else outside Nandi |
//!
//! The frontend invalidates exactly the queries a change affects
//! (`src/lib/live-changes.ts`) and shows who made it. The Refresh button
//! ([`refresh_from_disk`]) re-reads everything without the watchers.

pub mod author;
pub mod folders;
pub mod own_writes;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::mpsc;

use crate::model::Timestamp;
use crate::services::domain::{ChangeAction, ChangedBy, DomainEvent, EntityKind};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::repo_store::{EventPublisher, RepoStore, WeakRepoStore};
use crate::services::{AppState, WeakAppState};
use folders::{FolderArea, FolderChange, FolderWatcher, WatchedRoot};

/// How many changes the recent list keeps
pub const RECENT_CHANGES: usize = 200;
/// Files named in one change; the rest are counted
const PATHS_PER_CHANGE: usize = 5;
/// More notes than this in one batch are reported as one change
const NOTES_PER_BATCH: usize = 20;

/// What kind of thing changed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ChangeArea {
    Agent,
    Workflow,
    Trigger,
    Policy,
    /// `.nandi/settings.yaml`
    Settings,
    /// `.nandi/connectors.yaml`
    Connectors,
    /// `.nandi/manifest.yaml`
    Manifest,
    /// `.nandi/installed.yaml`
    Packs,
    /// `NANDI.md`
    Profile,
    /// My notes
    Notes,
    /// An agent's work folder
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FileAction {
    Created,
    Updated,
    Deleted,
}

impl From<ChangeAction> for FileAction {
    fn from(action: ChangeAction) -> Self {
        match action {
            ChangeAction::Created => Self::Created,
            ChangeAction::Updated => Self::Updated,
            ChangeAction::Deleted => Self::Deleted,
        }
    }
}

/// Who made a change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChangeAuthor {
    /// This app, on the user's command
    You,
    #[serde(rename_all = "camelCase")]
    Agent {
        agent_id: String,
        agent_name: String,
    },
    #[serde(rename_all = "camelCase")]
    Workflow {
        workflow_id: String,
        workflow_name: String,
    },
    /// An editor, a script, a `git pull`
    Outside,
}

/// One change, as `live://changed` sends it and the recent list keeps it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LiveChange {
    pub id: String,
    pub area: ChangeArea,
    /// The entity, note (`runbooks/etl.md`) or, for work, agent ID;
    /// `None` when the whole area was re-read
    pub target_id: Option<String>,
    /// What to call it: the agent's or workflow's name, the file's name
    pub label: String,
    /// Up to five changed files, relative to their folder
    pub paths: Vec<String>,
    /// How many files changed in all
    pub path_count: u32,
    pub action: FileAction,
    pub author: ChangeAuthor,
    pub at: Timestamp,
}

impl NandiEvent for LiveChange {
    const NAME: &'static str = "live://changed";
}

/// What [`refresh_from_disk`] found
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    /// Some definition differed from what the app had
    pub changed: bool,
}

// -- The feed ------------------------------------------------------------------

/// Publishes changes and keeps the latest [`RECENT_CHANGES`] (in memory:
/// the list starts empty with the app)
#[derive(Clone)]
pub struct ChangeFeed(Arc<FeedInner>);

struct FeedInner {
    bus: EventBus,
    recent: Mutex<VecDeque<LiveChange>>,
}

impl ChangeFeed {
    pub fn new(bus: EventBus) -> Self {
        Self(Arc::new(FeedInner {
            bus,
            recent: Mutex::default(),
        }))
    }

    pub fn record(&self, change: LiveChange) {
        {
            let mut recent = self.0.recent.lock().unwrap_or_else(|e| e.into_inner());
            recent.push_front(change.clone());
            recent.truncate(RECENT_CHANGES);
        }
        events::emit(&self.0.bus, &change);
    }

    /// Newest first
    pub fn recent(&self) -> Vec<LiveChange> {
        self.0
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// A single file the store doesn't load (a policy, installed.yaml,
    /// NANDI.md) changed: by the app if it just wrote it, else outside
    pub fn file_changed(&self, area: ChangeArea, target_id: Option<String>, path: &Path) {
        let name = file_name(path);
        let action = if path.exists() {
            FileAction::Updated
        } else {
            FileAction::Deleted
        };
        self.record(LiveChange {
            id: new_id(),
            area,
            label: target_id.clone().unwrap_or_else(|| name.clone()),
            target_id,
            paths: vec![name],
            path_count: 1,
            action,
            author: if own_writes::is_own(path) {
                ChangeAuthor::You
            } else {
                ChangeAuthor::Outside
            },
            at: now(),
        });
    }
}

/// Puts every store change (an agent, workflow, trigger or singleton) in
/// the feed. Holds the store weakly: the store holds its subscribers.
pub struct StoreChanges {
    feed: ChangeFeed,
    store: WeakRepoStore,
}

impl StoreChanges {
    /// Subscribes the feed to `store`
    pub fn subscribe(feed: &ChangeFeed, store: &RepoStore) {
        store.subscribe(Arc::new(Self {
            feed: feed.clone(),
            store: store.downgrade(),
        }));
    }
}

impl EventPublisher for StoreChanges {
    fn changed(&self, event: &DomainEvent) {
        let area = match event.kind {
            EntityKind::Agent => ChangeArea::Agent,
            EntityKind::Workflow => ChangeArea::Workflow,
            EntityKind::Trigger => ChangeArea::Trigger,
            EntityKind::Settings => ChangeArea::Settings,
            EntityKind::Connectors => ChangeArea::Connectors,
            EntityKind::Manifest => ChangeArea::Manifest,
        };
        let file = file_name(&event.source_path);
        let singleton = matches!(
            event.kind,
            EntityKind::Settings | EntityKind::Connectors | EntityKind::Manifest
        );
        let label = if singleton {
            file.clone()
        } else {
            self.store
                .upgrade()
                .and_then(|store| entity_name(&store, event.kind, &event.id))
                .unwrap_or_else(|| event.id.clone())
        };
        self.feed.record(LiveChange {
            id: new_id(),
            area,
            target_id: (!singleton).then(|| event.id.clone()),
            label,
            paths: vec![file],
            path_count: 1,
            action: event.action.into(),
            author: match event.changed_by {
                ChangedBy::User => ChangeAuthor::You,
                ChangedBy::Watcher | ChangedBy::Git => ChangeAuthor::Outside,
            },
            at: now(),
        });
    }

    fn invalid(&self, _: &crate::services::domain::repo::Problem, _: ChangedBy) {}
}

fn entity_name(store: &RepoStore, kind: EntityKind, id: &str) -> Option<String> {
    store.read(|repo| match kind {
        EntityKind::Agent => repo.agent(id).map(|a| a.name().to_string()),
        EntityKind::Workflow => repo.workflow(id).map(|w| w.name().to_string()),
        EntityKind::Trigger => repo.trigger(id).map(|t| t.name().to_string()),
        _ => None,
    })
}

// -- The service ---------------------------------------------------------------

/// The feed and the folder watcher, owned by `AppState`
pub struct LiveService {
    feed: ChangeFeed,
    folders: Mutex<Option<FolderWatcher>>,
}

impl LiveService {
    pub fn new(feed: ChangeFeed) -> Self {
        Self {
            feed,
            folders: Mutex::default(),
        }
    }

    pub fn feed(&self) -> &ChangeFeed {
        &self.feed
    }

    /// Watches the notes folder and the work folders (replacing any earlier
    /// watch). A no-op when they're the folders already watched.
    pub fn watch_folders(&self, state: &AppState, notes: PathBuf, work: PathBuf) {
        let roots = vec![
            WatchedRoot {
                area: FolderArea::Notes,
                path: notes,
            },
            WatchedRoot {
                area: FolderArea::Work,
                path: work,
            },
        ];
        let mut slot = self.folders.lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .as_ref()
            .is_some_and(|watching| watching.roots() == roots)
        {
            return;
        }
        *slot = None;
        let (tx, rx) = mpsc::unbounded_channel();
        match FolderWatcher::spawn(roots, tx, state.runtime()) {
            Ok(watcher) => {
                *slot = Some(watcher);
                state
                    .runtime()
                    .spawn(publish_folder_changes(state.downgrade(), rx));
            }
            Err(e) => log::warn!("Not watching the notes and work folders: {e}"),
        }
    }

    /// Stops the folder watcher (app shutdown)
    pub fn stop(&self) {
        self.folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}

/// Turns each batch of folder changes into feed entries, naming who made
/// them
async fn publish_folder_changes(
    weak: WeakAppState,
    mut rx: mpsc::UnboundedReceiver<Vec<FolderChange>>,
) {
    while let Some(changes) = rx.recv().await {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let since = crate::db::now_ms() - author::RECENT_RUN_MS;
        let runs = state
            .db()
            .call(move |conn| author::active_runs(conn, since))
            .await
            .unwrap_or_else(|e| {
                log::warn!("Couldn't read the runs going on: {e}");
                Vec::new()
            });
        let agents_can_write = crate::services::connectors::shared_folder(
            &state,
            crate::services::notes::NOTES_FOLDER_ID,
        )
        .is_some_and(|shared| shared.access == crate::services::connectors::folders::Access::Write);
        for change in changes {
            for live in to_live(&change, &runs, agents_can_write, state.store()) {
                state.live().feed().record(live);
            }
        }
    }
}

/// The feed entries for one area's changes: a note each (up to
/// [`NOTES_PER_BATCH`]), one per agent's work folder
fn to_live(
    change: &FolderChange,
    runs: &[author::ActiveRun],
    agents_can_write: bool,
    store: &RepoStore,
) -> Vec<LiveChange> {
    let (area, place) = match change.area {
        FolderArea::Notes => (ChangeArea::Notes, author::Place::Notes { agents_can_write }),
        FolderArea::Work => (
            ChangeArea::Work,
            author::Place::Work {
                agent_id: change.agent_id.as_deref().unwrap_or_default(),
            },
        ),
    };
    let author_for = |files: &[folders::ChangedFile]| {
        if !files.is_empty() && files.iter().all(|f| own_writes::is_own(&f.path)) {
            ChangeAuthor::You
        } else {
            author::author_of(author::writer(place, runs), store)
        }
    };
    let entry = |target_id: Option<String>, label: String, files: &[folders::ChangedFile]| {
        let action = match files {
            [only] => only.action,
            _ if !files.is_empty() && files.iter().all(|f| f.action == FileAction::Created) => {
                FileAction::Created
            }
            _ => FileAction::Updated,
        };
        LiveChange {
            id: new_id(),
            area,
            target_id,
            label,
            paths: files
                .iter()
                .take(PATHS_PER_CHANGE)
                .map(|f| f.relative.clone())
                .collect(),
            path_count: u32::try_from(files.len()).unwrap_or(u32::MAX),
            action,
            author: author_for(files),
            at: now(),
        }
    };
    if change.rescanned && change.files.is_empty() {
        return vec![entry(change.agent_id.clone(), String::new(), &[])];
    }
    match change.area {
        FolderArea::Notes if change.files.len() <= NOTES_PER_BATCH && !change.rescanned => change
            .files
            .iter()
            .map(|file| {
                entry(
                    Some(file.relative.clone()),
                    file.relative.clone(),
                    std::slice::from_ref(file),
                )
            })
            .collect(),
        FolderArea::Notes => vec![entry(None, String::new(), &change.files)],
        FolderArea::Work => {
            let agent_id = change.agent_id.clone().unwrap_or_default();
            let label = store.display_name(&agent_id);
            vec![entry(Some(agent_id), label, &change.files)]
        }
    }
}

/// Re-reads every definition from disk, watcher or not, and publishes what
/// changed. My notes and work folders are read from disk on every request,
/// so the frontend only refetches them.
pub async fn refresh_from_disk(state: &AppState) -> Result<RefreshReport, String> {
    let store = state.store().clone();
    let changed = tokio::task::spawn_blocking(move || store.rescan_all(ChangedBy::Watcher))
        .await
        .map_err(|e| format!("Refresh failed: {e}"))?;
    log::info!("Refreshed from disk (changed: {changed})");
    Ok(RefreshReport { changed })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn now() -> Timestamp {
    crate::db::now_ms() as Timestamp
}

#[cfg(test)]
pub(crate) mod testing {
    use std::time::Duration;

    /// Up to 20 s: file events arrive late on a slow CI runner. A passing
    /// check returns at once
    pub async fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
        for _ in 0..400 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {what}");
    }
}

#[cfg(test)]
mod tests;
