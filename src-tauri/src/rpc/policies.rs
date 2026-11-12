//! Policy commands (`services::policies`).

use crate::services::domain::policy::PolicySummary;
use crate::services::policies;
use crate::services::policies::AgentPolicies;

super::commands! {
    /// The policies over an agent and what each changed, or why it can't run
    fn agent_policies(state, agent_id: String) -> AgentPolicies => policies::agent_policies(state.store(), &agent_id);

    /// Every policy in the repo by id and name, broken ones included
    fn list_policies(state) -> Vec<PolicySummary> => policies::list(state.store());
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_repos_policies_are_listed_by_id_broken_ones_marked() {
        let state = AppState::for_tests(vec![]);
        assert_eq!(
            call(&state, "list_policies", json!({})).await,
            Ok(json!([]))
        );

        let layout = state.store().layout().clone();
        let named = layout.policy_path("no-shell");
        std::fs::create_dir_all(named.parent().unwrap()).unwrap();
        std::fs::write(
            &named,
            "---\nname: No shell\napplies_to: ['*']\ntools: { deny: [shell] }\n---\n",
        )
        .unwrap();
        std::fs::write(
            layout.policy_path("broken"),
            "---\ntools: { nope: 1 }\n---\n",
        )
        .unwrap();

        assert_eq!(
            call(&state, "list_policies", json!({})).await,
            Ok(json!([
                { "id": "broken", "name": "broken", "valid": false },
                { "id": "no-shell", "name": "No shell", "valid": true },
            ]))
        );
        state.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_agents_policies_say_what_they_change_or_why_it_cannot_run() {
        let state = AppState::for_tests(vec![]);
        let layout = state.store().layout().clone();
        let skill = layout.skill_path("writer");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "---\nname: Writer\n---\nWrite.\n").unwrap();
        let policy = layout.policy_path("no-shell");
        std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
        std::fs::write(
            &policy,
            "---\napplies_to: ['*']\ntools: { deny: [shell] }\n---\n",
        )
        .unwrap();
        state.store().load().unwrap();
        let writer = json!({ "agentId": "writer" });

        let body = call(&state, "agent_policies", writer.clone())
            .await
            .unwrap();
        assert_eq!(body["policies"], json!(["no-shell"]));
        assert!(body["blocked"].is_null());
        let shell = body["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|change| change["item"] == "tool shell")
            .expect("the denied tool");
        assert_eq!(
            (shell["outcome"].as_str(), shell["by"].as_str()),
            (Some("removed"), Some("no-shell"))
        );

        // A broken policy blocks the agent; the page says why
        std::fs::write(&policy, "---\napplies_to: ['*']\ntools: { nope: 1 }\n---\n").unwrap();
        let body = call(&state, "agent_policies", writer).await.unwrap();
        assert!(body["blocked"].as_str().unwrap().contains("'no-shell'"));
        assert_eq!(body["changes"], json!([]));

        let ghost = call(&state, "agent_policies", json!({ "agentId": "ghost" })).await;
        assert!(ghost.unwrap_err().contains("not found"));
        state.shutdown();
    }
}
