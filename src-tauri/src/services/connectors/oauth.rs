//! OAuth2 for connectors: authorization code + PKCE with a one-shot
//! localhost redirect server, falling back to the device flow (RFC 8628)
//! for providers that support it. Tokens go to the OS keyring
//! (`nandi.connector.<id>.oauth`) and never cross a transport.
//!
//! Both flows fit the frozen `start_oauth` / `poll_oauth` pair:
//!
//! - **PKCE**: `start_oauth` returns a `DeviceFlowState` whose
//!   `verificationUri` is the provider's sign-in page and whose `userCode`
//!   is empty. The browser comes back to `http://127.0.0.1:<port>/callback`;
//!   the code is redeemed there. `poll_oauth` reports when that's done.
//! - **Device**: the usual user code + verification URL; each `poll_oauth`
//!   asks the token endpoint once (no faster than `interval`).
//!
//! The one provider (`gmail`) is an auth-only stub: the API calls land in
//! a later wave. Client IDs are baked in at build time (`NANDI_GOOGLE_CLIENT_ID`)
//! or set per connector under `config.oauth` in `.nandi/connectors.yaml`
//! (public values only). The app-wide GitHub sign-in (`services::auth::github`)
//! runs on the same flows with its own keyring entry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use futures_util::future::BoxFuture;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::AbortHandle;

use super::catalog::config_list;
use super::{Connector, ConnectorDecl, DeviceFlowState, ToolDef};
use crate::services::secrets::Secrets;

/// How long a started sign-in stays valid
const FLOW_TTL: Duration = Duration::from_secs(10 * 60);
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_INTERVAL: u64 = 5;
const MAX_REQUEST_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct OAuthProvider {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Shown in the Library but not installable yet ("Coming soon")
    pub coming_soon: bool,
    /// Authorization endpoint (PKCE); `None` if the provider has none we can use
    pub authorize_url: Option<String>,
    pub token_url: String,
    /// Device authorization endpoint, for the device flow
    pub device_url: Option<String>,
    pub client_id: Option<String>,
    /// For "installed app" clients that need one (Google). Not a real secret
    /// for desktop apps, but still never written to the repo files.
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    /// Extra authorization parameters (e.g. Google's `access_type=offline`)
    pub auth_params: Vec<(String, String)>,
}

impl OAuthProvider {
    /// `pkce` | `device`: the flow `start_oauth` tries first
    pub fn flow_kind(&self) -> &'static str {
        if self.authorize_url.is_some() {
            "pkce"
        } else {
            "device"
        }
    }
}

fn build_env(value: Option<&'static str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The shipped providers (auth only for now).
pub fn providers() -> Vec<OAuthProvider> {
    vec![OAuthProvider {
        id: "gmail".into(),
        name: "Gmail".into(),
        description: "Sign in with Google to give agents Gmail access.".into(),
        coming_soon: true,
        authorize_url: Some("https://accounts.google.com/o/oauth2/v2/auth".into()),
        token_url: "https://oauth2.googleapis.com/token".into(),
        // Google's device flow doesn't allow Gmail scopes
        device_url: None,
        client_id: build_env(option_env!("NANDI_GOOGLE_CLIENT_ID")),
        client_secret: build_env(option_env!("NANDI_GOOGLE_CLIENT_SECRET")),
        scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".into()],
        auth_params: vec![
            ("access_type".into(), "offline".into()),
            ("prompt".into(), "consent".into()),
        ],
    }]
}

/// The provider for an OAuth2 declaration (`kind: gmail`), with
/// any `config.oauth` overrides:
///
/// ```yaml
/// config:
///   oauth: {client_id: abc.apps.googleusercontent.com, scopes: [...]}
/// ```
pub fn provider_for(decl: &ConnectorDecl) -> Result<OAuthProvider, String> {
    let mut provider = providers()
        .into_iter()
        .find(|p| p.id == decl.kind)
        .ok_or_else(|| format!("Connector '{}' doesn't use OAuth2", decl.id))?;
    if let Some(serde_yaml::Value::Mapping(overrides)) = decl.config.get("oauth") {
        let get = |key: &str| {
            overrides
                .get(key)
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_string)
        };
        if let Some(url) = get("authorize_url") {
            provider.authorize_url = Some(url);
        }
        if let Some(url) = get("token_url") {
            provider.token_url = url;
        }
        if let Some(url) = get("device_url") {
            provider.device_url = Some(url);
        }
        if let Some(id) = get("client_id") {
            provider.client_id = Some(id);
        }
        let mut nested = decl.clone();
        nested.config = overrides
            .iter()
            .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
            .collect();
        let scopes = config_list(&nested, "scopes");
        if !scopes.is_empty() {
            provider.scopes = scopes;
        }
    }
    Ok(provider)
}

