//! Capability definition commands (Tauri adapter for `services::capability`).
//! The declared-capability lookups of wave 2A stay in `commands/capability.rs`.

use tauri::State;

pub use crate::services::capability::{
    AmbiguityCluster, CapabilityDefinition, CapabilityEvidence, CapabilityListing, CapabilityOwner,
    CapabilityProvider, CapabilityProviderStats,
};

use crate::services::capability;
use crate::services::AppState;

/// Every capability this instance knows: local, from registries, and
/// seen on the network, with how this instance relates to each
#[tauri::command]
#[specta::specta]
pub async fn list_capabilities(
    state: State<'_, AppState>,
) -> Result<Vec<CapabilityListing>, String> {
    capability::list_capabilities(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_capability(
    state: State<'_, AppState>,
    id: String,
) -> Result<CapabilityDefinition, String> {
    capability::get_capability(&state, &id).await
}

/// Creates or replaces `.nandi/capabilities/<id>.yaml`; returns it as saved
#[tauri::command]
#[specta::specta]
pub async fn save_capability(
    state: State<'_, AppState>,
    definition: CapabilityDefinition,
) -> Result<CapabilityDefinition, String> {
    capability::save_capability(&state, definition).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_capability(state: State<'_, AppState>, id: String) -> Result<(), String> {
    capability::delete_capability(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn transfer_capability_ownership(
    state: State<'_, AppState>,
    id: String,
    new_owner: CapabilityOwner,
    reason: Option<String>,
) -> Result<CapabilityDefinition, String> {
    capability::transfer_capability_ownership(&state, &id, new_owner, reason).await
}

/// Agents and workflows providing `id`, here and on other instances. (Not
/// `list_capability_providers`, the Shared-with-me ranking.)
#[tauri::command]
#[specta::specta]
pub async fn list_providers_for_capability(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<CapabilityProvider>, String> {
    capability::list_providers_for_capability(&state, &id).await
}

/// `window_days`: `None` is 30
#[tauri::command]
#[specta::specta]
pub async fn get_capability_evidence(
    state: State<'_, AppState>,
    id: String,
    window_days: Option<u32>,
) -> Result<Vec<CapabilityEvidence>, String> {
    capability::get_capability_evidence(&state, &id, window_days).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_capability_provider_breakdown(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<CapabilityProviderStats>, String> {
    capability::get_capability_provider_breakdown(&state, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_capability_ambiguities(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<AmbiguityCluster>, String> {
    capability::list_capability_ambiguities(&state, &id).await
}

/// Adds an input-schema field from the cluster; returns the definition
#[tauri::command]
#[specta::specta]
pub async fn promote_ambiguity_to_schema(
    state: State<'_, AppState>,
    id: String,
    cluster_id: String,
) -> Result<CapabilityDefinition, String> {
    capability::promote_ambiguity_to_schema(&state, &id, &cluster_id).await
}
