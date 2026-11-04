//! Agent memory commands (wave final, Track C; Tauri adapter for
//! `services::agent_memory`).

use tauri::State;

pub use crate::services::agent_memory::MemoryEntry;

use crate::services::agent_memory;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn load_agent_memory(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<MemoryEntry>, String> {
    agent_memory::load_memory(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_agent_memory(
    state: State<'_, AppState>,
    agent_id: String,
    index: u32,
    content: String,
) -> Result<(), String> {
    agent_memory::update_memory_entry(&state, &agent_id, index, content).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_agent_memory(
    state: State<'_, AppState>,
    agent_id: String,
    index: u32,
) -> Result<(), String> {
    agent_memory::delete_memory(&state, &agent_id, index).await
}

/// Re-runs extraction on the agent's latest run; returns the whole memory
#[tauri::command]
#[specta::specta]
pub async fn refresh_agent_memory(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<MemoryEntry>, String> {
    agent_memory::refresh_from_run(&state, &agent_id, None).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_memory_enabled(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<bool, String> {
    agent_memory::memory_enabled(&state, &agent_id).await
}

/// Writes `memory_disabled` to the agent's skill.md
#[tauri::command]
#[specta::specta]
pub async fn set_agent_memory_enabled(
    state: State<'_, AppState>,
    agent_id: String,
    enabled: bool,
) -> Result<(), String> {
    agent_memory::set_memory_enabled(&state, &agent_id, enabled).await
}
