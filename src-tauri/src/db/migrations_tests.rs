use std::collections::BTreeMap;

use rusqlite::types::Value;
use sha2::{Digest, Sha256};

use super::*;

/// The SHA-256 of every shipped migration file. A migration that shipped
/// never changes: add the next one, and its checksum here.
const SHIPPED: &[(&str, &str)] = &[(
    "001_baseline.sql",
    "800374d023d39a296641f51e8a22a9b649ad09d2dc89803ef8e3aac946f4e1c3",
)];

const BASELINE_ROWS: &str = include_str!("migrations/fixtures/baseline_rows.sql");

/// Test-only steps after the baseline, the way a real one would look
const ADD_A_COLUMN: Migration = Migration {
    version: 2,
    file: "002_runs_label.sql",
    sql: "ALTER TABLE runs ADD COLUMN label TEXT NOT NULL DEFAULT '';",
};
const REBUILD_CONVERSATIONS: Migration = Migration {
    version: 3,
    file: "003_rebuild_conversations.sql",
    sql: "CREATE TABLE conversations_new (
              id         TEXT PRIMARY KEY,
              agent_id   TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              title      TEXT,
              pinned     INTEGER NOT NULL DEFAULT 0
          );
          INSERT INTO conversations_new (id, agent_id, created_at, title)
              SELECT id, agent_id, created_at, title FROM conversations;
          DROP TABLE conversations;
          ALTER TABLE conversations_new RENAME TO conversations;
          CREATE INDEX conversations_agent ON conversations(agent_id);",
};

fn chain(steps: &[Migration]) -> Vec<Migration> {
    [MIGRATIONS[0]]
        .into_iter()
        .chain(steps.iter().copied())
        .collect()
}

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

/// Every table, index and trigger, with its SQL
fn schema(conn: &Connection) -> Vec<(String, String, Option<String>)> {
    let mut stmt = conn
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

type Rows = BTreeMap<String, Vec<BTreeMap<String, Value>>>;

/// Every row of every table that has rows, by column name
fn rows(conn: &Connection) -> Rows {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut all = Rows::new();
    for table in tables.into_iter().filter(|t| t != "sqlite_sequence") {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
            .unwrap();
        let columns: Vec<String> = stmt.column_names().into_iter().map(Into::into).collect();
        let rows: Vec<BTreeMap<String, Value>> = stmt
            .query_map([], |row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, column)| Ok((column.clone(), row.get::<_, Value>(i)?)))
                    .collect::<rusqlite::Result<_>>()
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        if !rows.is_empty() {
            all.insert(table, rows);
        }
    }
    all
}

/// Each row before is still there after, with every column it had
fn assert_rows_survived(before: &Rows, after: &Rows) {
    for (table, rows) in before {
        let kept = after
            .get(table)
            .unwrap_or_else(|| panic!("{table} lost its rows"));
        assert_eq!(kept.len(), rows.len(), "rows in {table}");
        for (old, new) in rows.iter().zip(kept) {
            for (column, value) in old {
                assert_eq!(new.get(column), Some(value), "{table}.{column}");
            }
        }
    }
}

/// A baseline database file holding the fixture rows, as a person's
/// would be. WAL mode, like the app's.
fn baseline_file(dir: &Path) -> (Connection, PathBuf) {
    let path = dir.join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    upgrade(&conn, &MIGRATIONS[..1], Some(&path)).unwrap();
    conn.execute_batch(BASELINE_ROWS).unwrap();
    (conn, path)
}

#[test]
fn versions_count_from_one_without_gaps() {
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        assert_eq!(migration.version, index as i64 + 1, "{}", migration.file);
        let number = format!("{:03}_", migration.version);
        assert!(migration.file.starts_with(&number), "{}", migration.file);
    }
    assert_eq!(SCHEMA_VERSION, MIGRATIONS.len() as i64);
}

#[test]
fn shipped_migrations_never_change() {
    let embedded: Vec<(&str, String)> = MIGRATIONS
        .iter()
        .map(|m| (m.file, format!("{:x}", Sha256::digest(m.sql.as_bytes()))))
        .collect();
    let recorded: Vec<(&str, String)> = SHIPPED
        .iter()
        .map(|(file, sum)| (*file, sum.to_string()))
        .collect();
    assert_eq!(
        embedded, recorded,
        "a shipped migration changed, or a new one has no checksum in SHIPPED"
    );
}

#[test]
fn every_migration_file_is_in_the_chain() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db/migrations");
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    files.sort();
    let chained: Vec<&str> = MIGRATIONS.iter().map(|m| m.file).collect();
    assert_eq!(files, chained);
}

