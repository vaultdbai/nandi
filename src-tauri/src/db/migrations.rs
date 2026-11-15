//! Numbered, forward-only migrations of `state.db`.
//!
//! Each migration is a SQL file in `db/migrations/`, embedded in the app.
//! `PRAGMA user_version` holds the number of the last one applied: 0 is a
//! new, empty database. At startup [`upgrade`] applies the ones a database
//! hasn't had, in order, in one transaction. Before it changes a database
//! file it keeps a copy beside it, `state.db.bak-v<old>`.
//!
//! A new database runs the same chain from 001, so a fresh database and an
//! upgraded one always have the same schema. A shipped migration never
//! changes (a test holds their checksums): add the next numbered file.
//! See `docs/developer/data-model.md#schema-changes`.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// One step of the schema. `version` is its number, from 1 with no gaps.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i64,
    /// The file it was embedded from, for messages and the checksum test
    pub file: &'static str,
    pub sql: &'static str,
}

/// Declares a shipped migration from its file in `db/migrations/`.
macro_rules! migration {
    ($version:literal, $file:literal) => {
        Migration {
            version: $version,
            file: $file,
            sql: include_str!(concat!("migrations/", $file)),
        }
    };
}

/// Every migration the app ships, in order. Append only.
pub const MIGRATIONS: &[Migration] = &[migration!(1, "001_baseline.sql")];

/// The schema version this build writes: the last migration's number.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// What [`upgrade`] did to a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upgrade {
    /// It was already at the latest version
    UpToDate,
    /// A new, empty database got the whole schema
    Created { to: i64 },
    /// An existing database moved forward. `backup` is the copy kept
    /// first (none for a database without a file).
    Upgraded {
        from: i64,
        to: i64,
        backup: Option<PathBuf>,
    },
}

/// Brings `conn` to the last of `migrations`. `file` is the database's
/// path: an existing database file is copied to `<file>.bak-v<old>` before
/// it changes. A database newer than `migrations` is refused untouched.
/// A failed migration rolls everything back.
pub fn upgrade(
    conn: &Connection,
    migrations: &[Migration],
    file: Option<&Path>,
) -> Result<Upgrade, String> {
    let found: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| format!("Couldn't read the local database's version: {e}"))?;
    let latest = migrations.last().map_or(0, |m| m.version);
    if found > latest {
        return Err(format!(
            "The local database was written by a newer version of Nandi (database \
             version {found}; this version reads up to {latest}). Nothing was changed. \
             Install the newer Nandi again to open it."
        ));
    }
    if found == latest {
        return Ok(Upgrade::UpToDate);
    }
    let backup = match file {
        Some(path) if found > 0 => Some(back_up(conn, path, found)?),
        _ => None,
    };
    let pending = &migrations[found as usize..];
    apply(conn, pending, latest).map_err(|failure| {
        let kept = backup
            .as_ref()
            .map(|path| format!(" A copy of it from before is at {}.", path.display()))
            .unwrap_or_default();
        format!(
            "Nandi couldn't update its local database from version {found} to {latest}: \
             {failure}. Nothing was changed.{kept} Your agents and workflows in \
             ~/.nandi/repo aren't touched."
        )
    })?;
    Ok(if found == 0 {
        Upgrade::Created { to: latest }
    } else {
        Upgrade::Upgraded {
            from: found,
            to: latest,
            backup,
        }
    })
}

/// Applies `pending` and stamps `latest`, all in one transaction.
///
/// Foreign keys are off while it runs, so a migration can rebuild a table
/// (SQLite's 12-step way) without a cascade deleting rows elsewhere. Before
/// the commit every key is checked: a migration that breaks one fails.
fn apply(conn: &Connection, pending: &[Migration], latest: i64) -> Result<(), String> {
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .map_err(|e| e.to_string())?;
    let applied = apply_in_transaction(conn, pending, latest);
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(|e| e.to_string())?;
    applied
}

fn apply_in_transaction(
    conn: &Connection,
    pending: &[Migration],
    latest: i64,
) -> Result<(), String> {
    // Dropped without a commit, the transaction rolls back
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for migration in pending {
        tx.execute_batch(migration.sql)
            .map_err(|e| format!("migration {} failed: {e}", migration.file))?;
    }
    let broken_keys: i64 = tx
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    if broken_keys > 0 {
        return Err(format!("{broken_keys} rows lost the row they refer to"));
    }
    tx.pragma_update(None, "user_version", latest)
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// Copies the database file to `<file>.bak-v<version>`, after moving the
/// write-ahead log into it, so the copy holds every committed row.
fn back_up(conn: &Connection, file: &Path, version: i64) -> Result<PathBuf, String> {
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(|e| format!("Couldn't prepare the local database for an update: {e}"))?;
    if busy != 0 {
        return Err(
            "Couldn't update the local database: another program is using it. Close it \
             and start Nandi again."
                .into(),
        );
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".bak-v{version}"));
    let backup = file.with_file_name(name);
    std::fs::copy(file, &backup).map_err(|e| {
        format!(
            "Couldn't keep a copy of the local database before updating it ({}): {e}. \
             Nothing was changed.",
            backup.display()
        )
    })?;
    Ok(backup)
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;
