//! Capability presence on the relay (Tauri adapter for
//! `services::relay::presence`).

use crate::services::relay::presence;
use crate::services::relay::protocol::InstancePresence;

super::commands! {
    /// Providers of `capability_id` the relay knows (`*`: every capability)
    fn query_capability_presence(state, capability_id: String) -> Vec<InstancePresence> => presence::query_capability_presence(&state, &capability_id).await;
}
