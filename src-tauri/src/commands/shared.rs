//! Shared-agent and trust commands (Tauri adapter for `services::shared`).

use tauri::State;

pub use crate::services::shared::{CapabilityProviders, SharedAgent};
pub use crate::services::trust::{TrustPolicy, TrustRule};

use crate::services::shared;
use crate::services::AppState;

/// From my teams' registries and the relay
#[tauri::command]
#[specta::specta]
pub async fn list_agents_shared_with_me(
    state: State<'_, AppState>,
) -> Result<Vec<SharedAgent>, String> {
    shared::list_agents_shared_with_me(&state).await
}

/// Mine that accept delegation or are shared to a team
#[tauri::command]
#[specta::specta]
pub async fn list_agents_shared_by_me(
    state: State<'_, AppState>,
) -> Result<Vec<SharedAgent>, String> {
    shared::list_agents_shared_by_me(&state).await
}

/// Per capability: who serves it, ranked as delegation would pick, and my
/// pin. `capability` narrows it to one.
#[tauri::command]
#[specta::specta]
pub async fn list_capability_providers(
    state: State<'_, AppState>,
    capability: Option<String>,
) -> Result<Vec<CapabilityProviders>, String> {
    shared::list_capability_providers(&state, capability.as_deref()).await
}

/// Pins a provider card for a capability (my settings.yaml); `None` unpins
#[tauri::command]
#[specta::specta]
pub async fn set_provider_pin(
    state: State<'_, AppState>,
    capability: String,
    card_id: Option<String>,
) -> Result<(), String> {
    shared::set_provider_pin(&state, &capability, card_id.as_deref()).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_trust_policies(state: State<'_, AppState>) -> Result<Vec<TrustRule>, String> {
    shared::list_trust_policies(&state).await
}

/// `requester`: a card ID or `*`; `provider_agent`: a local agent ID
#[tauri::command]
#[specta::specta]
pub async fn set_trust_policy(
    state: State<'_, AppState>,
    requester: String,
    provider_agent: String,
    policy: TrustPolicy,
) -> Result<(), String> {
    shared::set_trust_policy(&state, &requester, &provider_agent, policy).await
}
