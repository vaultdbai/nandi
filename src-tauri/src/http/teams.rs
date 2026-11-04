//! Team routes, mirroring `commands/teams.rs`. Errors are
//! `400 {"error": "..."}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::team;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // GET returns `Team[]`; POST body `{"githubOrg": "..."}` returns `Team`
        .route("/teams", get(list).post(create))
        // Returns `TeamDetail`
        .route("/teams/{team_id}", get(detail))
        .route("/teams/{team_id}/join", post(join))
        .route("/teams/{team_id}/leave", post(leave))
        .route("/teams/{team_id}/sync", post(sync))
        // POST shares the agent; DELETE unshares it
        .route(
            "/teams/{team_id}/agents/{agent_id}",
            post(share).delete(unshare),
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateBody {
    /// The registry's git URL
    // Missing is a readable 400 from the service, not a bare 422
    #[serde(default)]
    github_org: String,
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(team::list_teams(&state).await)
}

async fn create(State(state): State<AppState>, Json(body): Json<CreateBody>) -> impl IntoResponse {
    ApiResult(team::create_team(&state, &body.github_org).await)
}

async fn detail(State(state): State<AppState>, Path(team_id): Path<String>) -> impl IntoResponse {
    ApiResult(team::get_team(&state, &team_id).await)
}

async fn join(State(state): State<AppState>, Path(team_id): Path<String>) -> impl IntoResponse {
    ApiResult(team::join_team(&state, &team_id).await)
}

async fn leave(State(state): State<AppState>, Path(team_id): Path<String>) -> impl IntoResponse {
    ApiResult(team::leave_team(&state, &team_id).await)
}

async fn sync(State(state): State<AppState>, Path(team_id): Path<String>) -> impl IntoResponse {
    ApiResult(team::sync_team_registry(&state, &team_id).await)
}

async fn share(
    State(state): State<AppState>,
    Path((team_id, agent_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(team::share_agent(&state, &team_id, &agent_id).await)
}

async fn unshare(
    State(state): State<AppState>,
    Path((team_id, agent_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(team::unshare_agent(&state, &team_id, &agent_id).await)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn routes_answer_json() {
        let state = AppState::for_tests(vec![]);
        let app = api().with_state(state.clone());

        let response = app
            .clone()
            .oneshot(Request::get("/teams").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"[]");
        state.shutdown();
    }
}
