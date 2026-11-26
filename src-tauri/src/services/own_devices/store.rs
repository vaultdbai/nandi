//! The `own_devices` rows of `state.db` (migration 010): what only this
//! Nandi knows about its person's devices. The relay knows the token and
//! when it last heard from one; nothing here leaves this computer.

use nandi_protocol::PublicKeys;
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::controls::AskLevel;

/// Where a device is, as this Nandi keeps it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    /// A code is shown; no phone claimed it yet
    Pairing,
    /// A phone claimed it; its fingerprint waits for the person
    Claimed,
    /// Confirmed, with a PIN: its envelopes are opened
    Active,
}

impl RowState {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "pairing" => Some(Self::Pairing),
            "claimed" => Some(Self::Claimed),
            "active" => Some(Self::Active),
            _ => None,
        }
    }
}

/// One device's row
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub device_id: String,
    pub name: String,
    pub state: RowState,
    /// From its claim; `None` while pairing
    pub keys: Option<PublicKeys>,
    pub pin_hash: Option<String>,
    pub failed_pins: u32,
    pub locked_at: Option<i64>,
    pub last_seq: u64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub paired_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    /// How much it may ask (migration 013)
    pub ask_level: AskLevel,
}

impl DeviceRow {
    pub fn is_locked(&self) -> bool {
        self.locked_at.is_some()
    }
}

const COLUMNS: &str = "device_id, name, state, signing_key, sealing_key, pin_hash, failed_pins, \
     locked_at, last_seq, created_at, expires_at, paired_at, last_seen_at, ask_level";

fn read(row: &Row) -> rusqlite::Result<DeviceRow> {
    let state: String = row.get("state")?;
    let signing: Option<String> = row.get("signing_key")?;
    let sealing: Option<String> = row.get("sealing_key")?;
    let failed: i64 = row.get("failed_pins")?;
    let seq: i64 = row.get("last_seq")?;
    Ok(DeviceRow {
        device_id: row.get("device_id")?,
        name: row.get("name")?,
        state: RowState::parse(&state).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                format!("unknown device state {state}").into(),
            )
        })?,
        keys: signing
            .zip(sealing)
            .map(|(signing, sealing)| PublicKeys { signing, sealing }),
        pin_hash: row.get("pin_hash")?,
        failed_pins: u32::try_from(failed).unwrap_or(u32::MAX),
        locked_at: row.get("locked_at")?,
        last_seq: u64::try_from(seq).unwrap_or_default(),
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
        paired_at: row.get("paired_at")?,
        last_seen_at: row.get("last_seen_at")?,
        ask_level: {
            let level: String = row.get("ask_level")?;
            AskLevel::parse(&level).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    13,
                    rusqlite::types::Type::Text,
                    format!("unknown ask level {level}").into(),
                )
            })?
        },
    })
}

/// Sets how much an active device may ask; false when there's no such device
pub fn set_ask_level(
    conn: &Connection,
    device_id: &str,
    level: AskLevel,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE own_devices SET ask_level = ?2 WHERE device_id = ?1 AND state = 'active'",
        params![device_id, level.as_str()],
    )?;
    Ok(changed == 1)
}

/// How much `device_id` may ask now: `None` when it's gone, not active or
/// locked (it may ask nothing)
pub fn ask_level_now(conn: &Connection, device_id: &str) -> rusqlite::Result<Option<AskLevel>> {
    Ok(get(conn, device_id)?
        .filter(|row| row.state == RowState::Active && !row.is_locked())
        .map(|row| row.ask_level))
}

/// Every device, oldest first
pub fn list(conn: &Connection) -> rusqlite::Result<Vec<DeviceRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM own_devices ORDER BY created_at, device_id"
    ))?;
    let rows = stmt.query_map([], read)?;
    rows.collect()
}

pub fn get(conn: &Connection, device_id: &str) -> rusqlite::Result<Option<DeviceRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM own_devices WHERE device_id = ?1"),
        [device_id],
        read,
    )
    .optional()
}

