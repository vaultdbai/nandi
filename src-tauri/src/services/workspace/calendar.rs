//! Read-only calendars: events, attendees and free/busy, from Outlook
//! (Graph) and Google Calendar, with each service's change feed.
//!
//! | Service  | Read a range                                        | Change feed                                                        | Expired cursor |
//! | -------- | --------------------------------------------------- | ------------------------------------------------------------------ | -------------- |
//! | Outlook  | `GET /me/calendarView?startDateTime&endDateTime`    | `GET /me/calendarView/delta` over the window, then its deltaLink   | 410: restart   |
//! | Google   | `GET calendars/primary/events?timeMin&timeMax`      | `GET calendars/primary/events?timeMin` for nextSyncToken, then `syncToken` | 410: restart |
//!
//! Only `GET` (`email::api`). Nothing is kept but the cursor; events live
//! for one check or one tool call.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::accounts::AccountKind;
use crate::services::email::api::{MailApi, MailError};

/// The change feed's window: from a day ago to two weeks ahead
pub const WINDOW_PAST_DAYS: i64 = 1;
pub const WINDOW_AHEAD_DAYS: i64 = 14;
/// The window moves on: a cursor older than this starts over
const CURSOR_MAX_AGE_SECS: i64 = 6 * 24 * 3600;
const MAX_PAGES: usize = 20;

/// One event as the Chief of Staff and the tools see it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Event {
    /// `<connector>:<id>`
    pub id: String,
    pub title: String,
    /// RFC 3339, or a date for all-day events
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub organizer: Option<String>,
    /// Up to 15 attendees' addresses
    pub attendees: Vec<String>,
    pub attendee_count: usize,
    /// Out of office / time off
    pub time_off: bool,
    pub cancelled: bool,
    /// Shows as busy (free/busy)
    pub busy: bool,
    pub location: Option<String>,
    pub link: Option<String>,
}

/// What a change check returned
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub events: Vec<Event>,
    pub cursor: Value,
    /// The feed started over (first read, or an expired cursor)
    pub restarted: bool,
}

const MAX_ATTENDEES: usize = 15;

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut at = value;
    for key in path {
        at = at.get(*key)?;
    }
    at.as_str().filter(|s| !s.is_empty())
}

