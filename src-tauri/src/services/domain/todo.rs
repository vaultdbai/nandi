//! A to-do's rules (`services::gateway::todos`): something the user must do,
//! check or remember, kept as an assignment to the user. It lives in
//! `state.db`, not the gold copy, but its rules are pure like the rest of
//! the domain: the text, its kind, where it came from, and the due time.

use chrono::{NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{EnumString, IntoStaticStr};

use super::error::{DomainError, DomainResult};

/// Longest to-do text, in characters: one sentence, not a document
pub const MAX_TEXT_CHARS: usize = 400;
/// Longest note on a done to-do, and longest reference
pub const MAX_NOTE_CHARS: usize = 1000;
pub const MAX_REF_CHARS: usize = 500;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What kind of to-do it is
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Type,
    EnumString,
    IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum TodoKind {
    /// An action, usually with a deadline (all work a person assigns)
    #[default]
    Do,
    /// Something to verify and close, usually without a deadline
    Check,
    /// A date to remember
    Reminder,
}

/// Where a to-do came from
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum TodoSource {
    /// An email (`source_ref`: its message ID)
    Email,
    /// A calendar event (`source_ref`: its ID)
    Event,
    /// A chat with Nandi
    Chat,
    /// The user added it by hand
    Manual,
}

impl TodoKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

impl TodoSource {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Where a to-do was read, and what Nandi prepared for it
#[derive(Debug, Clone, Copy, Default)]
pub struct TodoRefs<'a> {
    pub source: Option<&'a str>,
    pub related: Option<&'a str>,
}

/// A to-do about to be added, validated
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTodo {
    pub text: String,
    pub kind: TodoKind,
    /// Epoch ms
    pub due_at: Option<i64>,
    pub source: TodoSource,
    /// The same place read again names the same to-do
    pub source_ref: Option<String>,
    /// An Inbox item Nandi prepared for it (an approval, a Request), by its
    /// thread ID (`inbox:<kind>:<id>`)
    pub related_ref: Option<String>,
}

impl NewTodo {
    /// Checks the text and references and reads `due` ([`parse_due`] in
    /// `tz`). A `due` that isn't one of the accepted forms is refused, so
    /// a deadline is never dropped quietly.
    pub fn new<Tz: TimeZone>(
        text: &str,
        kind: TodoKind,
        due: Option<&str>,
        source: TodoSource,
        refs: TodoRefs<'_>,
        now: i64,
        tz: &Tz,
    ) -> DomainResult<Self> {
        let due = due.map(str::trim).filter(|d| !d.is_empty());
        let due_at = match due {
            Some(due) => Some(parse_due(due, now, tz).ok_or_else(|| {
                DomainError::Rule(format!(
                    "Couldn't read the due time '{due}': use YYYY-MM-DD, \"N days\" or an RFC 3339 time"
                ))
            })?),
            None => None,
        };
        Ok(Self {
            text: todo_text(text)?,
            kind,
            due_at,
            source,
            source_ref: reference("source_ref", refs.source)?,
            related_ref: reference("related_ref", refs.related)?,
        })
    }
}

/// A reference: trimmed, empty is none, at most [`MAX_REF_CHARS`]
fn reference(field: &str, value: Option<&str>) -> DomainResult<Option<String>> {
    let value = value.map(str::trim).filter(|r| !r.is_empty());
    if value.is_some_and(|r| r.chars().count() > MAX_REF_CHARS) {
        return Err(DomainError::Rule(format!(
            "A to-do's {field} is at most {MAX_REF_CHARS} characters"
        )));
    }
    Ok(value.map(str::to_string))
}

/// The text of a to-do: trimmed, not empty, at most [`MAX_TEXT_CHARS`]
pub fn todo_text(text: &str) -> DomainResult<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(DomainError::Rule("A to-do needs some text".to_string()));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(DomainError::Rule(format!(
            "A to-do is at most {MAX_TEXT_CHARS} characters: say the one thing to do"
        )));
    }
    Ok(text.to_string())
}

/// A note on a done to-do: trimmed, empty is none, cut to [`MAX_NOTE_CHARS`]
pub fn todo_note(note: Option<&str>) -> Option<String> {
    note.map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(MAX_NOTE_CHARS).collect())
}

