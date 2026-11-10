//! When the know-me to-do is in the Inbox. Pure: the service loads and
//! saves it (`local_settings`, key [`KEY`]).
//!
//! - The first one comes after onboarding ([`Cadence::start`]).
//! - **Later** hides it for a week; it comes back as the same to-do.
//! - **Skip** hides it for a month.
//! - A confirmed change ("answered") closes it; it comes back after three
//!   months to ask whether it's all still right.
//! - It never comes back (as a new to-do) sooner than a month after it
//!   last came, so it never nags. An open to-do the person ignores just
//!   stays, quietly: it isn't counted in the Inbox badge.
//!
//! Why `local_settings` and not the gold copy: this is when a reminder
//! last showed on this computer, runtime state like the onboarding step.
//! It defines nothing, and losing it costs at most one early to-do. The
//! questions themselves are content (the Chief of Staff's `know-me`
//! playbook), and the answers are the person's files (NANDI.md, My notes).

use serde::{Deserialize, Serialize};

/// The `local_settings` key
pub const KEY: &str = "knowMe.cadence";

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Later: back in a week
pub const LATER_MS: i64 = 7 * DAY_MS;
/// Skip, and the shortest time between two to-dos
pub const MONTH_MS: i64 = 30 * DAY_MS;
/// After an answer: "is this still right?" in three months
pub const RECHECK_MS: i64 = 90 * DAY_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cadence {
    /// The to-do is in the Inbox now
    pub open: bool,
    /// When a to-do last came into the Inbox (Later doesn't move it);
    /// `None` before the first one
    pub shown_at: Option<i64>,
    /// When a closed to-do comes back
    pub next_at: Option<i64>,
    /// Closed with Later: it comes back as the same to-do
    pub snoozed: bool,
    /// When the person last confirmed a change
    pub answered_at: Option<i64>,
}

impl Cadence {
    /// The first to-do, once (after onboarding)
    pub fn start(self, now: i64) -> Self {
        if self.shown_at.is_some() {
            return self;
        }
        Self {
            open: true,
            shown_at: Some(now),
            next_at: None,
            snoozed: false,
            ..self
        }
    }

    /// A closed to-do whose time came is open again
    pub fn tick(self, now: i64) -> Self {
        let due = !self.open && self.next_at.is_some_and(|at| at <= now);
        if !due {
            return self;
        }
        // Never twice in a month, whatever `next_at` says
        let last = self.shown_at.unwrap_or(i64::MIN / 2);
        if !self.snoozed && now < last + MONTH_MS {
            return self;
        }
        Self {
            open: true,
            shown_at: if self.snoozed {
                self.shown_at
            } else {
                Some(now)
            },
            next_at: None,
            snoozed: false,
            ..self
        }
    }

    /// Later: the same to-do in a week
    pub fn later(self, now: i64) -> Self {
        Self {
            open: false,
            next_at: Some(now + LATER_MS),
            snoozed: true,
            ..self
        }
    }

    /// Skip: a to-do again in a month at the soonest
    pub fn skip(self, now: i64) -> Self {
        let last = self.shown_at.unwrap_or(now);
        Self {
            open: false,
            next_at: Some((now + MONTH_MS).max(last + MONTH_MS)),
            snoozed: false,
            ..self
        }
    }

    /// The person confirmed a change: closed, back in three months
    pub fn answered(self, now: i64) -> Self {
        Self {
            open: false,
            shown_at: self.shown_at.or(Some(now)),
            next_at: Some(now + RECHECK_MS),
            snoozed: false,
            answered_at: Some(now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn the_first_to_do_comes_once() {
        let started = Cadence::default().start(NOW);
        assert!(started.open);
        assert_eq!(started.shown_at, Some(NOW));
        // Onboarding again doesn't make another
        let skipped = started.skip(NOW + DAY_MS);
        assert_eq!(skipped.start(NOW + 2 * DAY_MS), skipped);
    }

    #[test]
    fn a_skipped_to_do_comes_back_a_month_later_and_not_before() {
        let skipped = Cadence::default().start(NOW).skip(NOW + DAY_MS);
        assert!(!skipped.open);
        assert!(!skipped.tick(NOW + 20 * DAY_MS).open);
        assert!(!skipped.tick(NOW + 30 * DAY_MS).open);
        let back = skipped.tick(NOW + 31 * DAY_MS);
        assert!(back.open);
        assert_eq!(back.shown_at, Some(NOW + 31 * DAY_MS));
    }

    #[test]
    fn later_brings_back_the_same_to_do_in_a_week() {
        let later = Cadence::default().start(NOW).later(NOW);
        assert!(!later.tick(NOW + 6 * DAY_MS).open);
        let back = later.tick(NOW + LATER_MS);
        assert!(back.open);
        // The same to-do: it didn't count as a new one
        assert_eq!(back.shown_at, Some(NOW));
        // Skipping it then waits a month from now
        let skipped = back.skip(NOW + LATER_MS);
        assert_eq!(skipped.next_at, Some(NOW + LATER_MS + MONTH_MS));
    }

    #[test]
    fn at_most_one_new_to_do_a_month() {
        // A `next_at` hand-set too soon still waits a month from the last
        let mut early = Cadence::default().start(NOW).skip(NOW);
        early.next_at = Some(NOW + DAY_MS);
        assert!(!early.tick(NOW + 2 * DAY_MS).open);
        assert!(early.tick(NOW + MONTH_MS).open);
    }

    #[test]
    fn an_answer_closes_it_until_the_recheck() {
        let answered = Cadence::default().start(NOW).answered(NOW + DAY_MS);
        assert!(!answered.open);
        assert_eq!(answered.answered_at, Some(NOW + DAY_MS));
        assert!(!answered.tick(NOW + 60 * DAY_MS).open);
        assert!(answered.tick(NOW + DAY_MS + RECHECK_MS).open);
        // Never started (answered from What you offer): no to-do now
        let direct = Cadence::default().answered(NOW);
        assert!(!direct.open);
        assert_eq!(direct.start(NOW), direct);
    }
}
