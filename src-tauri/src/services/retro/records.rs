//! What the weekly retro reads: the run and approval records of a week,
//! from `state.db` only. A run that failed or was tried again, a call the
//! person refused, a draft they changed before it went out (a hand-over
//! answered "Changed it"). [`report`] lays them out for the model within
//! a budget; [`DecisionRecord`]s also feed the check.

use std::collections::BTreeMap;
use std::str::FromStr;

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::model::{ApprovalStatus, RunStatus, STUDIO_AGENT_ID};
use crate::services::pilots::handover::CHANGED_RESULT;

/// The longest report, in characters (a few thousand tokens)
pub const REPORT_CHARS: usize = 12_000;
/// The longest quote from one record
const QUOTE_CHARS: usize = 400;
/// Records of each kind listed per agent; the rest are counted
const PER_KIND: usize = 6;

/// One run, as the retro sees it
#[derive(Debug, Clone, PartialEq)]
pub struct RunRecord {
    pub id: String,
    pub agent_id: String,
    pub started_at: i64,
    pub status: RunStatus,
    /// 2 and up: tried again
    pub attempt: i64,
    /// What it was asked
    pub input: String,
    /// Its last words, or its error
    pub summary: Option<String>,
    /// The trigger, workflow run or Request that started it
    pub source_id: Option<String>,
}

/// One decision the person made on an approval
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRecord {
    pub run_id: String,
    pub agent_id: String,
    pub tool: String,
    /// The call's arguments (JSON)
    pub args: String,
    /// Approved or rejected
    pub status: ApprovalStatus,
    /// When they decided
    pub at: i64,
    /// What went out, when they changed the draft first
    pub edited: Option<String>,
}

impl DecisionRecord {
    /// A refusal, or a draft the person changed before it went out
    pub fn is_correction(&self) -> bool {
        self.status == ApprovalStatus::Rejected || self.edited.is_some()
    }
}

/// Runs started in `[since, until)`, oldest first; Studio's left out
pub fn runs(conn: &Connection, since: i64, until: i64) -> rusqlite::Result<Vec<RunRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, started_at, status, attempt, input, summary, source_id FROM runs
          WHERE started_at >= ?1 AND started_at < ?2 AND agent_id != ?3
          ORDER BY started_at, id",
    )?;
    let rows = stmt.query_map(params![since, until, STUDIO_AGENT_ID], |row| {
        let status: String = row.get(3)?;
        Ok(RunRecord {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            started_at: row.get(2)?,
            status: RunStatus::from_str(&status).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    format!("runs: bad status '{status}'").into(),
                )
            })?,
            attempt: row.get(4)?,
            input: row.get(5)?,
            summary: row.get(6)?,
            source_id: row.get(7)?,
        })
    })?;
    rows.collect()
}

/// Approvals the person decided in `[since, until)`, oldest first, with
/// what went out when they changed a hand-over's draft
pub fn decisions(
    conn: &Connection,
    since: i64,
    until: i64,
) -> rusqlite::Result<Vec<DecisionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT a.run_id, a.agent_id, a.tool, a.args, a.status,
                COALESCE(a.resolved_at, a.created_at) AS at,
                json_extract(m.metadata, '$.result') AS result
           FROM approvals a LEFT JOIN messages m ON m.id = a.tool_call_id
          WHERE a.status IN ('approved', 'rejected')
            AND COALESCE(a.resolved_at, a.created_at) >= ?1
            AND COALESCE(a.resolved_at, a.created_at) < ?2
            AND a.agent_id != ?3
          ORDER BY at, a.id",
    )?;
    let rows = stmt.query_map(params![since, until, STUDIO_AGENT_ID], |row| {
        let status: String = row.get(4)?;
        let status = if status == "rejected" {
            ApprovalStatus::Rejected
        } else {
            ApprovalStatus::Approved
        };
        let result: Option<String> = row.get(6)?;
        Ok(DecisionRecord {
            run_id: row.get(0)?,
            agent_id: row.get(1)?,
            tool: row.get(2)?,
            args: row.get(3)?,
            status,
            at: row.get(5)?,
            edited: result.filter(|text| {
                status == ApprovalStatus::Approved && text.starts_with(CHANGED_RESULT)
            }),
        })
    })?;
    rows.collect()
}

