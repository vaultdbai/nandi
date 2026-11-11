//! Web transport: REST (`/api/*`), events (`/ws`), and the frontend (`/`),
//! all behind token auth. A thin adapter over `services/`, like the Tauri
//! commands. No Tauri types here.
//!
//! Binds `127.0.0.1:0` (a random free port) by default; `0.0.0.0` only
//! with an explicit `--server`.

mod access;
mod analysis;
// Wave CD (see docs/developer/interaction-layer.md)
mod ask;
pub mod auth;
// Wave B
mod aws_auth;
mod bedrock;
mod capabilities;
mod capability;
// Wave A
mod capability_registries;
mod companion;
// Wave EF
mod config;
mod connectors;
mod decisions;
mod delegation;
mod knowme;
// Wave final
mod delegation_clarification;
mod email;
mod gateway;
mod groups;
mod harness_sessions;
mod harnesses;
mod help;
mod identity;
mod images;
mod live;
mod machine;
mod manual_steps;
mod marketplace;
mod marketplace_search;
mod mascot;
mod memory;
mod network;
mod notes;
mod palette;
mod pilots;
mod policies;
mod relay;
mod remote;
mod repo;
mod requests;
mod routes;
mod shared;
mod static_files;
mod studio;
mod tasks;
mod teams;
mod teams_roster;
mod tools;
mod triggers;
mod work;
mod workflow_builder;
mod workspace;
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
                .merge(marketplace::api())
                .merge(decisions::api())
                .merge(knowme::api())
                .merge(mascot::api())
                .merge(triggers::api())
                .merge(repo::api())
                .merge(pilots::api())
                .merge(policies::api())
                .merge(connectors::api())
                .merge(images::api())
                .merge(machine::api())
                .merge(help::api())
                // Wave 2A
                .merge(capability::api())
                .merge(relay::api())
                .merge(delegation::api())
                .merge(teams::api())
                .merge(shared::api())
                // Wave 2B redesign
                .merge(analysis::api())
                .merge(workflow_builder::api())
                .merge(marketplace_search::api())
                .merge(companion::api())
                .merge(palette::api())
                // Wave final
                .merge(memory::api())
                // Wave final, network-first
                .merge(identity::api())
                .merge(remote::api())
                .merge(tasks::api())
                .merge(requests::api())
                .merge(delegation_clarification::api())
                .merge(gateway::api())
                .merge(network::api())
                .merge(notes::api())
                .merge(live::api())
                .merge(email::api())
                .merge(workspace::api())
                .merge(access::api())
                .merge(teams_roster::api())
                .merge(groups::api())
                // Wave A
                .merge(capabilities::api())
                .merge(tools::api())
                .merge(capability_registries::api())
                // Wave B
                .merge(harnesses::api())
                .merge(harness_sessions::api())
                .merge(aws_auth::api())
                .merge(bedrock::api())
                // Wave CD (see docs/developer/interaction-layer.md)
                .merge(ask::api())
                .merge(studio::api())
                .merge(work::api())
                // Wave EF (see docs/developer/network-and-reassignment.md)
                .merge(manual_steps::api())
                .merge(config::api()),
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
            ("GET", "/api/capabilities/summarize/agents"),
            ("GET", "/api/agents/a/card"),
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
            ("GET", "/api/delegation/providers?capability=x"),
            ("POST", "/api/delegation/tasks"),
            ("GET", "/api/delegation/history?limit=5"),
            ("GET", "/api/delegation/returns"),
            ("GET", "/api/agents/a/delegation"),
            ("PUT", "/api/agents/a/delegation"),
            ("GET", "/api/teams"),
            ("POST", "/api/teams"),
            ("GET", "/api/teams/org"),
            ("POST", "/api/teams/org/join"),
            ("POST", "/api/teams/org/leave"),
            ("POST", "/api/teams/org/sync"),
            ("POST", "/api/teams/org/agents/a"),
            ("DELETE", "/api/teams/org/agents/a"),
            ("GET", "/api/shared/with-me"),
            ("GET", "/api/shared/by-me"),
            ("GET", "/api/trust/policies"),
            ("PUT", "/api/trust/policies"),
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
            ("POST", "/api/runs/r1/analyze"),
            ("GET", "/api/runs/r1/summary"),
            ("GET", "/api/workflows/w1/graph"),
            ("PUT", "/api/workflows/w1/yaml"),
            ("GET", "/api/marketplace/search?q=x"),
            ("GET", "/api/companion/snapshot?dayStart=0"),
            ("POST", "/api/companion/toggle"),
            ("POST", "/api/companion/pin"),
            ("POST", "/api/companion/messages"),
            ("POST", "/api/companion/notify"),
            ("GET", "/api/palette/search?q=x"),
            ("POST", "/api/palette/actions/open_view"),
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
            ("GET", "/api/agents/a1/memory"),
            ("PUT", "/api/agents/a1/memory/0"),
            ("DELETE", "/api/agents/a1/memory/0"),
            ("POST", "/api/agents/a1/memory/refresh"),
            ("GET", "/api/agents/a1/memory-enabled"),
            ("PUT", "/api/agents/a1/memory-enabled"),
            // Network-first
            ("GET", "/api/remote/status"),
            ("PUT", "/api/remote/config"),
            ("DELETE", "/api/remote"),
            ("POST", "/api/remote/test"),
            ("POST", "/api/remote/push"),
            ("POST", "/api/remote/pull"),
            ("GET", "/api/remote/branches"),
            ("GET", "/api/identity/status"),
            ("GET", "/api/identity/verified"),
            ("PUT", "/api/identity/display-name"),
            ("PUT", "/api/identity/user-slug"),
            ("PUT", "/api/identity/onboarding-step"),
            ("GET", "/api/github/sign-in"),
            ("DELETE", "/api/github/sign-in"),
            ("POST", "/api/github/sign-in/start"),
            ("POST", "/api/github/sign-in/poll"),
            ("PUT", "/api/connectors/github/secret"),
            ("PUT", "/api/connectors/filesystem/config"),
            ("POST", "/api/triggers/cron-preview"),
            ("POST", "/api/triggers/calendar-preview"),
            ("GET", "/api/workflow-templates"),
            ("POST", "/api/runs/r1/file"),
            ("POST", "/api/llm-keys/anthropic/test"),
            ("GET", "/api/ollama"),
            ("GET", "/api/tasks"),
            ("GET", "/api/tasks/t1"),
            ("POST", "/api/tasks/t1/cancel"),
            ("GET", "/api/tasks/t1/status"),
            ("GET", "/api/tasks/t1/events"),
            ("GET", "/api/tasks/t1/messages"),
            ("GET", "/api/requests/t1"),
            ("POST", "/api/requests/t1/retry"),
            ("GET", "/api/runs/r1/request"),
            ("GET", "/api/eta/summarize"),
            ("POST", "/api/delegation/t1/clarify"),
            ("POST", "/api/delegation/t1/escalate/q1"),
            ("GET", "/api/delegation/pending-clarifications"),
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
            ("GET", "/api/agents/a1/clarification-policy"),
            ("GET", "/api/network/snapshot"),
            ("POST", "/api/network/refresh"),
            ("GET", "/api/teams/t1/roster"),
            ("GET", "/api/teams/t1/members/ada/profile"),
            ("GET", "/api/members/by-instance/i2"),
            ("PUT", "/api/members/i2/trust"),
            ("POST", "/api/members/i2/agents/a1/block"),
            ("DELETE", "/api/members/i2/agents/a1/block"),
            ("POST", "/api/members/i2/delegations/pause"),
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
        let registry = r#"{"url":"https://example.com/caps.git"}"#;
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
            ("GET", "/api/capabilities/declared", "{}"),
            ("GET", "/api/capabilities/c1/agents", "{}"),
            ("GET", "/api/tools", "{}"),
            ("GET", "/api/tools/usage", "{}"),
            ("GET", "/api/tools/builtin.shell", "{}"),
            (
                "PUT",
                "/api/tools/builtin.shell/enabled",
                r#"{"enabled":false}"#,
            ),
            ("GET", "/api/capability-registries", "{}"),
            ("POST", "/api/capability-registries", registry),
            (
                "DELETE",
                "/api/capability-registries?url=https%3A%2F%2Fexample.com%2Fcaps.git",
                "{}",
            ),
            ("POST", "/api/capability-registries/sync", registry),
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
            ("GET", "/api/harnesses", "{}"),
            ("GET", "/api/harnesses/default", "{}"),
            (
                "PUT",
                "/api/harnesses/default",
                r#"{"harnessId":"claude-code","model":null}"#,
            ),
            ("DELETE", "/api/harnesses/default", "{}"),
            ("GET", "/api/harnesses/nope/health", "{}"),
            ("POST", "/api/harnesses/nope/test", "{}"),
            ("GET", "/api/harness-sessions?limit=5", "{}"),
            ("GET", "/api/harness-sessions/s1", "{}"),
            ("GET", "/api/harness-sessions/s1/events?afterSeq=0", "{}"),
            ("DELETE", "/api/harness-sessions/s1", "{}"),
            ("GET", "/api/aws/bedrock-api-key", "{}"),
            ("GET", "/api/aws/sso/login/l1", "{}"),
            ("GET", "/api/bedrock/gateway", "{}"),
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
        let ask = r#"{"text":"What can you do?","context":{"currentView":"home"}}"#;
        let artifact = r#"{"artifact":{"kind":"agent","id":"nope"}}"#;
        let preview = r#"{"artifact":{"kind":"agent","id":"nope"},"instruction":"x"}"#;
        // Side-effect-free routes only (no model calls, no git writes)
        for (method, uri, body) in [
            ("POST", "/api/ask", ask),
            ("GET", "/api/ask/history", "{}"),
            ("POST", "/api/studio/load", artifact),
            ("POST", "/api/studio/preview", preview),
            ("GET", "/api/studio/push-targets", "{}"),
            (
                "POST",
                "/api/studio/validate",
                r#"{"artifact":{"kind":"agent","id":"a"},"content":"---\nmodel: x\n---\n"}"#,
            ),
            ("GET", "/api/work", "{}"),
            ("GET", "/api/work/w1", "{}"),
            ("POST", "/api/work/w1/pause", "{}"),
            ("POST", "/api/work/w1/resume", "{}"),
            ("POST", "/api/work/w1/cancel", "{}"),
            ("GET", "/api/work/dismissed", "{}"),
            ("POST", "/api/work/w1/dismiss", "{}"),
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
