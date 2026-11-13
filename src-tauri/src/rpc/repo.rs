//! Repo commands (Tauri adapter for `services::repo_service`).

use crate::services::agents::AgentInfo;
use crate::services::domain::playbook::Playbook;
use crate::services::projection::ProjectionStats;
use crate::services::repo_service::{self, RepoStatus};
use crate::services::repo_service::{
    AgentDefinitionInput, AgentPlaybooks, GitStatus, PlaybookInput,
};

super::commands! {
    /// Wipes the local projection and rebuilds it from the repo files. Refuses
    /// while a file is invalid unless `force` (which skips invalid files).
    fn repo_rebuild(state, force: Option<bool>) -> ProjectionStats => repo_service::rebuild(&state, force.unwrap_or(false)).await;

    /// Counts, problems (invalid files, broken references), and whether edits
    /// apply live.
    fn repo_status(state) -> RepoStatus => repo_service::status(&state).await;

    /// Writes the agent's files (gold copy) and returns the saved agent.
    /// Creates the agent if it has no folder yet. Marketplace agents are
    /// read-only until forked.
    fn repo_save_agent(state, agent_id: String, input: AgentDefinitionInput) -> AgentInfo => repo_service::save_agent(&state, &agent_id, input).await;

    /// Replaces the agent's `instructions.md` (the user's rules). For an
    /// installed agent this is an override that updates keep.
    fn repo_save_agent_instructions(state, agent_id: String, instructions: String) -> () => repo_service::save_agent_instructions(&state, &agent_id, instructions).await;

    /// Back to an installed agent's shipped instructions; whether it had the
    /// user's own
    fn repo_reset_agent_instructions(state, agent_id: String) -> bool => repo_service::reset_agent_instructions(&state, &agent_id).await;

    /// The agent's playbooks (`skills/`), and whether they're read-only
    fn repo_agent_playbooks(state, agent_id: String) -> AgentPlaybooks => repo_service::agent_playbooks(&state, &agent_id).await;

    /// Validates and writes one playbook. `previous`: its old name when
    /// renamed or edited, `None` for a new one
    fn repo_save_agent_playbook(state, agent_id: String, input: PlaybookInput, previous: Option<String>) -> Playbook => repo_service::save_agent_playbook(&state, &agent_id, input, previous).await;

    /// Removes one playbook
    fn repo_delete_agent_playbook(state, agent_id: String, name: String) -> () => repo_service::delete_agent_playbook(&state, &agent_id, name).await;

    /// One entity as stored in its file. `kind`: `agent` | `workflow` |
    /// `trigger` | `connector` | `settings`
    fn repo_load_entity(state, kind: String, id: String) -> serde_json::Value => repo_service::load_entity(&state, &kind, &id).await;

    fn repo_git_status(state) -> GitStatus => repo_service::git_status(&state).await;

    /// Unified diff of one file against `HEAD`
    fn repo_git_diff(state, path: String) -> String => repo_service::git_diff(&state, &path).await;

    /// Stages everything and commits it; returns the commit hash. Never pushes
    /// (that's "Sync now").
    fn repo_git_commit(state, message: String) -> String => repo_service::git_commit(&state, &message).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    #[tokio::test]
    async fn save_load_status_diff_and_commit() {
        let state = AppState::for_tests(vec![]);
        git2::Repository::init(&state.store().layout().root).unwrap();

        let input = json!({ "name": "Writer", "skillBody": "Write.", "capabilities": { "tools": ["shell"] } });
        let saved = call(
            &state,
            "repo_save_agent",
            json!({ "agentId": "writer", "input": input }),
        );
        assert_eq!(saved.await.unwrap()["name"], "Writer");

        let entity = call(
            &state,
            "repo_load_entity",
            json!({ "kind": "agent", "id": "writer" }),
        );
        let entity = entity.await.unwrap();
        assert_eq!(entity["skillBody"], "Write.");
        assert_eq!(entity["capabilities"]["tools"][0], "shell");

        let status = call(&state, "repo_git_status", json!({})).await.unwrap();
        assert!(status["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == ".nandi/agents/writer/skill.md"));

        let path = ".nandi/agents/writer/skill.md";
        let diff = call(&state, "repo_git_diff", json!({ "path": path }))
            .await
            .unwrap();
        assert!(diff.as_str().unwrap().contains("+name: Writer"), "{diff}");

        let commit = call(
            &state,
            "repo_git_commit",
            json!({ "message": "Add writer" }),
        );
        assert_eq!(commit.await.unwrap().as_str().unwrap().len(), 40);

        // A path outside the repo is refused
        let outside = call(&state, "repo_git_diff", json!({ "path": "../outside" })).await;
        assert!(outside.is_err());
        state.shutdown();
    }
}
