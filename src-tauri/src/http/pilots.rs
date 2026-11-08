//! Pilot routes (`/api/pilots/...`). Bodies mirror the Tauri commands in
//! `commands/pilots.rs`; errors are `400 {"error": "..."}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::pilots;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `PilotDefinition[]`
        .route("/pilots", get(list))
        .route("/pilots/{id}/install", post(install))
        // Returns the pid
        .route("/pilots/{id}/start", post(start))
        .route("/pilots/{id}/stop", post(stop))
        // Returns `PilotStatus`
        .route("/pilots/{id}/status", get(status))
        // Returns `AgentPilotPermission[]`
        .route("/agents/{agent_id}/pilots", get(permissions))
        // Body: `{"capability": "..."}`
        .route("/agents/{agent_id}/pilots/{pilot_id}/grant", post(grant))
        // Body: `{"capability": "..."}`
        .route("/agents/{agent_id}/pilots/{pilot_id}/revoke", post(revoke))
        // Body `{ name, value }` (empty value removes); write-only
        .route(
            "/agents/{agent_id}/pilots/{pilot_id}/secret",
            axum::routing::put(set_secret),
        )
        // Body `{ name, value }`: a new site secret (skill.md + keychain)
        .route(
            "/agents/{agent_id}/pilots/{pilot_id}/secrets",
            post(add_secret),
        )
        // "Sign out everywhere"; returns whether there was a profile
        .route(
            "/agents/{agent_id}/pilots/{pilot_id}/profile",
            axum::routing::delete(clear_profile),
        )
}

async fn list() -> impl IntoResponse {
    Json(pilots::list_pilots())
}

async fn install(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(pilots::install_pilot(&state, &id).await)
}

async fn start(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(pilots::start_pilot(&state, &id).await)
}

async fn stop(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(pilots::stop_pilot(&state, &id).await)
}

async fn status(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(pilots::pilot_status(&state, &id).await)
}

#[derive(serde::Deserialize)]
struct SecretBody {
    name: String,
    #[serde(default)]
    value: String,
}

async fn set_secret(
    State(state): State<AppState>,
    Path((agent_id, pilot_id)): Path<(String, String)>,
    Json(body): Json<SecretBody>,
) -> impl IntoResponse {
    ApiResult(pilots::set_pilot_secret(&state, &agent_id, &pilot_id, &body.name, &body.value).await)
}

async fn add_secret(
    State(state): State<AppState>,
    Path((agent_id, pilot_id)): Path<(String, String)>,
    Json(body): Json<SecretBody>,
) -> impl IntoResponse {
    ApiResult(pilots::add_pilot_secret(&state, &agent_id, &pilot_id, &body.name, &body.value).await)
}

async fn clear_profile(
    State(state): State<AppState>,
    Path((agent_id, pilot_id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(pilots::clear_browser_profile(&state, &agent_id, &pilot_id).await)
}

async fn permissions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(pilots::agent_pilot_permissions(&state, &agent_id).await)
}

#[derive(Deserialize)]
struct GrantBody {
    capability: String,
}

async fn grant(
    State(state): State<AppState>,
    Path((agent_id, pilot_id)): Path<(String, String)>,
    Json(body): Json<GrantBody>,
) -> impl IntoResponse {
    ApiResult(pilots::grant_pilot_capability(&state, &agent_id, &pilot_id, &body.capability).await)
}

async fn revoke(
    State(state): State<AppState>,
    Path((agent_id, pilot_id)): Path<(String, String)>,
    Json(body): Json<GrantBody>,
) -> impl IntoResponse {
    ApiResult(pilots::revoke_pilot_capability(&state, &agent_id, &pilot_id, &body.capability).await)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::services::AppState;

    async fn send(method: &str, uri: &str, body: &str) -> (StatusCode, serde_json::Value) {
        let state = AppState::for_tests(vec![]);
        let app = super::api().with_state(state.clone());
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        state.shutdown();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn lists_this_platforms_pilots() {
        let (status, body) = send("GET", "/pilots", "").await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<&str> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"opendesk"));
    }

    #[tokio::test]
    async fn errors_are_400_with_the_message() {
        let (status, body) = send("GET", "/pilots/nope/status", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("Unknown pilot"));

        let (status, body) = send("GET", "/pilots/opendesk/status", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["installed"], false);

        let (status, body) = send(
            "POST",
            "/agents/ghost/pilots/opendesk/grant",
            r#"{"capability":"browser"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("not found"));

        let (status, body) = send(
            "POST",
            "/agents/writer/pilots/opendesk/revoke",
            r#"{"capability":"nope"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("no capability"));

        let (status, _) = send("GET", "/agents/ghost/pilots", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
