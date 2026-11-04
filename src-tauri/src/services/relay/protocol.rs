//! The relay wire protocol, from the `nandi-protocol` crate
//! (`vaultdbai/nandi-spec`): JSON text frames, one `RelayMessage` each,
//! tagged by `type`. The client trait is in `client_api`.

pub use nandi_protocol::directory::{MemberRecord, PeopleMatch, ASK_ME};
pub use nandi_protocol::message::{
    DutyState, PeerDelivery, PeerMessage, RelayMessage, TaskAssigned, TaskCompleted, TaskFailed,
    ALL_CAPABILITIES, VERSION_MISMATCH,
};
pub use nandi_protocol::presence::{CapabilityPresence, InstancePresence, ProviderPresence};
pub use nandi_protocol::PROTOCOL_VERSION;
