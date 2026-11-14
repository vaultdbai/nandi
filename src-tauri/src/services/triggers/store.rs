//! SQLite access for `triggers` (read-only here: the projection writes it),
//! `trigger_state`, and `trigger_fires`.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::schema::{
    Trigger, TriggerFire, TriggerFireStatus, TriggerFireSummary, TriggerStats, TriggerTargetType,
};
use crate::db::{self, role};
use crate::model::RunStatus;
use crate::services::run_waiter;
use crate::services::workflows::WorkflowRunStatus;

const TRIGGER_COLUMNS: &str = "t.id, t.name, t.enabled, t.source_type, t.source_config,
     t.target_type, t.target_id, t.input_template, t.priority, t.rate_limit_seconds,
     t.created_at, t.updated_at, s.last_fired_at";
const TRIGGER_FROM: &str = "triggers t LEFT JOIN trigger_state s ON s.trigger_id = t.id";

fn trigger_row(row: &Row) -> rusqlite::Result<Trigger> {
    let mut trigger = Trigger {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        source_type: db::text_enum(row, 3)?,
        source_config: db::json_col(row, 4)?,
        target_type: db::text_enum(row, 5)?,
        target_id: row.get(6)?,
        input_template: row.get(7)?,
        priority: row.get(8)?,
        rate_limit_seconds: row.get(9)?,
        created_at: db::ts(row.get(10)?),
        updated_at: db::ts(row.get(11)?),
        last_fired_at: row.get::<_, Option<i64>>(12)?.map(db::ts),
        has_secret: false,
    };
    // Webhook and device triggers always get a secret (in the keyring)
    trigger.has_secret = trigger.source_type.has_secret();
    Ok(trigger)
}

pub fn list_triggers(conn: &Connection) -> rusqlite::Result<Vec<Trigger>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TRIGGER_COLUMNS} FROM {TRIGGER_FROM} ORDER BY t.name COLLATE NOCASE, t.id"
    ))?;
    let rows = stmt.query_map([], trigger_row)?;
    rows.collect()
}

pub fn get_trigger(conn: &Connection, id: &str) -> rusqlite::Result<Option<Trigger>> {
    conn.query_row(
        &format!("SELECT {TRIGGER_COLUMNS} FROM {TRIGGER_FROM} WHERE t.id = ?1"),
        [id],
        trigger_row,
    )
    .optional()
}

pub fn set_last_fired(conn: &Connection, id: &str, at: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO trigger_state (trigger_id, last_fired_at) VALUES (?1, ?2)
         ON CONFLICT(trigger_id) DO UPDATE SET last_fired_at = excluded.last_fired_at",
        params![id, at],
    )?;
    Ok(())
}

/// Forgets a deleted trigger's runtime state and fire history.
pub fn delete_history(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM trigger_fires WHERE trigger_id = ?1", [id])?;
    conn.execute("DELETE FROM trigger_state WHERE trigger_id = ?1", [id])?;
    Ok(())
}

const FIRE_COLUMNS: &str = "id, trigger_id, fired_at, payload, run_id, status, error";

/// A fire (`f`) with its trigger (`t`) and the status of the run it
/// dispatched, from `runs` or `workflow_runs` by the trigger's target.
/// `fire_row` reads columns 0–7; 8 is the trigger's name, 9 its source.
const FIRE_SELECT: &str = "SELECT f.id, f.trigger_id, f.fired_at, f.payload, f.run_id, f.status,
        f.error,
        CASE t.target_type
          WHEN 'workflow' THEN (SELECT status FROM workflow_runs WHERE id = f.run_id)
          ELSE (SELECT status FROM runs WHERE id = f.run_id)
        END,
        t.name, t.source_type
   FROM trigger_fires f JOIN triggers t ON t.id = f.trigger_id";

fn fire_row(row: &Row) -> rusqlite::Result<TriggerFire> {
    Ok(TriggerFire {
        id: row.get(0)?,
        trigger_id: row.get(1)?,
        fired_at: db::ts(row.get(2)?),
        payload: db::json_col(row, 3)?,
        run_id: row.get(4)?,
        status: db::text_enum(row, 5)?,
        error: row.get(6)?,
        run_status: db::opt_text_enum(row, 7)?,
    })
}

