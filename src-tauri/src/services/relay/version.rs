//! Which protocol version this Nandi speaks with a relay, and which side
//! must update when the two can't talk.
//!
//! A relay accepts the client versions in its range
//! (`min_supported_protocol..=protocol_version` on `/health`, see
//! [`nandi_protocol::ProtocolRange`]). Each release talks to the one
//! before it in both directions: a relay one version newer takes this
//! Nandi as it is, and with a relay one version older this Nandi speaks the
//! relay's version and hides what that relay can't do yet
//! ([`super::features`]).
//!
//! The client reads `/health` before each dial and registers with the
//! version [`spoken_with`] picks. Joining checks the same before it saves
//! the URL; after a `version_mismatch` the client reads it again to say
//! which side to update.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::protocol::PROTOCOL_VERSION;
use nandi_protocol::{ProtocolRange, RelayHealth, VersionGap};

/// The side that must update before this Nandi and a relay can talk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum UpdateNeeded {
    /// This Nandi is older than the oldest version the relay accepts
    Nandi,
    /// The relay is older than the oldest version this Nandi speaks: its
    /// operator must update it
    Relay,
}

/// The protocol version this Nandi speaks with a relay whose `/health` is
/// `health`: ours when the relay accepts it, else the relay's own when
/// this Nandi still speaks it (one version back). `Err`: the side that
/// must update.
pub fn spoken_with(health: &RelayHealth) -> Result<u32, UpdateNeeded> {
    let relay = health.protocol_range();
    match relay.mismatch(PROTOCOL_VERSION) {
        None => Ok(PROTOCOL_VERSION),
        Some(VersionGap::PeerTooOld) => Err(UpdateNeeded::Nandi),
        Some(VersionGap::PeerTooNew) if ProtocolRange::OURS.accepts(relay.current) => {
            Ok(relay.current)
        }
        Some(VersionGap::PeerTooNew) => Err(UpdateNeeded::Relay),
    }
}

/// Which side must update for this Nandi and a relay with this `/health`
/// to talk, or `None` when they can
pub fn update_needed(health: &RelayHealth) -> Option<UpdateNeeded> {
    spoken_with(health).err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nandi_protocol::{oldest_supported, MIN_SUPPORTED_PROTOCOL};

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
    fn the_two_ranges_decide_the_version_spoken_or_who_updates() {
        let ours = PROTOCOL_VERSION;
        // A relay one release back speaks what we accept from it
        let back = MIN_SUPPORTED_PROTOCOL;
        let too_old = MIN_SUPPORTED_PROTOCOL - 1;
        // (relay's min_supported, relay's current, what this Nandi does)
        let cases = [
            (ours, ours, Ok(ours)),
            (ours - 1, ours, Ok(ours)),
            // A newer relay takes us as we are
            (ours, ours + 1, Ok(ours)),
            (ours, ours + 2, Ok(ours)),
            (ours + 1, ours + 2, Err(UpdateNeeded::Nandi)),
            // An older relay in our window: we speak its version
            (oldest_supported(back), back, Ok(back)),
            (oldest_supported(too_old), too_old, Err(UpdateNeeded::Relay)),
        ];
        for (min, current, expected) in cases {
            let relay = health(min, current);
            assert_eq!(
                spoken_with(&relay),
                expected,
                "relay {min}..={current}, app {ours}"
            );
            assert_eq!(update_needed(&relay), expected.err());
        }
    }
}
