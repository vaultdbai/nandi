//! Moves the relay instance token Nandi 0.1.0 kept in `state.db` into the
//! OS keyring, where it lives now ([`RELAY_TOKEN_ENTRY`]), so a person who
//! had joined a relay stays joined after the upgrade.
//!
//! 0.1.0 kept the token in the clear in `relay_state` (key `relay_token`,
//! a JSON string), next to the relay it is for (`relay_token_relay`). That
//! second row has the same key and meaning today, so only the token moves.
//!
//! It runs at startup on the database file, before [`crate::db::Db::open`]
//! upgrades it, so the copy kept for the upgrade (`state.db.bak-v2`) never
//! holds the token. The row is deleted with `secure_delete` and the
//! write-ahead log checkpointed, so no copy of the token is left in either
//! file.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::RELAY_TOKEN_ENTRY;
use crate::services::secrets::Secrets;

/// The `relay_state` key where Nandi 0.1.0 kept the instance token
pub const TOKEN_KEY_0_1_0: &str = "relay_token";

/// Moves a token found in `relay_state` of the database file `db_file`
/// into the keyring, then deletes it from the database. Returns whether it
/// moved one. A keyring that can't be written leaves the row for the next
/// start; a missing file or table means there is nothing to move.
pub fn move_to_keyring(db_file: &Path, secrets: &Secrets) -> Result<bool, String> {
    if !db_file.is_file() {
        return Ok(false);
    }
    let failed = |e: rusqlite::Error| format!("Couldn't move the relay token to the keyring: {e}");
    let conn = Connection::open(db_file).map_err(failed)?;
    let Some(stored) = stored_token(&conn).map_err(failed)? else {
        return Ok(false);
    };
    // Stored as JSON: a quoted string
    let token = serde_json::from_str::<String>(&stored).unwrap_or(stored);
    secrets.set(RELAY_TOKEN_ENTRY, &token)?;
    conn.execute_batch("PRAGMA secure_delete = ON")
        .map_err(failed)?;
    conn.execute("DELETE FROM relay_state WHERE key = ?1", [TOKEN_KEY_0_1_0])
        .map_err(failed)?;
    // Moves the zeroed page into the file and empties the log
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(failed)?;
    log::info!("Moved the relay instance token from state.db to the keyring");
    Ok(true)
}

/// The token row's value, if the database has one
fn stored_token(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let has_table = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'relay_state'")?
        .exists([])?;
    if !has_table {
        return Ok(None);
    }
    conn.query_row(
        "SELECT value FROM relay_state WHERE key = ?1",
        [TOKEN_KEY_0_1_0],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::relay::RELAY_TOKEN_RELAY_KEY;

    const TOKEN: &str = "tok-from-0.1.0-a1b2c3";
    const RELAY: &str = "\"wss://relay.example.test/relay\"";

    /// A state.db file in WAL mode, like the app's, with `relay_state`
    /// holding `rows`
    fn state_db(dir: &Path, rows: &[(&str, String)]) -> std::path::PathBuf {
        let path = dir.join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("CREATE TABLE relay_state (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        for (key, value) in rows {
            conn.execute(
                "INSERT INTO relay_state (key, value) VALUES (?1, ?2)",
                [key, value.as_str()],
            )
            .unwrap();
        }
        path
    }

    fn keys(path: &Path) -> Vec<String> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn
            .prepare("SELECT key FROM relay_state ORDER BY key")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// Whether `text` is anywhere in the database's files
    fn on_disk(path: &Path, text: &str) -> bool {
        let wal = path.with_file_name("state.db-wal");
        [path.to_path_buf(), wal].iter().any(|file| {
            std::fs::read(file)
                .map(|bytes| bytes.windows(text.len()).any(|w| w == text.as_bytes()))
                .unwrap_or(false)
        })
    }

    #[test]
    fn a_0_1_0_token_moves_to_the_keyring_and_leaves_no_trace() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_db(
            dir.path(),
            &[
                (TOKEN_KEY_0_1_0, format!("\"{TOKEN}\"")),
                (RELAY_TOKEN_RELAY_KEY, RELAY.to_string()),
                ("instance_id", "\"inst-me\"".to_string()),
            ],
        );
        assert!(on_disk(&path, TOKEN), "the fixture holds the token");
        let secrets = Secrets::in_memory();

        assert_eq!(move_to_keyring(&path, &secrets), Ok(true));
        assert_eq!(
            secrets.get(RELAY_TOKEN_ENTRY).unwrap().as_deref(),
            Some(TOKEN)
        );
        // The relay it is for stays: the app reconnects without joining
        assert_eq!(keys(&path), ["instance_id", RELAY_TOKEN_RELAY_KEY]);
        assert!(
            !on_disk(&path, TOKEN),
            "no copy left in state.db or its log"
        );
        // Nothing left for the next start
        assert_eq!(move_to_keyring(&path, &secrets), Ok(false));
    }

    /// Makes a case's database in a folder and returns its path
    type MakeDb = fn(&Path) -> std::path::PathBuf;

    #[test]
    fn nothing_to_move() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&str, MakeDb); 3] = [
            ("no database yet", |dir| dir.join("state.db")),
            ("no relay_state table", |dir| {
                let path = dir.join("state.db");
                Connection::open(&path)
                    .unwrap()
                    .execute_batch("CREATE TABLE other (x TEXT)")
                    .unwrap();
                path
            }),
            ("no token row", |dir| {
                state_db(dir, &[("instance_id", "\"inst-me\"".to_string())])
            }),
        ];
        for (case, make) in cases {
            let case_dir = dir.path().join(case.replace(' ', "-"));
            std::fs::create_dir_all(&case_dir).unwrap();
            let path = make(&case_dir);
            let secrets = Secrets::in_memory();
            assert_eq!(move_to_keyring(&path, &secrets), Ok(false), "{case}");
            assert_eq!(secrets.get(RELAY_TOKEN_ENTRY).unwrap(), None, "{case}");
        }
    }
}
