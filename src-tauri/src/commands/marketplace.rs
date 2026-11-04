//! Marketplace commands (Tauri adapter for `services::marketplace`).

use tauri::State;

use crate::services::marketplace::consent::ConsentStatus;
use crate::services::marketplace::install::{
    AppliedUpdate, InstalledAgentInfo, InstalledItem, InstalledPackInfo, UpdateAvailable,
};
use crate::services::marketplace::manifest::{AgentManifest, PackManifest};
use crate::services::marketplace::{
    self, AddRegistryInput, AgentDetail, CommunityRegistry, ForkedAgent, MarketplaceFilter,
    PackDetail, PublishAgentInput, PublishResult, RegistryInfo,
};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_available_packs(state: State<'_, AppState>) -> Result<Vec<PackManifest>, String> {
    marketplace::list_available_packs(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_marketplace_packs(
    state: State<'_, AppState>,
    filter: MarketplaceFilter,
) -> Result<Vec<PackManifest>, String> {
    marketplace::list_packs(&state, &filter).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_available_agents(
    state: State<'_, AppState>,
    filter: MarketplaceFilter,
) -> Result<Vec<AgentManifest>, String> {
    marketplace::list_available_agents(&state, &filter).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_detail(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<AgentDetail, String> {
    marketplace::get_agent_detail(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_pack_detail(
    state: State<'_, AppState>,
    pack_id: String,
) -> Result<PackDetail, String> {
    marketplace::get_pack_detail(&state, &pack_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn install_pack(
    state: State<'_, AppState>,
    pack_id: String,
) -> Result<InstalledPackInfo, String> {
    marketplace::install_pack(&state, &pack_id).await
}

/// What a pack in a local folder or `.zip` would install, for the consent
/// dialog (unverified, registry `local`). Needs no internet or relay.
#[tauri::command]
#[specta::specta]
pub async fn inspect_local_pack(
    state: State<'_, AppState>,
    path: String,
) -> Result<PackManifest, String> {
    marketplace::inspect_local_pack(&state, &path).await
}

/// Installs the pack in a local folder or `.zip` through the same checks
/// as a registry pack, recorded as an unverified local install
#[tauri::command]
#[specta::specta]
pub async fn install_local_pack(
    state: State<'_, AppState>,
    path: String,
) -> Result<InstalledPackInfo, String> {
    marketplace::install_local_pack(&state, &path).await
}

#[tauri::command]
#[specta::specta]
pub async fn install_agent(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<InstalledAgentInfo, String> {
    marketplace::install_agent(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_installed(state: State<'_, AppState>) -> Result<Vec<InstalledItem>, String> {
    marketplace::list_installed(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn uninstall_installed(
    state: State<'_, AppState>,
    kind: String,
    id: String,
) -> Result<(), String> {
    marketplace::uninstall(&state, &kind, &id).await
}

#[tauri::command]
#[specta::specta]
pub async fn fork_installed_agent(
    state: State<'_, AppState>,
    installed_id: String,
    new_id: String,
    new_name: String,
) -> Result<ForkedAgent, String> {
    marketplace::fork_agent(&state, &installed_id, &new_id, &new_name).await
}

#[tauri::command]
#[specta::specta]
pub async fn check_for_updates(state: State<'_, AppState>) -> Result<Vec<UpdateAvailable>, String> {
    marketplace::check_for_updates(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn apply_update(
    state: State<'_, AppState>,
    installed_id: String,
    new_ref: String,
) -> Result<AppliedUpdate, String> {
    marketplace::apply_update(&state, &installed_id, &new_ref).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_agent_consent(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<ConsentStatus, String> {
    marketplace::get_agent_consent(&state, &agent_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn grant_agent_consent(
    state: State<'_, AppState>,
    agent_id: String,
    categories: Vec<String>,
) -> Result<ConsentStatus, String> {
    marketplace::grant_agent_consent(&state, &agent_id, categories).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_registries(state: State<'_, AppState>) -> Result<Vec<RegistryInfo>, String> {
    marketplace::list_registries(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn add_registry(
    state: State<'_, AppState>,
    input: AddRegistryInput,
) -> Result<RegistryInfo, String> {
    marketplace::add_registry(&state, input).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_community_registry(
    state: State<'_, AppState>,
) -> Result<CommunityRegistry, String> {
    marketplace::create_community_registry(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn publish_agent(
    state: State<'_, AppState>,
    input: PublishAgentInput,
) -> Result<PublishResult, String> {
    marketplace::publish_agent(&state, input).await
}

#[tauri::command]
#[specta::specta]
pub async fn remove_registry(state: State<'_, AppState>, url: String) -> Result<(), String> {
    marketplace::remove_registry(&state, &url).await
}
