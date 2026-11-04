//! Shared-agent and trust routes, mirroring `commands/shared.rs`. Errors
//! are `400 {"error": "..."}`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::shared;
use crate::services::trust::TrustPolicy;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `SharedAgent[]`
        .route("/shared/with-me", get(with_me))
        // Returns `SharedAgent[]`
        .route("/shared/by-me", get(by_me))
        // `?capability=` narrows it; returns `CapabilityProviders[]`
        .route("/shared/capabilities", get(capabilities))
        // PUT body `{"capability", "cardId"}` (`cardId: null` unpins)
        .route("/shared/pins", put(set_pin))
        // GET returns `TrustRule[]`; PUT body
        // `{"requester", "providerAgent", "policy"}`
        .route("/trust/policies", get(policies).put(set_policy))
}

async fn with_me(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(shared::list_agents_shared_with_me(&state).await)
}

async fn by_me(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(shared::list_agents_shared_by_me(&state).await)
}

#[derive(Deserialize)]
struct CapabilityQuery {
    capability: Option<String>,
}

async fn capabilities(
    State(state): State<AppState>,
    Query(query): Query<CapabilityQuery>,
) -> impl IntoResponse {
    ApiResult(shared::list_capability_providers(&state, query.capability.as_deref()).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinBody {
    capability: String,
    card_id: Option<String>,
}

async fn set_pin(
    State(state): State<AppState>,
    body: Result<Json<PinBody>, JsonRejection>,
) -> impl IntoResponse {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return ApiResult(Err(rejection.body_text())),
    };
    ApiResult(shared::set_provider_pin(&state, &body.capability, body.card_id.as_deref()).await)
}

async fn policies(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(shared::list_trust_policies(&state).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyBody {
    requester: String,
    provider_agent: String,
    policy: TrustPolicy,
}

/// A malformed body is a 400 with the reason, like any service error
async fn set_policy(
    State(state): State<AppState>,
    body: Result<Json<PolicyBody>, JsonRejection>,
) -> impl IntoResponse {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return ApiResult(Err(rejection.body_text())),
    };
    ApiResult(
        shared::set_trust_policy(&state, &body.requester, &body.provider_agent, body.policy).await,
    )
}
