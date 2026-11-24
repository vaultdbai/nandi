use std::collections::BTreeMap;

use rusqlite::types::Value;
use sha2::{Digest, Sha256};

use super::*;
use crate::services::builtin_agents::DEFAULT_AGENT;

/// The SHA-256 of every shipped migration file. A migration that shipped
/// never changes: add the next one, and its checksum here.
const SHIPPED: &[(&str, &str)] = &[
    (
        "002_nandi_0_1_0.sql",
        "2dbbf27ac6af7577478004042985dadf867577e86ce1e2d6a95a5913c03f002a",
    ),
    (
        "006_since_0_1_0.sql",
        "8d4fbe8866f4bcee4e2a9edf271bfe0d75b127773a8fbb35e7727d18c66dcbf9",
    ),
    (
        "007_run_retries.sql",
        "44e5336244bf01effcf19c94dde51b8d0eb03e787a4039dd3d548f8cdf8724cf",
    ),
    (
        "008_todos.sql",
        "ac8509d299913e2c779ceb9df344e8aea75c46fa778e847e1008d1aaa5966ce2",
    ),
    (
        "009_retro.sql",
        "be257c53771a2024ca35797ab4c60ca5ecaaad39eab286f469c984c353283753",
    ),
    (
        "010_own_devices.sql",
        "4db81492e3af13823e3299074599bbc126dd038a3f227cdf4283f245db31782e",
    ),
    (
        "011_agent_md.sql",
        "d36c531c49042a5437f075b92fa9e8ad0f3ccda9582ab721f05a34a126c4a5be",
    ),
];

/// A person's rows as Nandi 0.1.0 wrote them
const V0_1_0_ROWS: &str = include_str!("migrations/fixtures/v0_1_0_rows.sql");

/// The version Nandi 0.1.0 stamped
const V0_1_0: i64 = 2;

// What 006 does to a 0.1.0 database's rows, for the row-by-row check

/// Tables of removed features: dropped with their rows
const REMOVED_TABLES: &[&str] = &[
    "teams",
    "team_members",
    "roster_cache",
    "capability_registries",
    "circuit_breakers",
];
/// Columns 007 adds to every existing run (a first attempt), and 008 to
/// every assignment (work to do, from no to-do source)
fn added_columns() -> [(&'static str, &'static str, Value); 8] {
    [
        ("runs", "retry_of", Value::Null),
        ("runs", "attempt", Value::Integer(1)),
        ("gateway_assignments", "kind", Value::Text("do".into())),
        ("gateway_assignments", "source", Value::Null),
        ("gateway_assignments", "source_ref", Value::Null),
        ("gateway_assignments", "related_ref", Value::Null),
        ("gateway_assignments", "run_id", Value::Null),
        ("gateway_assignments", "assigned_agent", Value::Null),
    ]
}
/// Columns nothing reads: dropped
const REMOVED_COLUMNS: &[(&str, &str)] = &[
    ("capabilities_projection", "registry_origin"),
    ("capability_evidence", "approval_required"),
    ("reassigned_tasks", "new_task_id"),
];
/// Views of the gold copy: emptied, rebuilt from the files at startup
const REBUILT_FROM_THE_GOLD_COPY: &[&str] = &[
    "agents",
    "workflows",
    "triggers",
    "settings",
    "connectors",
    "sync_state",
    "capability_index",
];
/// Columns holding an agent ID: `chief-of-staff` becomes `nandi`
const AGENT_ID_COLUMNS: &[(&str, &str)] = &[
    ("conversations", "agent_id"),
    ("runs", "agent_id"),
    ("agent_queue", "agent_id"),
    ("approvals", "agent_id"),
    ("harness_sessions", "agent_id"),
    ("delegations", "target_agent"),
    ("delegations", "requester_agent"),
    ("tasks", "requester_agent"),
    ("tasks", "provider_agent"),
    ("request_returns", "provider_agent"),
    ("eta_history", "provider_agent"),
    ("capability_evidence", "provider_id"),
    ("capabilities_projection", "owner_id"),
    ("request_steps", "target"),
    ("agent_state", "agent_id"),
    ("agent_memory", "agent_id"),
    ("trust_policies", "provider_agent"),
    ("blocked_members", "agent_id"),
    ("capability_providers", "provider_id"),
    ("capability_aggregates", "provider_id"),
    ("provider_circuits", "provider_id"),
];

