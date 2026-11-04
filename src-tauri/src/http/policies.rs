//! Policy routes (`/api/policies`, `/api/agents/{agent_id}/policies`). Bodies mirror the
//! Tauri commands in `commands/policies.rs`; errors are `400 {"error": "..."}`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use super::routes::ApiResult;
use crate::services::policies;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `PolicySummary[]`
        .route("/policies", get(list))
        // Returns `AgentPolicies`
        .route("/agents/{agent_id}/policies", get(agent_policies))
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(policies::list(state.store()))
}

async fn agent_policies(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    ApiResult(policies::agent_policies(state.store(), &agent_id))
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::services::AppState;

    const TOKEN: &str = "test-token-0123456789abcdef0123456789";

    /// `GET uri` through the whole API, signed in
    async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
        let app = super::super::router(state.clone(), super::super::auth::WebToken::new(TOKEN));
        let request = Request::get(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_repos_policies_are_listed_by_id_broken_ones_marked() {
        let state = AppState::for_tests(vec![]);
        let (status, body) = get(&state, "/api/policies").await;
        assert_eq!((status, body), (StatusCode::OK, serde_json::json!([])));

        let layout = state.store().layout().clone();
        let named = layout.policy_path("no-shell");
        std::fs::create_dir_all(named.parent().unwrap()).unwrap();
        std::fs::write(
            &named,
            "---
name: No shell
applies_to: ['*']
tools: { deny: [shell] }
---
",
        )
        .unwrap();
        std::fs::write(
            layout.policy_path("broken"),
            "---
tools: { nope: 1 }
---
",
        )
        .unwrap();

        let (status, body) = get(&state, "/api/policies").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!([
                { "id": "broken", "name": "broken", "valid": false },
                { "id": "no-shell", "name": "No shell", "valid": true },
            ])
        );
        state.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_agents_policies_say_what_they_change_or_why_it_cannot_run() {
        let state = AppState::for_tests(vec![]);
        let layout = state.store().layout().clone();
        let skill = layout.skill_path("writer");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, "---\nname: Writer\n---\nWrite.\n").unwrap();
        let policy = layout.policy_path("no-shell");
        std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
        std::fs::write(
            &policy,
            "---\napplies_to: ['*']\ntools: { deny: [shell] }\n---\n",
        )
        .unwrap();
        state.store().load().unwrap();

        let (status, body) = get(&state, "/api/agents/writer/policies").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["policies"], serde_json::json!(["no-shell"]));
        assert!(body["blocked"].is_null());
        let shell = body["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|change| change["item"] == "tool shell")
            .expect("the denied tool");
        assert_eq!(
            (shell["outcome"].as_str(), shell["by"].as_str()),
            (Some("removed"), Some("no-shell"))
        );

        // A broken policy blocks the agent; the page says why
        std::fs::write(&policy, "---\napplies_to: ['*']\ntools: { nope: 1 }\n---\n").unwrap();
        let (status, body) = get(&state, "/api/agents/writer/policies").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["blocked"].as_str().unwrap().contains("'no-shell'"));
        assert_eq!(body["changes"], serde_json::json!([]));

        let (status, body) = get(&state, "/api/agents/ghost/policies").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("not found"));
        state.shutdown();
    }
}
