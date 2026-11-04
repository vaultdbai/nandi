//! The relay wire protocol, from the `nandi-protocol` crate
//! (`vaultdbai/nandi-spec`): JSON text frames, one `RelayMessage` each,
//! tagged by `type`. The client trait is in `client_api`.
//!
//! Person-to-person traffic is re-exported in its opened form (`opened`):
//! the rest of the app never sees an envelope. The relay client seals it
//! on the way out and opens it on the way in (`sealing`).

pub use super::opened::{DutyState, PeerDelivery, PeerMessage, TaskAssigned, TaskCompleted};
pub use nandi_protocol::directory::{MemberRecord, PeopleMatch, ASK_ME};
pub use nandi_protocol::message::{RelayMessage, TaskFailed, ALL_CAPABILITIES, VERSION_MISMATCH};
pub use nandi_protocol::presence::{CapabilityPresence, InstancePresence, ProviderPresence};
pub use nandi_protocol::routing::{
    NoRoute, RequestKind, RouteCandidate, RoutePlan, RouteReason, RouteTarget, RoutingHints,
    RoutingQuery, MAX_CANDIDATES,
};
pub use nandi_protocol::PROTOCOL_VERSION;
