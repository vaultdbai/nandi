//! The two mail providers, how to sign in to each, and where their APIs
//! are. Client IDs are baked in at build time (`NANDI_MICROSOFT_CLIENT_ID`,
//! `NANDI_GOOGLE_CLIENT_ID`, `NANDI_GOOGLE_CLIENT_SECRET`); a build
//! without one can't connect that provider and says so.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use crate::services::auth::oauth::{build_env, LoopbackRedirect, OAuthProvider};
use crate::services::workspace::AccountFamily;

/// Who the user is, and a sign-in that renews: every Microsoft sign-in
const MICROSOFT_SIGN_IN: &[&str] = &["openid", "profile", "email", "offline_access", "User.Read"];
/// Who the user is: every Google sign-in
const GOOGLE_SIGN_IN: &[&str] = &["openid", "email", "profile"];

const MICROSOFT_MAIL: &[&str] = &["Mail.Read"];
/// The person's files and files shared with them
const MICROSOFT_FILES: &[&str] = &["Files.Read", "Files.Read.All"];
const MICROSOFT_CALENDAR: &[&str] = &["Calendars.Read"];
const GOOGLE_MAIL: &[&str] = &["https://www.googleapis.com/auth/gmail.readonly"];
const GOOGLE_FILES: &[&str] = &["https://www.googleapis.com/auth/drive.readonly"];
const GOOGLE_CALENDAR: &[&str] = &["https://www.googleapis.com/auth/calendar.readonly"];

/// What an account's sign-in lets Nandi read, each read-only. The sign-in
/// asks for exactly these scopes (incremental consent: files and calendar
/// only when the person ticks them)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Reads {
    pub mail: bool,
    /// OneDrive or Google Drive
    pub files: bool,
    pub calendar: bool,
}

impl Reads {
    #[cfg(test)]
    pub const MAIL: Self = Self {
        mail: true,
        files: false,
        calendar: false,
    };

    /// Only files, or only the calendar
    pub fn only(family: AccountFamily) -> Self {
        Self::default().with(family, true)
    }

    pub fn has(self, family: AccountFamily) -> bool {
        match family {
            AccountFamily::Files => self.files,
            AccountFamily::Calendar => self.calendar,
        }
    }

    pub fn with(mut self, family: AccountFamily, on: bool) -> Self {
        match family {
            AccountFamily::Files => self.files = on,
            AccountFamily::Calendar => self.calendar = on,
        }
        self
    }

    pub fn is_empty(self) -> bool {
        !self.mail && !self.files && !self.calendar
    }

    /// Everything `other` reads, this reads too
    pub fn covers(self, other: Self) -> bool {
        (self.mail || !other.mail)
            && (self.files || !other.files)
            && (self.calendar || !other.calendar)
    }

    /// The read-only scopes a sign-in to `cloud` asks for
    pub fn scopes(self, cloud: Cloud) -> Vec<&'static str> {
        let (sign_in, mail, files, calendar) = match cloud {
            Cloud::Microsoft => (
                MICROSOFT_SIGN_IN,
                MICROSOFT_MAIL,
                MICROSOFT_FILES,
                MICROSOFT_CALENDAR,
            ),
            Cloud::Google => (GOOGLE_SIGN_IN, GOOGLE_MAIL, GOOGLE_FILES, GOOGLE_CALENDAR),
        };
        let mut scopes = sign_in.to_vec();
        for (on, more) in [
            (self.mail, mail),
            (self.files, files),
            (self.calendar, calendar),
        ] {
            if on {
                scopes.extend_from_slice(more);
            }
        }
        scopes
    }
}

/// Whose sign-in an account uses
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Cloud {
    Microsoft,
    Google,
}

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

    pub fn cloud(self) -> Cloud {
        match self {
            Self::Outlook => Cloud::Microsoft,
            Self::Gmail => Cloud::Google,
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

    /// The keyring entry sign-in attempt `attempt` lands in before Nandi knows whose
    /// mailbox it is; connecting moves it to the mailbox's own entry
    pub fn sign_in_entry(self, attempt: &str) -> String {
        format!("nandi.email.{}.sign-in.{attempt}", self.id())
    }
}

/// One connected mailbox: its provider and its address. Connecting a
/// second address of the same provider adds a mailbox; each keeps its own
/// sign-in, cursor, last check and background read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MailboxId {
    pub provider: MailProvider,
    pub email: String,
}

