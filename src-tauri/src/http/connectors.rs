//! Connector routes (`/api/connectors/...`), mirroring the Tauri commands
//! in `commands/connectors.rs`. Tokens and API keys never come back out.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::connectors::{self, ConnectorInfo, ConnectorStatus};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `ConnectorInfo[]`
        .route("/connectors", get(list))
        .route("/connectors/{id}/install", post(install))
        .route("/connectors/{id}/start", post(start))
        .route("/connectors/{id}/stop", post(stop))
        // Returns `ConnectorStatus`
        .route("/connectors/{id}/status", get(status))
        // Body `{ config }`: the connector's settings (never secrets)
        .route("/connectors/{id}/config", put(configure))
        // Body `{ key, value }`; an empty value removes it. Write only
        .route("/connectors/{id}/secret", put(set_secret))
        // Returns `FolderUsers[]`
        .route("/connectors/{id}/folder-users", get(folder_users))
        // Returns `ConnectorEntry[]` (`?refresh=true` asks the registries again)
        .route("/registry-connectors", get(registry_list))
        // Body `{ registryUrl, connectorId }`; returns `ConnectorConsent`
        .route("/registry-connectors/consent", post(registry_consent))
        // Body `{ registryUrl, connectorId, apiKey? }`
        .route("/registry-connectors/install", post(registry_install))
        .route("/connectors/{id}/remove", post(remove))
        // Body `ScaffoldConnectorInput`; returns `ConnectorInfo`
        .route("/connectors/scaffold", post(scaffold))
        // Body `PublishConnectorInput`; returns `PublishConnectorResult`
        .route("/connectors/publish", post(publish))
        // Returns `AgentConnectorChoice[]`; PUT body `{ connectors }`
        .route(
            "/agents/{agent_id}/connectors",
            get(agent_connectors).put(set_agent_connectors),
        )
}

async fn folder_users(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<connectors::FolderUsers>> {
    ApiResult(connectors::parse_id(&id).map(|_| connectors::folder_users(&state, &id)))
}

async fn agent_connectors(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<connectors::AgentConnectorChoice>> {
    ApiResult(connectors::agent_connectors(&state, &id))
}

#[derive(Deserialize)]
struct AgentConnectorsBody {
    #[serde(default)]
    connectors: Vec<connectors::AgentConnectorChoice>,
}

async fn set_agent_connectors(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AgentConnectorsBody>,
) -> ApiResult<()> {
    ApiResult(connectors::set_agent_connectors(&state, &id, body.connectors).await)
}

#[derive(Deserialize)]
struct ConfigBody {
    #[serde(default)]
    config: std::collections::BTreeMap<String, serde_json::Value>,
}

async fn configure(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ConfigBody>,
) -> ApiResult<()> {
    ApiResult(
        async {
            connectors::parse_id(&id)?;
            connectors::configure_connector(&state, &id, connectors::yaml_config(body.config)?)
                .await
        }
        .await,
    )
}

#[derive(Deserialize)]
struct SecretBody {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
}

async fn set_secret(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SecretBody>,
) -> ApiResult<()> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::set_connector_secret(&state, &id, &body.key, &body.value).await,
        Err(e) => Err(e),
    })
}

async fn list(State(state): State<AppState>) -> ApiResult<Vec<ConnectorInfo>> {
    ApiResult(connectors::list_connectors(&state).await)
}

async fn install(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::install_connector(&state, &id).await,
        Err(e) => Err(e),
    })
}

async fn start(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::start_connector(&state, &id).await,
        Err(e) => Err(e),
    })
}

async fn stop(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::stop_connector(&state, &id).await,
        Err(e) => Err(e),
    })
}

async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ConnectorStatus> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::connector_status(&state, &id).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct RefreshQuery {
    #[serde(default)]
    refresh: bool,
}

async fn registry_list(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<RefreshQuery>,
) -> ApiResult<Vec<crate::services::marketplace::connectors::ConnectorEntry>> {
    ApiResult(crate::services::marketplace::connectors::list(&state, query.refresh).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryConnectorBody {
    registry_url: String,
    connector_id: String,
    #[serde(default)]
    api_key: Option<String>,
}

async fn registry_consent(
    State(state): State<AppState>,
    Json(body): Json<RegistryConnectorBody>,
) -> ApiResult<crate::services::marketplace::connectors::ConnectorConsent> {
    ApiResult(
        crate::services::marketplace::connectors::consent(
            &state,
            &body.registry_url,
            &body.connector_id,
        )
        .await,
    )
}

async fn registry_install(
    State(state): State<AppState>,
    Json(body): Json<RegistryConnectorBody>,
) -> ApiResult<()> {
    ApiResult(
        crate::services::marketplace::connectors::install(
            &state,
            &body.registry_url,
            &body.connector_id,
            body.api_key,
        )
        .await,
    )
}

async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    ApiResult(
        async {
            connectors::parse_id(&id)?;
            connectors::remove_connector(&state, &id).await
        }
        .await,
    )
}

async fn scaffold(
    State(state): State<AppState>,
    Json(input): Json<crate::services::studio::connector::ScaffoldConnectorInput>,
) -> ApiResult<ConnectorInfo> {
    ApiResult(crate::services::studio::connector::scaffold(&state, input).await)
}

async fn publish(
    State(state): State<AppState>,
    Json(input): Json<crate::services::marketplace::connectors::PublishConnectorInput>,
) -> ApiResult<crate::services::marketplace::connectors::PublishConnectorResult> {
    ApiResult(crate::services::marketplace::connectors::publish(&state, input).await)
}
