//! Other members' keys, pinned the first time this Nandi sees them.
//!
//! The relay's directory carries every member's public keys (the relay
//! pins them too, and refuses another device under the same ID). This
//! module keeps our own pin, so a relay that swaps a key can't redirect
//! work unnoticed:
//!
//! | The directory shows | What happens |
//! | --- | --- |
//! | Keys for someone new | Pinned |
//! | The pinned keys | Nothing |
//! | Other keys | Kept as the change; sending to them is refused with [`PEER_KEY_CHANGED`] until the user accepts it on their profile |
//! | The pinned keys again, after a change | The change is dropped |
//!
//! Keys also come from the relay when this Nandi first seals for someone
//! or opens something from them (`PublicKeysQuery`), pinned the same way.
//! [`PinnedKeys`] is the relay client's key book: every send to another
//! instance, and every envelope opened, asks it, so a changed key blocks
//! both until the user accepts it. The table is `peer_keys`, a runtime
//! table: nothing in the gold copy.

use nandi_protocol::PublicKeys;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::Db;
use crate::services::relay::protocol::MemberRecord;
use crate::services::relay::sealing::PeerKeyBook;

/// The stable code a "their key changed" refusal starts with
pub const PEER_KEY_CHANGED: &str = "peer_key_changed";

/// A member's key, as the profile shows it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PeerKeyStatus {
    /// The pinned key's short fingerprint (`3F2A 91C0 7B44 E1D9`)
    pub fingerprint: String,
    /// The fingerprint of other keys the relay now shows for them; sending
    /// waits until the user accepts it
    pub changed_fingerprint: Option<String>,
}

impl PeerKeyStatus {
    pub fn changed(&self) -> bool {
        self.changed_fingerprint.is_some()
    }
}

/// What seeing a member's keys did
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    /// No keys were pinned for them; these are now
    Pinned,
    Same,
    /// Other keys than the pinned ones (kept as the change)
    Changed,
    /// The pinned keys again: a change seen earlier is dropped
    Restored,
}

struct Row {
    pinned: PublicKeys,
    changed: Option<PublicKeys>,
}

