//! The two mail providers, how to sign in to each, and where their APIs
//! are. Client IDs are baked in at build time (`NANDI_MICROSOFT_CLIENT_ID`,
//! `NANDI_GOOGLE_CLIENT_ID`, `NANDI_GOOGLE_CLIENT_SECRET`); a build
//! without one can't connect that provider and says so.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::auth::oauth::{build_env, LoopbackRedirect, OAuthProvider};

/// Read-only Microsoft Graph scopes: mail, and who the user is
const MICROSOFT_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "User.Read",
    "Mail.Read",
];

/// Read-only Gmail, and who the user is
const GOOGLE_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/gmail.readonly",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MailProvider {
    /// Outlook and Microsoft 365, through Microsoft Graph
    Outlook,
    Gmail,
}

impl MailProvider {
    pub const ALL: [Self; 2] = [Self::Outlook, Self::Gmail];

    pub fn id(self) -> &'static str {
        match self {
            Self::Outlook => "outlook",
            Self::Gmail => "gmail",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Outlook => "Outlook",
            Self::Gmail => "Gmail",
        }
    }

    /// The provider whose message IDs start `<id>:`
    pub fn of_message_id(id: &str) -> Option<(Self, &str)> {
        Self::ALL.into_iter().find_map(|provider| {
            id.strip_prefix(provider.id())
                .and_then(|rest| rest.strip_prefix(':'))
                .filter(|rest| !rest.is_empty())
                .map(|rest| (provider, rest))
        })
    }

    /// The keyring entry of its sign-in
    pub fn token_entry(self) -> String {
        format!("nandi.email.{}.oauth", self.id())
    }

    /// `local_settings` keys of its change cursor and its last check
    pub fn cursor_key(self) -> String {
        format!("email.{}.cursor", self.id())
    }

    pub fn state_key(self) -> String {
        format!("email.{}.state", self.id())
    }

    pub fn backfill_key(self) -> String {
        format!("email.{}.backfill", self.id())
    }
}

/// The APIs' addresses; tests point them at a local mock
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// `https://login.microsoftonline.com/common/oauth2/v2.0`
    pub microsoft_login: String,
    pub graph: String,
    pub google_authorize: String,
    pub google_token: String,
    pub gmail: String,
    pub google_userinfo: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            microsoft_login: "https://login.microsoftonline.com/common/oauth2/v2.0".into(),
            graph: "https://graph.microsoft.com/v1.0".into(),
            google_authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            google_token: "https://oauth2.googleapis.com/token".into(),
            gmail: "https://gmail.googleapis.com/gmail/v1".into(),
            google_userinfo: "https://openidconnect.googleapis.com/v1/userinfo".into(),
        }
    }
}

/// What this build can sign in with
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderSetup {
    pub endpoints: Endpoints,
    pub microsoft_client_id: Option<String>,
    pub google_client_id: Option<String>,
    /// Google requires it for desktop clients; it isn't confidential
    pub google_client_secret: Option<String>,
}

impl ProviderSetup {
    /// The client IDs baked into this build
    pub fn of_this_build() -> Self {
        Self {
            endpoints: Endpoints::default(),
            microsoft_client_id: build_env(option_env!("NANDI_MICROSOFT_CLIENT_ID")),
            google_client_id: build_env(option_env!("NANDI_GOOGLE_CLIENT_ID")),
            google_client_secret: build_env(option_env!("NANDI_GOOGLE_CLIENT_SECRET")),
        }
    }

    /// Whether this build can connect `provider`
    pub fn available(&self, provider: MailProvider) -> bool {
        match provider {
            MailProvider::Outlook => self.microsoft_client_id.is_some(),
            MailProvider::Gmail => {
                self.google_client_id.is_some() && self.google_client_secret.is_some()
            }
        }
    }

