//! Joining a relay: what "Join" in onboarding and Network › Connection does,
//! and the invite a member copies for a teammate.
//!
//! [`join_relay`] asks the relay's `/health` first (reachable, a Nandi
//! relay, the same protocol version), then:
//!
//! | The relay                          | What joining does                                              |
//! | ---------------------------------- | -------------------------------------------------------------- |
//! | open (`auth: open`)                | Saves the URL and connects; no join code needed                |
//! | secured, URL carries `?token=`     | Saves it (the operator issued this instance's token)           |
//! | secured, a token kept for it       | Reuses it (joined before, then disconnected)                   |
//! | secured, a join code               | Enrolls once (`POST /relay/enroll`), saves the token, connects |
//! | secured, no code                   | [`JoinOutcome::CodeNeeded`]                                    |
//! | public (`public: true`)            | [`JoinOutcome::PublicRelay`]: join it with the connected email |
//!
//! The join code is the one the user typed, or the build's own
//! ([`super::BuiltInRelay`]) for the build's relay. A typed code is kept in
//! the OS keychain ([`JOIN_CODE_ENTRY`]) with its relay, only so "Copy
//! invite message" can pass it on. Neither the code nor the token is ever
//! logged, returned to the UI after joining, or shown.
//!
//! Joining is the only place an install enrolls. Nothing enrolls at
//! startup, so an install that never joins takes no seat.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::client::split_token;
use super::device_keys::DeviceKeyring;
use super::protocol::PROTOCOL_VERSION;
use super::{instances_endpoint, saved_token_for, set_relay_url, validate_url, with_token};
use crate::services::network;
use crate::services::AppState;
use nandi_protocol::{EnrollRequest, EnrollResponse, PublicKeys, RelayHealth};

/// Keychain entry holding the join code the user typed, with its relay
/// (`{"relay", "code"}`)
pub const JOIN_CODE_ENTRY: &str = "nandi.relay.join_code";

/// How long the relay's `/health` may take to answer
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

/// How long enrolling may take
const ENROLL_TIMEOUT: Duration = Duration::from_secs(10);

/// What joining a relay came to. Everything but `Joined` leaves the
/// relay settings as they were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JoinOutcome {
    /// Saved and connecting; `relay` is the URL without any token
    Joined { relay: String },
    /// Not a `ws://` or `wss://` URL
    InvalidUrl,
    /// Nothing answered at that address, or not as a Nandi relay
    Unreachable { detail: String },
    /// The relay speaks another protocol version: update Nandi or the relay
    VersionMismatch { relay: u32, app: u32 },
    /// The relay is secured, and no join code was given
    CodeNeeded,
    /// The relay didn't accept the join code
    WrongCode,
    /// The relay has no free seat; `message` is the relay's own words
    RelayFull { message: String },
    /// This Nandi's ID enrolled there before and has lost its token
    AlreadyEnrolled { message: String },
    /// The relay's operator removed this Nandi
    Removed { message: String },
    /// The relay doesn't take enrollments (no join code set up there)
    NoEnrollment,
    /// A public relay: Nandis join it with their connected email, not with
    /// a code ("Join the public relay", [`super::public`])
    PublicRelay,
    /// Any other refusal, in the relay's words
    Refused { message: String },
}

/// What "Copy invite message" passes on: the relay without this instance's
/// token, and the join code when the user typed one for this relay
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayInvite {
    pub url: String,
    pub join_code: Option<String>,
}

/// The join code kept in the keychain, and the relay it is for
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KeptJoinCode {
    relay: String,
    code: String,
}

/// Joins the relay at `url` (see the module docs). `join_code` is what the
/// user typed (blank: none). `Err` only when saving fails locally.
pub async fn join_relay(
    state: &AppState,
    url: &str,
    join_code: Option<&str>,
) -> Result<JoinOutcome, String> {
    let url = url.trim();
    if validate_url(url).is_err() {
        return Ok(JoinOutcome::InvalidUrl);
    }
    let (base, url_token) = split_token(url);
    let typed = join_code.map(str::trim).filter(|code| !code.is_empty());
    let health = match health(&base).await {
        Ok(health) => health,
        Err(outcome) => return Ok(outcome),
    };
    if health.relay.protocol_version != PROTOCOL_VERSION {
        return Ok(JoinOutcome::VersionMismatch {
            relay: health.relay.protocol_version,
            app: PROTOCOL_VERSION,
        });
    }
    if health.public {
        return Ok(JoinOutcome::PublicRelay);
    }
    let health = health.relay;
    let token = if health.auth == "open" {
        None
    } else if let Some(token) = url_token {
        Some(token)
    } else if let Some(token) = saved_token_for(state.db(), &base)? {
        Some(token)
    } else {
        let built_in = state.built_in_relay();
        let code = typed.or_else(|| {
            built_in
                .join_code()
                .filter(|_| built_in.url() == Some(base.as_str()))
        });
        let Some(code) = code else {
            return Ok(JoinOutcome::CodeNeeded);
        };
        let instance_id = network::local_instance_id(state.db())?;
        // First relay use: this device's keys, which the relay pins
        let keys = DeviceKeyring::new(state.secrets().clone())
            .public_keys()
            .map_err(|e| log::warn!("Enrolling without this device's keys: {e}"))
            .ok();
        match enroll(&base, &instance_id, code, keys.as_ref()).await {
            Ok(token) => Some(token),
            Err(refused) => return Ok(refused.into()),
        }
    };
    set_relay_url(state, &with_token(&base, token.as_deref())).await?;
    if let Some(code) = typed.filter(|_| health.auth != "open") {
        keep_join_code(state, &base, code);
    }
    log::info!("Joined the relay {base}");
    Ok(JoinOutcome::Joined { relay: base })
}

