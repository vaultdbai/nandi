//! Roster, member profile, and trust commands (Tauri adapter for
//! `services::team::{roster, trust}`).

use crate::services::network::peer_keys;
use crate::services::network::peer_keys::PeerKeyStatus;
use crate::services::team::roster::{MemberProfile, TeamRoster};
use crate::services::team::trust::{TrustRelationship, TrustUpdate};
use crate::services::team::{roster, trust, trust_requests};

super::commands! {
    fn get_team_roster(state, team_id: String) -> TeamRoster => roster::get_team_roster(&state, &team_id).await;

    fn get_member_profile(state, team_id: String, handle: String) -> MemberProfile => roster::get_member_profile(&state, &team_id, &handle).await;

    fn get_member_by_instance(state, instance_id: String) -> MemberProfile => roster::get_member_by_instance(&state, &instance_id).await;

    /// Accepts a member's new key ("Their key changed"): sending to them works
    /// again
    fn accept_peer_key(state, instance_id: String) -> PeerKeyStatus => peer_keys::accept(state.db(), &instance_id);

    /// "Trust everyone on my company's relay", and whether it applies
    fn get_relay_trust(state) -> trust::RelayTrust => Ok(trust::relay_trust(state.db()));

    fn set_relay_trust(state, enabled: bool) -> trust::RelayTrust => { trust::set_trusts_relay_members(state.db(), enabled)?; Ok(trust::relay_trust(state.db())) };

    fn set_trust_relationship(state, instance_id: String, update: TrustUpdate) -> TrustRelationship => trust::set_trust_relationship(&state, &instance_id, update).await;

    fn block_agent_from_member(state, instance_id: String, agent_id: String) -> TrustRelationship => trust::block_agent_from_member(&state, &instance_id, &agent_id).await;

    fn unblock_agent_from_member(state, instance_id: String, agent_id: String) -> TrustRelationship => trust::unblock_agent_from_member(&state, &instance_id, &agent_id).await;

    /// People refused because this Nandi doesn't trust them yet
    fn list_trust_requests(state) -> Vec<trust_requests::TrustRequest> => trust_requests::list_trust_requests(&state).await;

    /// Trust them, or not now (`block`: refuse them from now on)
    fn respond_trust_request(state, instance_id: String, answer: trust_requests::TrustAnswer, block: bool) -> () => trust_requests::respond_trust_request(&state, &instance_id, answer, block).await;

    fn pause_delegations_from_member(state, instance_id: String, paused: bool) -> TrustRelationship => trust::pause_delegations_from_member(&state, &instance_id, paused).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    /// A member's trust is set by instance and shows on their profile; a
    /// task this instance isn't serving can't be reassigned
    #[tokio::test]
    async fn trust_is_set_by_instance_and_a_foreign_task_is_not_reassigned() {
        let state = AppState::for_tests(vec![]);
        let snapshot = call(&state, "get_network_snapshot", json!({}))
            .await
            .unwrap();
        assert_eq!(snapshot["members"], json!([]));

        let update = json!({
            "instanceId": "bob-1",
            "update": { "addedAsTrustedPeer": null, "delegationsPaused": true },
        });
        let trust = call(&state, "set_trust_relationship", update)
            .await
            .unwrap();
        assert_eq!(trust["delegationsPaused"], true);
        let profile = call(
            &state,
            "get_member_by_instance",
            json!({ "instanceId": "bob-1" }),
        )
        .await
        .unwrap();
        assert_eq!(profile["trust"]["delegationsPaused"], true);

        let request = json!({ "request": { "taskId": "nope", "newProviderInstance": "bob-1" } });
        assert!(call(&state, "reassign_task", request).await.is_err());
        state.shutdown();
    }
}
