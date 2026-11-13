//! OAuth2 sign-in, shared by every feature that signs in to a provider
//! (the GitHub sign-in, email): authorization code + PKCE with a one-shot
//! loopback redirect server, falling back to the device flow (RFC 8628)
//! for providers that support it, and token refresh. Tokens go to the OS
//! keyring under the entry the caller names and never cross a transport.
//!
//! Both flows fit one `start` / `poll` pair:
//!
//! - **PKCE**: `start` returns a `DeviceFlowState` whose `verificationUri`
//!   is the provider's sign-in page and whose `userCode` is empty. The
//!   browser comes back to the provider's loopback redirect
//!   (`http://127.0.0.1:<port>/callback`, or `http://localhost:<port>` for
//!   Microsoft); the code is redeemed there. `poll` reports when that's done.
//! - **Device**: the usual user code + verification URL; each `poll`
//!   asks the token endpoint once (no faster than `interval`).
//!
//! [`fresh_access_token`] hands a feature a usable access token: the
//! stored one while it's valid, else one renewed with the stored refresh
//! token. When the provider refuses the refresh, the answer is
//! [`TokenError::NeedsSignIn`] and the feature asks the user to sign in
//! again.
//!
//! A sign-in that asked for `openid` also keeps the provider's OpenID
//! Connect ID token; [`fresh_id_token`] hands out one that's still valid
//! (renewing the sign-in when it isn't), for proving who the person is to
//! VaultDB.ai's services (`services::identity`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::AbortHandle;

use crate::services::secrets::Secrets;

/// How long a started sign-in stays valid
const FLOW_TTL: Duration = Duration::from_secs(10 * 60);
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_INTERVAL: u64 = 5;
const MAX_REQUEST_BYTES: usize = 8 * 1024;

/// Codes for an in-progress sign-in (device flow or PKCE), shown to the
/// user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFlowState {
    /// Opaque code passed back to the poll
    pub device_code: String,
    /// Short code the user types at `verification_uri`; empty for PKCE
    pub user_code: String,
    pub verification_uri: String,
    /// Minimum seconds between polls
    pub interval: u32,
    /// Unix timestamp (seconds) after which the codes are invalid
    pub expires_at: u32,
}

/// Where the browser comes back to after a PKCE sign-in, as the provider
/// has it registered. Nandi always listens on 127.0.0.1 (and `::1` for
/// `localhost`), on a port picked at sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopbackRedirect {
    pub host: &'static str,
    pub path: &'static str,
}

impl LoopbackRedirect {
    /// `http://127.0.0.1:<port>/callback` (Google, most providers)
    pub const CALLBACK: Self = Self {
        host: "127.0.0.1",
        path: "/callback",
    };
    /// `http://localhost:<port>` (Microsoft Entra's `http://localhost`
    /// registration, which matches any port)
    pub const LOCALHOST: Self = Self {
        host: "localhost",
        path: "/",
    };