fn read(conn: &rusqlite::Connection, instance_id: &str) -> Result<Option<Row>, String> {
    conn.query_row(
        "SELECT signing_key, sealing_key, changed_signing_key, changed_sealing_key
         FROM peer_keys WHERE instance_id = ?1",
        params![instance_id],
        |row| {
            Ok(Row {
                pinned: PublicKeys {
                    signing: row.get(0)?,
                    sealing: row.get(1)?,
                },
                changed: row
                    .get::<_, Option<String>>(2)?
                    .zip(row.get::<_, Option<String>>(3)?)
                    .map(|(signing, sealing)| PublicKeys { signing, sealing }),
            })
        },
    )
    .optional()
    .map_err(|e| format!("Failed to read the keys of {instance_id}: {e}"))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Pins `keys` for `instance_id` on first sight, or notes that they changed
pub fn observe(db: &Db, instance_id: &str, keys: &PublicKeys) -> Result<Sight, String> {
    if !keys.is_well_formed() {
        return Err(format!("The relay sent malformed keys for {instance_id}"));
    }
    let conn = db.conn()?;
    let sight = match read(&conn, instance_id)? {
        None => {
            conn.execute(
                "INSERT INTO peer_keys (instance_id, signing_key, sealing_key, pinned_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![instance_id, keys.signing, keys.sealing, now()],
            )
            .map_err(|e| e.to_string())?;
            Sight::Pinned
        }
        Some(row) if &row.pinned == keys => {
            if row.changed.is_none() {
                return Ok(Sight::Same);
            }
            set_changed(&conn, instance_id, None)?;
            Sight::Restored
        }
        Some(row) => {
            if row.changed.as_ref() != Some(keys) {
                set_changed(&conn, instance_id, Some(keys))?;
            }
            Sight::Changed
        }
    };
    Ok(sight)
}

fn set_changed(
    conn: &rusqlite::Connection,
    instance_id: &str,
    keys: Option<&PublicKeys>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE peer_keys SET changed_signing_key = ?2, changed_sealing_key = ?3,
           changed_at = CASE WHEN ?2 IS NULL THEN NULL ELSE ?4 END
         WHERE instance_id = ?1",
        params![
            instance_id,
            keys.map(|k| k.signing.as_str()),
            keys.map(|k| k.sealing.as_str()),
            now()
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Takes every key the directory lists (ourselves left out); the members
/// whose key changed
pub fn observe_directory(db: &Db, records: &[MemberRecord], local_instance: &str) -> Vec<String> {
    let mut changed = Vec::new();
    for record in records {
        let id = &record.card.instance_id;
        let Some(keys) = &record.public_keys else {
            continue;
        };
        if id.is_empty() || id == local_instance {
            continue;
        }
        match observe(db, id, keys) {
            Ok(Sight::Changed) => changed.push(id.clone()),
            Ok(_) => {}
            Err(e) => log::warn!("Couldn't check the keys of {id}: {e}"),
        }
    }
    changed
}

/// The key we pinned for `instance_id`, if any
pub fn status(db: &Db, instance_id: &str) -> Result<Option<PeerKeyStatus>, String> {
    let conn = db.conn()?;
    let Some(row) = read(&conn, instance_id)? else {
        return Ok(None);
    };
    let fingerprint = |keys: &PublicKeys| {
        nandi_seal::fingerprint(keys).map_err(|e| format!("A stored key is malformed: {e}"))
    };
    Ok(Some(PeerKeyStatus {
        fingerprint: fingerprint(&row.pinned)?,
        changed_fingerprint: row.changed.as_ref().map(fingerprint).transpose()?,
    }))
}

/// The user accepts `instance_id`'s new key: it's pinned, and sending to
/// them works again
pub fn accept(db: &Db, instance_id: &str) -> Result<PeerKeyStatus, String> {
    {
        let conn = db.conn()?;
        let changed = conn
            .execute(
                "UPDATE peer_keys SET signing_key = changed_signing_key,
                   sealing_key = changed_sealing_key, pinned_at = ?2,
                   changed_signing_key = NULL, changed_sealing_key = NULL, changed_at = NULL
                 WHERE instance_id = ?1 AND changed_signing_key IS NOT NULL",
                params![instance_id, now()],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("{instance_id}'s key hasn't changed"));
        }
    }
    log::info!("Accepted the new key of {instance_id}");
    status(db, instance_id)?.ok_or_else(|| format!("No key for {instance_id}"))
}

/// The refusal while `instance_id`'s key changed and the user hasn't
/// accepted it
pub fn changed(instance_id: &str) -> String {
    format!(
        "{PEER_KEY_CHANGED}: {instance_id}'s key changed. Check the new key on their profile and accept it to send to them again"
    )
}

/// The keys pinned for `instance_id`; `Err` ([`PEER_KEY_CHANGED`]) while a
/// change waits for the user
pub fn pinned(db: &Db, instance_id: &str) -> Result<Option<PublicKeys>, String> {
    let conn = db.conn()?;
    match read(&conn, instance_id)? {
        Some(row) if row.changed.is_some() => Err(changed(instance_id)),
        row => Ok(row.map(|row| row.pinned)),
    }
}

/// The relay lists `keys` for `instance_id`: pinned on first sight; the
/// pinned keys, or `Err` ([`PEER_KEY_CHANGED`]) when they aren't them
pub fn learn(db: &Db, instance_id: &str, keys: &PublicKeys) -> Result<PublicKeys, String> {
    match observe(db, instance_id, keys)? {
        Sight::Changed => Err(changed(instance_id)),
        Sight::Pinned | Sight::Same | Sight::Restored => Ok(keys.clone()),
    }
}

/// The relay client's key book over `peer_keys`
pub struct PinnedKeys {
    db: Db,
}

impl PinnedKeys {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

impl PeerKeyBook for PinnedKeys {
    fn pinned(&self, instance: &str) -> Result<Option<PublicKeys>, String> {
        pinned(&self.db, instance)
    }

    fn learn(&self, instance: &str, keys: &PublicKeys) -> Result<PublicKeys, String> {
        learn(&self.db, instance, keys)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Real keys, the same for the same `tag` within a test run
    pub(crate) fn keys(tag: char) -> PublicKeys {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static KEYS: OnceLock<Mutex<HashMap<char, PublicKeys>>> = OnceLock::new();
        KEYS.get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .entry(tag)
            .or_insert_with(|| {
                nandi_seal::DeviceKeys::generate()
                    .unwrap()
                    .public_keys()
                    .clone()
            })
            .clone()
    }

    #[test]
    fn keys_pin_on_first_sight() {
        let db = Db::in_memory();
        assert_eq!(status(&db, "bob").unwrap(), None);
        assert_eq!(pinned(&db, "bob").unwrap(), None);
        assert_eq!(observe(&db, "bob", &keys('a')).unwrap(), Sight::Pinned);
        assert_eq!(observe(&db, "bob", &keys('a')).unwrap(), Sight::Same);
        let pinned = status(&db, "bob").unwrap().unwrap();
        assert_eq!(pinned.fingerprint.len(), 19);
        assert!(!pinned.changed());
        assert_eq!(super::pinned(&db, "bob").unwrap(), Some(keys('a')));
    }

    #[test]
    fn keys_learned_from_the_relay_pin_like_the_directory_s() {
        let db = Db::in_memory();
        let book = PinnedKeys::new(db.clone());
        assert_eq!(book.learn("bob", &keys('a')).unwrap(), keys('a'));
        assert_eq!(book.pinned("bob").unwrap(), Some(keys('a')));
        assert_eq!(book.learn("bob", &keys('a')).unwrap(), keys('a'));
        let refused = book.learn("bob", &keys('b')).unwrap_err();
        assert!(refused.starts_with(PEER_KEY_CHANGED), "{refused}");
        // The change waits on the profile, and blocks sending
        assert!(status(&db, "bob").unwrap().unwrap().changed());
        assert!(book
            .pinned("bob")
            .unwrap_err()
            .starts_with(PEER_KEY_CHANGED));
    }

    #[test]
    fn a_changed_key_blocks_sending_until_accepted() {
        let db = Db::in_memory();
        observe(&db, "bob", &keys('a')).unwrap();
        let before = status(&db, "bob").unwrap().unwrap().fingerprint;
        assert_eq!(observe(&db, "bob", &keys('b')).unwrap(), Sight::Changed);
        assert_eq!(observe(&db, "bob", &keys('b')).unwrap(), Sight::Changed);
        let changed = status(&db, "bob").unwrap().unwrap();
        assert_eq!(changed.fingerprint, before);
        assert!(changed.changed());
        assert_ne!(
            changed.changed_fingerprint.as_deref(),
            Some(before.as_str())
        );
        let refused = pinned(&db, "bob").unwrap_err();
        assert!(refused.starts_with(PEER_KEY_CHANGED), "{refused}");

        let accepted = accept(&db, "bob").unwrap();
        assert_eq!(
            Some(accepted.fingerprint.clone()),
            changed.changed_fingerprint
        );
        assert!(!accepted.changed());
        assert_eq!(pinned(&db, "bob").unwrap(), Some(keys('b')));
        assert_eq!(observe(&db, "bob", &keys('b')).unwrap(), Sight::Same);
        // Nothing to accept now
        assert!(accept(&db, "bob").is_err());
        assert!(accept(&db, "nobody").is_err());
    }

    #[test]
    fn the_pinned_key_again_drops_the_change() {
        let db = Db::in_memory();
        observe(&db, "bob", &keys('a')).unwrap();
        observe(&db, "bob", &keys('b')).unwrap();
        assert_eq!(observe(&db, "bob", &keys('a')).unwrap(), Sight::Restored);
        assert!(!status(&db, "bob").unwrap().unwrap().changed());
        assert_eq!(pinned(&db, "bob").unwrap(), Some(keys('a')));
    }

    #[test]
    fn malformed_keys_are_ignored() {
        let db = Db::in_memory();
        let mut bad = keys('a');
        bad.signing = "x".into();
        assert!(observe(&db, "bob", &bad).is_err());
        assert_eq!(status(&db, "bob").unwrap(), None);
    }
}
