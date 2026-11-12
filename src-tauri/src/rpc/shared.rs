//! Shared-agent and trust commands (Tauri adapter for `services::shared`).

use crate::services::shared;
use crate::services::shared::{CapabilityProviders, SharedAgent};
use crate::services::trust::{TrustPolicy, TrustRule};

super::commands! {
    /// From my teams' registries and the relay
    fn list_agents_shared_with_me(state) -> Vec<SharedAgent> => shared::list_agents_shared_with_me(&state).await;

    /// Mine that accept delegation or are shared to a team
    fn list_agents_shared_by_me(state) -> Vec<SharedAgent> => shared::list_agents_shared_by_me(&state).await;

    /// Per capability: who serves it, ranked as delegation would pick, and my
    /// pin. `capability` narrows it to one.
    fn list_capability_providers(state, capability: Option<String>) -> Vec<CapabilityProviders> => shared::list_capability_providers(&state, capability.as_deref()).await;

    /// Pins a provider card for a capability (my settings.yaml); `None` unpins
    fn set_provider_pin(state, capability: String, card_id: Option<String>) -> () => shared::set_provider_pin(&state, &capability, card_id.as_deref()).await;

    fn list_trust_policies(state) -> Vec<TrustRule> => shared::list_trust_policies(&state).await;

    /// `requester`: a card ID or `*`; `provider_agent`: a local agent ID
    fn set_trust_policy(state, requester: String, provider_agent: String, policy: TrustPolicy) -> () => shared::set_trust_policy(&state, &requester, &provider_agent, policy).await;
}
