//! Model passthrough for harness CLIs: `/anthropic/...`, `/bedrock/...`
//! and `/openai/...`.
//!
//! A harness CLI speaks its provider's own API, pointed at the proxy
//! (`ANTHROPIC_BASE_URL=http://nandi-egress:11435/anthropic`) with its
//! session's proxy token where the provider key would go (Bedrock:
//! `ANTHROPIC_BEDROCK_BASE_URL=…/bedrock` with
//! `CLAUDE_CODE_SKIP_BEDROCK_AUTH=1`, so the host signs). The host checks
//! the token, finds the session's [`ModelUpstream`], swaps in the real
//! credential and forwards the call, streaming the answer back. Provider
//! keys, subscription tokens and gateway keys stay on the host: the
//! container holds only the proxy token, which is worthless once its
//! session ends.
//!
//! - The token comes in `Authorization: Bearer` or `x-api-key`, whichever
//!   the CLI sends. A missing, wrong or revoked token gets 401
//! - A token with no open model session gets 403
//! - Only the API's model paths are forwarded ([`ModelApi::allows`]), so a
//!   gateway's admin routes stay out of reach even though the host holds
//!   its key
//! - Every call is audited to `egress.log` (method and path, no query, no
//!   credentials)

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::Router;

use super::sigv4::AwsSigner;
use super::{error, record, EgressKind, EgressLogEntry};
use crate::runtime::ProxyTokens;

/// Requests carry whole conversations; 32 MB is well above any CLI's
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// The Anthropic beta a Claude subscription token needs on every call
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Headers never forwarded in either direction: hop-by-hop ones, the
/// incoming credentials (the host adds its own) and the length (re-set)
const DROPPED_HEADERS: &[&str] = &[
    "authorization",
    "connection",
    "content-length",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "x-amz-content-sha256",
    "x-amz-date",
    "x-amz-security-token",
    "x-api-key",
];

/// The Bedrock runtime calls a Claude model session makes
const BEDROCK_ACTIONS: &[&str] = &[
    "invoke",
    "invoke-with-response-stream",
    "converse",
    "converse-stream",
    "count-tokens",
];

/// The provider APIs the proxy passes through, each under its own prefix
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelApi {
    /// Anthropic Messages: Anthropic, Microsoft Foundry, LiteLLM, a team's
    /// own Anthropic-compatible gateway
    Anthropic,
    /// Bedrock runtime's own API, signed with the host's AWS identity or
    /// its Bedrock API key
    Bedrock,
    /// OpenAI's Responses and Chat Completions (Codex)
    OpenAi,
}

impl ModelApi {
    /// Where the proxy serves it
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Anthropic => "/anthropic",
            Self::Bedrock => "/bedrock",
            Self::OpenAi => "/openai",
        }
    }

    /// Whether `path` (after the prefix) is one of the API's model calls
    pub fn allows(self, path: &str) -> bool {
        match self {
            Self::Anthropic => matches!(
                path,
                "/v1/messages" | "/v1/messages/count_tokens" | "/v1/models"
            ),
            Self::OpenAi => matches!(
                path,
                "/v1/responses" | "/v1/chat/completions" | "/v1/models"
            ),
            // `/model/{model id}/{action}`, the model ID percent-encoded
            Self::Bedrock => match path.strip_prefix("/model/").map(|p| p.split('/')) {
                Some(mut parts) => {
                    let model = parts.next().unwrap_or_default();
                    let action = parts.next().unwrap_or_default();
                    !model.is_empty()
                        && !model.starts_with('.')
                        && BEDROCK_ACTIONS.contains(&action)
                        && parts.next().is_none()
                }
                None => false,
            },
        }
    }
}

/// A secret that never prints
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// How the host signs a session's model calls
#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamAuth {
    /// `x-api-key`: an Anthropic or Foundry API key
    ApiKey(Secret),
    /// `Authorization: Bearer`: a gateway's key
    Bearer(Secret),
    /// A Claude subscription token: a bearer token plus Anthropic's OAuth
    /// beta
    ClaudeSubscription(Secret),
    /// AWS SigV4 as the host's own AWS identity
    AwsSigV4(AwsSigner),
}

/// Where a session's model calls go, and the credential the host adds
#[derive(Debug, Clone, PartialEq)]
pub struct ModelUpstream {
    pub api: ModelApi,
    /// e.g. `https://api.anthropic.com`; the call's path is appended
    pub base_url: String,
    pub auth: UpstreamAuth,
}

