//! Runs across every agent: filtered, paged listing and aggregate stats for
//! the global runs view.
//!
//! The timeline is the `runs` table. Workflow steps and trigger fires are
//! agent runs too (`source_type` `workflow`, or a trigger source), so they
//! are already in it; `workflow_runs`, `trigger_fires`/`triggers`, and the
//! `agents` projection are joined in so a search also matches the workflow,
//! the trigger's name, and the agent's name.
//!
//! The types below are the frozen wave-1 contract between the backend and
//! the UI: fields may be added, never renamed or removed.

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, Db};
use crate::model::{RunStatus, RunSummary, SourceType, Timestamp, STUDIO_AGENT_ID};

/// Every field narrows the result; `None` doesn't filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunFilters {
    /// Inclusive, ms since the Unix epoch
    #[serde(default)]
    pub date_from: Option<Timestamp>,
    /// Exclusive, ms since the Unix epoch
    #[serde(default)]
    pub date_to: Option<Timestamp>,
    /// A `RunStatus` value, e.g. `failed` (`awaitingApproval` or
    /// `awaiting_approval`)
    #[serde(default)]
    pub status: Option<String>,
    /// A `SourceType` value, e.g. `webhook`, or `trigger` for any trigger
    /// source (`cron`, `file`, `webhook`, `device`, `event`)
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Matches the run's input and summary (and its source label, agent
    /// name, workflow, and trigger name)
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunPage {
    pub rows: Vec<RunSummary>,
    /// Matching runs across all pages
    pub total: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
    pub total_runs: u32,
    pub completed: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub total_tokens_in: f64,
    pub total_tokens_out: f64,
    pub total_cost_usd: f64,
    pub avg_duration_ms: f64,
}

/// `[from, to)`, ms since the Unix epoch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub from: Timestamp,
    pub to: Timestamp,
}

/// Rows per page when the caller doesn't say.
pub const DEFAULT_PAGE_SIZE: u32 = 100;

/// The most rows one page may hold.
pub const MAX_PAGE_SIZE: u32 = 200;

/// The `sourceType` filter that matches every trigger-started run.
const TRIGGER_SOURCE: &str = "trigger";

const ALL_STATUSES: [RunStatus; 6] = [
    RunStatus::Queued,
    RunStatus::Running,
    RunStatus::AwaitingApproval,
    RunStatus::Completed,
    RunStatus::Failed,
    RunStatus::Cancelled,
];

const ALL_SOURCES: [SourceType; 12] = [
    SourceType::User,
    SourceType::Workflow,
    SourceType::Agent,
    SourceType::Scheduled,
    SourceType::Cron,
    SourceType::Calendar,
    SourceType::File,
    SourceType::Webhook,
    SourceType::Device,
    SourceType::Event,
    SourceType::Email,
    SourceType::CalendarCheck,
];

const TRIGGER_SOURCES: [SourceType; 8] = [
    SourceType::Cron,
    SourceType::Calendar,
    SourceType::File,
    SourceType::Webhook,
    SourceType::Device,
    SourceType::Event,
    SourceType::Email,
    SourceType::CalendarCheck,
];

/// Runs across every agent matching `filters`, newest first. Studio runs
/// are left out unless `filters.agent_id` asks for them. `limit` is capped
/// at `MAX_PAGE_SIZE`.
pub async fn list_all_runs(
    db: &Db,
    filters: RunFilters,
    limit: u32,
    offset: u32,
) -> Result<RunPage, String> {
    let (clause, params) = where_clause(&filters)?;
    let limit = limit.min(MAX_PAGE_SIZE);
    db.call(move |conn| {
        let total: u32 = conn.query_row(
            &format!("SELECT COUNT(*) FROM runs WHERE {clause}"),
            params_from_iter(params.iter()),
            |row| row.get(0),
        )?;
        let limit_param = params.len() + 1;
        let offset_param = params.len() + 2;
        let mut page_params = params;
        page_params.push(Value::Integer(i64::from(limit)));
        page_params.push(Value::Integer(i64::from(offset)));
        let ids: Vec<String> = conn
            .prepare(&format!(
                "SELECT id FROM runs WHERE {clause}
                  ORDER BY started_at DESC, id DESC
                  LIMIT ?{limit_param} OFFSET ?{offset_param}"
            ))?
            .query_map(params_from_iter(page_params.iter()), |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(run) = db::get_run(conn, &id)? {
                rows.push(run);
            }
        }
        Ok(RunPage { rows, total })
    })
    .await
}