fn quote(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > QUOTE_CHARS {
        let cut: String = text.chars().take(QUOTE_CHARS).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

/// What the retro is told about one agent
#[derive(Debug, Default)]
struct AgentWeek<'a> {
    runs: Vec<&'a RunRecord>,
    decisions: Vec<&'a DecisionRecord>,
}

/// The week laid out for the model, per agent, within [`REPORT_CHARS`]:
/// counts, then the corrections, failures and retries with their run IDs.
/// `is_pack` says which agents came from a pack (never edited). Quotes are
/// untrusted: they may hold text from mail or the web.
pub fn report(
    runs: &[RunRecord],
    decisions: &[DecisionRecord],
    is_pack: &dyn Fn(&str) -> bool,
    skip_run: &dyn Fn(&RunRecord) -> bool,
) -> Value {
    let mut agents: BTreeMap<&str, AgentWeek> = BTreeMap::new();
    for run in runs.iter().filter(|run| !skip_run(run)) {
        agents.entry(&run.agent_id).or_default().runs.push(run);
    }
    for decision in decisions {
        agents
            .entry(&decision.agent_id)
            .or_default()
            .decisions
            .push(decision);
    }
    let mut used = 0usize;
    let mut out = Vec::new();
    for (agent, week) in agents {
        let failed: Vec<&&RunRecord> = week
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::Failed)
            .collect();
        let retried: Vec<&&RunRecord> = week.runs.iter().filter(|r| r.attempt > 1).collect();
        let refused: Vec<&&DecisionRecord> = week
            .decisions
            .iter()
            .filter(|d| d.status == ApprovalStatus::Rejected)
            .collect();
        let edited: Vec<&&DecisionRecord> = week
            .decisions
            .iter()
            .filter(|d| d.edited.is_some())
            .collect();
        let mut entry = json!({
            "agent": agent,
            "fromPack": is_pack(agent),
            "runs": week.runs.len(),
            "decisions": week.decisions.len(),
            "refused": refused.len(),
            "edited": edited.len(),
            "failed": failed.len(),
            "retried": retried.len(),
        });
        let budget_left = REPORT_CHARS.saturating_sub(used);
        let mut lists = serde_json::Map::new();
        lists.insert(
            "refusedCalls".into(),
            json!(refused
                .iter()
                .take(PER_KIND)
                .map(|d| json!({ "run": d.run_id, "tool": d.tool, "args": quote(&d.args) }))
                .collect::<Vec<_>>()),
        );
        lists.insert(
            "editedDrafts".into(),
            json!(edited
                .iter()
                .take(PER_KIND)
                .map(|d| json!({
                    "run": d.run_id,
                    "prepared": quote(&d.args),
                    "whatWentOut": quote(d.edited.as_deref().unwrap_or_default()),
                }))
                .collect::<Vec<_>>()),
        );
        lists.insert(
            "failedRuns".into(),
            json!(failed
                .iter()
                .take(PER_KIND)
                .map(|r| json!({
                    "run": r.id,
                    "asked": quote(&r.input),
                    "error": quote(r.summary.as_deref().unwrap_or_default()),
                }))
                .collect::<Vec<_>>()),
        );
        lists.insert(
            "retriedRuns".into(),
            json!(retried
                .iter()
                .take(PER_KIND)
                .map(|r| json!({ "run": r.id, "attempt": r.attempt, "asked": quote(&r.input) }))
                .collect::<Vec<_>>()),
        );
        let with_lists = {
            let mut full = entry.clone();
            if let Value::Object(map) = &mut full {
                map.extend(lists);
            }
            full
        };
        let size = with_lists.to_string().len();
        if size <= budget_left {
            entry = with_lists;
            used += size;
        } else {
            // Over budget: the counts only
            used += entry.to_string().len();
            if let Value::Object(map) = &mut entry {
                map.insert("cut".into(), json!("details left out: over the budget"));
            }
        }
        out.push(entry);
    }
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, Db};

    #[test]
    fn reads_corrections_failures_and_retries_from_the_records() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conv = db::tests::start_run(&conn, "writer", "r1");
        db::tests::start_run(&conn, "writer", "r2");
        conn.execute(
            "UPDATE runs SET status = 'failed', summary = 'Site timed out', attempt = 2,
                             started_at = 1000 WHERE id = 'r2'",
            [],
        )
        .unwrap();
        conn.execute("UPDATE runs SET started_at = 1000 WHERE id = 'r1'", [])
            .unwrap();
        let approval = |id: &str, call: &str, tool: &str, status: &str, result: Option<&str>| {
            conn.execute(
                "INSERT INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool,
                                        args, status, created_at, resolved_at)
                 VALUES (?1, 'writer', 'r1', ?1, ?2, ?3, '{\"text\":\"Hi all\"}', ?4, 1000, 1100)",
                params![id, call, tool, status],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (id, conversation_id, run_id, role, content, created_at, metadata)
                 VALUES (?1, ?2, 'r1', 'tool_call', '', 1000, ?3)",
                params![
                    call,
                    conv,
                    json!({ "tool": tool, "args": {}, "status": "done", "result": result }).to_string()
                ],
            )
            .unwrap();
        };
        approval("a1", "c1", "post", "rejected", None);
        approval(
            "a2",
            "c2",
            "pilot_browser__browser_hand_over",
            "approved",
            Some("The person changed it and published it on x.com themselves. What went out:\n\nHello all"),
        );
        approval("a3", "c3", "post", "approved", Some("Posted"));
        approval("a4", "c4", "post", "expired", None);

        let runs = runs(&conn, 0, 2000).unwrap();
        assert_eq!(runs.len(), 2);
        let decisions = decisions(&conn, 0, 2000).unwrap();
        assert_eq!(decisions.len(), 3, "an expired approval is no decision");
        assert!(decisions[0].is_correction() && decisions[1].is_correction());
        assert!(!decisions[2].is_correction());

        let report = report(&runs, &decisions, &|_| false, &|_| false);
        let writer = &report[0];
        assert_eq!(writer["agent"], "writer");
        assert_eq!(
            (
                &writer["refused"],
                &writer["edited"],
                &writer["failed"],
                &writer["retried"]
            ),
            (&json!(1), &json!(1), &json!(1), &json!(1))
        );
        assert_eq!(writer["refusedCalls"][0]["run"], "r1");
        assert!(writer["editedDrafts"][0]["whatWentOut"]
            .as_str()
            .unwrap()
            .contains("Hello all"));
        assert_eq!(writer["failedRuns"][0]["error"], "Site timed out");
        // A run the retro skips (its own) isn't listed
        let without = super::report(&runs, &[], &|_| true, &|r: &RunRecord| r.id == "r2");
        assert_eq!(without[0]["runs"], 1);
        assert_eq!(without[0]["fromPack"], true);
    }

    #[test]
    fn a_long_week_keeps_to_the_budget() {
        let long = "x".repeat(5000);
        let runs: Vec<RunRecord> = (0..200)
            .map(|i| RunRecord {
                id: format!("r{i}"),
                agent_id: format!("agent-{}", i % 40),
                started_at: i,
                status: RunStatus::Failed,
                attempt: 2,
                input: long.clone(),
                summary: Some(long.clone()),
                source_id: None,
            })
            .collect();
        let report = report(&runs, &[], &|_| false, &|_| false);
        assert_eq!(report.as_array().unwrap().len(), 40);
        assert!(report.to_string().len() <= REPORT_CHARS + 40 * 200);
        assert!(report.to_string().contains("over the budget"));
    }
}
