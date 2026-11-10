//! A relay's limits per person, as the app tells them.
//!
//! A public relay counts the messages that carry work (a request, a
//! reassignment, a clarification, a group change) per person per UTC day.
//! Past the limit it refuses the message with an `Error` coded
//! [`DAILY_LIMIT`] naming when the count resets (00:00 UTC); a request also
//! gets its `refused` event, a reassignment its `ReassignResult` and a
//! group change a `GroupReply` refused with that code.
//!
//! The client retells each of those refusals as [`daily_limit`]:
//! `daily_limit@<reset, ms since the epoch>: <the relay's words>`, so the
//! UI shows a calm line with the reset time in the user's own clock
//! (`errors.dailyLimit`), wherever the action was taken.

/// The relay's refusal code past the messages a person may send a day
pub const DAILY_LIMIT: &str = "daily_limit";

const DAY_MS: i64 = 24 * 3600 * 1000;

/// A daily-limit refusal as the app passes it on: the reset time the
/// relay named (else the next 00:00 UTC), then its words
pub fn daily_limit(message: &str) -> String {
    let resets = resets_at(message).unwrap_or_else(|| next_utc_midnight(now_ms()));
    format!("{DAILY_LIMIT}@{resets}: {message}")
}

/// The reset time a refusal names ("… It resets at 2026-11-11T00:00:00Z")
fn resets_at(message: &str) -> Option<i64> {
    message.split_whitespace().rev().find_map(|word| {
        let word = word.trim_end_matches(['.', ',', ';', ')']);
        chrono::DateTime::parse_from_rfc3339(word)
            .ok()
            .map(|time| time.timestamp_millis())
    })
}

/// The start of the UTC day after `now_ms`
pub fn next_utc_midnight(now_ms: i64) -> i64 {
    now_ms - now_ms.rem_euclid(DAY_MS) + DAY_MS
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-10T00:00:00Z
    const NOV_10: i64 = 1_794_268_800_000;

    #[test]
    fn the_reset_the_relay_names_is_kept() {
        let said = "You've sent the 500 messages a day this relay allows \
                    (counted across your Nandis). It resets at 2026-11-11T00:00:00Z";
        assert_eq!(
            daily_limit(said),
            format!("daily_limit@{}: {said}", NOV_10 + DAY_MS)
        );
    }

    #[test]
    fn without_one_it_is_the_next_utc_midnight() {
        let at_13_20 = NOV_10 + 13 * 3600 * 1000 + 20 * 60 * 1000;
        assert_eq!(next_utc_midnight(at_13_20), NOV_10 + DAY_MS);
        assert_eq!(next_utc_midnight(NOV_10), NOV_10 + DAY_MS);
        let told = daily_limit("Too many messages today");
        let resets: i64 = told
            .strip_prefix("daily_limit@")
            .and_then(|rest| rest.split(':').next())
            .and_then(|ms| ms.parse().ok())
            .unwrap();
        assert_eq!(resets.rem_euclid(DAY_MS), 0);
        assert!(resets > now_ms());
        assert!(told.ends_with(": Too many messages today"));
    }
}
