//! Requests held for the owner's approval (task 5). A capability on "Needs
//! my approval" (`policy.require_approval`) no longer refuses: the task
//! stays `Delivered` at stage `approval`, a `gateway_holds` row puts it in
//! the owner's Inbox, and routing waits until the owner approves (it goes
//! on), declines (it's rejected), or the hold expires. The row survives a
//! restart: a resumed task finds its decision there.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::oneshot;

use crate::model::Timestamp;
use crate::services::events::NandiEvent;

/// `gateway://hold_changed`
pub const GATEWAY_HOLD_CHANGED: &str = "gateway://hold_changed";

/// How long a held request waits for the owner (as tool approvals do)
pub const HOLD_TTL_MS: i64 = crate::model::APPROVAL_TTL_MS;

/// Where a hold is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum HoldStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl HoldStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::Pending, Self::Approved, Self::Rejected, Self::Expired]
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}

/// A request waiting for the owner, as the Inbox shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHold {
    /// The gateway task (its Request page)
    pub task_id: String,
    /// Shown only under Details
    pub caller_instance: String,
    /// Who asked, in words
    pub caller_name: String,
    pub capability: Option<String>,
    /// What they asked, in their words (first lines)
    pub summary: String,
    pub status: HoldStatus,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// `gateway://hold_changed`: a hold appeared or was decided
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayHoldChanged {
    pub hold: GatewayHold,
}

impl NandiEvent for GatewayHoldChanged {
    const NAME: &'static str = GATEWAY_HOLD_CHANGED;
}

/// How long a summary may be
const MAX_SUMMARY_CHARS: usize = 280;

/// The first lines of a request, for the Inbox
pub fn summarize(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_SUMMARY_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_SUMMARY_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// What a waiting hold ends as
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldOutcome {
    Approved,
    Rejected,
    Expired,
    /// The requester (or a timeout on its side) ended the task
    Ended,
}

impl HoldOutcome {
    /// A decided row read back (a resumed task finds it there)
    pub fn of(status: HoldStatus) -> Option<Self> {
        match status {
            HoldStatus::Pending => None,
            HoldStatus::Approved => Some(Self::Approved),
            HoldStatus::Rejected => Some(Self::Rejected),
            HoldStatus::Expired => Some(Self::Expired),
        }
    }
}

// -- Rows ---------------------------------------------------------------------

const COLUMNS: &str =
    "task_id, caller_instance, caller_name, capability, summary, status, created_at, expires_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<GatewayHold> {
    let status: String = row.get(5)?;
    Ok(GatewayHold {
        task_id: row.get(0)?,
        caller_instance: row.get(1)?,
        caller_name: row.get(2)?,
        capability: row.get(3)?,
        summary: row.get(4)?,
        status: HoldStatus::parse(&status).unwrap_or(HoldStatus::Expired),
        created_at: row.get::<_, i64>(6)? as f64,
        expires_at: row.get::<_, i64>(7)? as f64,
    })
}

pub fn get(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<GatewayHold>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM gateway_holds WHERE task_id = ?1"),
        [task_id],
        from_row,
    )
    .optional()
}

/// The hold for `hold.task_id`: a new pending row, or the one already there
/// (a resumed task keeps its expiry and any decision)
pub fn open(conn: &Connection, hold: &GatewayHold) -> rusqlite::Result<GatewayHold> {
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO gateway_holds ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        ),
        params![
            hold.task_id,
            hold.caller_instance,
            hold.caller_name,
            hold.capability,
            hold.summary,
            HoldStatus::Pending.as_str(),
            hold.created_at as i64,
            hold.expires_at as i64,
        ],
    )?;
    Ok(get(conn, &hold.task_id)?.unwrap_or_else(|| hold.clone()))
}

