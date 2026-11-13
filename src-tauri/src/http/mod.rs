//! Web transport: commands (`POST /api/rpc/{name}`), the trigger receivers,
//! emergency data, events (`/ws`) and the frontend (`/`), all behind token
//! auth. A thin adapter over `rpc/` and `services/`. No Tauri types here.
//!
//! Binds `127.0.0.1:0` (a random free port) by default; `0.0.0.0` only
//! with an explicit `--server`.

pub mod auth;
mod recovery;
mod rpc;
mod static_files;
mod triggers;
pub mod ws;

use std::net::SocketAddr;

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::get;
use axum::Router;

use crate::services::AppState;
use auth::WebToken;

/// Default port for `--server` mode
pub const DEFAULT_SERVER_PORT: u16 = 17463;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub fn router(state: AppState, token: WebToken) -> Router {
    Router::new()
        .nest(
            "/api",
            rpc::api().merge(triggers::api()).merge(recovery::api()),
        )
        .route("/ws", get(ws::handler))
        .fallback(static_files::handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(token, auth::require_token))
        .with_state(state)
}

/// Binds and starts serving in the background. Records the bound port in
/// `AppState` and returns the address.
pub async fn serve(
    state: AppState,
    bind: SocketAddr,
    token: WebToken,
) -> Result<SocketAddr, String> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("Web server could not bind {bind}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    state.set_http_port(addr.port());

    let app = router(state.clone(), token);
    state.runtime().spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("Web server stopped: {e}");
        }
    });
    log::info!("Web server listening on http://{addr}");
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    const TOKEN: &str = "test-token-0123456789abcdef0123456789";

    fn app() -> (AppState, Router) {
        let state = AppState::for_tests(vec![]);
        (state.clone(), router(state, WebToken::new(TOKEN)))
    }

    fn get(uri: &str) -> Request<Body> {
        Request::get(uri).body(Body::empty()).unwrap()
    }

    fn authed(method: &str, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn is_json(response: &axum::response::Response) -> bool {
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value.as_bytes().starts_with(b"application/json"))
    }

    async fn json(response: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn rejects_requests_without_a_valid_token() {
        let (_, app) = app();
        let rpc = |uri: &str| Request::post(uri).body(Body::from("{}")).unwrap();
        for request in [
            rpc("/api/rpc/get_workflows"),
            rpc("/api/rpc/get_workflows?token=wrong"),
            Request::post("/api/rpc/get_workflows")
                .header(header::AUTHORIZATION, "Bearer wrong")
                .body(Body::from("{}"))
                .unwrap(),
            get("/"),
        ] {
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn exchanges_a_query_token_for_a_cookie_and_strips_it() {
        let (_, app) = app();
        let response = app
            .clone()
            .oneshot(get(&format!("/studio?x=1&token={TOKEN}")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/studio?x=1");
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));

        // The cookie alone now works for the API
        let cookie_value = cookie.split(';').next().unwrap().to_string();
        let response = app
            .oneshot(
                Request::post("/api/rpc/get_workflows")
                    .header(header::COOKIE, cookie_value)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_unrouted_api_path_falls_through_to_the_frontend() {
        let (state, app) = app();
        // Not JSON: the frontend's HTML, so a client never reads it as data
        let missing = app
            .oneshot(authed("GET", "/api/relay/nope", ""))
            .await
            .unwrap();
        assert!(!is_json(&missing));
        state.shutdown();
    }

    #[tokio::test]
    async fn wave_b_routes_are_mounted() {
        let (state, app) = app();
        // Only routes without side effects outside the test (no CLI
        // probes, no browser, no AWS calls)
        for (method, uri, body) in [
            ("POST", "/api/rpc/list_harnesses", "{}"),
            ("POST", "/api/rpc/get_harness_default", "{}"),
            (
                "POST",
                "/api/rpc/set_harness_default",
                r#"{"harnessId":"claude-code","model":null}"#,
            ),
            ("POST", "/api/rpc/clear_harness_default", "{}"),
            (
                "POST",
                "/api/rpc/get_harness_health",
                r#"{"harnessId":"nope"}"#,
            ),
            ("POST", "/api/rpc/test_harness", r#"{"harnessId":"nope"}"#),
            ("POST", "/api/rpc/get_aws_bedrock_api_key_status", "{}"),
            ("POST", "/api/rpc/poll_aws_sso_login", r#"{"loginId":"l1"}"#),
            ("POST", "/api/rpc/get_litellm_gateway_status", "{}"),
        ] {
            // Each answers with JSON, whatever the status
            let response = app
                .clone()
                .oneshot(authed(method, uri, body))
                .await
                .unwrap();
            assert!(is_json(&response), "{method} {uri}: {}", response.status());
        }
        state.shutdown();
    }

    #[tokio::test]
    async fn wave_cd_routes_are_mounted() {
        let (state, app) = app();
        let ask = r#"{"request":{"text":"What can you do?","context":{"currentView":"home"}}}"#;
        let artifact = r#"{"artifact":{"kind":"agent","id":"nope"}}"#;
        let preview = r#"{"artifact":{"kind":"agent","id":"nope"},"instruction":"x"}"#;
        // Side-effect-free routes only (no model calls, no git writes)
        for (method, uri, body) in [
            ("POST", "/api/rpc/ask", ask),
            ("POST", "/api/rpc/list_ask_history", "{}"),
            ("POST", "/api/rpc/load_artifact", artifact),
            ("POST", "/api/rpc/preview_edit", preview),
            ("POST", "/api/rpc/list_push_targets", "{}"),
            (
                "POST",
                "/api/rpc/validate_artifact",
                r#"{"artifact":{"kind":"agent","id":"a"},"content":"---\nmodel: x\n---\n"}"#,
            ),
            ("POST", "/api/rpc/list_pending_approvals", "{}"),
        ] {
            // Each answers with JSON, whatever the status
            let response = app
                .clone()
                .oneshot(authed(method, uri, body))
                .await
                .unwrap();
            assert_ne!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
            assert!(is_json(&response), "{method} {uri}: {}", response.status());
        }
        state.shutdown();
    }

    #[tokio::test]
    async fn rpc_runs_a_command_with_camel_case_arguments() {
        let (state, app) = app();
        let call = |name: &str, body: &str| {
            app.clone()
                .oneshot(authed("POST", &format!("/api/rpc/{name}"), body))
        };

        let found = call("request_for_run", r#"{"runId":"r1"}"#).await.unwrap();
        assert_eq!(found.status(), StatusCode::OK);
        assert_eq!(json(found).await, serde_json::Value::Null);

        let missing_arg = call("request_for_run", "{}").await.unwrap();
        assert_eq!(missing_arg.status(), StatusCode::BAD_REQUEST);
        assert!(json(missing_arg).await["error"]
            .as_str()
            .unwrap()
            .contains("Invalid arguments for request_for_run"));

        let unknown = call("nope", "{}").await.unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        state.shutdown();
    }

    #[tokio::test]
    async fn service_errors_are_400_with_the_message() {
        let (_, app) = app();
        let response = app
            .oneshot(authed(
                "POST",
                "/api/rpc/send_message",
                r#"{"agentId":"../etc","text":"x","attachments":[],"clientMessageId":null}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(json(response).await["error"]
            .as_str()
            .unwrap()
            .contains("Invalid agent id"));
    }

    #[tokio::test]
    async fn websocket_streams_events_after_auth() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let state = AppState::for_tests(vec![]);
        let addr = serve(
            state.clone(),
            "127.0.0.1:0".parse().unwrap(),
            WebToken::new(TOKEN),
        )
        .await
        .unwrap();
        assert_eq!(state.http_port(), Some(addr.port()));

        // No token: refused before the upgrade
        let refused = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await;
        assert!(refused.is_err());

        let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

        let conversation = crate::services::chat::create_studio_conversation(&state)
            .await
            .unwrap();
        crate::services::chat::send_studio_message(
            &state,
            conversation.id,
            "hi".into(),
            vec![],
            None,
        )
        .await
        .unwrap();

        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
        assert_eq!(event["event"], "agent://run");
        assert_eq!(event["payload"]["agentId"], "@studio");
        state.shutdown();
    }

    #[tokio::test]
    async fn webhooks_use_the_trigger_secret_instead_of_the_token() {
        let (state, app) = app();
        let created = app
            .clone()
            .oneshot(authed(
                "POST",
                "/api/rpc/create_trigger",
                r#"{"input": {"name": "Hook", "sourceType": "webhook", "sourceConfig": {},
                    "targetType": "agent", "targetId": "writer"}}"#,
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created = json(created).await;
        let (id, secret) = (
            created["trigger"]["id"].as_str().unwrap().to_string(),
            created["secret"].as_str().unwrap().to_string(),
        );

        let hook = |secret: Option<&str>, body: &str| {
            let mut request = Request::post(format!("/api/triggers/webhook/{id}"))
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(secret) = secret {
                request = request.header("X-Trigger-Secret", secret);
            }
            request.body(Body::from(body.to_string())).unwrap()
        };
        let started = std::time::Instant::now();
        let accepted = app
            .clone()
            .oneshot(hook(Some(&secret), r#"{"order": 1}"#))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));

        for (secret, body, status) in [
            (None, "{}", StatusCode::UNAUTHORIZED),
            (Some("wrong"), "{}", StatusCode::UNAUTHORIZED),
            (Some(secret.as_str()), "not json", StatusCode::BAD_REQUEST),
        ] {
            let response = app.clone().oneshot(hook(secret, body)).await.unwrap();
            assert_eq!(response.status(), status, "{secret:?} {body}");
        }
        // Managing triggers still needs the token
        let unauthed = Request::post("/api/rpc/list_triggers")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let response = app.clone().oneshot(unauthed).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let listed = json(
            app.oneshot(authed("POST", "/api/rpc/list_triggers", "{}"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(listed[0]["hasSecret"], true);
        assert!(!listed.to_string().contains(&secret));
        state.shutdown();
    }
}
