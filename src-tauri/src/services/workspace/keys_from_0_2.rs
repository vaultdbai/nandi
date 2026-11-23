//! Moves the calendars and drives Nandi 0.2 connected on their own under
//! the account with the same address, so each account holds its own mail,
//! files and calendar and nobody signs in again.
//!
//! 0.2 kept them apart, one per kind: `config.json` `workspace.accounts`
//! (`[{kind, email, name}]`), the keyring entry `nandi.<kind>.oauth` and the
//! `local_settings` keys `workspace.<kind>.cursor`, `.state` and `.seen`.
//! Each moves to the account `email.accounts` holds for the same provider
//! and address:
//!
//! | 0.2                              | Now                                                           |
//! | -------------------------------- | ------------------------------------------------------------- |
//! | `workspace.accounts[]`           | `email.accounts[].files` / `.calendar`                        |
//! | keyring `nandi.<kind>.oauth`     | keyring `nandi.email.<provider>.<hash>.<files\|calendar>.oauth` |
//! | `workspace.<kind>.<what>`        | `<files\|calendar>.<provider>.<hash>.<what>`                    |
//!
//! - **A mailbox with that address:** it reads the drive or calendar on the
//!   sign-in it had ([`ReadSignIn::Apart`]): two tokens can't be merged into
//!   one. The account's next sign-in that asks for it takes it over.
//! - **No mailbox with that address:** it becomes an account that reads no
//!   mail (it never asked for mail, and nothing starts reading mail the
//!   person didn't connect). Its first drive or calendar is the account's
//!   own sign-in; a second one keeps its own.
//!
//! It runs at startup, before anything reads a calendar or drive. Each token
//! is written under its new name before the old one is deleted, the
//! database's keys move in one transaction, and `config.json` changes last,
//! in one write that adds the reads and drops `workspace.accounts`. A start
//! that stops halfway moves the rest the next time; a second run finds
//! nothing to move.

use std::path::Path;

use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;

use super::{AccountKind, Source};
use crate::db;
use crate::services::email::settings::{EmailAccount, ReadSignIn};
use crate::services::email::{EmailSettings, MailboxId};
use crate::services::secrets::Secrets;
use crate::utils::nandi_config;

/// What each calendar's or drive's keys were called in Nandi 0.2
const SETTINGS_0_2: [&str; 3] = ["cursor", "state", "seen"];

/// One of 0.2's `workspace.accounts`
#[derive(Debug, Clone, Deserialize)]
struct Account0_2 {
    kind: AccountKind,
    email: String,
    #[serde(default)]
    name: Option<String>,
}

/// One calendar's or drive's keys: from its 0.2 names to its account's
#[derive(Debug, Clone, PartialEq, Eq)]
struct Move {
    token: (String, String),
    settings: Vec<(String, String)>,
}

/// 0.2's `workspace.accounts`, read from the file itself: the settings
/// Nandi has now don't hold them
fn accounts_0_2(nandi_dir: &Path) -> Result<Vec<Account0_2>, String> {
    let path = nandi_dir.join("config.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
    };
    let config: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    match config.pointer("/workspace/accounts") {
        Some(accounts) => serde_json::from_value(accounts.clone())
            .map_err(|e| format!("Couldn't read the calendars and drives of Nandi 0.2: {e}")),
        None => Ok(Vec::new()),
    }
}

/// Puts each 0.2 calendar or drive under its account in `email` and says
/// which keys move where
fn plan(email: &mut EmailSettings, old: &[Account0_2]) -> Vec<Move> {
    let mut moves = Vec::new();
    for connection in old {
        let kind = connection.kind;
        let family = kind.family();
        let mailbox = MailboxId::new(kind.provider(), &connection.email);
        if email.account(&mailbox).is_none() {
            let mut account = EmailAccount::mailbox(mailbox.provider, &mailbox.email);
            account.name = connection.name.clone();
            account.mail = false;
            email.accounts.push(account);
        }
        let Some(account) = email.account_mut(&mailbox) else {
            continue;
        };
        if account.read(family).is_some() {
            continue;
        }
        // An account without mail signs in as its first calendar or drive
        let sign_in = if account.reads().is_empty() {
            ReadSignIn::Account
        } else {
            ReadSignIn::Apart
        };
        account.set_read(family, Some(sign_in));
        let Some(source) = Source::of(account, family) else {
            continue;
        };
        moves.push(Move {
            token: (format!("nandi.{}.oauth", kind.id()), source.entry.clone()),
            settings: SETTINGS_0_2
                .iter()
                .map(|what| {
                    (
                        format!("workspace.{}.{what}", kind.id()),
                        mailbox.read_key(family, what),
                    )
                })
                .collect(),
        });
    }
    moves
}