impl ModelUpstream {
    /// Anthropic's own API
    pub const ANTHROPIC_URL: &'static str = "https://api.anthropic.com";

    pub fn anthropic(base_url: impl Into<String>, auth: UpstreamAuth) -> Self {
        Self {
            api: ModelApi::Anthropic,
            base_url: base_url.into(),
            auth,
        }
    }

    /// OpenAI's own API
    pub fn openai(auth: UpstreamAuth) -> Self {
        Self {
            api: ModelApi::OpenAi,
            base_url: "https://api.openai.com".to_string(),
            auth,
        }
    }

    /// Bedrock runtime in `region`
    pub fn bedrock(region: &str, auth: UpstreamAuth) -> Self {
        Self {
            api: ModelApi::Bedrock,
            base_url: format!("https://bedrock-runtime.{region}.amazonaws.com"),
            auth,
        }
    }

    fn url(&self, path: &str, query: Option<&str>) -> String {
        let base = self.base_url.trim_end_matches('/');
        match query {
            Some(query) => format!("{base}{path}?{query}"),
            None => format!("{base}{path}"),
        }
    }

    /// The headers that carry the host's credential on a call to `url`
    /// with `body`, and the CLI's `anthropic-beta` list (with the OAuth beta
    /// for a subscription)
    async fn credential_headers(
        &self,
        method: &str,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<(String, String)>, String> {
        let betas = headers
            .get("anthropic-beta")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bearer = |token: &Secret| {
            (
                "authorization".to_string(),
                format!("Bearer {}", token.expose()),
            )
        };
        let mut added = match &self.auth {
            UpstreamAuth::ApiKey(key) => vec![("x-api-key".to_string(), key.expose().to_string())],
            UpstreamAuth::Bearer(token) => vec![bearer(token)],
            UpstreamAuth::ClaudeSubscription(token) => {
                let betas = with_oauth_beta(headers.get("anthropic-beta"));
                return Ok(vec![bearer(token), ("anthropic-beta".to_string(), betas)]);
            }
            UpstreamAuth::AwsSigV4(signer) => {
                let content_type = headers
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok());
                signer.sign(method, url, content_type, body).await?
            }
        };
        added.extend(betas.map(|betas| ("anthropic-beta".to_string(), betas)));
        Ok(added)
    }
}

/// The CLI's `anthropic-beta` list with the OAuth beta added once
fn with_oauth_beta(betas: Option<&HeaderValue>) -> String {
    let mut betas: Vec<&str> = betas
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if !betas.contains(&OAUTH_BETA) {
        betas.push(OAUTH_BETA);
    }
    betas.join(",")
}

/// The open model sessions, by proxy identity (the agent, or a test
/// session's identity). A session is open while its harness runs: the
/// [`ModelSessionGuard`] closes it, however the run ends.
#[derive(Clone, Default)]
pub struct ModelSessions(Arc<RwLock<HashMap<String, ModelUpstream>>>);

impl ModelSessions {
    /// The proxy's own sessions
    pub fn global() -> &'static Self {
        static SESSIONS: OnceLock<ModelSessions> = OnceLock::new();
        SESSIONS.get_or_init(Self::default)
    }

    /// Opens `identity`'s model session (replacing an older one) until the
    /// guard drops
    #[must_use = "the session closes when the guard drops"]
    pub fn open(&self, identity: &str, upstream: ModelUpstream) -> ModelSessionGuard {
        self.write().insert(identity.to_string(), upstream);
        ModelSessionGuard {
            sessions: self.clone(),
            identity: identity.to_string(),
        }
    }

    fn upstream(&self, identity: &str, api: ModelApi) -> Option<ModelUpstream> {
        let sessions = self.0.read().unwrap_or_else(|e| e.into_inner());
        sessions.get(identity).filter(|u| u.api == api).cloned()
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, ModelUpstream>> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// Closes a model session when dropped
pub struct ModelSessionGuard {
    sessions: ModelSessions,
    identity: String,
}

impl Drop for ModelSessionGuard {
    fn drop(&mut self) {
        self.sessions.write().remove(&self.identity);
    }
}

#[derive(Clone)]
pub(super) struct ModelProxyState {
    tokens: ProxyTokens,
    sessions: ModelSessions,
    client: reqwest::Client,
}

impl ModelProxyState {
    pub(super) fn new(tokens: ProxyTokens, sessions: ModelSessions) -> Self {
        let client = reqwest::Client::builder()
            // A redirect would carry the host's credential somewhere else
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            tokens,
            sessions,
            client,
        }
    }
}

