//! Relay commands (Tauri adapter for `services::relay`).

use tauri::State;

pub use crate::services::relay::join::{JoinOutcome, RelayInvite};
pub use crate::services::relay::{RelayPeer, RelayStatus};

use crate::services::domain::AgentCard;
use crate::services::relay;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_relay_status(state: State<'_, AppState>) -> Result<RelayStatus, String> {
    relay::get_relay_status(&state).await
}

/// Saves the relay URL and reconnects
#[tauri::command]
#[specta::specta]
pub async fn set_relay_url(state: State<'_, AppState>, url: String) -> Result<(), String> {
    relay::set_relay_url(&state, &url).await
}

/// "Join": joins the relay at `url`, enrolling with `join_code` (or the
/// build's own code for the build's relay) when it is secured
#[tauri::command]
#[specta::specta]
pub async fn join_relay(
    state: State<'_, AppState>,
    url: String,
    join_code: Option<String>,
) -> Result<JoinOutcome, String> {
    relay::join::join_relay(&state, &url, join_code.as_deref()).await
}

/// What "Copy invite message" passes on (`None`: no relay in use)
#[tauri::command]
#[specta::specta]
pub async fn get_relay_invite(state: State<'_, AppState>) -> Result<Option<RelayInvite>, String> {
    relay::join::relay_invite(&state)
}

/// Agents online at the relay, optionally only those serving `capability`
#[tauri::command]
#[specta::specta]
pub async fn list_online_agents(
    state: State<'_, AppState>,
    capability: Option<String>,
) -> Result<Vec<AgentCard>, String> {
    relay::list_online_agents(&state, capability.as_deref()).await
}

/// "Test connection": WebSocket ping to `url`; the round trip in ms, or the
/// exact error
#[tauri::command]
#[specta::specta]
pub async fn test_relay_connection(url: String) -> Result<u32, String> {
    relay::test_relay_connection(&url).await
}

/// The instances at the relay (its `/instances` endpoint)
#[tauri::command]
#[specta::specta]
pub async fn get_relay_instances(state: State<'_, AppState>) -> Result<Vec<RelayPeer>, String> {
    relay::get_relay_instances(&state).await
}
