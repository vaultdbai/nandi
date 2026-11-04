//! LLM API key commands (Tauri adapter for `services::llm_keys`).

use tauri::State;

use crate::services::llm_keys::{self, KeyedProvider, LlmKeyStatus};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn set_llm_api_key(
    state: State<'_, AppState>,
    provider: KeyedProvider,
    api_key: String,
) -> Result<(), String> {
    llm_keys::set_llm_api_key(&state, provider, api_key).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_llm_key_status(state: State<'_, AppState>) -> Result<Vec<LlmKeyStatus>, String> {
    llm_keys::get_llm_key_status(&state).await
}

/// The default model and the model calls actually use with the keys set
#[tauri::command]
#[specta::specta]
pub async fn get_default_model(
    state: State<'_, AppState>,
) -> Result<llm_keys::DefaultModelStatus, String> {
    llm_keys::get_default_model(&state).await
}

/// Checks a key (`None`: the stored one) without storing it
#[tauri::command]
#[specta::specta]
pub async fn test_llm_key(
    state: State<'_, AppState>,
    provider: KeyedProvider,
    api_key: Option<String>,
) -> Result<llm_keys::LlmKeyTest, String> {
    llm_keys::test_llm_key(&state, provider, api_key).await
}

/// `provider`'s current models, fetched with its stored key, newest first
#[tauri::command]
#[specta::specta]
pub async fn list_provider_models(
    state: State<'_, AppState>,
    provider: KeyedProvider,
) -> Result<Vec<String>, String> {
    llm_keys::list_provider_models(&state, provider).await
}

/// Whether Ollama runs on this computer, and its models
#[tauri::command]
#[specta::specta]
pub async fn detect_ollama() -> Result<llm_keys::OllamaStatus, String> {
    Ok(llm_keys::detect_ollama().await)
}

/// `<provider>:<model>`, e.g. `claude:claude-sonnet-5`
#[tauri::command]
#[specta::specta]
pub async fn set_default_model(
    state: State<'_, AppState>,
    model: String,
) -> Result<llm_keys::DefaultModelStatus, String> {
    llm_keys::set_default_model(&state, model).await
}

/// Turns the free trial on (its model becomes the default) or off
#[tauri::command]
#[specta::specta]
pub async fn set_free_trial(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<llm_keys::DefaultModelStatus, String> {
    llm_keys::set_free_trial(&state, enabled).await
}