/// Moves 0.2's calendars and drives in `nandi_dir`'s `config.json` under
/// their accounts. Returns how many keys it moved. A keyring that can't be
/// written leaves everything for the next start; the database's keys move
/// all or none.
pub fn upgrade(nandi_dir: &Path, conn: &Connection, secrets: &Secrets) -> Result<usize, String> {
    let old = accounts_0_2(nandi_dir)?;
    if old.is_empty() {
        return Ok(0);
    }
    let mut email = nandi_config::load_from(nandi_dir)?
        .email
        .unwrap_or_default();
    let moves = plan(&mut email, &old);
    let mut moved = 0;
    let mut problems = Vec::new();
    for one in &moves {
        match move_token(&one.token.0, &one.token.1, secrets) {
            Ok(true) => moved += 1,
            Ok(false) => {}
            Err(e) => problems.push(e),
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "Couldn't move the calendar and drive sign-ins to their accounts: {}",
            problems.join("; ")
        ));
    }
    moved += move_settings(&moves, conn).map_err(|e| {
        format!("Couldn't move the calendar and drive cursors to their accounts: {e}")
    })?;
    // Last: the accounts read them, and `workspace.accounts` is gone
    nandi_config::update_in(nandi_dir, |config| config.email = Some(email))?;
    log::info!(
        "Moved {} calendars and drives under their accounts ({moved} keys)",
        old.len()
    );
    Ok(moved)
}

/// A sign-in, from its 0.2 entry to its account's. A missing old entry was
/// moved already (or never saved): nothing to do
fn move_token(old: &str, new: &str, secrets: &Secrets) -> Result<bool, String> {
    let Some(token) = secrets.read(old)? else {
        return Ok(false);
    };
    secrets.set(new, &token)?;
    secrets.delete(old)?;
    Ok(true)
}

