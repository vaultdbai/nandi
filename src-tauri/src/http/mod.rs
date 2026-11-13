//! Web transport: REST (`/api/*`), events (`/ws`), and the frontend (`/`),
//! all behind token auth. A thin adapter over `services/`, like the Tauri
//! commands. No Tauri types here.
//!
//! Binds `127.0.0.1:0` (a random free port) by default; `0.0.0.0` only
//! with an explicit `--server`.

// Wave CD (see docs/developer/interaction-layer.md)
pub mod auth;
// Wave B
mod aws_auth;
mod capabilities;
// Wave A
mod companion;
// Wave EF
mod connectors;
// Wave final
mod email;
mod gateway;
mod marketplace;
mod pilots;
mod relay;
mod repo;
mod routes;
mod rpc;
mod static_files;
mod triggers;
pub mod ws;

use std::net::SocketAddr;

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use crate::services::AppState;
use auth::WebToken;

/// Default port for `--server` mode
pub const DEFAULT_SERVER_PORT: u16 = 17463;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub fn router(state: AppState, token: WebToken) -> Router {
    Router::new()
        .nest(
            "/api",
            routes::api()
                .merge(rpc::api())
                .merge(marketplace::api())
                .merge(triggers::api())
                .merge(repo::api())
                .merge(pilots::api())
                .merge(connectors::api())
                // Wave 2A
                .merge(relay::api())
                // Wave 2B redesign
                .merge(companion::api())
                // Wave final
                // Wave final, network-first
                .merge(gateway::api())
                .merge(email::api())
                // Wave A
                .merge(capabilities::api())
                // Wave B
                .merge(aws_auth::api()), // Wave CD (see docs/developer/interaction-layer.md),
        )
        .route("/ws", get(ws::handler))
        .fallback(static_files::handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(token, auth::require_token))
        .with_state(state)
}

