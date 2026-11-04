//! Filesystem helpers for the gold copy. Every write is atomic: a temp file
//! in the same folder, flushed, then renamed over the target, so readers
//! (and the file watcher) never see a half-written file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// Marker in temp file names; the watcher ignores paths containing it.
pub const TEMP_MARKER: &str = ".nandi-tmp-";

/// Writes `contents` to `path` atomically. Returns `false` (and leaves the
/// file alone, mtime included) if it already has exactly these contents.
pub fn write_if_changed(path: &Path, contents: &[u8]) -> std::io::Result<bool> {
    if fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    write_atomic(path, contents)?;
    Ok(true)
}

pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(
        ".{name}{TEMP_MARKER}{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Removes a file; a missing file is fine. Returns whether it existed.
pub fn remove_if_exists(path: &Path) -> std::io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Moves a folder to `<trash_dir>/<name>-<ms>` and returns where it went.
pub fn move_to_trash(dir: &Path, trash_dir: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(trash_dir)?;
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".to_string());
    let target = trash_dir.join(format!("{name}-{}", crate::db::now_ms()));
    fs::rename(dir, &target)?;
    Ok(target)
}

/// (created, modified). Filesystems without a birth time report the
/// modification time for both.
pub fn file_times(path: &Path) -> (SystemTime, SystemTime) {
    let Ok(meta) = fs::metadata(path) else {
        let now = SystemTime::now();
        return (now, now);
    };
    let modified = meta.modified().unwrap_or_else(|_| SystemTime::now());
    let created = meta.created().unwrap_or(modified).min(modified);
    (created, modified)
}

/// Hex SHA-256 of a file's bytes, for drift detection.
pub fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_atomically_and_skips_identical_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/file.yaml");
        assert!(write_if_changed(&path, b"one").unwrap());
        assert!(!write_if_changed(&path, b"one").unwrap());
        assert!(write_if_changed(&path, b"two").unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        // No temp files left behind
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["file.yaml"]);
        assert!(remove_if_exists(&path).unwrap());
        assert!(!remove_if_exists(&path).unwrap());
    }

    #[test]
    fn hashes_are_stable_hex() {
        assert_eq!(
            content_hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
