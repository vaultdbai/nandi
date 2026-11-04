//! Command palette routes, mirroring `commands/palette.rs`. Errors (and
//! malformed bodies) are `400 {"error": "..."}`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use super::routes::ApiResult;
use crate::services::palette;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?q=`; returns `PaletteResult[]`
        .route("/palette/search", get(search))
        // Body: `{"args": {...}}` (optional); returns `PaletteActionOutcome`
        .route("/palette/actions/{action_id}", post(execute))
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    ApiResult(palette::search_palette(&state, &query.q).await)
}

#[derive(Deserialize)]
struct ExecuteBody {
    #[serde(default)]
    args: Value,
}

async fn execute(
    State(state): State<AppState>,
    Path(action_id): Path<String>,
    body: Result<Json<ExecuteBody>, JsonRejection>,
) -> impl IntoResponse {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return ApiResult(Err(rejection.body_text())),
    };
    ApiResult(palette::execute_palette_action(&state, &action_id, body.args).await)
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
    async fn searches_actions() {
        let (status, body) = send("GET", "/palette/search?q=%3Ehome", "").await;
        assert_eq!(status, StatusCode::OK);
        let results = body.as_array().unwrap();
        assert_eq!(results[0]["name"], "Go to Home");
        assert!(results.iter().all(|r| r["kind"] == "action"));
    }

    #[tokio::test]
    async fn executes_ui_actions() {
        let (status, body) = send(
            "POST",
            "/palette/actions/open_view",
            r#"{"args":{"view":{"kind":"network","tab":"people"}}}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["navigate"]["kind"], "network");
    }

    #[tokio::test]
    async fn errors_and_bad_bodies_are_400() {
        let (status, body) = send("POST", "/palette/actions/nope", "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("Unknown"));

        let (status, body) = send("POST", "/palette/actions/open_view", "not json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].is_string());

        let (status, body) = send("POST", "/palette/actions/open_view", r#"{"args":"x"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("Invalid"));
    }
}
