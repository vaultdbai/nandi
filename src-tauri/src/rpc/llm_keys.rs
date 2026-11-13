//! LLM API key commands (Tauri adapter for `services::llm_keys`).

use crate::services::llm_keys::{self, KeyedProvider, LlmKeyStatus};
use crate::services::llm_trial::TrialAccount;

super::commands! {
    /// Whether Ollama runs on this computer, and its models
    fn detect_ollama() -> llm_keys::OllamaStatus => Ok(llm_keys::detect_ollama().await);

    fn set_llm_api_key(state, provider: KeyedProvider, api_key: String) -> () => llm_keys::set_llm_api_key(&state, provider, api_key).await;

    fn get_llm_key_status(state) -> Vec<LlmKeyStatus> => llm_keys::get_llm_key_status(&state).await;

    /// The default model and the model calls actually use with the keys set
    fn get_default_model(state) -> llm_keys::DefaultModelStatus => llm_keys::get_default_model(&state).await;

    /// Checks a key (`None`: the stored one) without storing it
    fn test_llm_key(state, provider: KeyedProvider, api_key: Option<String>) -> llm_keys::LlmKeyTest => llm_keys::test_llm_key(&state, provider, api_key).await;

    /// `provider`'s current models, fetched with its stored key, newest first
    fn list_provider_models(state, provider: KeyedProvider) -> Vec<String> => llm_keys::list_provider_models(&state, provider).await;

    /// `<provider>:<model>`, e.g. `claude:claude-sonnet-5`
    fn set_default_model(state, model: String) -> llm_keys::DefaultModelStatus => llm_keys::set_default_model(&state, model).await;

    /// Turns the free trial on (its model becomes the default) or off
    fn set_free_trial(state, enabled: bool) -> llm_keys::DefaultModelStatus => llm_keys::set_free_trial(&state, enabled).await;

    /// The free trial's allowance as the gateway counts it
    fn get_free_trial_account(state) -> TrialAccount => llm_keys::get_free_trial_account(&state).await;
}
