//! The public relay: VaultDB.ai's relay for people without a company relay
//! ("Join the public relay" in onboarding and Network › Connection).
//!
//! People join it by signing in with GitHub, Google or Microsoft (the
//! relay runs the OAuth device flow), not with a join code. One account,
//! several Nandis; the relay pins each Nandi's device keys at its first
//! sign-in, as `/relay/enroll` does.
//!
//! | Step | Here | At the relay |
//! | --- | --- | --- |
//! | Offer | [`offer`] | `GET /health`: `public`, `signin` (the providers) |
//! | Start | [`start_sign_in`] | `POST /relay/signin/{provider}/start` with this instance's ID and public keys → a user code and a page |
//! | Poll | [`poll_sign_in`] | `POST /relay/signin/{provider}/poll` every `interval` s → `pending`, `slow_down` or `linked` |
//! | Linked | the token is kept exactly like an enrolled one, and the app connects | |
//! | Account | [`account`] | `GET /relay/account?instanceId=` with the token as bearer |
//!
//! Which relay: [`super::BuiltInRelay::public_url`], `NANDI_PUBLIC_RELAY_URL`
//! when set (staging, tests), else [`PUBLIC_RELAY_URL`]. Nothing here runs
//! at startup or blocks onboarding: an unreachable relay is a calm
//! [`PublicRelayOffer::Unreachable`], and the rest of Nandi works the same.
//!
//! What the app keeps: the token in `relay_state` (as enrolling does) and,
//! next to it, the account it was linked to ([`RELAY_ACCOUNT_KEY`]), so the
//! Connection view can name it while the relay is away and relay trust
//! knows not to trust everyone on a public relay
//! ([`on_public_relay`]). Never the token in the UI or the logs.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::client::split_token;
use super::device_keys::DeviceKeyring;
use super::join::{health, RelayHealthReport};
use super::protocol::PROTOCOL_VERSION;
use super::{
    configured_url, instances_endpoint, read_state, saved_token_for, set_relay_url, with_token,
    write_state, RELAY_TOKEN_RELAY_KEY,
};
use crate::db::Db;
use crate::model::Timestamp;
use crate::services::network;
use crate::services::AppState;

/// VaultDB.ai's public relay
pub const PUBLIC_RELAY_URL: &str = "wss://relay.vaultdb.ai/relay";

/// Points the app at another public relay (staging, tests)
pub const PUBLIC_RELAY_URL_ENV: &str = "NANDI_PUBLIC_RELAY_URL";

/// `relay_state` key: the account this Nandi signed in to the relay with
/// (`{"relay", "account", "provider", "display"}`)
pub const RELAY_ACCOUNT_KEY: &str = "relay_account";

/// How long a sign-in call to the relay may take (it asks the provider)
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the account may take
const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(5);

/// The public relay this app offers: `NANDI_PUBLIC_RELAY_URL`, else
/// VaultDB.ai's
pub fn public_relay_url() -> String {
    crate::utils::env::var(PUBLIC_RELAY_URL_ENV)
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| PUBLIC_RELAY_URL.to_string())
}

/// Who a person signs in to the public relay with
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum RelaySignInProvider {
    Github,
    Google,
    Microsoft,
}

impl RelaySignInProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    /// `None` for a provider this app doesn't know
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim() {
            "github" => Some(Self::Github),
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }
}

/// Whether the public relay can be joined now, and how
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicRelayOffer {
    /// This app offers no public relay
    None,
    /// Sign in with one of `providers` to join
    Ready {
        relay: String,
        providers: Vec<RelaySignInProvider>,
    },
    /// It didn't answer (offline, or not live yet), or not as a public
    /// relay: join later from Network › Connection
    Unreachable { relay: String, detail: String },
    /// It speaks another protocol version: update Nandi
    #[serde(rename_all = "camelCase")]
    VersionMismatch {
        relay: String,
        relay_version: u32,
        app: u32,
    },
}

