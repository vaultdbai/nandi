//! Opening a file a run touched: desktop-only, as it opens on the host.
//! Finding the file is `rpc::snapshots::resolve_run_file`.

use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::services::{snapshots, AppState};

/// Opens a file a run touched with its default app, or shows it in the
/// file manager (`reveal`)
#[tauri::command]
#[specta::specta]
pub async fn open_run_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    path: String,
    reveal: bool,
) -> Result<(), String> {
    let file = snapshots::run_file_path(&state, run_id, path)
        .await?
        .ok_or("That file isn't on this computer anymore")?;
    if reveal {
        app.opener().reveal_item_in_dir(&file)
    } else {
        app.opener().open_path(file, None::<&str>)
    }
    .map_err(|e| format!("Couldn't open the file: {e}"))
}