/// Totals over the runs started in `range` (Studio runs left out). The
/// average duration counts finished runs only.
pub async fn get_run_stats(db: &Db, range: DateRange) -> Result<RunStats, String> {
    if range.from > range.to {
        return Err(format!(
            "Invalid date range: from ({}) is after to ({})",
            range.from, range.to
        ));
    }
    let (from, to) = (range.from as i64, range.to as i64);
    db.call(move |conn| {
        conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(status = 'completed'), 0),
                    COALESCE(SUM(status = 'failed'), 0),
                    COALESCE(SUM(status = 'cancelled'), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(AVG(CASE WHEN ended_at IS NOT NULL
                                      THEN ended_at - started_at END), 0.0)
               FROM runs
              WHERE started_at >= ?1 AND started_at < ?2 AND agent_id != ?3",
            params![from, to, STUDIO_AGENT_ID],
            |row| {
                Ok(RunStats {
                    total_runs: row.get(0)?,
                    completed: row.get(1)?,
                    failed: row.get(2)?,
                    cancelled: row.get(3)?,
                    total_tokens_in: row.get(4)?,
                    total_tokens_out: row.get(5)?,
                    total_cost_usd: row.get(6)?,
                    avg_duration_ms: row.get(7)?,
                })
            },
        )
    })
    .await
}

/// What a search matches, with `?N` standing for the `LIKE` pattern.
const SEARCH_CONDITION: &str = "(runs.input LIKE ?N ESCAPE '\\'
      OR COALESCE(runs.summary, '') LIKE ?N ESCAPE '\\'
      OR COALESCE(runs.source_label, '') LIKE ?N ESCAPE '\\'
      OR runs.agent_id LIKE ?N ESCAPE '\\'
      OR EXISTS (SELECT 1 FROM agents a
                  WHERE a.id = runs.agent_id AND a.name LIKE ?N ESCAPE '\\')
      OR EXISTS (SELECT 1 FROM workflow_runs w
                  WHERE w.id = runs.source_id AND w.workflow_id LIKE ?N ESCAPE '\\')
      OR EXISTS (SELECT 1 FROM trigger_fires f JOIN triggers t ON t.id = f.trigger_id
                  WHERE f.run_id = runs.id AND t.name LIKE ?N ESCAPE '\\'))";

/// The SQL condition for `filters`, with its parameters numbered `?1…`.
fn where_clause(filters: &RunFilters) -> Result<(String, Vec<Value>), String> {
    let mut conditions = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    match non_empty(&filters.agent_id) {
        Some(agent_id) => {
            let p = bind(&mut params, Value::Text(agent_id.into()));
            conditions.push(format!("runs.agent_id = {p}"));
        }
        None => {
            let p = bind(&mut params, Value::Text(STUDIO_AGENT_ID.into()));
            conditions.push(format!("runs.agent_id != {p}"));
        }
    }
    if let Some(from) = filters.date_from {
        let p = bind(&mut params, Value::Integer(from as i64));
        conditions.push(format!("runs.started_at >= {p}"));
    }
    if let Some(to) = filters.date_to {
        let p = bind(&mut params, Value::Integer(to as i64));
        conditions.push(format!("runs.started_at < {p}"));
    }
    if let Some(status) = non_empty(&filters.status) {
        let status = parse_status(status)?;
        let p = bind(&mut params, Value::Text(status.as_str().into()));
        conditions.push(format!("runs.status = {p}"));
    }
    if let Some(source) = non_empty(&filters.source_type) {
        let placeholders: Vec<String> = parse_sources(source)?
            .into_iter()
            .map(|source| bind(&mut params, Value::Text(source.as_str().into())))
            .collect();
        conditions.push(format!("runs.source_type IN ({})", placeholders.join(", ")));
    }
    if let Some(search) = non_empty(&filters.search) {
        let p = bind(
            &mut params,
            Value::Text(format!("%{}%", escape_like(search))),
        );
        conditions.push(SEARCH_CONDITION.replace("?N", &p));
    }
    Ok((conditions.join(" AND "), params))
}

