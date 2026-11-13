//! Network directory commands (Tauri adapter for `services::network`).

use crate::services::network;
use crate::services::network::NetworkSnapshot;

super::commands! {
    fn get_network_snapshot(state) -> NetworkSnapshot => network::get_network_snapshot(&state).await;

    fn refresh_network(state) -> NetworkSnapshot => network::refresh_network(&state).await;
}