/// The cursors, last checks and events seen, in one transaction
fn move_settings(moves: &[Move], conn: &Connection) -> rusqlite::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut moved = 0;
    for (old, new) in moves.iter().flat_map(|m| &m.settings) {
        if let Some(value) = db::get_setting(&tx, old)? {
            db::set_setting(&tx, new, &value)?;
            db::delete_setting(&tx, old)?;
            moved += 1;
        }
    }
    tx.commit()?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::services::email::{MailProvider, Reads};
    use crate::services::workspace::{sources_of, AccountFamily};

    /// A `config.json` as Nandi 0.2 left it: an Outlook mailbox, its
    /// calendar and OneDrive connected apart, and a Google Drive and
    /// Calendar whose address has no mailbox
    fn nandi_0_2(dir: &Path, conn: &Connection, secrets: &Secrets) {
        let config = json!({
            "email": {
                "interval_minutes": 30,
                "accounts": [{"provider": "outlook", "email": "dev@contoso.com", "name": "Dev"}]
            },
            "workspace": {
                "accounts": [
                    {"kind": "outlookCalendar", "email": "Dev@Contoso.com", "name": "Dev"},
                    {"kind": "oneDrive", "email": "dev@contoso.com"},
                    {"kind": "googleDrive", "email": "dev@gmail.com", "name": "Dev G"},
                    {"kind": "googleCalendar", "email": "dev@gmail.com"}
                ],
                "calendarIntervalMinutes": 120
            },
            "theme": "dark"
        });
        std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
        for kind in [
            "outlook-calendar",
            "onedrive",
            "google-drive",
            "google-calendar",
        ] {
            secrets
                .set(
                    &format!("nandi.{kind}.oauth"),
                    &format!("{{\"access_token\":\"{kind}\"}}"),
                )
                .unwrap();
        }
        secrets
            .set("nandi.email.outlook.0.oauth", "the mailbox's own")
            .unwrap();
        db::set_setting(conn, "workspace.outlook-calendar.cursor", &json!("d-1")).unwrap();
        db::set_setting(conn, "workspace.outlook-calendar.seen", &json!({"e1": "x"})).unwrap();
        db::set_setting(conn, "workspace.onedrive.cursor", &json!("t-1")).unwrap();
        db::set_setting(
            conn,
            "workspace.google-calendar.state",
            &json!({"lastFound": 2}),
        )
        .unwrap();
    }

    fn token(secrets: &Secrets, entry: &str) -> Option<String> {
        secrets.get(entry).unwrap()
    }

    fn setting(conn: &Connection, key: &str) -> Option<Value> {
        db::get_setting(conn, key).unwrap()
    }

    #[test]
    fn calendars_and_drives_move_under_their_accounts_and_keep_their_sign_ins() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        nandi_0_2(dir.path(), &conn, &secrets);

        // 4 sign-ins and 4 database keys
        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 8);

        let config = nandi_config::load_from(dir.path()).unwrap();
        let email = config.email.unwrap();
        let outlook = MailboxId::new(MailProvider::Outlook, "dev@contoso.com");
        let gmail = MailboxId::new(MailProvider::Gmail, "dev@gmail.com");
        // The mailbox reads its calendar and OneDrive on their own sign-ins
        let mailbox = email.account(&outlook).unwrap();
        assert!(mailbox.mail);
        assert_eq!(mailbox.calendar, Some(ReadSignIn::Apart));
        assert_eq!(mailbox.files, Some(ReadSignIn::Apart));
        assert_eq!(
            mailbox.reads(),
            Reads::MAIL,
            "its own sign-in still asks for mail only"
        );
        assert_eq!(
            token(&secrets, &outlook.read_token_entry(AccountFamily::Calendar)).as_deref(),
            Some(r#"{"access_token":"outlook-calendar"}"#)
        );
        assert_eq!(
            token(&secrets, &outlook.read_token_entry(AccountFamily::Files)).as_deref(),
            Some(r#"{"access_token":"onedrive"}"#)
        );
        // The Google address had no mailbox: an account that reads no mail,
        // signed in as its drive, with its calendar's own sign-in beside it
        let google = email.account(&gmail).unwrap();
        assert!(!google.mail);
        assert_eq!(google.name.as_deref(), Some("Dev G"));
        assert_eq!(google.files, Some(ReadSignIn::Account));
        assert_eq!(google.calendar, Some(ReadSignIn::Apart));
        assert_eq!(
            token(&secrets, &gmail.token_entry()).as_deref(),
            Some(r#"{"access_token":"google-drive"}"#)
        );
        assert_eq!(
            token(&secrets, &gmail.read_token_entry(AccountFamily::Calendar)).as_deref(),
            Some(r#"{"access_token":"google-calendar"}"#)
        );
        assert!(
            email.first_of(MailProvider::Gmail).is_none(),
            "no mail is read"
        );
        for kind in [
            "outlook-calendar",
            "onedrive",
            "google-drive",
            "google-calendar",
        ] {
            assert_eq!(
                token(&secrets, &format!("nandi.{kind}.oauth")),
                None,
                "{kind}"
            );
        }
        // Each reads with the sign-in and scopes it was given
        let files = sources_of(&email, AccountFamily::Files);
        assert_eq!(files.len(), 2);
        assert_eq!(
            files[0].entry,
            outlook.read_token_entry(AccountFamily::Files)
        );
        assert_eq!(files[0].reads, Reads::only(AccountFamily::Files));
        assert_eq!(files[1].entry, gmail.token_entry());

        // The cursors moved to each account's names
        assert_eq!(
            setting(&conn, &outlook.read_key(AccountFamily::Calendar, "cursor")),
            Some(json!("d-1"))
        );
        assert_eq!(
            setting(&conn, &outlook.read_key(AccountFamily::Calendar, "seen")),
            Some(json!({"e1": "x"}))
        );
        assert_eq!(
            setting(&conn, &outlook.read_key(AccountFamily::Files, "cursor")),
            Some(json!("t-1"))
        );
        assert_eq!(
            setting(&conn, &gmail.read_key(AccountFamily::Calendar, "state")),
            Some(json!({"lastFound": 2}))
        );
        for old in [
            "workspace.outlook-calendar.cursor",
            "workspace.outlook-calendar.seen",
            "workspace.onedrive.cursor",
            "workspace.google-calendar.state",
        ] {
            assert_eq!(setting(&conn, old), None, "{old}");
        }
        // `workspace.accounts` is gone; the interval and other settings stay
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("config.json")).unwrap())
                .unwrap();
        assert_eq!(raw.pointer("/workspace/accounts"), None);
        assert_eq!(raw["workspace"]["calendarIntervalMinutes"], 120);
        assert_eq!(raw["theme"], "dark");
        assert_eq!(
            token(&secrets, "nandi.email.outlook.0.oauth").as_deref(),
            Some("the mailbox's own")
        );

        // The next start finds nothing to move and changes nothing
        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 0);
        assert_eq!(
            nandi_config::load_from(dir.path()).unwrap().email.unwrap(),
            email
        );
    }

    #[test]
    fn a_start_that_stopped_halfway_moves_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        nandi_0_2(dir.path(), &conn, &secrets);
        // Stopped after one sign-in moved (new written, old deleted) and
        // another was written but its old entry not yet deleted
        let outlook = MailboxId::new(MailProvider::Outlook, "dev@contoso.com");
        let calendar = outlook.read_token_entry(AccountFamily::Calendar);
        secrets
            .set(&calendar, r#"{"access_token":"outlook-calendar"}"#)
            .unwrap();
        secrets.delete("nandi.outlook-calendar.oauth").unwrap();
        let files = outlook.read_token_entry(AccountFamily::Files);
        secrets
            .set(&files, r#"{"access_token":"onedrive"}"#)
            .unwrap();

        upgrade(dir.path(), &conn, &secrets).unwrap();

        assert_eq!(
            token(&secrets, &calendar).as_deref(),
            Some(r#"{"access_token":"outlook-calendar"}"#)
        );
        assert_eq!(
            token(&secrets, &files).as_deref(),
            Some(r#"{"access_token":"onedrive"}"#)
        );
        assert_eq!(token(&secrets, "nandi.onedrive.oauth"), None);
        let email = nandi_config::load_from(dir.path()).unwrap().email.unwrap();
        assert_eq!(
            email.account(&outlook).unwrap().calendar,
            Some(ReadSignIn::Apart)
        );
        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 0);
    }

    #[test]
    fn a_calendar_whose_sign_in_was_lost_still_moves_and_asks_to_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        nandi_0_2(dir.path(), &conn, &secrets);
        secrets.delete("nandi.outlook-calendar.oauth").unwrap();

        upgrade(dir.path(), &conn, &secrets).unwrap();

        let outlook = MailboxId::new(MailProvider::Outlook, "dev@contoso.com");
        let email = nandi_config::load_from(dir.path()).unwrap().email.unwrap();
        assert_eq!(
            email.account(&outlook).unwrap().calendar,
            Some(ReadSignIn::Apart)
        );
        assert_eq!(
            token(&secrets, &outlook.read_token_entry(AccountFamily::Calendar)),
            None
        );
    }

    #[test]
    fn nothing_moves_without_0_2_calendars_or_drives() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 0);
        std::fs::write(
            dir.path().join("config.json"),
            json!({"workspace": {"calendarIntervalMinutes": 60}}).to_string(),
        )
        .unwrap();
        db::set_setting(&conn, "workspace.onedrive.cursor", &json!("t-1")).unwrap();
        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 0);
        assert_eq!(
            setting(&conn, "workspace.onedrive.cursor"),
            Some(json!("t-1"))
        );
    }
}