pub fn insert_fire(conn: &Connection, fire: &TriggerFire) -> rusqlite::Result<()> {
    conn.execute(
        &format!("INSERT INTO trigger_fires ({FIRE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            fire.id,
            fire.trigger_id,
            fire.fired_at as i64,
            fire.payload.to_string(),
            fire.run_id,
            fire.status.as_str(),
            fire.error,
        ],
    )?;
    Ok(())
}

/// Records what a fire led to. Returns the updated fire.
pub fn finish_fire(
    conn: &Connection,
    id: &str,
    status: TriggerFireStatus,
    run_id: Option<&str>,
    error: Option<&str>,
) -> rusqlite::Result<Option<TriggerFire>> {
    conn.execute(
        "UPDATE trigger_fires SET status = ?2, run_id = ?3, error = ?4 WHERE id = ?1",
        params![id, status.as_str(), run_id, error],
    )?;
    conn.query_row(&format!("{FIRE_SELECT} WHERE f.id = ?1"), [id], fire_row)
        .optional()
}

/// The latest fire of a calendar duty in `role` (`primary` or `backup`).
pub fn duty_fire(
    conn: &Connection,
    trigger_id: &str,
    duty_id: &str,
    role: &str,
) -> rusqlite::Result<Option<TriggerFire>> {
    conn.query_row(
        &format!(
            "{FIRE_SELECT}
              WHERE f.trigger_id = ?1 AND json_extract(f.payload, '$.duty_id') = ?2
                AND json_extract(f.payload, '$.role') = ?3
              ORDER BY f.fired_at DESC, f.rowid DESC LIMIT 1"
        ),
        params![trigger_id, duty_id, role],
        fire_row,
    )
    .optional()
}

/// The status of the agent run or workflow run `run_id`, as stored.
pub fn run_status(
    conn: &Connection,
    target: TriggerTargetType,
    run_id: &str,
) -> rusqlite::Result<Option<String>> {
    let table = match target {
        TriggerTargetType::Agent => "runs",
        TriggerTargetType::Workflow => "workflow_runs",
    };
    conn.query_row(
        &format!("SELECT status FROM {table} WHERE id = ?1"),
        [run_id],
        |row| row.get(0),
    )
    .optional()
}