/// The invite for the relay in use (`None`: none), for "Copy invite
/// message". Never carries this instance's token.
pub fn relay_invite(state: &AppState) -> Result<Option<RelayInvite>, String> {
    let Some(url) = super::configured_url(state) else {
        return Ok(None);
    };
    let join_code = state
        .secrets()
        .get(JOIN_CODE_ENTRY)?
        .and_then(|kept| serde_json::from_str::<KeptJoinCode>(&kept).ok())
        .filter(|kept| kept.relay == url)
        .map(|kept| kept.code);
    Ok(Some(RelayInvite { url, join_code }))
}

fn keep_join_code(state: &AppState, relay: &str, code: &str) {
    let kept = KeptJoinCode {
        relay: relay.to_string(),
        code: code.to_string(),
    };
    let saved = serde_json::to_string(&kept)
        .map_err(|e| e.to_string())
        .and_then(|json| state.secrets().set(JOIN_CODE_ENTRY, &json));
    if let Err(e) = saved {
        // Joining worked; only the invite goes without the code
        log::warn!("The join code wasn't kept for invites: {e}");
    }
}

/// A relay's `GET /health`: the protocol's fields, and whether it is a
/// public relay and whose ID tokens it accepts
#[derive(Debug, Clone, Deserialize)]
pub(super) struct RelayHealthReport {
    #[serde(flatten)]
    pub relay: RelayHealth,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub identity: Vec<String>,
}

/// `GET <relay host>/health`, or why it isn't a relay we can join
pub(super) async fn health(base: &str) -> Result<RelayHealthReport, JoinOutcome> {
    let mut endpoint = instances_endpoint(base).map_err(|_| JoinOutcome::InvalidUrl)?;
    endpoint.set_path("/health");
    let unreachable = |detail: String| JoinOutcome::Unreachable { detail };
    let response = reqwest::Client::new()
        .get(endpoint.as_str())
        .timeout(HEALTH_TIMEOUT)
        .send()
        .await
        .map_err(|e| unreachable(format!("Couldn't reach {endpoint}: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(unreachable(format!("{endpoint} answered {status}")));
    }
    response
        .json::<RelayHealthReport>()
        .await
        .map_err(|_| unreachable(format!("{endpoint} isn't a Nandi relay")))
}

/// Why enrolling didn't give a token
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EnrollError {
    /// The relay couldn't be reached, or failed on its side
    Unreachable(String),
    /// The relay answered no: its HTTP status and its reason
    Refused { status: u16, message: String },
}

impl std::fmt::Display for EnrollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) => f.write_str(e),
            Self::Refused { status, message } => write!(f, "{status}: {message}"),
        }
    }
}

impl From<EnrollError> for JoinOutcome {
    fn from(error: EnrollError) -> Self {
        match error {
            EnrollError::Unreachable(detail) => Self::Unreachable { detail },
            EnrollError::Refused { status, message } => match status {
                401 => Self::WrongCode,
                402 => Self::RelayFull { message },
                403 => Self::Removed { message },
                404 => Self::NoEnrollment,
                409 => Self::AlreadyEnrolled { message },
                _ => Self::Refused { message },
            },
        }
    }
}

/// `POST <relay>/enroll {"instanceId", "joinCode", "publicKeys"}` → the
/// instance's token
pub(super) async fn enroll(
    relay_url: &str,
    instance_id: &str,
    code: &str,
    public_keys: Option<&PublicKeys>,
) -> Result<String, EnrollError> {
    let endpoint = enroll_endpoint(relay_url).map_err(EnrollError::Unreachable)?;
    let response = reqwest::Client::new()
        .post(endpoint.as_str())
        .timeout(ENROLL_TIMEOUT)
        .json(&EnrollRequest {
            instance_id: instance_id.to_string(),
            join_code: code.to_string(),
            public_keys: public_keys.cloned(),
        })
        .send()
        .await
        .map_err(|e| EnrollError::Unreachable(format!("Couldn't reach {endpoint}: {e}")))?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    let reason = || {
        body["error"]
            .as_str()
            .unwrap_or("no reason given")
            .to_string()
    };
    if status.is_server_error() {
        return Err(EnrollError::Unreachable(format!(
            "{endpoint} answered {status}: {}",
            reason()
        )));
    }
    if !status.is_success() {
        return Err(EnrollError::Refused {
            status: status.as_u16(),
            message: reason(),
        });
    }
    serde_json::from_value::<EnrollResponse>(body)
        .ok()
        .map(|enrolled| enrolled.token)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| EnrollError::Unreachable(format!("{endpoint} sent no token")))
}

/// `wss://host/relay` → `https://host/relay/enroll` (any path prefix kept)
pub(super) fn enroll_endpoint(relay_url: &str) -> Result<reqwest::Url, String> {
    let mut url = instances_endpoint(relay_url)?;
    let (base, _) = split_token(relay_url);
    let path = reqwest::Url::parse(&base)
        .map(|u| u.path().trim_end_matches('/').to_string())
        .unwrap_or_default();
    url.set_path(&format!("{path}/enroll"));
    Ok(url)
}

#[cfg(test)]
#[path = "join_tests.rs"]
mod tests;