impl MailboxId {
    pub fn new(provider: MailProvider, email: &str) -> Self {
        Self {
            provider,
            email: email.trim().to_ascii_lowercase(),
        }
    }

    /// Whether `provider` and `email` name this mailbox (addresses match
    /// whatever their case)
    pub fn is(&self, provider: MailProvider, email: &str) -> bool {
        self.provider == provider && self.email.trim().eq_ignore_ascii_case(email.trim())
    }

    /// `<provider>.<hash>`: a stable name for keys, without the address
    /// itself (keyring user names and `local_settings` keys can be listed)
    fn key(&self) -> String {
        let digest = Sha256::digest(self.email.trim().to_ascii_lowercase().as_bytes());
        let hash: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        format!("{}.{hash}", self.provider.id())
    }

    /// The keyring entry of its sign-in
    pub fn token_entry(&self) -> String {
        format!("nandi.email.{}.oauth", self.key())
    }

    /// `local_settings` keys of its change cursor, its last check and its
    /// background read
    pub fn cursor_key(&self) -> String {
        format!("email.{}.cursor", self.key())
    }

    pub fn state_key(&self) -> String {
        format!("email.{}.state", self.key())
    }

    pub fn backfill_key(&self) -> String {
        format!("email.{}.backfill", self.key())
    }

    /// The keyring entry of a sign-in its files or calendar have apart
    /// from the account's ([`super::settings::ReadSignIn::Apart`])
    pub fn read_token_entry(&self, family: AccountFamily) -> String {
        format!("nandi.email.{}.{}.oauth", self.key(), family.id())
    }

