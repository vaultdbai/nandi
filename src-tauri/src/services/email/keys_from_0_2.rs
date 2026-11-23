//! Moves what Nandi 0.2 kept per provider to the mailbox's own names, so a
//! person who connected Outlook or Gmail stays connected after the update
//! and no mail is read twice.
//!
//! 0.2 held at most one mailbox per provider, so its keys named only the
//! provider: the keyring entry `nandi.email.<provider>.oauth` and the
//! `local_settings` keys `email.<provider>.cursor`, `.state` and
//! `.backfill`. Each moves to the name of the account `config.json` holds
//! for that provider ([`MailboxId`]).
//!
//! It runs at startup, before anything reads a mailbox. Each value is
//! written under its new name before the old one is deleted, so a start
//! that stops halfway moves the rest the next time, and a second run finds
//! nothing to move.

use std::path::Path;

use rusqlite::Connection;

use super::provider::{MailProvider, MailboxId};
use crate::db;
use crate::services::secrets::Secrets;
use crate::utils::nandi_config;

/// What each mailbox's keys were called in Nandi 0.2
const SETTINGS_0_2: [&str; 3] = ["cursor", "state", "backfill"];

fn token_entry_0_2(provider: MailProvider) -> String {
    format!("nandi.email.{}.oauth", provider.id())
}

fn setting_0_2(provider: MailProvider, what: &str) -> String {
    format!("email.{}.{what}", provider.id())
}

fn new_setting(mailbox: &MailboxId, what: &str) -> String {
    match what {
        "cursor" => mailbox.cursor_key(),
        "state" => mailbox.state_key(),
        _ => mailbox.backfill_key(),
    }
}

/// Moves the 0.2 keys of each connected mailbox in `nandi_dir`'s
/// `config.json` to its own names. Returns how many it moved. A keyring
/// that can't be written leaves its entry for the next start; the
/// database's keys move all or none.
pub fn upgrade(nandi_dir: &Path, conn: &Connection, secrets: &Secrets) -> Result<usize, String> {
    let Some(settings) = nandi_config::load_from(nandi_dir)?.email else {
        return Ok(0);
    };
    let mailboxes: Vec<MailboxId> = MailProvider::ALL
        .into_iter()
        .filter_map(|provider| settings.first_of(provider).map(|a| a.id()))
        .collect();
    let mut moved = 0;
    let mut problems = Vec::new();
    for mailbox in &mailboxes {
        match move_sign_in(mailbox, secrets) {
            Ok(true) => moved += 1,
            Ok(false) => {}
            Err(e) => problems.push(e),
        }
    }
    match move_settings(&mailboxes, conn) {
        Ok(count) => moved += count,
        Err(e) => problems.push(e.to_string()),
    }
    if moved > 0 {
        log::info!("Moved {moved} email keys to their mailbox's own names");
    }
    if problems.is_empty() {
        Ok(moved)
    } else {
        Err(format!(
            "Couldn't move every email key to its mailbox's name: {}",
            problems.join("; ")
        ))
    }
}

/// The sign-in, from the provider's entry to the mailbox's
fn move_sign_in(mailbox: &MailboxId, secrets: &Secrets) -> Result<bool, String> {
    let old = token_entry_0_2(mailbox.provider);
    let Some(token) = secrets.read(&old)? else {
        return Ok(false);
    };
    secrets.set(&mailbox.token_entry(), &token)?;
    secrets.delete(&old)?;
    Ok(true)
}

