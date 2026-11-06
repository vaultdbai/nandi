//! The app's own recent writes, so a watcher event they cause is shown as
//! "by you" rather than "outside Nandi". Every gold-copy write goes
//! through `repos::fs_atomic`, and My notes through `notes::NotesFolder`;
//! both note the path here. Process-wide: a write and its watcher event
//! are on different tasks.

use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long after a write its watcher event still counts as the app's.
/// The watcher reports at most `watcher::MAX_DELAY` after the change.
pub const WINDOW: Duration = Duration::from_secs(5);

static WRITES: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Mutex::default);

/// Records that the app is about to write (or remove) `path`
pub fn note(path: &Path) {
    let now = Instant::now();
    let mut writes = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    writes.retain(|_, at| now.duration_since(*at) < WINDOW);
    writes.insert(key(path), now);
}

/// Whether the app wrote `path` in the last [`WINDOW`]
pub fn is_own(path: &Path) -> bool {
    WRITES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key(path))
        .is_some_and(|at| at.elapsed() < WINDOW)
}

/// One spelling per file: the same components, `/` between them, and no
/// case on Windows (where `C:\X` and `c:/x` are the same file)
fn key(path: &Path) -> String {
    let joined = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
        .replace('\\', "/");
    if cfg!(windows) {
        joined.to_lowercase()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noted_write_is_the_apps_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes").join("deadlines.md");
        assert!(!is_own(&path));
        note(&path);
        assert!(is_own(&path));
        // The same file, spelled another way
        assert!(is_own(&dir.path().join("notes/./deadlines.md")));
        assert!(!is_own(&dir.path().join("notes").join("other.md")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_ignore_case_and_separators() {
        assert_eq!(
            key(Path::new(r"C:\Users\Me\notes\A.md")),
            key(Path::new("c:/users/me/notes/a.md"))
        );
    }
}
