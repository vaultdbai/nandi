//! Relay commands (Tauri adapter for `services::relay`).

use crate::services::domain::AgentCard;
use crate::services::relay;
use crate::services::relay::join::{JoinOutcome, RelayInvite};
use crate::services::relay::public::{PublicRelayAccount, PublicRelayJoin, PublicRelayOffer};
use crate::services::relay::{RelayPeer, RelayStatus};

super::commands! {
    fn get_relay_status(state) -> RelayStatus => relay::get_relay_status(&state).await;

    /// Saves the relay URL and reconnects
    fn set_relay_url(state, url: String) -> () => relay::set_relay_url(&state, &url).await;

    /// "Join": joins the relay at `url`, enrolling with `join_code` (or the
    /// build's own code for the build's relay) when it is secured
    fn join_relay(state, url: String, join_code: Option<String>) -> JoinOutcome => relay::join::join_relay(&state, &url, join_code.as_deref()).await;

    /// What "Copy invite message" passes on (`None`: no relay in use)
    fn get_relay_invite(state) -> Option<RelayInvite> => relay::join::relay_invite(&state);

    /// "Join the public relay": whether it can be joined now, and as whom
    fn get_public_relay_offer(state) -> PublicRelayOffer => Ok(relay::public::offer(&state).await);

    /// Joins the public relay with this Nandi's connected email; on `joined`
    /// the app keeps the token and connects
    fn join_public_relay(state) -> PublicRelayJoin => relay::public::join(&state).await;

    /// The public-relay account this Nandi joined as (the Account card)
    fn get_public_relay_account(state) -> PublicRelayAccount => relay::public::account(&state).await;

    /// Agents online at the relay, optionally only those serving `capability`
    fn list_online_agents(state, capability: Option<String>) -> Vec<AgentCard> => relay::list_online_agents(&state, capability.as_deref()).await;

    /// The instances at the relay (its `/instances` endpoint)
    fn get_relay_instances(state) -> Vec<RelayPeer> => relay::get_relay_instances(&state).await;

    /// "Test connection": WebSocket ping to `url`; the round trip in ms, or the
    /// exact error
    fn test_relay_connection(url: String) -> u32 => relay::test_relay_connection(&url).await;
}
