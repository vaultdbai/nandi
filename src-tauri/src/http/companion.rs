//! Companion routes, mirroring `commands/companion.rs`. Window control only
//! exists in the desktop app, so `/companion/toggle` stays 501 in a browser.
//! Errors are `400 {"error": "..."}`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::not_implemented;
use super::routes::ApiResult;
use crate::services::companion;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?dayStart=`; returns `CompanionSnapshot`
        .route("/companion/snapshot", get(snapshot))
        // Desktop only: there is no window to toggle in a browser
        .route("/companion/toggle", post(not_implemented))
        // Body: `{"pinned": true}`. Saves the preference; the desktop
        // window applies it next time it's shown.
        .route("/companion/pin", post(pin))
        // Body: `{"text", "conversationId"?, "scopeRequestId"?}`; returns `SendMessageResult`
        .route("/companion/messages", post(message))
        // Body: `{"notify": true}`: "Notify me"
        .route("/companion/notify", post(notify))
        // Body: `{"notify": true}`: "Notify me when requests come back"
        .route("/companion/notify-results", post(notify_results))
}

async fn notify_results(
    State(state): State<AppState>,
    Json(body): Json<NotifyBody>,
) -> impl IntoResponse {
    ApiResult(companion::set_notify_results(&state, body.notify).await)
}

#[derive(Deserialize)]
struct NotifyBody {
    #[serde(default)]
    notify: bool,
}

async fn notify(State(state): State<AppState>, Json(body): Json<NotifyBody>) -> impl IntoResponse {
    ApiResult(companion::set_notify(&state, body.notify).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotQuery {
    #[serde(default)]
    day_start: f64,
}

async fn snapshot(
    State(state): State<AppState>,
    Query(query): Query<SnapshotQuery>,
) -> impl IntoResponse {
    ApiResult(companion::get_companion_snapshot(&state, query.day_start).await)
}

#[derive(Deserialize)]
struct PinBody {
    #[serde(default)]
    pinned: bool,
}

async fn pin(State(state): State<AppState>, Json(body): Json<PinBody>) -> impl IntoResponse {
    ApiResult(companion::set_pinned(&state, body.pinned).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageBody {
    #[serde(default)]
    text: String,
    conversation_id: Option<String>,
    scope_request_id: Option<String>,
}

async fn message(
    State(state): State<AppState>,
    Json(body): Json<MessageBody>,
) -> impl IntoResponse {
    ApiResult(
        companion::send_companion_message(
            &state,
            &body.text,
            body.conversation_id.as_deref(),
            body.scope_request_id.as_deref(),
        )
        .await,
    )
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn snapshot_pin_and_messages_work_over_http() {
        let state = AppState::for_tests(vec![]);
        let app = api().with_state(state.clone());

        let (status, body) = call(&app, "POST", "/companion/pin", r#"{"pinned":true}"#).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, body) = call(&app, "GET", "/companion/snapshot?dayStart=0", "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["pinned"], true);
        assert_eq!(body["health"], "healthy");

        let (status, _) = call(&app, "POST", "/companion/messages", r#"{"text":" "}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = call(
            &app,
            "POST",
            "/companion/messages",
            r#"{"text":"What's happening?"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["runId"].is_string());

        let (status, body) = call(&app, "POST", "/companion/notify", r#"{"notify":false}"#).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, body) = call(&app, "GET", "/companion/snapshot?dayStart=0", "").await;
        assert_eq!(body["notify"], false);

        let (status, _) = call(&app, "POST", "/companion/toggle", "{}").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        state.shutdown();
    }
}
