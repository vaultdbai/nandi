//! Who this Nandi's person is: the Outlook or Gmail account they connected
//! (Settings › Email), locked as this Nandi's identity. There are no
//! sign-in screens.
//!
//! - **Captured at connect** ([`lock_from_mail`]): the mail sign-in asks
//!   for `openid email profile`, so the provider's token endpoint returns
//!   an ID token. Its verified email and name become the identity, the
//!   same way VaultDB.ai's services read it: Gmail's `email` with
//!   `email_verified`, Outlook's `preferred_username` (a work account's
//!   UPN, whose domain the company proved, or a personal account's sign-in
//!   address; never Microsoft's settable `email` claim).
//! - **Locked:** the first identity stays (`config.json` `identity`); the
//!   UI shows it read-only in Settings › Account. Connecting another
//!   mailbox later doesn't change it.
//! - **Proved once** ([`fresh_id_token`]): joining the public relay and
//!   linking the free trial send a fresh ID token of the identity's
//!   mailbox. Each service checks it and pins this Nandi's device key to
//!   the email, and from then on the key alone proves the person.
//!
//! [`machine_id`] is the salted hash of this computer each service gets
//! alongside.

pub mod machine_id;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::model::Timestamp;
use crate::services::auth::oauth::{self, TokenError};
use crate::services::email::MailProvider;
use crate::services::AppState;
use crate::utils::nandi_config;

/// Where to connect a mailbox, for messages that point there
pub const CONNECT_EMAIL_HINT: &str = "Connect your Outlook or Gmail in Settings › Email";

/// This Nandi's person, from the first mailbox connected
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedIdentity {
    /// The mailbox it came from
    pub provider: MailProvider,
    /// Verified, lowercase
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    /// When it was locked (ms since the epoch)
    pub locked_at: Timestamp,
}

