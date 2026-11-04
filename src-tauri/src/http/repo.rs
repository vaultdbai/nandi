//! Agent editor and git routes (`/api/repo/...`). Bodies mirror the Tauri
//! commands in `commands/repo.rs`.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::repo_service::{self, AgentDefinitionInput};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Body: `AgentDefinitionInput`; returns `AgentInfo`
        .route("/repo/agents/{id}/save", post(save_agent))
        // Body: `{"instructions": "..."}`; an installed agent's go to its override
        .route(
            "/repo/agents/{id}/instructions",
            post(save_instructions).delete(reset_instructions),
        )
        // Returns the entity as stored in its file
        .route("/repo/entities/{kind}/{id}", get(load_entity))
        // Returns `GitStatus`
        .route("/repo/git/status", get(git_status))
        // `?path=`; returns the unified diff as a string
        .route("/repo/git/diff", get(git_diff))
        // Body: `{"message": "..."}`; returns the commit hash
        .route("/repo/git/commit", post(git_commit))
}

async fn save_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<AgentDefinitionInput>,
) -> impl IntoResponse {
    ApiResult(repo_service::save_agent(&state, &id, input).await)
}

#[derive(Deserialize)]
struct InstructionsBody {
    instructions: String,
}

async fn save_instructions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<InstructionsBody>,
) -> impl IntoResponse {
    ApiResult(repo_service::save_agent_instructions(&state, &id, body.instructions).await)
}

async fn reset_instructions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(repo_service::reset_agent_instructions(&state, &id).await)
}

async fn load_entity(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> impl IntoResponse {
    ApiResult(repo_service::load_entity(&state, &kind, &id).await)
}

async fn git_status(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(repo_service::git_status(&state).await)
}

#[derive(Deserialize)]
struct DiffQuery {
    path: String,
}

async fn git_diff(
    State(state): State<AppState>,
    Query(query): Query<DiffQuery>,
) -> impl IntoResponse {
    ApiResult(repo_service::git_diff(&state, &query.path).await)
}

#[derive(Deserialize)]
struct CommitBody {
    message: String,
}

async fn git_commit(
    State(state): State<AppState>,
    Json(body): Json<CommitBody>,
) -> impl IntoResponse {
    ApiResult(repo_service::git_commit(&state, &body.message).await)
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::http::auth::WebToken;
    use crate::services::AppState;

    const TOKEN: &str = "test-token";

    fn request(method: &str, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn json(response: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn save_load_status_diff_and_commit_over_http() {
        let state = AppState::for_tests(vec![]);
        git2::Repository::init(&state.store().layout().root).unwrap();
        let app = crate::http::router(state.clone(), WebToken::new(TOKEN));

        let response = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/repo/agents/writer/save",
                r#"{"name":"Writer","skillBody":"Write.","capabilities":{"tools":["shell"]}}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await["name"], "Writer");

        let response = app
            .clone()
            .oneshot(request("GET", "/api/repo/entities/agent/writer", ""))
            .await
            .unwrap();
        let entity = json(response).await;
        assert_eq!(entity["skillBody"], "Write.");
        assert_eq!(entity["capabilities"]["tools"][0], "shell");

        let response = app
            .clone()
            .oneshot(request("GET", "/api/repo/git/status", ""))
            .await
            .unwrap();
        let status = json(response).await;
        assert!(status["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == ".nandi/agents/writer/skill.md"));

        let response = app
            .clone()
            .oneshot(request(
                "GET",
                "/api/repo/git/diff?path=.nandi/agents/writer/skill.md",
                "",
            ))
            .await
            .unwrap();
        let diff = json(response).await;
        assert!(diff.as_str().unwrap().contains("+name: Writer"), "{diff}");

        let response = app
            .clone()
            .oneshot(request(
                "POST",
                "/api/repo/git/commit",
                r#"{"message":"Add writer"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await.as_str().unwrap().len(), 40);

        // Service errors are 400 with the message
        let response = app
            .oneshot(request("GET", "/api/repo/git/diff?path=../outside", ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        state.shutdown();
    }
}
