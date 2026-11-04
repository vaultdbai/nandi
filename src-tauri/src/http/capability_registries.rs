//! Capability registry routes (`/api/capability-registries`), mirroring
//! `commands/capability_registries.rs`. A registry is named by its URL,
//! which doesn't fit a path segment, so DELETE takes it in the query
//! (`?url=...`) and sync in the body.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::capability::registry::{self, RegistryKind};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // GET `CapabilityRegistry[]`; POST body `AddRegistryBody`;
        // DELETE `?url=<registry url>`
        .route("/capability-registries", get(list).post(add).delete(remove))
        // Body `{ url }`; returns `SyncResult`
        .route("/capability-registries/sync", post(sync))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddRegistryBody {
    url: String,
    kind: Option<RegistryKind>,
    branch_prefix: Option<String>,
    user_slug: Option<String>,
}

#[derive(Deserialize)]
struct UrlParam {
    url: String,
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(registry::list_capability_registries(&state).await)
}

async fn add(
    State(state): State<AppState>,
    Json(body): Json<AddRegistryBody>,
) -> impl IntoResponse {
    ApiResult(
        registry::add_capability_registry(
            &state,
            &body.url,
            body.kind,
            body.branch_prefix,
            body.user_slug,
        )
        .await,
    )
}

async fn remove(State(state): State<AppState>, Query(param): Query<UrlParam>) -> impl IntoResponse {
    ApiResult(registry::remove_capability_registry(&state, &param.url).await)
}

async fn sync(State(state): State<AppState>, Json(param): Json<UrlParam>) -> impl IntoResponse {
    ApiResult(registry::sync_capability_registry(&state, &param.url).await)
}
