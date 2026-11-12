//! Repo config commands (`services::config`).

use crate::services::config::repo_config;
use crate::services::config::repo_config::RepoLlmConfigStatus;

super::commands! {
    /// What `.nandi/llm.yaml` holds and what's in effect
    fn get_repo_llm_config(state) -> RepoLlmConfigStatus => repo_config::get_repo_llm_config(&state).await;

    /// Re-reads `.nandi/llm.yaml`
    fn reload_repo_llm_config(state) -> RepoLlmConfigStatus => repo_config::reload_repo_llm_config(&state).await;
}