    /// `local_settings` key of its files' or calendar's `what` (`cursor`,
    /// `state`, `seen`): `<files|calendar>.<provider>.<hash>.<what>`
    pub fn read_key(&self, family: AccountFamily, what: &str) -> String {
        format!("{}.{}.{what}", family.id(), self.key())
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
    pub google_calendar: String,
    pub google_drive: String,
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
            google_calendar: "https://www.googleapis.com/calendar/v3".into(),
            google_drive: "https://www.googleapis.com/drive/v3".into(),
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

    /// An account's sign-in, asking for exactly what it `reads`
    pub fn oauth(&self, provider: MailProvider, reads: Reads) -> OAuthProvider {
        let cloud = provider.cloud();
        self.account_oauth(cloud, provider.id(), &reads.scopes(cloud))
    }

    /// The sign-in of one account connector (`id`) with exactly `scopes`:
    /// incremental consent, so calendar and files scopes are asked for only
    /// when the person connects them
    pub fn account_oauth(&self, cloud: Cloud, id: &str, scope_list: &[&str]) -> OAuthProvider {
        let scopes = scope_list.iter().map(|s| (*s).to_string()).collect();
        let login = &self.endpoints.microsoft_login;
        match cloud {
            Cloud::Microsoft => OAuthProvider {
                id: id.into(),
                name: "Microsoft".into(),
                authorize_url: Some(format!("{login}/authorize")),
                token_url: format!("{login}/token"),
                // Microsoft allows the device flow with mail scopes: the
                // fallback when the redirect server can't start
                device_url: Some(format!("{login}/devicecode")),
                client_id: self.microsoft_client_id.clone(),
                client_secret: None,
                scopes,
                auth_params: vec![("prompt".into(), "select_account".into())],
                redirect: LoopbackRedirect::LOCALHOST,
            },
            Cloud::Google => OAuthProvider {
                id: id.into(),
                name: "Google".into(),
                authorize_url: Some(self.endpoints.google_authorize.clone()),
                token_url: self.endpoints.google_token.clone(),
                // Google's device flow doesn't allow Gmail scopes
                device_url: None,
                client_id: self.google_client_id.clone(),
                client_secret: self.google_client_secret.clone(),
                scopes,
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
    fn each_mailbox_has_its_own_keys_without_its_address() {
        let work = MailboxId::new(MailProvider::Gmail, " Dev@Gmail.com ");
        assert_eq!(work.email, "dev@gmail.com");
        assert_eq!(work, MailboxId::new(MailProvider::Gmail, "dev@gmail.com"));
        assert!(work.is(MailProvider::Gmail, "DEV@gmail.com"));
        assert!(!work.is(MailProvider::Outlook, "dev@gmail.com"));
        let home = MailboxId::new(MailProvider::Gmail, "dev.home@gmail.com");
        let keys = |m: &MailboxId| {
            [
                m.token_entry(),
                m.cursor_key(),
                m.state_key(),
                m.backfill_key(),
                m.read_token_entry(AccountFamily::Files),
                m.read_key(AccountFamily::Files, "cursor"),
                m.read_key(AccountFamily::Calendar, "cursor"),
            ]
        };
        for (mine, theirs) in keys(&work).iter().zip(keys(&home)) {
            assert_ne!(*mine, theirs);
            assert!(!mine.contains("dev"), "{mine}");
        }
        // Stable: the same address always has the same names
        assert_eq!(work.cursor_key(), "email.gmail.d0f676a9b2ab.cursor");
        assert_eq!(work.token_entry(), "nandi.email.gmail.d0f676a9b2ab.oauth");
        assert_eq!(
            work.read_key(AccountFamily::Files, "cursor"),
            "files.gmail.d0f676a9b2ab.cursor"
        );
        assert_eq!(
            work.read_key(AccountFamily::Calendar, "seen"),
            "calendar.gmail.d0f676a9b2ab.seen"
        );
        assert_eq!(
            work.read_token_entry(AccountFamily::Calendar),
            "nandi.email.gmail.d0f676a9b2ab.calendar.oauth"
        );
    }

    #[test]
    fn scopes_are_read_only() {
        let setup = ProviderSetup::default();
        let all = Reads {
            mail: true,
            files: true,
            calendar: true,
        };
        for provider in MailProvider::ALL {
            for scope in setup.oauth(provider, all).scopes {
                let lower = scope.to_ascii_lowercase();
                assert!(
                    !lower.contains("send")
                        && !lower.contains("readwrite")
                        && !lower.contains("modify")
                        && !lower.ends_with("/drive")
                        && !lower.ends_with("/calendar"),
                    "{scope}"
                );
            }
        }
        assert!(setup
            .oauth(MailProvider::Gmail, Reads::MAIL)
            .scopes
            .contains(&"https://www.googleapis.com/auth/gmail.readonly".to_string()));
        assert!(setup
            .oauth(MailProvider::Outlook, Reads::MAIL)
            .scopes
            .contains(&"Mail.Read".to_string()));
    }

    #[test]
    fn a_sign_in_asks_only_for_what_the_person_ticked() {
        let setup = ProviderSetup::default();
        let mail_only = setup.oauth(MailProvider::Outlook, Reads::MAIL).scopes;
        assert!(!mail_only
            .iter()
            .any(|s| s.starts_with("Files") || s.starts_with("Calendars")));
        let with_files = setup
            .oauth(
                MailProvider::Outlook,
                Reads::MAIL.with(AccountFamily::Files, true),
            )
            .scopes;
        assert!(with_files.contains(&"Files.Read.All".to_string()));
        assert!(!with_files.contains(&"Calendars.Read".to_string()));
        // A drive on its own: no mail at all
        let drive = Reads::only(AccountFamily::Files).scopes(Cloud::Google);
        assert_eq!(
            drive,
            [
                "openid",
                "email",
                "profile",
                "https://www.googleapis.com/auth/drive.readonly"
            ]
        );
        let calendar = Reads::only(AccountFamily::Calendar).scopes(Cloud::Microsoft);
        assert!(calendar.contains(&"Calendars.Read") && !calendar.contains(&"Mail.Read"));
    }

    #[test]
    fn reads_cover_what_they_include() {
        let all = Reads {
            mail: true,
            files: true,
            calendar: true,
        };
        assert!(all.covers(Reads::MAIL));
        assert!(!Reads::MAIL.covers(all));
        assert!(Reads::MAIL.covers(Reads::default()));
        assert!(Reads::default().is_empty());
        assert!(Reads::only(AccountFamily::Files).has(AccountFamily::Files));
        assert!(!Reads::only(AccountFamily::Files).has(AccountFamily::Calendar));
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
            setup.oauth(MailProvider::Outlook, Reads::MAIL).redirect,
            LoopbackRedirect::LOCALHOST
        );
        assert!(setup
            .oauth(MailProvider::Outlook, Reads::MAIL)
            .authorize_url
            .unwrap()
            .contains("/common/"));
        assert_eq!(
            setup.oauth(MailProvider::Gmail, Reads::MAIL).redirect,
            LoopbackRedirect::CALLBACK
        );
    }
}
