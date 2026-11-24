//! `POST /fetch`: an HTTP request on an agent's behalf, to a host in its
//! agent.md `network.allow` list.
//!
//! Body: `{ agent_id?, url, method?, headers?, body?, body_base64? }`.
//! Returns `{ status, headers, body | bodyBase64 }`. Redirects are returned,
//! not followed, so a redirect can't lead outside the allowlist.
//!
//! With `key: "<name>"`, the request carries a Lock box key
//! (`services::lock_box`): released only if the agent may use it at this
//! host now, put in the header its rule names, after any header of that
//! name the agent set is dropped. The container never holds the value; it
//! is taken out of the reply too, should the site echo it. A redirect is
//! returned, never followed, so the key never reaches another host.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::{address, error, record, EgressKind, EgressLogEntry, ProxyContext};
use crate::audit;

const MAX_REQUEST_BYTES: usize = 10 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

/// Never forwarded in either direction
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "content-length",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Clone)]
struct FetchState {
    context: ProxyContext,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchRequest {
    #[serde(default, alias = "agent_id")]
    agent_id: Option<String>,
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default, alias = "body_base64")]
    body_base64: Option<String>,
    /// A Lock box key to add, by name
    #[serde(default)]
    key: Option<String>,
}

/// What stands in a reply where a released key was
const KEY_HIDDEN: &str = "[key hidden]";

/// `bytes` with every copy of `secret` replaced by [`KEY_HIDDEN`]
fn hide_key(bytes: Vec<u8>, secret: &str) -> Vec<u8> {
    let needle = secret.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(needle) {
            out.extend_from_slice(KEY_HIDDEN.as_bytes());
            i += needle.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    /// Set instead of `body` when the response isn't UTF-8
    #[serde(skip_serializing_if = "Option::is_none")]
    body_base64: Option<String>,
}

/// How the proxy's requests go out
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // The host's own proxy settings must not apply (or loop back here)
        .no_proxy()
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
}

/// The client for a request to `url`, pinned to `addrs` (checked by
/// `address::reachable`) so the name is never resolved again. An IP literal
/// needs no pin: the shared client connects to it as checked.
fn client_for(shared: &reqwest::Client, url: &Url, addrs: &[SocketAddr]) -> reqwest::Client {
    match url.domain() {
        Some(name) => client_builder()
            .resolve_to_addrs(name, addrs)
            .build()
            .unwrap_or_default(),
        None => shared.clone(),
    }
}

pub(super) fn router(context: ProxyContext) -> Router {
    let client = client_builder().build().unwrap_or_default();
    Router::new()
        .route("/fetch", post(handle_fetch))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(FetchState { context, client })
}

