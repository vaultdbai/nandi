//! Work assigned to the owner (`assign-me`). Once the owner approves the
//! request (a hold), the gateway doesn't route it to an agent: it opens an
//! assignment, a `gateway_assignments` row the Inbox shows until the owner
//! marks it Done (the requester gets the note as the result) or Declines it
//! (the requester's task is refused). It lasts as long as that takes; the
//! row survives a restart, and a resumed task finds it (or its outcome)
//! there.
//!
//! The requester may attach links (`input.links`: URLs, or `{url, name}`).
//! The relay carries no files, so a file goes as a link to where it lives.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use strum::{EnumString, IntoStaticStr};
use tokio::sync::oneshot;

use crate::db::text_enum;
use crate::model::Timestamp;
use crate::services::events::NandiEvent;

/// `gateway://assignment_changed`
pub const GATEWAY_ASSIGNMENT_CHANGED: &str = "gateway://assignment_changed";

/// How much of the request the Inbox keeps
const MAX_REQUEST_CHARS: usize = 4000;

/// How many links an assignment keeps, and how long a link's name may be
const MAX_LINKS: usize = 10;
const MAX_LINK_NAME_CHARS: usize = 200;

/// Where an assignment is
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum AssignmentStatus {
    Open,
    Done,
    Declined,
    /// The requester withdrew it
    Cancelled,
}

impl AssignmentStatus {
    /// The text stored in the database
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Work someone assigned to the owner, as the Inbox shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayAssignment {
    /// The gateway task (its Request page)
    pub task_id: String,
    /// Shown only under Details
    pub caller_instance: String,
    /// Who assigned it, in words
    pub caller_name: String,
    /// What they asked, in their words
    pub request: String,
    pub status: AssignmentStatus,
    /// The owner's note when they closed it (what they did, or why not)
    pub note: Option<String>,
    pub created_at: Timestamp,
    pub due_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
    /// What the requester attached
    pub links: Vec<AssignmentLink>,
}

/// A link attached to an assignment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentLink {
    /// What it is, in the requester's words (else the URL)
    pub name: String,
    /// `http(s)` only
    pub url: String,
}

/// `gateway://assignment_changed`: an assignment opened or closed
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayAssignmentChanged {
    pub assignment: GatewayAssignment,
}

impl NandiEvent for GatewayAssignmentChanged {
    const NAME: &'static str = GATEWAY_ASSIGNMENT_CHANGED;
}

/// The request text, cut to what the Inbox keeps
pub fn request_of(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_REQUEST_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_REQUEST_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// The links in a request's `input.links`: each a URL or `{url, name?}`.
/// Only `http(s)` URLs are kept (no `file:` or script links), once each,
/// up to [`MAX_LINKS`].
pub fn links_of(input: &Value) -> Vec<AssignmentLink> {
    let Some(items) = input.get("links").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut links: Vec<AssignmentLink> = Vec::new();
    for item in items {
        let (url, name) = match item {
            Value::String(url) => (url.as_str(), None),
            Value::Object(map) => match map.get("url").and_then(Value::as_str) {
                Some(url) => (url, map.get("name").and_then(Value::as_str)),
                None => continue,
            },
            _ => continue,
        };
        let url = url.trim();
        let web = reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
        if !web || links.iter().any(|link| link.url == url) {
            continue;
        }
        let name = name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(url);
        links.push(AssignmentLink {
            name: name.chars().take(MAX_LINK_NAME_CHARS).collect(),
            url: url.to_string(),
        });
        if links.len() == MAX_LINKS {
            break;
        }
    }
    links
}

// -- Rows ---------------------------------------------------------------------

const COLUMNS: &str =
    "task_id, caller_instance, caller_name, request, status, note, created_at, due_at, closed_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<GatewayAssignment> {
    Ok(GatewayAssignment {
        task_id: row.get(0)?,
        caller_instance: row.get(1)?,
        caller_name: row.get(2)?,
        request: row.get(3)?,
        status: text_enum(row, 4)?,
        note: row.get(5)?,
        created_at: row.get::<_, i64>(6)? as f64,
        due_at: row.get::<_, Option<i64>>(7)?.map(|t| t as f64),
        closed_at: row.get::<_, Option<i64>>(8)?.map(|t| t as f64),
        links: Vec::new(),
    })
}

