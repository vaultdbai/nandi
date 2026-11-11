//! Cron source: one task per enabled cron trigger that sleeps until the
//! schedule's next occurrence (local time), then publishes a fire.
//!
//! Schedules use standard POSIX semantics (`0 9 * * 1-5` is 9:00 on
//! weekdays; day-of-week 0 and 7 are Sunday). An optional leading seconds
//! field allows sub-minute schedules (`*/5 * * * * *`).

use chrono::{DateTime, Local};
use croner::parser::{CronParser, Seconds};
use croner::Cron;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use super::schema::{TriggerEvent, TriggerSourceType};
use crate::model::Timestamp;

/// Parses a 5-field (or 6-field, seconds first) cron expression.
pub fn parse_schedule(schedule: &str) -> Result<Cron, String> {
    CronParser::builder()
        .seconds(Seconds::Optional)
        .build()
        .parse(schedule.trim())
        .map_err(|e| format!("Invalid cron schedule '{schedule}': {e}"))
}

/// The schedule's next occurrence strictly after `after`.
pub fn next_after(cron: &Cron, after: &DateTime<Local>) -> Option<DateTime<Local>> {
    cron.find_next_occurrence(after, false).ok()
}

/// Next occurrences a preview lists
pub const PREVIEW_COUNT: usize = 3;

/// A schedule checked as the user types: whether it parses and when it
/// would fire next. Uses the scheduler's own parser, so they always agree.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CronPreview {
    pub valid: bool,
    /// Why it doesn't parse (the parser's words, for Details)
    pub error: Option<String>,
    /// The next occurrences (epoch ms, local time zone)
    pub next: Vec<Timestamp>,
}

pub fn preview(schedule: &str, after: DateTime<Local>, count: usize) -> CronPreview {
    match parse_schedule(schedule) {
        Err(error) => CronPreview {
            valid: false,
            error: Some(error),
            next: Vec::new(),
        },
        Ok(cron) => {
            let mut next = Vec::with_capacity(count);
            let mut cursor = after;
            while next.len() < count {
                let Some(at) = next_after(&cron, &cursor) else {
                    break;
                };
                next.push(at.timestamp_millis() as Timestamp);
                cursor = at;
            }
            CronPreview {
                valid: true,
                error: None,
                next,
            }
        }
    }
}

/// Starts the job; abort the handle to stop it (on reload or disable).
pub fn spawn(
    runtime: &tokio::runtime::Handle,
    trigger_id: String,
    cron: Cron,
    events: mpsc::Sender<TriggerEvent>,
) -> AbortHandle {
    runtime
        .spawn(async move {
            let mut after = Local::now();
            loop {
                let Some(next) = next_after(&cron, &after) else {
                    log::warn!("Cron trigger '{trigger_id}' has no future occurrences");
                    return;
                };
                let wait = (next - Local::now()).to_std().unwrap_or_default();
                tokio::time::sleep(wait).await;
                let event = TriggerEvent {
                    trigger_id: trigger_id.clone(),
                    source: TriggerSourceType::Cron,
                    payload: json!({
                        "scheduled_at": next.to_rfc3339(),
                        "trigger_id": trigger_id,
                    }),
                    test: false,
                    handover: false,
                };
                if events.send(event).await.is_err() {
                    return; // engine stopped
                }
                // Never fire the same occurrence twice, even if the sleep
                // woke a little early
                after = next.max(Local::now());
            }
        })
        .abort_handle()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Timelike, Weekday};

    #[test]
    fn parses_standard_and_seconds_schedules() {
        assert!(parse_schedule("0 9 * * 1-5").is_ok());
        assert!(parse_schedule("*/5 * * * * *").is_ok());
        assert!(parse_schedule("@daily").is_ok());
        for bad in ["", "nope", "61 * * * *", "* * * *"] {
            assert!(parse_schedule(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn weekday_schedules_skip_the_weekend() {
        let cron = parse_schedule("0 9 * * 1-5").unwrap();
        // Friday 2026-10-09 10:00 → next is Monday 09:00
        let friday = Local.with_ymd_and_hms(2026, 10, 9, 10, 0, 0).unwrap();
        let next = next_after(&cron, &friday).unwrap();
        assert_eq!(next.weekday(), Weekday::Mon);
        assert_eq!((next.hour(), next.minute(), next.second()), (9, 0, 0));
        assert_eq!(next.day(), 12);
    }

    #[test]
    fn previews_list_the_next_runs_or_say_why_not() {
        let friday = Local.with_ymd_and_hms(2026, 10, 9, 10, 0, 0).unwrap();
        let p = preview("0 9 * * 1-5", friday, PREVIEW_COUNT);
        assert!(p.valid && p.error.is_none());
        let days: Vec<u32> = p
            .next
            .iter()
            .map(|ms| Local.timestamp_millis_opt(*ms as i64).unwrap().day())
            .collect();
        // Mon 12, Tue 13, Wed 14: the weekend is skipped
        assert_eq!(days, [12, 13, 14]);

        let bad = preview("61 * * * *", friday, PREVIEW_COUNT);
        assert!(!bad.valid && bad.next.is_empty());
        assert!(bad.error.unwrap().contains("Invalid cron schedule"));
    }

    #[test]
    fn seconds_schedules_tick_every_second() {
        let cron = parse_schedule("* * * * * *").unwrap();
        let start = Local.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
        let first = next_after(&cron, &start).unwrap();
        assert_eq!((first - start).num_seconds(), 1);
        assert_eq!(
            (next_after(&cron, &first).unwrap() - first).num_seconds(),
            1
        );
    }
}
