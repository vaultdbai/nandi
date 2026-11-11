//! The client for Nandi's trial gateway (`trial.vaultdb.ai`, private repo
//! `nandi-cloud/trial-gateway`), which holds the free trial's DeepSeek key
//! and counts each person's tokens. The app holds no key.
//!
//! Every request is signed with this instance's device key
//! (`relay::device_keys`, the same pair the relay pins): five headers,
//! `X-Nandi-Instance`, `X-Nandi-Key`, `X-Nandi-Machine` (this computer's
//! salted hash, `identity::machine_id`), `X-Nandi-Time` (epoch ms) and
//! `X-Nandi-Signature` over [`signing_string`]:
//!
//! ```text
//! nandi-trial-v2\n<METHOD>\n<path>\n<instance>\n<machine>\n<time>\n<hex sha256(body)>
//! ```
//!
//! The body is serialized once, and those exact bytes are hashed, signed
//! and sent. A `stale_time` answer carries the server's clock: the client
//! keeps the difference and signs again once. Times only go up, so two
//! identical calls never share a millisecond (the gateway refuses replays).
//!
//! Errors are `{"error": {"code", "message", ...}}` ([`GatewayError`]);
//! [`GatewayError::user_message`] says what to do about each. The chat path
//! doesn't use the DeepSeek adapter's retries: it waits out a short
//! `Retry-After` (10 seconds or less) once, and reports longer ones.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nandi_seal::DeviceKeys;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use specta::Type;
use tokio::sync::mpsc;

use crate::llm::{deepseek, LlmRequest, LlmResponse};

/// Where the gateway runs unless `NANDI_TRIAL_GATEWAY_URL` says otherwise
pub const DEFAULT_GATEWAY_URL: &str = "https://trial.vaultdb.ai";
/// Points the app at another gateway (staging, a local one in tests)
pub const GATEWAY_URL_ENV: &str = "NANDI_TRIAL_GATEWAY_URL";

pub const INSTANCE_HEADER: &str = "x-nandi-instance";
pub const KEY_HEADER: &str = "x-nandi-key";
pub const MACHINE_HEADER: &str = "x-nandi-machine";
pub const TIME_HEADER: &str = "x-nandi-time";
pub const SIGNATURE_HEADER: &str = "x-nandi-signature";
/// The first line of every signing string
const SIGNING_PREFIX: &str = "nandi-trial-v2";

/// The longest `Retry-After` a trial call waits out before trying again
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

pub const CHAT_PATH: &str = "/v1/chat/completions";
pub const TRIAL_PATH: &str = "/v1/trial";
pub const LINK_PATH: &str = "/v1/link";

/// What to do instead of the trial
pub const OWN_MODEL_HINT: &str = "Add your own key (Anthropic, OpenAI, or DeepSeek) in Settings › Models & runtimes, sign in to Claude Code (`claude`), or run a local model with Ollama.";

/// The gateway's base URL: `override_url` (from `NANDI_TRIAL_GATEWAY_URL`)
/// when set, else [`DEFAULT_GATEWAY_URL`]. No trailing slash.
pub fn gateway_url(override_url: Option<&str>) -> String {
    override_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(DEFAULT_GATEWAY_URL)
        .trim_end_matches('/')
        .to_string()
}

/// Lowercase hex SHA-256 of `body`
fn body_hash(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What the device key signs for one request
pub fn signing_string(
    method: &str,
    path: &str,
    instance: &str,
    machine: &str,
    time: i64,
    body: &[u8],
) -> String {
    let hash = body_hash(body);
    format!("{SIGNING_PREFIX}\n{method}\n{path}\n{instance}\n{machine}\n{time}\n{hash}")
}

/// `GET /v1/trial`: the person's allowance as the gateway counts it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrialAccount {
    /// `google:<sub>` once this computer is linked to an email, else none
    pub account: Option<String>,
    /// The linked email
    #[serde(default)]
    pub email: Option<String>,
    pub used: u32,
    pub limit: u32,
    /// The trial is paused for everyone (its daily cap), until UTC midnight
    #[serde(default)]
    pub paused: bool,
}