/// A due time (epoch ms) from `YYYY-MM-DD` (the end of that day in `tz`),
/// an RFC 3339 time, or `N days` (`N day`, `Nd`) from `now`
pub fn parse_due<Tz: TimeZone>(due: &str, now: i64, tz: &Tz) -> Option<i64> {
    let due = due.trim();
    if let Ok(date) = NaiveDate::parse_from_str(due, "%Y-%m-%d") {
        let end = date.and_hms_opt(23, 59, 59)?;
        return tz
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
    use chrono::Utc;

    const NOW: i64 = 1_800_000_000_000;

    fn new(text: &str, due: Option<&str>, refs: TodoRefs<'_>) -> DomainResult<NewTodo> {
        NewTodo::new(text, TodoKind::Do, due, TodoSource::Email, refs, NOW, &Utc)
    }

    #[test]
    fn due_reads_a_date_a_time_or_days() {
        assert_eq!(parse_due("3 days", NOW, &Utc), Some(NOW + 3 * DAY_MS));
        assert_eq!(parse_due("1 day", NOW, &Utc), Some(NOW + DAY_MS));
        assert_eq!(parse_due("2d", NOW, &Utc), Some(NOW + 2 * DAY_MS));
        let end_of_day = Utc
            .with_ymd_and_hms(2027, 1, 15, 23, 59, 59)
            .unwrap()
            .timestamp_millis();
        assert_eq!(parse_due("2027-01-15", NOW, &Utc), Some(end_of_day));
        let noon = Utc
            .with_ymd_and_hms(2027, 1, 15, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(parse_due("2027-01-15T12:00:00Z", NOW, &Utc), Some(noon));
        assert_eq!(parse_due("next Friday", NOW, &Utc), None);
        assert_eq!(parse_due("", NOW, &Utc), None);
    }

    #[test]
    fn a_new_todo_is_checked() {
        let refs = TodoRefs {
            source: Some(" msg-1 "),
            related: Some("inbox:approval:a1"),
        };
        let todo = new("  Reply to the landlord  ", Some("2 days"), refs).unwrap();
        assert_eq!(todo.text, "Reply to the landlord");
        assert_eq!(todo.kind, TodoKind::Do);
        assert_eq!(todo.due_at, Some(NOW + 2 * DAY_MS));
        assert_eq!(todo.source_ref.as_deref(), Some("msg-1"));
        assert_eq!(todo.related_ref.as_deref(), Some("inbox:approval:a1"));

        let blank = TodoRefs {
            source: Some(""),
            related: None,
        };
        let plain = new("Call the dentist", Some(" "), blank).unwrap();
        assert_eq!((plain.due_at, plain.source_ref), (None, None));
    }

    #[test]
    fn bad_text_due_or_refs_are_refused() {
        let error = |text: &str, due: Option<&str>| {
            new(text, due, TodoRefs::default()).unwrap_err().to_string()
        };
        assert!(error("   ", None).contains("needs some text"));
        assert!(error(&"x".repeat(MAX_TEXT_CHARS + 1), None).contains("at most"));
        assert!(error("Pay rent", Some("soonish")).contains("Couldn't read the due time"));
        let long = "r".repeat(MAX_REF_CHARS + 1);
        let refs = TodoRefs {
            source: None,
            related: Some(&long),
        };
        let error = new("Pay", None, refs).unwrap_err().to_string();
        assert!(error.contains("related_ref"), "{error}");
    }

    #[test]
    fn a_note_is_trimmed_and_capped() {
        assert_eq!(todo_note(Some("  sent  ")).as_deref(), Some("sent"));
        assert_eq!(todo_note(Some("   ")), None);
        assert_eq!(todo_note(None), None);
        let long = "n".repeat(MAX_NOTE_CHARS + 5);
        assert_eq!(
            todo_note(Some(&long)).unwrap().chars().count(),
            MAX_NOTE_CHARS
        );
    }

    #[test]
    fn kinds_and_sources_are_stored_as_words() {
        assert_eq!(TodoKind::default(), TodoKind::Do);
        assert_eq!(TodoKind::Reminder.as_str(), "reminder");
        assert_eq!(TodoSource::Manual.as_str(), "manual");
        assert_eq!("event".parse::<TodoSource>().unwrap(), TodoSource::Event);
        assert_eq!("check".parse::<TodoKind>().unwrap(), TodoKind::Check);
    }
}
