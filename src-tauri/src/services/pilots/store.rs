//! Pilot state in SQLite `local_settings`: machine-local runtime data
//! (installed, running, health, pid, port) under `pilots.<id>.<field>`, and
//! the user's per-agent consent under
//! `consent.<agent_id>.<pilot_id>.<capability>`, and the person's grant of
//! My Nandi browser's shared profile to an agent's visible browser under
//! `consent.<agent_id>.browser.profile` (`chrome` or `edge`).
//!
//! None of this is a definition (the catalog is code, an agent's `pilots:`
//! block is in its agent.md), so it doesn't belong in the repo files.

use std::sync::LazyLock;

use rusqlite::{params, Connection};
use serde_json::Value;

use super::profiles::BrowserKind;
use super::PilotStatus;
use crate::db;

/// This app process. A `running` row from an earlier session describes a
/// process this session doesn't own, so it doesn't count as running.
static SESSION: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

pub fn key(id: &str, field: &str) -> String {
    format!("pilots.{id}.{field}")
}

pub fn consent_key(agent_id: &str, pilot_id: &str, capability: &str) -> String {
    format!("consent.{agent_id}.{pilot_id}.{capability}")
}

const PROFILE_GRANT: &str = "profile";

/// Where the person's grant of a shared browser profile to `agent_id` is kept
pub fn profile_key(agent_id: &str) -> String {
    consent_key(agent_id, super::browser::BROWSER, PROFILE_GRANT)
}

/// The shared browser profile the person granted `agent_id`, if any
pub fn browser_profile(conn: &Connection, agent_id: &str) -> rusqlite::Result<Option<BrowserKind>> {
    Ok(db::get_setting(conn, &profile_key(agent_id))?
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok())))
}

/// Grants `agent_id` the shared `kind` profile, or (`None`) takes the grant
/// back so it uses its own again
pub fn set_browser_profile(
    conn: &Connection,
    agent_id: &str,
    kind: Option<BrowserKind>,
) -> rusqlite::Result<()> {
    match kind {
        Some(kind) => set(conn, &profile_key(agent_id), &Value::from(kind.id())),
        None => remove(conn, &profile_key(agent_id)),
    }
}

/// Every agent granted a shared browser profile, sorted by agent
pub fn browser_profiles(conn: &Connection) -> rusqlite::Result<Vec<(String, BrowserKind)>> {
    let suffix = format!(".{}.{PROFILE_GRANT}", super::browser::BROWSER);
    let mut stmt = conn
        .prepare("SELECT key, value FROM local_settings WHERE key LIKE 'consent.%' ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (key, value) = row?;
        let agent = key
            .strip_prefix("consent.")
            .and_then(|rest| rest.strip_suffix(suffix.as_str()));
        let kind = serde_json::from_str::<String>(&value)
            .ok()
            .and_then(|s| s.parse::<BrowserKind>().ok());
        if let (Some(agent), Some(kind)) = (agent, kind) {
            out.push((agent.to_string(), kind));
        }
    }
    Ok(out)
}

fn set(conn: &Connection, key: &str, value: &Value) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

fn remove(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM local_settings WHERE key = ?1", [key])?;
    Ok(())
}

fn get_bool(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    Ok(db::get_setting(conn, key)?.and_then(|v| v.as_bool()) == Some(true))
}

fn get_u64(conn: &Connection, key: &str) -> rusqlite::Result<Option<u64>> {
    Ok(db::get_setting(conn, key)?.and_then(|v| v.as_u64()))
}

/// A pilot's row set, as last written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PersistedPilot {
    pub installed: bool,
    pub running: bool,
    pub healthy: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    /// The app session that started it
    pub session: Option<String>,
}

impl PersistedPilot {
    /// Running, and started by this app session.
    pub fn is_live(&self) -> bool {
        self.running && self.session.as_deref() == Some(SESSION.as_str())
    }

