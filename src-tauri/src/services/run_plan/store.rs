//! `run_plans`: each run's plan, by revision. A run continuing a compacted
//! one (`runs.parent_run_id`) carries its parent's plan on.

use rusqlite::{params, Connection, OptionalExtension};

use super::{RunPlan, RunPlanStep};
use crate::db;

/// How far up a compaction chain a plan is looked for
const MAX_LINEAGE: usize = 32;

fn latest_own(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunPlan>> {
    conn.query_row(
        "SELECT revision, steps, updated_at FROM run_plans WHERE run_id = ?1 ORDER BY revision DESC LIMIT 1",
        [run_id],
        |row| {
            let steps: String = row.get(1)?;
            Ok(RunPlan {
                run_id: run_id.to_string(),
                revision: row.get(0)?,
                steps: serde_json::from_str(&steps).unwrap_or_default(),
                updated_at: db::ts(row.get(2)?),
            })
        },
    )
    .optional()
}

/// The plan `run_id` works to: its own latest revision, else the one it
/// continues from a compacted parent
pub fn current(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunPlan>> {
    let mut id = run_id.to_string();
    for _ in 0..MAX_LINEAGE {
        if let Some(plan) = latest_own(conn, &id)? {
            return Ok(Some(plan));
        }
        let parent: Option<String> = conn
            .query_row(
                "SELECT parent_run_id FROM runs WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        match parent {
            Some(parent) => id = parent,
            None => break,
        }
    }
    Ok(None)
}

/// Sets `run_id`'s plan: a new revision after its latest own one
pub fn set(conn: &Connection, run_id: &str, steps: Vec<RunPlanStep>) -> rusqlite::Result<RunPlan> {
    let revision = latest_own(conn, run_id)?.map_or(1, |plan| plan.revision + 1);
    save(conn, run_id, revision, steps)
}

/// Saves `plan` as `run_id`'s latest revision: in place when it's the run's
/// own, as its first revision when it came from a compacted parent
pub fn update(conn: &Connection, run_id: &str, plan: &RunPlan) -> rusqlite::Result<RunPlan> {
    let revision = if plan.run_id == run_id {
        plan.revision
    } else {
        latest_own(conn, run_id)?.map_or(1, |own| own.revision + 1)
    };
    save(conn, run_id, revision, plan.steps.clone())
}

fn save(
    conn: &Connection,
    run_id: &str,
    revision: u32,
    steps: Vec<RunPlanStep>,
) -> rusqlite::Result<RunPlan> {
    let now = db::now_ms();
    let json = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO run_plans (run_id, revision, steps, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (run_id, revision) DO UPDATE SET steps = excluded.steps, updated_at = excluded.updated_at",
        params![run_id, revision, json, now],
    )?;
    Ok(RunPlan {
        run_id: run_id.to_string(),
        revision,
        steps,
        updated_at: db::ts(now),
    })
}

/// Every revision of `run_id`'s own plan, oldest first
#[cfg(test)]
pub fn revisions(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<u32>> {
    let mut stmt =
        conn.prepare("SELECT revision FROM run_plans WHERE run_id = ?1 ORDER BY revision")?;
    let rows = stmt.query_map([run_id], |row| row.get(0))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::run_plan::{apply_update, parse_steps};
    use serde_json::json;

    fn conn_with_runs() -> db::Db {
        let db = db::Db::in_memory();
        let conn = db.conn().unwrap();
        conn.execute_batch(
            "INSERT INTO conversations (id, agent_id, created_at) VALUES ('c1', 'a', 0);
             INSERT INTO runs (id, agent_id, conversation_id, started_at, status, input)
                 VALUES ('r1', 'a', 'c1', 0, 'compacted', 'pay the bill');
             INSERT INTO runs (id, agent_id, conversation_id, started_at, status, input, parent_run_id)
                 VALUES ('r2', 'a', 'c1', 1, 'running', 'pay the bill', 'r1');",
        )
        .unwrap();
        drop(conn);
        db
    }

    fn steps(titles: &[&str]) -> Vec<RunPlanStep> {
        let steps: Vec<_> = titles
            .iter()
            .map(|t| json!({ "title": t, "kind": "other" }))
            .collect();
        parse_steps(&json!({ "steps": steps })).unwrap()
    }

    #[test]
    fn a_later_plan_set_is_kept_as_a_revision() {
        let db = conn_with_runs();
        let conn = db.conn().unwrap();
        assert_eq!(current(&conn, "r1").unwrap(), None);
        let first = set(&conn, "r1", steps(&["a"])).unwrap();
        assert_eq!(first.revision, 1);
        let second = set(&conn, "r1", steps(&["a", "b"])).unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(revisions(&conn, "r1").unwrap(), [1, 2]);
        let plan = current(&conn, "r1").unwrap().unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.revision, 2);
    }

    #[test]
    fn updates_change_the_latest_revision_in_place() {
        let db = conn_with_runs();
        let conn = db.conn().unwrap();
        let mut plan = set(&conn, "r1", steps(&["a"])).unwrap();
        apply_update(&mut plan, &json!({ "step": 1, "status": "done" })).unwrap();
        update(&conn, "r1", &plan).unwrap();
        assert_eq!(revisions(&conn, "r1").unwrap(), [1]);
        assert_eq!(
            current(&conn, "r1").unwrap().unwrap().steps[0].status,
            plan.steps[0].status
        );
    }

    #[test]
    fn a_compacted_run_s_child_carries_its_plan_on() {
        let db = conn_with_runs();
        let conn = db.conn().unwrap();
        set(&conn, "r1", steps(&["a", "b"])).unwrap();
        let mut plan = current(&conn, "r2").unwrap().unwrap();
        assert_eq!(plan.run_id, "r1");
        apply_update(&mut plan, &json!({ "step": 1, "status": "done" })).unwrap();
        let saved = update(&conn, "r2", &plan).unwrap();
        assert_eq!((saved.run_id.as_str(), saved.revision), ("r2", 1));
        assert_eq!(current(&conn, "r2").unwrap().unwrap().run_id, "r2");
        // The parent's own plan is left as it was
        assert!(current(&conn, "r1").unwrap().unwrap().steps[0]
            .status
            .is_open());
    }

    #[test]
    fn plans_go_with_their_run() {
        let db = conn_with_runs();
        let conn = db.conn().unwrap();
        set(&conn, "r2", steps(&["a"])).unwrap();
        conn.execute("DELETE FROM runs WHERE id = 'r2'", [])
            .unwrap();
        assert!(revisions(&conn, "r2").unwrap().is_empty());
    }
}
