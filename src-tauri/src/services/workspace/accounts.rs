//! The account connectors beside mail: Outlook calendar, Google Calendar,
//! OneDrive and Google Drive. Each has its own sign-in with only its own
//! read-only scopes (incremental consent: calendar and files scopes are
//! asked for only when the person connects them), kept in the keyring as
//! `nandi.<id>.oauth`.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::email::{Cloud, ProviderSetup};

/// What kind of data an account holds
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AccountFamily {
    Calendar,
    Files,
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

/// Read-only Graph calendars, and who the user is
const OUTLOOK_CALENDAR_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "User.Read",
    "Calendars.Read",
];
/// Read-only OneDrive: the person's files and files shared with them
const ONEDRIVE_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "User.Read",
    "Files.Read",
    "Files.Read.All",
];
const GOOGLE_CALENDAR_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/calendar.readonly",
];
const GOOGLE_DRIVE_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/drive.readonly",
];

impl AccountKind {
    pub const ALL: [Self; 4] = [
        Self::OutlookCalendar,
        Self::GoogleCalendar,
        Self::OneDrive,
        Self::GoogleDrive,
    ];

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

    pub fn cloud(self) -> Cloud {
        match self {
            Self::OutlookCalendar | Self::OneDrive => Cloud::Microsoft,
            Self::GoogleCalendar | Self::GoogleDrive => Cloud::Google,
        }
    }

    pub fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::OutlookCalendar => OUTLOOK_CALENDAR_SCOPES,
            Self::GoogleCalendar => GOOGLE_CALENDAR_SCOPES,
            Self::OneDrive => ONEDRIVE_SCOPES,
            Self::GoogleDrive => GOOGLE_DRIVE_SCOPES,
        }
    }

    pub fn of_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
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

    pub fn token_entry(self) -> String {
        format!("nandi.{}.oauth", self.id())
    }

    /// `local_settings` key of its change cursor
    pub fn cursor_key(self) -> String {
        format!("workspace.{}.cursor", self.id())
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
    fn each_account_asks_only_for_its_own_read_scopes() {
        for kind in AccountKind::ALL {
            let scopes = kind.scopes();
            assert!(scopes.iter().all(|s| !s.contains("Mail")), "{kind:?}");
            assert!(
                scopes.iter().all(|s| !s.contains("ReadWrite")
                    && !s.ends_with("/calendar")
                    && !s.ends_with("/drive")),
                "{kind:?} is read-only"
            );
        }
        assert!(AccountKind::OutlookCalendar
            .scopes()
            .contains(&"Calendars.Read"));
        assert!(AccountKind::OneDrive.scopes().contains(&"Files.Read.All"));
        assert!(AccountKind::GoogleCalendar
            .scopes()
            .contains(&"https://www.googleapis.com/auth/calendar.readonly"));
        assert!(AccountKind::GoogleDrive
            .scopes()
            .contains(&"https://www.googleapis.com/auth/drive.readonly"));
    }

    #[test]
    fn ids_round_trip_and_name_their_items() {
        for kind in AccountKind::ALL {
            assert_eq!(AccountKind::of_id(kind.id()), Some(kind));
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
