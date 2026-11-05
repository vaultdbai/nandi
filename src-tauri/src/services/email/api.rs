//! The only way the email feature talks to a mail provider: `GET`, with
//! the sign-in's token, to that provider's API only. There is no other
//! method, so nothing can be sent, moved, deleted or marked read.
//!
//! A 401 renews the token once; 429 and 503 wait for `Retry-After` (a
//! few seconds at most) and try again; anything else is a [`MailError`].

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use super::provider::{MailProvider, ProviderSetup};
use crate::services::auth::oauth::{self, OAuthProvider, TokenError};
use crate::services::secrets::Secrets;

/// Longest wait for `Retry-After` before giving up until the next check
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);
const RETRIES: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    /// The sign-in is gone or refused: the user signs in again
    NeedsSignIn(String),
    /// 410, or Gmail's 404 for history: the cursor can't be used
    CursorExpired,
    NotFound,
    /// Network, throttling, or the provider's own trouble: next check
    Unavailable(String),
    Failed(String),
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsSignIn(reason) => write!(f, "Sign in again: {reason}"),
            Self::CursorExpired => f.write_str("The change cursor expired"),
            Self::NotFound => f.write_str("Not found"),
            Self::Unavailable(reason) | Self::Failed(reason) => f.write_str(reason),
        }
    }
}

impl From<TokenError> for MailError {
    fn from(error: TokenError) -> Self {
        match error {
            TokenError::NeedsSignIn(reason) => Self::NeedsSignIn(reason),
            TokenError::Unavailable(reason) => Self::Unavailable(reason),
        }
    }
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

/// `url` is `base` or below it (`base/…`, `base?…`)
fn is_under(url: &str, base: &str) -> bool {
    url.strip_prefix(base)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('?'))
}

/// One mailbox's read-only API, for one check or one tool call
pub struct MailApi {
    pub provider: MailProvider,
    oauth: OAuthProvider,
    entry: String,
    /// The only addresses the token goes to
    bases: Vec<String>,
    secrets: Secrets,
    token: tokio::sync::Mutex<Option<String>>,
}

impl MailApi {
    pub fn new(provider: MailProvider, setup: &ProviderSetup, secrets: &Secrets) -> Self {
        Self {
            provider,
            oauth: setup.oauth(provider),
            entry: provider.token_entry(),
            bases: setup.api_bases(provider),
            secrets: secrets.clone(),
            token: tokio::sync::Mutex::new(None),
        }
    }

    /// `base` + `path` with query `params`, for [`Self::get`]
    pub fn url(&self, base: &str, path: &[&str], params: &[(&str, &str)]) -> String {
        let mut url = match reqwest::Url::parse(base) {
            Ok(url) => url,
            Err(_) => return String::new(),
        };
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.pop_if_empty().extend(path);
        }
        if !params.is_empty() {
            url.query_pairs_mut().extend_pairs(params);
        }
        url.to_string()
    }

    async fn token(&self, renew: bool) -> Result<String, MailError> {
        let mut cached = self.token.lock().await;
        if !renew {
            if let Some(token) = cached.as_ref() {
                return Ok(token.clone());
            }
        }
        let token = if renew {
            oauth::refresh(&self.entry, &self.oauth, &self.secrets).await?
        } else {
            oauth::fresh_access_token(&self.entry, &self.oauth, &self.secrets).await?
        };
        *cached = Some(token.clone());
        Ok(token)
    }

    /// `GET url` as JSON. `url` must be on the provider's API (a delta or
    /// next link the API returned counts, once checked).
    pub async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Value, MailError> {
        if !self.bases.iter().any(|base| is_under(url, base)) {
            return Err(MailError::Failed(format!(
                "Refusing to send the {} sign-in to another address",
                self.provider.name()
            )));
        }
        let mut renewed = false;
        let mut attempt = 0;
        loop {
            let token = self.token(false).await?;
            let mut request = http_client().get(url).bearer_auth(&token);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            let response = request.send().await.map_err(|e| {
                MailError::Unavailable(format!(
                    "Couldn't reach {}: {}",
                    self.provider.name(),
                    e.without_url()
                ))
            })?;
            let status = response.status().as_u16();
            match status {
                200..=299 => {
                    return response.json().await.map_err(|e| {
                        MailError::Unavailable(format!(
                            "Unexpected answer from {}: {}",
                            self.provider.name(),
                            e.without_url()
                        ))
                    })
                }
                401 if !renewed => {
                    renewed = true;
                    self.token(true).await?;
                }
                401 => {
                    return Err(MailError::NeedsSignIn(format!(
                        "{} refused the sign-in",
                        self.provider.name()
                    )))
                }
                404 => return Err(MailError::NotFound),
                410 => return Err(MailError::CursorExpired),
                429 | 503 if attempt < RETRIES => {
                    attempt += 1;
                    let wait = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map_or(Duration::from_secs(1), Duration::from_secs);
                    if wait > MAX_RETRY_WAIT {
                        return Err(MailError::Unavailable(format!(
                            "{} asked Nandi to wait; it checks again later",
                            self.provider.name()
                        )));
                    }
                    tokio::time::sleep(wait).await;
                }
                403 => {
                    return Err(MailError::Failed(format!(
                        "{} didn't allow reading this mailbox (403)",
                        self.provider.name()
                    )))
                }
                _ => {
                    return Err(MailError::Unavailable(format!(
                        "{} answered {status}",
                        self.provider.name()
                    )))
                }
            }
        }
    }
}
