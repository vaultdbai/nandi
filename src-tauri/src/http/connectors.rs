//! Connector routes (`/api/connectors/...`), mirroring the Tauri commands
//! in `commands/connectors.rs`. Tokens and API keys never come back out.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::connectors::{self, ConnectorInfo, ConnectorStatus, DeviceFlowState};
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
        // Returns `DeviceFlowState`
        .route("/connectors/{id}/oauth/start", post(oauth_start))
        // Returns `true` once authorized
        .route("/connectors/{id}/oauth/poll", post(oauth_poll))
        // Body `{ config }`: the connector's settings (never secrets)
        .route("/connectors/{id}/config", put(configure))
        // Body `{ key, value }`; an empty value removes it. Write only
        .route("/connectors/{id}/secret", put(set_secret))
        // Returns `FolderUsers[]`
        .route("/connectors/{id}/folder-users", get(folder_users))
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

async fn oauth_start(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<DeviceFlowState> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::start_oauth(&state, &id).await,
        Err(e) => Err(e),
    })
}

async fn oauth_poll(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<bool> {
    ApiResult(match connectors::parse_id(&id) {
        Ok(_) => connectors::poll_oauth(&state, &id).await,
        Err(e) => Err(e),
    })
}
