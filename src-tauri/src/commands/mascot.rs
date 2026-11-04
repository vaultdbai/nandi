//! Mascot commands (Tauri adapter for `services::mascot`).

use tauri::State;

use crate::services::mascot::{self, MascotInfo, PhraseResponse};
use crate::services::AppState;

/// The bull's name
#[tauri::command]
#[specta::specta]
pub async fn mascot_get(state: State<'_, AppState>) -> Result<MascotInfo, String> {
    Ok(mascot::read(state.nandi_dir()))
}

/// Renames the bull (`null`: back to the default)
#[tauri::command]
#[specta::specta]
pub async fn mascot_set(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<MascotInfo, String> {
    mascot::write(state.nandi_dir(), name.as_deref())
}

/// The bull's reply to a typed phrase, if it's the hidden one. Only words
/// come back: nothing is approved, run or routed.
#[tauri::command]
#[specta::specta]
pub async fn mascot_phrase(
    state: State<'_, AppState>,
    input: String,
) -> Result<Option<PhraseResponse>, String> {
    let name = mascot::read(state.nandi_dir()).name;
    Ok(mascot::handle_phrase(&input, &name))
}
