//! Watches the Nandi folders that aren't the repo: My notes and the agents'
//! work folders (`~/.nandi/local/work/<agent>/`). Batched like the repo
//! watcher (`watcher::next_batch`), with the same temp-file rules, and
//! re-scanned when the OS may have dropped events.
//!
//! Only folders that are Nandi's are watched ([`watchable`]): never a drive
//! root or the home folder itself, even when someone shared one as My
//! notes. The Refresh button covers what isn't watched.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use super::FileAction;
use crate::services::watcher::{self, Batch, Signal};

/// Folders in an agent's work folder that are build output or caches,
/// never worth reporting
const NOISY_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".cache",
    "dist",
];

/// Which folder a change is in
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FolderArea {
    Notes,
    /// `work/`, one folder per agent
    Work,
}

/// A folder to watch
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedRoot {
    pub area: FolderArea,
    pub path: PathBuf,
}

/// One changed file (or folder)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    /// Inside the notes folder or the agent's work folder, with `/`
    pub relative: String,
    pub action: FileAction,
}

/// What changed in one area in one batch
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderChange {
    pub area: FolderArea,
    /// Work: the agent whose folder it is
    pub agent_id: Option<String>,
    /// Sorted by `relative`
    pub files: Vec<ChangedFile>,
    /// Events may have been lost: anything in the area may have changed
    pub rescanned: bool,
}

/// Whether `path` may be watched: an existing folder that isn't a drive
/// root or the home folder
pub fn watchable(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err(format!("{} isn't a folder", path.display()));
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if canonical.parent().is_none() {
        return Err(format!("{} is a drive root", path.display()));
    }
    let home = dirs::home_dir().and_then(|home| home.canonicalize().ok());
    if home.is_some_and(|home| home == canonical) {
        return Err(format!("{} is the home folder", path.display()));
    }
    Ok(())
}

/// Maps changed paths to areas
pub struct FolderClassifier {
    /// Each root as given and canonical
    roots: Vec<(PathBuf, FolderArea)>,
}

/// Where a path is
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub area: FolderArea,
    pub agent_id: Option<String>,
    pub relative: String,
}

impl FolderClassifier {
    pub fn new(roots: &[WatchedRoot]) -> Self {
        let mut out = Vec::new();
        for root in roots {
            out.push((root.path.clone(), root.area));
            if let Ok(canonical) = root.path.canonicalize() {
                if canonical != root.path {
                    out.push((canonical, root.area));
                }
            }
        }
        Self { roots: out }
    }

