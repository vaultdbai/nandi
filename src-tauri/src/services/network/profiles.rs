//! Member profiles by instance (wave EF): the gateway card, recent
//! delegations, and our trust toward them. The profile itself is built by
//! `team::roster` (shared with team rosters); this is the network-side
//! entry point.

use crate::services::team::roster::{self, MemberProfile};
use crate::services::AppState;

/// One member's profile, by instance ID
pub async fn get_member_profile(
    state: &AppState,
    instance_id: &str,
) -> Result<MemberProfile, String> {
    roster::get_member_by_instance(state, instance_id).await
}