/// `POST /v1/link`: the person this computer's trial is now linked to
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrialLink {
    pub account: String,
    pub email: String,
    #[serde(default)]
    pub name: String,
    pub used: u32,
    pub limit: u32,
}

/// The gateway's `{"error": {...}}` and `Retry-After`
#[derive(Debug, Clone, PartialEq)]
pub struct ApiFailure {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after: Option<u64>,
    /// The rest of `error` (`serverTime`, `used`, `left`, ...)
    pub fields: Map<String, Value>,
}

/// Why a gateway call failed
#[derive(Debug, Clone, PartialEq)]
pub enum GatewayError {
    /// No answer at all (offline, DNS, refused, timed out)
    Unreachable(String),
    /// The gateway's `{"error": {...}}`
    Api(Box<ApiFailure>),
    /// An answer that isn't what the contract says, or a stream cut off
    Response(String),
    /// This device's keys can't be read
    Keys(String),
    /// This computer's machine ID can't be read
    Machine(String),
}

impl GatewayError {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api(failure) => Some(&failure.code),
            _ => None,
        }
    }

    pub fn retry_after(&self) -> Option<u64> {
        match self {
            Self::Api(failure) => failure.retry_after,
            _ => None,
        }
    }

    fn field_u64(&self, name: &str) -> Option<u64> {
        match self {
            Self::Api(failure) => failure.fields.get(name).and_then(Value::as_u64),
            _ => None,
        }
    }

    fn server_time(&self) -> Option<i64> {
        match self {
            Self::Api(failure) => failure.fields.get("serverTime").and_then(Value::as_i64),
            _ => None,
        }
    }

    /// A short call can be tried again after `Retry-After`
    fn retryable(&self) -> bool {
        matches!(
            self.code(),
            Some("trial_busy" | "upstream_busy" | "rate_limited")
        )
    }

    /// What the user reads, with what to do next
    pub fn user_message(&self) -> String {
        let (code, message) = match self {
            Self::Unreachable(_) => {
                return "Can't reach the free trial right now. Check this computer's internet connection, or set up your own model in Settings › Models & runtimes.".to_string();
            }
            Self::Response(detail) => {
                return format!("The free trial's answer was cut off ({detail}). Try again.");
            }
            Self::Keys(detail) => {
                return format!(
                    "The free trial needs this device's keys, which can't be read: {detail}"
                );
            }
            Self::Machine(detail) => {
                return format!(
                    "The free trial counts tokens per computer, and this computer's ID can't be read: {detail}"
                );
            }
            Self::Api(failure) => (failure.code.as_str(), failure.message.as_str()),
        };
        let wait = self.retry_after().map(wait_words);
        match code {
            "trial_used_up" => super::TRIAL_USED_UP.to_string(),
            "trial_not_enough" => match self.field_u64("left") {
                Some(left) => format!(
                    "Only {left} free tokens are left, not enough for this conversation. Start a shorter one, or: {OWN_MODEL_HINT}"
                ),
                None => format!("The free tokens left aren't enough for this conversation. Start a shorter one, or: {OWN_MODEL_HINT}"),
            },
            "trial_busy" => "Your free tokens left are in use by your other calls right now. Try again when they finish.".to_string(),
            "trial_paused" => format!(
                "The free trial is paused for today: it has a daily limit shared by everyone. It's back {}. Meanwhile: {OWN_MODEL_HINT}",
                wait.unwrap_or_else(|| "after midnight UTC".to_string())
            ),
            "rate_limited" => format!(
                "Too many free-trial calls from this computer. Try again {}.",
                wait.unwrap_or_else(|| "in a minute".to_string())
            ),
            "too_many_devices" => format!(
                "Too many new computers started the free trial from this network today. Try again tomorrow, or: {OWN_MODEL_HINT}"
            ),
            "stale_time" => "This computer's clock is more than 5 minutes off, so the free trial can't check its calls. Set the clock automatically in your system settings, then try again.".to_string(),
            "key_mismatch" => format!(
                "The free trial knows this Nandi by other keys (its device keys were reset). The trial can't be used from this install: {OWN_MODEL_HINT}"
            ),
            "upstream_busy" => format!(
                "The free trial's model is busy. Try again {}.",
                wait.unwrap_or_else(|| "shortly".to_string())
            ),
            "upstream_error" => format!("The free trial's model refused the request: {message}"),
            "machine_mismatch" => format!(
                "The free trial knows this Nandi from another computer (its data was copied here). The trial can't be used from this install: {OWN_MODEL_HINT}"
            ),
            "already_linked" => "This computer's free trial is linked to another email. A computer stays with the first email linked on it.".to_string(),
            "id_token_invalid" | "id_token_expired" => "The free trial couldn't confirm your connected email. Reconnect it in Settings › Email.".to_string(),
            "email_unverified" => "Your email provider hasn't verified this address, so the free trial can't be linked to it.".to_string(),
            "identity_unavailable" | "link_unavailable" => "The free trial can't link to your email right now. It still works on this computer; it links again later.".to_string(),
            // bad_signature, replayed, model_not_allowed, bad_request,
            // internal: the gateway's sentence says it
            _ => format!("The free trial refused the call ({code}): {message}"),
        }
    }
}