/// Decides a pending hold; `None` when it isn't pending (decided already)
pub fn decide(
    conn: &Connection,
    task_id: &str,
    status: HoldStatus,
    now: i64,
) -> rusqlite::Result<Option<GatewayHold>> {
    let updated = conn.execute(
        "UPDATE gateway_holds SET status = ?1, decided_at = ?2
          WHERE task_id = ?3 AND status = 'pending'",
        params![status.as_str(), now, task_id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    get(conn, task_id)
}

/// Pending holds, oldest first
pub fn pending(conn: &Connection) -> rusqlite::Result<Vec<GatewayHold>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM gateway_holds WHERE status = 'pending' ORDER BY created_at"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect()
}

// -- Waiters ------------------------------------------------------------------

/// Routing waits on these; a decision wakes the one for its task. The
/// owner's note with a decision (a refusal's reason, an approval's
/// condition) is kept here until routing takes it for the requester; it
/// lives in this process only, like the waiter.
#[derive(Default)]
pub struct HoldWaiters {
    waiters: Mutex<HashMap<String, oneshot::Sender<HoldStatus>>>,
    notes: Mutex<HashMap<String, String>>,
}

impl HoldWaiters {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<HoldStatus>>> {
        self.waiters.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notes(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.notes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps the owner's note for the task's requester (blank: none)
    pub fn set_note(&self, task_id: &str, note: Option<&str>) {
        match note.map(str::trim).filter(|n| !n.is_empty()) {
            Some(note) => {
                self.notes().insert(task_id.to_string(), note.to_string());
            }
            None => {
                self.notes().remove(task_id);
            }
        }
    }

    /// The owner's note for the task, once
    pub fn take_note(&self, task_id: &str) -> Option<String> {
        self.notes().remove(task_id)
    }

    pub fn wait(&self, task_id: &str) -> oneshot::Receiver<HoldStatus> {
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(task_id.to_string(), sender);
        receiver
    }

    /// Wakes the task's waiter, if one is waiting in this process
    pub fn wake(&self, task_id: &str, status: HoldStatus) {
        if let Some(sender) = self.lock().remove(task_id) {
            let _ = sender.send(status);
        }
    }

    pub fn forget(&self, task_id: &str) {
        self.lock().remove(task_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(task_id: &str, created_at: i64) -> GatewayHold {
        GatewayHold {
            task_id: task_id.into(),
            caller_instance: "desktop-sam".into(),
            caller_name: "Sam Lee".into(),
            capability: Some("finance.risk".into()),
            summary: "Summarize Q3 risk".into(),
            status: HoldStatus::Pending,
            created_at: created_at as f64,
            expires_at: (created_at + HOLD_TTL_MS) as f64,
        }
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn a_hold_opens_once_and_is_decided_once() {
        let conn = conn();
        let opened = open(&conn, &hold("t1", 10)).unwrap();
        assert_eq!(opened.status, HoldStatus::Pending);
        // Resumed after a restart: the same row, the same expiry
        let again = open(&conn, &hold("t1", 99)).unwrap();
        assert_eq!(again.created_at, 10.0);
        assert_eq!(pending(&conn).unwrap().len(), 1);

        let decided = decide(&conn, "t1", HoldStatus::Approved, 20)
            .unwrap()
            .unwrap();
        assert_eq!(decided.status, HoldStatus::Approved);
        assert_eq!(decide(&conn, "t1", HoldStatus::Rejected, 30).unwrap(), None);
        assert!(pending(&conn).unwrap().is_empty());
        assert_eq!(HoldOutcome::of(decided.status), Some(HoldOutcome::Approved));
    }

    #[test]
    fn pending_holds_are_oldest_first() {
        let conn = conn();
        open(&conn, &hold("new", 20)).unwrap();
        open(&conn, &hold("old", 10)).unwrap();
        let ids: Vec<String> = pending(&conn)
            .unwrap()
            .into_iter()
            .map(|h| h.task_id)
            .collect();
        assert_eq!(ids, ["old", "new"]);
    }

    #[test]
    fn every_status_reads_back_and_maps_to_an_outcome() {
        for (status, outcome) in [
            (HoldStatus::Pending, None),
            (HoldStatus::Approved, Some(HoldOutcome::Approved)),
            (HoldStatus::Rejected, Some(HoldOutcome::Rejected)),
            (HoldStatus::Expired, Some(HoldOutcome::Expired)),
        ] {
            assert_eq!(HoldStatus::parse(status.as_str()), Some(status));
            assert_eq!(HoldOutcome::of(status), outcome);
        }
    }

    #[test]
    fn a_decision_wakes_its_waiter() {
        let waiters = HoldWaiters::default();
        let mut receiver = waiters.wait("t1");
        waiters.wake("other", HoldStatus::Approved);
        assert!(receiver.try_recv().is_err());
        waiters.set_note("t1", Some(" Not this quarter "));
        waiters.wake("t1", HoldStatus::Rejected);
        assert_eq!(receiver.try_recv().unwrap(), HoldStatus::Rejected);
        assert_eq!(waiters.take_note("t1").as_deref(), Some("Not this quarter"));
        assert_eq!(waiters.take_note("t1"), None);
        waiters.set_note("t2", Some("  "));
        assert_eq!(waiters.take_note("t2"), None);
    }

    #[test]
    fn summaries_are_short() {
        assert_eq!(summarize("  hi  "), "hi");
        let long = summarize(&"x".repeat(1000));
        assert_eq!(long.chars().count(), MAX_SUMMARY_CHARS + 1);
    }
}
