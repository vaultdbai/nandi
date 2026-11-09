//! Requests that came back while nobody watched: an answer, Done or Decline, or a request that ended at its due
//! date, recorded when the requester's history row closes after the live
//! wait (`history::close`). Each one is an Inbox item ("Lena answered: …")
//! until the user dismisses it, and a `request://returned` event, which the
//! companion turns into a notification unless "Notify me when requests
//! come back" is off.
//!
//! Answers that came back during the live wait are already in the chat, so
//! they aren't recorded here.

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::assignment::ASSIGN_ME;
use super::{TaskResult, TaskStatus};
use crate::db::{now_ms, Db};
use crate::services::events::NandiEvent;

/// How long an undismissed return stays in the Inbox
pub const KEPT_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// The setting behind "Notify me when requests come back" (on by default)
pub const NOTIFY_RESULTS_KEY: &str = "companion.notifyResults";

/// Longest summary kept
const SUMMARY_CHARS: usize = 280;

/// How a request came back
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReturnOutcome {
    /// A question or request answered
    Answered,
    /// Assigned work marked Done
    Done,
    /// Turned down by the person
    Declined,
    /// Not answered by its due date
    Expired,
    /// Ended with an error
    Failed,
}

impl ReturnOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Done => "done",
            Self::Declined => "declined",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "answered" => Self::Answered,
            "done" => Self::Done,
            "declined" => Self::Declined,
            "expired" => Self::Expired,
            _ => Self::Failed,
        }
    }
}

/// One request that came back
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestReturn {
    pub task_id: String,
    /// Who answered: their display name from the network roster
    pub from_name: String,
    /// Provider card ID
    pub provider_agent: String,
    pub capability: String,
    pub outcome: ReturnOutcome,
    /// The answer, the person's note, or why it ended (one paragraph)
    pub summary: String,
    /// What was asked (one paragraph)
    pub request: String,
    pub returned_at: f64,
}

/// `request://returned`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestReturned {
    pub item: RequestReturn,
}

impl NandiEvent for RequestReturned {
    const NAME: &'static str = "request://returned";
}

/// What a closed outbound `result` looks like as a return
pub fn from_result(result: &TaskResult, from_name: &str) -> RequestReturn {
    let error = result.error.clone().unwrap_or_default();
    let outcome = match result.status {
        TaskStatus::Completed if result.capability == ASSIGN_ME => ReturnOutcome::Done,
        TaskStatus::Completed => ReturnOutcome::Answered,
        TaskStatus::TimedOut if error.contains("due date") => ReturnOutcome::Expired,
        _ if error.to_lowercase().contains("declined") => ReturnOutcome::Declined,
        _ => ReturnOutcome::Failed,
    };
    let summary = match outcome {
        ReturnOutcome::Answered | ReturnOutcome::Done => text_of(&result.output),
        _ => error,
    };
    RequestReturn {
        task_id: result.task_id.clone(),
        from_name: from_name.to_string(),
        provider_agent: result.provider_agent.clone(),
        capability: result.capability.clone(),
        outcome,
        summary: shorten(&summary),
        request: shorten(&text_of(&result.input)),
        returned_at: now_ms() as f64,
    }
}

/// The readable part of a task's input or output: its `text`, `answer`,
/// `note` or `summary`, a plain string, else compact JSON
pub(crate) fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Object(map) => ["text", "answer", "note", "summary", "output"]
            .iter()
            .find_map(|key| map.get(*key))
            .map(text_of)
            .unwrap_or_else(|| value.to_string()),
        other => other.to_string(),
    }
}

fn shorten(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= SUMMARY_CHARS {
        return text.to_string();
    }
    let mut short: String = text.chars().take(SUMMARY_CHARS - 1).collect();
    short.push('…');
    short
}

pub fn insert(conn: &Connection, item: &RequestReturn) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO request_returns
            (task_id, from_name, provider_agent, capability, outcome, summary, request, returned_at, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)
         ON CONFLICT(task_id) DO UPDATE SET
            from_name = excluded.from_name, outcome = excluded.outcome,
            summary = excluded.summary, returned_at = excluded.returned_at, seen_at = NULL",
        params![
            item.task_id,
            item.from_name,
            item.provider_agent,
            item.capability,
            item.outcome.as_str(),
            item.summary,
            item.request,
            item.returned_at,
        ],
    )?;
    // Keep the table small
    conn.execute(
        "DELETE FROM request_returns WHERE returned_at < ?1",
        [(now_ms() - KEPT_MS) as f64],
    )?;
    Ok(())
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<RequestReturn> {
    Ok(RequestReturn {
        task_id: row.get(0)?,
        from_name: row.get(1)?,
        provider_agent: row.get(2)?,
        capability: row.get(3)?,
        outcome: ReturnOutcome::parse(&row.get::<_, String>(4)?),
        summary: row.get(5)?,
        request: row.get(6)?,
        returned_at: row.get(7)?,
    })
}