/// "in 40 seconds", "in about 3 hours"
fn wait_words(seconds: u64) -> String {
    match seconds {
        0..=90 => format!("in {seconds} seconds"),
        91..=5_400 => format!("in about {} minutes", seconds.div_ceil(60)),
        _ => format!("in about {} hours", seconds.div_ceil(3_600)),
    }
}

/// Short calls (status, linking): a tighter deadline than model calls
fn control_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("failed to build HTTP client")
    })
}

fn local_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// One instance's signed connection to the gateway
pub struct GatewayClient {
    base_url: String,
    instance: String,
    /// This computer's salted hash (`identity::machine_id`, the trial's)
    machine: String,
    keys: DeviceKeys,
    /// Server clock minus ours, learned from `stale_time`
    offset_ms: AtomicI64,
    /// The last time signed, so each request's time is new
    last_ms: AtomicI64,
}

impl GatewayClient {
    pub fn new(base_url: String, instance: String, machine: String, keys: DeviceKeys) -> Self {
        Self {
            base_url,
            instance,
            machine,
            keys,
            offset_ms: AtomicI64::new(0),
            last_ms: AtomicI64::new(0),
        }
    }

    /// The time to sign: ours plus the learned offset, always after the
    /// last one
    fn next_time(&self) -> i64 {
        let now = local_now_ms() + self.offset_ms.load(Ordering::Relaxed);
        let previous = self
            .last_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
                Some(now.max(last + 1))
            })
            .unwrap_or(now);
        now.max(previous + 1)
    }

    /// The five signing headers for `method path` with `body`
    pub fn headers(&self, method: &str, path: &str, body: &[u8]) -> Vec<(&'static str, String)> {
        let time = self.next_time();
        let message = signing_string(method, path, &self.instance, &self.machine, time, body);
        vec![
            (INSTANCE_HEADER, self.instance.clone()),
            (KEY_HEADER, self.keys.public_keys().signing.clone()),
            (MACHINE_HEADER, self.machine.clone()),
            (TIME_HEADER, time.to_string()),
            (SIGNATURE_HEADER, self.keys.sign(message.as_bytes())),
        ]
    }

    async fn send_once(
        &self,
        http: &reqwest::Client,
        method: reqwest::Method,
        path: &str,
        body: &[u8],
        accept: &str,
    ) -> Result<reqwest::Response, GatewayError> {
        let url = format!("{}{path}", self.base_url);
        let mut request = http
            .request(method.clone(), &url)
            .header(reqwest::header::ACCEPT, accept);
        for (name, value) in self.headers(method.as_str(), path, body) {
            request = request.header(name, value);
        }
        if method != reqwest::Method::GET {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_vec());
        }
        let response = request
            .send()
            .await
            .map_err(|e| GatewayError::Unreachable(e.without_url().to_string()))?;
        if response.status().is_success() {
            return Ok(response);
        }
        Err(read_error(response).await)
    }

    /// Sends a signed request. A `stale_time` answer adjusts the clock by
    /// the server's and signs again, once.
    async fn send(
        &self,
        http: &reqwest::Client,
        method: reqwest::Method,
        path: &str,
        body: &[u8],
        accept: &str,
    ) -> Result<reqwest::Response, GatewayError> {
        match self
            .send_once(http, method.clone(), path, body, accept)
            .await
        {
            Err(error) if error.code() == Some("stale_time") => {
                let Some(server) = error.server_time() else {
                    return Err(error);
                };
                self.offset_ms
                    .store(server - local_now_ms(), Ordering::Relaxed);
                log::warn!("Free trial: this computer's clock is off; adjusted by the gateway's");
                self.send_once(http, method, path, body, accept).await
            }
            other => other,
        }
    }

    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &[u8],
    ) -> Result<T, GatewayError> {
        let response = self
            .send(control_client(), method, path, body, "application/json")
            .await?;
        response
            .json()
            .await
            .map_err(|e| GatewayError::Response(e.without_url().to_string()))
    }

    /// One streamed chat completion on the trial's model. A short
    /// `Retry-After` (busy, rate limited) is waited out once.
    pub async fn chat(
        &self,
        http: &reqwest::Client,
        model: &str,
        request: &LlmRequest,
        max_tokens: u32,
        text: Option<mpsc::Sender<String>>,
    ) -> Result<LlmResponse, GatewayError> {
        // Serialized once: these bytes are signed and sent
        let body = serde_json::to_vec(&deepseek::request_body(model, request, max_tokens))
            .map_err(|e| GatewayError::Response(e.to_string()))?;
        let post = || {
            self.send(
                http,
                reqwest::Method::POST,
                CHAT_PATH,
                &body,
                "text/event-stream",
            )
        };
        let response = match post().await {
            Err(error) if error.retryable() => match error.retry_after() {
                Some(seconds) if Duration::from_secs(seconds) <= MAX_RETRY_AFTER => {
                    log::warn!(
                        "Free trial: {}; trying again in {seconds}s",
                        error.code().unwrap_or_default()
                    );
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                    post().await?
                }
                _ => return Err(error),
            },
            other => other?,
        };
        deepseek::read_stream(response, model, text)
            .await
            .map_err(GatewayError::Response)
    }

    /// `GET /v1/trial`
    pub async fn account(&self) -> Result<TrialAccount, GatewayError> {
        self.json(reqwest::Method::GET, TRIAL_PATH, b"").await
    }

    /// `POST /v1/link` with the connected mailbox's ID token
    pub async fn link(&self, id_token: &str) -> Result<TrialLink, GatewayError> {
        let body = serde_json::to_vec(&serde_json::json!({ "idToken": id_token }))
            .map_err(|e| GatewayError::Response(e.to_string()))?;
        self.json(reqwest::Method::POST, LINK_PATH, &body).await
    }
}

/// The gateway's `{"error": {...}}` and `Retry-After`
async fn read_error(response: reqwest::Response) -> GatewayError {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok());
    let body = response.text().await.unwrap_or_default();
    parse_error(status, retry_after, &body)
}

fn parse_error(status: u16, retry_after: Option<u64>, body: &str) -> GatewayError {
    let error = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| json.get("error").and_then(Value::as_object).cloned());
    let Some(mut fields) = error else {
        // Not the gateway (a proxy page, a captive portal)
        let detail: String = body.chars().take(200).collect();
        return GatewayError::Api(Box::new(ApiFailure {
            status,
            code: "unexpected".to_string(),
            message: format!("HTTP {status}: {detail}"),
            retry_after,
            fields: Map::new(),
        }));
    };
    let code = fields
        .remove("code")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unexpected".to_string());
    let message = fields
        .remove("message")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    GatewayError::Api(Box::new(ApiFailure {
        status,
        code,
        message,
        retry_after,
        fields,
    }))
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
