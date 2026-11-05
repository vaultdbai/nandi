//! What the email feature keeps, outside the repo (it belongs to this
//! computer, like the sign-in in its keyring):
//!
//! | What                                   | Where                                   |
//! | -------------------------------------- | --------------------------------------- |
//! | Sign-in tokens                         | keyring `nandi.email.<provider>.oauth`  |
//! | Accounts, interval, watch list         | `~/.nandi/config.json` `email`          |
//! | Which agent may read which mailbox     | `~/.nandi/config.json` `connector_grants` |
//! | Change cursor, last check              | `local_settings` (`email.<provider>.*`) |
//!
//! No mail is kept anywhere.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::provider::MailProvider;
use crate::model::Timestamp;

pub const DEFAULT_INTERVAL_MINUTES: u32 = 5;
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
}

impl Default for EmailSettings {
    fn default() -> Self {
        Self {
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            watch: Vec::new(),
            accounts: Vec::new(),
        }
    }
}

impl EmailSettings {
    pub fn account(&self, provider: MailProvider) -> Option<&EmailAccount> {
        self.accounts.iter().find(|a| a.provider == provider)
    }

    pub fn account_mut(&mut self, provider: MailProvider) -> Option<&mut EmailAccount> {
        self.accounts.iter_mut().find(|a| a.provider == provider)
    }
}

/// A connected mailbox (no secrets)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAccount {
    pub provider: MailProvider,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub paused: bool,
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
    /// Emails handed to the Chief of Staff by the last check
    #[serde(default)]
    pub last_found: u32,
    #[serde(default)]
    pub last_run_id: Option<String>,
    #[serde(default)]
    pub needs_sign_in: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Read or write access, kept apart (Wave 2 adds write grants)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum GrantAccess {
    Read,
}

/// "This agent may use this connector": per agent and connector, kept
/// until the user removes it (`config.json`, this computer only)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorGrant {
    pub agent: String,
    /// `outlook` or `gmail`
    pub connector: String,
    pub access: GrantAccess,
}

pub fn allows(
    grants: &[ConnectorGrant],
    agent: &str,
    connector: &str,
    access: GrantAccess,
) -> bool {
    grants
        .iter()
        .any(|g| g.agent == agent && g.connector == connector && g.access == access)
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
    fn grants_are_per_agent_and_connector() {
        let grants = [ConnectorGrant {
            agent: "chief-of-staff".into(),
            connector: "outlook".into(),
            access: GrantAccess::Read,
        }];
        assert!(allows(
            &grants,
            "chief-of-staff",
            "outlook",
            GrantAccess::Read
        ));
        assert!(!allows(
            &grants,
            "chief-of-staff",
            "gmail",
            GrantAccess::Read
        ));
        assert!(!allows(
            &grants,
            "pr-reviewer",
            "outlook",
            GrantAccess::Read
        ));
    }

    #[test]
    fn old_files_without_email_settings_read_as_defaults() {
        let settings: EmailSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, EmailSettings::default());
    }
}