fn rfc3339(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Graph's `2026-11-07T15:00:00.0000000` in UTC (we ask for UTC)
fn graph_time(value: &Value, key: &str) -> String {
    str_at(value, &[key, "dateTime"])
        .map(|t| {
            let t = t.split('.').next().unwrap_or(t);
            format!("{t}Z")
        })
        .unwrap_or_default()
}

pub fn outlook_event(kind: AccountKind, item: &Value) -> Option<Event> {
    let raw = item.get("id")?.as_str()?;
    let removed = item.get("@removed").is_some();
    let attendees: Vec<String> = item
        .get("attendees")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| str_at(a, &["emailAddress", "address"]).map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    let show_as = str_at(item, &["showAs"]).unwrap_or("busy");
    Some(Event {
        id: format!("{}:{raw}", kind.id()),
        title: str_at(item, &["subject"]).unwrap_or_default().to_string(),
        start: graph_time(item, "start"),
        end: graph_time(item, "end"),
        all_day: item
            .get("isAllDay")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        organizer: str_at(item, &["organizer", "emailAddress", "address"]).map(str::to_lowercase),
        attendee_count: attendees.len(),
        attendees: attendees.into_iter().take(MAX_ATTENDEES).collect(),
        time_off: show_as == "oof",
        cancelled: removed
            || item
                .get("isCancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        busy: matches!(show_as, "busy" | "oof" | "tentative" | "workingElsewhere"),
        location: str_at(item, &["location", "displayName"]).map(str::to_string),
        link: str_at(item, &["webLink"]).map(str::to_string),
    })
}

pub fn google_event(kind: AccountKind, item: &Value) -> Option<Event> {
    let raw = item.get("id")?.as_str()?;
    let time = |key: &str| {
        str_at(item, &[key, "dateTime"])
            .or_else(|| str_at(item, &[key, "date"]))
            .unwrap_or_default()
            .to_string()
    };
    let attendees: Vec<String> = item
        .get("attendees")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| str_at(a, &["email"]).map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    let event_type = str_at(item, &["eventType"]).unwrap_or("default");
    Some(Event {
        id: format!("{}:{raw}", kind.id()),
        title: str_at(item, &["summary"]).unwrap_or_default().to_string(),
        start: time("start"),
        end: time("end"),
        all_day: str_at(item, &["start", "date"]).is_some(),
        organizer: str_at(item, &["organizer", "email"]).map(str::to_lowercase),
        attendee_count: attendees.len(),
        attendees: attendees.into_iter().take(MAX_ATTENDEES).collect(),
        time_off: event_type == "outOfOffice",
        cancelled: str_at(item, &["status"]) == Some("cancelled"),
        busy: str_at(item, &["transparency"]) != Some("transparent"),
        location: str_at(item, &["location"]).map(str::to_string),
        link: str_at(item, &["htmlLink"]).map(str::to_string),
    })
}

/// One account's calendar
pub struct Calendar<'a> {
    pub kind: AccountKind,
    pub api: &'a MailApi,
    /// Graph, or Google Calendar's base
    pub base: &'a str,
}

const UTC: (&str, &str) = ("Prefer", "outlook.timezone=\"UTC\"");
const GRAPH_SELECT: &str =
    "id,subject,start,end,isAllDay,organizer,attendees,showAs,isCancelled,location,webLink";

impl Calendar<'_> {
    fn is_google(&self) -> bool {
        self.kind == AccountKind::GoogleCalendar
    }

    fn parse(&self, item: &Value) -> Option<Event> {
        if self.is_google() {
            google_event(self.kind, item)
        } else {
            outlook_event(self.kind, item)
        }
    }

    /// Events from `start` to `end` (unix seconds), soonest first
    pub async fn events(&self, start: i64, end: i64, limit: u32) -> Result<Vec<Event>, MailError> {
        let (from, to) = (rfc3339(start), rfc3339(end));
        let top = limit.to_string();
        let url = if self.is_google() {
            self.api.url(
                self.base,
                &["calendars", "primary", "events"],
                &[
                    ("timeMin", from.as_str()),
                    ("timeMax", to.as_str()),
                    ("singleEvents", "true"),
                    ("orderBy", "startTime"),
                    ("maxResults", top.as_str()),
                ],
            )
        } else {
            self.api.url(
                self.base,
                &["me", "calendarView"],
                &[
                    ("startDateTime", from.as_str()),
                    ("endDateTime", to.as_str()),
                    ("$select", GRAPH_SELECT),
                    ("$orderby", "start/dateTime"),
                    ("$top", top.as_str()),
                ],
            )
        };
        let page = self.api.get(&url, &[UTC]).await?;
        let key = if self.is_google() { "items" } else { "value" };
        Ok(page
            .get(key)
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|i| self.parse(i)).collect())
            .unwrap_or_default())
    }

    /// One event, with up to 2,000 characters of its description
    pub async fn event(&self, id: &str) -> Result<(Event, String), MailError> {
        let (url, body_key) = if self.is_google() {
            (
                self.api
                    .url(self.base, &["calendars", "primary", "events", id], &[]),
                vec!["description"],
            )
        } else {
            (
                self.api.url(
                    self.base,
                    &["me", "events", id],
                    &[("$select", &format!("{GRAPH_SELECT},bodyPreview"))],
                ),
                vec!["bodyPreview"],
            )
        };
        let item = self.api.get(&url, &[UTC]).await?;
        let event = self.parse(&item).ok_or(MailError::NotFound)?;
        let body: String = str_at(&item, &body_key)
            .unwrap_or_default()
            .chars()
            .take(2000)
            .collect();
        Ok((event, body))
    }

    /// Changes since `cursor` (`Null`: start over with the window)
    pub async fn changes(&self, cursor: &Value, now: i64) -> Result<Changes, MailError> {
        let fresh = cursor
            .get("at")
            .and_then(Value::as_i64)
            .is_none_or(|at| now - at > CURSOR_MAX_AGE_SECS);
        let link = cursor
            .get("link")
            .and_then(Value::as_str)
            .filter(|_| !fresh);
        let began = cursor
            .get("at")
            .and_then(Value::as_i64)
            .filter(|_| !fresh)
            .unwrap_or(now);
        match self.read_feed(link, now, began).await {
            Err(MailError::CursorExpired) if link.is_some() => self.read_feed(None, now, now).await,
            other => other,
        }
    }

    /// `began`: when this cursor's window began (kept in the cursor)
    async fn read_feed(
        &self,
        link: Option<&str>,
        now: i64,
        began: i64,
    ) -> Result<Changes, MailError> {
        let restarted = link.is_none();
        let start = rfc3339(now - WINDOW_PAST_DAYS * 24 * 3600);
        let end = rfc3339(now + WINDOW_AHEAD_DAYS * 24 * 3600);
        let mut events = Vec::new();
        let mut next = match (link, self.is_google()) {
            (Some(link), _) => link.to_string(),
            (None, true) => self.api.url(
                self.base,
                &["calendars", "primary", "events"],
                &[
                    ("timeMin", start.as_str()),
                    ("singleEvents", "true"),
                    ("maxResults", "250"),
                ],
            ),
            (None, false) => self.api.url(
                self.base,
                &["me", "calendarView", "delta"],
                &[
                    ("startDateTime", start.as_str()),
                    ("endDateTime", end.as_str()),
                ],
            ),
        };
        let mut cursor = Value::Null;
        for _ in 0..MAX_PAGES {
            let page = self
                .api
                .get(&next, &[UTC, ("Prefer", "odata.maxpagesize=100")])
                .await?;
            let key = if self.is_google() { "items" } else { "value" };
            if let Some(items) = page.get(key).and_then(Value::as_array) {
                events.extend(items.iter().filter_map(|i| self.parse(i)));
            }
            if self.is_google() {
                if let Some(token) = page.get("nextPageToken").and_then(Value::as_str) {
                    next = with_param(&next, "pageToken", token);
                    continue;
                }
                if let Some(sync) = page.get("nextSyncToken").and_then(Value::as_str) {
                    let link = self.api.url(
                        self.base,
                        &["calendars", "primary", "events"],
                        &[("syncToken", sync), ("singleEvents", "true")],
                    );
                    cursor = serde_json::json!({ "link": link });
                }
                break;
            }
            if let Some(link) = page.get("@odata.nextLink").and_then(Value::as_str) {
                next = link.to_string();
                continue;
            }
            if let Some(link) = page.get("@odata.deltaLink").and_then(Value::as_str) {
                cursor = serde_json::json!({ "link": link });
            }
            break;
        }
        // Keep when this cursor's window began, so it starts over weekly
        if let Value::Object(map) = &mut cursor {
            map.insert("at".into(), Value::from(began));
        }
        Ok(Changes {
            events,
            cursor,
            restarted,
        })
    }
}