    /// Live and passing its health check: the only state tools route to.
    pub fn is_routable(&self) -> bool {
        self.is_live() && self.healthy
    }

    pub fn status(&self) -> PilotStatus {
        let live = self.is_live();
        PilotStatus {
            installed: self.installed,
            running: live,
            healthy: live && self.healthy,
            pid: self.pid.filter(|_| live),
        }
    }
}

pub fn load(conn: &Connection, id: &str) -> rusqlite::Result<PersistedPilot> {
    Ok(PersistedPilot {
        installed: get_bool(conn, &key(id, "installed"))?,
        running: get_bool(conn, &key(id, "running"))?,
        healthy: get_bool(conn, &key(id, "healthy"))?,
        pid: get_u64(conn, &key(id, "pid"))?.and_then(|p| u32::try_from(p).ok()),
        port: get_u64(conn, &key(id, "port"))?.and_then(|p| u16::try_from(p).ok()),
        session: db::get_setting(conn, &key(id, "session"))?
            .and_then(|v| v.as_str().map(str::to_string)),
    })
}

pub fn save_installed(conn: &Connection, id: &str, installed: bool) -> rusqlite::Result<()> {
    set(conn, &key(id, "installed"), &Value::Bool(installed))
}

pub fn save_running(
    conn: &Connection,
    id: &str,
    pid: Option<u32>,
    port: u16,
) -> rusqlite::Result<()> {
    set(conn, &key(id, "running"), &Value::Bool(true))?;
    set(conn, &key(id, "healthy"), &Value::Bool(false))?;
    match pid {
        Some(pid) => set(conn, &key(id, "pid"), &Value::from(pid))?,
        None => remove(conn, &key(id, "pid"))?,
    }
    set(conn, &key(id, "port"), &Value::from(port))?;
    set(conn, &key(id, "session"), &Value::String(SESSION.clone()))
}

pub fn save_healthy(conn: &Connection, id: &str, healthy: bool) -> rusqlite::Result<()> {
    set(conn, &key(id, "healthy"), &Value::Bool(healthy))
}

pub fn save_stopped(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    set(conn, &key(id, "running"), &Value::Bool(false))?;
    set(conn, &key(id, "healthy"), &Value::Bool(false))?;
    remove(conn, &key(id, "pid"))
}

pub fn grant(
    conn: &Connection,
    agent_id: &str,
    pilot_id: &str,
    capability: &str,
) -> rusqlite::Result<()> {
    set(
        conn,
        &consent_key(agent_id, pilot_id, capability),
        &Value::Bool(true),
    )
}

pub fn revoke(
    conn: &Connection,
    agent_id: &str,
    pilot_id: &str,
    capability: &str,
) -> rusqlite::Result<()> {
    remove(conn, &consent_key(agent_id, pilot_id, capability))
}

