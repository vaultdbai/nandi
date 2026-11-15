//! What the connected relay can do, by the protocol version this Nandi
//! speaks with it ([`super::version::spoken_with`]).
//!
//! Each person-to-person feature needs the relay to read the message it
//! sends ([`RelayFeature::message`], whose
//! [`nandi_protocol::MessageKind::since`] is the version it came with). A
//! relay one version older may not have the newest ones yet. Such a
//! feature is hidden, never an error:
//!
//! - [`super::RelayStatus::missing_features`] lists them, so a screen shows
//!   "Your relay needs an update for this" in their place.
//! - The client never sends a message the relay can't read: the call
//!   answers [`RelayNeedsUpdate`] instead, which crosses the bridge as text
//!   that starts with [`RELAY_NEEDS_UPDATE`] (exported to `bindings.ts`).

use serde::{Deserialize, Serialize};
use specta::Type;

use nandi_protocol::MessageKind;

/// The stable code a [`RelayNeedsUpdate`] error starts with
pub const RELAY_NEEDS_UPDATE: &str = "relay_needs_update";

/// A person-to-person feature the relay may be too old for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RelayFeature {
    /// The member directory
    Directory,
    /// Finding who is responsible for a topic
    PeopleSearch,
    /// The relay's plan of who gets a request
    Routing,
    /// Asking or assigning someone, kept for them while they're away
    Requests,
    /// Handing a request on to someone else
    Reassignment,
    /// Calendar handovers to a backup
    Handover,
    /// Groups and their policies
    Groups,
}

impl RelayFeature {
    /// Every feature, in the order screens list them
    pub const ALL: [Self; 7] = [
        Self::Directory,
        Self::PeopleSearch,
        Self::Routing,
        Self::Requests,
        Self::Reassignment,
        Self::Handover,
        Self::Groups,
    ];

    /// The message the feature sends: a relay that reads it has the
    /// feature
    pub const fn message(self) -> MessageKind {
        match self {
            Self::Directory => MessageKind::QueryDirectory,
            Self::PeopleSearch => MessageKind::FindPeople,
            Self::Routing => MessageKind::RouteQuery,
            Self::Requests => MessageKind::SubmitTask,
            Self::Reassignment => MessageKind::Reassign,
            Self::Handover => MessageKind::DutyWatch,
            Self::Groups => MessageKind::GroupRequest,
        }
    }

    /// Whether a relay speaking `version` has it
    pub fn available_at(self, version: u32) -> bool {
        self.message().known_at(version)
    }

    /// The features a relay speaking `version` doesn't have yet
    pub fn missing_at(version: u32) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|feature| !feature.available_at(version))
            .collect()
    }
}

/// A person-to-person action the connected relay is too old for: its
/// operator must update it. The screen says so calmly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelayNeedsUpdate;

impl RelayNeedsUpdate {
    /// Whether `error` (a command's error text) is this one
    pub fn is(error: &str) -> bool {
        error.starts_with(RELAY_NEEDS_UPDATE)
    }
}

impl std::fmt::Display for RelayNeedsUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{RELAY_NEEDS_UPDATE}: your relay needs an update for this"
        )
    }
}

impl std::error::Error for RelayNeedsUpdate {}

impl From<RelayNeedsUpdate> for String {
    fn from(error: RelayNeedsUpdate) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nandi_protocol::PROTOCOL_VERSION;

    #[test]
    fn a_feature_is_there_from_the_version_its_message_came_with() {
        // (the relay's version, the features it doesn't have)
        let cases = [
            (1, RelayFeature::ALL.to_vec()),
            (
                2,
                vec![
                    RelayFeature::Routing,
                    RelayFeature::Requests,
                    RelayFeature::Reassignment,
                    RelayFeature::Handover,
                    RelayFeature::Groups,
                ],
            ),
            (
                5,
                vec![
                    RelayFeature::Requests,
                    RelayFeature::Reassignment,
                    RelayFeature::Handover,
                    RelayFeature::Groups,
                ],
            ),
            (6, vec![RelayFeature::Groups]),
            (7, vec![]),
            (PROTOCOL_VERSION, vec![]),
        ];
        for (version, missing) in cases {
            assert_eq!(
                RelayFeature::missing_at(version),
                missing,
                "relay at {version}"
            );
            for feature in RelayFeature::ALL {
                assert_eq!(
                    feature.available_at(version),
                    !missing.contains(&feature),
                    "{feature:?} at {version}"
                );
            }
        }
    }

    #[test]
    fn the_error_text_starts_with_the_stable_code() {
        let text = String::from(RelayNeedsUpdate);
        assert!(text.starts_with("relay_needs_update: "));
        assert!(RelayNeedsUpdate::is(&text));
        assert!(!RelayNeedsUpdate::is("relay_unavailable: not connected"));
    }
}