/// `url` with `key=value` set (replacing an earlier one)
fn with_param(url: &str, key: &str, value: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != key)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(kept)
        .append_pair(key, value);
    parsed.to_string()
}

/// Busy intervals from events (free/busy without a POST)
pub fn busy(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e.busy && !e.cancelled)
        .map(|e| {
            serde_json::json!({
                "start": e.start, "end": e.end,
                "status": if e.time_off { "time_off" } else { "busy" },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graph_events_read_time_off_cancellations_and_attendees() {
        let item = json!({
            "id": "e1", "subject": "Quarterly review",
            "start": {"dateTime": "2026-11-09T15:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-11-09T16:00:00.0000000", "timeZone": "UTC"},
            "organizer": {"emailAddress": {"address": "Lena@Contoso.com"}},
            "attendees": [{"emailAddress": {"address": "me@contoso.com"}}],
            "showAs": "oof", "isAllDay": false, "webLink": "https://outlook.office.com/x"
        });
        let event = outlook_event(AccountKind::OutlookCalendar, &item).unwrap();
        assert_eq!(event.id, "outlook-calendar:e1");
        assert_eq!(event.start, "2026-11-09T15:00:00Z");
        assert_eq!(event.organizer.as_deref(), Some("lena@contoso.com"));
        assert!(event.time_off && event.busy && !event.cancelled);
        let removed = outlook_event(
            AccountKind::OutlookCalendar,
            &json!({"id": "e2", "@removed": {"reason": "deleted"}}),
        )
        .unwrap();
        assert!(removed.cancelled);
    }

    #[test]
    fn google_events_read_all_day_out_of_office_and_free_time() {
        let item = json!({
            "id": "g1", "summary": "Vacation", "eventType": "outOfOffice",
            "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-27"},
            "organizer": {"email": "me@gmail.com"}, "transparency": "opaque"
        });
        let event = google_event(AccountKind::GoogleCalendar, &item).unwrap();
        assert!(event.all_day && event.time_off && event.busy);
        assert_eq!(event.start, "2026-11-20");
        let free = google_event(
            AccountKind::GoogleCalendar,
            &json!({"id": "g2", "transparency": "transparent", "status": "cancelled"}),
        )
        .unwrap();
        assert!(!free.busy && free.cancelled);
        assert_eq!(busy(&[event, free]).len(), 1);
    }

    #[test]
    fn page_tokens_replace_the_old_one() {
        let url = with_param("https://x/events?a=1&pageToken=old", "pageToken", "new");
        assert!(url.contains("pageToken=new") && !url.contains("old") && url.contains("a=1"));
    }
}
