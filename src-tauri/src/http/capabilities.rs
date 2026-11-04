//! Capability definition routes (`/api/capabilities/...`), mirroring
//! `commands/capabilities.rs`. The wave 2A lookups are in `capability.rs`.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::capability::{self, CapabilityDefinition, CapabilityOwner};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // GET `CapabilityListing[]`; POST body `CapabilityDefinition`
        .route("/capabilities", get(list).post(save))
        // PUT body `CapabilityDefinition` (its `id` is the path's)
        .route(
            "/capabilities/{id}",
            get(get_one).put(replace).delete(delete),
        )
        // Body `{ newOwner: CapabilityOwner, reason?: string }`
        .route("/capabilities/{id}/transfer", post(transfer))
        // Returns `CapabilityProvider[]`
        .route("/capabilities/{id}/providers", get(providers))
        // Returns `CapabilityProviderStats[]`
        .route("/capabilities/{id}/breakdown", get(breakdown))
        // `?windowDays=30`; returns `CapabilityEvidence[]`
        .route("/capabilities/{id}/evidence", get(evidence))
        // Returns `AmbiguityCluster[]`
        .route("/capabilities/{id}/ambiguities", get(ambiguities))
        .route(
            "/capabilities/{id}/ambiguities/{cluster}/promote",
            post(promote),
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferBody {
    new_owner: CapabilityOwner,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowQuery {
    window_days: Option<u32>,
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(capability::list_capabilities(&state).await)
}

async fn save(
    State(state): State<AppState>,
    Json(definition): Json<CapabilityDefinition>,
) -> impl IntoResponse {
    ApiResult(capability::save_capability(&state, definition).await)
}

async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(capability::get_capability(&state, &id).await)
}

async fn replace(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut definition): Json<CapabilityDefinition>,
) -> impl IntoResponse {
    definition.id = id;
    ApiResult(capability::save_capability(&state, definition).await)
}

async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(capability::delete_capability(&state, &id).await)
}

async fn transfer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TransferBody>,
) -> impl IntoResponse {
    ApiResult(
        capability::transfer_capability_ownership(&state, &id, body.new_owner, body.reason).await,
    )
}

async fn providers(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(capability::list_providers_for_capability(&state, &id).await)
}

async fn breakdown(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(capability::get_capability_provider_breakdown(&state, &id).await)
}

async fn evidence(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WindowQuery>,
) -> impl IntoResponse {
    ApiResult(capability::get_capability_evidence(&state, &id, query.window_days).await)
}

async fn ambiguities(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(capability::list_capability_ambiguities(&state, &id).await)
}

async fn promote(
    State(state): State<AppState>,
    Path((id, cluster)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(capability::promote_ambiguity_to_schema(&state, &id, &cluster).await)
}
