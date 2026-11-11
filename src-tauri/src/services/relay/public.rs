//! The public relay: VaultDB.ai's relay for people without a company relay
//! ("Join the public relay" in onboarding and Network › Connection).
//!
//! There is no sign-in. A Nandi joins with the identity its person
//! connected: the Outlook or Gmail mailbox locked as this Nandi's identity
//! (`services::identity`). The app sends a fresh ID token of that mailbox
//! once; the relay checks it, locks the email to this Nandi's device keys
//! and issues the instance token. From then on the pinned keys alone prove
//! the person. Without a connected email the public relay can't be joined
//! ([`PublicRelayOffer::NeedsEmail`]).
//!
//! | Step | Here | At the relay |
//! | --- | --- | --- |
//! | Offer | [`offer`] | `GET /health`: `public`, `identity` (whose ID tokens it accepts) |
//! | Join | [`join`] | `POST /relay/public/join` with this instance's ID, public keys, the ID token and the machine hash (`identity::machine_id`, the relay's salt) → the token, kept exactly like an enrolled one; the app connects |
//! | Account | [`account`] | `GET /relay/account?instanceId=` with the token as bearer |
//!
//! Which relay: [`super::BuiltInRelay::public_url`], `NANDI_PUBLIC_RELAY_URL`
//! when set (staging, tests), else [`PUBLIC_RELAY_URL`]. Nothing here runs
//! at startup or blocks onboarding: an unreachable relay is a calm
//! [`PublicRelayOffer::Unreachable`], and the rest of Nandi works the same.
//!
//! What the app keeps: the token in `relay_state` (as enrolling does) and,
//! next to it, the account it joined as ([`RELAY_ACCOUNT_KEY`]), so the
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
use crate::services::identity::{self, machine_id, NoIdToken};
use crate::services::network;
use crate::services::AppState;

/// VaultDB.ai's public relay
pub const PUBLIC_RELAY_URL: &str = "wss://relay.vaultdb.ai/relay";

/// Points the app at another public relay (staging, tests)
pub const PUBLIC_RELAY_URL_ENV: &str = "NANDI_PUBLIC_RELAY_URL";

/// `relay_state` key: the account this Nandi joined the relay as
/// (`{"relay", "account", "email", "name"}`)
pub const RELAY_ACCOUNT_KEY: &str = "relay_account";

/// How long joining may take (the relay may fetch the provider's keys)
const JOIN_TIMEOUT: Duration = Duration::from_secs(15);

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

/// Whether the public relay can be joined now, and how
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicRelayOffer {
    /// This app offers no public relay
    None,
    /// "Join the public relay" as `email`, this Nandi's identity
    Ready { relay: String, email: String },
    /// No email is connected: connect Outlook or Gmail in Settings › Email
    /// first
    NeedsEmail { relay: String },
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
    /// `id_token_invalid`: the relay didn't accept the mailbox's ID token
    IdTokenInvalid,
    /// `id_token_expired`
    IdTokenExpired,
    /// `email_unverified`: the provider hasn't verified the email
    EmailUnverified,
    /// `identity_unavailable`: the relay can't check sign-ins right now
    IdentityUnavailable,
    /// `account_suspended`
    AccountSuspended,
    /// `revoked`: the operator removed this Nandi
    Revoked,
    /// `already_linked`: this Nandi joined with another email
    AlreadyLinked,
    /// `key_mismatch`: the relay pinned other keys for this Nandi
    KeyMismatch,
    /// `device_limit`: the account links its most Nandis already
    DeviceLimit,
    /// `machine_limit`: this computer joined its most Nandis or accounts
    MachineLimit,
    /// `signup_limit`: too many new accounts from this network today
    SignupLimit,
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
            "id_token_invalid" => Self::IdTokenInvalid,
            "id_token_expired" => Self::IdTokenExpired,
            "email_unverified" => Self::EmailUnverified,
            "identity_unavailable" => Self::IdentityUnavailable,
            "account_suspended" => Self::AccountSuspended,
            "revoked" => Self::Revoked,
            "already_linked" => Self::AlreadyLinked,
            "key_mismatch" => Self::KeyMismatch,
            "device_limit" => Self::DeviceLimit,
            "machine_limit" => Self::MachineLimit,
            "signup_limit" => Self::SignupLimit,
            "invalid_request" => Self::InvalidRequest,
            "unauthorized" => Self::Unauthorized,
            "no_account" => Self::NoAccount,
            _ => Self::Other,
        }
    }
}

/// Joining the public relay
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicRelayJoin {
    /// Joined: the token is kept and the app is connecting to `relay`
    Joined {
        relay: String,
        account: String,
        email: String,
        name: String,
    },
    /// This Nandi has no connected email to join with (or its mailbox
    /// needs connecting again): `message` says what to do
    NeedsEmail { message: String },
    Refused {
        refusal: PublicRelayRefusal,
        /// The relay's words
        message: String,
    },
    /// The relay (or the mail provider) didn't answer; trying again may
    /// work
    Unreachable { detail: String },
}