/// Keyring entry for a connector's tokens.
pub fn token_entry(connector_id: &str) -> String {
    format!(
        "nandi.connector.{}.oauth",
        connector_id.to_ascii_lowercase()
    )
}

/// No `Debug`: holds tokens.
#[derive(Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    /// Unix seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_type: Option<String>,
}

fn load_token(entry: &str, secrets: &Secrets) -> Option<StoredToken> {
    let raw = secrets.get(entry).ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

pub fn has_token(connector_id: &str, secrets: &Secrets) -> bool {
    stored_access_token(&token_entry(connector_id), secrets).is_some()
}

/// The stored access token, for the connector's API calls (later waves).
pub fn access_token(connector_id: &str, secrets: &Secrets) -> Option<String> {
    stored_access_token(&token_entry(connector_id), secrets)
}

/// Seconds before `expires_at` a token counts as expired already, so it
/// doesn't run out mid-call
const EXPIRY_MARGIN_SECS: u64 = 60;

/// The access token a sign-in stored at keyring `entry`, unless it has
/// expired: a provider rejects it anyway, and treating it as signed out
/// lets a fallback (the GitHub CLI's sign-in) and "Sign in" take over
pub fn stored_access_token(entry: &str, secrets: &Secrets) -> Option<String> {
    let now = unix_now();
    load_token(entry, secrets)
        .filter(|t| t.expires_at.is_none_or(|at| now + EXPIRY_MARGIN_SECS < at))
        .map(|t| t.access_token)
        .filter(|t| !t.is_empty())
}

fn store_token(entry: &str, token: &StoredToken, secrets: &Secrets) -> Result<(), String> {
    let json = serde_json::to_string(token).map_err(|e| e.to_string())?;
    secrets.set(entry, &json)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// RFC 7636 S256: BASE64URL(SHA256(verifier))
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("nandi/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client")
    })
}

/// A token endpoint's answer. No `Debug`: may hold tokens.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

enum TokenPoll {
    Token(StoredToken),
    Pending,
    SlowDown(u64),
}

