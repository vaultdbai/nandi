//! Bedrock routes (`/api/bedrock/...`), mirroring `commands/bedrock.rs`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::harness::{bedrock_models, gateway};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?region=`; returns `BedrockModel[]`
        .route("/bedrock/models", get(models))
        // Body `{ region }`; returns `BedrockModel[]`
        .route("/bedrock/models/refresh", post(refresh))
        // Returns `GatewayStatus`
        .route("/bedrock/gateway", get(status))
        // Body `{ model }`; returns `GatewayStatus`
        .route("/bedrock/gateway/start", post(start))
        // Returns `GatewayStatus`
        .route("/bedrock/gateway/stop", post(stop))
}

#[derive(Deserialize)]
struct RegionQuery {
    region: Option<String>,
}

#[derive(Deserialize)]
struct StartBody {
    model: String,
}

async fn models(
    State(state): State<AppState>,
    Query(query): Query<RegionQuery>,
) -> impl IntoResponse {
    ApiResult(bedrock_models::list_bedrock_models(&state, query.region).await)
}

async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RegionQuery>,
) -> impl IntoResponse {
    ApiResult(bedrock_models::refresh_bedrock_models(&state, body.region).await)
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::get_litellm_gateway_status(&state).await)
}

async fn start(State(state): State<AppState>, Json(body): Json<StartBody>) -> impl IntoResponse {
    ApiResult(gateway::start_litellm_gateway(&state, &body.model).await)
}

async fn stop(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(gateway::stop_litellm_gateway(&state).await)
}
