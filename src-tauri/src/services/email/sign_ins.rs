//! Sign-ins in progress, on the record, so none outlives an exit.
//!
//! Each `start_email_sign_in` gets its own keyring entry
//! (`nandi.email.<provider>.sign-in.<attempt>`, [`MailProvider::sign_in_entry`]),
//! held in memory by `EmailService.sign_ins` while it runs. The OS keyring
//! can't list its entries, so the entry's name is also kept in
//! `local_settings` (`email.sign-ins.<entry>`) from the moment the attempt
//! starts until it ends, connected or not.
//!
//! If Nandi exits in between, the name stays behind. At the next start,
//! before anything can begin a new attempt, [`clean_abandoned`] deletes
//! each such entry from the keyring and then its record, so an interrupted
//! clean-up finishes the time after. It never deletes anything that isn't
//! a sign-in entry.

use rusqlite::Connection;
use serde_json::json;

use super::provider::MailProvider;
use crate::db;
use crate::services::secrets::Secrets;
use crate::services::AppState;

const PREFIX: &str = "email.sign-ins.";

fn key(entry: &str) -> String {
    format!("{PREFIX}{entry}")
}

/// Whether `entry` is a sign-in attempt's keyring entry, and nothing else
fn is_sign_in_entry(entry: &str) -> bool {
    MailProvider::ALL.into_iter().any(|provider| {
        entry
            .strip_prefix(&provider.sign_in_entry(""))
            .is_some_and(|attempt| {
                !attempt.is_empty()
                    && attempt
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    })
}

/// An attempt is starting: its entry is on the record before any token
/// can land in it
pub async fn begun(state: &AppState, entry: &str) -> Result<(), String> {
    let key = key(entry);
    state
        .db()
        .call(move |conn| db::set_setting(conn, &key, &json!(true)))
        .await
        .map_err(|e| format!("Couldn't start the sign-in: {e}"))
}

/// An attempt ended and its entry is gone (moved to the mailbox, or
/// deleted): it comes off the record
pub async fn ended(state: &AppState, entry: &str) {
    let key = key(entry);
    let result = state
        .db()
        .call(move |conn| db::delete_setting(conn, &key))
        .await;
    if let Err(e) = result {
        log::warn!("Couldn't clear a finished sign-in's record: {e}");
    }
}

/// At start, before any attempt: deletes every sign-in entry an earlier
/// run left behind, keyring first. Returns how many went
pub fn clean_abandoned(conn: &Connection, secrets: &Secrets) -> Result<usize, String> {
    let keys = db::setting_keys_with_prefix(conn, PREFIX).map_err(|e| e.to_string())?;
    let mut cleaned = 0;
    for key in keys {
        let entry = &key[PREFIX.len()..];
        if is_sign_in_entry(entry) {
            secrets.delete(entry)?;
            cleaned += 1;
        } else {
            log::warn!("Not a sign-in entry, left alone: {entry}");
        }
        db::delete_setting(conn, &key).map_err(|e| e.to_string())?;
    }
    if cleaned > 0 {
        log::info!("Removed {cleaned} unfinished email sign-in(s) from the keyring");
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sign_in_entries_are_ever_deleted() {
        assert!(is_sign_in_entry(
            &MailProvider::Gmail.sign_in_entry("6f1c2a9e-0d4b-4c1e-9a7f-3b2d1e0f9a8c")
        ));
        assert!(is_sign_in_entry("nandi.email.outlook.sign-in.test"));
        for other in [
            "nandi.email.outlook.sign-in.",
            "nandi.email.gmail.1a2b3c4d5e6f.oauth",
            "nandi.llm.anthropic",
            "nandi.email.gmail.sign-in.x.y",
        ] {
            assert!(!is_sign_in_entry(other), "{other}");
        }
    }

    #[test]
    fn an_attempt_left_by_an_exit_is_removed_at_the_next_start() {
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        let left = MailProvider::Gmail.sign_in_entry("left-behind");
        let done = MailProvider::Outlook.sign_in_entry("finished");
        let mailbox = "nandi.email.gmail.1a2b3c4d5e6f.oauth";
        for entry in [&left, &done, &mailbox.to_string()] {
            secrets.set(entry, r#"{"access_token":"t"}"#).unwrap();
        }
        // An attempt that started and never ended, and one that ended
        db::set_setting(&conn, &key(&left), &json!(true)).unwrap();
        db::set_setting(&conn, &key(&done), &json!(true)).unwrap();
        db::delete_setting(&conn, &key(&done)).unwrap();
        // A record that doesn't name a sign-in entry
        db::set_setting(&conn, &key(mailbox), &json!(true)).unwrap();

        assert_eq!(clean_abandoned(&conn, &secrets).unwrap(), 1);
        assert_eq!(secrets.get(&left).unwrap(), None);
        // Only what was on the record goes; a mailbox's sign-in never does
        assert!(secrets.get(&done).unwrap().is_some());
        assert!(secrets.get(mailbox).unwrap().is_some());
        assert!(db::setting_keys_with_prefix(&conn, PREFIX)
            .unwrap()
            .is_empty());
        // A second start finds nothing
        assert_eq!(clean_abandoned(&conn, &secrets).unwrap(), 0);
    }

    #[test]
    fn a_split_sign_in_goes_with_all_its_pieces() {
        let store = db::Db::in_memory();
        let conn = store.conn().unwrap();
        let secrets = Secrets::in_memory();
        let left = MailProvider::Outlook.sign_in_entry("big");
        // A personal Microsoft sign-in is several KB: kept in pieces
        secrets.set(&left, &"x".repeat(4000)).unwrap();
        db::set_setting(&conn, &key(&left), &json!(true)).unwrap();
        clean_abandoned(&conn, &secrets).unwrap();
        assert_eq!(secrets.get(&left).unwrap(), None);
        assert_eq!(secrets.get(&format!("{left}#1")).unwrap(), None);
    }
}