    fn uri(self, port: u16) -> String {
        match self.path {
            "/" => format!("http://{}:{port}", self.host),
            path => format!("http://{}:{port}{path}", self.host),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OAuthProvider {
    pub id: String,
    pub name: String,
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
    pub redirect: LoopbackRedirect,
}

/// A value baked in at build time (`option_env!`), if set and not blank
pub fn build_env(value: Option<&'static str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
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
    /// The OpenID Connect ID token, when the sign-in asked for `openid`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id_token: Option<String>,
}

impl StoredToken {
    fn is_fresh(&self, now: u64) -> bool {
        !self.access_token.is_empty()
            && self
                .expires_at
                .is_none_or(|at| now + EXPIRY_MARGIN_SECS < at)
    }
}

fn load_token(entry: &str, secrets: &Secrets) -> Option<StoredToken> {
    let raw = secrets.get(entry).ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

/// Whether a sign-in stored anything at keyring `entry` (a token that may
/// still be refreshed counts)
#[cfg(test)]
pub fn has_sign_in(entry: &str, secrets: &Secrets) -> bool {
    load_token(entry, secrets).is_some()
}

/// Seconds before `expires_at` a token counts as expired already, so it
/// doesn't run out mid-call
const EXPIRY_MARGIN_SECS: u64 = 60;

/// The access token a sign-in stored at keyring `entry`, unless it has
/// expired: a provider rejects it anyway, and treating it as signed out
/// lets a fallback (the GitHub CLI's sign-in) and "Sign in" take over
pub fn stored_access_token(entry: &str, secrets: &Secrets) -> Option<String> {
    load_token(entry, secrets)
        .filter(|t| t.is_fresh(unix_now()))
        .map(|t| t.access_token)
}

/// The claims of an ID token (its middle part). Read without checking the
/// signature: the token came straight from the provider's token endpoint
/// over TLS (OpenID Connect Core 3.1.3.7); the services it's sent to check
/// it fully.
pub fn id_token_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The ID token the sign-in at keyring `entry` keeps, valid or not
pub fn stored_id_token(entry: &str, secrets: &Secrets) -> Option<String> {
    load_token(entry, secrets)?
        .id_token
        .filter(|token| !token.is_empty())
}

/// Seconds an ID token must still be valid to be handed out
const ID_TOKEN_MARGIN_SECS: u64 = 300;

fn id_token_is_fresh(token: &str, now: u64) -> bool {
    id_token_claims(token)
        .and_then(|claims| claims.get("exp").and_then(serde_json::Value::as_u64))
        .is_some_and(|exp| now + ID_TOKEN_MARGIN_SECS < exp)
}

/// An ID token for the sign-in at keyring `entry` that's valid for a few
/// minutes more: the kept one, else one from renewing the sign-in
pub async fn fresh_id_token(
    entry: &str,
    provider: &OAuthProvider,
    secrets: &Secrets,
) -> Result<String, TokenError> {
    if let Some(token) = stored_id_token(entry, secrets) {
        if id_token_is_fresh(&token, unix_now()) {
            return Ok(token);
        }
    }
    refresh(entry, provider, secrets).await?;
    stored_id_token(entry, secrets)
        .filter(|token| id_token_is_fresh(token, unix_now()))
        .ok_or_else(|| {
            TokenError::NeedsSignIn(format!(
                "The {} sign-in gave no ID token; connect it again",
                provider.name
            ))
        })
}

/// Deletes the sign-in at keyring `entry` (signing out)
pub fn forget(entry: &str, secrets: &Secrets) -> Result<(), String> {
    secrets.delete(entry)
}

fn store_token(entry: &str, token: &StoredToken, secrets: &Secrets) -> Result<(), String> {
    let json = serde_json::to_string(token).map_err(|e| e.to_string())?;
    secrets.set(entry, &json).map_err(|e| {
        log::warn!("Saving the sign-in ({entry}) failed: {e}");
        format!("Signed in, but couldn't save your sign-in ({e})")
    })
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
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

enum TokenPoll {
    Token(StoredToken),
    Pending,
    SlowDown(u64),
}

/// `Err` only when the server couldn't be reached or didn't answer in
/// JSON; OAuth errors are in the response
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

/// `error: description`, or whichever of the two is there
fn oauth_error(error: &str, description: Option<&str>) -> String {
    match description.map(str::trim).filter(|d| !d.is_empty()) {
        Some(description) => format!("{error}: {description}"),
        None => error.to_string(),
    }
}

fn interpret(response: TokenResponse) -> Result<TokenPoll, String> {
    if let Some(access_token) = response.access_token.filter(|t| !t.is_empty()) {
        return Ok(TokenPoll::Token(StoredToken {
            access_token,
            refresh_token: response.refresh_token,
            expires_at: response.expires_in.map(|s| unix_now() + s),
            token_type: response.token_type,
            id_token: response.id_token.filter(|t| !t.is_empty()),
        }));
    }
    match response.error.as_deref() {
        Some("authorization_pending") => Ok(TokenPoll::Pending),
        Some("slow_down") => Ok(TokenPoll::SlowDown(
            response.interval.unwrap_or(DEFAULT_INTERVAL + 5),
        )),
        Some("expired_token") => Err("The sign-in code expired. Please start again.".into()),
        Some("access_denied") => Err("Sign-in was cancelled.".into()),
        Some(error) => Err(oauth_error(error, response.error_description.as_deref())),
        None => Err("The sign-in server didn't return a token".into()),
    }
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

/// Why there's no usable access token
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// Never signed in, or the provider refused the refresh token (revoked,
    /// expired: Google's Testing mode ends sign-ins after 7 days). Only a
    /// new sign-in helps.
    NeedsSignIn(String),
    /// The sign-in server couldn't be reached; try again later
    Unavailable(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsSignIn(reason) | Self::Unavailable(reason) => f.write_str(reason),
        }
    }
}

/// One refresh at a time: providers that rotate refresh tokens (Microsoft)
/// would otherwise see the same one redeemed twice
fn refresh_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// A usable access token for the sign-in at keyring `entry`: the stored
/// one while it's valid, else a refreshed one (stored again).
pub async fn fresh_access_token(
    entry: &str,
    provider: &OAuthProvider,
    secrets: &Secrets,
) -> Result<String, TokenError> {
    match load_token(entry, secrets) {
        None => Err(TokenError::NeedsSignIn("Not signed in".into())),
        Some(token) if token.is_fresh(unix_now()) => Ok(token.access_token),
        Some(_) => refresh(entry, provider, secrets).await,
    }
}

/// Renews the access token with the stored refresh token, even if the
/// access token looks valid (after the API answered 401). A rotated refresh
/// token replaces the old one; without one, the old one is kept.
pub async fn refresh(
    entry: &str,
    provider: &OAuthProvider,
    secrets: &Secrets,
) -> Result<String, TokenError> {
    let _one_at_a_time = refresh_lock().lock().await;
    let stored = load_token(entry, secrets)
        .ok_or_else(|| TokenError::NeedsSignIn("Not signed in".into()))?;
    let refresh_token = stored
        .refresh_token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| TokenError::NeedsSignIn("The sign-in can't be renewed".into()))?;
    let client_id = provider.client_id.clone().ok_or_else(|| {
        TokenError::NeedsSignIn(format!("{} sign-in isn't set up", provider.name))
    })?;
    let scope = provider.scopes.join(" ");
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", client_id.as_str()),
        ("scope", scope.as_str()),
    ];
    if let Some(secret) = &provider.client_secret {
        form.push(("client_secret", secret.as_str()));
    }
    let response = post_token(&provider.token_url, &form)
        .await
        .map_err(TokenError::Unavailable)?;
    match interpret(response) {
        Ok(TokenPoll::Token(mut token)) => {
            if token.refresh_token.is_none() {
                token.refresh_token = Some(refresh_token);
            }
            if token.id_token.is_none() {
                token.id_token = stored.id_token;
            }
            store_token(entry, &token, secrets).map_err(TokenError::Unavailable)?;
            log::info!("Renewed the {} sign-in", provider.name);
            Ok(token.access_token)
        }
        Ok(_) => Err(TokenError::Unavailable(
            "The sign-in server didn't return a token".into(),
        )),
        Err(reason) => {
            log::warn!("The {} sign-in can't be renewed: {reason}", provider.name);
            Err(TokenError::NeedsSignIn(reason))
        }
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

/// Sign-ins in progress, one per keyring entry.
#[derive(Default)]
pub struct OAuthFlows(Mutex<HashMap<String, Flow>>);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl OAuthFlows {
    /// Starts (or restarts) the sign-in whose token goes to keyring `entry`:
    /// PKCE when the provider has an authorization endpoint, else (or if
    /// the redirect server can't start) the device flow.
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

    /// Drops the sign-in in progress for `entry`, if any
    pub fn cancel(&self, entry: &str) {
        lock(&self.0).remove(entry);
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
        return Err(oauth_error(&error, body.error_description.as_deref()));
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
    redirect_path: &'static str,
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

/// The loopback listeners: 127.0.0.1, plus `::1` on the same port for a
/// `localhost` redirect (a browser may resolve it to either)
struct Listeners {
    v4: TcpListener,
    v6: Option<TcpListener>,
}

impl Listeners {
    async fn bind(redirect: LoopbackRedirect) -> Result<(Self, u16), String> {
        let v4 = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Couldn't start the sign-in redirect server: {e}"))?;
        let port = v4.local_addr().map_err(|e| e.to_string())?.port();
        let v6 = if redirect.host == "localhost" {
            TcpListener::bind(("::1", port)).await.ok()
        } else {
            None
        };
        Ok((Self { v4, v6 }, port))
    }

    async fn accept(&self) -> std::io::Result<TcpStream> {
        match &self.v6 {
            Some(v6) => tokio::select! {
                accepted = self.v4.accept() => accepted.map(|(stream, _)| stream),
                accepted = v6.accept() => accepted.map(|(stream, _)| stream),
            },
            None => self.v4.accept().await.map(|(stream, _)| stream),
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
    let (listeners, port) = Listeners::bind(provider.redirect).await?;
    let redirect_uri = provider.redirect.uri(port);
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
        redirect_path: provider.redirect.path,
        verifier,
        state,
        secrets,
    };
    let task_outcome = outcome.clone();
    let server = tokio::spawn(async move {
        if tokio::time::timeout(FLOW_TTL, serve_redirect(listeners, exchange, &task_outcome))
            .await
            .is_err()
        {
            log::info!("A sign-in expired before the browser came back");
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
async fn serve_redirect(listeners: Listeners, exchange: Exchange, outcome: &Mutex<Outcome>) {
    loop {
        let Ok(mut stream) = listeners.accept().await else {
            continue;
        };
        let Some(target) = read_request_target(&mut stream).await else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, 400, "Bad request").await;
            continue;
        };
        if url.path() != exchange.redirect_path {
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
            let reason = oauth_error(error, query.get("error_description").map(String::as_str));
            log::warn!("Sign-in ({}) was refused: {reason}", exchange.entry);
            *lock(outcome) = Outcome::Failed(format!("Sign-in was refused: {reason}"));
            respond(
                &mut stream,
                200,
                "Sign-in didn't finish. Return to Nandi to see why.",
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
                log::warn!("Sign-in ({}) failed: {e}", exchange.entry);
                respond(
                    &mut stream,
                    200,
                    &format!(
                        "Sign-in failed: {}. Return to Nandi to try again.",
                        e.trim_end_matches('.')
                    ),
                )
                .await;
                *lock(outcome) = Outcome::Failed(e);
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

/// Text for the redirect page, with `<`, `>`, `&` and quotes escaped
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let message = html_escape(message);
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::routing::post;
    use axum::{Form, Json, Router};
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    type Captured = Arc<Mutex<Vec<HashMap<String, String>>>>;

    const ENTRY: &str = "nandi.test.oauth";

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
            authorize_url: pkce.then(|| "https://auth.example/authorize".into()),
            token_url: format!("{base}/token"),
            device_url: (!pkce).then(|| format!("{base}/device")),
            client_id: Some("client-1".into()),
            client_secret: None,
            scopes: vec!["mail.read".into(), "profile".into()],
            auth_params: vec![("access_type".into(), "offline".into())],
            redirect: LoopbackRedirect::CALLBACK,
        }
    }

    fn token_router(captured: Captured, answer: serde_json::Value) -> Router {
        Router::new()
            .route(
                "/token",
                post(
                    move |State(captured): State<Captured>,
                          Form(form): Form<HashMap<String, String>>| {
                        let answer = answer.clone();
                        async move {
                            lock(&captured).push(form);
                            Json(answer)
                        }
                    },
                ),
            )
            .with_state(captured)
    }

    fn stored(secrets: &Secrets, access: &str, refresh: Option<&str>, expires_at: Option<u64>) {
        store_token(
            ENTRY,
            &StoredToken {
                access_token: access.into(),
                refresh_token: refresh.map(str::to_string),
                expires_at,
                token_type: None,
                id_token: None,
            },
            secrets,
        )
        .unwrap();
    }

    /// An unsigned ID token expiring at `exp`
    fn id_token(exp: u64) -> String {
        let claims = URL_SAFE_NO_PAD.encode(json!({"sub": "1", "exp": exp}).to_string());
        format!("eyJhbGciOiJSUzI1NiJ9.{claims}.c2ln")
    }

    #[tokio::test]
    async fn a_fresh_id_token_is_kept_and_an_old_one_renewed() {
        let captured: Captured = Arc::default();
        let renewed = id_token(unix_now() + 3600);
        let base = serve(token_router(
            captured.clone(),
            json!({"access_token": "new", "expires_in": 3600, "id_token": renewed}),
        ))
        .await;
        let secrets = Secrets::in_memory();
        // No ID token kept at all, and the refresh brings one
        stored(&secrets, "old", Some("ref-1"), Some(unix_now() + 3600));
        let provider = mock_provider(&base, true);
        assert_eq!(
            fresh_id_token(ENTRY, &provider, &secrets).await.unwrap(),
            renewed
        );
        assert_eq!(lock(&captured).len(), 1);
        // Kept and valid: no call
        assert_eq!(
            fresh_id_token(ENTRY, &provider, &secrets).await.unwrap(),
            renewed
        );
        assert_eq!(lock(&captured).len(), 1);
        assert_eq!(id_token_claims(&renewed).unwrap()["sub"], json!("1"));
        assert!(!id_token_is_fresh(&id_token(unix_now() + 60), unix_now()));
    }

    #[tokio::test]
    async fn a_renewal_without_an_id_token_needs_a_new_sign_in() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(
            captured,
            json!({"access_token": "new", "expires_in": 3600}),
        ))
        .await;
        let secrets = Secrets::in_memory();
        stored(&secrets, "old", Some("ref-1"), Some(unix_now() + 3600));
        let error = fresh_id_token(ENTRY, &mock_provider(&base, true), &secrets)
            .await
            .unwrap_err();
        assert!(matches!(error, TokenError::NeedsSignIn(ref r) if r.contains("ID token")));
    }

    #[tokio::test]
    async fn pkce_flow_redeems_the_code_with_the_verifier() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(
            captured.clone(),
            json!({
                "access_token": "tok-1",
                "refresh_token": "ref-1",
                "expires_in": 3600,
                "token_type": "Bearer",
            }),
        ))
        .await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();

        let started = flows
            .start(ENTRY, mock_provider(&base, true), secrets.clone())
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
        assert!(redirect.ends_with("/callback"));
        assert!(!flows.poll(ENTRY, &secrets).await.unwrap());

        // A callback without our state is refused and doesn't end the flow
        let forged = reqwest::get(format!("{redirect}?code=evil&state=forged"))
            .await
            .unwrap();
        assert_eq!(forged.status(), 400);
        assert!(!flows.poll(ENTRY, &secrets).await.unwrap());

        let page = reqwest::get(format!("{redirect}?code=abc&state={}", params["state"]))
            .await
            .unwrap();
        assert!(page.text().await.unwrap().contains("Signed in"));
        assert!(flows.poll(ENTRY, &secrets).await.unwrap());

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
        assert_eq!(
            stored_access_token(ENTRY, &secrets).as_deref(),
            Some("tok-1")
        );
        let raw = secrets.get(ENTRY).unwrap().unwrap();
        assert!(raw.contains("ref-1"), "refresh token kept in the keyring");
    }

    #[tokio::test]
    async fn a_localhost_redirect_has_no_path_and_answers_on_the_root() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(captured, json!({"access_token": "t"}))).await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();
        let mut provider = mock_provider(&base, true);
        provider.redirect = LoopbackRedirect::LOCALHOST;
        let started = flows.start(ENTRY, provider, secrets.clone()).await.unwrap();
        let auth = reqwest::Url::parse(&started.verification_uri).unwrap();
        let params: HashMap<String, String> = auth.query_pairs().into_owned().collect();
        let redirect = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("localhost"));
        assert_eq!(params["redirect_uri"].matches('/').count(), 2, "no path");
        let port = redirect.port().unwrap();
        reqwest::get(format!(
            "http://127.0.0.1:{port}/?code=abc&state={}",
            params["state"]
        ))
        .await
        .unwrap();
        assert!(flows.poll(ENTRY, &secrets).await.unwrap());
    }

    /// Starts a PKCE sign-in against `base` and comes back with `code`;
    /// returns the browser page
    async fn sign_in_through_the_browser(
        flows: &OAuthFlows,
        base: &str,
        secrets: &Secrets,
    ) -> String {
        let started = flows
            .start(ENTRY, mock_provider(base, true), secrets.clone())
            .await
            .unwrap();
        let auth = reqwest::Url::parse(&started.verification_uri).unwrap();
        let params: HashMap<String, String> = auth.query_pairs().into_owned().collect();
        reqwest::get(format!(
            "{}?code=abc&state={}",
            params["redirect_uri"], params["state"]
        ))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_large_microsoft_sign_in_is_stored_within_the_windows_limit() {
        // A personal Microsoft account's tokens: several KB, over the
        // 2,560-byte credential limit (the in-memory store enforces it)
        let captured: Captured = Arc::default();
        let access = "EwB4A8l6BAAU".repeat(250);
        let refresh = "M.C556_BAY.".repeat(120);
        let base = serve(token_router(
            captured,
            json!({"access_token": access, "refresh_token": refresh, "expires_in": 3600, "token_type": "Bearer"}),
        ))
        .await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();
        let page = sign_in_through_the_browser(&flows, &base, &secrets).await;
        assert!(page.contains("Signed in"), "{page}");
        assert!(flows.poll(ENTRY, &secrets).await.unwrap());
        assert_eq!(
            stored_access_token(ENTRY, &secrets).as_deref(),
            Some(access.as_str())
        );
        assert!(secrets.get(ENTRY).unwrap().unwrap().contains(&refresh));
    }

    #[tokio::test]
    async fn a_token_endpoint_error_is_shown_on_the_page_and_to_the_app() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(
            captured,
            json!({
                "error": "invalid_client",
                "error_description": "AADSTS7000218: The request body must contain the following parameter: 'client_assertion' or 'client_secret'.",
            }),
        ))
        .await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();
        let page = sign_in_through_the_browser(&flows, &base, &secrets).await;
        assert!(
            page.contains("Sign-in failed: invalid_client: AADSTS7000218"),
            "{page}"
        );
        // Escaped for the page
        assert!(page.contains("&#39;") || page.contains("'client_secret'"));
        let error = flows.poll(ENTRY, &secrets).await.unwrap_err();
        assert!(error.contains("AADSTS7000218"), "{error}");
        assert!(!has_sign_in(ENTRY, &secrets));
    }

    #[test]
    fn the_page_escapes_html() {
        assert_eq!(
            html_escape("<b>\"x\" & y</b>"),
            "&lt;b&gt;&quot;x&quot; &amp; y&lt;/b&gt;"
        );
    }

    #[tokio::test]
    async fn pkce_flow_reports_a_refusal_with_its_description() {
        let base = serve(Router::new()).await;
        let secrets = Secrets::in_memory();
        let flows = OAuthFlows::default();
        let started = flows
            .start(ENTRY, mock_provider(&base, true), secrets.clone())
            .await
            .unwrap();
        let auth = reqwest::Url::parse(&started.verification_uri).unwrap();
        let params: HashMap<String, String> = auth.query_pairs().into_owned().collect();
        reqwest::get(format!(
            "{}?error=access_denied&error_description=AADSTS65001%20needs%20consent&state={}",
            params["redirect_uri"], params["state"]
        ))
        .await
        .unwrap();
        let error = flows.poll(ENTRY, &secrets).await.unwrap_err();
        assert!(error.contains("access_denied"));
        assert!(error.contains("AADSTS65001"));
        assert!(!has_sign_in(ENTRY, &secrets));
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
            .start(ENTRY, mock_provider(&base, false), secrets.clone())
            .await
            .unwrap();
        assert_eq!(started.user_code, "ABCD-1234");
        // Too soon: answered locally, without hitting the server
        assert!(!flows.poll(ENTRY, &secrets).await.unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!flows.poll(ENTRY, &secrets).await.unwrap());
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(flows.poll(ENTRY, &secrets).await.unwrap());
        assert_eq!(
            stored_access_token(ENTRY, &secrets).as_deref(),
            Some("gh-tok")
        );
        // Done: later polls see the stored token
        assert!(flows.poll(ENTRY, &secrets).await.unwrap());
    }

    #[tokio::test]
    async fn needs_a_client_id() {
        let mut provider = mock_provider("http://127.0.0.1:1", true);
        provider.client_id = None;
        let error = OAuthFlows::default()
            .start(ENTRY, provider, Secrets::in_memory())
            .await
            .unwrap_err();
        assert!(error.contains("client_id"));
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
        let check = |expires_at: Option<u64>| {
            stored(&secrets, "t", Some("r"), expires_at);
            stored_access_token(ENTRY, &secrets)
        };
        assert_eq!(check(None).as_deref(), Some("t"));
        assert_eq!(check(Some(unix_now() + 3600)).as_deref(), Some("t"));
        assert_eq!(check(Some(unix_now() - 1)), None);
        // About to run out: as good as expired
        assert_eq!(check(Some(unix_now() + 10)), None);
        // Still a sign-in: it can be renewed
        assert!(has_sign_in(ENTRY, &secrets));
    }

    #[tokio::test]
    async fn a_fresh_token_is_used_without_asking_the_server() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(captured.clone(), json!({"access_token": "x"}))).await;
        let secrets = Secrets::in_memory();
        stored(&secrets, "live", Some("r"), Some(unix_now() + 3600));
        let token = fresh_access_token(ENTRY, &mock_provider(&base, true), &secrets)
            .await
            .unwrap();
        assert_eq!(token, "live");
        assert!(lock(&captured).is_empty());
    }