/// Why the public relay said no, from its refusal `code`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PublicRelayRefusal {
    /// `not_public`: the relay isn't a public one
    NotPublic,
    /// `provider_not_supported`
    ProviderNotSupported,
    /// `signin_unavailable`: the provider is down
    SigninUnavailable,
    /// `signin_not_found`: no such sign-in is waiting
    SigninNotFound,
    /// `signin_expired`: the code expired
    SigninExpired,
    /// `signin_denied`: cancelled on the provider's page
    SigninDenied,
    /// `account_suspended`
    AccountSuspended,
    /// `revoked`: the operator removed this Nandi
    Revoked,
    /// `already_linked`: this Nandi is another account's
    AlreadyLinked,
    /// `key_mismatch`: the relay pinned other keys for this Nandi
    KeyMismatch,
    /// `device_limit`: the account links its most Nandis already
    DeviceLimit,
    /// `signup_limit`: too many new accounts from this network today
    SignupLimit,
    /// `signin_limit`: too many sign-ins waiting from this network
    SigninLimit,
    /// `invalid_request`
    InvalidRequest,
    /// `unauthorized`: the token didn't work
    Unauthorized,
    /// `no_account`: this Nandi isn't linked to an account
    NoAccount,
    /// Anything else, in the relay's words
    Other,
}

impl PublicRelayRefusal {
    pub fn from_code(code: &str) -> Self {
        match code {
            "not_public" => Self::NotPublic,
            "provider_not_supported" => Self::ProviderNotSupported,
            "signin_unavailable" => Self::SigninUnavailable,
            "signin_not_found" => Self::SigninNotFound,
            "signin_expired" => Self::SigninExpired,
            "signin_denied" => Self::SigninDenied,
            "account_suspended" => Self::AccountSuspended,
            "revoked" => Self::Revoked,
            "already_linked" => Self::AlreadyLinked,
            "key_mismatch" => Self::KeyMismatch,
            "device_limit" => Self::DeviceLimit,
            "signup_limit" => Self::SignupLimit,
            "signin_limit" => Self::SigninLimit,
            "invalid_request" => Self::InvalidRequest,
            "unauthorized" => Self::Unauthorized,
            "no_account" => Self::NoAccount,
            _ => Self::Other,
        }
    }
}

/// Starting a sign-in
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicSignInStart {
    /// Show `user_code` and open `verification_uri`; poll every `interval`
    /// seconds for up to `expires_in`
    #[serde(rename_all = "camelCase")]
    Started {
        signin_id: String,
        provider: RelaySignInProvider,
        user_code: String,
        verification_uri: String,
        /// The page with the code filled in, when the provider has one
        verification_uri_complete: Option<String>,
        expires_in: u32,
        interval: u32,
    },
    Refused {
        refusal: PublicRelayRefusal,
        /// The relay's words
        message: String,
    },
    /// The relay didn't answer
    Unreachable { detail: String },
}

/// One poll of a sign-in
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicSignInPoll {
    /// Not approved yet: ask again in `interval` seconds
    Pending { interval: u32 },
    /// Asked too soon: ask again in `interval` seconds
    SlowDown { interval: u32 },
    /// Signed in: the token is kept and the app is connecting to `relay`
    Linked {
        relay: String,
        account: String,
        provider: RelaySignInProvider,
        /// The account's name at the provider (their login or name)
        display: String,
    },
    Refused {
        refusal: PublicRelayRefusal,
        message: String,
    },
    /// The relay didn't answer this time; asking again may work
    Unreachable { detail: String },
}

/// `GET /relay/account`: the account, its Nandis, its limits and today's
/// usage
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccount {
    /// `github:583231`, `google:<sub>`, `microsoft:<tid>.<oid>`
    pub account: String,
    pub provider: RelaySignInProvider,
    pub display: String,
    pub created_at: Timestamp,
    /// The instance IDs of this account's Nandis
    pub instances: Vec<String>,
    pub limits: RelayLimits,
    pub usage: RelayUsage,
}

/// The relay's limits per person (all of a person's Nandis share them)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayLimits {
    pub groups_per_person: u32,
    pub members_per_group: u32,
    /// `None`: no daily limit
    pub messages_per_day: Option<u32>,
}

