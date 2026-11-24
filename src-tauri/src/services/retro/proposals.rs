//! The weekly retro's proposals in `state.db` (`retro_proposals`,
//! migration 009): what Nandi proposed, what the person decided, the gold
//! copy commits, and the check's verdict. Run history, like approvals:
//! the change itself lives in the gold copy.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::check::{RetroRates, RetroVerdict};

/// Where a proposal stands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProposalStatus {
    /// Waits in the Inbox for a yes or a no
    Pending,
    /// The person said yes: committed to the gold copy
    Applied,
    /// The person said no
    Refused,
    /// Applied, then undone
    Undone,
    /// Applied, and the person kept it after the check said it didn't help
    Kept,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applied => "applied",
            Self::Refused => "refused",
            Self::Undone => "undone",
            Self::Kept => "kept",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Applied,
            Self::Refused,
            Self::Undone,
            Self::Kept,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }
}

/// One proposed change to one agent's definitions
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub id: String,
    /// The retro run that proposed it
    pub run_id: String,
    /// The agent it changes (one agent per proposal)
    pub agent_id: String,
    /// `agent`, `playbook:<name>`, `workflow:<id>` or `trigger:<id>`
    pub target: String,
    /// Its file, relative to the gold copy's root
    pub path: String,
    /// Why, in one line for the person
    pub reason: String,
    /// The runs it learned from
    pub runs: Vec<String>,
    /// The file when proposed; `None` for a new playbook
    pub before: Option<String>,
    /// The proposed file
    pub after: String,
    /// `before` → `after` as a unified diff
    pub diff: String,
    pub status: ProposalStatus,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub commit_id: Option<String>,
    pub undo_commit_id: Option<String>,
    pub verdict: Option<RetroVerdict>,
    pub rates: Option<RetroRates>,
}

const COLUMNS: &str = "id, run_id, agent_id, target, path, reason, runs, before, after, diff, \
status, created_at, decided_at, commit_id, undo_commit_id, verdict, rates";

fn bad(column: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("retro_proposals: bad {what}").into(),
    )
}

fn row(row: &Row) -> rusqlite::Result<Proposal> {
    let runs: String = row.get("runs")?;
    let status: String = row.get("status")?;
    let verdict: Option<String> = row.get("verdict")?;
    let rates: Option<String> = row.get("rates")?;
    Ok(Proposal {
        id: row.get("id")?,
        run_id: row.get("run_id")?,
        agent_id: row.get("agent_id")?,
        target: row.get("target")?,
        path: row.get("path")?,
        reason: row.get("reason")?,
        runs: serde_json::from_str(&runs).map_err(|_| bad(6, "runs"))?,
        before: row.get("before")?,
        after: row.get("after")?,
        diff: row.get("diff")?,
        status: ProposalStatus::parse(&status).ok_or_else(|| bad(10, "status"))?,
        created_at: row.get("created_at")?,
        decided_at: row.get("decided_at")?,
        commit_id: row.get("commit_id")?,
        undo_commit_id: row.get("undo_commit_id")?,
        verdict: verdict
            .map(|v| RetroVerdict::parse(&v).ok_or_else(|| bad(15, "verdict")))
            .transpose()?,
        rates: rates
            .map(|r| serde_json::from_str(&r).map_err(|_| bad(16, "rates")))
            .transpose()?,
    })
}

pub fn insert(conn: &Connection, proposal: &Proposal) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO retro_proposals ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
        ),
        params![
            proposal.id,
            proposal.run_id,
            proposal.agent_id,
            proposal.target,
            proposal.path,
            proposal.reason,
            serde_json::to_string(&proposal.runs).unwrap_or_else(|_| "[]".to_string()),
            proposal.before,
            proposal.after,
            proposal.diff,
            proposal.status.as_str(),
            proposal.created_at,
            proposal.decided_at,
            proposal.commit_id,
            proposal.undo_commit_id,
            proposal.verdict.map(RetroVerdict::as_str),
            proposal
                .rates
                .as_ref()
                .and_then(|r| serde_json::to_string(r).ok()),
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Proposal>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM retro_proposals WHERE id = ?1"),
        params![id],
        row,
    )
    .optional()
}

/// Proposals made or decided since `since_ms`, oldest first
pub fn since(conn: &Connection, since_ms: i64) -> rusqlite::Result<Vec<Proposal>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM retro_proposals
          WHERE created_at >= ?1 OR decided_at >= ?1 OR (status = 'applied' AND verdict IS NOT NULL)
          ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map(params![since_ms], row)?;
    rows.collect()
}

/// Applied proposals the check has no verdict on yet
pub fn awaiting_check(conn: &Connection) -> rusqlite::Result<Vec<Proposal>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM retro_proposals
          WHERE status = 'applied' AND verdict IS NULL ORDER BY created_at"
    ))?;
    let rows = stmt.query_map([], row)?;
    rows.collect()
}

/// How many proposals were made since `since_ms`
pub fn count_since(conn: &Connection, since_ms: i64) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM retro_proposals WHERE created_at >= ?1",
        params![since_ms],
        |row| row.get(0),
    )
}

/// Moves `id` from `from` to `to` (with the commit, for apply and undo).
/// `false` when it wasn't at `from`.
pub fn decide(
    conn: &Connection,
    id: &str,
    from: ProposalStatus,
    to: ProposalStatus,
    commit: Option<&str>,
    at: i64,
) -> rusqlite::Result<bool> {
    let column = match to {
        ProposalStatus::Applied => "commit_id",
        ProposalStatus::Undone => "undo_commit_id",
        _ => "",
    };
    let changed = if column.is_empty() {
        conn.execute(
            "UPDATE retro_proposals SET status = ?3, decided_at = COALESCE(decided_at, ?4)
              WHERE id = ?1 AND status = ?2",
            params![id, from.as_str(), to.as_str(), at],
        )?
    } else {
        conn.execute(
            &format!(
                "UPDATE retro_proposals SET status = ?3, {column} = ?5,
                        decided_at = CASE WHEN ?3 = 'applied' THEN ?4 ELSE decided_at END
                  WHERE id = ?1 AND status = ?2"
            ),
            params![id, from.as_str(), to.as_str(), at, commit],
        )?
    };
    Ok(changed == 1)
}

/// Records the check's verdict, once
pub fn set_verdict(
    conn: &Connection,
    id: &str,
    verdict: RetroVerdict,
    rates: &RetroRates,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE retro_proposals SET verdict = ?2, rates = ?3 WHERE id = ?1 AND verdict IS NULL",
        params![
            id,
            verdict.as_str(),
            serde_json::to_string(rates).unwrap_or_default()
        ],
    )?;
    Ok(())
}

/// When the last weekly retro run started, if one did
pub fn last_retro_run(conn: &Connection, trigger_id: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT MAX(started_at) FROM runs WHERE source_id = ?1",
        params![trigger_id],
        |row| row.get(0),
    )
}
