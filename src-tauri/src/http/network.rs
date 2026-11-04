//! Network directory routes (`/api/network/...`), mirroring
//! `commands/network.rs`. A member's profile and trust are also under
//! `/api/members/...` (`teams_roster.rs`).

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};

use super::routes::ApiResult;
use crate::services::network::profiles;
use crate::services::team::trust::{self, TrustUpdate};
use crate::services::{network, AppState};

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `NetworkSnapshot`
        .route("/network/snapshot", get(snapshot))
        .route("/network/refresh", post(refresh))
        // Returns `MemberProfile`
        .route("/network/members/{instance_id}", get(member))
        // Body `TrustUpdate`; returns `TrustRelationship`
        .route("/network/members/{instance_id}/trust", put(set_trust))
}

async fn snapshot(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(network::get_network_snapshot(&state).await)
}

async fn refresh(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(network::refresh_network(&state).await)
}

async fn member(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(profiles::get_member_profile(&state, &instance_id).await)
}

async fn set_trust(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(update): Json<TrustUpdate>,
) -> impl IntoResponse {
    ApiResult(trust::set_trust_relationship(&state, &instance_id, update).await)
}