async fn handle_fetch(
    State(state): State<FetchState>,
    headers: HeaderMap,
    body: Result<Json<FetchRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let started = Instant::now();
    // Authenticate before looking at the body
    if let Err(response) = state.context.authenticate(&headers, None) {
        return response;
    }
    let Json(request) = match body {
        Ok(body) => body,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, &rejection.body_text()),
    };
    let agent_id = match state
        .context
        .authenticate(&headers, request.agent_id.as_deref())
    {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let method = request
        .method
        .as_deref()
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase();
    let Ok(method) = reqwest::Method::from_bytes(method.as_bytes()) else {
        return error(StatusCode::BAD_REQUEST, "invalid method");
    };
    if method == reqwest::Method::CONNECT {
        return error(StatusCode::BAD_REQUEST, "use the proxy's CONNECT support");
    }
    let url = match Url::parse(&request.url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() => url,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "url must be an absolute http(s) URL",
            )
        }
    };
    let key_name = request
        .key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string);
    let target = match &key_name {
        Some(key) => format!("{method} {} [key {key}]", redact_url(&url)),
        None => format!("{method} {}", redact_url(&url)),
    };
    let host = url.host_str().unwrap_or_default().to_string();

    let entry = |allowed: bool, status: Option<u16>, error: Option<String>| EgressLogEntry {
        ts: audit::now_ms() as f64,
        agent_id: agent_id.clone(),
        kind: EgressKind::Fetch,
        target: target.clone(),
        allowed,
        status,
        duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
        error,
    };

    let capabilities = match state.context.capabilities(&agent_id) {
        Ok(capabilities) => capabilities,
        Err(response) => return response,
    };
    // `port()` is `None` for the scheme's default port
    if !capabilities.allows_request(&host, url.port(), method.as_str()) {
        let message = format!("{method} {host} is not in this agent's network.allow");
        record(entry(false, Some(403), Some(message.clone()))).await;
        return error(StatusCode::FORBIDDEN, &message);
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let named = |ip| capabilities.names_address(ip, url.port());
    let addrs = match address::reachable(&host, port, named).await {
        Ok(addrs) => addrs,
        Err(message) => {
            record(entry(false, Some(403), Some(message.clone()))).await;
            return error(StatusCode::FORBIDDEN, &message);
        }
    };

    // Every egress check passed: now the Lock box's own rules
    let released = match &key_name {
        None => None,
        Some(key) => {
            let Some(keys) = &state.context.keys else {
                let message = "this proxy has no Lock box".to_string();
                record(entry(false, Some(403), Some(message.clone()))).await;
                return error(StatusCode::FORBIDDEN, &message);
            };
            match keys.release(&agent_id, key, &url).await {
                Ok(released) => Some(released),
                Err(message) => {
                    record(entry(false, Some(403), Some(message.clone()))).await;
                    return error(StatusCode::FORBIDDEN, &message);
                }
            }
        }
    };

    let body = match (&request.body, &request.body_base64) {
        (Some(_), Some(_)) => {
            return error(StatusCode::BAD_REQUEST, "send body or bodyBase64, not both")
        }
        (Some(text), None) => text.clone().into_bytes(),
        (None, Some(encoded)) => match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(bytes) => bytes,
            Err(_) => return error(StatusCode::BAD_REQUEST, "bodyBase64 is not valid base64"),
        },
        (None, None) => Vec::new(),
    };

    let mut outgoing = client_for(&state.client, &url, &addrs).request(method, url);
    for (name, value) in &request.headers {
        let lower = name.to_ascii_lowercase();
        // The agent's own copy of the key's header never goes out
        let replaced = released.as_ref().is_some_and(|k| k.header == lower);
        if !HOP_BY_HOP.contains(&lower.as_str()) && !replaced {
            outgoing = outgoing.header(name, value);
        }
    }
    if let Some(key) = &released {
        let Ok(mut value) = axum::http::HeaderValue::from_str(&key.header_value) else {
            return error(
                StatusCode::FORBIDDEN,
                "the key can't go in a request header",
            );
        };
        // Kept out of any debug output of the request
        value.set_sensitive(true);
        outgoing = outgoing.header(key.header.as_str(), value);
    }
    if !body.is_empty() {
        outgoing = outgoing.body(body);
    }

    let response = match outgoing.send().await {
        Ok(response) => response,
        Err(e) => {
            // Without the URL: it may carry secrets in its query
            let message = e.without_url().to_string();
            record(entry(true, None, Some(message.clone()))).await;
            return error(StatusCode::BAD_GATEWAY, &message);
        }
    };
    let status = response.status().as_u16();
    let secret = released.as_ref().map(|k| k.secret.as_str());
    let headers: BTreeMap<String, String> = response
        .headers()
        .iter()
        .filter(|(name, _)| !HOP_BY_HOP.contains(&name.as_str()))
        .filter_map(|(name, value)| {
            let value = value.to_str().ok()?.to_string();
            let value = match secret {
                Some(secret) if !secret.is_empty() => value.replace(secret, KEY_HIDDEN),
                _ => value,
            };
            Some((name.to_string(), value))
        })
        .collect();
    let bytes = match read_limited(response).await {
        Ok(bytes) => bytes,
        Err(message) => {
            record(entry(true, Some(status), Some(message.clone()))).await;
            return error(StatusCode::BAD_GATEWAY, &message);
        }
    };
    record(entry(true, Some(status), None)).await;
    let bytes = match secret {
        Some(secret) => hide_key(bytes, secret),
        None => bytes,
    };

    let (body, body_base64) = match String::from_utf8(bytes) {
        Ok(text) => (Some(text), None),
        Err(e) => (
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
        ),
    };
    Json(FetchResponse {
        status,
        headers,
        body,
        body_base64,
    })
    .into_response()
}

