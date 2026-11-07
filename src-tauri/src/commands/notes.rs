//! My notes commands (Tauri adapter for `services::notes`).

use tauri::State;

pub use crate::services::notes::{MyNotes, NoteFile};

use crate::services::notes;
use crate::services::AppState;

/// The notes folder and its notes; creates and shares it on first use
#[tauri::command]
#[specta::specta]
pub async fn get_my_notes(state: State<'_, AppState>) -> Result<MyNotes, String> {
    notes::my_notes(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn read_note(state: State<'_, AppState>, name: String) -> Result<String, String> {
    notes::read_note(&state, name).await
}

/// Creates or replaces a note
#[tauri::command]
#[specta::specta]
pub async fn save_note(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<NoteFile, String> {
    notes::save_note(&state, name, content).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_note(state: State<'_, AppState>, name: String) -> Result<(), String> {
    notes::delete_note(&state, name).await
}

/// Lets agents update the notes (without asking each time), or not
#[tauri::command]
#[specta::specta]
pub async fn set_notes_agents_can_write(
    state: State<'_, AppState>,
    allow: bool,
) -> Result<MyNotes, String> {
    notes::set_agents_can_write(&state, allow).await
}

/// The Chief of Staff's open capability suggestions
#[tauri::command]
#[specta::specta]
pub async fn list_note_suggestions(
    state: State<'_, AppState>,
) -> Result<Vec<crate::services::notes::corrections::Suggestion>, String> {
    notes::suggestions(&state).await
}

/// Accepts or dismisses a suggestion (remembered in corrections.md)
#[tauri::command]
#[specta::specta]
pub async fn answer_note_suggestion(
    state: State<'_, AppState>,
    id: String,
    accept: bool,
) -> Result<String, String> {
    notes::answer_suggestion(&state, id, accept).await
}
