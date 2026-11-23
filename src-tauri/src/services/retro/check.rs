//! The check (slice 3): did an applied change help? For the changed
//! agent, the share of the person's decisions that were corrections (a
//! refusal, or a draft they changed before it went out) over its runs
//! after the change, against the four weeks before. Computed from run
//! records alone ([`DecisionRecord`]), never from what the model says.
//!
//! The verdict is set once, when the agent has had [`MIN_DECISIONS`]
//! decisions since the change. Four weeks without enough is `unclear`.
//! A change that didn't help gets one calm Inbox note offering the undo.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::records::DecisionRecord;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How far back "before" looks
pub const BEFORE_MS: i64 = 28 * DAY_MS;
/// How long "after" waits for enough decisions
pub const AFTER_MS: i64 = 28 * DAY_MS;
/// Decisions after the change before there's a verdict
pub const MIN_DECISIONS: u32 = 5;

/// Decisions and how many were corrections
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RetroRate {
    pub decisions: u32,
    pub corrections: u32,
}

impl RetroRate {
    /// Over `decisions`
    pub fn of<'a>(decisions: impl IntoIterator<Item = &'a DecisionRecord>) -> Self {
        let mut rate = Self::default();
        for decision in decisions {
            rate.decisions += 1;
            if decision.is_correction() {
                rate.corrections += 1;
            }
        }
        rate
    }

    /// Corrections per decision, 0 to 1 (0 with no decisions)
    pub fn value(&self) -> f64 {
        if self.decisions == 0 {
            0.0
        } else {
            f64::from(self.corrections) / f64::from(self.decisions)
        }
    }
}

/// The agent's rate before a change and after it
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RetroRates {
    pub before: RetroRate,
    pub after: RetroRate,
}

/// What the check found
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RetroVerdict {
    /// Fewer corrections than before
    Helped,
    /// No fewer
    Same,
    /// More
    Worse,
    /// Not enough decisions in four weeks to tell
    Unclear,
}

impl RetroVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Helped => "helped",
            Self::Same => "same",
            Self::Worse => "worse",
            Self::Unclear => "unclear",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Helped, Self::Same, Self::Worse, Self::Unclear]
            .into_iter()
            .find(|v| v.as_str() == text)
    }
}

/// `agent_id`'s rates around a change applied at `applied_at`, from
/// `decisions` (any agents', any time): the four weeks before, and since
pub fn rates(decisions: &[DecisionRecord], agent_id: &str, applied_at: i64) -> RetroRates {
    let mine = || decisions.iter().filter(|d| d.agent_id == agent_id);
    RetroRates {
        before: RetroRate::of(
            mine().filter(|d| d.at < applied_at && d.at >= applied_at - BEFORE_MS),
        ),
        after: RetroRate::of(mine().filter(|d| d.at >= applied_at)),
    }
}

/// The verdict on `rates` at `now`, or `None` while it's too early
pub fn verdict(rates: &RetroRates, applied_at: i64, now: i64) -> Option<RetroVerdict> {
    if rates.after.decisions < MIN_DECISIONS {
        return (now - applied_at >= AFTER_MS).then_some(RetroVerdict::Unclear);
    }
    let (before, after) = (rates.before.value(), rates.after.value());
    Some(if after < before {
        RetroVerdict::Helped
    } else if after > before {
        RetroVerdict::Worse
    } else {
        RetroVerdict::Same
    })
}

/// Whether the person hears about it: the change didn't lower the
/// corrections, and there still are some
pub fn needs_note(verdict: Option<RetroVerdict>, rates: Option<&RetroRates>) -> bool {
    matches!(verdict, Some(RetroVerdict::Same | RetroVerdict::Worse))
        && rates.is_some_and(|r| r.after.corrections > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ApprovalStatus;

    fn decision(agent: &str, at: i64, status: ApprovalStatus, edited: bool) -> DecisionRecord {
        DecisionRecord {
            run_id: format!("run-{at}"),
            agent_id: agent.to_string(),
            tool: "post".to_string(),
            args: "{}".to_string(),
            status,
            at,
            edited: edited.then(|| "What went out".to_string()),
        }
    }

    const APPLIED: i64 = 100 * DAY_MS;

    #[test]
    fn refusals_and_edits_are_corrections_approvals_are_not() {
        use ApprovalStatus::*;
        let decisions = [
            decision("writer", APPLIED - DAY_MS, Rejected, false),
            decision("writer", APPLIED - DAY_MS, Approved, true),
            decision("writer", APPLIED - DAY_MS, Approved, false),
            decision("writer", APPLIED - DAY_MS, Approved, false),
            // Too long before, and someone else's: not counted
            decision("writer", APPLIED - 40 * DAY_MS, Rejected, false),
            decision("other", APPLIED - DAY_MS, Rejected, false),
            decision("writer", APPLIED + DAY_MS, Approved, false),
            decision("writer", APPLIED + DAY_MS, Rejected, false),
        ];
        let rates = rates(&decisions, "writer", APPLIED);
        assert_eq!(
            rates.before,
            RetroRate {
                decisions: 4,
                corrections: 2
            }
        );
        assert_eq!(
            rates.after,
            RetroRate {
                decisions: 2,
                corrections: 1
            }
        );
        assert_eq!(rates.before.value(), 0.5);
    }

    #[test]
    fn the_verdict_waits_for_enough_decisions() {
        let rate = |decisions, corrections| RetroRate {
            decisions,
            corrections,
        };
        let early = RetroRates {
            before: rate(4, 2),
            after: rate(4, 0),
        };
        assert_eq!(verdict(&early, APPLIED, APPLIED + DAY_MS), None);
        assert_eq!(
            verdict(&early, APPLIED, APPLIED + AFTER_MS),
            Some(RetroVerdict::Unclear)
        );
        let helped = RetroRates {
            before: rate(4, 2),
            after: rate(6, 1),
        };
        assert_eq!(
            verdict(&helped, APPLIED, APPLIED + DAY_MS),
            Some(RetroVerdict::Helped)
        );
        let same = RetroRates {
            before: rate(4, 2),
            after: rate(6, 3),
        };
        assert_eq!(
            verdict(&same, APPLIED, APPLIED + DAY_MS),
            Some(RetroVerdict::Same)
        );
        let worse = RetroRates {
            before: rate(4, 1),
            after: rate(5, 3),
        };
        assert_eq!(
            verdict(&worse, APPLIED, APPLIED + DAY_MS),
            Some(RetroVerdict::Worse)
        );
        assert!(needs_note(Some(RetroVerdict::Same), Some(&same)));
        assert!(needs_note(Some(RetroVerdict::Worse), Some(&worse)));
        assert!(!needs_note(Some(RetroVerdict::Helped), Some(&helped)));
        let clean = RetroRates {
            before: rate(4, 0),
            after: rate(5, 0),
        };
        assert!(
            !needs_note(Some(RetroVerdict::Same), Some(&clean)),
            "nothing to fix"
        );
        assert!(!needs_note(Some(RetroVerdict::Unclear), Some(&early)));
    }
}
