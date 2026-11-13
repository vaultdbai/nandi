//! Capability definition commands (`services::capability`). The
//! declared-capability lookups are in `rpc/capability.rs`.

use crate::services::capability;
use crate::services::capability::{
    AmbiguityCluster, CapabilityDefinition, CapabilityEvidence, CapabilityListing, CapabilityOwner,
    CapabilityProvider, CapabilityProviderStats,
};

super::commands! {
    /// Every capability this instance knows: local, from registries, and
    /// seen on the network, with how this instance relates to each
    fn list_capabilities(state) -> Vec<CapabilityListing> => capability::list_capabilities(&state).await;

    fn get_capability(state, id: String) -> CapabilityDefinition => capability::get_capability(&state, &id).await;

    /// Creates or replaces `.nandi/capabilities/<id>.yaml`; returns it as saved
    fn save_capability(state, definition: CapabilityDefinition) -> CapabilityDefinition => capability::save_capability(&state, definition).await;

    fn delete_capability(state, id: String) -> () => capability::delete_capability(&state, &id).await;

    fn transfer_capability_ownership(state, id: String, new_owner: CapabilityOwner, reason: Option<String>) -> CapabilityDefinition => capability::transfer_capability_ownership(&state, &id, new_owner, reason).await;

    /// Agents and workflows providing `id`, here and on other instances. (Not
    /// `list_capability_providers`, the Shared-with-me ranking.)
    fn list_providers_for_capability(state, id: String) -> Vec<CapabilityProvider> => capability::list_providers_for_capability(&state, &id).await;

    /// `window_days`: `None` is 30
    fn get_capability_evidence(state, id: String, window_days: Option<u32>) -> Vec<CapabilityEvidence> => capability::get_capability_evidence(&state, &id, window_days).await;

    fn get_capability_provider_breakdown(state, id: String) -> Vec<CapabilityProviderStats> => capability::get_capability_provider_breakdown(&state, &id).await;

    fn list_capability_ambiguities(state, id: String) -> Vec<AmbiguityCluster> => capability::list_capability_ambiguities(&state, &id).await;

    /// Adds an input-schema field from the cluster; returns the definition
    fn promote_ambiguity_to_schema(state, id: String, cluster_id: String) -> CapabilityDefinition => capability::promote_ambiguity_to_schema(&state, &id, &cluster_id).await;
}
