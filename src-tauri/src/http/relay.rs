//! Relay routes, mirroring `commands/relay.rs`. Errors are
//! `400 {"error": "..."}`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::relay;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `RelayStatus`
        .route("/relay/status", get(status))
        // Body: `{"url": "wss://..."}` (empty: no relay)
        .route("/relay/url", put(set_url))
        // Body: `{"url": "wss://...", "joinCode": "..."}`; returns `JoinOutcome`
        .route("/relay/join", post(join))
        // Returns `RelayInvite | null`
        .route("/relay/invite", get(invite))
        // Returns `PublicRelayOffer`
        .route("/relay/public", get(public_offer))
        // Returns `PublicRelayJoin`
        .route("/relay/public/join", post(public_join))
        // Returns `PublicRelayAccount`
        .route("/relay/public/account", get(public_account))
        // `?capability=` (optional); returns `AgentCard[]`
        .route("/relay/agents", get(agents))
        // Body: `{"url": "ws://..."}`; returns the round trip in ms
        .route("/relay/test", post(test_connection))
        // Returns `RelayPeer[]` from the relay's `/instances`
        .route("/relay/instances", get(instances))
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(relay::get_relay_status(&state).await)
}

#[derive(Deserialize)]
struct UrlBody {
    #[serde(default)]
    url: String,
}

async fn set_url(State(state): State<AppState>, Json(body): Json<UrlBody>) -> impl IntoResponse {
    ApiResult(relay::set_relay_url(&state, &body.url).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinBody {
    #[serde(default)]
    url: String,
    #[serde(default)]
    join_code: Option<String>,
}

async fn join(State(state): State<AppState>, Json(body): Json<JoinBody>) -> impl IntoResponse {
    ApiResult(relay::join::join_relay(&state, &body.url, body.join_code.as_deref()).await)
}

async fn invite(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(relay::join::relay_invite(&state))
}

async fn public_offer(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(relay::public::offer(&state).await))
}

async fn public_join(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(relay::public::join(&state).await)
}

async fn public_account(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(relay::public::account(&state).await)
}

#[derive(Deserialize)]
struct AgentsQuery {
    capability: Option<String>,
}

async fn agents(
    State(state): State<AppState>,
    Query(query): Query<AgentsQuery>,
) -> impl IntoResponse {
    ApiResult(relay::list_online_agents(&state, query.capability.as_deref()).await)
}

async fn test_connection(Json(body): Json<UrlBody>) -> impl IntoResponse {
    ApiResult(relay::test_relay_connection(&body.url).await)
}

async fn instances(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(relay::get_relay_instances(&state).await)
}
