//! The `lock_box_keys` rows of `state.db` (migration 012): each key's
//! rules and its last use. Never its value: that is in the OS keychain.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::services::domain::lock_box::{KeyHeader, KeyKind, KeyRules, LockLevel};

/// One key's row
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub name: String,
    pub rules: KeyRules,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
    pub last_used_by: Option<String>,
    pub last_used_host: Option<String>,
}

const COLUMNS: &str = "name, kind, hosts, agents, lock, unlock_minutes, header_name, \
     header_prefix, created_at, updated_at, last_used_at, last_used_by, last_used_host";

fn bad(column: usize, what: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, what.into())
}

fn read(row: &Row) -> rusqlite::Result<KeyRow> {
    let kind: String = row.get("kind")?;
    let lock: String = row.get("lock")?;
    let hosts: String = row.get("hosts")?;
    let agents: String = row.get("agents")?;
    let minutes: i64 = row.get("unlock_minutes")?;
    let header_name: Option<String> = row.get("header_name")?;
    let header_prefix: String = row.get("header_prefix")?;
    Ok(KeyRow {
        name: row.get("name")?,
        rules: KeyRules {
            kind: KeyKind::parse(&kind)
                .ok_or_else(|| bad(1, format!("unknown key kind {kind}")))?,
            hosts: serde_json::from_str(&hosts).map_err(|e| bad(2, e.to_string()))?,
            agents: serde_json::from_str(&agents).map_err(|e| bad(3, e.to_string()))?,
            lock: LockLevel::parse(&lock).ok_or_else(|| bad(4, format!("unknown lock {lock}")))?,
            unlock_minutes: u32::try_from(minutes).unwrap_or(1),
            header: header_name.map(|name| KeyHeader {
                name,
                prefix: header_prefix,
            }),
        },
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        last_used_at: row.get("last_used_at")?,
        last_used_by: row.get("last_used_by")?,
        last_used_host: row.get("last_used_host")?,
    })
}

/// Every key, by name
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<KeyRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM lock_box_keys ORDER BY name"
    ))?;
    let rows = stmt.query_map([], read)?;
    rows.collect()
}

pub fn get(conn: &Connection, name: &str) -> rusqlite::Result<Option<KeyRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM lock_box_keys WHERE name = ?1"),
        [name],
        read,
    )
    .optional()
}

fn json(list: &[String]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".to_string())
}

/// A new key's rules; `false` when a key of that name exists
pub fn insert(conn: &Connection, name: &str, rules: &KeyRules, now: i64) -> rusqlite::Result<bool> {
    let header = rules.header.as_ref();
    let added = conn.execute(
        "INSERT OR IGNORE INTO lock_box_keys
             (name, kind, hosts, agents, lock, unlock_minutes, header_name, header_prefix,
              created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            name,
            rules.kind.as_str(),
            json(&rules.hosts),
            json(&rules.agents),
            rules.lock.as_str(),
            rules.unlock_minutes,
            header.map(|h| h.name.as_str()),
            header.map(|h| h.prefix.as_str()).unwrap_or_default(),
            now,
        ],
    )?;
    Ok(added == 1)
}

/// New rules for a key; `false` when there is no such key
pub fn update(conn: &Connection, name: &str, rules: &KeyRules, now: i64) -> rusqlite::Result<bool> {
    let header = rules.header.as_ref();
    let changed = conn.execute(
        "UPDATE lock_box_keys
            SET kind = ?2, hosts = ?3, agents = ?4, lock = ?5, unlock_minutes = ?6,
                header_name = ?7, header_prefix = ?8, updated_at = ?9
          WHERE name = ?1",
        params![
            name,
            rules.kind.as_str(),
            json(&rules.hosts),
            json(&rules.agents),
            rules.lock.as_str(),
            rules.unlock_minutes,
            header.map(|h| h.name.as_str()),
            header.map(|h| h.prefix.as_str()).unwrap_or_default(),
            now,
        ],
    )?;
    Ok(changed == 1)
}

pub fn delete(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM lock_box_keys WHERE name = ?1", [name])? == 1)
}

/// A release: who used it last, where and when
pub fn used(
    conn: &Connection,
    name: &str,
    agent: &str,
    host: &str,
    at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE lock_box_keys SET last_used_at = ?2, last_used_by = ?3, last_used_host = ?4
          WHERE name = ?1",
        params![name, at, agent, host],
    )?;
    Ok(())
}