/// `assignment` with its links read in
fn with_links(
    conn: &Connection,
    mut assignment: GatewayAssignment,
) -> rusqlite::Result<GatewayAssignment> {
    let mut stmt = conn.prepare_cached(
        "SELECT name, url FROM gateway_assignment_links WHERE task_id = ?1 ORDER BY position",
    )?;
    let links = stmt.query_map([&assignment.task_id], |row| {
        Ok(AssignmentLink {
            name: row.get(0)?,
            url: row.get(1)?,
        })
    })?;
    assignment.links = links.collect::<rusqlite::Result<_>>()?;
    Ok(assignment)
}

pub fn get(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<GatewayAssignment>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM gateway_assignments WHERE task_id = ?1"),
        [task_id],
        from_row,
    )
    .optional()?
    .map(|assignment| with_links(conn, assignment))
    .transpose()
}

/// The assignment for `assignment.task_id`: a new open row, or the one
/// already there (a resumed task keeps its dates and any outcome)
pub fn open(
    conn: &Connection,
    assignment: &GatewayAssignment,
) -> rusqlite::Result<GatewayAssignment> {
    let inserted = conn.execute(
        &format!(
            "INSERT OR IGNORE INTO gateway_assignments ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, NULL)"
        ),
        params![
            assignment.task_id,
            assignment.caller_instance,
            assignment.caller_name,
            assignment.request,
            AssignmentStatus::Open.as_str(),
            assignment.created_at as i64,
            assignment.due_at.map(|t| t as i64),
        ],
    )?;
    // A resumed task keeps the links it was opened with
    if inserted > 0 {
        for (position, link) in assignment.links.iter().enumerate() {
            conn.execute(
                "INSERT INTO gateway_assignment_links (task_id, position, name, url)
                 VALUES (?1, ?2, ?3, ?4)",
                params![assignment.task_id, position as i64, link.name, link.url],
            )?;
        }
    }
    Ok(get(conn, &assignment.task_id)?.unwrap_or_else(|| assignment.clone()))
}

/// Closes an open assignment; `None` when it isn't open (closed already)
pub fn close(
    conn: &Connection,
    task_id: &str,
    status: AssignmentStatus,
    note: Option<&str>,
    now: i64,
) -> rusqlite::Result<Option<GatewayAssignment>> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    let updated = conn.execute(
        "UPDATE gateway_assignments SET status = ?1, note = ?2, closed_at = ?3
          WHERE task_id = ?4 AND status = 'open'",
        params![status.as_str(), note, now, task_id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    get(conn, task_id)
}

/// Open assignments, soonest due first (then the oldest)
pub fn open_list(conn: &Connection) -> rusqlite::Result<Vec<GatewayAssignment>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM gateway_assignments WHERE status = 'open'
          ORDER BY due_at IS NULL, due_at, created_at"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    rows.map(|row| with_links(conn, row?)).collect()
}

// -- Waiters ------------------------------------------------------------------

/// The gateway task waits on these; closing an assignment wakes its task
#[derive(Default)]
pub struct AssignmentWaiters(Mutex<HashMap<String, oneshot::Sender<AssignmentStatus>>>);

impl AssignmentWaiters {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<AssignmentStatus>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn wait(&self, task_id: &str) -> oneshot::Receiver<AssignmentStatus> {
        let (sender, receiver) = oneshot::channel();
        self.lock().insert(task_id.to_string(), sender);
        receiver
    }