    #[tokio::test]
    async fn an_expired_token_is_refreshed_and_stored() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(
            captured.clone(),
            json!({"access_token": "new", "expires_in": 3600}),
        ))
        .await;
        let secrets = Secrets::in_memory();
        stored(&secrets, "old", Some("ref-1"), Some(unix_now() - 5));
        let mut provider = mock_provider(&base, true);
        provider.client_secret = Some("desktop-secret".into());
        let token = fresh_access_token(ENTRY, &provider, &secrets)
            .await
            .unwrap();
        assert_eq!(token, "new");
        let forms = lock(&captured).clone();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0]["grant_type"], "refresh_token");
        assert_eq!(forms[0]["refresh_token"], "ref-1");
        assert_eq!(forms[0]["client_secret"], "desktop-secret");
        assert_eq!(stored_access_token(ENTRY, &secrets).as_deref(), Some("new"));
        // No new refresh token: the old one is kept
        assert!(secrets.get(ENTRY).unwrap().unwrap().contains("ref-1"));
    }

    #[tokio::test]
    async fn a_rotated_refresh_token_replaces_the_old_one() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(
            captured,
            json!({"access_token": "new", "refresh_token": "ref-2", "expires_in": 60}),
        ))
        .await;
        let secrets = Secrets::in_memory();
        stored(&secrets, "old", Some("ref-1"), Some(unix_now() + 3600));
        // Forced (the API answered 401), though the token looked valid
        refresh(ENTRY, &mock_provider(&base, true), &secrets)
            .await
            .unwrap();
        let raw = secrets.get(ENTRY).unwrap().unwrap();
        assert!(raw.contains("ref-2") && !raw.contains("ref-1"));
    }

    #[tokio::test]
    async fn a_refused_refresh_needs_a_new_sign_in() {
        let captured: Captured = Arc::default();
        let base = serve(token_router(
            captured,
            json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."}),
        ))
        .await;
        let secrets = Secrets::in_memory();
        stored(&secrets, "old", Some("ref-1"), Some(unix_now() - 5));
        let error = fresh_access_token(ENTRY, &mock_provider(&base, true), &secrets)
            .await
            .unwrap_err();
        assert!(matches!(error, TokenError::NeedsSignIn(ref r) if r.contains("invalid_grant")));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_not_a_sign_out() {
        let secrets = Secrets::in_memory();
        stored(&secrets, "old", Some("ref-1"), Some(unix_now() - 5));
        let error = fresh_access_token(ENTRY, &mock_provider("http://127.0.0.1:1", true), &secrets)
            .await
            .unwrap_err();
        assert!(matches!(error, TokenError::Unavailable(_)));
        assert!(has_sign_in(ENTRY, &secrets), "the sign-in is kept");
    }

    #[tokio::test]
    async fn no_sign_in_or_no_refresh_token_needs_a_sign_in() {
        let secrets = Secrets::in_memory();
        let provider = mock_provider("http://127.0.0.1:1", true);
        assert!(matches!(
            fresh_access_token(ENTRY, &provider, &secrets).await,
            Err(TokenError::NeedsSignIn(_))
        ));
        stored(&secrets, "old", None, Some(unix_now() - 5));
        assert!(matches!(
            fresh_access_token(ENTRY, &provider, &secrets).await,
            Err(TokenError::NeedsSignIn(_))
        ));
        forget(ENTRY, &secrets).unwrap();
        assert!(!has_sign_in(ENTRY, &secrets));
    }
}
