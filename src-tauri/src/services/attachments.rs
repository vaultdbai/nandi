//! Message attachments.
//!
//! The file picker runs natively (see the Tauri adapter) and clients only
//! ever hold opaque attachment IDs. If the webview supplied paths, any script running in it
//! could copy arbitrary files (e.g. `~/.ssh`) into an agent's reach.
//!
//! Picked files are copied to `~/.nandi/attachments/staging/<id>/`; sending
//! the message moves them to `~/.nandi/attachments/<turn_id>/<id>/`.

use std::path::{Path, PathBuf};

use crate::model::{AttachmentInfo, AttachmentKind};
use crate::utils::nandi_config;

const STAGING_DIR: &str = "staging";
const MAX_FILES_PER_PICK: usize = 10;
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic"];

fn attachments_dir() -> Result<PathBuf, String> {
    Ok(nandi_config::nandi_dir()?.join("attachments"))
}

fn kind_of(name: &str) -> AttachmentKind {
    let extension = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        AttachmentKind::Image
    } else {
        AttachmentKind::File
    }
}

/// A staged file: its info and where it currently lives.
#[derive(Debug, Clone)]
pub struct Staged {
    pub info: AttachmentInfo,
    pub path: PathBuf,
}

fn validate_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| format!("Invalid attachment id '{id}'"))
}

/// Copies `source` into the staging area under a fresh ID.
fn stage(root: &Path, source: &Path) -> Result<AttachmentInfo, String> {
    let metadata =
        std::fs::metadata(source).map_err(|e| format!("Can't read {}: {e}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("Attachment has no file name")?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{name} is larger than {} MB",
            MAX_FILE_BYTES / 1024 / 1024
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let dir = root.join(STAGING_DIR).join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to stage {name}: {e}"))?;
    std::fs::copy(source, dir.join(&name)).map_err(|e| format!("Failed to stage {name}: {e}"))?;

    Ok(AttachmentInfo {
        kind: kind_of(&name),
        size_bytes: metadata.len() as f64,
        name,
        id,
    })
}

/// Reads back a staged attachment by ID.
fn find_staged(root: &Path, id: &str) -> Result<Staged, String> {
    validate_id(id)?;
    let dir = root.join(STAGING_DIR).join(id);
    let entry = std::fs::read_dir(&dir)
        .map_err(|_| "Attachment not found (it may have been sent or removed)".to_string())?
        .filter_map(Result::ok)
        .find(|entry| entry.path().is_file())
        .ok_or("Attachment not found")?;
    let name = entry.file_name().to_string_lossy().into_owned();
    let size = entry.metadata().map(|m| m.len()).unwrap_or_default();
    Ok(Staged {
        info: AttachmentInfo {
            id: id.to_string(),
            kind: kind_of(&name),
            size_bytes: size as f64,
            name,
        },
        path: entry.path(),
    })
}

/// Stages files chosen in a *native* file picker. Only call this with paths
/// from a trusted picker in the transport adapter, never with paths a
/// client sent: that would let it copy any file on disk.
pub async fn stage_picked_files(paths: Vec<PathBuf>) -> Result<Vec<AttachmentInfo>, String> {
    let root = attachments_dir()?;
    let paths: Vec<PathBuf> = paths.into_iter().take(MAX_FILES_PER_PICK).collect();
    tokio::task::spawn_blocking(move || paths.iter().map(|p| stage(&root, p)).collect())
        .await
        .map_err(|e| format!("Attachment task failed: {e}"))?
}

/// Deletes a staged attachment the user removed before sending.
pub async fn discard_attachment(id: String) -> Result<(), String> {
    validate_id(&id)?;
    let dir = attachments_dir()?.join(STAGING_DIR).join(&id);
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to remove attachment: {e}")),
    }
}

/// Resolves the IDs passed to `send_message`.
pub async fn resolve_staged(ids: &[String]) -> Result<Vec<Staged>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let (root, ids) = (attachments_dir()?, ids.to_vec());
    tokio::task::spawn_blocking(move || ids.iter().map(|id| find_staged(&root, id)).collect())
        .await
        .map_err(|e| format!("Attachment task failed: {e}"))?
}

/// Moves staged files into `~/.nandi/attachments/<turn_id>/<id>/`.
pub async fn commit_staged(staged: Vec<Staged>, turn_id: &str) -> Result<(), String> {
    if staged.is_empty() {
        return Ok(());
    }
    let (root, turn_id) = (attachments_dir()?, turn_id.to_string());
    tokio::task::spawn_blocking(move || commit_into(&root, &staged, &turn_id))
        .await
        .map_err(|e| format!("Attachment task failed: {e}"))?
}

fn commit_into(root: &Path, staged: &[Staged], turn_id: &str) -> Result<(), String> {
    for file in staged {
        let dir = root.join(turn_id).join(&file.info.id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::rename(&file.path, dir.join(&file.info.name)).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir(root.join(STAGING_DIR).join(&file.info.id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_resolves_and_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("photo.PNG");
        std::fs::write(&source, b"png").unwrap();
        let root = tmp.path().join("attachments");

        let info = stage(&root, &source).unwrap();
        assert_eq!(info.name, "photo.PNG");
        assert_eq!(info.kind, AttachmentKind::Image);
        assert_eq!(info.size_bytes, 3.0);

        let staged = find_staged(&root, &info.id).unwrap();
        commit_into(&root, &[staged], "turn-1").unwrap();
        assert!(root
            .join("turn-1")
            .join(&info.id)
            .join("photo.PNG")
            .is_file());
        assert!(find_staged(&root, &info.id).is_err(), "no longer staged");
    }

    #[test]
    fn rejects_bad_ids_and_directories() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_staged(tmp.path(), "../../etc").is_err());
        assert!(stage(tmp.path(), tmp.path()).is_err());
    }
}