    pub fn classify(&self, path: &Path) -> Option<Hit> {
        let (base, area) = self.roots.iter().find(|(base, _)| path.starts_with(base))?;
        let parts: Vec<String> = path
            .strip_prefix(base)
            .ok()?
            .components()
            .filter_map(|c| match c {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        if parts
            .iter()
            .any(|part| watcher::ignored(part) || part.starts_with('.'))
        {
            return None;
        }
        match area {
            // The folder itself, a note, or a folder of notes
            FolderArea::Notes => {
                let relative = parts.join("/");
                let is_note = relative.to_ascii_lowercase().ends_with(".md");
                (relative.is_empty() || is_note || !path.is_file()).then_some(Hit {
                    area: *area,
                    agent_id: None,
                    relative,
                })
            }
            FolderArea::Work => match parts.as_slice() {
                [] | [_] => None,
                [agent, rest @ ..] => {
                    if rest.iter().any(|part| NOISY_DIRS.contains(&part.as_str())) {
                        return None;
                    }
                    Some(Hit {
                        area: *area,
                        agent_id: Some(agent.clone()),
                        relative: rest.join("/"),
                    })
                }
            },
        }
    }

    /// The changes in a batch, one per area (and agent)
    pub fn changes(&self, batch: &Batch, roots: &[WatchedRoot]) -> Vec<FolderChange> {
        let mut grouped: BTreeMap<(FolderArea, Option<String>), Vec<ChangedFile>> = BTreeMap::new();
        for (path, created) in &batch.paths {
            let Some(hit) = self.classify(path) else {
                continue;
            };
            let action = if !path.exists() {
                FileAction::Deleted
            } else if *created {
                FileAction::Created
            } else {
                FileAction::Updated
            };
            grouped
                .entry((hit.area, hit.agent_id))
                .or_default()
                .push(ChangedFile {
                    path: path.clone(),
                    relative: hit.relative,
                    action,
                });
        }
        if batch.rescan.is_some() {
            for root in roots {
                grouped.entry((root.area, None)).or_default();
            }
        }
        grouped
            .into_iter()
            .map(|((area, agent_id), mut files)| {
                files.sort_by(|a, b| a.relative.cmp(&b.relative));
                FolderChange {
                    area,
                    rescanned: batch.rescan.is_some() && agent_id.is_none(),
                    agent_id,
                    files,
                }
            })
            .filter(|change| change.rescanned || !change.files.is_empty())
            .collect()
    }
}

/// A running folder watcher. Dropping it stops watching.
pub struct FolderWatcher {
    roots: Vec<WatchedRoot>,
    _watcher: notify::RecommendedWatcher,
    task: AbortHandle,
}

impl Drop for FolderWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FolderWatcher {
    /// Watches every [`watchable`] root, sending each batch's changes to
    /// `out`. Roots that can't be watched are logged and left out.
    pub fn spawn(
        roots: Vec<WatchedRoot>,
        out: mpsc::UnboundedSender<Vec<FolderChange>>,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Self, String> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Signal>();
        let mut notify = watcher::notify_watcher(tx)
            .map_err(|e| format!("Couldn't create the folder watcher: {e}"))?;
        let mut watched = Vec::new();
        for root in roots {
            match watchable(&root.path).and_then(|()| {
                notify
                    .watch(&root.path, RecursiveMode::Recursive)
                    .map_err(|e| e.to_string())
            }) {
                Ok(()) => {
                    log::info!("Watching {} for changes", root.path.display());
                    watched.push(root);
                }
                Err(e) => log::warn!("Not watching {}: {e}", root.path.display()),
            }
        }
        let classifier = FolderClassifier::new(&watched);
        let task_roots = watched.clone();
        let task = runtime.spawn(async move {
            while let Some(batch) = watcher::next_batch(&mut rx).await {
                if let Some(why) = &batch.rescan {
                    for root in &task_roots {
                        log::warn!("Re-scanning {}: {why}", root.path.display());
                    }
                }
                let changes = classifier.changes(&batch, &task_roots);
                if !changes.is_empty() && out.send(changes).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            roots: watched,
            _watcher: notify,
            task: task.abort_handle(),
        })
    }

    /// The roots being watched
    pub fn roots(&self) -> &[WatchedRoot] {
        &self.roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::live::testing::wait_until;
    use std::fs;
    use std::sync::{Arc, Mutex};

    fn roots(notes: &Path, work: &Path) -> Vec<WatchedRoot> {
        vec![
            WatchedRoot {
                area: FolderArea::Notes,
                path: notes.to_path_buf(),
            },
            WatchedRoot {
                area: FolderArea::Work,
                path: work.to_path_buf(),
            },
        ]
    }

    #[test]
    fn classifies_notes_and_work_files() {
        let c = FolderClassifier::new(&roots(Path::new("/n"), Path::new("/w")));
        let hit = |area, agent: Option<&str>, relative: &str| {
            Some(Hit {
                area,
                agent_id: agent.map(str::to_string),
                relative: relative.into(),
            })
        };
        assert_eq!(
            c.classify(Path::new("/n/deadlines.md")),
            hit(FolderArea::Notes, None, "deadlines.md")
        );
        assert_eq!(
            c.classify(Path::new("/n/runbooks/etl.md")),
            hit(FolderArea::Notes, None, "runbooks/etl.md")
        );
        assert_eq!(
            c.classify(Path::new("/w/writer/report.md")),
            hit(FolderArea::Work, Some("writer"), "report.md")
        );
        assert_eq!(
            c.classify(Path::new("/w/writer/out/chart.png")),
            hit(FolderArea::Work, Some("writer"), "out/chart.png")
        );
        for ignored in [
            "/n/.deadlines.md.swp",
            "/n/deadlines.md~",
            "/n/deadlines.md.tmp",
            "/n/.git/index",
            "/w/writer",
            "/w",
            "/w/writer/node_modules/x/index.js",
            "/w/writer/.git/HEAD",
            "/w/writer/.report.md.nandi-tmp-1",
            "/elsewhere/a.md",
        ] {
            assert_eq!(c.classify(Path::new(ignored)), None, "{ignored}");
        }
    }

    #[test]
    fn a_batch_that_lost_events_rescans_every_area() {
        let c = FolderClassifier::new(&roots(Path::new("/n"), Path::new("/w")));
        let mut batch = Batch::default();
        batch.add(Signal::Rescan("the OS dropped file events".into()));
        let changes = c.changes(&batch, &roots(Path::new("/n"), Path::new("/w")));
        assert_eq!(changes.len(), 2);
        assert!(changes
            .iter()
            .all(|c| c.rescanned && c.files.is_empty() && c.agent_id.is_none()));
    }

    #[test]
    fn the_home_folder_and_drive_roots_are_never_watched() {
        if let Some(home) = dirs::home_dir() {
            assert!(watchable(&home).is_err());
        }
        let root = std::env::temp_dir()
            .ancestors()
            .last()
            .unwrap()
            .to_path_buf();
        assert!(watchable(&root).is_err());
        let tmp = tempfile::tempdir().unwrap();
        assert!(watchable(tmp.path()).is_ok());
        assert!(watchable(&tmp.path().join("missing")).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn changes_in_real_folders_arrive_batched_by_area() {
        let tmp = tempfile::tempdir().unwrap();
        let (notes, work) = (tmp.path().join("notes"), tmp.path().join("work"));
        fs::create_dir_all(&notes).unwrap();
        fs::create_dir_all(work.join("writer")).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let watcher =
            FolderWatcher::spawn(roots(&notes, &work), tx, &tokio::runtime::Handle::current())
                .unwrap();
        assert_eq!(watcher.roots().len(), 2);
        let seen = Arc::new(Mutex::new(Vec::<FolderChange>::new()));
        let collector = {
            let seen = seen.clone();
            tokio::spawn(async move {
                while let Some(changes) = rx.recv().await {
                    seen.lock().unwrap().extend(changes);
                }
            })
        };
        let has = |area: FolderArea, relative: &str| {
            seen.lock().unwrap().iter().any(|change| {
                change.area == area && change.files.iter().any(|f| f.relative == relative)
            })
        };

        fs::write(notes.join("deadlines.md"), "# Deadlines").unwrap();
        fs::write(notes.join(".deadlines.md.swp"), "x").unwrap();
        fs::write(work.join("writer").join("report.md"), "done").unwrap();
        wait_until("the note and the report", || {
            has(FolderArea::Notes, "deadlines.md") && has(FolderArea::Work, "report.md")
        })
        .await;
        let work_agent = seen
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.area == FolderArea::Work)
            .and_then(|c| c.agent_id.clone());
        assert_eq!(work_agent.as_deref(), Some("writer"));
        assert!(!has(FolderArea::Notes, ".deadlines.md.swp"));

        fs::remove_file(notes.join("deadlines.md")).unwrap();
        wait_until("the deleted note", || {
            seen.lock().unwrap().iter().any(|change| {
                change
                    .files
                    .iter()
                    .any(|f| f.relative == "deadlines.md" && f.action == FileAction::Deleted)
            })
        })
        .await;
        drop(watcher);
        collector.abort();
    }
}
