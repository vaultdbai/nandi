//! Pilot commands (`services::pilots`): install, run, and what each agent may use.

use crate::services::pilots;
use crate::services::pilots::{AgentPilotPermission, PilotDefinition, PilotStatus};

super::commands! {
    fn list_pilots() -> Vec<PilotDefinition> => Ok(pilots::list_pilots());

    fn install_pilot(state, id: String) -> () => pilots::install_pilot(&state, &id).await;

    /// Returns the pilot's process ID
    fn start_pilot(state, id: String) -> u32 => pilots::start_pilot(&state, &id).await;

    fn stop_pilot(state, id: String) -> () => pilots::stop_pilot(&state, &id).await;

    fn pilot_status(state, id: String) -> PilotStatus => pilots::pilot_status(&state, &id).await;

    fn grant_pilot_capability(state, agent_id: String, pilot_id: String, capability: String) -> () => pilots::grant_pilot_capability(&state, &agent_id, &pilot_id, &capability).await;

    fn revoke_pilot_capability(state, agent_id: String, pilot_id: String, capability: String) -> () => pilots::revoke_pilot_capability(&state, &agent_id, &pilot_id, &capability).await;

    /// Saves (or, empty, removes) a pilot secret the agent's skill.md names
    fn set_pilot_secret(state, agent_id: String, pilot_id: String, name: String, value: String) -> () => pilots::set_pilot_secret(&state, &agent_id, &pilot_id, &name, &value).await;

    /// Adds a site secret to the agent's browser: the name into skill.md, the
    /// value into the keychain
    fn add_pilot_secret(state, agent_id: String, pilot_id: String, name: String, value: String) -> () => pilots::add_pilot_secret(&state, &agent_id, &pilot_id, &name, &value).await;

    /// "Sign out everywhere": clears the agent's browser profile. `false`: it
    /// had none
    fn clear_browser_profile(state, agent_id: String, pilot_id: String) -> bool => pilots::clear_browser_profile(&state, &agent_id, &pilot_id).await;

    /// The pilots the agent's skill.md declares, and what the user granted each
    fn agent_pilot_permissions(state, agent_id: String) -> Vec<AgentPilotPermission> => pilots::agent_pilot_permissions(&state, &agent_id).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(name: &str, args: Value) -> Result<Value, String> {
        let state = AppState::for_tests(vec![]);
        let result = crate::rpc::call(state.clone(), name, args).await.unwrap();
        state.shutdown();
        result
    }

    #[tokio::test]
    async fn lists_this_platforms_pilots() {
        let pilots = call("list_pilots", json!({})).await.unwrap();
        let ids: Vec<&str> = pilots
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"opendesk"));
    }

    #[tokio::test]
    async fn errors_carry_the_message() {
        let unknown = call("pilot_status", json!({ "id": "nope" })).await;
        assert!(unknown.unwrap_err().contains("Unknown pilot"));

        let status = call("pilot_status", json!({ "id": "opendesk" })).await;
        assert_eq!(status.unwrap()["installed"], false);

        let grant = json!({ "agentId": "ghost", "pilotId": "opendesk", "capability": "browser" });
        let granted = call("grant_pilot_capability", grant).await;
        assert!(granted.unwrap_err().contains("not found"));

        let revoke = json!({ "agentId": "writer", "pilotId": "opendesk", "capability": "nope" });
        let revoked = call("revoke_pilot_capability", revoke).await;
        assert!(revoked.unwrap_err().contains("no capability"));

        let permissions = call("agent_pilot_permissions", json!({ "agentId": "ghost" })).await;
        assert!(permissions.is_err());
    }
}
