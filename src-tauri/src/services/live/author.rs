//! Who changed a file in a watched folder, when the app didn't write it
//! itself (`own_writes`).
//!
//! Agents write from their container (their work folder) or through the
//! `filesystem` connector (My notes), neither of which says which run wrote
//! what. So a change is put down to the run that was going when it
//! happened:
//!
//! | Folder                  | Written by                                                      |
//! | ----------------------- | --------------------------------------------------------------- |
//! | `work/<agent>/`         | that agent's run, else outside Nandi                            |
//! | My notes, agents write  | Chief of Staff's run, else the latest run, else outside Nandi   |
//! | My notes, read-only     | outside Nandi (agents can't write there)                        |
//!
//! A run started by a workflow is shown as that workflow.

use rusqlite::Connection;

use super::ChangeAuthor;
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::repo_store::RepoStore;

/// A run that ended this recently still counts: the watcher reports a
/// change up to two seconds late (`watcher::MAX_DELAY`)
pub const RECENT_RUN_MS: i64 = 10_000;

/// A run going on (or just ended) when a file changed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRun {
    pub agent_id: String,
    /// The workflow that started it, if one did
    pub workflow_id: Option<String>,
}

/// Where the change happened
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place<'a> {
    /// An agent's work folder
    Work { agent_id: &'a str },
    /// My notes; `agents_can_write` when the share lets agents write
    Notes { agents_can_write: bool },
}

/// The run that made a change at `place`, from the runs going at the time
/// (newest first). `None`: outside Nandi.
pub fn writer<'a>(place: Place<'_>, runs: &'a [ActiveRun]) -> Option<&'a ActiveRun> {
    match place {
        Place::Work { agent_id } => runs.iter().find(|run| run.agent_id == agent_id),
        Place::Notes {
            agents_can_write: false,
        } => None,
        Place::Notes {
            agents_can_write: true,
        } => runs
            .iter()
            .find(|run| run.agent_id == CHIEF_OF_STAFF)
            .or_else(|| runs.first()),
    }
}

/// Runs going now or ended since `since_ms`, newest first
pub fn active_runs(conn: &Connection, since_ms: i64) -> rusqlite::Result<Vec<ActiveRun>> {
    let mut statement = conn.prepare(
        "SELECT r.agent_id, w.workflow_id FROM runs r
           LEFT JOIN workflow_runs w ON r.source_type = 'workflow' AND w.id = r.source_id
          WHERE r.status IN ('running', 'awaiting_approval') OR r.ended_at >= ?1
          ORDER BY r.started_at DESC",
    )?;
    let rows = statement.query_map([since_ms], |row| {
        Ok(ActiveRun {
            agent_id: row.get(0)?,
            workflow_id: row.get(1)?,
        })
    })?;
    rows.collect()
}

/// The author a run stands for, named as the user knows it
pub fn author_of(run: Option<&ActiveRun>, store: &RepoStore) -> ChangeAuthor {
    let Some(run) = run else {
        return ChangeAuthor::Outside;
    };
    if let Some(workflow_id) = &run.workflow_id {
        let workflow_name = store
            .read(|repo| repo.workflow(workflow_id).map(|w| w.name().to_string()))
            .unwrap_or_else(|| workflow_id.clone());
        return ChangeAuthor::Workflow {
            workflow_id: workflow_id.clone(),
            workflow_name,
        };
    }
    ChangeAuthor::Agent {
        agent_id: run.agent_id.clone(),
        agent_name: store.display_name(&run.agent_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(agent: &str, workflow: Option<&str>) -> ActiveRun {
        ActiveRun {
            agent_id: agent.into(),
            workflow_id: workflow.map(str::to_string),
        }
    }

    #[test]
    fn a_work_folder_is_written_by_its_own_agents_run() {
        let runs = [run("writer", None), run("analyst", Some("weekly"))];
        let found = writer(
            Place::Work {
                agent_id: "analyst",
            },
            &runs,
        );
        assert_eq!(found, Some(&runs[1]));
        assert_eq!(writer(Place::Work { agent_id: "nobody" }, &runs), None);
        assert_eq!(writer(Place::Work { agent_id: "writer" }, &[]), None);
    }

    #[test]
    fn notes_are_written_by_chief_of_staff_first_then_the_latest_run() {
        let notes = Place::Notes {
            agents_can_write: true,
        };
        let runs = [run("writer", None), run(CHIEF_OF_STAFF, None)];
        assert_eq!(writer(notes, &runs), Some(&runs[1]));
        let runs = [run("writer", Some("weekly")), run("analyst", None)];
        assert_eq!(writer(notes, &runs), Some(&runs[0]));
        assert_eq!(writer(notes, &[]), None);
        // Read-only notes: no agent could have written them
        let read_only = Place::Notes {
            agents_can_write: false,
        };
        assert_eq!(writer(read_only, &[run(CHIEF_OF_STAFF, None)]), None);
    }
}