async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "response is larger than {} MB",
                MAX_RESPONSE_BYTES / 1024 / 1024
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// `https://host:port/path`: no credentials, query, or fragment.
fn redact_url(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{host}:{port}{}", url.scheme(), url.path()),
        None => format!("{}://{host}{}", url.scheme(), url.path()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{allow_hosts, context};
    use super::*;
    use crate::runtime::ProxyTokens;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use axum::routing::any;
    use serde_json::Value;
    use tower::ServiceExt;

    /// An upstream that reports what it received.
    async fn start_upstream() -> u16 {
        let app = Router::new()
            .route(
                "/echo",
                any(
                    |method: axum::http::Method, headers: HeaderMap, body: String| async move {
                        let custom = headers
                            .get("x-custom")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        format!("{method} {custom} {body}")
                    },
                ),
            )
            .route(
                "/bytes",
                any(|| async {
                    (
                        [(header::CONTENT_TYPE, "application/octet-stream")],
                        vec![0xffu8, 0xfe],
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        port
    }

    fn post_fetch(token: Option<&str>, body: Value) -> Request<Body> {
        let mut request = Request::post("/fetch").header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    async fn json_body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn forwards_allowed_requests() {
        let port = start_upstream().await;
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let app = router(context(
            tokens,
            allow_hosts(&[&format!("127.0.0.1:{port}")]),
        ));

        let response = app
            .clone()
            .oneshot(post_fetch(
                Some(&token),
                serde_json::json!({
                    "agent_id": "writer",
                    "url": format!("http://127.0.0.1:{port}/echo?secret=1"),
                    "method": "post",
                    "headers": { "X-Custom": "yes", "Proxy-Authorization": "leak" },
                    "body": "hello",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        assert_eq!(json["status"], 200);
        assert_eq!(json["body"], "POST yes hello");

        let response = app
            .oneshot(post_fetch(
                Some(&token),
                serde_json::json!({ "url": format!("http://127.0.0.1:{port}/bytes") }),
            ))
            .await
            .unwrap();
        let json = json_body(response).await;
        assert_eq!(json["bodyBase64"], "//4=");
        assert!(json.get("body").is_none());
    }

    #[tokio::test]
    async fn rejects_unauthenticated_and_disallowed_requests() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let app = router(context(tokens, allow_hosts(&["api.example.com"])));
        let request =
            |token: Option<&str>, body: Value| app.clone().oneshot(post_fetch(token, body));

        let allowed = serde_json::json!({ "url": "https://api.example.com/x" });
        for token in [None, Some("guess")] {
            let response = request(token, allowed.clone()).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let impersonating =
            serde_json::json!({ "agent_id": "admin", "url": "https://api.example.com/x" });
        let response = request(Some(&token), impersonating).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        for url in [
            "https://evil.example.org/x",
            "https://api.example.com:8443/x",
        ] {
            let response = request(Some(&token), serde_json::json!({ "url": url }))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{url}");
            assert!(json_body(response).await["error"]
                .as_str()
                .unwrap()
                .contains("network.allow"));
        }

        for url in ["file:///etc/passwd", "not a url"] {
            let response = request(Some(&token), serde_json::json!({ "url": url }))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{url}");
        }
    }

    #[tokio::test]
    async fn an_allowed_name_that_resolves_to_the_host_is_refused() {
        let port = start_upstream().await;
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let app = router(context(
            tokens,
            allow_hosts(&[&format!("localhost:{port}"), "*"]),
        ));

        for url in [
            format!("http://localhost:{port}/echo"),
            "http://169.254.169.254/latest/meta-data".to_string(),
        ] {
            let response = app
                .clone()
                .oneshot(post_fetch(Some(&token), serde_json::json!({ "url": url })))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{url}");
            assert!(json_body(response).await["error"]
                .as_str()
                .unwrap()
                .contains("internal address"));
        }
    }

    // --- Lock box keys ------------------------------------------------------

    use crate::services::egress::KeyRelease;
    use crate::services::lock_box::tests as lock_box;
    use crate::services::lock_box::ReleasedKey;
    use std::sync::{Arc, Mutex};

    const KEY: &str = "sk-proxy-test-key-91c2e7";

    /// Releases `KEY` as `x-api-key` for any request (the rules themselves
    /// are tested in `lock_box::tests`), and counts the releases
    struct AnyHost(Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl KeyRelease for AnyHost {
        async fn release(&self, agent: &str, key: &str, url: &Url) -> Result<ReleasedKey, String> {
            self.0.lock().unwrap().push(format!("{agent} {key} {url}"));
            if key != "api" {
                return Err(format!("Key '{key}' wasn't released: no such key"));
            }
            Ok(ReleasedKey {
                header: "x-api-key".into(),
                header_value: format!("Key {KEY}"),
                secret: KEY.into(),
            })
        }
    }

    /// The real Lock box, judging each request as if it went to
    /// `https://api.github.com` (a test upstream is plain HTTP on loopback)
    struct AsIfGitHub(crate::services::AppState);

    #[async_trait::async_trait]
    impl KeyRelease for AsIfGitHub {
        async fn release(&self, agent: &str, key: &str, url: &Url) -> Result<ReleasedKey, String> {
            let as_if = Url::parse(&format!("https://api.github.com{}", url.path())).unwrap();
            crate::services::lock_box::release(&self.0, agent, key, &as_if).await
        }
    }

    /// An upstream that keeps the key headers it got and echoes them back
    /// in its body and a header; `/redirect` sends the caller to `elsewhere`
    async fn start_key_upstream(elsewhere: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
        let got: Arc<Mutex<Vec<String>>> = Arc::default();
        let kept = got.clone();
        let app = Router::new()
            .route(
                "/echo",
                any(move |headers: HeaderMap| {
                    let kept = kept.clone();
                    async move {
                        let seen: Vec<String> = ["x-api-key", "authorization"]
                            .iter()
                            .flat_map(|name| headers.get_all(*name).iter())
                            .filter_map(|v| v.to_str().ok().map(str::to_string))
                            .collect();
                        kept.lock().unwrap().extend(seen.clone());
                        let echo = seen.join(" | ");
                        ([("x-echo", echo.clone())], format!("you sent: {echo}"))
                    }
                }),
            )
            .route(
                "/redirect",
                any(move || async move {
                    (
                        StatusCode::FOUND,
                        [(
                            header::LOCATION,
                            format!("http://127.0.0.1:{elsewhere}/steal"),
                        )],
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        (port, got)
    }

    /// A host that counts every request that reaches it
    async fn start_counter() -> (u16, Arc<Mutex<usize>>) {
        let hits: Arc<Mutex<usize>> = Arc::default();
        let counted = hits.clone();
        let app = Router::new().fallback(any(move || {
            let counted = counted.clone();
            async move {
                *counted.lock().unwrap() += 1;
                "stolen"
            }
        }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        (port, hits)
    }

    #[tokio::test]
    async fn a_key_replaces_the_agent_s_own_header_and_is_hidden_in_the_reply() {
        let (port, got) = start_key_upstream(1).await;
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let releases = Arc::new(AnyHost(Mutex::default()));
        let app = router(super::super::tests::context_with_keys(
            tokens,
            allow_hosts(&[&format!("127.0.0.1:{port}")]),
            releases.clone(),
        ));
        let response = app
            .oneshot(post_fetch(
                Some(&token),
                serde_json::json!({
                    "url": format!("http://127.0.0.1:{port}/echo"),
                    "headers": { "X-Api-Key": "agent-made", "x-api-key": "also-agent", "Authorization": "Bearer mine" },
                    "key": "api",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = json_body(response).await;
        // The upstream got the released key, once, and none of the agent's copies
        let seen = got.lock().unwrap().clone();
        assert_eq!(
            seen,
            [format!("Key {KEY}"), "Bearer mine".to_string()],
            "{seen:?}"
        );
        // What comes back never shows it
        let text = json.to_string();
        assert!(!text.contains(KEY), "{text}");
        assert!(text.contains("[key hidden]"), "{text}");
        assert_eq!(
            releases.0.lock().unwrap().as_slice(),
            [format!("writer api http://127.0.0.1:{port}/echo")]
        );
    }

    #[tokio::test]
    async fn a_key_never_follows_a_redirect_to_another_host() {
        let (elsewhere, hits) = start_counter().await;
        let (port, got) = start_key_upstream(elsewhere).await;
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let app = router(super::super::tests::context_with_keys(
            tokens,
            allow_hosts(&[
                &format!("127.0.0.1:{port}"),
                &format!("127.0.0.1:{elsewhere}"),
            ]),
            Arc::new(AnyHost(Mutex::default())),
        ));
        let response = app
            .oneshot(post_fetch(
                Some(&token),
                serde_json::json!({ "url": format!("http://127.0.0.1:{port}/redirect"), "key": "api" }),
            ))
            .await
            .unwrap();
        let json = json_body(response).await;
        assert_eq!(
            json["status"], 302,
            "the redirect is returned, not followed"
        );
        assert_eq!(*hits.lock().unwrap(), 0, "the other host got nothing");
        assert!(got.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_key_sends_nothing() {
        let (port, got) = start_key_upstream(1).await;
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let with_keys = router(super::super::tests::context_with_keys(
            tokens.clone(),
            allow_hosts(&[&format!("127.0.0.1:{port}")]),
            Arc::new(AnyHost(Mutex::default())),
        ));
        let without = router(context(
            tokens,
            allow_hosts(&[&format!("127.0.0.1:{port}")]),
        ));
        for (app, key) in [(with_keys, "unknown"), (without, "api")] {
            let response = app
                .oneshot(post_fetch(
                    Some(&token),
                    serde_json::json!({ "url": format!("http://127.0.0.1:{port}/echo"), "key": key }),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{key}");
        }
        assert!(
            got.lock().unwrap().is_empty(),
            "the upstream was never called"
        );
    }

    /// Every text cell of every table of `state.db`
    async fn database_text(state: &crate::services::AppState) -> String {
        state
            .db()
            .call(|conn| {
                let tables: Vec<String> = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
                    .query_map([], |row| row.get(0))?
                    .collect::<Result<_, _>>()?;
                let mut out = String::new();
                for table in tables {
                    let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\""))?;
                    let columns = stmt.column_count();
                    let mut rows = stmt.query([])?;
                    while let Some(row) = rows.next()? {
                        for i in 0..columns {
                            if let Ok(text) = row.get::<_, String>(i) {
                                out.push_str(&text);
                                out.push('\n');
                            }
                        }
                    }
                }
                Ok(out)
            })
            .await
            .unwrap()
    }

    /// A whole run's use of a key, then a search for its value everywhere
    /// it must never be: the reply, the run record (messages, approvals,
    /// run events), the decision record, every gold-copy file, and the logs
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_released_key_s_value_never_reaches_a_record_or_a_log() {
        let watching = crate::test_log::watch();
        let state = lock_box::state_with_agent();
        let run = lock_box::start_run(&state).await;
        lock_box::add_key(
            &state,
            "github",
            lock_box::rules(
                &["api.github.com"],
                crate::services::domain::lock_box::LockLevel::Ask,
            ),
        )
        .await;
        let (port, got) = start_key_upstream(1).await;
        let tokens = ProxyTokens::default();
        let token = tokens.issue(lock_box::AGENT);
        let app = router(super::super::tests::context_with_keys(
            tokens,
            allow_hosts(&[&format!("127.0.0.1:{port}")]),
            Arc::new(AsIfGitHub(state.clone())),
        ));
        let fetching = tokio::spawn(app.oneshot(post_fetch(
            Some(&token),
            serde_json::json!({ "url": format!("http://127.0.0.1:{port}/echo"), "key": "github" }),
        )));
        let card = lock_box::pending_key_approval(&state, &run.run_id).await;
        crate::services::chat::approve_action(
            &state,
            None,
            card.id,
            crate::model::ApprovalDecision::Approved,
        )
        .await
        .unwrap();
        let response = fetching.await.unwrap().unwrap();
        let reply = json_body(response).await.to_string();
        assert_eq!(
            got.lock().unwrap().as_slice(),
            [format!("Bearer {}", lock_box::VALUE)],
            "the site got the key"
        );

        let value = lock_box::VALUE;
        assert!(!reply.contains(value), "the reply: {reply}");
        let db = database_text(&state).await;
        assert!(db.contains("github"), "the search reads the run record");
        assert!(!db.contains(value), "state.db holds the value");
        let record = lock_box::decisions(&state);
        assert!(record.contains("- **Decision:** released"), "{record}");
        assert!(
            !record.contains(value),
            "the decision record holds the value"
        );
        let mut files = Vec::new();
        let root = state.store().layout().root.clone();
        let mut dirs = vec![root];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    files.push((path, String::from_utf8_lossy(&bytes).into_owned()));
                }
            }
        }
        for (path, text) in &files {
            assert!(!text.contains(value), "{} holds the value", path.display());
        }
        if watching {
            assert!(
                !crate::test_log::contains(value),
                "a log line held the value"
            );
        }
        state.shutdown();
    }
    #[test]
    fn redacts_urls() {
        let url = Url::parse("https://user:pw@api.example.com:8443/a/b?key=secret#frag").unwrap();
        assert_eq!(redact_url(&url), "https://api.example.com:8443/a/b");
        let url = Url::parse("https://api.example.com/?token=x").unwrap();
        assert_eq!(redact_url(&url), "https://api.example.com/");
    }
}
