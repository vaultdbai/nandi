//! Harness session commands: history, recorded ACP events, and kill.

use tauri::State;

pub use crate::services::harness::{HarnessEventRecord, HarnessSessionSummary};

use crate::services::harness;
use crate::services::AppState;

/// Newest first. `limit`: `None` is 100
#[tauri::command]
#[specta::specta]
pub async fn list_harness_sessions(
    state: State<'_, AppState>,
    agent_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<HarnessSessionSummary>, String> {
    harness::list_harness_sessions(&state, agent_id, limit).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_harness_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<HarnessSessionSummary, String> {
    harness::get_harness_session(&state, &session_id).await
}

/// In order; `after_seq` pages past events already seen
#[tauri::command]
#[specta::specta]
pub async fn get_harness_events(
    state: State<'_, AppState>,
    session_id: String,
    after_seq: Option<u32>,
) -> Result<Vec<HarnessEventRecord>, String> {
    harness::get_harness_events(&state, &session_id, after_seq).await
}

/// Stops the CLI (and its container); the run fails
#[tauri::command]
#[specta::specta]
pub async fn kill_harness_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    harness::kill_harness_session(&state, &session_id).await
}