/// Test-only steps after the shipped chain, the way a real one would look
const ADD_A_COLUMN: Migration = Migration {
    version: 12,
    file: "012_runs_label.sql",
    sql: "ALTER TABLE runs ADD COLUMN label TEXT NOT NULL DEFAULT '';",
};
const REBUILD_CONVERSATIONS: Migration = Migration {
    version: 13,
    file: "013_rebuild_conversations.sql",
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

/// The shipped chain, then `steps`
fn chain(steps: &[Migration]) -> Vec<Migration> {
    MIGRATIONS.iter().chain(steps).copied().collect()
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

type Row = BTreeMap<String, Value>;
type Rows = BTreeMap<String, Vec<Row>>;

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
        let rows: Vec<Row> = stmt
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

/// `value` in `table.column` after 006: an agent ID is renamed
fn renamed(table: &str, column: &str, value: &Value) -> Value {
    let chief_of_staff = Value::Text("chief-of-staff".into());
    if AGENT_ID_COLUMNS.contains(&(table, column)) && *value == chief_of_staff {
        Value::Text(DEFAULT_AGENT.into())
    } else {
        value.clone()
    }
}

/// The rows a 0.1.0 database should hold after the migrations, worked out
/// from the rows it held before and the lists above
fn expected_after_0_1_0(before: &Rows) -> Rows {
    let kept_tables = before.iter().filter(|(table, _)| {
        !REMOVED_TABLES.contains(&table.as_str())
            && !REBUILT_FROM_THE_GOLD_COPY.contains(&table.as_str())
    });
    kept_tables
        .map(|(table, rows)| {
            let rows = rows
                .iter()
                .map(|row| {
                    let mut row: Row = row
                        .iter()
                        .filter(|(column, _)| {
                            !REMOVED_COLUMNS.contains(&(table.as_str(), column.as_str()))
                        })
                        .map(|(column, value)| (column.clone(), renamed(table, column, value)))
                        .collect();
                    for (added_to, column, value) in added_columns() {
                        if added_to == table {
                            row.insert(column.to_string(), value);
                        }
                    }
                    row
                })
                .collect();
            (table.clone(), rows)
        })
        .collect()
}

/// A database file Nandi 0.1.0 wrote, holding the fixture rows. WAL mode,
/// like the app's.
fn v0_1_0_file(dir: &Path) -> (Connection, PathBuf) {
    let path = dir.join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    upgrade(&conn, &MIGRATIONS[..1], Some(&path)).unwrap();
    assert_eq!(user_version(&conn), V0_1_0);
    conn.execute_batch(V0_1_0_ROWS).unwrap();
    (conn, path)
}

#[test]
fn versions_rise_and_name_their_files() {
    for pair in MIGRATIONS.windows(2) {
        assert!(pair[0].version < pair[1].version, "{}", pair[1].file);
    }
    for migration in MIGRATIONS {
        let number = format!("{:03}_", migration.version);
        assert!(migration.file.starts_with(&number), "{}", migration.file);
    }
    assert_eq!(MIGRATIONS[0].version, V0_1_0, "0.1.0's schema comes first");
    assert_eq!(SCHEMA_VERSION, MIGRATIONS.last().unwrap().version);
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
fn each_version_has_its_standing() {
    let cases = [
        (0, Standing::New),
        (1, Standing::Unreleased),
        (2, Standing::Behind),
        (3, Standing::Unreleased),
        (4, Standing::Unreleased),
        (5, Standing::Unreleased),
        (6, Standing::Behind),
        (7, Standing::Behind),
        (8, Standing::Behind),
        (9, Standing::Behind),
        (10, Standing::Behind),
        (11, Standing::Current),
        (12, Standing::Newer),
    ];
    for (version, expected) in cases {
        assert_eq!(standing(version, MIGRATIONS), expected, "version {version}");
    }
}

#[test]
fn a_fresh_database_and_an_upgraded_one_have_the_same_schema() {
    for migrations in [
        MIGRATIONS.to_vec(),
        chain(&[ADD_A_COLUMN, REBUILD_CONVERSATIONS]),
    ] {
        let latest = migrations.last().unwrap().version;
        let fresh = Connection::open_in_memory().unwrap();
        upgrade(&fresh, &migrations, None).unwrap();
        // Stopped after each step, as a person on that version would be
        for stop in 1..=migrations.len() {
            let upgraded = Connection::open_in_memory().unwrap();
            upgrade(&upgraded, &migrations[..stop], None).unwrap();
            upgrade(&upgraded, &migrations, None).unwrap();
            assert_eq!(schema(&upgraded), schema(&fresh), "stopped at {stop}");
            assert_eq!(user_version(&upgraded), latest);
        }
    }
}

#[test]
fn a_0_1_0_database_upgrades_row_by_row() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    let before = rows(&conn);
    for table in [
        "conversations",
        "runs",
        "approvals",
        "agent_state",
        "request_steps",
        "relay_state",
        "agents",
        "teams",
        "capability_evidence",
    ] {
        assert!(before.contains_key(table), "the fixture has {table} rows");
    }

    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert!(matches!(done, Upgrade::Upgraded { from: V0_1_0, .. }));
    let after = rows(&conn);
    for (table, rows) in &expected_after_0_1_0(&before) {
        assert_eq!(after.get(table), Some(rows), "{table}");
    }
    for table in REMOVED_TABLES.iter().chain(REBUILT_FROM_THE_GOLD_COPY) {
        assert!(!after.contains_key(*table), "{table} has no rows");
    }
}

#[test]
fn no_row_still_names_the_chief_of_staff() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();

    let chief_of_staff = Value::Text("chief-of-staff".into());
    for (table, rows) in rows(&conn) {
        for row in rows {
            for (column, value) in row {
                assert_ne!(value, chief_of_staff, "{table}.{column}");
            }
        }
    }
    // The person's own agent keeps its rows as they were
    let writer: String = conn
        .query_row(
            "SELECT agent_id FROM conversations WHERE id = 'conv-writer'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(writer, "writer");
}

#[test]
fn an_existing_nandi_row_wins_over_the_renamed_one() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    conn.execute(
        "INSERT INTO agent_state (agent_id, status, error, updated_at)
             VALUES (?1, 'running', NULL, 3000)",
        [DEFAULT_AGENT],
    )
    .unwrap();
    upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();

    let states: Vec<(String, String)> = conn
        .prepare("SELECT agent_id, status FROM agent_state ORDER BY agent_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let expected = [
        ("chief-of-staff", "idle"),
        (DEFAULT_AGENT, "running"),
        ("writer", "error"),
    ]
    .map(|(id, status)| (id.to_string(), status.to_string()));
    assert_eq!(states, expected);
}

#[test]
fn every_row_survives_a_later_step() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    let before = rows(&conn);
    // A table rebuilt under a cascade loses nothing
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
    let (conn, path) = v0_1_0_file(dir.path());
    let before = rows(&conn);

    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    let backup = dir.path().join("state.db.bak-v2");
    assert_eq!(
        done,
        Upgrade::Upgraded {
            from: V0_1_0,
            to: SCHEMA_VERSION,
            backup: Some(backup.clone())
        }
    );
    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    // The copy is the old version with every committed row (none left in the WAL)
    let copy = Connection::open(&backup).unwrap();
    assert_eq!(user_version(&copy), V0_1_0);
    assert_eq!(rows(&copy), before);
    // Up to date now: nothing more to do
    let again = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
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
        version: 13,
        file: "013_broken.sql",
        sql: "ALTER TABLE no_such_table ADD COLUMN x TEXT;",
    };
    // Deletes a conversation its messages and runs still point to
    let breaks_a_key = Migration {
        version: 13,
        file: "013_breaks_a_key.sql",
        sql: "DELETE FROM conversations WHERE id = 'conv-cos';",
    };
    let cases = [
        (broken_sql, "migration 013_broken.sql failed"),
        (breaks_a_key, "lost the row they refer to"),
    ];
    for (failing, reason) in cases {
        let dir = tempfile::tempdir().unwrap();
        let (conn, path) = v0_1_0_file(dir.path());
        let before = rows(&conn);
        let schema_before = schema(&conn);

        let error = upgrade(&conn, &chain(&[ADD_A_COLUMN, failing]), Some(&path)).unwrap_err();
        assert!(error.contains("from version 2 to 13"), "{error}");
        assert!(error.contains(reason), "{error}");
        assert!(error.contains("Nothing was changed"), "{error}");
        assert!(error.contains("state.db.bak-v2"), "{error}");
        // 006 to 008 went back too
        assert_eq!(user_version(&conn), V0_1_0);
        assert_eq!(schema(&conn), schema_before);
        assert_eq!(rows(&conn), before);
        let keys_on: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(keys_on, "foreign keys are back on");
    }
}