/// Whether the run the trigger's latest dispatched fire started is still
/// queued or running. An agent run is followed into the child a
/// compaction left it going on in.
pub fn last_run_in_progress(
    conn: &Connection,
    trigger_id: &str,
    target: TriggerTargetType,
) -> rusqlite::Result<bool> {
    let run_id: Option<String> = conn
        .query_row(
            "SELECT run_id FROM trigger_fires WHERE trigger_id = ?1 AND run_id IS NOT NULL
              ORDER BY fired_at DESC, rowid DESC LIMIT 1",
            [trigger_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(false);
    };
    Ok(match target {
        TriggerTargetType::Agent => run_waiter::newest_run(conn, &run_id)?
            .is_some_and(|run| matches!(run.status, RunStatus::Queued | RunStatus::Running)),
        TriggerTargetType::Workflow => {
            run_status(conn, target, &run_id)?.as_deref()
                == Some(WorkflowRunStatus::Running.as_str())
        }
    })
}

/// A trigger's fires, newest first.
pub fn list_fires(
    conn: &Connection,
    trigger_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<TriggerFire>> {
    let mut stmt = conn.prepare(&format!(
        "{FIRE_SELECT} WHERE f.trigger_id = ?1
          ORDER BY f.fired_at DESC, f.rowid DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![trigger_id, limit], fire_row)?;
    rows.collect()
}

/// Every trigger's latest fires, newest first, for the dashboard.
pub fn recent_fires(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<TriggerFireSummary>> {
    let mut stmt = conn.prepare(&format!(
        "{FIRE_SELECT} ORDER BY f.fired_at DESC, f.rowid DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], |row| {
        Ok(TriggerFireSummary {
            fire: fire_row(row)?,
            trigger_name: row.get(8)?,
            source_type: db::text_enum(row, 9)?,
        })
    })?;
    rows.collect()
}

/// Fires since `since` (epoch ms) that started an agent run, newest first,
/// with their trigger's name. Fires that ran a workflow are left out: the
/// workflow run is their work item.
pub fn agent_fires_since(
    conn: &Connection,
    since: i64,
    limit: u32,
) -> rusqlite::Result<Vec<(TriggerFire, String)>> {
    let mut stmt = conn.prepare(&format!(
        "{FIRE_SELECT}
          WHERE t.target_type = ?1 AND f.run_id IS NOT NULL AND f.fired_at >= ?2
          ORDER BY f.fired_at DESC, f.rowid DESC LIMIT ?3"
    ))?;
    let rows = stmt.query_map(
        params![TriggerTargetType::Agent.as_str(), since, limit],
        |row| Ok((fire_row(row)?, row.get::<_, String>(8)?)),
    )?;
    rows.collect()
}

pub fn stats(conn: &Connection, day_start: i64) -> rusqlite::Result<TriggerStats> {
    Ok(TriggerStats {
        enabled: conn.query_row("SELECT COUNT(*) FROM triggers WHERE enabled = 1", [], |r| {
            r.get(0)
        })?,
        fired_today: conn.query_row(
            "SELECT COUNT(*) FROM trigger_fires WHERE fired_at >= ?1 AND status = 'queued'",
            [day_start],
            |r| r.get(0),
        )?,
    })
}

/// The final reply of the trigger's latest completed agent run, for
/// `{{last_run.output}}`.
pub fn last_output(conn: &Connection, trigger_id: &str) -> rusqlite::Result<Option<String>> {
    let run_id: Option<String> = conn
        .query_row(
            "SELECT f.run_id FROM trigger_fires f JOIN runs r ON r.id = f.run_id
              WHERE f.trigger_id = ?1 AND r.status = 'completed'
              ORDER BY f.fired_at DESC LIMIT 1",
            [trigger_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    Ok(db::list_run_messages(conn, &run_id)?
        .into_iter()
        .rev()
        .find(|m| m.role == role::ASSISTANT && !m.content.trim().is_empty())
        .map(|m| m.content))
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Type;

    use super::*;
    use crate::services::triggers::schema::{FireRunStatus, TriggerSourceType};

    fn conn_with_trigger(source_type: &str, source_config: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO triggers (id, name, source_type, source_config, target_type, target_id,
                                   created_at, updated_at)
             VALUES ('t1', 'Nightly', ?1, ?2, 'agent', 'a1', 0, 0)",
            params![source_type, source_config],
        )
        .unwrap();
        conn
    }

    fn read_error(conn: &Connection) -> Option<usize> {
        match get_trigger(conn, "t1") {
            Err(rusqlite::Error::FromSqlConversionFailure(index, Type::Text, _)) => Some(index),
            _ => None,
        }
    }

    #[test]
    fn a_valid_trigger_row_reads() {
        let conn = conn_with_trigger("cron", r#"{"schedule":"0 9 * * *"}"#);
        let trigger = get_trigger(&conn, "t1").unwrap().unwrap();
        assert_eq!(trigger.source_type, TriggerSourceType::Cron);
        assert_eq!(trigger.target_type, TriggerTargetType::Agent);
    }

    #[test]
    fn an_unknown_source_type_fails_the_read_instead_of_becoming_a_webhook() {
        let conn = conn_with_trigger("satellite", "{}");
        assert_eq!(read_error(&conn), Some(3));
    }

    #[test]
    fn a_source_config_that_is_not_json_fails_the_read_instead_of_becoming_null() {
        let conn = conn_with_trigger("cron", "{not json");
        assert_eq!(read_error(&conn), Some(4));
    }

    #[test]
    fn a_compacted_run_reads_as_running_and_an_unknown_status_is_an_error() {
        assert_eq!("compacted".parse(), Ok(FireRunStatus::Running));
        assert_eq!(
            "awaiting_approval".parse(),
            Ok(FireRunStatus::AwaitingApproval)
        );
        assert!("finished".parse::<FireRunStatus>().is_err());
    }
}