pub(super) fn router(state: ModelProxyState) -> Router {
    Router::new()
        .route(
            "/openai/{*path}",
            any(|state: State<ModelProxyState>, request: Request| {
                forward(state, ModelApi::OpenAi, request)
            }),
        )
        .route(
            "/bedrock/{*path}",
            any(|state: State<ModelProxyState>, request: Request| {
                forward(state, ModelApi::Bedrock, request)
            }),
        )
        .route(
            "/anthropic/{*path}",
            any(|state: State<ModelProxyState>, request: Request| {
                forward(state, ModelApi::Anthropic, request)
            }),
        )
        .with_state(state)
}

/// The model routes alone on a loopback port, for live tests
#[cfg(test)]
pub(crate) async fn serve_for_tests(
    tokens: ProxyTokens,
    sessions: ModelSessions,
) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(ModelProxyState::new(tokens, sessions));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

/// The proxy token a CLI sends where its provider key would go
fn proxy_token(headers: &HeaderMap) -> Option<&str> {
    super::llm_proxy::bearer_token(headers).or_else(|| {
        headers
            .get("x-api-key")?
            .to_str()
            .ok()
            .filter(|token| !token.is_empty())
    })
}

async fn forward(
    State(state): State<ModelProxyState>,
    api: ModelApi,
    request: Request,
) -> Response {
    let started = Instant::now();
    // Authenticate before looking at anything else
    let Some(identity) = proxy_token(request.headers()).and_then(|t| state.tokens.agent_for(t))
    else {
        return error(StatusCode::UNAUTHORIZED, "missing or invalid proxy token");
    };
    let path = request
        .uri()
        .path()
        .strip_prefix(api.prefix())
        .unwrap_or_default()
        .to_string();
    let method = request.method().clone();
    let target = format!("{method} {}{path}", api.prefix());
    let entry = |allowed: bool, status: Option<u16>, message: Option<String>| EgressLogEntry {
        ts: crate::db::ts(crate::db::now_ms()),
        agent_id: identity.clone(),
        kind: EgressKind::Llm,
        target: target.clone(),
        allowed,
        status,
        duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
        error: message,
    };

    if !api.allows(&path) {
        let message = format!("{target} is not a model call");
        record(entry(false, Some(404), Some(message.clone()))).await;
        return error(StatusCode::NOT_FOUND, &message);
    }
    let Some(upstream) = state.sessions.upstream(&identity, api) else {
        let message = "this session has no model route".to_string();
        record(entry(false, Some(403), Some(message.clone()))).await;
        return error(StatusCode::FORBIDDEN, &message);
    };

    let url = upstream.url(&path, request.uri().query());
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "request body too large"),
    };
    let credential = match upstream
        .credential_headers(method.as_str(), &url, &parts.headers, &body)
        .await
    {
        Ok(credential) => credential,
        Err(message) => {
            record(entry(true, None, Some(message.clone()))).await;
            return error(StatusCode::BAD_GATEWAY, &message);
        }
    };
    let mut outgoing = state.client.request(method, url).body(body);
    for (name, value) in &parts.headers {
        if !DROPPED_HEADERS.contains(&name.as_str()) && name != "anthropic-beta" {
            outgoing = outgoing.header(name, value);
        }
    }
    for (name, value) in credential {
        outgoing = outgoing.header(name, value);
    }

    let answer = match outgoing.send().await {
        Ok(answer) => answer,
        Err(e) => {
            // Without the URL: its query is the CLI's
            let message = e.without_url().to_string();
            record(entry(true, None, Some(message.clone()))).await;
            return error(StatusCode::BAD_GATEWAY, &message);
        }
    };
    record(entry(true, Some(answer.status().as_u16()), None)).await;
    stream_back(answer)
}

