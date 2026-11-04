//! Roster, member, and trust routes, mirroring `commands/teams_roster.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::network::peer_keys;
use crate::services::team::roster;
use crate::services::team::trust::{self, TrustUpdate};
use crate::services::team::trust_requests::{self, TrustAnswer};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `TeamRoster`
        .route("/teams/{id}/roster", get(team_roster))
        // Returns `MemberProfile`
        .route("/teams/{id}/members/{handle}/profile", get(profile))
        .route("/members/by-instance/{instance_id}", get(by_instance))
        // Body `TrustUpdate`; returns `TrustRelationship`
        .route("/members/{instance_id}/trust", put(set_trust))
        .route(
            "/members/{instance_id}/agents/{agent_id}/block",
            post(block).delete(unblock),
        )
        // Body `{"paused": bool}` (default true)
        .route("/members/{instance_id}/delegations/pause", post(pause))
        // Their key changed: accept the new one; returns `PeerKeyStatus`
        .route("/members/{instance_id}/key/accept", post(accept_key))
        // Returns `TrustRequest[]`
        .route("/trust/requests", get(trust_requests_list))
        // GET returns `RelayTrust`; PUT body `{"enabled": bool}`
        .route("/trust/relay", get(relay_trust).put(set_relay_trust))
        // Body `{"answer": "trust" | "deny", "block": bool}`
        .route("/trust/requests/{instance_id}", post(respond))
}

#[derive(Deserialize)]
struct RespondBody {
    #[serde(default)]
    answer: Option<TrustAnswer>,
    #[serde(default)]
    block: bool,
}

async fn relay_trust(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(trust::relay_trust(state.db())))
}

#[derive(serde::Deserialize)]
struct RelayTrustBody {
    enabled: bool,
}

async fn set_relay_trust(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<RelayTrustBody>,
) -> impl IntoResponse {
    ApiResult(
        trust::set_trusts_relay_members(state.db(), body.enabled)
            .map(|()| trust::relay_trust(state.db())),
    )
}

async fn trust_requests_list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(trust_requests::list_trust_requests(&state).await)
}

async fn respond(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(body): Json<RespondBody>,
) -> impl IntoResponse {
    ApiResult(match body.answer {
        Some(answer) => {
            trust_requests::respond_trust_request(&state, &instance_id, answer, body.block).await
        }
        None => Err("Body needs an \"answer\"".to_string()),
    })
}

#[derive(Deserialize)]
struct PauseBody {
    #[serde(default = "yes")]
    paused: bool,
}

fn yes() -> bool {
    true
}

async fn team_roster(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(roster::get_team_roster(&state, &id).await)
}

async fn profile(
    State(state): State<AppState>,
    Path((id, handle)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(roster::get_member_profile(&state, &id, &handle).await)
}

async fn by_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(roster::get_member_by_instance(&state, &instance_id).await)
}

async fn set_trust(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(update): Json<TrustUpdate>,
) -> impl IntoResponse {
    ApiResult(trust::set_trust_relationship(&state, &instance_id, update).await)
}

async fn block(
    State(state): State<AppState>,
    Path((instance_id, agent_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(trust::block_agent_from_member(&state, &instance_id, &agent_id).await)
}

async fn unblock(
    State(state): State<AppState>,
    Path((instance_id, agent_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(trust::unblock_agent_from_member(&state, &instance_id, &agent_id).await)
}

async fn pause(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    body: Option<Json<PauseBody>>,
) -> impl IntoResponse {
    let paused = body.is_none_or(|Json(body)| body.paused);
    ApiResult(trust::pause_delegations_from_member(&state, &instance_id, paused).await)
}

async fn accept_key(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(peer_keys::accept(state.db(), &instance_id))
}
