//! Network directory commands (Tauri adapter for `services::network`).

use tauri::State;

pub use crate::services::network::NetworkSnapshot;

use crate::services::network;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_network_snapshot(state: State<'_, AppState>) -> Result<NetworkSnapshot, String> {
    network::get_network_snapshot(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn refresh_network(state: State<'_, AppState>) -> Result<NetworkSnapshot, String> {
    network::refresh_network(&state).await
}
