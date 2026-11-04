//! Marketplace endpoints (`/api/marketplace/*`), mirroring the Tauri
//! commands in `commands/marketplace.rs`. Registry tokens go in (to the
//! keyring) and never come back out.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::marketplace::{self, AddRegistryInput, MarketplaceFilter, PublishAgentInput};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/marketplace/packs", get(list_packs))
        .route("/marketplace/packs/{id}/detail", get(pack_detail))
        .route("/marketplace/agents", get(list_agents))
        .route("/marketplace/agents/{id}/detail", get(agent_detail))
        .route("/marketplace/install/pack/{id}", post(install_pack))
        .route("/marketplace/install/agent/{id}", post(install_agent))
        // Body `{ path }` (a folder or `.zip` on the host): `PackManifest`
        .route("/marketplace/local/inspect", post(inspect_local_pack))
        // Body `{ path }`: `InstalledPackInfo`
        .route("/marketplace/local/install", post(install_local_pack))
        .route("/marketplace/installed", get(list_installed))
        .route(
            "/marketplace/consent/{agent_id}",
            get(agent_consent).post(grant_consent),
        )
        .route("/marketplace/installed/{kind}/{id}", delete(uninstall))
        .route("/marketplace/fork/{installed_id}", post(fork))
        .route("/marketplace/updates", get(check_for_updates))
        .route(
            "/marketplace/updates/{installed_id}/apply",
            post(apply_update),
        )
        .route(
            "/marketplace/registries",
            get(list_registries).post(add_registry),
        )
        .route("/marketplace/registries/{url}", delete(remove_registry))
        .route(
            "/marketplace/community/create",
            post(create_community_registry),
        )
        .route("/marketplace/publish", post(publish_agent))
}

/// `?query=…&tags=a,b&verifiedOnly=true&installedOnly=true&refresh=true`
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct FilterQuery {
    query: Option<String>,
    tags: Option<String>,
    verified_only: bool,
    installed_only: bool,
    refresh: bool,
}

impl From<FilterQuery> for MarketplaceFilter {
    fn from(q: FilterQuery) -> Self {
        Self {
            query: q.query,
            tags: q
                .tags
                .map(|tags| {
                    tags.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            verified_only: q.verified_only,
            installed_only: q.installed_only,
            refresh: q.refresh,
        }
    }
}

async fn list_packs(
    State(state): State<AppState>,
    Query(filter): Query<FilterQuery>,
) -> impl IntoResponse {
    ApiResult(marketplace::list_packs(&state, &filter.into()).await)
}

async fn list_agents(
    State(state): State<AppState>,
    Query(filter): Query<FilterQuery>,
) -> impl IntoResponse {
    ApiResult(marketplace::list_available_agents(&state, &filter.into()).await)
}

async fn agent_detail(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(marketplace::get_agent_detail(&state, &id).await)
}

async fn pack_detail(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(marketplace::get_pack_detail(&state, &id).await)
}

async fn install_pack(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(marketplace::install_pack(&state, &id).await)
}

#[derive(Deserialize)]
struct LocalPackBody {
    path: String,
}

async fn inspect_local_pack(
    State(state): State<AppState>,
    Json(body): Json<LocalPackBody>,
) -> impl IntoResponse {
    ApiResult(marketplace::inspect_local_pack(&state, &body.path).await)
}

async fn install_local_pack(
    State(state): State<AppState>,
    Json(body): Json<LocalPackBody>,
) -> impl IntoResponse {
    ApiResult(marketplace::install_local_pack(&state, &body.path).await)
}

async fn install_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(marketplace::install_agent(&state, &id).await)
}

async fn list_installed(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(marketplace::list_installed(&state).await)
}

async fn uninstall(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(marketplace::uninstall(&state, &kind, &id).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForkBody {
    new_id: String,
    new_name: String,
}

async fn fork(
    State(state): State<AppState>,
    Path(installed_id): Path<String>,
    Json(body): Json<ForkBody>,
) -> impl IntoResponse {
    ApiResult(marketplace::fork_agent(&state, &installed_id, &body.new_id, &body.new_name).await)
}

async fn agent_consent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(marketplace::get_agent_consent(&state, &agent_id).await)
}

#[derive(Deserialize)]
struct ConsentBody {
    categories: Vec<String>,
}

async fn grant_consent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<ConsentBody>,
) -> impl IntoResponse {
    ApiResult(marketplace::grant_agent_consent(&state, &agent_id, body.categories).await)
}

async fn check_for_updates(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(marketplace::check_for_updates(&state).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyBody {
    new_ref: String,
}

async fn apply_update(
    State(state): State<AppState>,
    Path(installed_id): Path<String>,
    Json(body): Json<ApplyBody>,
) -> impl IntoResponse {
    ApiResult(marketplace::apply_update(&state, &installed_id, &body.new_ref).await)
}

async fn list_registries(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(marketplace::list_registries(&state).await)
}

async fn add_registry(
    State(state): State<AppState>,
    Json(input): Json<AddRegistryInput>,
) -> impl IntoResponse {
    ApiResult(marketplace::add_registry(&state, input).await)
}

async fn publish_agent(
    State(state): State<AppState>,
    Json(input): Json<PublishAgentInput>,
) -> impl IntoResponse {
    ApiResult(marketplace::publish_agent(&state, input).await)
}

async fn create_community_registry(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(marketplace::create_community_registry(&state).await)
}

/// `DELETE /api/marketplace/registries/{url}` (the URL percent-encoded)
async fn remove_registry(
    State(state): State<AppState>,
    Path(url): Path<String>,
) -> impl IntoResponse {
    ApiResult(marketplace::remove_registry(&state, &url).await)
}
