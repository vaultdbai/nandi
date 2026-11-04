//! Capability presence on the relay (Tauri adapter for
//! `services::relay::presence`).

use tauri::State;

pub use crate::services::relay::protocol::InstancePresence;

use crate::services::relay::presence;
use crate::services::AppState;

/// Providers of `capability_id` the relay knows (`*`: every capability)
#[tauri::command]
#[specta::specta]
pub async fn query_capability_presence(
    state: State<'_, AppState>,
    capability_id: String,
) -> Result<Vec<InstancePresence>, String> {
    presence::query_capability_presence(&state, &capability_id).await
}
