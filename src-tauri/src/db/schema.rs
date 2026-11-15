//! Opening a database on the schema: the migrations, then default settings.

use super::migrations;
use super::*;

pub use super::migrations::{standing, Standing, Upgrade, MIGRATIONS, SCHEMA_VERSION};

/// Brings a database without a file (in memory, in tests) to the latest
/// schema and seeds default settings. Safe to run on every open.
#[cfg(test)]
pub fn init_db(conn: &Connection) -> Result<(), String> {
    migrations::upgrade(conn, MIGRATIONS, None)?;
    seed_defaults(conn).map_err(|e| e.to_string())
}

/// Brings the database file at `path` to the latest schema, keeping a copy
/// of it first when it is upgraded, and seeds default settings. A database
/// newer than this build is refused untouched.
pub fn init_db_file(conn: &Connection, path: &Path) -> Result<Upgrade, String> {
    let upgrade = migrations::upgrade(conn, MIGRATIONS, Some(path))?;
    seed_defaults(conn).map_err(|e| e.to_string())?;
    Ok(upgrade)
}

fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO local_settings (key, value) VALUES (?1, ?2)",
        params![
            crate::engine::CANNED_SCRIPT_KEY,
            crate::engine::default_script().to_string()
        ],
    )?;
    Ok(())
}
