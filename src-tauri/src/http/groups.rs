//! Group routes (`/api/groups/...`), mirroring `commands/groups.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::domain::group::AccessLevel;
use crate::services::groups::{Groups, InviteRequest};
use crate::services::AppState;
use nandi_protocol::GroupRole;

pub fn api() -> Router<AppState> {
    Router::new()
        // GET returns `GroupsOverview`; POST body `{name, description, policy}` returns `GroupView`
        .route("/groups", get(list).post(create))
        // Body `{code}`; returns `GroupView`
        .route("/groups/join", post(join))
        // GET returns `GroupView`; PUT body `{name, description}`
        .route("/groups/{id}", get(one).put(update))
        // Body `{policy}`; signs the next version
        .route("/groups/{id}/policy", put(set_policy))
        // Body `InviteRequest`; returns `GroupInvite`
        .route("/groups/{id}/invites", post(invite))
        .route("/groups/invites/{code}", delete(revoke))
        .route("/groups/{id}/leave", post(leave))
        // PUT body `{role}`; DELETE removes them
        .route(
            "/groups/{id}/members/{instance_id}",
            put(set_role).delete(remove),
        )
        // GET returns `AccessLevel`; PUT body `{level}`
        .route("/groups/{id}/access", get(access).put(set_access))
}

fn groups(state: &AppState) -> Result<Groups, String> {
    Groups::of(state)
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.overview().await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    #[serde(default)]
    description: String,
    policy: String,
}

async fn create(State(state): State<AppState>, Json(body): Json<CreateBody>) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.create(&body.name, &body.description, &body.policy).await,
        Err(e) => Err(e),
    })
}

async fn one(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.get(&id).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct UpdateBody {
    name: String,
    #[serde(default)]
    description: String,
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.update(&id, &body.name, &body.description).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct PolicyBody {
    policy: String,
}

async fn set_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PolicyBody>,
) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.set_policy(&id, &body.policy).await,
        Err(e) => Err(e),
    })
}

async fn invite(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<InviteRequest>,
) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.invite(&id, body).await,
        Err(e) => Err(e),
    })
}

async fn revoke(State(state): State<AppState>, Path(code): Path<String>) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.revoke_invite(&code).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct JoinBody {
    code: String,
}

async fn join(State(state): State<AppState>, Json(body): Json<JoinBody>) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.join(&body.code).await,
        Err(e) => Err(e),
    })
}

async fn leave(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.leave(&id).await,
        Err(e) => Err(e),
    })
}

#[derive(Deserialize)]
struct RoleBody {
    role: GroupRole,
}

async fn set_role(
    State(state): State<AppState>,
    Path((id, instance_id)): Path<(String, String)>,
    Json(body): Json<RoleBody>,
) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.set_role(&id, &instance_id, body.role).await,
        Err(e) => Err(e),
    })
}

async fn remove(
    State(state): State<AppState>,
    Path((id, instance_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(match groups(&state) {
        Ok(g) => g.remove_member(&id, &instance_id).await,
        Err(e) => Err(e),
    })
}

async fn access(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(groups(&state).and_then(|g| g.access(&id)))
}

#[derive(Deserialize)]
struct AccessBody {
    level: AccessLevel,
}

async fn set_access(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AccessBody>,
) -> impl IntoResponse {
    ApiResult(groups(&state).and_then(|g| g.set_access(&id, body.level)))
}
