//! `POST /fetch`: an HTTP request on an agent's behalf, to a host in its
//! skill.md `network.allow` list.
//!
//! Body: `{ agent_id?, url, method?, headers?, body?, body_base64? }`.
//! Returns `{ status, headers, body | bodyBase64 }`. Redirects are returned,
//! not followed, so a redirect can't lead outside the allowlist.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::{error, record, EgressKind, EgressLogEntry, ProxyContext};
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

pub(super) fn router(context: ProxyContext) -> Router {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // The host's own proxy settings must not apply (or loop back here)
        .no_proxy()
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();
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
    let target = format!("{method} {}", redact_url(&url));
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

    let mut outgoing = state.client.request(method, url);
    for (name, value) in &request.headers {
        if !HOP_BY_HOP.contains(&name.to_ascii_lowercase().as_str()) {
            outgoing = outgoing.header(name, value);
        }
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
    let headers: BTreeMap<String, String> = response
        .headers()
        .iter()
        .filter(|(name, _)| !HOP_BY_HOP.contains(&name.as_str()))
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
        .collect();
    let bytes = match read_limited(response).await {
        Ok(bytes) => bytes,
        Err(message) => {
            record(entry(true, Some(status), Some(message.clone()))).await;
            return error(StatusCode::BAD_GATEWAY, &message);
        }
    };
    record(entry(true, Some(status), None)).await;

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

    #[test]
    fn redacts_urls() {
        let url = Url::parse("https://user:pw@api.example.com:8443/a/b?key=secret#frag").unwrap();
        assert_eq!(redact_url(&url), "https://api.example.com:8443/a/b");
        let url = Url::parse("https://api.example.com/?token=x").unwrap();
        assert_eq!(redact_url(&url), "https://api.example.com/");
    }
}
