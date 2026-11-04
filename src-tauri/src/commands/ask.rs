//! Ask box commands (Tauri adapter for `services::ask`).

use tauri::State;

pub use crate::services::ask::{AskHistoryEntry, AskRequest, AskResponse};

use crate::services::ask;
use crate::services::AppState;

/// Classifies the ask and answers it: an inline answer, a plan, an edit
/// preview, or a clarifying question
#[tauri::command]
#[specta::specta]
pub async fn ask(state: State<'_, AppState>, request: AskRequest) -> Result<AskResponse, String> {
    ask::ask(&state, request).await
}

/// Starts answering; returns the request ID. `ask://streaming_delta` and
/// `ask://completed` follow.
#[tauri::command]
#[specta::specta]
pub async fn ask_streaming(
    state: State<'_, AppState>,
    request: AskRequest,
) -> Result<String, String> {
    ask::ask_streaming(&state, request).await
}

/// Recent asks, newest first; `limit` `None` is 20
#[tauri::command]
#[specta::specta]
pub async fn list_ask_history(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<AskHistoryEntry>, String> {
    ask::list_ask_history(&state, limit).await
}
