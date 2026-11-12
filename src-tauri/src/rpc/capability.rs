//! Capability commands (Tauri adapter for `services::capability_index`).

use crate::services::capability_index;
use crate::services::domain::AgentCard;

super::commands! {
    /// Local agents serving `capability`
    fn find_agents_by_capability(state, capability: String) -> Vec<AgentCard> => capability_index::find_agents_by_capability(&state, &capability).await;

    /// Every capability a local agent serves
    fn list_declared_capabilities(state) -> Vec<String> => capability_index::list_declared_capabilities(&state).await;

    fn get_agent_card(state, agent_id: String) -> AgentCard => capability_index::get_agent_card(&state, &agent_id).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::domain::{Agent, AgentId, EntityKind};
    use crate::services::repos::Repository;
    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    #[tokio::test]
    async fn serves_capabilities_and_cards() {
        let state = AppState::for_tests(vec![]);
        let agent = Agent::from_skill_md(
            AgentId::parse("papers").unwrap(),
            "---\nname: Papers\nserves: [{name: find-papers}]\n---\n",
        )
        .unwrap();
        state.store().repos().agents.save(&agent).unwrap();
        state.store().commit(EntityKind::Agent, "papers").unwrap();

        let declared = call(&state, "list_declared_capabilities", json!({})).await;
        assert_eq!(declared, Ok(json!(["find-papers"])));

        let agents = call(
            &state,
            "find_agents_by_capability",
            json!({ "capability": "find-papers" }),
        )
        .await
        .unwrap();
        assert_eq!(agents[0]["name"], "Papers");
        assert_eq!(agents[0]["capabilities"][0]["name"], "find-papers");

        let card = call(&state, "get_agent_card", json!({ "agentId": "papers" }))
            .await
            .unwrap();
        assert!(card["id"].as_str().unwrap().ends_with("/papers"));

        let ghost = call(&state, "get_agent_card", json!({ "agentId": "ghost" })).await;
        assert!(ghost.unwrap_err().contains("not found"));
        state.shutdown();
    }
}