/// `GET /relay/account`: the account, its Nandis, its limits and today's
/// usage
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccount {
    /// `google:<sub>`, `microsoft:<tid>.<oid>`
    pub account: String,
    /// `google` or `microsoft`
    pub provider: String,
    /// The verified email the account was locked to
    pub email: String,
    pub name: String,
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
    /// The relay in use isn't a public relay this Nandi joined
    NotPublic,
    /// Joined, but the relay didn't give the account now: what this Nandi
    /// kept
    Unavailable {
        relay: String,
        account: String,
        email: String,
        name: String,
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
    email: String,
    name: String,
}

/// The account kept for the relay this Nandi's token is for, if that is a
/// public relay it joined
fn kept_account(db: &Db) -> Option<KeptAccount> {
    let kept = read_state::<KeptAccount>(db, RELAY_ACCOUNT_KEY)
        .ok()
        .flatten()?;
    let token_relay = read_state::<String>(db, RELAY_TOKEN_RELAY_KEY)
        .ok()
        .flatten()?;
    (kept.relay == token_relay).then_some(kept)
}

/// Whether this Nandi's relay token is from joining a public relay: anyone
/// with an email can join there, so its members aren't trusted as a
/// company's
/// ([`crate::services::team::trust::on_a_secured_relay`])
pub fn on_public_relay(db: &Db) -> bool {
    kept_account(db).is_some()
}

/// Asks the public relay whether it can be joined now (its `/health`)
pub async fn offer(state: &AppState) -> PublicRelayOffer {
    let Some(relay) = state.built_in_relay().public_url().map(str::to_string) else {
        return PublicRelayOffer::None;
    };
    let email = identity::identity(state).map(|locked| locked.email);
    match health(&relay).await {
        Ok(report) => offer_from(relay, &report, email),
        Err(outcome) => PublicRelayOffer::Unreachable {
            detail: match outcome {
                super::join::JoinOutcome::Unreachable { detail } => detail,
                _ => format!("{relay} isn't a relay address"),
            },
            relay,
        },
    }
}

fn offer_from(
    relay: String,
    report: &RelayHealthReport,
    email: Option<String>,
) -> PublicRelayOffer {
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
    if report.identity.is_empty() {
        return PublicRelayOffer::Unreachable {
            detail: format!("{relay} accepts no Outlook or Gmail sign-in"),
            relay,
        };
    }
    match email {
        Some(email) => PublicRelayOffer::Ready { relay, email },
        None => PublicRelayOffer::NeedsEmail { relay },
    }
}

/// Joins this Nandi to the public relay with its identity's ID token. On
/// `joined` the token is kept exactly like an enrolled one and the app
/// connects. `Err` only when something local fails (the instance ID, this
/// device's keys, saving).
pub async fn join(state: &AppState) -> Result<PublicRelayJoin, String> {
    let Some(relay) = state.built_in_relay().public_url().map(str::to_string) else {
        return Ok(PublicRelayJoin::Unreachable {
            detail: "This build offers no public relay".to_string(),
        });
    };
    let id_token = match identity::fresh_id_token(state).await {
        Ok((token, _)) => token,
        Err(NoIdToken::Unavailable(detail)) => return Ok(PublicRelayJoin::Unreachable { detail }),
        Err(NoIdToken::NoEmail) => {
            return Ok(PublicRelayJoin::NeedsEmail {
                message: format!(
                    "{} to join groups on the public relay.",
                    identity::CONNECT_EMAIL_HINT
                ),
            })
        }
        Err(problem) => {
            return Ok(PublicRelayJoin::NeedsEmail {
                message: problem.message(),
            })
        }
    };
    let machine_hash = machine_id::this_machine(machine_id::Service::Relay)?;
    let instance_id = network::local_instance_id(state.db())?;
    // First relay use: this device's keys, which the relay pins
    let keys = DeviceKeyring::new(state.secrets().clone()).public_keys()?;
    let endpoint = endpoint(&relay, "/public/join")?;
    let body = serde_json::json!({
        "instanceId": instance_id,
        "publicKeys": keys,
        "idToken": id_token,
        "machineHash": machine_hash,
    });
    let joined = match call(endpoint, &body).await {
        Answer::Unreachable(detail) => return Ok(PublicRelayJoin::Unreachable { detail }),
        Answer::Refused { refusal, message } => {
            return Ok(PublicRelayJoin::Refused { refusal, message })
        }
        Answer::Ok(body) => match serde_json::from_value::<Joined>(body) {
            Ok(joined) => joined,
            Err(e) => {
                return Ok(PublicRelayJoin::Unreachable {
                    detail: format!("The relay's answer wasn't a join: {e}"),
                })
            }
        },
    };
    if joined.instance_id != instance_id || joined.token.trim().is_empty() {
        return Ok(PublicRelayJoin::Refused {
            refusal: PublicRelayRefusal::Other,
            message: "The relay joined another Nandi than this one".to_string(),
        });
    }
    let (base, _) = split_token(&relay);
    write_state(
        state.db(),
        RELAY_ACCOUNT_KEY,
        &KeptAccount {
            relay: base.clone(),
            account: joined.account.clone(),
            email: joined.email.clone(),
            name: joined.name.clone(),
        },
    )?;
    set_relay_url(state, &with_token(&base, Some(&joined.token))).await?;
    log::info!("Joined the public relay {base}");
    Ok(PublicRelayJoin::Joined {
        relay: base,
        account: joined.account,
        email: joined.email,
        name: joined.name,
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
        email: kept.email.clone(),
        name: kept.name.clone(),
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

/// `POST /relay/public/join`'s answer
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Joined {
    instance_id: String,
    token: String,
    account: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    name: String,
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
        .timeout(JOIN_TIMEOUT)
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
    // The relay's own refusals carry a code (keys it can't fetch are
    // `identity_unavailable`, a 502); anything else didn't come from it
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