/// The upstream's answer, streamed as it arrives (model calls stream SSE)
fn stream_back(answer: reqwest::Response) -> Response {
    let mut response = Response::builder().status(answer.status().as_u16());
    for (name, value) in answer.headers() {
        if !DROPPED_HEADERS.contains(&name.as_str()) {
            response = response.header(name, value);
        }
    }
    response
        .body(Body::from_stream(answer.bytes_stream()))
        .unwrap_or_else(|_| error(StatusCode::BAD_GATEWAY, "unreadable upstream answer"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::http::{header, Request};
    use axum::routing::post;
    use axum::Json;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    /// A fake provider on loopback that echoes what reached it: the
    /// credential headers, the path and the body
    pub(crate) async fn fake_upstream() -> String {
        async fn echo(request: Request<Body>) -> Json<Value> {
            let (parts, body) = request.into_parts();
            let header = |name: &str| {
                parts
                    .headers
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let seen = json!({
                "authorization": header("authorization"),
                "x_api_key": header("x-api-key"),
                "anthropic_beta": header("anthropic-beta"),
                "path": parts.uri.to_string(),
            });
            let body = to_bytes(body, usize::MAX).await.unwrap();
            let mut seen = seen;
            seen["body"] = json!(String::from_utf8_lossy(&body));
            Json(seen)
        }
        let app = Router::new()
            .route("/v1/messages", post(echo))
            .route("/v1/messages/count_tokens", post(echo))
            .route("/foundry/v1/messages", post(echo))
            .route("/model/{model}/invoke-with-response-stream", post(echo))
            .route("/v1/responses", post(echo));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    fn proxy(tokens: &ProxyTokens, sessions: &ModelSessions) -> Router {
        router(ModelProxyState::new(tokens.clone(), sessions.clone()))
    }

    fn messages_call(path: &str, credential: Option<(&str, &str)>) -> Request<Body> {
        let mut request = Request::post(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header("anthropic-beta", "fine-grained-tool-streaming-2025-05-14");
        if let Some((name, value)) = credential {
            request = request.header(name, value);
        }
        request.body(Body::from(r#"{"model":"sonnet"}"#)).unwrap()
    }

    async fn json_of(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn swaps_the_proxy_token_for_the_host_key() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let upstream =
            ModelUpstream::anthropic(&base, UpstreamAuth::ApiKey(Secret::new("sk-host")));
        let _session = sessions.open("writer", upstream);

        let bearer = format!("Bearer {token}");
        let response = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/anthropic/v1/messages?beta=true",
                Some(("authorization", &bearer)),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let seen = json_of(response).await;
        assert_eq!(seen["x_api_key"], "sk-host");
        assert_eq!(seen["authorization"], Value::Null, "the proxy token stays");
        assert_eq!(seen["path"], "/v1/messages?beta=true");
        assert_eq!(seen["body"], r#"{"model":"sonnet"}"#);
    }

    #[tokio::test]
    async fn a_subscription_token_goes_as_a_bearer_with_the_oauth_beta() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let upstream = ModelUpstream::anthropic(
            &base,
            UpstreamAuth::ClaudeSubscription(Secret::new("sk-ant-oat")),
        );
        let _session = sessions.open("writer", upstream);

        // Claude Code with ANTHROPIC_API_KEY sends `x-api-key`
        let response = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/anthropic/v1/messages",
                Some(("x-api-key", &token)),
            ))
            .await
            .unwrap();

        let seen = json_of(response).await;
        assert_eq!(seen["authorization"], "Bearer sk-ant-oat");
        assert_eq!(seen["x_api_key"], Value::Null);
        assert_eq!(
            seen["anthropic_beta"],
            "fine-grained-tool-streaming-2025-05-14,oauth-2025-04-20"
        );
    }

    #[tokio::test]
    async fn keeps_the_upstream_base_path() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let upstream = ModelUpstream::anthropic(
            format!("{base}/foundry/"),
            UpstreamAuth::Bearer(Secret::new("gateway-key")),
        );
        let _session = sessions.open("writer", upstream);

        let response = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/anthropic/v1/messages",
                Some(("x-api-key", &token)),
            ))
            .await
            .unwrap();

        let seen = json_of(response).await;
        assert_eq!(seen["path"], "/foundry/v1/messages");
        assert_eq!(seen["authorization"], "Bearer gateway-key");
    }

    #[tokio::test]
    async fn refuses_missing_wrong_and_revoked_tokens() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let upstream =
            ModelUpstream::anthropic(&base, UpstreamAuth::ApiKey(Secret::new("sk-host")));
        let _session = sessions.open("writer", upstream);
        let call = |credential: Option<(&str, &str)>| {
            proxy(&tokens, &sessions).oneshot(messages_call("/anthropic/v1/messages", credential))
        };

        let missing = call(None).await.unwrap();
        let wrong = call(Some(("x-api-key", "guess"))).await.unwrap();
        tokens.revoke("writer");
        let revoked = call(Some(("x-api-key", &token))).await.unwrap();

        for response in [missing, wrong, revoked] {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(!json_of(response).await.to_string().contains("sk-host"));
        }
    }

    #[tokio::test]
    async fn a_closed_session_has_no_route() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let upstream =
            ModelUpstream::anthropic(&base, UpstreamAuth::ApiKey(Secret::new("sk-host")));
        drop(sessions.open("writer", upstream));

        let response = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/anthropic/v1/messages",
                Some(("x-api-key", &token)),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn forwards_only_model_calls() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let upstream = ModelUpstream::anthropic(&base, UpstreamAuth::Bearer(Secret::new("master")));
        let _session = sessions.open("writer", upstream);

        // LiteLLM's admin routes, for example
        for path in [
            "/anthropic/key/generate",
            "/anthropic/model/new",
            "/anthropic/v1/../key/list",
        ] {
            let response = proxy(&tokens, &sessions)
                .oneshot(messages_call(path, Some(("x-api-key", &token))))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        let counted = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/anthropic/v1/messages/count_tokens",
                Some(("x-api-key", &token)),
            ))
            .await
            .unwrap();
        assert_eq!(counted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bedrock_calls_are_signed_on_the_host() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("writer");
        let keys: crate::services::egress::credentials::SignInFn = Arc::new(|_| {
            Box::pin(async {
                Ok(crate::services::egress::credentials::SignInKeys {
                    access_key: "AKIDHOST".into(),
                    secret_key: "host-secret-key".into(),
                    session_token: Some("host-session".into()),
                    expires_at: None,
                    region: "us-east-1".into(),
                })
            })
        });
        let upstream = ModelUpstream {
            base_url: base,
            ..ModelUpstream::bedrock(
                "us-east-1",
                UpstreamAuth::AwsSigV4(AwsSigner::new("bedrock", "us-east-1", None, keys)),
            )
        };
        let _session = sessions.open("writer", upstream);
        let bearer = format!("Bearer {token}");
        let path = "/bedrock/model/us.anthropic.claude-sonnet-5%3A0/invoke-with-response-stream";

        let response = proxy(&tokens, &sessions)
            .oneshot(messages_call(path, Some(("authorization", &bearer))))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let seen = json_of(response).await;
        let authorization = seen["authorization"].as_str().unwrap();
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDHOST/"));
        assert!(!authorization.contains(&token), "the proxy token stays");
        assert!(!seen.to_string().contains("host-secret-key"));
        assert_eq!(
            seen["path"],
            "/model/us.anthropic.claude-sonnet-5%3A0/invoke-with-response-stream"
        );
    }

    #[tokio::test]
    async fn openai_calls_carry_the_host_key() {
        let base = fake_upstream().await;
        let (tokens, sessions) = (ProxyTokens::default(), ModelSessions::default());
        let token = tokens.issue("coder");
        let upstream = ModelUpstream {
            base_url: base,
            ..ModelUpstream::openai(UpstreamAuth::Bearer(Secret::new("sk-host-openai")))
        };
        let _session = sessions.open("coder", upstream);
        let bearer = format!("Bearer {token}");

        let response = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/openai/v1/responses",
                Some(("authorization", &bearer)),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_of(response).await["authorization"],
            "Bearer sk-host-openai"
        );
        // An Anthropic session's token doesn't open the OpenAI route
        let anthropic =
            ModelUpstream::anthropic("http://unused", UpstreamAuth::ApiKey(Secret::new("k")));
        let _other = sessions.open("coder", anthropic);
        let refused = proxy(&tokens, &sessions)
            .oneshot(messages_call(
                "/openai/v1/responses",
                Some(("authorization", &bearer)),
            ))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn bedrock_forwards_only_model_actions() {
        let allows = |path| ModelApi::Bedrock.allows(path);
        assert!(allows("/model/anthropic.claude-v2/invoke"));
        assert!(allows("/model/us.anthropic.claude%3A0/converse-stream"));
        assert!(!allows("/model/x/invoke/extra"));
        assert!(!allows("/model//invoke"));
        assert!(!allows("/model/../guardrails"));
        assert!(!allows("/guardrail/g/version/1/apply"));
        assert!(!allows("/foundation-models"));
    }

    #[test]
    fn upstream_credentials_never_print() {
        let upstream = ModelUpstream::anthropic(
            ModelUpstream::ANTHROPIC_URL,
            UpstreamAuth::ClaudeSubscription(Secret::new("sk-ant-oat-secret")),
        );
        assert!(!format!("{upstream:?}").contains("sk-ant-oat-secret"));
    }

    #[test]
    fn the_oauth_beta_is_added_once() {
        let value = HeaderValue::from_static("a, oauth-2025-04-20");
        assert_eq!(with_oauth_beta(Some(&value)), "a,oauth-2025-04-20");
        assert_eq!(with_oauth_beta(None), "oauth-2025-04-20");
    }
}
