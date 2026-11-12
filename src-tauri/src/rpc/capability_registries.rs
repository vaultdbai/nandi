//! Capability registry commands (Tauri adapter for
//! `services::capability::registry`). Not the marketplace registries.

use crate::services::capability::registry;
use crate::services::capability::registry::{CapabilityRegistry, RegistryKind, SyncResult};

super::commands! {
    fn list_capability_registries(state) -> Vec<CapabilityRegistry> => registry::list_capability_registries(&state).await;

    /// Subscribes and does the first sync. `kind`: `None` detects it from the URL.
    fn add_capability_registry(state, url: String, kind: Option<RegistryKind>, branch_prefix: Option<String>, user_slug: Option<String>) -> CapabilityRegistry => registry::add_capability_registry(&state, &url, kind, branch_prefix, user_slug).await;

    /// Unsubscribes and drops its capabilities
    fn remove_capability_registry(state, url: String) -> () => registry::remove_capability_registry(&state, &url).await;

    /// Fetches (read-only) and re-indexes one registry
    fn sync_capability_registry(state, url: String) -> SyncResult => registry::sync_capability_registry(&state, &url).await;
}