async fn post_token(url: &str, form: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let response = http_client()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .map_err(|e| format!("Could not reach the sign-in server: {}", e.without_url()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Unexpected response from the sign-in server ({status}): {e}"))?;
    // Don't echo the body in errors: it may contain a token
    serde_json::from_str(&body)
        .map_err(|_| format!("Unexpected response from the sign-in server ({status})"))
}

fn interpret(response: TokenResponse) -> Result<TokenPoll, String> {
    if let Some(access_token) = response.access_token.filter(|t| !t.is_empty()) {
        return Ok(TokenPoll::Token(StoredToken {
            access_token,
            refresh_token: response.refresh_token,
            expires_at: response.expires_in.map(|s| unix_now() + s),
            token_type: response.token_type,
        }));
    }
    match response.error.as_deref() {
        Some("authorization_pending") => Ok(TokenPoll::Pending),
        Some("slow_down") => Ok(TokenPoll::SlowDown(
            response.interval.unwrap_or(DEFAULT_INTERVAL + 5),
        )),
        Some("expired_token") => Err("The sign-in code expired. Please start again.".into()),
        Some("access_denied") => Err("Sign-in was cancelled.".into()),
        Some(error) => Err(response
            .error_description
            .unwrap_or_else(|| error.to_string())),
        None => Err("The sign-in server didn't return a token".into()),
    }
}

// ---------------------------------------------------------------------------
// Flows in progress
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Pending,
    Done,
    Failed(String),
}

struct PkceFlow {
    outcome: Arc<Mutex<Outcome>>,
    server: AbortHandle,
    expires_at: u64,
}

impl Drop for PkceFlow {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[derive(Clone)]
struct DeviceFlow {
    token_url: String,
    client_id: String,
    device_code: String,
    interval: Duration,
    next_poll: Instant,
    expires_at: u64,
}

enum Flow {
    Pkce(PkceFlow),
    Device(DeviceFlow),
}

/// Sign-ins in progress, one per connector.
#[derive(Default)]
pub struct OAuthFlows(Mutex<HashMap<String, Flow>>);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl OAuthFlows {
    /// Starts (or restarts) the sign-in whose token goes to keyring `entry`
    /// (`token_entry(entry)` for a connector): PKCE when the provider
    /// has an authorization endpoint, else (or if the redirect server can't
    /// start) the device flow.
    pub async fn start(
        &self,
        entry: &str,
        provider: OAuthProvider,
        secrets: Secrets,
    ) -> Result<DeviceFlowState, String> {
        let client_id = provider.client_id.clone().ok_or_else(|| {
            format!(
                "{} sign-in isn't configured: no client_id is set",
                provider.name
            )
        })?;
        lock(&self.0).remove(entry);
        if provider.authorize_url.is_some() {
            match start_pkce(entry, &provider, &client_id, secrets).await {
                Ok((flow, state)) => {
                    lock(&self.0).insert(entry.to_string(), Flow::Pkce(flow));
                    return Ok(state);
                }
                Err(e) if provider.device_url.is_some() => {
                    log::warn!(
                        "PKCE sign-in unavailable for {}, using the device flow: {e}",
                        provider.name
                    );
                }
                Err(e) => return Err(e),
            }
        }
        if provider.device_url.is_none() {
            return Err(format!(
                "{} has no sign-in flow Nandi supports",
                provider.name
            ));
        }
        let (flow, state) = start_device(&provider, &client_id).await?;
        lock(&self.0).insert(entry.to_string(), Flow::Device(flow));
        log::info!("Started the device sign-in for {}", provider.name);
        Ok(state)
    }

    /// Checks once; `true` once the token is in the keyring.
    pub async fn poll(&self, entry: &str, secrets: &Secrets) -> Result<bool, String> {
        let device = {
            let mut flows = lock(&self.0);
            match flows.get(entry) {
                None if stored_access_token(entry, secrets).is_some() => return Ok(true),
                None => return Err("No sign-in is in progress; start again".into()),
                Some(Flow::Pkce(flow)) => {
                    let outcome = lock(&flow.outcome).clone();
                    let expired = unix_now() > flow.expires_at;
                    return match outcome {
                        Outcome::Done => {
                            flows.remove(entry);
                            Ok(true)
                        }
                        Outcome::Failed(error) => {
                            flows.remove(entry);
                            Err(error)
                        }
                        Outcome::Pending if expired => {
                            flows.remove(entry);
                            Err("The sign-in expired. Please start again.".into())
                        }
                        Outcome::Pending => Ok(false),
                    };
                }
                Some(Flow::Device(flow)) => {
                    if unix_now() > flow.expires_at {
                        flows.remove(entry);
                        return Err("The sign-in code expired. Please start again.".into());
                    }
                    if Instant::now() < flow.next_poll {
                        return Ok(false);
                    }
                    flow.clone()
                }
            }
        };

        let response = post_token(
            &device.token_url,
            &[
                ("client_id", device.client_id.as_str()),
                ("device_code", device.device_code.as_str()),
                ("grant_type", DEVICE_GRANT_TYPE),
            ],
        )
        .await;
        let result = response.and_then(interpret);
        let mut flows = lock(&self.0);
        match result {
            Ok(TokenPoll::Token(token)) => {
                flows.remove(entry);
                drop(flows);
                store_token(entry, &token, secrets)?;
                log::info!("Signed in ({entry})");
                Ok(true)
            }
            Ok(poll) => {
                if let Some(Flow::Device(flow)) = flows.get_mut(entry) {
                    if let TokenPoll::SlowDown(interval) = poll {
                        flow.interval = Duration::from_secs(interval.max(1));
                    }
                    flow.next_poll = Instant::now() + flow.interval;
                }
                Ok(false)
            }
            Err(e) => {
                flows.remove(entry);
                Err(e)
            }
        }
    }
}

async fn start_device(
    provider: &OAuthProvider,
    client_id: &str,
) -> Result<(DeviceFlow, DeviceFlowState), String> {
    #[derive(Deserialize)]
    struct DeviceCode {
        device_code: Option<String>,
        user_code: Option<String>,
        verification_uri: Option<String>,
        /// Google's name for it
        verification_url: Option<String>,
        expires_in: Option<u64>,
        interval: Option<u64>,
        error: Option<String>,
        error_description: Option<String>,
    }
    let url = provider.device_url.as_deref().unwrap_or_default();
    let scope = provider.scopes.join(" ");
    let response = http_client()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[("client_id", client_id), ("scope", scope.as_str())])
        .send()
        .await
        .map_err(|e| format!("Could not reach {}: {}", provider.name, e.without_url()))?;
    let body: DeviceCode = response
        .json()
        .await
        .map_err(|e| format!("Unexpected response from {}: {e}", provider.name))?;
    if let Some(error) = body.error {
        return Err(body.error_description.unwrap_or(error));
    }
    let (Some(device_code), Some(user_code), Some(verification_uri)) = (
        body.device_code,
        body.user_code,
        body.verification_uri.or(body.verification_url),
    ) else {
        return Err(format!(
            "Incomplete device code response from {}",
            provider.name
        ));
    };
    let interval = body.interval.unwrap_or(DEFAULT_INTERVAL).max(1);
    let expires_at = unix_now() + body.expires_in.unwrap_or(FLOW_TTL.as_secs());
    let flow = DeviceFlow {
        token_url: provider.token_url.clone(),
        client_id: client_id.to_string(),
        device_code: device_code.clone(),
        interval: Duration::from_secs(interval),
        next_poll: Instant::now() + Duration::from_secs(interval),
        expires_at,
    };
    let state = DeviceFlowState {
        device_code,
        user_code,
        verification_uri,
        interval: u32::try_from(interval).unwrap_or(u32::MAX),
        expires_at: u32::try_from(expires_at).unwrap_or(u32::MAX),
    };
    Ok((flow, state))
}

/// What the redirect server needs to redeem the code. No `Debug`.
struct Exchange {
    entry: String,
    token_url: String,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    verifier: String,
    state: String,
    secrets: Secrets,
}

impl Exchange {
    async fn redeem(&self, code: &str) -> Result<(), String> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("client_id", self.client_id.as_str()),
            ("code_verifier", self.verifier.as_str()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret.as_str()));
        }
        match interpret(post_token(&self.token_url, &form).await?)? {
            TokenPoll::Token(token) => store_token(&self.entry, &token, &self.secrets),
            _ => Err("The sign-in server didn't return a token".into()),
        }
    }
}