/// What the person used of them
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayUsage {
    pub groups: u32,
    pub messages_today: u32,
    /// When the daily count resets (00:00 UTC)
    pub resets_at: Timestamp,
}

/// The Account card on Network › Connection
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicRelayAccount {
    /// The relay in use isn't a public relay this Nandi signed in to
    NotPublic,
    /// Signed in, but the relay didn't give the account now: what this
    /// Nandi kept
    Unavailable {
        relay: String,
        account: String,
        provider: RelaySignInProvider,
        display: String,
        detail: String,
    },
    #[serde(rename_all = "camelCase")]
    Account {
        relay: String,
        /// This Nandi's instance ID, among `account.instances`
        this_instance: String,
        account: RelayAccount,
    },
}

/// What this Nandi keeps about its public-relay account
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeptAccount {
    relay: String,
    account: String,
    provider: RelaySignInProvider,
    display: String,
}

/// The account kept for the relay this Nandi's token is for, if that is a
/// public relay it signed in to
fn kept_account(db: &Db) -> Option<KeptAccount> {
    let kept = read_state::<KeptAccount>(db, RELAY_ACCOUNT_KEY)
        .ok()
        .flatten()?;
    let token_relay = read_state::<String>(db, RELAY_TOKEN_RELAY_KEY)
        .ok()
        .flatten()?;
    (kept.relay == token_relay).then_some(kept)
}

/// Whether this Nandi's relay token is from signing in to a public relay:
/// anyone can sign in there, so its members aren't trusted as a company's
/// ([`crate::services::team::trust::on_a_secured_relay`])
pub fn on_public_relay(db: &Db) -> bool {
    kept_account(db).is_some()
}

/// Asks the public relay whether it can be joined now (its `/health`)
pub async fn offer(state: &AppState) -> PublicRelayOffer {
    let Some(relay) = state.built_in_relay().public_url().map(str::to_string) else {
        return PublicRelayOffer::None;
    };
    match health(&relay).await {
        Ok(report) => offer_from(relay, &report),
        Err(outcome) => PublicRelayOffer::Unreachable {
            detail: match outcome {
                super::join::JoinOutcome::Unreachable { detail } => detail,
                _ => format!("{relay} isn't a relay address"),
            },
            relay,
        },
    }
}

fn offer_from(relay: String, report: &RelayHealthReport) -> PublicRelayOffer {
    if report.relay.protocol_version != PROTOCOL_VERSION {
        return PublicRelayOffer::VersionMismatch {
            relay,
            relay_version: report.relay.protocol_version,
            app: PROTOCOL_VERSION,
        };
    }
    if !report.public {
        return PublicRelayOffer::Unreachable {
            detail: format!("{relay} isn't a public relay"),
            relay,
        };
    }
    let providers: Vec<RelaySignInProvider> = report
        .signin
        .iter()
        .filter_map(|name| RelaySignInProvider::parse(name))
        .collect();
    if providers.is_empty() {
        return PublicRelayOffer::Unreachable {
            detail: format!("{relay} offers no sign-in this app knows"),
            relay,
        };
    }
    PublicRelayOffer::Ready { relay, providers }
}

/// Starts signing this Nandi in to the public relay with `provider`. `Err`
/// only when something local fails (the instance ID, this device's keys)
pub async fn start_sign_in(
    state: &AppState,
    provider: RelaySignInProvider,
) -> Result<PublicSignInStart, String> {
    let Some(relay) = state.built_in_relay().public_url() else {
        return Ok(PublicSignInStart::Unreachable {
            detail: "This build offers no public relay".to_string(),
        });
    };
    let instance_id = network::local_instance_id(state.db())?;
    // First relay use: this device's keys, which the relay pins
    let keys = DeviceKeyring::new(state.secrets().clone()).public_keys()?;
    let endpoint = endpoint(relay, &format!("/signin/{}/start", provider.as_str()))?;
    let body = serde_json::json!({ "instanceId": instance_id, "publicKeys": keys });
    Ok(match call(endpoint, &body).await {
        Answer::Unreachable(detail) => PublicSignInStart::Unreachable { detail },
        Answer::Refused { refusal, message } => PublicSignInStart::Refused { refusal, message },
        Answer::Ok(body) => match serde_json::from_value::<Started>(body) {
            Ok(started) => PublicSignInStart::Started {
                signin_id: started.signin_id,
                provider,
                user_code: started.user_code,
                verification_uri: started.verification_uri,
                verification_uri_complete: started.verification_uri_complete,
                expires_in: started.expires_in,
                interval: started.interval.max(1),
            },
            Err(e) => PublicSignInStart::Unreachable {
                detail: format!("The relay's answer wasn't a sign-in: {e}"),
            },
        },
    })
}