#[test]
fn a_fresh_database_and_an_upgraded_one_have_the_same_schema() {
    for migrations in [
        MIGRATIONS.to_vec(),
        chain(&[ADD_A_COLUMN, REBUILD_CONVERSATIONS]),
    ] {
        let fresh = Connection::open_in_memory().unwrap();
        upgrade(&fresh, &migrations, None).unwrap();
        // Stopped after each step, as a person on that version would be
        for stop in 1..=migrations.len() {
            let upgraded = Connection::open_in_memory().unwrap();
            upgrade(&upgraded, &migrations[..stop], None).unwrap();
            upgrade(&upgraded, &migrations, None).unwrap();
            assert_eq!(schema(&upgraded), schema(&fresh), "stopped at {stop}");
            assert_eq!(user_version(&upgraded), migrations.len() as i64);
        }
    }
}

#[test]
fn every_baseline_row_survives_the_migration_chain() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = baseline_file(dir.path());
    let before = rows(&conn);
    for table in [
        "runs",
        "messages",
        "approvals",
        "intake_queue",
        "tasks",
        "delegations",
    ] {
        assert!(before.contains_key(table), "the fixture has {table} rows");
    }
    // The shipped chain, then one with a table rebuilt under a cascade
    upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert_rows_survived(&before, &rows(&conn));
    upgrade(
        &conn,
        &chain(&[ADD_A_COLUMN, REBUILD_CONVERSATIONS]),
        Some(&path),
    )
    .unwrap();
    assert_rows_survived(&before, &rows(&conn));
}

#[test]
fn an_upgrade_keeps_a_copy_of_the_database_first() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = baseline_file(dir.path());
    let before = rows(&conn);

    let done = upgrade(&conn, &chain(&[ADD_A_COLUMN]), Some(&path)).unwrap();
    let backup = dir.path().join("state.db.bak-v1");
    assert_eq!(
        done,
        Upgrade::Upgraded {
            from: 1,
            to: 2,
            backup: Some(backup.clone())
        }
    );
    assert_eq!(user_version(&conn), 2);
    // The copy is the old version with every committed row (none left in the WAL)
    let copy = Connection::open(&backup).unwrap();
    assert_eq!(user_version(&copy), 1);
    assert_eq!(rows(&copy), before);
    // Up to date now: nothing more to do
    let again = upgrade(&conn, &chain(&[ADD_A_COLUMN]), Some(&path)).unwrap();
    assert_eq!(again, Upgrade::UpToDate);
}

#[test]
fn a_new_database_gets_the_whole_schema_and_no_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = Connection::open(&path).unwrap();
    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert_eq!(done, Upgrade::Created { to: SCHEMA_VERSION });
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_failed_migration_rolls_every_step_back() {
    let broken_sql = Migration {
        version: 3,
        file: "003_broken.sql",
        sql: "ALTER TABLE no_such_table ADD COLUMN x TEXT;",
    };
    // Deletes a conversation its messages and runs still point to
    let breaks_a_key = Migration {
        version: 3,
        file: "003_breaks_a_key.sql",
        sql: "DELETE FROM conversations WHERE id = 'conv-agent';",
    };
    let cases = [
        (broken_sql, "migration 003_broken.sql failed"),
        (breaks_a_key, "lost the row they refer to"),
    ];
    for (failing, reason) in cases {
        let dir = tempfile::tempdir().unwrap();
        let (conn, path) = baseline_file(dir.path());
        let before = rows(&conn);
        let schema_before = schema(&conn);

        let error = upgrade(&conn, &chain(&[ADD_A_COLUMN, failing]), Some(&path)).unwrap_err();
        assert!(error.contains("from version 1 to 3"), "{error}");
        assert!(error.contains(reason), "{error}");
        assert!(error.contains("Nothing was changed"), "{error}");
        assert!(error.contains("state.db.bak-v1"), "{error}");
        // Step 002 went back too
        assert_eq!(user_version(&conn), 1);
        assert_eq!(schema(&conn), schema_before);
        assert_eq!(rows(&conn), before);
        let keys_on: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(keys_on, "foreign keys are back on");
    }
}

#[test]
fn a_database_from_a_newer_nandi_is_refused_untouched() {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    let error = upgrade(&conn, MIGRATIONS, None).unwrap_err();
    assert!(error.contains("newer version of Nandi"), "{error}");
    assert!(error.contains("Nothing was changed"), "{error}");
    assert!(schema(&conn).is_empty());
    assert_eq!(user_version(&conn), SCHEMA_VERSION + 1);
}