#[test]
fn a_database_from_an_unreleased_build_is_refused_untouched() {
    for version in [1, 3, 4, 5] {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        let error = upgrade(&conn, MIGRATIONS, None).unwrap_err();
        assert!(error.contains("never released"), "{error}");
        assert!(error.contains("Reset your dev data"), "{error}");
        assert!(!error.contains("newer"), "{error}");
        assert!(schema(&conn).is_empty());
        assert_eq!(user_version(&conn), version);
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

#[test]
fn assignments_from_0_2_0_read_as_work_to_do_and_to_dos_are_one_per_source() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    // Where 0.2.0 with run retries left it, with an assignment from Lena
    let at_007 = MIGRATIONS.iter().position(|m| m.version == 7).unwrap();
    upgrade(&conn, &MIGRATIONS[..=at_007], Some(&path)).unwrap();
    conn.execute(
        "INSERT INTO gateway_assignments (task_id, caller_instance, caller_name, request,
                                          status, created_at)
         VALUES ('g1', 'desktop-lena', 'Lena', 'Send the evidence', 'open', 1)",
        [],
    )
    .unwrap();
    let before = rows(&conn);

    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert!(matches!(
        done,
        Upgrade::Upgraded {
            from: 7,
            to: SCHEMA_VERSION,
            ..
        }
    ));
    assert!(dir.path().join("state.db.bak-v7").exists());
    assert_rows_survived(&before, &rows(&conn));
    let (kind, source): (String, Option<String>) = conn
        .query_row(
            "SELECT kind, source FROM gateway_assignments WHERE task_id = 'g1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((kind.as_str(), source), ("do", None));

    let add = |id: &str, source_ref: Option<&str>| {
        conn.execute(
            "INSERT INTO gateway_assignments (task_id, caller_instance, caller_name, request,
                                              status, created_at, source, source_ref)
             VALUES (?1, '', 'Nandi', 'Reply to the landlord', 'open', 1, 'email', ?2)",
            rusqlite::params![id, source_ref],
        )
    };
    add("t1", Some("msg-1")).unwrap();
    assert!(
        add("t2", Some("msg-1")).is_err(),
        "one to-do per source_ref"
    );
    add("t3", None).unwrap();
    add("t4", None).unwrap();
}

