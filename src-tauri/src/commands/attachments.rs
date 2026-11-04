//! Attachment commands. The native file picker lives here: it's the only
//! trusted source of paths for `services::attachments::stage_picked_files`.

use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tokio::sync::oneshot;

use crate::model::{AttachmentInfo, AttachmentKind};
use crate::services::attachments::{self, IMAGE_EXTENSIONS};

/// Opens the system file picker (images only for `kind: image`) and stages
/// the chosen files. Cancelling returns an empty list.
#[tauri::command]
#[specta::specta]
pub async fn pick_attachments(
    app: AppHandle,
    kind: AttachmentKind,
) -> Result<Vec<AttachmentInfo>, String> {
    let (tx, rx) = oneshot::channel();
    let mut dialog = app.dialog().file();
    if kind == AttachmentKind::Image {
        dialog = dialog.add_filter("Images", IMAGE_EXTENSIONS);
    }
    dialog.pick_files(move |files| {
        let _ = tx.send(files);
    });
    let Some(files) = rx
        .await
        .map_err(|_| "File picker closed unexpectedly".to_string())?
    else {
        return Ok(Vec::new());
    };
    let paths: Vec<PathBuf> = files
        .into_iter()
        .filter_map(|file| file.into_path().ok())
        .collect();
    attachments::stage_picked_files(paths).await
}

#[tauri::command]
#[specta::specta]
pub async fn discard_attachment(id: String) -> Result<(), String> {
    attachments::discard_attachment(id).await
}
