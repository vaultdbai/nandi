//! Cron source: one task per enabled cron trigger that sleeps until the
//! schedule's next occurrence (local time), then publishes a fire.
//!
//! - It waits on the wall clock, a minute at a time at most, so a laptop
//!   that slept through 8:00 fires when it wakes, not hours later.
//! - A time missed while the app was closed fires once at the next start
//!   ([`missed`]): the morning digest still comes when the laptop opens at
//!   9. Only the latest missed time, only within [`CATCH_UP_WINDOW`], and
//!   only for schedules an hour or more apart (a five-minute one just
//!   fires next).
//!
//! Schedules use standard POSIX semantics (`0 9 * * 1-5` is 9:00 on
//! weekdays; day-of-week 0 and 7 are Sunday). An optional leading seconds
//! field allows sub-minute schedules (`*/5 * * * * *`).

use std::time::Duration;

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
/// A missed time older than this is left: the next one comes soon enough
pub const CATCH_UP_WINDOW: chrono::TimeDelta = chrono::TimeDelta::hours(12);
/// Schedules closer than this aren't caught up
const CATCH_UP_MIN_GAP: chrono::TimeDelta = chrono::TimeDelta::hours(1);
/// The longest single sleep: the clock is checked again after it
const MAX_SLEEP: Duration = Duration::from_secs(60);

/// The latest time the schedule was due after `last_fired` and by `now`,
/// if one was missed within [`CATCH_UP_WINDOW`]. A trigger that never
/// fired has nothing to catch up.
pub fn missed(
    cron: &Cron,
    last_fired: Option<DateTime<Local>>,
    now: DateTime<Local>,
) -> Option<DateTime<Local>> {
    let mut due = next_after(cron, &last_fired?)?;
    let gap = next_after(cron, &due)? - due;
    if due > now || gap < CATCH_UP_MIN_GAP {
        return None;
    }
    // At most a window's worth of steps, as the gap is an hour or more
    while let Some(later) = next_after(cron, &due).filter(|at| *at <= now) {
        due = later;
    }
    (now - due <= CATCH_UP_WINDOW).then_some(due)
}

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

fn fire(trigger_id: &str, at: DateTime<Local>, caught_up: bool) -> TriggerEvent {
    let mut payload = json!({
        "scheduled_at": at.to_rfc3339(),
        "trigger_id": trigger_id,
    });
    if caught_up {
        payload["caught_up"] = json!(true);
    }
    TriggerEvent {
        trigger_id: trigger_id.to_string(),
        source: TriggerSourceType::Cron,
        payload,
        test: false,
        handover: false,
    }
}

/// Starts the job; abort the handle to stop it (on reload or disable).
/// `catch_up`: a time missed while the app was closed, fired first.
pub fn spawn(
    runtime: &tokio::runtime::Handle,
    trigger_id: String,
    cron: Cron,
    events: mpsc::Sender<TriggerEvent>,
    catch_up: Option<DateTime<Local>>,
) -> AbortHandle {
    runtime
        .spawn(async move {
            if let Some(at) = catch_up {
                log::info!("Cron trigger '{trigger_id}' missed {at}: firing it now");
                if events.send(fire(&trigger_id, at, true)).await.is_err() {
                    return;
                }
            }
            let mut after = Local::now();
            loop {
                let Some(next) = next_after(&cron, &after) else {
                    log::warn!("Cron trigger '{trigger_id}' has no future occurrences");
                    return;
                };
                // Short sleeps on the wall clock: a sleeping laptop stops
                // the timers, not the clock
                while let Ok(left) = (next - Local::now()).to_std() {
                    if left.is_zero() {
                        break;
                    }
                    tokio::time::sleep(left.min(MAX_SLEEP)).await;
                }
                if events.send(fire(&trigger_id, next, false)).await.is_err() {
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
    fn a_time_missed_while_closed_is_caught_up_once() {
        let daily = parse_schedule("0 8 * * *").unwrap();
        let at = |d, h, m| Local.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap();
        // Fired yesterday at 8, opened today at 9: today's 8:00
        assert_eq!(
            missed(&daily, Some(at(16, 8, 0)), at(17, 9, 0)),
            Some(at(17, 8, 0))
        );
        // Closed for three days: only the latest time
        assert_eq!(
            missed(&daily, Some(at(13, 8, 0)), at(16, 9, 30)),
            Some(at(16, 8, 0))
        );
        // Already fired today, or not due yet
        assert_eq!(missed(&daily, Some(at(17, 8, 0)), at(17, 9, 0)), None);
        assert_eq!(missed(&daily, Some(at(16, 8, 0)), at(17, 7, 0)), None);
        // Too late to be of use, or never fired
        assert_eq!(missed(&daily, Some(at(16, 8, 0)), at(17, 21, 0)), None);
        assert_eq!(missed(&daily, None, at(17, 9, 0)), None);
        // Every five minutes: the next one comes soon enough
        let often = parse_schedule("*/5 * * * *").unwrap();
        assert_eq!(missed(&often, Some(at(17, 7, 0)), at(17, 9, 0)), None);
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