/// The capabilities of `pilot_id` the user granted to `agent_id`, sorted.
pub fn granted(conn: &Connection, agent_id: &str, pilot_id: &str) -> rusqlite::Result<Vec<String>> {
    let prefix = format!("consent.{agent_id}.{pilot_id}.");
    let mut stmt = conn.prepare(
        "SELECT key, value FROM local_settings WHERE substr(key, 1, ?2) = ?1 ORDER BY key",
    )?;
    let rows = stmt.query_map(params![prefix, prefix.len() as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut granted = Vec::new();
    for row in rows {
        let (key, value) = row?;
        if value == "true" {
            granted.push(key[prefix.len()..].to_string());
        }
    }
    Ok(granted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn round_trips_runtime_state() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        assert_eq!(load(&conn, "p").unwrap(), PersistedPilot::default());

        save_installed(&conn, "p", true).unwrap();
        save_running(&conn, "p", Some(42), 17898).unwrap();
        let running = load(&conn, "p").unwrap();
        assert!(running.is_live() && !running.is_routable());
        assert_eq!(running.port, Some(17898));
        assert_eq!(
            running.status(),
            PilotStatus {
                installed: true,
                running: true,
                healthy: false,
                pid: Some(42)
            }
        );

        save_healthy(&conn, "p", true).unwrap();
        assert!(load(&conn, "p").unwrap().is_routable());
        // The raw key layout is part of the contract
        assert_eq!(
            db::get_setting(&conn, "pilots.p.healthy").unwrap(),
            Some(Value::Bool(true))
        );

        save_stopped(&conn, "p").unwrap();
        let stopped = load(&conn, "p").unwrap();
        assert_eq!(stopped.status().pid, None);
        assert!(!stopped.is_live() && stopped.installed);
    }

    #[test]
    fn rows_from_another_session_are_not_running() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        save_running(&conn, "p", Some(7), 1).unwrap();
        save_healthy(&conn, "p", true).unwrap();
        set(&conn, &key("p", "session"), &Value::String("old".into())).unwrap();
        let stale = load(&conn, "p").unwrap();
        assert!(!stale.is_routable());
        assert_eq!(stale.status(), PilotStatus::default());
    }

    #[test]
    fn consent_is_per_agent_and_pilot() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        grant(&conn, "writer", "opendesk", "browser").unwrap();
        grant(&conn, "writer", "opendesk", "app_control").unwrap();
        grant(&conn, "other", "opendesk", "file_system").unwrap();
        grant(&conn, "writer", "other-pilot", "browser").unwrap();
        assert_eq!(
            granted(&conn, "writer", "opendesk").unwrap(),
            ["app_control", "browser"]
        );
        assert_eq!(
            db::get_setting(&conn, "consent.writer.opendesk.browser").unwrap(),
            Some(Value::Bool(true))
        );
        assert!(granted(&conn, "writer", "macpilot").unwrap().is_empty());

        revoke(&conn, "writer", "opendesk", "browser").unwrap();
        assert_eq!(
            granted(&conn, "writer", "opendesk").unwrap(),
            ["app_control"]
        );
        assert_eq!(
            granted(&conn, "other", "opendesk").unwrap(),
            ["file_system"],
            "revoking one agent's grant leaves another's"
        );
        // Revoking what was never granted is a no-op
        revoke(&conn, "writer", "macpilot", "browser").unwrap();
    }

    #[test]
    fn the_shared_profile_grant_is_per_agent_and_apart_from_capabilities() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        assert_eq!(browser_profile(&conn, "cos").unwrap(), None, "its own");
        grant(&conn, "cos", "browser", "browser").unwrap();
        set_browser_profile(&conn, "cos", Some(BrowserKind::Chrome)).unwrap();
        set_browser_profile(&conn, "shopper", Some(BrowserKind::Edge)).unwrap();
        assert_eq!(
            browser_profile(&conn, "cos").unwrap(),
            Some(BrowserKind::Chrome)
        );
        assert_eq!(
            db::get_setting(&conn, "consent.cos.browser.profile").unwrap(),
            Some(Value::from("chrome"))
        );
        assert_eq!(
            granted(&conn, "cos", "browser").unwrap(),
            ["browser"],
            "not a capability"
        );
        assert_eq!(
            browser_profiles(&conn).unwrap(),
            [
                ("cos".to_string(), BrowserKind::Chrome),
                ("shopper".to_string(), BrowserKind::Edge)
            ]
        );

        set_browser_profile(&conn, "cos", None).unwrap();
        assert_eq!(browser_profile(&conn, "cos").unwrap(), None);
        assert_eq!(
            browser_profile(&conn, "shopper").unwrap(),
            Some(BrowserKind::Edge),
            "taking one back leaves another's"
        );
        // A value that isn't a browser type is no grant
        set(&conn, &profile_key("odd"), &Value::from("firefox")).unwrap();
        assert_eq!(browser_profile(&conn, "odd").unwrap(), None);
        assert_eq!(browser_profiles(&conn).unwrap().len(), 1);
    }
}