/// The verified email and name in an ID token's claims, as the services
/// read them
pub fn from_claims(
    provider: MailProvider,
    claims: &Value,
) -> Result<(String, Option<String>), String> {
    let claim = |name: &str| {
        claims
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let looks_like_email =
        |value: &&str| value.contains('@') && !value.contains(' ') && !value.contains('#');
    let email = match provider {
        MailProvider::Gmail => {
            let verified = match claims.get("email_verified") {
                Some(Value::Bool(yes)) => *yes,
                Some(Value::String(text)) => text.eq_ignore_ascii_case("true"),
                _ => false,
            };
            if !verified {
                return Err("Google hasn't verified this account's email address".to_string());
            }
            claim("email")
        }
        MailProvider::Outlook => claim("preferred_username"),
    }
    .filter(looks_like_email)
    .ok_or_else(|| {
        format!(
            "The {} sign-in didn't name an email address",
            provider.name()
        )
    })?;
    Ok((
        email.to_ascii_lowercase(),
        claim("name").map(str::to_string),
    ))
}

/// The locked identity, if a mailbox was ever connected
pub fn identity(state: &AppState) -> Option<VerifiedIdentity> {
    nandi_config::load_from(state.nandi_dir())
        .ok()
        .and_then(|config| config.identity)
}

/// After a mailbox connected: locks the identity from its sign-in's ID
/// token unless one is locked already. The identity now, if any.
pub fn lock_from_mail(
    state: &AppState,
    provider: MailProvider,
    now: Timestamp,
) -> Result<Option<VerifiedIdentity>, String> {
    if let Some(locked) = identity(state) {
        return Ok(Some(locked));
    }
    let Some(token) = oauth::stored_id_token(&provider.token_entry(), state.secrets()) else {
        log::warn!(
            "{} connected without an ID token: no identity",
            provider.name()
        );
        return Ok(None);
    };
    let claims = oauth::id_token_claims(&token)
        .ok_or_else(|| format!("The {} ID token couldn't be read", provider.name()))?;
    let (email, name) = from_claims(provider, &claims)?;
    let locked = VerifiedIdentity {
        provider,
        email,
        name,
        locked_at: now,
    };
    let kept = locked.clone();
    nandi_config::update_in(state.nandi_dir(), move |config| {
        config.identity.get_or_insert(kept);
    })?;
    log::info!(
        "Locked this Nandi's identity to its {} mailbox",
        provider.name()
    );
    Ok(identity(state))
}

/// Why there's no ID token to prove the identity with
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoIdToken {
    /// No mailbox was ever connected
    NoEmail,
    /// The identity's mailbox isn't connected now (or another account of
    /// it is): connect `email` again
    Disconnected { email: String },
    /// Its sign-in ended: connect it again
    SignIn(String),
    /// The provider couldn't be reached
    Unavailable(String),
}

impl NoIdToken {
    pub fn message(&self) -> String {
        match self {
            Self::NoEmail => format!("{CONNECT_EMAIL_HINT} first."),
            Self::Disconnected { email } => {
                format!("Connect {email} again in Settings › Email: this Nandi's identity is that mailbox.")
            }
            Self::SignIn(detail) => {
                format!("The mailbox's sign-in ended ({detail}). Reconnect it in Settings › Email.")
            }
            Self::Unavailable(detail) => {
                format!("Couldn't reach the mail provider to prove who you are: {detail}")
            }
        }
    }
}

/// A fresh ID token of the identity's mailbox, and the identity
pub async fn fresh_id_token(state: &AppState) -> Result<(String, VerifiedIdentity), NoIdToken> {
    let locked = identity(state).ok_or(NoIdToken::NoEmail)?;
    let provider = locked.provider;
    let connected = crate::services::email::connected_account(state, provider)
        .is_some_and(|email| email.eq_ignore_ascii_case(&locked.email));
    if !connected {
        return Err(NoIdToken::Disconnected {
            email: locked.email,
        });
    }
    let setup = state.email().setup();
    let token = oauth::fresh_id_token(
        &provider.token_entry(),
        &setup.oauth(provider),
        state.secrets(),
    )
    .await
    .map_err(|e| match e {
        TokenError::NeedsSignIn(detail) => NoIdToken::SignIn(detail),
        TokenError::Unavailable(detail) => NoIdToken::Unavailable(detail),
    })?;
    // The token must still name the locked person
    let same = oauth::id_token_claims(&token)
        .and_then(|claims| from_claims(provider, &claims).ok())
        .is_some_and(|(email, _)| email == locked.email);
    if !same {
        return Err(NoIdToken::Disconnected {
            email: locked.email,
        });
    }
    Ok((token, locked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gmail_needs_a_verified_email() {
        let claims = json!({"email": "Ada@Example.com", "email_verified": true, "name": "Ada"});
        assert_eq!(
            from_claims(MailProvider::Gmail, &claims).unwrap(),
            ("ada@example.com".to_string(), Some("Ada".to_string()))
        );
        let unverified = json!({"email": "ada@example.com", "email_verified": false});
        assert!(from_claims(MailProvider::Gmail, &unverified)
            .unwrap_err()
            .contains("verified"));
        assert!(from_claims(MailProvider::Gmail, &json!({"email": "ada@example.com"})).is_err());
    }

    #[test]
    fn outlook_uses_the_sign_in_name_not_the_settable_email() {
        let claims = json!({
            "preferred_username": "dev@contoso.com", "email": "ceo@example.com", "name": "Dev"
        });
        assert_eq!(
            from_claims(MailProvider::Outlook, &claims).unwrap().0,
            "dev@contoso.com"
        );
        assert!(from_claims(MailProvider::Outlook, &json!({"email": "ceo@example.com"})).is_err());
        // A guest's UPN isn't an address
        let guest = json!({"preferred_username": "dev_gmail.com#EXT#@contoso.onmicrosoft.com"});
        assert!(from_claims(MailProvider::Outlook, &guest).is_err());
    }

    #[test]
    fn messages_point_to_settings_email() {
        assert!(NoIdToken::NoEmail.message().contains("Settings › Email"));
        let again = NoIdToken::Disconnected {
            email: "ada@example.com".into(),
        };
        assert!(again.message().contains("ada@example.com"));
    }
}
