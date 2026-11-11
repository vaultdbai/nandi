//! File source: a `notify` watcher per enabled file trigger. Events are
//! filtered by kind and file-name glob, then debounced per path (default
//! 2s): a burst of writes to one file is one fire.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use super::schema::{expand_home, FileConfig, FileEventKind, TriggerEvent, TriggerSourceType};

/// A running watch. Dropping it stops the watcher and its debounce task.
pub struct FileWatch {
    _watcher: RecommendedWatcher,
    task: AbortHandle,
}

impl Drop for FileWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The kind of change, as triggers name it (`None` for access, metadata…).
pub fn event_kind(kind: &EventKind) -> Option<FileEventKind> {
    match kind {
        EventKind::Create(_) => Some(FileEventKind::Create),
        // A file moved in counts as created, moved out as deleted
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Some(FileEventKind::Create),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => Some(FileEventKind::Delete),
        EventKind::Modify(ModifyKind::Metadata(_)) => None,
        EventKind::Modify(_) => Some(FileEventKind::Modify),
        EventKind::Remove(_) => Some(FileEventKind::Delete),
        _ => None,
    }
}

/// Whether a change to `path` should fire: its kind is watched and its
/// file name matches a pattern (or there are none).
pub fn matches(config: &FileConfig, path: &Path, kind: FileEventKind) -> bool {
    if !config.events().contains(&kind) {
        return false;
    }
    if config.patterns.is_empty() {
        return true;
    }
    let Some(name) = path.file_name().map(|n| n.to_string_lossy()) else {
        return false;
    };
    let options = glob::MatchOptions {
        case_sensitive: !cfg!(windows),
        ..Default::default()
    };
    config
        .patterns
        .iter()
        .any(|pattern| glob::Pattern::new(pattern).is_ok_and(|p| p.matches_with(&name, options)))
}

/// Starts watching. A missing folder is an error the caller logs as a
/// warning; the trigger stays saved and is retried on the next reload.
pub fn spawn(
    runtime: &tokio::runtime::Handle,
    trigger_id: String,
    config: FileConfig,
    events: mpsc::Sender<TriggerEvent>,
) -> Result<FileWatch, String> {
    let root = expand_home(&config.path);
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<notify::Event>();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) => {
                let _ = tx.send(event);
            }
            Err(e) => log::warn!("File watcher error: {e}"),
        })
        .map_err(|e| format!("Couldn't create a file watcher: {e}"))?;
    let mode = if config.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher
        .watch(&root, mode)
        .map_err(|e| format!("Couldn't watch {}: {e}", root.display()))?;

    let debounce = config.debounce();
    let task = runtime.spawn(async move {
        // Per path: the kind to report and when the burst is over
        let mut pending: HashMap<PathBuf, (FileEventKind, Instant)> = HashMap::new();
        loop {
            let next_due = pending.values().map(|(_, due)| *due).min();
            tokio::select! {
                received = rx.recv() => {
                    let Some(event) = received else { return };
                    let Some(kind) = event_kind(&event.kind) else { continue };
                    for path in event.paths {
                        if !matches(&config, &path, kind) {
                            continue;
                        }
                        let due = Instant::now() + debounce;
                        pending
                            .entry(path)
                            // A create followed by writes is still a create
                            .and_modify(|(k, d)| {
                                if *k != FileEventKind::Create || kind == FileEventKind::Delete {
                                    *k = kind;
                                }
                                *d = due;
                            })
                            .or_insert((kind, due));
                    }
                }
                _ = sleep_until(next_due) => {
                    let now = Instant::now();
                    let ready: Vec<PathBuf> = pending
                        .iter()
                        .filter(|(_, (_, due))| *due <= now)
                        .map(|(path, _)| path.clone())
                        .collect();
                    for path in ready {
                        let Some((kind, _)) = pending.remove(&path) else { continue };
                        let event = TriggerEvent {
                            trigger_id: trigger_id.clone(),
                            source: TriggerSourceType::File,
                            payload: payload(&path, kind),
                            test: false,
                            handover: false,
                        };
                        if events.send(event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    Ok(FileWatch {
        _watcher: watcher,
        task: task.abort_handle(),
    })
}

async fn sleep_until(due: Option<Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

/// `{path, event, size, modified_at}`; size and time are null once deleted.
fn payload(path: &Path, kind: FileEventKind) -> serde_json::Value {
    let metadata = std::fs::metadata(path).ok();
    let modified_at = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t: SystemTime| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
    json!({
        "path": path.display().to_string(),
        "event": kind.as_str(),
        "size": metadata.as_ref().map(|m| m.len()),
        "modified_at": modified_at,
    })
}

/// How long tests wait for a debounced fire.
#[cfg(test)]
pub const TEST_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, MetadataKind, RemoveKind};

    fn config(patterns: &[&str], events: &[FileEventKind]) -> FileConfig {
        FileConfig {
            path: "/in".into(),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
            events: events.to_vec(),
            debounce_ms: None,
            recursive: false,
        }
    }

    #[test]
    fn maps_notify_kinds() {
        use FileEventKind::*;
        assert_eq!(
            event_kind(&EventKind::Create(CreateKind::File)),
            Some(Create)
        );
        assert_eq!(
            event_kind(&EventKind::Modify(ModifyKind::Data(DataChange::Content))),
            Some(Modify)
        );
        assert_eq!(
            event_kind(&EventKind::Modify(ModifyKind::Name(RenameMode::To))),
            Some(Create)
        );
        assert_eq!(
            event_kind(&EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))),
            None
        );
        assert_eq!(
            event_kind(&EventKind::Remove(RemoveKind::File)),
            Some(Delete)
        );
    }

    #[test]
    fn matches_file_name_globs_and_kinds() {
        use FileEventKind::*;
        let pdfs = config(&["*.pdf", "contract-??.docx"], &[]);
        assert!(matches(&pdfs, Path::new("/in/a.pdf"), Create));
        assert!(matches(&pdfs, Path::new("/in/a.pdf"), Modify));
        assert!(
            !matches(&pdfs, Path::new("/in/a.pdf"), Delete),
            "default: create+modify"
        );
        assert!(matches(&pdfs, Path::new("/in/contract-01.docx"), Create));
        assert!(!matches(&pdfs, Path::new("/in/contract-001.docx"), Create));
        assert!(!matches(&pdfs, Path::new("/in/a.pdf.tmp"), Create));
        // Patterns match the name, not the folder
        assert!(!matches(&pdfs, Path::new("/x.pdf/notes.txt"), Create));

        let everything = config(&[], &[Delete]);
        assert!(matches(&everything, Path::new("/in/anything"), Delete));
        assert!(!matches(&everything, Path::new("/in/anything"), Create));
    }

    #[test]
    fn payload_describes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hello").unwrap();
        let value = payload(&path, FileEventKind::Create);
        assert_eq!(value["event"], "create");
        assert_eq!(value["size"], 5);
        assert!(value["modified_at"].is_string());
        let gone = payload(&dir.path().join("gone"), FileEventKind::Delete);
        assert!(gone["size"].is_null());
    }
}
