//! At most one weekly retro a week: the trigger's fire is held when a
//! retro run started less than [`MIN_GAP_MS`] ago (a catch-up after a
//! missed Friday, a second "Run now").

use crate::db::Db;
use crate::services::triggers::FireGuard;

use super::{proposals, WEEKLY_RETRO};

/// The least time between two retros
pub const MIN_GAP_MS: i64 = 6 * 24 * 60 * 60 * 1000;

/// Why a retro starting at `now` should wait, after one at `last`. Pure.
pub fn hold_reason(last: Option<i64>, now: i64) -> Option<String> {
    let last = last?;
    (now - last < MIN_GAP_MS).then(|| "Skipped: the weekly retro already ran this week".to_string())
}

/// Holds the weekly retro trigger's fires within a week of the last one
pub struct RetroGuard(Db);

impl RetroGuard {
    pub fn new(db: Db) -> Self {
        Self(db)
    }
}

impl FireGuard for RetroGuard {
    fn hold(&self, trigger_id: &str) -> Option<String> {
        if trigger_id != WEEKLY_RETRO {
            return None;
        }
        let conn = self.0.conn().ok()?;
        let last = proposals::last_retro_run(&conn, WEEKLY_RETRO).ok()?;
        hold_reason(last, crate::db::now_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_retro_a_week() {
        let now = 100 * MIN_GAP_MS;
        assert_eq!(hold_reason(None, now), None);
        assert!(hold_reason(Some(now - MIN_GAP_MS + 1), now).is_some());
        assert_eq!(hold_reason(Some(now - MIN_GAP_MS), now), None);

        let db = Db::in_memory();
        let guard = RetroGuard::new(db.clone());
        assert_eq!(guard.hold(WEEKLY_RETRO), None, "never ran");
        let conn = db.conn().unwrap();
        crate::db::tests::start_run(
            &conn,
            crate::services::builtin_agents::DEFAULT_AGENT,
            "retro-1",
        );
        conn.execute(
            "UPDATE runs SET source_id = ?1 WHERE id = 'retro-1'",
            [WEEKLY_RETRO],
        )
        .unwrap();
        drop(conn);
        assert!(guard.hold(WEEKLY_RETRO).is_some(), "ran just now");
        assert_eq!(guard.hold("morning-digest"), None);
    }
}