/// `501 {"error": "not implemented"}`: a frozen route its track hasn't
/// implemented yet.
async fn not_implemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({ "error": crate::services::redesign::NOT_IMPLEMENTED })),
    )
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
        for request in [
            get("/api/workflows"),
            get("/api/workflows?token=wrong"),
            Request::get("/api/workflows")
                .header(header::AUTHORIZATION, "Bearer wrong")
                .body(Body::empty())
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
                Request::get("/api/workflows")
                    .header(header::COOKIE, cookie_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn routes_map_onto_services() {
        let (state, app) = app();

        let created = app
            .clone()
            .oneshot(authed("POST", "/api/studio/conversations", ""))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let id = json(created).await["id"].as_str().unwrap().to_string();

        let sent = app
            .clone()
            .oneshot(authed(
                "POST",
                &format!("/api/studio/conversations/{id}/messages"),
                r#"{"text":"hello"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(sent.status(), StatusCode::OK);
        assert!(json(sent).await["runId"].is_string());

        let list = app
            .clone()
            .oneshot(authed("GET", "/api/studio/conversations", ""))
            .await
            .unwrap();
        assert_eq!(json(list).await[0]["title"], "hello");

        let dashboard = app
            .clone()
            .oneshot(authed("GET", "/api/snapshots/dashboard?dayStart=0", ""))
            .await
            .unwrap();
        assert_eq!(dashboard.status(), StatusCode::OK);
        assert!(json(dashboard).await["stats"]["runsToday"].is_number());
        state.shutdown();
    }

    #[tokio::test]
    async fn wave_2a_routes_are_mounted() {
        let (state, app) = app();
        for (method, uri) in [
            ("GET", "/api/capabilities"),
            ("GET", "/api/relay/status"),
            ("PUT", "/api/relay/url"),
            ("POST", "/api/relay/join"),
            ("GET", "/api/relay/invite"),
            ("GET", "/api/relay/public"),
            ("GET", "/api/relay/public/account"),
            ("POST", "/api/relay/public/join"),
            ("GET", "/api/relay/agents"),
            ("POST", "/api/relay/test"),
            ("GET", "/api/relay/instances"),
        ] {
            // Every route answers JSON (success or error); unmatched paths
            // fall through to the frontend's HTML instead.
            let response = app
                .clone()
                .oneshot(authed(method, uri, "{}"))
                .await
                .unwrap();
            assert!(is_json(&response), "{method} {uri}: {}", response.status());
        }
        let missing = app
            .clone()
            .oneshot(authed("GET", "/api/relay/nope", ""))
            .await
            .unwrap();
        assert!(!is_json(&missing));
        state.shutdown();
    }

    #[tokio::test]
    async fn wave_2b_redesign_routes_are_mounted() {
        let (state, app) = app();
        for (method, uri) in [
            ("GET", "/api/companion/snapshot?dayStart=0"),
            ("POST", "/api/companion/toggle"),
            ("POST", "/api/companion/pin"),
            ("POST", "/api/companion/messages"),
            ("POST", "/api/companion/notify"),
        ] {
            // JSON (501 while stubbed, or the track's answer); unmatched
            // paths fall through to the frontend's HTML instead
            let response = app
                .clone()
                .oneshot(authed(method, uri, "{}"))
                .await
                .unwrap();
            assert!(is_json(&response), "{method} {uri}: {}", response.status());
        }
        state.shutdown();
    }

    #[tokio::test]
    async fn wave_final_routes_are_mounted() {
        let (state, app) = app();
        for (method, uri) in [
            // Network-first
            ("PUT", "/api/connectors/github/secret"),
            ("PUT", "/api/connectors/filesystem/config"),
            ("POST", "/api/triggers/cron-preview"),
            ("POST", "/api/triggers/calendar-preview"),
            ("GET", "/api/workflow-templates"),
            ("POST", "/api/runs/r1/file"),
            ("POST", "/api/llm-keys/anthropic/test"),
            ("GET", "/api/ollama"),
            ("GET", "/api/gateway/status"),
            ("GET", "/api/gateway/config"),
            ("PUT", "/api/gateway/config"),
            ("GET", "/api/gateway/manifest"),
            ("PUT", "/api/gateway/manifest"),
            ("GET", "/api/gateway/callers"),
            ("PUT", "/api/gateway/callers/i2"),
            ("DELETE", "/api/gateway/callers/i2"),
            ("GET", "/api/gateway/usage/i2"),
            ("POST", "/api/gateway/chat"),
            ("GET", "/api/gateway/holds"),
            ("POST", "/api/gateway/holds/t1/decide"),
            ("GET", "/api/gateway/assignments"),
            ("POST", "/api/gateway/assignments/t1/close"),
        ] {
            // JSON (501 while stubbed, or the track's answer)
            let response = app
                .clone()
                .oneshot(authed(method, uri, "{}"))
                .await
                .unwrap();
            assert!(is_json(&response), "{method} {uri}: {}", response.status());
        }
        state.shutdown();
    }

    #[tokio::test]
    async fn wave_a_routes_are_mounted() {
        let (state, app) = app();
        let definition = r#"{"id":"c1","name":"C","description":"","version":"1.0.0",
            "owner":{"kind":"user","id":"ada","displayName":"Ada"},
            "createdAt":null,"updatedAt":null}"#;
        let owner = r#"{"newOwner":{"kind":"user","id":"bob","displayName":"Bob"}}"#;
        for (method, uri, body) in [
            ("GET", "/api/capabilities", "{}"),
            ("POST", "/api/capabilities", definition),
            ("GET", "/api/capabilities/c1", "{}"),
            ("PUT", "/api/capabilities/c1", definition),
            ("DELETE", "/api/capabilities/c1", "{}"),
            ("POST", "/api/capabilities/c1/transfer", owner),
            ("GET", "/api/capabilities/c1/providers", "{}"),
            ("GET", "/api/capabilities/c1/breakdown", "{}"),
            ("GET", "/api/capabilities/c1/evidence?windowDays=7", "{}"),
            ("GET", "/api/capabilities/c1/ambiguities", "{}"),
            ("POST", "/api/capabilities/c1/ambiguities/k1/promote", "{}"),
        ] {
            // JSON (501 while stubbed, or the track's answer)
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
            ("GET", "/api/aws/bedrock-api-key", "{}"),
            ("GET", "/api/aws/sso/login/l1", "{}"),
            ("POST", "/api/rpc/get_litellm_gateway_status", "{}"),
        ] {
            // JSON (501 while stubbed, or the track's answer)
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
            ("GET", "/api/approvals/pending", "{}"),
        ] {
            // JSON (501 while stubbed, or the track's answer)
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
                "/api/agents/..%2Fetc/messages",
                r#"{"text":"x"}"#,
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
                "/api/triggers",
                r#"{"name": "Hook", "sourceType": "webhook", "sourceConfig": {},
                    "targetType": "agent", "targetId": "writer"}"#,
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
        // Everything else under /api/triggers still needs the token
        let response = app.clone().oneshot(get("/api/triggers")).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let listed = json(
            app.oneshot(authed("GET", "/api/triggers", ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(listed[0]["hasSecret"], true);
        assert!(!listed.to_string().contains(&secret));
        state.shutdown();
    }
}
