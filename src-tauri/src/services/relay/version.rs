//! Whether a relay takes this Nandi's protocol version, and which side
//! must update when it doesn't.
//!
//! A relay accepts the client versions in its range
//! (`min_supported_protocol..=protocol_version` on `/health`, see
//! [`nandi_protocol::ProtocolRange`]). Joining checks the range before it
//! saves the URL; the client checks it again after the relay refuses a
//! registration with `version_mismatch`, to say which side to update.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::protocol::PROTOCOL_VERSION;
use nandi_protocol::{RelayHealth, VersionGap};

/// The side that must update before this Nandi and a relay can talk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UpdateNeeded {
    /// This Nandi is older than the oldest version the relay accepts
    Nandi,
    /// This Nandi is newer than the relay: its operator must update it
    Relay,
}

/// Which side must update for a relay with this `/health` to take this
/// Nandi, or `None` when it takes it as it is
pub fn update_needed(health: &RelayHealth) -> Option<UpdateNeeded> {
    let gap = health.protocol_range().mismatch(PROTOCOL_VERSION)?;
    Some(match gap {
        VersionGap::PeerTooOld => UpdateNeeded::Nandi,
        VersionGap::PeerTooNew => UpdateNeeded::Relay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(min_supported_protocol: u32, protocol_version: u32) -> RelayHealth {
        RelayHealth {
            status: "ok".into(),
            instances_connected: 0,
            uptime_seconds: 0,
            version: "1".into(),
            auth: "open".into(),
            tls: false,
            protocol_version,
            min_supported_protocol,
            visibility: Default::default(),
        }
    }

    #[test]
    fn the_relay_range_decides_who_updates() {
        let ours = PROTOCOL_VERSION;
        // (relay's min_supported, relay's current, the side to update)
        let cases = [
            (ours, ours, None),
            (ours - 1, ours, None),
            (ours, ours + 2, None),
            (ours + 1, ours + 2, Some(UpdateNeeded::Nandi)),
            (ours - 2, ours - 1, Some(UpdateNeeded::Relay)),
        ];
        for (min, current, expected) in cases {
            assert_eq!(
                update_needed(&health(min, current)),
                expected,
                "relay {min}..={current}, app {ours}"
            );
        }
    }
}