#[test]
fn a_0_2_database_gains_the_retro_proposals_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    // Where 0.2.x with to-dos left it
    let at_008 = MIGRATIONS.iter().position(|m| m.version == 8).unwrap();
    upgrade(&conn, &MIGRATIONS[..=at_008], Some(&path)).unwrap();
    let before = rows(&conn);

    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert!(matches!(
        done,
        Upgrade::Upgraded {
            from: 8,
            to: SCHEMA_VERSION,
            ..
        }
    ));
    assert!(dir.path().join("state.db.bak-v8").exists());
    assert_rows_survived(&before, &rows(&conn));
    conn.execute(
        "INSERT INTO retro_proposals (id, run_id, agent_id, target, path, reason, after, diff,
                                      status, created_at)
         VALUES ('p1', 'r1', 'writer', 'agent', '.nandi/agents/writer/agent.md', 'Shorter',
                 'text', '', 'pending', 1)",
        [],
    )
    .unwrap();
    let runs: String = conn
        .query_row(
            "SELECT runs FROM retro_proposals WHERE id = 'p1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, "[]");
}

#[test]
fn a_0_2_database_gains_the_own_devices_table_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    // Where 0.2.x with the weekly retro left it
    let at_009 = MIGRATIONS.iter().position(|m| m.version == 9).unwrap();
    upgrade(&conn, &MIGRATIONS[..=at_009], Some(&path)).unwrap();
    let before = rows(&conn);

    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert!(matches!(
        done,
        Upgrade::Upgraded {
            from: 9,
            to: SCHEMA_VERSION,
            ..
        }
    ));
    assert!(dir.path().join("state.db.bak-v9").exists());
    assert_rows_survived(&before, &rows(&conn));
    conn.execute(
        "INSERT INTO own_devices (device_id, name, state, created_at)
         VALUES ('device.0123456789abcdef0123456789abcdef', 'Phone', 'pairing', 1)",
        [],
    )
    .unwrap();
    let (failed, seq): (i64, i64) = conn
        .query_row("SELECT failed_pins, last_seq FROM own_devices", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!((failed, seq), (0, 0));
}