/// Asks the relay whether the person approved the sign-in. On `linked`
/// the token is kept exactly like an enrolled one and the app connects.
/// `Err` only when saving fails locally.
pub async fn poll_sign_in(
    state: &AppState,
    provider: RelaySignInProvider,
    signin_id: &str,
) -> Result<PublicSignInPoll, String> {
    let Some(relay) = state.built_in_relay().public_url().map(str::to_string) else {
        return Ok(PublicSignInPoll::Unreachable {
            detail: "This build offers no public relay".to_string(),
        });
    };
    let endpoint = endpoint(&relay, &format!("/signin/{}/poll", provider.as_str()))?;
    let body = serde_json::json!({ "signinId": signin_id.trim() });
    let polled = match call(endpoint, &body).await {
        Answer::Unreachable(detail) => return Ok(PublicSignInPoll::Unreachable { detail }),
        Answer::Refused { refusal, message } => {
            return Ok(PublicSignInPoll::Refused { refusal, message })
        }
        Answer::Ok(body) => body,
    };
    let unexpected = |e: String| PublicSignInPoll::Unreachable {
        detail: format!("The relay's answer wasn't a sign-in poll: {e}"),
    };
    match polled["status"].as_str() {
        Some("pending") => Ok(PublicSignInPoll::Pending {
            interval: interval_of(&polled),
        }),
        Some("slow_down") => Ok(PublicSignInPoll::SlowDown {
            interval: interval_of(&polled),
        }),
        Some("linked") => match serde_json::from_value::<Linked>(polled) {
            Ok(linked) => link(state, relay, provider, linked).await,
            Err(e) => Ok(unexpected(e.to_string())),
        },
        other => Ok(unexpected(format!("status {other:?}"))),
    }
}

/// The relay linked this Nandi to an account: keep its token as enrolling
/// would, remember the account, and connect
async fn link(
    state: &AppState,
    relay: String,
    provider: RelaySignInProvider,
    linked: Linked,
) -> Result<PublicSignInPoll, String> {
    let instance_id = network::local_instance_id(state.db())?;
    if linked.instance_id != instance_id || linked.token.trim().is_empty() {
        return Ok(PublicSignInPoll::Refused {
            refusal: PublicRelayRefusal::Other,
            message: "The relay linked another Nandi than this one".to_string(),
        });
    }
    let (base, _) = split_token(&relay);
    write_state(
        state.db(),
        RELAY_ACCOUNT_KEY,
        &KeptAccount {
            relay: base.clone(),
            account: linked.account.clone(),
            provider,
            display: linked.display.clone(),
        },
    )?;
    set_relay_url(state, &with_token(&base, Some(&linked.token))).await?;
    log::info!("Signed in to the public relay {base}");
    Ok(PublicSignInPoll::Linked {
        relay: base,
        account: linked.account,
        provider,
        display: linked.display,
    })
}