/// Adds `value` as the next parameter and returns its placeholder (`?N`).
fn bind(params: &mut Vec<Value>, value: Value) -> String {
    params.push(value);
    format!("?{}", params.len())
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// A status as the UI sends it (`awaitingApproval`) or as stored
/// (`awaiting_approval`).
pub fn parse_status(value: &str) -> Result<RunStatus, String> {
    ALL_STATUSES
        .into_iter()
        .find(|status| status.as_str() == value || serde_name(status).as_deref() == Some(value))
        .ok_or_else(|| format!("Unknown run status '{value}'"))
}

/// One source type, or every trigger source for `trigger`.
pub fn parse_sources(value: &str) -> Result<Vec<SourceType>, String> {
    if value == TRIGGER_SOURCE {
        return Ok(TRIGGER_SOURCES.to_vec());
    }
    ALL_SOURCES
        .into_iter()
        .find(|source| source.as_str() == value)
        .map(|source| vec![source])
        .ok_or_else(|| format!("Unknown run source '{value}'"))
}

fn serde_name(value: &impl Serialize) -> Option<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Some(name),
        _ => None,
    }
}

/// `%`, `_`, and `\` match literally (with `ESCAPE '\'`).
fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86_400_000.0;

    fn run(id: &str, agent_id: &str, started_at: f64) -> RunSummary {
        RunSummary {
            id: id.into(),
            agent_id: agent_id.into(),
            conversation_id: String::new(),
            status: RunStatus::Completed,
            input: format!("input of {id}"),
            summary: None,
            started_at,
            ended_at: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
            model: None,
            source_type: SourceType::User,
            source_id: None,
            source_label: None,
            priority: 100,
            queued_at: None,
            parent_run_id: None,
            compaction_summary: None,
        }
    }

    /// Inserts `run`, then sets the columns `insert_run` leaves out.
    fn insert(conn: &rusqlite::Connection, mut run: RunSummary) {
        run.conversation_id = db::ensure_agent_conversation(conn, &run.agent_id).unwrap();
        db::insert_run(conn, &run).unwrap();
        conn.execute(
            "UPDATE runs SET ended_at = ?2, summary = ?3, input_tokens = ?4,
                             output_tokens = ?5, cost_usd = ?6 WHERE id = ?1",
            params![
                run.id,
                run.ended_at.map(|t| t as i64),
                run.summary,
                run.input_tokens,
                run.output_tokens,
                run.cost_usd
            ],
        )
        .unwrap();
    }

    /// a1 (writer, day 1, completed, user), a2 (writer, day 2, failed,
    /// cron trigger "Nightly digest"), b1 (reader, day 3, cancelled,
    /// webhook, summary "50% done"), w1 (reader, day 4, workflow
    /// "daily-report"), s1 (Studio, day 2)
    async fn seeded() -> Db {
        let db = Db::in_memory();
        db.call(|conn| {
            conn.execute_batch(
                "INSERT INTO agents (id, name, created_at, updated_at, source_path, projection_version)
                 VALUES ('reader', 'Research Reader', 0, 0, 'reader/skill.md', 1);
                 INSERT INTO triggers (id, name, source_type, source_config, target_type,
                                       target_id, created_at, updated_at)
                 VALUES ('nightly', 'Nightly digest', 'cron', '{}', 'agent', 'writer', 0, 0);
                 INSERT INTO trigger_fires (id, trigger_id, fired_at, payload, run_id, status)
                 VALUES ('f1', 'nightly', 0, '{}', 'a2', 'queued');
                 INSERT INTO workflow_runs (id, workflow_id, status, started_at)
                 VALUES ('wr1', 'daily-report', 'completed', 0);",
            )?;

            let mut a1 = run("a1", "writer", DAY);
            a1.ended_at = Some(DAY + 1_000.0);
            a1.input_tokens = 100;
            a1.output_tokens = 10;
            a1.cost_usd = Some(0.5);
            insert(conn, a1);

            let mut a2 = run("a2", "writer", 2.0 * DAY);
            a2.status = RunStatus::Failed;
            a2.source_type = SourceType::Cron;
            a2.source_id = Some("nightly".into());
            a2.ended_at = Some(2.0 * DAY + 3_000.0);
            a2.input_tokens = 50;
            a2.output_tokens = 5;
            insert(conn, a2);

            let mut b1 = run("b1", "reader", 3.0 * DAY);
            b1.status = RunStatus::Cancelled;
            b1.source_type = SourceType::Webhook;
            b1.summary = Some("50% done".into());
            insert(conn, b1);

            let mut w1 = run("w1", "reader", 4.0 * DAY);
            w1.status = RunStatus::AwaitingApproval;
            w1.source_type = SourceType::Workflow;
            w1.source_id = Some("wr1".into());
            w1.source_label = Some("Daily Report · step 1".into());
            insert(conn, w1);

            insert(conn, run("s1", STUDIO_AGENT_ID, 2.0 * DAY));
            Ok(())
        })
        .await
        .unwrap();
        db
    }

    async fn ids(db: &Db, filters: RunFilters) -> Vec<String> {
        let page = list_all_runs(db, filters, DEFAULT_PAGE_SIZE, 0)
            .await
            .unwrap();
        page.rows.into_iter().map(|r| r.id).collect()
    }

    fn filters(f: impl FnOnce(&mut RunFilters)) -> RunFilters {
        let mut filters = RunFilters::default();
        f(&mut filters);
        filters
    }

    #[tokio::test]
    async fn lists_every_agent_newest_first_without_studio() {
        let db = seeded().await;
        let page = list_all_runs(&db, RunFilters::default(), DEFAULT_PAGE_SIZE, 0)
            .await
            .unwrap();
        assert_eq!(page.total, 4);
        let ids: Vec<_> = page.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["w1", "b1", "a2", "a1"]);
        // Full rows, not just IDs
        assert_eq!(page.rows[3].input_tokens, 100);
        assert_eq!(page.rows[3].cost_usd, Some(0.5));
    }

    #[tokio::test]
    async fn pages_with_the_total_across_pages() {
        let db = seeded().await;
        let page = list_all_runs(&db, RunFilters::default(), 2, 1)
            .await
            .unwrap();
        assert_eq!(page.total, 4);
        let ids: Vec<_> = page.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["b1", "a2"]);

        let past_end = list_all_runs(&db, RunFilters::default(), 2, 10)
            .await
            .unwrap();
        assert!(past_end.rows.is_empty());
        assert_eq!(past_end.total, 4);
    }

    #[tokio::test]
    async fn filters_narrow_the_result() {
        let db = seeded().await;

        let by_agent = filters(|f| f.agent_id = Some("writer".into()));
        assert_eq!(ids(&db, by_agent).await, ["a2", "a1"]);
        let studio = filters(|f| f.agent_id = Some(STUDIO_AGENT_ID.into()));
        assert_eq!(ids(&db, studio).await, ["s1"]);

        let failed = filters(|f| f.status = Some("failed".into()));
        assert_eq!(ids(&db, failed).await, ["a2"]);
        let awaiting = filters(|f| f.status = Some("awaitingApproval".into()));
        assert_eq!(ids(&db, awaiting).await, ["w1"]);

        let webhook = filters(|f| f.source_type = Some("webhook".into()));
        assert_eq!(ids(&db, webhook).await, ["b1"]);
        let trigger = filters(|f| f.source_type = Some("trigger".into()));
        assert_eq!(ids(&db, trigger).await, ["b1", "a2"]);
        let workflow = filters(|f| f.source_type = Some("workflow".into()));
        assert_eq!(ids(&db, workflow).await, ["w1"]);

        // `[from, to)`
        let day_two = filters(|f| {
            f.date_from = Some(2.0 * DAY);
            f.date_to = Some(3.0 * DAY);
        });
        assert_eq!(ids(&db, day_two).await, ["a2"]);

        // Combined
        let writer_triggers = filters(|f| {
            f.agent_id = Some("writer".into());
            f.source_type = Some("trigger".into());
        });
        assert_eq!(ids(&db, writer_triggers).await, ["a2"]);
    }

    #[tokio::test]
    async fn search_matches_input_summary_and_joined_names() {
        let db = seeded().await;
        let search = |text: &str| {
            let text = text.to_string();
            filters(move |f| f.search = Some(text))
        };

        // Input, case-insensitive
        assert_eq!(ids(&db, search("INPUT OF A")).await, ["a2", "a1"]);
        // Summary; `%` and `_` are literal
        assert_eq!(ids(&db, search("50%")).await, ["b1"]);
        assert!(ids(&db, search("5_%")).await.is_empty());
        // Agent name (from the `agents` projection) and agent ID
        assert_eq!(ids(&db, search("research")).await, ["w1", "b1"]);
        assert_eq!(ids(&db, search("writer")).await, ["a2", "a1"]);
        // Trigger name (via `trigger_fires`)
        assert_eq!(ids(&db, search("nightly")).await, ["a2"]);
        // Workflow (via `workflow_runs`) and the source label
        assert_eq!(ids(&db, search("daily-report")).await, ["w1"]);
        assert_eq!(ids(&db, search("step 1")).await, ["w1"]);
        // Blank values don't filter
        assert_eq!(ids(&db, search("  ")).await.len(), 4);
    }

    #[test]
    fn statuses_parse_in_either_spelling() {
        assert_eq!(
            parse_status("awaitingApproval").unwrap(),
            RunStatus::AwaitingApproval
        );
        assert_eq!(
            parse_status("awaiting_approval").unwrap(),
            RunStatus::AwaitingApproval
        );
        assert_eq!(parse_status("failed").unwrap(), RunStatus::Failed);
        assert!(parse_status("bogus").unwrap_err().contains("bogus"));
    }

    #[test]
    fn sources_parse_with_trigger_as_a_group() {
        assert_eq!(parse_sources("user").unwrap(), [SourceType::User]);
        assert_eq!(parse_sources("trigger").unwrap(), TRIGGER_SOURCES);
        assert!(parse_sources("nope").is_err());
    }

    #[test]
    fn like_escapes_wildcards() {
        assert_eq!(escape_like(r"50%_a\b"), r"50\%\_a\\b");
    }

    #[tokio::test]
    async fn unknown_filter_values_are_errors() {
        let db = seeded().await;
        let bad_status = filters(|f| f.status = Some("bogus".into()));
        assert!(list_all_runs(&db, bad_status, 10, 0).await.is_err());
        let bad_source = filters(|f| f.source_type = Some("nope".into()));
        assert!(list_all_runs(&db, bad_source, 10, 0).await.is_err());
    }

    #[tokio::test]
    async fn page_size_is_capped() {
        let db = Db::in_memory();
        db.call(|conn| {
            for i in 0..(MAX_PAGE_SIZE + 5) {
                insert(conn, run(&format!("r{i}"), "writer", f64::from(i)));
            }
            Ok(())
        })
        .await
        .unwrap();
        let page = list_all_runs(&db, RunFilters::default(), u32::MAX, 0)
            .await
            .unwrap();
        assert_eq!(page.rows.len(), MAX_PAGE_SIZE as usize);
        assert_eq!(page.total, MAX_PAGE_SIZE + 5);
    }

    #[tokio::test]
    async fn stats_sum_the_range() {
        let db = seeded().await;
        let all = get_run_stats(
            &db,
            DateRange {
                from: 0.0,
                to: 10.0 * DAY,
            },
        )
        .await
        .unwrap();
        assert_eq!(all.total_runs, 4);
        assert_eq!((all.completed, all.failed, all.cancelled), (1, 1, 1));
        assert_eq!(all.total_tokens_in, 150.0);
        assert_eq!(all.total_tokens_out, 15.0);
        assert_eq!(all.total_cost_usd, 0.5);
        // a1 took 1s, a2 3s; the others haven't ended
        assert_eq!(all.avg_duration_ms, 2_000.0);

        let day_one = get_run_stats(
            &db,
            DateRange {
                from: DAY,
                to: 2.0 * DAY,
            },
        )
        .await
        .unwrap();
        assert_eq!(day_one.total_runs, 1);
        assert_eq!(day_one.completed, 1);

        let empty = get_run_stats(&db, DateRange { from: 0.0, to: 1.0 })
            .await
            .unwrap();
        assert_eq!(empty.total_runs, 0);
        assert_eq!(empty.avg_duration_ms, 0.0);
        assert_eq!(empty.total_cost_usd, 0.0);

        assert!(get_run_stats(&db, DateRange { from: 2.0, to: 1.0 })
            .await
            .is_err());
    }
}