async fn start_pkce(
    entry: &str,
    provider: &OAuthProvider,
    client_id: &str,
    secrets: Secrets,
) -> Result<(PkceFlow, DeviceFlowState), String> {
    let authorize_url = provider.authorize_url.as_deref().unwrap_or_default();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Couldn't start the sign-in redirect server: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let verifier = random_token(32);
    let state = random_token(16);
    let scope = provider.scopes.join(" ");
    let challenge = code_challenge(&verifier);
    let mut params: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", &redirect_uri),
        ("scope", &scope),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ];
    params.extend(
        provider
            .auth_params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    );
    let url = reqwest::Url::parse_with_params(authorize_url, &params)
        .map_err(|e| format!("Invalid authorization URL for {}: {e}", provider.name))?;

    let outcome = Arc::new(Mutex::new(Outcome::Pending));
    let exchange = Exchange {
        entry: entry.to_string(),
        token_url: provider.token_url.clone(),
        client_id: client_id.to_string(),
        client_secret: provider.client_secret.clone(),
        redirect_uri,
        verifier,
        state,
        secrets,
    };
    let task_outcome = outcome.clone();
    let server = tokio::spawn(async move {
        if tokio::time::timeout(FLOW_TTL, serve_redirect(listener, exchange, &task_outcome))
            .await
            .is_err()
        {
            log::info!("A connector sign-in expired before the browser came back");
        }
    })
    .abort_handle();

    let expires_at = unix_now() + FLOW_TTL.as_secs();
    let flow = PkceFlow {
        outcome,
        server,
        expires_at,
    };
    let state = DeviceFlowState {
        // Opaque: the browser round trip carries the real state
        device_code: uuid::Uuid::new_v4().to_string(),
        user_code: String::new(),
        verification_uri: url.to_string(),
        interval: 1,
        expires_at: u32::try_from(expires_at).unwrap_or(u32::MAX),
    };
    Ok((flow, state))
}