    /// Wakes the task's waiter, if one is waiting in this process
    pub fn wake(&self, task_id: &str, status: AssignmentStatus) {
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

    fn assignment(task_id: &str, created_at: i64, due_at: Option<i64>) -> GatewayAssignment {
        GatewayAssignment {
            task_id: task_id.into(),
            caller_instance: "desktop-sam".into(),
            caller_name: "Sam Lee".into(),
            request: "Upload the SOC 1 evidence".into(),
            status: AssignmentStatus::Open,
            note: None,
            created_at: created_at as f64,
            due_at: due_at.map(|t| t as f64),
            closed_at: None,
            links: Vec::new(),
        }
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn an_assignment_opens_once_and_closes_once() {
        let conn = conn();
        let opened = open(&conn, &assignment("t1", 10, Some(500))).unwrap();
        assert_eq!(opened.status, AssignmentStatus::Open);
        // Resumed after a restart: the same row, the same dates
        let again = open(&conn, &assignment("t1", 99, None)).unwrap();
        assert_eq!((again.created_at, again.due_at), (10.0, Some(500.0)));
        assert_eq!(open_list(&conn).unwrap().len(), 1);

        let done = close(&conn, "t1", AssignmentStatus::Done, Some(" Uploaded "), 20)
            .unwrap()
            .unwrap();
        assert_eq!(done.status, AssignmentStatus::Done);
        assert_eq!(done.note.as_deref(), Some("Uploaded"));
        assert_eq!(done.closed_at, Some(20.0));
        assert_eq!(
            close(&conn, "t1", AssignmentStatus::Declined, None, 30).unwrap(),
            None
        );
        assert!(open_list(&conn).unwrap().is_empty());
        // A resumed task finds the outcome
        assert_eq!(
            open(&conn, &assignment("t1", 99, None)).unwrap().status,
            AssignmentStatus::Done
        );
    }

    #[test]
    fn open_assignments_are_soonest_due_first() {
        let conn = conn();
        open(&conn, &assignment("undated", 1, None)).unwrap();
        open(&conn, &assignment("later", 2, Some(900))).unwrap();
        open(&conn, &assignment("sooner", 3, Some(100))).unwrap();
        let ids: Vec<String> = open_list(&conn)
            .unwrap()
            .into_iter()
            .map(|a| a.task_id)
            .collect();
        assert_eq!(ids, ["sooner", "later", "undated"]);
    }

    #[test]
    fn every_status_reads_back() {
        for status in [
            AssignmentStatus::Open,
            AssignmentStatus::Done,
            AssignmentStatus::Declined,
            AssignmentStatus::Cancelled,
        ] {
            assert_eq!(status.as_str().parse(), Ok(status));
        }
        assert!("nope".parse::<AssignmentStatus>().is_err());
    }

    #[test]
    fn closing_wakes_its_waiter() {
        let waiters = AssignmentWaiters::default();
        let mut receiver = waiters.wait("t1");
        waiters.wake("other", AssignmentStatus::Done);
        assert!(receiver.try_recv().is_err());
        waiters.wake("t1", AssignmentStatus::Declined);
        assert_eq!(receiver.try_recv().unwrap(), AssignmentStatus::Declined);
    }

    #[test]
    fn links_are_kept_in_order_once() {
        let conn = conn();
        let mut first = assignment("t1", 10, None);
        first.links = links_of(&serde_json::json!({ "links": [
            "https://drive.example.com/soc1",
            { "url": "https://wiki.example.com/audit", "name": " Audit checklist " },
        ]}));
        open(&conn, &first).unwrap();
        // Resumed: the links stay as they were
        let mut again = assignment("t1", 99, None);
        again.links = links_of(&serde_json::json!({ "links": ["https://other.example.com"] }));
        let kept = open(&conn, &again).unwrap();
        let names: Vec<&str> = kept.links.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["https://drive.example.com/soc1", "Audit checklist"]);
        assert_eq!(open_list(&conn).unwrap()[0].links, kept.links);
    }

    #[test]
    fn only_web_links_are_attached() {
        let links = links_of(&serde_json::json!({ "links": [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "not a url",
            { "name": "no url" },
            42,
            "https://a.example.com",
            { "url": "https://a.example.com", "name": "again" },
            "http://b.example.com/x",
        ]}));
        let urls: Vec<&str> = links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, ["https://a.example.com", "http://b.example.com/x"]);
        assert!(links_of(&serde_json::json!({ "text": "no links" })).is_empty());
        let many: Vec<String> = (0..20).map(|i| format!("https://e.com/{i}")).collect();
        assert_eq!(
            links_of(&serde_json::json!({ "links": many })).len(),
            MAX_LINKS
        );
    }

    #[test]
    fn long_requests_are_cut() {
        assert_eq!(request_of("  hi  "), "hi");
        let long = request_of(&"x".repeat(5000));
        assert_eq!(long.chars().count(), MAX_REQUEST_CHARS + 1);
    }
}
