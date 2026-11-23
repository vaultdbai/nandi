//! What the email feature keeps, outside the repo (it belongs to this
//! computer, like the sign-in in its keyring):
//!
//! | What                                | Where                                         |
//! | ----------------------------------- | --------------------------------------------- |
//! | Sign-in tokens                      | keyring `nandi.email.<provider>.<hash>.oauth` |
//! | Accounts, what each reads, interval | `~/.nandi/config.json` `email`                |
//! | Which agent may read which mailbox  | grants (`services::grants`)                   |
//! | Change cursor, last check, backfill | `local_settings` `email.<provider>.<hash>.*`  |
//! | Its files' and calendar's cursors   | `local_settings` `files.<provider>.<hash>.*`, `calendar.<provider>.<hash>.*` |
//!
//! `<hash>` names the mailbox without its address ([`MailboxId`]), so a
//! person can connect several of each provider. No mail is kept anywhere.
//! An account can also read its files and calendar (`services::workspace`),
//! on the same sign-in.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::profile::Profile;
use super::provider::{MailProvider, MailboxId, Reads};
use crate::model::Timestamp;
use crate::services::workspace::AccountFamily;

pub const DEFAULT_INTERVAL_MINUTES: u32 = 30;
pub const MIN_INTERVAL_MINUTES: u32 = 1;
pub const MAX_INTERVAL_MINUTES: u32 = 240;
/// Most watch-list entries
pub const MAX_WATCH: usize = 100;

fn default_interval() -> u32 {
    DEFAULT_INTERVAL_MINUTES
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailSettings {
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
    /// Senders or domains that always come through, even if they look
    /// like bulk mail
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watch: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<EmailAccount>,
    /// How far back the background read of older mail goes
    #[serde(default)]
    pub backfill: BackfillRange,
}

/// How much older mail the background read covers
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BackfillRange {
    #[default]
    ThreeMonths,
    Year,
    Everything,
    Off,
}

impl BackfillRange {
    /// The oldest time it reads back to (unix seconds); `None`: no limit
    pub fn floor(self, now: i64) -> Option<i64> {
        const DAY: i64 = 24 * 60 * 60;
        match self {
            Self::ThreeMonths => Some(now - 91 * DAY),
            Self::Year => Some(now - 365 * DAY),
            Self::Everything | Self::Off => None,
        }
    }
}

/// One mailbox's background read of older mail (`local_settings`
/// `email.<provider>.<hash>.backfill`), apart from the change cursor
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillState {
    /// Unix seconds: everything newer has been read; the next batch is
    /// older than this. `None` until the first batch
    #[serde(default)]
    pub before: Option<i64>,
    /// Emails handed to Nandi so far
    #[serde(default)]
    pub read: u32,
    #[serde(default)]
    pub paused: bool,
    /// Reached the start of the mailbox or the range, or stopped
    #[serde(default)]
    pub done: bool,
}

impl Default for EmailSettings {
    fn default() -> Self {
        Self {
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            watch: Vec::new(),
            accounts: Vec::new(),
            backfill: BackfillRange::default(),
        }
    }
}

impl EmailSettings {
    pub fn account(&self, mailbox: &MailboxId) -> Option<&EmailAccount> {
        self.accounts.iter().find(|a| a.is(mailbox))
    }

    pub fn account_mut(&mut self, mailbox: &MailboxId) -> Option<&mut EmailAccount> {
        self.accounts.iter_mut().find(|a| a.is(mailbox))
    }

    /// The accounts whose mail Nandi reads
    pub fn mailboxes(&self) -> impl Iterator<Item = &EmailAccount> {
        self.accounts.iter().filter(|a| a.mail)
    }

    /// The first-connected mailbox of `provider`
    pub fn first_of(&self, provider: MailProvider) -> Option<&EmailAccount> {
        self.mailboxes().find(|a| a.provider == provider)
    }
}

/// Whose sign-in an account's files or calendar use
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadSignIn {
    /// The account's sign-in asked for it
    Account,
    /// A sign-in of its own, at keyring
    /// [`MailboxId::read_token_entry`]: a calendar or drive connected on
    /// its own before 0.3 (`services::workspace::keys_from_0_2`). The
    /// account's next sign-in that asks for it takes it over.
    Apart,
}

fn yes() -> bool {
    true
}

fn is_yes(on: &bool) -> bool {
    *on
}

/// A connected account (no secrets): its mail, and its files and calendar
/// when the person ticked them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAccount {
    pub provider: MailProvider,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Title, department and manager, when the mailbox gave them
    #[serde(default, skip_serializing_if = "Profile::is_empty")]
    pub profile: Profile,
    #[serde(default)]
    pub paused: bool,
    /// Nandi reads its mail. Off only for a calendar or drive whose address
    /// had no mailbox when it moved under its account (0.2's own connections)
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub mail: bool,
    /// Its OneDrive or Google Drive
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<ReadSignIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<ReadSignIn>,
}

impl EmailAccount {
    /// A new account that reads mail only
    pub fn mailbox(provider: MailProvider, email: &str) -> Self {
        Self {
            provider,
            email: email.to_string(),
            name: None,
            profile: Profile::default(),
            paused: false,
            mail: true,
            files: None,
            calendar: None,
        }
    }

    pub fn id(&self) -> MailboxId {
        MailboxId::new(self.provider, &self.email)
    }

