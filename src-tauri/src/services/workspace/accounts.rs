//! What an account reads beside mail: Outlook calendar, Google Calendar,
//! OneDrive and Google Drive. They read through the account's own sign-in
//! (`email::Reads` says which read-only scopes it asked for), so each of
//! several accounts has its own drive and calendar.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::email::{MailProvider, ProviderSetup};

/// What kind of data an account holds beside mail
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AccountFamily {
    Calendar,
    Files,
}

impl AccountFamily {
    pub const ALL: [Self; 2] = [Self::Files, Self::Calendar];

    /// The family's part of key names: `files`, `calendar`
    pub fn id(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::Files => "files",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AccountKind {
    /// Outlook and Microsoft 365 calendars, through Graph (`Calendars.Read`)
    OutlookCalendar,
    /// `calendar.readonly`
    GoogleCalendar,
    /// OneDrive and files shared with the person (`Files.Read`, `Files.Read.All`)
    OneDrive,
    /// `drive.readonly`
    GoogleDrive,
}

impl AccountKind {
    pub const ALL: [Self; 4] = [
        Self::OutlookCalendar,
        Self::GoogleCalendar,
        Self::OneDrive,
        Self::GoogleDrive,
    ];

    /// The files or calendar of a `provider` account
    pub fn of(provider: MailProvider, family: AccountFamily) -> Self {
        match (provider, family) {
            (MailProvider::Outlook, AccountFamily::Calendar) => Self::OutlookCalendar,
            (MailProvider::Gmail, AccountFamily::Calendar) => Self::GoogleCalendar,
            (MailProvider::Outlook, AccountFamily::Files) => Self::OneDrive,
            (MailProvider::Gmail, AccountFamily::Files) => Self::GoogleDrive,
        }
    }

    /// The connector ID grants and skill.md use
    pub fn id(self) -> &'static str {
        match self {
            Self::OutlookCalendar => "outlook-calendar",
            Self::GoogleCalendar => "google-calendar",
            Self::OneDrive => "onedrive",
            Self::GoogleDrive => "google-drive",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::OutlookCalendar => "Outlook calendar",
            Self::GoogleCalendar => "Google Calendar",
            Self::OneDrive => "OneDrive",
            Self::GoogleDrive => "Google Drive",
        }
    }

    pub fn family(self) -> AccountFamily {
        match self {
            Self::OutlookCalendar | Self::GoogleCalendar => AccountFamily::Calendar,
            Self::OneDrive | Self::GoogleDrive => AccountFamily::Files,
        }
    }

    /// The account provider whose sign-in it reads with
    pub fn provider(self) -> MailProvider {
        match self {
            Self::OutlookCalendar | Self::OneDrive => MailProvider::Outlook,
            Self::GoogleCalendar | Self::GoogleDrive => MailProvider::Gmail,
        }
    }

    /// The account whose item IDs start `<id>:`
    pub fn of_item_id(item: &str) -> Option<(Self, &str)> {
        Self::ALL.into_iter().find_map(|kind| {
            item.strip_prefix(kind.id())
                .and_then(|rest| rest.strip_prefix(':'))
                .filter(|rest| !rest.is_empty())
                .map(|rest| (kind, rest))
        })
    }

    /// The only API addresses its token is sent to
    pub fn api_bases(self, setup: &ProviderSetup) -> Vec<String> {
        let e = &setup.endpoints;
        match self {
            Self::OutlookCalendar | Self::OneDrive => vec![e.graph.clone()],
            Self::GoogleCalendar => vec![e.google_calendar.clone(), e.google_userinfo.clone()],
            Self::GoogleDrive => vec![e.google_drive.clone(), e.google_userinfo.clone()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_provider_has_one_drive_and_one_calendar() {
        for kind in AccountKind::ALL {
            assert_eq!(AccountKind::of(kind.provider(), kind.family()), kind);
        }
        assert_eq!(
            AccountKind::of(MailProvider::Gmail, AccountFamily::Files),
            AccountKind::GoogleDrive
        );
        assert_eq!(AccountKind::OneDrive.provider(), MailProvider::Outlook);
    }

    #[test]
    fn ids_name_their_connectors_and_items() {
        for kind in AccountKind::ALL {
            assert!(crate::services::connectors::is_account_connector(kind.id()));
        }
        assert_eq!(
            AccountKind::of_item_id("google-calendar:abc"),
            Some((AccountKind::GoogleCalendar, "abc"))
        );
        assert_eq!(AccountKind::of_item_id("onedrive:"), None);
        assert_eq!(AccountKind::of_item_id("gmail:abc"), None);
    }
}