    pub fn oauth(&self, provider: MailProvider) -> OAuthProvider {
        let scopes = |list: &[&str]| list.iter().map(|s| (*s).to_string()).collect();
        let login = &self.endpoints.microsoft_login;
        match provider {
            MailProvider::Outlook => OAuthProvider {
                id: provider.id().into(),
                name: "Microsoft".into(),
                authorize_url: Some(format!("{login}/authorize")),
                token_url: format!("{login}/token"),
                // Microsoft allows the device flow with mail scopes: the
                // fallback when the redirect server can't start
                device_url: Some(format!("{login}/devicecode")),
                client_id: self.microsoft_client_id.clone(),
                client_secret: None,
                scopes: scopes(MICROSOFT_SCOPES),
                auth_params: vec![("prompt".into(), "select_account".into())],
                redirect: LoopbackRedirect::LOCALHOST,
            },
            MailProvider::Gmail => OAuthProvider {
                id: provider.id().into(),
                name: "Google".into(),
                authorize_url: Some(self.endpoints.google_authorize.clone()),
                token_url: self.endpoints.google_token.clone(),
                // Google's device flow doesn't allow Gmail scopes
                device_url: None,
                client_id: self.google_client_id.clone(),
                client_secret: self.google_client_secret.clone(),
                scopes: scopes(GOOGLE_SCOPES),
                auth_params: vec![
                    ("access_type".into(), "offline".into()),
                    ("prompt".into(), "consent".into()),
                ],
                redirect: LoopbackRedirect::CALLBACK,
            },
        }
    }

    /// The API addresses a sign-in's token may be sent to
    pub fn api_bases(&self, provider: MailProvider) -> Vec<String> {
        match provider {
            MailProvider::Outlook => vec![self.endpoints.graph.clone()],
            MailProvider::Gmail => vec![
                self.endpoints.gmail.clone(),
                self.endpoints.google_userinfo.clone(),
            ],
        }
    }

    /// The page an IT admin opens to approve Nandi for their company
    pub fn admin_consent_url(&self) -> Option<String> {
        let client_id = self.microsoft_client_id.as_deref()?;
        reqwest::Url::parse_with_params(
            "https://login.microsoftonline.com/organizations/adminconsent",
            &[("client_id", client_id)],
        )
        .ok()
        .map(|url| url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ids_name_their_provider() {
        assert_eq!(
            MailProvider::of_message_id("outlook:AAMk=="),
            Some((MailProvider::Outlook, "AAMk=="))
        );
        assert_eq!(
            MailProvider::of_message_id("gmail:18c"),
            Some((MailProvider::Gmail, "18c"))
        );
        assert_eq!(MailProvider::of_message_id("gmail:"), None);
        assert_eq!(MailProvider::of_message_id("imap:1"), None);
    }

    #[test]
    fn scopes_are_read_only() {
        let setup = ProviderSetup::default();
        for provider in MailProvider::ALL {
            for scope in setup.oauth(provider).scopes {
                let lower = scope.to_ascii_lowercase();
                assert!(
                    !lower.contains("send")
                        && !lower.contains("readwrite")
                        && !lower.contains("modify"),
                    "{scope}"
                );
            }
        }
        assert!(setup
            .oauth(MailProvider::Gmail)
            .scopes
            .contains(&"https://www.googleapis.com/auth/gmail.readonly".to_string()));
        assert!(setup
            .oauth(MailProvider::Outlook)
            .scopes
            .contains(&"Mail.Read".to_string()));
    }

    #[test]
    fn a_build_without_client_ids_has_no_providers() {
        let setup = ProviderSetup::default();
        assert!(!setup.available(MailProvider::Outlook));
        assert!(!setup.available(MailProvider::Gmail));
        assert_eq!(setup.admin_consent_url(), None);
        let setup = ProviderSetup {
            microsoft_client_id: Some("c62f".into()),
            google_client_id: Some("430".into()),
            ..ProviderSetup::default()
        };
        assert!(setup.available(MailProvider::Outlook));
        // Google needs its desktop client secret too
        assert!(!setup.available(MailProvider::Gmail));
        assert!(setup
            .admin_consent_url()
            .unwrap()
            .contains("client_id=c62f"));
    }

    #[test]
    fn microsoft_comes_back_to_localhost_and_google_to_the_loopback_address() {
        let setup = ProviderSetup::default();
        assert_eq!(
            setup.oauth(MailProvider::Outlook).redirect,
            LoopbackRedirect::LOCALHOST
        );
        assert!(setup
            .oauth(MailProvider::Outlook)
            .authorize_url
            .unwrap()
            .contains("/common/"));
        assert_eq!(
            setup.oauth(MailProvider::Gmail).redirect,
            LoopbackRedirect::CALLBACK
        );
    }
}
