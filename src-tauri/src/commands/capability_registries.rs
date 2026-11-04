//! Capability registry commands (Tauri adapter for
//! `services::capability::registry`). Not the marketplace registries.

use tauri::State;

pub use crate::services::capability::registry::{CapabilityRegistry, RegistryKind, SyncResult};

use crate::services::capability::registry;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_capability_registries(
    state: State<'_, AppState>,
) -> Result<Vec<CapabilityRegistry>, String> {
    registry::list_capability_registries(&state).await
}

/// Subscribes and does the first sync. `kind`: `None` detects it from the URL.
#[tauri::command]
#[specta::specta]
pub async fn add_capability_registry(
    state: State<'_, AppState>,
    url: String,
    kind: Option<RegistryKind>,
    branch_prefix: Option<String>,
    user_slug: Option<String>,
) -> Result<CapabilityRegistry, String> {
    registry::add_capability_registry(&state, &url, kind, branch_prefix, user_slug).await
}

/// Unsubscribes and drops its capabilities
#[tauri::command]
#[specta::specta]
pub async fn remove_capability_registry(
    state: State<'_, AppState>,
    url: String,
) -> Result<(), String> {
    registry::remove_capability_registry(&state, &url).await
}

/// Fetches (read-only) and re-indexes one registry
#[tauri::command]
#[specta::specta]
pub async fn sync_capability_registry(
    state: State<'_, AppState>,
    url: String,
) -> Result<SyncResult, String> {
    registry::sync_capability_registry(&state, &url).await
}