#[test]
fn retro_proposals_name_agent_md_after_011() {
    let dir = tempfile::tempdir().unwrap();
    let (conn, path) = v0_1_0_file(dir.path());
    // Where a build with the weekly retro and own devices left it
    let at_010 = MIGRATIONS.iter().position(|m| m.version == 10).unwrap();
    upgrade(&conn, &MIGRATIONS[..=at_010], Some(&path)).unwrap();
    let insert = |id: &str, target: &str, file: &str| {
        conn.execute(
            "INSERT INTO retro_proposals (id, run_id, agent_id, target, path, reason, after,
                                          diff, status, created_at)
             VALUES (?1, 'r1', 'writer', ?2, ?3, 'Shorter', 'text', '', 'pending', 1)",
            rusqlite::params![id, target, file],
        )
        .unwrap();
    };
    insert("p1", "skill", ".nandi/agents/writer/skill.md");
    insert(
        "p2",
        "playbook:skill",
        ".nandi/agents/writer/skills/skill.md",
    );
    insert("p3", "workflow:daily", ".nandi/workflows/daily.yaml");

    let done = upgrade(&conn, MIGRATIONS, Some(&path)).unwrap();
    assert!(matches!(done, Upgrade::Upgraded { from: 10, .. }));
    let proposal = |id: &str| -> (String, String) {
        conn.query_row(
            "SELECT target, path FROM retro_proposals WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(
        proposal("p1"),
        ("agent".into(), ".nandi/agents/writer/agent.md".into())
    );
    assert_eq!(
        proposal("p2"),
        (
            "playbook:skill".into(),
            ".nandi/agents/writer/skills/skill.md".into()
        ),
        "a playbook keeps its name"
    );
    assert_eq!(
        proposal("p3"),
        (
            "workflow:daily".into(),
            ".nandi/workflows/daily.yaml".into()
        )
    );
}