/// Answers browser requests until the matching callback arrives.
async fn serve_redirect(listener: TcpListener, exchange: Exchange, outcome: &Mutex<Outcome>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let Some(target) = read_request_target(&mut stream).await else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, 400, "Bad request").await;
            continue;
        };
        if url.path() != "/callback" {
            respond(&mut stream, 404, "Not found").await;
            continue;
        }
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        // Anything without our state is not our browser round trip
        if query.get("state") != Some(&exchange.state) {
            respond(
                &mut stream,
                400,
                "This sign-in link doesn't match. Start again from Nandi.",
            )
            .await;
            continue;
        }
        if let Some(error) = query.get("error") {
            *lock(outcome) = Outcome::Failed(format!("Sign-in was refused: {error}"));
            respond(
                &mut stream,
                200,
                "Sign-in was cancelled. You can close this window.",
            )
            .await;
            return;
        }
        let Some(code) = query.get("code") else {
            respond(&mut stream, 400, "The sign-in response had no code.").await;
            continue;
        };
        match exchange.redeem(code).await {
            Ok(()) => {
                *lock(outcome) = Outcome::Done;
                log::info!("Signed in ({})", exchange.entry);
                respond(
                    &mut stream,
                    200,
                    "Signed in to Nandi. You can close this window.",
                )
                .await;
            }
            Err(e) => {
                *lock(outcome) = Outcome::Failed(e);
                respond(
                    &mut stream,
                    200,
                    "Sign-in failed. Return to Nandi to try again.",
                )
                .await;
            }
        }
        return;
    }
}

/// The request target of `GET <target> HTTP/1.1`.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < MAX_REQUEST_BYTES {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.lines().next()?.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET" && target.starts_with('/')).then(|| target.to_string())
}

async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Nandi</title>\
         <body style=\"font-family:system-ui;padding:3rem\"><p>{message}</p></body>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

// ---------------------------------------------------------------------------
// The connector side: auth only for now
// ---------------------------------------------------------------------------

/// An OAuth2 connector (`gmail`). It has no tools yet; it's
/// healthy once signed in.
pub struct OAuthConnector {
    connector_id: String,
    provider: OAuthProvider,
    secrets: Secrets,
}

impl OAuthConnector {
    pub fn new(connector_id: &str, provider: OAuthProvider, secrets: Secrets) -> Self {
        Self {
            connector_id: connector_id.to_string(),
            provider,
            secrets,
        }
    }
}