/// The cursor, last check and background read, in one transaction
fn move_settings(mailboxes: &[MailboxId], conn: &Connection) -> rusqlite::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut moved = 0;
    for mailbox in mailboxes {
        for what in SETTINGS_0_2 {
            let old = setting_0_2(mailbox.provider, what);
            if let Some(value) = db::get_setting(&tx, &old)? {
                db::set_setting(&tx, &new_setting(mailbox, what), &value)?;
                db::delete_setting(&tx, &old)?;
                moved += 1;
            }
        }
    }
    tx.commit()?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::services::email::settings::{EmailAccount, EmailSettings};

    fn account(provider: MailProvider, email: &str) -> EmailAccount {
        EmailAccount::mailbox(provider, email)
    }

    /// A `config.json` with 0.2's two mailboxes, and their keys under 0.2's
    /// names
    fn nandi_0_2(dir: &Path, conn: &Connection, secrets: &Secrets) {
        nandi_config::update_in(dir, |config| {
            let email = config.email.get_or_insert_with(EmailSettings::default);
            email.accounts = vec![
                account(MailProvider::Outlook, "Dev@Company.com"),
                account(MailProvider::Gmail, "dev@gmail.com"),
            ];
        })
        .unwrap();
        secrets
            .set("nandi.email.outlook.oauth", r#"{"access_token":"ms"}"#)
            .unwrap();
        secrets
            .set("nandi.email.gmail.oauth", r#"{"access_token":"g"}"#)
            .unwrap();
        db::set_setting(conn, "email.outlook.cursor", &json!({"inbox": "d-1"})).unwrap();
        db::set_setting(conn, "email.outlook.state", &json!({"lastFound": 2})).unwrap();
        db::set_setting(conn, "email.gmail.cursor", &json!("120")).unwrap();
        db::set_setting(conn, "email.gmail.backfill", &json!({"read": 60})).unwrap();
    }

    fn setting(conn: &Connection, key: &str) -> Option<Value> {
        db::get_setting(conn, key).unwrap()
    }

    #[test]
    fn a_0_2_mailbox_keeps_its_sign_in_and_cursor_under_its_own_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        nandi_0_2(dir.path(), &conn, &secrets);

        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 6);

        let outlook = MailboxId::new(MailProvider::Outlook, "dev@company.com");
        let gmail = MailboxId::new(MailProvider::Gmail, "dev@gmail.com");
        assert_eq!(
            secrets.get(&outlook.token_entry()).unwrap().as_deref(),
            Some(r#"{"access_token":"ms"}"#)
        );
        assert_eq!(
            secrets.get(&gmail.token_entry()).unwrap().as_deref(),
            Some(r#"{"access_token":"g"}"#)
        );
        assert_eq!(secrets.get("nandi.email.outlook.oauth").unwrap(), None);
        assert_eq!(secrets.get("nandi.email.gmail.oauth").unwrap(), None);
        assert_eq!(
            setting(&conn, &outlook.cursor_key()),
            Some(json!({"inbox": "d-1"}))
        );
        assert_eq!(
            setting(&conn, &outlook.state_key()),
            Some(json!({"lastFound": 2}))
        );
        assert_eq!(setting(&conn, &gmail.cursor_key()), Some(json!("120")));
        assert_eq!(
            setting(&conn, &gmail.backfill_key()),
            Some(json!({"read": 60}))
        );
        for old in [
            "email.outlook.cursor",
            "email.outlook.state",
            "email.gmail.cursor",
            "email.gmail.backfill",
        ] {
            assert_eq!(setting(&conn, old), None, "{old}");
        }

        // The next start finds nothing to move and changes nothing
        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 0);
        assert_eq!(setting(&conn, &gmail.cursor_key()), Some(json!("120")));
    }

    #[test]
    fn a_start_that_stopped_halfway_moves_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        nandi_0_2(dir.path(), &conn, &secrets);
        // Stopped after writing the new entry, before deleting the old one
        let gmail = MailboxId::new(MailProvider::Gmail, "dev@gmail.com");
        secrets
            .set(&gmail.token_entry(), r#"{"access_token":"g"}"#)
            .unwrap();

        upgrade(dir.path(), &conn, &secrets).unwrap();

        assert_eq!(
            secrets.get(&gmail.token_entry()).unwrap().as_deref(),
            Some(r#"{"access_token":"g"}"#)
        );
        assert_eq!(secrets.get("nandi.email.gmail.oauth").unwrap(), None);
    }

    #[test]
    fn nothing_moves_without_a_connected_mailbox() {
        let dir = tempfile::tempdir().unwrap();
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        db::set_setting(&conn, "email.gmail.cursor", &json!("120")).unwrap();

        assert_eq!(upgrade(dir.path(), &conn, &secrets).unwrap(), 0);
        assert_eq!(setting(&conn, "email.gmail.cursor"), Some(json!("120")));
    }
}
