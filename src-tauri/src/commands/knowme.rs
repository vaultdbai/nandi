//! Getting to know you (Tauri adapter for `services::knowme`).

use tauri::State;

pub use crate::services::knowme::proposal::KnowMeProposal;
pub use crate::services::knowme::{KnowMeStatus, KnowMeTodoAction};

use crate::services::knowme;
use crate::services::AppState;

/// The know-me to-do (brought back when its time came), the groups a
/// personal item may be shared with, and the latest confirmed changes
#[tauri::command]
#[specta::specta]
pub async fn get_know_me(state: State<'_, AppState>) -> Result<KnowMeStatus, String> {
    knowme::status(&state).await
}

/// The to-do's Later (back in a week) or Skip (back in a month)
#[tauri::command]
#[specta::specta]
pub async fn know_me_todo(
    state: State<'_, AppState>,
    action: KnowMeTodoAction,
) -> Result<KnowMeStatus, String> {
    knowme::todo(&state, action).await
}

/// Confirms a `knowme_propose` card, with the person's edits
#[tauri::command]
#[specta::specta]
pub async fn confirm_know_me_proposal(
    state: State<'_, AppState>,
    approval_id: String,
    proposal: KnowMeProposal,
) -> Result<(), String> {
    knowme::confirm(&state, approval_id, proposal).await
}
