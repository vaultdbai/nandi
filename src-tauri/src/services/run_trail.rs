//! The run trail: the audit-relevant runtime state that must survive a
//! `state.db` reset (owner decision 5, 2026-11-14).
//!
//! Every `run_events` row and every approval decision is appended to one
//! file, `~/.nandi/local/run-trail.jsonl`, as its transaction commits. Each
//! line is a [`TrailEntry`] with a `record` field naming its kind, chained
//! to the line before it like the audit logs (`audit::append_all_blocking`).
//! One file keeps both kinds in the order they happened, with one chain to
//! check.
//!
//! SQLite stays the queryable copy. When `state.db` is new (both tables
//! empty), [`RunTrail::restore`] puts the rows back from the file, the way
//! the gold copy rebuilds the definitions. A broken chain or a line that
//! can't be read is reported (in the [`Restored`] report and the app log),
//! never skipped silently.
//!
//! Only `EventBus` writes the trail: its transaction stages the entries
//! ([`TrailEntry`]) and appends them right after the commit, one fsync per
//! commit. Streamed tokens record no run event, so they never touch it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audit::{self, ChainCheck};
use crate::db::{self, DecidedApproval, RunEventRow};

/// The trail's file name, in `~/.nandi/local/`
pub const FILE_NAME: &str = "run-trail.jsonl";

/// One line of the run trail
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum TrailEntry {
    /// A `run_events` row
    RunEvent(RunEventRow),
    /// An approval as it closed: approved, rejected or expired
    ApprovalDecision(DecidedApproval),
}

/// Where the run trail is written. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct RunTrail {
    /// `None`: no trail (tests that don't look at it)
    path: Option<Arc<PathBuf>>,
}

/// What [`RunTrail::restore`] put back
#[derive(Debug, Clone, PartialEq)]
pub struct Restored {
    pub run_events: u32,
    pub approval_decisions: u32,
    /// Lines that aren't a [`TrailEntry`] (1-based line numbers)
    pub unreadable: Vec<u32>,
    /// Whether every line follows the one before it
    pub chain: ChainCheck,
}

impl Restored {
    /// The trail was read whole and its chain holds
    pub fn is_clean(&self) -> bool {
        self.unreadable.is_empty() && matches!(self.chain, ChainCheck::Intact { .. })
    }
}

impl RunTrail {
    /// A trail at `path`
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(Arc::new(path)),
        }
    }

    /// No trail: nothing is written or restored
    pub fn off() -> Self {
        Self::default()
    }

    /// The trail in `local_dir` (`~/.nandi/local`)
    pub fn in_local_dir(local_dir: &Path) -> Self {
        Self::at(local_dir.join(FILE_NAME))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref().map(PathBuf::as_path)
    }

    /// Appends `entries` in order, with one fsync. Called right after the
    /// transaction that wrote them commits, on its blocking thread. A
    /// failure marks the trail degraded (`audit`), never fails the commit.
    pub fn append(&self, entries: Vec<TrailEntry>) {
        let Some(path) = self.path() else {
            return;
        };
        let lines: Vec<Value> = entries
            .iter()
            .filter_map(|entry| match serde_json::to_value(entry) {
                Ok(line) => Some(line),
                Err(e) => {
                    log::error!("Run trail: an entry could not be written: {e}");
                    None
                }
            })
            .collect();
        audit::append_all_blocking(path, lines);
    }

    /// Puts the trail's rows back into a new `state.db`: only when
    /// `run_events` and `approvals` are both empty, so a database that
    /// already has them is never touched. `None` when there was nothing to
    /// do. Everything readable is restored in one transaction; a broken
    /// chain or an unreadable line is in the report and the app log.
    pub fn restore(&self, conn: &mut Connection) -> Result<Option<Restored>, String> {
        let Some(path) = self.path() else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(None);
        }
        let database_error = |e: rusqlite::Error| format!("Run trail restore failed: {e}");
        if !db::run_trail_tables_empty(conn).map_err(database_error)? {
            return Ok(None);
        }
        let (entries, unreadable) = read(path)?;
        let chain = audit::verify(path)?;

        let tx = conn.transaction().map_err(database_error)?;
        let (mut run_events, mut approval_decisions) = (0, 0);
        for entry in &entries {
            match entry {
                TrailEntry::RunEvent(row) => {
                    db::restore_run_event(&tx, row).map_err(database_error)?;
                    run_events += 1;
                }
                TrailEntry::ApprovalDecision(decided) => {
                    db::restore_approval(&tx, decided).map_err(database_error)?;
                    approval_decisions += 1;
                }
            }
        }
        tx.commit().map_err(database_error)?;

        let restored = Restored {
            run_events,
            approval_decisions,
            unreadable,
            chain,
        };
        report(path, &restored);
        Ok(Some(restored))
    }
}

/// Every [`TrailEntry`] in the file at `path`, in order, and the line
/// numbers of the lines that aren't one
fn read(path: &Path) -> Result<(Vec<TrailEntry>, Vec<u32>), String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    let (mut entries, mut unreadable) = (Vec::new(), Vec::new());
    for (index, line) in BufReader::new(file).split(b'\n').enumerate() {
        let line = line.map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        match serde_json::from_slice::<TrailEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(_) => unreadable.push(index as u32 + 1),
        }
    }
    Ok((entries, unreadable))
}

fn report(path: &Path, restored: &Restored) {
    log::info!(
        "Restored {} run events and {} approval decisions from {}",
        restored.run_events,
        restored.approval_decisions,
        path.display()
    );
    if restored.is_clean() {
        return;
    }
    if let ChainCheck::Broken { line, reason } = &restored.chain {
        log::error!(
            "Run trail {} is broken at line {line} ({reason}): rows from there may have been edited",
            path.display()
        );
    }
    if !restored.unreadable.is_empty() {
        log::error!(
            "Run trail {}: lines {:?} could not be read and were not restored",
            path.display(),
            restored.unreadable
        );
    }
}

#[cfg(test)]
mod tests;
