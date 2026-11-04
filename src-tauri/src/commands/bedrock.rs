//! Bedrock model catalog and LiteLLM gateway commands.

use tauri::State;

pub use crate::services::harness::bedrock_models::BedrockModel;
pub use crate::services::harness::gateway::LitellmGatewayStatus;

use crate::services::harness::{bedrock_models, gateway};
use crate::services::AppState;

/// `region`: `None` is the AWS preferences' region
#[tauri::command]
#[specta::specta]
pub async fn list_bedrock_models(
    state: State<'_, AppState>,
    region: Option<String>,
) -> Result<Vec<BedrockModel>, String> {
    bedrock_models::list_bedrock_models(&state, region).await
}

#[tauri::command]
#[specta::specta]
pub async fn refresh_bedrock_models(
    state: State<'_, AppState>,
    region: Option<String>,
) -> Result<Vec<BedrockModel>, String> {
    bedrock_models::refresh_bedrock_models(&state, region).await
}

/// Starts the gateway serving one third-party Bedrock model
#[tauri::command]
#[specta::specta]
pub async fn start_litellm_gateway(
    state: State<'_, AppState>,
    model: String,
) -> Result<LitellmGatewayStatus, String> {
    gateway::start_litellm_gateway(&state, &model).await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_litellm_gateway(
    state: State<'_, AppState>,
) -> Result<LitellmGatewayStatus, String> {
    gateway::stop_litellm_gateway(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_litellm_gateway_status(
    state: State<'_, AppState>,
) -> Result<LitellmGatewayStatus, String> {
    gateway::get_litellm_gateway_status(&state).await
}