impl Connector for OAuthConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        Vec::new()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        _args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            Err(format!(
                "{} tools aren't available yet ('{name}')",
                self.provider.name
            ))
        })
    }

    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if has_token(&self.connector_id, &self.secrets) {
                Ok(())
            } else {
                Err(format!("Not signed in to {}", self.provider.name))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::routing::post;
    use axum::{Form, Json, Router};
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    type Captured = Arc<Mutex<Vec<HashMap<String, String>>>>;

    async fn serve(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn mock_provider(base: &str, pkce: bool) -> OAuthProvider {
        OAuthProvider {
            id: "mock".into(),
            name: "Mock".into(),
            description: String::new(),
            coming_soon: false,
            authorize_url: pkce.then(|| "https://auth.example/authorize".into()),
            token_url: format!("{base}/token"),
            device_url: (!pkce).then(|| format!("{base}/device")),
            client_id: Some("client-1".into()),
            client_secret: None,
            scopes: vec!["mail.read".into(), "profile".into()],
            auth_params: vec![("access_type".into(), "offline".into())],
        }
    }

    #[tokio::test]
    async fn pkce_flow_redeems_the_code_with_the_verifier() {
        let captured: Captured = Arc::default();
        let router = Router::new()
            .route(
                "/token",
                post(
                    |State(captured): State<Captured>,
                     Form(form): Form<HashMap<String, String>>| async move {
                        lock(&captured).push(form);
                        Json(json!({
                            "access_token": "tok-1",
                            "refresh_token": "ref-1",
                            "expires_in": 3600,
                            "token_type": "Bearer",
                        }))
                    },
                ),
            )
            .with_state(captured.clone());
        let base = serve(router).await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();

        let started = flows
            .start(
                &token_entry("mail"),
                mock_provider(&base, true),
                secrets.clone(),
            )
            .await
            .unwrap();
        assert!(started.user_code.is_empty(), "PKCE has no user code");
        let auth = reqwest::Url::parse(&started.verification_uri).unwrap();
        let params: HashMap<String, String> = auth.query_pairs().into_owned().collect();
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["scope"], "mail.read profile");
        assert_eq!(params["access_type"], "offline");
        let redirect = params["redirect_uri"].clone();
        assert!(redirect.starts_with("http://127.0.0.1:"));
        assert!(!flows.poll(&token_entry("mail"), &secrets).await.unwrap());

        // A callback without our state is refused and doesn't end the flow
        let forged = reqwest::get(format!("{redirect}?code=evil&state=forged"))
            .await
            .unwrap();
        assert_eq!(forged.status(), 400);
        assert!(!flows.poll(&token_entry("mail"), &secrets).await.unwrap());

        let page = reqwest::get(format!("{redirect}?code=abc&state={}", params["state"]))
            .await
            .unwrap();
        assert!(page.text().await.unwrap().contains("Signed in"));
        assert!(flows.poll(&token_entry("mail"), &secrets).await.unwrap());

        let forms = lock(&captured).clone();
        assert_eq!(forms.len(), 1, "only the real code is redeemed");
        let form = &forms[0];
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "abc");
        assert_eq!(form["redirect_uri"], redirect);
        assert_eq!(
            code_challenge(&form["code_verifier"]),
            params["code_challenge"]
        );
        assert_eq!(access_token("mail", &secrets).as_deref(), Some("tok-1"));
        let raw = secrets.get(&token_entry("mail")).unwrap().unwrap();
        assert!(raw.contains("ref-1"), "refresh token kept in the keyring");
    }

    #[tokio::test]
    async fn pkce_flow_reports_a_refusal() {
        let base = serve(Router::new()).await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();
        let started = flows
            .start(
                &token_entry("mail"),
                mock_provider(&base, true),
                secrets.clone(),
            )
            .await
            .unwrap();
        let auth = reqwest::Url::parse(&started.verification_uri).unwrap();
        let params: HashMap<String, String> = auth.query_pairs().into_owned().collect();
        reqwest::get(format!(
            "{}?error=access_denied&state={}",
            params["redirect_uri"], params["state"]
        ))
        .await
        .unwrap();
        let error = flows
            .poll(&token_entry("mail"), &secrets)
            .await
            .unwrap_err();
        assert!(error.contains("access_denied"));
        assert!(!has_token("mail", &secrets));
    }

    #[tokio::test]
    async fn device_flow_polls_until_authorized() {
        let calls = Arc::new(AtomicU32::new(0));
        let router = Router::new()
            .route(
                "/device",
                post(|| async {
                    Json(json!({
                        "device_code": "dev-1",
                        "user_code": "ABCD-1234",
                        "verification_uri": "https://example.com/device",
                        "expires_in": 600,
                        "interval": 1,
                    }))
                }),
            )
            .route(
                "/token",
                post(
                    |State(calls): State<Arc<AtomicU32>>,
                     Form(form): Form<HashMap<String, String>>| async move {
                        assert_eq!(form["device_code"], "dev-1");
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            Json(json!({"error": "authorization_pending"}))
                        } else {
                            Json(json!({"access_token": "gh-tok"}))
                        }
                    },
                ),
            )
            .with_state(calls.clone());
        let base = serve(router).await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();

        let started = flows
            .start(
                &token_entry("gh"),
                mock_provider(&base, false),
                secrets.clone(),
            )
            .await
            .unwrap();
        assert_eq!(started.user_code, "ABCD-1234");
        // Too soon: answered locally, without hitting the server
        assert!(!flows.poll(&token_entry("gh"), &secrets).await.unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!flows.poll(&token_entry("gh"), &secrets).await.unwrap());
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(flows.poll(&token_entry("gh"), &secrets).await.unwrap());
        assert_eq!(access_token("gh", &secrets).as_deref(), Some("gh-tok"));
        // Done: later polls see the stored token
        assert!(flows.poll(&token_entry("gh"), &secrets).await.unwrap());
    }

    #[tokio::test]
    async fn needs_a_client_id() {
        let mut provider = mock_provider("http://127.0.0.1:1", true);
        provider.client_id = None;
        let error = OAuthFlows::default()
            .start(&token_entry("mail"), provider, Secrets::in_memory())
            .await
            .unwrap_err();
        assert!(error.contains("client_id"));
    }

    #[test]
    fn config_overrides_the_provider() {
        let manifest = crate::services::domain::ConnectorManifest::from_yaml(
            "connectors:\n  - id: mail\n    kind: gmail\n    config:\n      oauth:\n        client_id: my-client\n        scopes: [a, b]\n",
        )
        .unwrap();
        let provider = provider_for(manifest.get("mail").unwrap()).unwrap();
        assert_eq!(provider.client_id.as_deref(), Some("my-client"));
        assert_eq!(provider.scopes, ["a", "b"]);
        assert_eq!(provider.flow_kind(), "pkce");
    }

    #[test]
    fn challenge_matches_rfc_7636_example() {
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn an_expired_sign_in_counts_as_signed_out() {
        let secrets = Secrets::in_memory();
        let stored = |expires_at: Option<u64>| {
            store_token(
                "nandi.test.token",
                &StoredToken {
                    access_token: "t".into(),
                    refresh_token: Some("r".into()),
                    expires_at,
                    token_type: None,
                },
                &secrets,
            )
            .unwrap();
            stored_access_token("nandi.test.token", &secrets)
        };
        assert_eq!(stored(None).as_deref(), Some("t"));
        assert_eq!(stored(Some(unix_now() + 3600)).as_deref(), Some("t"));
        assert_eq!(stored(Some(unix_now() - 1)), None);
        // About to run out: as good as expired
        assert_eq!(stored(Some(unix_now() + 10)), None);
    }

    #[tokio::test]
    async fn the_connector_is_healthy_once_signed_in() {
        let secrets = Secrets::in_memory();
        let connector =
            OAuthConnector::new("gh", mock_provider("http://x", false), secrets.clone());
        assert!(connector.list_tools().is_empty());
        assert!(connector.health_check().await.is_err());
        store_token(
            &token_entry("gh"),
            &StoredToken {
                access_token: "t".into(),
                refresh_token: None,
                expires_at: None,
                token_type: None,
            },
            &secrets,
        )
        .unwrap();
        connector.health_check().await.unwrap();
    }
}
