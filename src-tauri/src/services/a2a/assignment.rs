//! Work assigned to a person (`assign-me`): it lasts days, not minutes.
//!
//! The requester's tool returns once the assignment reached the person (or
//! waits in the offline queue for them); the task stays open until they
//! mark it Done or Decline it in their Inbox (`gateway::assignments`). Its
//! deadline is the due date, sweeps leave it alone, it never fails over to
//! someone else, and the result comes back however late: pushed when it's
//! done, or fetched by the requester's status poll after either side was
//! offline.
//!
//! Every other request to another Nandi has a due date too
//! ([`request_due_at`]): a question (`ask-me`) is due in
//! [`DEFAULT_ASK_DUE_DAYS`], anything else in [`DEFAULT_REQUEST_DUE_DAYS`],
//! unless the request names one. The requester waits live for a while; if
//! no answer came by then, the request stays open (queued for someone
//! offline, or waiting on them) until its due date, and the answer is kept
//! whenever it arrives.

use std::time::Duration;

use chrono::{Local, NaiveDate, TimeZone};
use serde_json::Value;

/// The capability of work assigned to a person
pub const ASSIGN_ME: &str = "assign-me";

/// Due when the request names no date
pub const DEFAULT_DUE_DAYS: i64 = 7;

/// A question to a person (`ask-me`) without a date is due in this many days
pub const DEFAULT_ASK_DUE_DAYS: i64 = 2;

/// Any other request to another Nandi without a date
pub const DEFAULT_REQUEST_DUE_DAYS: i64 = 1;

/// A question to a person
pub use crate::services::relay::protocol::ASK_ME;

/// How long the requester's tool waits to see the assignment taken (or
/// refused) before it returns "assigned"
pub const ACCEPT_WINDOW: Duration = Duration::from_secs(10);

/// How often the requester asks after its open assignments
pub const POLL_EVERY_MS: i64 = 60_000;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Whether tasks of `capability` last until a person ends them
pub fn is_lasting(capability: &str) -> bool {
    capability == ASSIGN_ME
}

/// The due date (epoch ms) of an assignment: `input.due` as `YYYY-MM-DD`
/// (the end of that day, local time), an RFC 3339 time, or `N days`; else
/// [`DEFAULT_DUE_DAYS`] from `now`. Never before an hour from now.
pub fn due_at(input: &Value, now: i64) -> i64 {
    let named = input
        .get("due")
        .and_then(Value::as_str)
        .and_then(|due| parse_due(due.trim(), now));
    named
        .unwrap_or(now + DEFAULT_DUE_DAYS * DAY_MS)
        .max(now + 60 * 60 * 1000)
}

/// The due date (epoch ms) of a request to another Nandi: an
/// assignment's ([`due_at`]), else `input.due`, else
/// [`DEFAULT_ASK_DUE_DAYS`] for a question and [`DEFAULT_REQUEST_DUE_DAYS`]
/// for anything else. Never before an hour from now.
pub fn request_due_at(capability: &str, input: &Value, now: i64) -> i64 {
    if is_lasting(capability) {
        return due_at(input, now);
    }
    let days = if capability == ASK_ME {
        DEFAULT_ASK_DUE_DAYS
    } else {
        DEFAULT_REQUEST_DUE_DAYS
    };
    input
        .get("due")
        .and_then(Value::as_str)
        .and_then(|due| parse_due(due.trim(), now))
        .unwrap_or(now + days * DAY_MS)
        .max(now + 60 * 60 * 1000)
}

fn parse_due(due: &str, now: i64) -> Option<i64> {
    if let Ok(date) = NaiveDate::parse_from_str(due, "%Y-%m-%d") {
        let end = date.and_hms_opt(23, 59, 59)?;
        return Local
            .from_local_datetime(&end)
            .earliest()
            .map(|t| t.timestamp_millis());
    }
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(due) {
        return Some(time.timestamp_millis());
    }
    let days = due
        .strip_suffix("days")
        .or_else(|| due.strip_suffix("day"))
        .or_else(|| due.strip_suffix('d'))?
        .trim()
        .parse::<i64>()
        .ok()?;
    Some(now + days * DAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn only_assignments_last() {
        assert!(is_lasting("assign-me"));
        assert!(!is_lasting("ask-me"));
    }

    #[test]
    fn the_due_date_comes_from_the_request() {
        assert_eq!(due_at(&json!({}), NOW), NOW + 7 * DAY_MS);
        assert_eq!(due_at(&json!({ "due": "3 days" }), NOW), NOW + 3 * DAY_MS);
        assert_eq!(due_at(&json!({ "due": "2d" }), NOW), NOW + 2 * DAY_MS);
        let at = due_at(&json!({ "due": "2027-01-15T12:00:00Z" }), NOW);
        assert_eq!(
            at,
            chrono::DateTime::parse_from_rfc3339("2027-01-15T12:00:00Z")
                .unwrap()
                .timestamp_millis()
        );
        // The end of the day, local time
        let day = due_at(&json!({ "due": "2027-01-15" }), NOW);
        let local = Local.timestamp_millis_opt(day).unwrap();
        assert_eq!(
            local.format("%Y-%m-%d %H:%M").to_string(),
            "2027-01-15 23:59"
        );
    }

    #[test]
    fn every_request_to_another_nandi_has_a_due_date() {
        assert_eq!(request_due_at("ask-me", &json!({}), NOW), NOW + 2 * DAY_MS);
        assert_eq!(request_due_at("review-pr", &json!({}), NOW), NOW + DAY_MS);
        assert_eq!(
            request_due_at("ask-me", &json!({ "due": "5 days" }), NOW),
            NOW + 5 * DAY_MS
        );
        assert_eq!(
            request_due_at("assign-me", &json!({}), NOW),
            NOW + 7 * DAY_MS
        );
        assert_eq!(
            request_due_at("ask-me", &json!({ "due": "2001-01-01" }), NOW),
            NOW + 3_600_000
        );
    }

    #[test]
    fn a_bad_or_past_due_date_falls_back() {
        assert_eq!(due_at(&json!({ "due": "soon" }), NOW), NOW + 7 * DAY_MS);
        assert_eq!(due_at(&json!({ "due": 5 }), NOW), NOW + 7 * DAY_MS);
        // Never before an hour from now
        assert_eq!(
            due_at(&json!({ "due": "2001-01-01" }), NOW),
            NOW + 3_600_000
        );
    }
}