/// Returns not yet dismissed, newest first
pub async fn list(db: &Db) -> Result<Vec<RequestReturn>, String> {
    db.call(|conn| {
        let mut stmt = conn.prepare(
            "SELECT task_id, from_name, provider_agent, capability, outcome, summary, request, returned_at
               FROM request_returns WHERE seen_at IS NULL AND returned_at >= ?1
              ORDER BY returned_at DESC",
        )?;
        let rows = stmt.query_map([(now_ms() - KEPT_MS) as f64], from_row)?;
        rows.collect()
    })
    .await
}

/// The user takes one out of the Inbox (recorded in the decisions file)
pub async fn dismiss_request_return(
    state: &crate::services::AppState,
    task_id: String,
) -> Result<(), String> {
    dismiss(state.db(), task_id.clone()).await?;
    crate::services::inbox::audit::record(
        state,
        crate::services::inbox::audit::Decided::new("requestReturn", task_id, "dismissed"),
    );
    Ok(())
}

/// Takes one out of the Inbox
pub async fn dismiss(db: &Db, task_id: String) -> Result<(), String> {
    db.call(move |conn| {
        conn.execute(
            "UPDATE request_returns SET seen_at = ?2 WHERE task_id = ?1",
            params![task_id, now_ms() as f64],
        )?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::a2a::DelegationDirection;
    use serde_json::json;

    fn result(
        capability: &str,
        status: TaskStatus,
        output: Value,
        error: Option<&str>,
    ) -> TaskResult {
        TaskResult {
            task_id: "t1".into(),
            provider_agent: "lena/gateway".into(),
            output,
            status,
            duration_ms: 1,
            tokens_in: 0,
            tokens_out: 0,
            error: error.map(str::to_string),
            direction: DelegationDirection::Outbound,
            capability: capability.into(),
            requester_agent: "me/chief-of-staff".into(),
            provider_owner: None,
            requester_owner: None,
            input: json!({"text": "When is the SOC 1 walkthrough?"}),
            started_at: 0.0,
            ended_at: Some(1.0),
        }
    }

    #[test]
    fn each_ending_reads_as_what_happened() {
        let answered = from_result(
            &result(
                "ask-me",
                TaskStatus::Completed,
                json!({"text": "Nov 12"}),
                None,
            ),
            "Lena Fischer",
        );
        assert_eq!(answered.outcome, ReturnOutcome::Answered);
        assert_eq!(answered.summary, "Nov 12");
        assert_eq!(answered.request, "When is the SOC 1 walkthrough?");
        assert_eq!(answered.from_name, "Lena Fischer");

        let done = from_result(
            &result(
                "assign-me",
                TaskStatus::Completed,
                json!({"status": "done", "note": "Rotated"}),
                None,
            ),
            "Sam",
        );
        assert_eq!(done.outcome, ReturnOutcome::Done);
        assert_eq!(done.summary, "Rotated");

        let expired = from_result(
            &result(
                "ask-me",
                TaskStatus::TimedOut,
                Value::Null,
                Some("No answer by its due date"),
            ),
            "Lena",
        );
        assert_eq!(expired.outcome, ReturnOutcome::Expired);

        let declined = from_result(
            &result(
                "assign-me",
                TaskStatus::Failed,
                Value::Null,
                Some("Lena declined this"),
            ),
            "Lena",
        );
        assert_eq!(declined.outcome, ReturnOutcome::Declined);
    }

    #[test]
    fn long_answers_are_shortened() {
        let long = "x".repeat(1000);
        let item = from_result(
            &result("ask-me", TaskStatus::Completed, json!(long), None),
            "L",
        );
        assert_eq!(item.summary.chars().count(), SUMMARY_CHARS);
        assert!(item.summary.ends_with('…'));
    }

    #[tokio::test]
    async fn returns_are_listed_until_dismissed() {
        let db = Db::in_memory();
        let item = from_result(
            &result("ask-me", TaskStatus::Completed, json!("Nov 12"), None),
            "Lena",
        );
        let stored = item.clone();
        db.call(move |conn| insert(conn, &stored)).await.unwrap();
        assert_eq!(list(&db).await.unwrap(), vec![item]);
        dismiss(&db, "t1".into()).await.unwrap();
        assert!(list(&db).await.unwrap().is_empty());
    }
}
