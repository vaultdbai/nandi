//! Marketplace commands (Tauri adapter for `services::marketplace`).

use crate::services::marketplace::consent::ConsentStatus;
use crate::services::marketplace::install::{
    AppliedUpdate, InstalledAgentInfo, InstalledItem, InstalledPackInfo, UpdateAvailable,
};
use crate::services::marketplace::manifest::{AgentManifest, PackManifest};
use crate::services::marketplace::{
    self, AddRegistryInput, AgentDetail, CommunityRegistry, ForkedAgent, MarketplaceFilter,
    PackDetail, PublishAgentInput, PublishResult, RegistryInfo,
};

super::commands! {
    fn list_available_packs(state) -> Vec<PackManifest> => marketplace::list_available_packs(&state).await;

    fn list_marketplace_packs(state, filter: MarketplaceFilter) -> Vec<PackManifest> => marketplace::list_packs(&state, &filter).await;

    fn list_available_agents(state, filter: MarketplaceFilter) -> Vec<AgentManifest> => marketplace::list_available_agents(&state, &filter).await;

    fn get_agent_detail(state, agent_id: String) -> AgentDetail => marketplace::get_agent_detail(&state, &agent_id).await;

    fn get_pack_detail(state, pack_id: String) -> PackDetail => marketplace::get_pack_detail(&state, &pack_id).await;

    fn install_pack(state, pack_id: String) -> InstalledPackInfo => marketplace::install_pack(&state, &pack_id).await;

    /// What a pack in a local folder or `.zip` would install, for the consent
    /// dialog (unverified, registry `local`). Needs no internet or relay.
    fn inspect_local_pack(state, path: String) -> PackManifest => marketplace::inspect_local_pack(&state, &path).await;

    /// Installs the pack in a local folder or `.zip` through the same checks
    /// as a registry pack, recorded as an unverified local install
    fn install_local_pack(state, path: String) -> InstalledPackInfo => marketplace::install_local_pack(&state, &path).await;

    fn install_agent(state, agent_id: String) -> InstalledAgentInfo => marketplace::install_agent(&state, &agent_id).await;

    fn list_installed(state) -> Vec<InstalledItem> => marketplace::list_installed(&state).await;

    fn uninstall_installed(state, kind: String, id: String) -> () => marketplace::uninstall(&state, &kind, &id).await;

    fn fork_installed_agent(state, installed_id: String, new_id: String, new_name: String) -> ForkedAgent => marketplace::fork_agent(&state, &installed_id, &new_id, &new_name).await;

    fn check_for_updates(state) -> Vec<UpdateAvailable> => marketplace::check_for_updates(&state).await;

    fn apply_update(state, installed_id: String, new_ref: String) -> AppliedUpdate => marketplace::apply_update(&state, &installed_id, &new_ref).await;

    fn get_agent_consent(state, agent_id: String) -> ConsentStatus => marketplace::get_agent_consent(&state, &agent_id).await;

    fn grant_agent_consent(state, agent_id: String, categories: Vec<String>) -> ConsentStatus => marketplace::grant_agent_consent(&state, &agent_id, categories).await;

    fn list_registries(state) -> Vec<RegistryInfo> => marketplace::list_registries(&state).await;

    fn add_registry(state, input: AddRegistryInput) -> RegistryInfo => marketplace::add_registry(&state, input).await;

    fn create_community_registry(state) -> CommunityRegistry => marketplace::create_community_registry(&state).await;

    fn publish_agent(state, input: PublishAgentInput) -> PublishResult => marketplace::publish_agent(&state, input).await;

    fn remove_registry(state, url: String) -> () => marketplace::remove_registry(&state, &url).await;
}
