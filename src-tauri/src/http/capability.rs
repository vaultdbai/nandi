//! Capability routes, mirroring `commands/capability.rs`. Errors are
//! `400 {"error": "..."}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use super::routes::ApiResult;
use crate::services::capability_index;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `string[]`: every capability a local agent serves.
        // (`/capabilities` itself lists definitions: `http/capabilities.rs`)
        .route("/capabilities/declared", get(list))
        // Returns `AgentCard[]`. `{id}` like the wave A routes beside it.
        .route("/capabilities/{id}/agents", get(find))
        // Returns `AgentCard`
        .route("/agents/{agent_id}/card", get(card))
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(capability_index::list_declared_capabilities(&state).await)
}

async fn find(State(state): State<AppState>, Path(capability): Path<String>) -> impl IntoResponse {
    ApiResult(capability_index::find_agents_by_capability(&state, &capability).await)
}

async fn card(State(state): State<AppState>, Path(agent_id): Path<String>) -> impl IntoResponse {
    ApiResult(capability_index::get_agent_card(&state, &agent_id).await)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::services::domain::{Agent, AgentId, EntityKind};
    use crate::services::repos::Repository;
    use crate::services::AppState;

    async fn get(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
        let app = super::api().with_state(state.clone());
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn serves_capabilities_and_cards() {
        let state = AppState::for_tests(vec![]);
        let agent = Agent::from_skill_md(
            AgentId::parse("papers").unwrap(),
            "---\nname: Papers\nserves: [{name: find-papers}]\n---\n",
        )
        .unwrap();
        state.store().repos().agents.save(&agent).unwrap();
        state.store().commit(EntityKind::Agent, "papers").unwrap();

        let (status, body) = get(&state, "/capabilities/declared").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!(["find-papers"]));

        let (status, body) = get(&state, "/capabilities/find-papers/agents").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body[0]["name"], "Papers");
        assert_eq!(body[0]["capabilities"][0]["name"], "find-papers");

        let (status, body) = get(&state, "/agents/papers/card").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["id"].as_str().unwrap().ends_with("/papers"));

        let (status, body) = get(&state, "/agents/ghost/card").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("not found"));
        state.shutdown();
    }
}