/// A new pairing: any other open pairing goes (the relay replaced it)
pub fn open_pairing(
    conn: &Connection,
    device_id: &str,
    name: &str,
    now: i64,
    expires_at: i64,
) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM own_devices WHERE state = 'pairing'", [])?;
    conn.execute(
        "INSERT INTO own_devices (device_id, name, state, created_at, expires_at)
         VALUES (?1, ?2, 'pairing', ?3, ?4)",
        params![device_id, name, now, expires_at],
    )?;
    Ok(())
}

/// A phone claimed `device_id` with `keys`; false when there's no pairing
/// of ours waiting for it
pub fn claimed(
    conn: &Connection,
    device_id: &str,
    keys: &PublicKeys,
    expires_at: Option<i64>,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE own_devices
         SET state = 'claimed', signing_key = ?2, sealing_key = ?3,
             expires_at = COALESCE(?4, expires_at)
         WHERE device_id = ?1 AND state IN ('pairing', 'claimed')",
        params![device_id, keys.signing, keys.sealing, expires_at],
    )?;
    Ok(changed == 1)
}

/// The person confirmed it with `pin_hash`
pub fn activate(
    conn: &Connection,
    device_id: &str,
    pin_hash: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE own_devices
         SET state = 'active', pin_hash = ?2, paired_at = ?3, expires_at = NULL,
             failed_pins = 0, locked_at = NULL
         WHERE device_id = ?1 AND state = 'claimed'",
        params![device_id, pin_hash, now],
    )?;
    Ok(changed == 1)
}

/// Takes envelope `seq` from `device_id`: true when it's newer than every
/// one taken before (and is now the cursor), false for a replay
pub fn take_seq(conn: &Connection, device_id: &str, seq: u64, now: i64) -> rusqlite::Result<bool> {
    let Ok(seq) = i64::try_from(seq) else {
        return Ok(false);
    };
    let changed = conn.execute(
        "UPDATE own_devices SET last_seq = ?2, last_seen_at = ?3
         WHERE device_id = ?1 AND state = 'active' AND last_seq < ?2",
        params![device_id, seq, now],
    )?;
    Ok(changed == 1)
}

/// One more wrong PIN: the count after it, locking the device at `limit`
pub fn wrong_pin(
    conn: &Connection,
    device_id: &str,
    limit: u32,
    now: i64,
) -> rusqlite::Result<u32> {
    conn.execute(
        "UPDATE own_devices
         SET failed_pins = failed_pins + 1,
             locked_at = CASE WHEN failed_pins + 1 >= ?2 THEN COALESCE(locked_at, ?3)
                              ELSE locked_at END
         WHERE device_id = ?1",
        params![device_id, limit, now],
    )?;
    conn.query_row(
        "SELECT failed_pins FROM own_devices WHERE device_id = ?1",
        [device_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
}

/// A right PIN: the count starts again
pub fn right_pin(conn: &Connection, device_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE own_devices SET failed_pins = 0 WHERE device_id = ?1 AND locked_at IS NULL",
        [device_id],
    )?;
    Ok(())
}

/// The desktop unlocks it, with a new PIN hash when one was chosen
pub fn unlock(
    conn: &Connection,
    device_id: &str,
    pin_hash: Option<&str>,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE own_devices
         SET failed_pins = 0, locked_at = NULL, pin_hash = COALESCE(?2, pin_hash)
         WHERE device_id = ?1 AND state = 'active'",
        params![device_id, pin_hash],
    )?;
    Ok(changed == 1)
}

pub fn remove(conn: &Connection, device_id: &str) -> rusqlite::Result<bool> {
    let changed = conn.execute("DELETE FROM own_devices WHERE device_id = ?1", [device_id])?;
    Ok(changed == 1)
}

pub fn remove_all(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM own_devices", [])
}

/// Codes and claims that lapsed before the person confirmed them
pub fn remove_lapsed(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM own_devices
         WHERE state != 'active' AND expires_at IS NOT NULL AND expires_at <= ?1",
        [now],
    )
}