/// The Account card: the account this Nandi signed in to the relay in use
/// with, as the relay tells it now (or as kept, while it doesn't answer)
pub async fn account(state: &AppState) -> Result<PublicRelayAccount, String> {
    let Some(kept) = kept_account(state.db()) else {
        return Ok(PublicRelayAccount::NotPublic);
    };
    if configured_url(state).as_deref() != Some(kept.relay.as_str()) {
        return Ok(PublicRelayAccount::NotPublic);
    }
    let unavailable = |detail: String| PublicRelayAccount::Unavailable {
        relay: kept.relay.clone(),
        account: kept.account.clone(),
        provider: kept.provider,
        display: kept.display.clone(),
        detail,
    };
    let Some(token) = saved_token_for(state.db(), &kept.relay)? else {
        return Ok(unavailable("No token is kept for this relay".to_string()));
    };
    let instance_id = network::local_instance_id(state.db())?;
    let mut endpoint = endpoint(&kept.relay, "/account")?;
    endpoint
        .query_pairs_mut()
        .append_pair("instanceId", &instance_id);
    let response = reqwest::Client::new()
        .get(endpoint.as_str())
        .bearer_auth(token)
        .timeout(ACCOUNT_TIMEOUT)
        .send()
        .await;
    let answer = match response {
        Ok(response) => read_answer(&endpoint, response).await,
        Err(e) => Answer::Unreachable(format!("Couldn't reach {endpoint}: {e}")),
    };
    Ok(match answer {
        Answer::Unreachable(detail) => unavailable(detail),
        Answer::Refused { message, .. } => unavailable(message),
        Answer::Ok(body) => match serde_json::from_value::<RelayAccount>(body) {
            Ok(account) => PublicRelayAccount::Account {
                relay: kept.relay.clone(),
                this_instance: instance_id,
                account,
            },
            Err(e) => unavailable(format!("The relay's answer wasn't an account: {e}")),
        },
    })
}

// ---------------------------------------------------------------------------
// The relay's HTTP API
// ---------------------------------------------------------------------------

/// `POST /relay/signin/{provider}/start`'s answer
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    signin_id: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u32,
    interval: u32,
}

/// `{"status": "linked", ...}`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Linked {
    instance_id: String,
    token: String,
    account: String,
    #[serde(default)]
    display: String,
}

fn interval_of(body: &serde_json::Value) -> u32 {
    body["interval"]
        .as_u64()
        .and_then(|interval| u32::try_from(interval).ok())
        .unwrap_or(5)
        .max(1)
}

/// What the relay answered
enum Answer {
    Ok(serde_json::Value),
    /// `{"error", "code"}`
    Refused {
        refusal: PublicRelayRefusal,
        message: String,
    },
    /// Nothing, or not as the relay (a proxy's error page)
    Unreachable(String),
}

async fn call(endpoint: reqwest::Url, body: &serde_json::Value) -> Answer {
    let response = reqwest::Client::new()
        .post(endpoint.as_str())
        .timeout(SIGNIN_TIMEOUT)
        .json(body)
        .send()
        .await;
    match response {
        Ok(response) => read_answer(&endpoint, response).await,
        Err(e) => Answer::Unreachable(format!("Couldn't reach {endpoint}: {e}")),
    }
}

async fn read_answer(endpoint: &reqwest::Url, response: reqwest::Response) -> Answer {
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if status.is_success() {
        return Answer::Ok(body);
    }
    // The relay's own refusals carry a code (a provider that is down is
    // `signin_unavailable`, a 502); anything else didn't come from it
    match body["code"].as_str() {
        Some(code) => Answer::Refused {
            refusal: PublicRelayRefusal::from_code(code),
            message: body["error"]
                .as_str()
                .unwrap_or("no reason given")
                .to_string(),
        },
        None => Answer::Unreachable(format!("{endpoint} answered {status}")),
    }
}

/// `wss://host/relay` → `https://host/relay<suffix>` (any path prefix kept)
fn endpoint(relay_url: &str, suffix: &str) -> Result<reqwest::Url, String> {
    let mut url = instances_endpoint(relay_url)?;
    let (base, _) = split_token(relay_url);
    let path = reqwest::Url::parse(&base)
        .map(|u| u.path().trim_end_matches('/').to_string())
        .unwrap_or_default();
    url.set_path(&format!("{path}{suffix}"));
    Ok(url)
}

#[cfg(test)]
#[path = "public_tests.rs"]
mod tests;