    pub fn is(&self, mailbox: &MailboxId) -> bool {
        mailbox.is(self.provider, &self.email)
    }

    /// Its files' or calendar's sign-in, while Nandi reads them
    pub fn read(&self, family: AccountFamily) -> Option<ReadSignIn> {
        match family {
            AccountFamily::Files => self.files,
            AccountFamily::Calendar => self.calendar,
        }
    }

    pub fn set_read(&mut self, family: AccountFamily, sign_in: Option<ReadSignIn>) {
        match family {
            AccountFamily::Files => self.files = sign_in,
            AccountFamily::Calendar => self.calendar = sign_in,
        }
    }

    /// What the account's own sign-in asked for: the scopes it renews with
    pub fn reads(&self) -> Reads {
        Reads {
            mail: self.mail,
            files: self.files == Some(ReadSignIn::Account),
            calendar: self.calendar == Some(ReadSignIn::Account),
        }
    }

    /// Everything Nandi reads of it, whichever sign-in
    pub fn wants(&self) -> Reads {
        Reads {
            mail: self.mail,
            files: self.files.is_some(),
            calendar: self.calendar.is_some(),
        }
    }

    /// The keyring entry and scopes its files or calendar are read with
    pub fn read_sign_in(&self, family: AccountFamily) -> Option<(String, Reads)> {
        match self.read(family)? {
            ReadSignIn::Account => Some((self.id().token_entry(), self.reads())),
            ReadSignIn::Apart => Some((self.id().read_token_entry(family), Reads::only(family))),
        }
    }
}

/// Checks the interval and watch list a client sends
pub fn validate(interval_minutes: u32, watch: &[String]) -> Result<Vec<String>, String> {
    if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&interval_minutes) {
        return Err(format!(
            "Check every {MIN_INTERVAL_MINUTES} to {MAX_INTERVAL_MINUTES} minutes"
        ));
    }
    let mut clean: Vec<String> = Vec::new();
    for entry in watch {
        let entry = entry.trim().trim_start_matches('@').to_ascii_lowercase();
        if entry.is_empty() {
            continue;
        }
        let valid = entry.len() <= 254
            && !entry.contains(char::is_whitespace)
            && entry.contains('.')
            && entry.matches('@').count() <= 1;
        if !valid {
            return Err(format!(
                "'{entry}' isn't an email address or a domain (like contoso.com)"
            ));
        }
        if !clean.contains(&entry) {
            clean.push(entry);
        }
    }
    if clean.len() > MAX_WATCH {
        return Err(format!("Watch at most {MAX_WATCH} senders or domains"));
    }
    Ok(clean)
}

/// One mailbox's last check, machine-local runtime state
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckState {
    #[serde(default)]
    pub last_checked_at: Option<Timestamp>,
    /// Unix seconds: mail up to here has been handed over
    #[serde(default)]
    pub caught_up_to: Option<i64>,
    /// Emails handed to Nandi by the last check
    #[serde(default)]
    pub last_found: u32,
    #[serde(default)]
    pub last_run_id: Option<String>,
    #[serde(default)]
    pub needs_sign_in: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_interval_and_watch_list() {
        assert_eq!(
            validate(
                5,
                &[
                    " @Vendor.com ".into(),
                    "ceo@board.org".into(),
                    "vendor.com".into()
                ]
            )
            .unwrap(),
            ["vendor.com", "ceo@board.org"]
        );
        assert!(validate(0, &[]).is_err());
        assert!(validate(241, &[]).is_err());
        assert!(validate(5, &["not a domain".into()]).is_err());
        assert!(validate(5, &["localhost".into()]).is_err());
    }

    #[test]
    fn an_account_reads_mail_and_only_what_was_ticked() {
        let old: EmailAccount =
            serde_json::from_str(r#"{"provider":"gmail","email":"dev@gmail.com"}"#).unwrap();
        assert!(old.mail);
        assert_eq!(old.wants(), Reads::MAIL);
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            serde_json::json!({"provider": "gmail", "email": "dev@gmail.com", "paused": false})
        );
        let mut drive = old.clone();
        drive.set_read(AccountFamily::Files, Some(ReadSignIn::Apart));
        drive.set_read(AccountFamily::Calendar, Some(ReadSignIn::Account));
        // The account's sign-in renews with what it asked for; the drive
        // connected apart keeps its own sign-in and scopes
        assert_eq!(
            drive.reads(),
            Reads::MAIL.with(AccountFamily::Calendar, true)
        );
        assert!(drive.wants().files);
        assert_eq!(
            drive.read_sign_in(AccountFamily::Files),
            Some((
                drive.id().read_token_entry(AccountFamily::Files),
                Reads::only(AccountFamily::Files)
            ))
        );
        assert_eq!(
            drive.read_sign_in(AccountFamily::Calendar).unwrap().0,
            drive.id().token_entry()
        );
        let mut settings = EmailSettings::default();
        let mut no_mail = EmailAccount::mailbox(MailProvider::Gmail, "x@gmail.com");
        no_mail.mail = false;
        settings.accounts = vec![no_mail, old];
        assert_eq!(
            settings.first_of(MailProvider::Gmail).unwrap().email,
            "dev@gmail.com",
            "an account without mail isn't a mailbox"
        );
    }

    #[test]
    fn old_files_without_email_settings_read_as_defaults() {
        let settings: EmailSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, EmailSettings::default());
    }
}
