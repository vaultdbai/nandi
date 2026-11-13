//! Bedrock model catalog and LiteLLM gateway commands.

use crate::services::harness::bedrock_models::BedrockModel;
use crate::services::harness::gateway::LitellmGatewayStatus;
use crate::services::harness::{bedrock_models, gateway};

super::commands! {
    /// `region`: `None` is the AWS preferences' region
    fn list_bedrock_models(state, region: Option<String>) -> Vec<BedrockModel> => bedrock_models::list_bedrock_models(&state, region).await;

    fn refresh_bedrock_models(state, region: Option<String>) -> Vec<BedrockModel> => bedrock_models::refresh_bedrock_models(&state, region).await;

    /// Starts the gateway serving one third-party Bedrock model
    fn start_litellm_gateway(state, model: String) -> LitellmGatewayStatus => gateway::start_litellm_gateway(&state, &model).await;

    fn stop_litellm_gateway(state) -> LitellmGatewayStatus => gateway::stop_litellm_gateway(&state).await;

    fn get_litellm_gateway_status(state) -> LitellmGatewayStatus => gateway::get_litellm_gateway_status(&state).await;
}
