//! Manual workflow steps (wave EF, Track D): a step with
//! `step_type: manual` suspends its run (`waiting`) until a person
//! completes it, which may be days later. The run's state lives in
//! `workflow_runs`, so a restart resumes exactly where it stopped; the
//! `manual_steps` table indexes what's waiting (and when it's due) for the
//! Work view.
//!
//! ```yaml
//! - id: sign-off
//!   step_type: manual
//!   instructions: "Review {{draft.output}} and paste the final copy"
//!   deadline_hours: 48
//! ```
//!
//! What the person hands back (text, plus any attached files) is the
//! step's output, `{{sign-off.output}}` for later steps.

use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, Db};
use crate::model::Timestamp;
use crate::services::attachments;
use crate::services::events::NandiEvent;
use crate::services::AppState;

/// `workflow://manual_step_ready`
pub const MANUAL_STEP_READY: &str = "workflow://manual_step_ready";
/// `workflow://manual_step_completed`
pub const MANUAL_STEP_COMPLETED: &str = "workflow://manual_step_completed";

/// `manual_steps.status` (besides `pending`)
pub const COMPLETED: &str = "completed";
pub const CANCELLED: &str = "cancelled";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualStepConfig {
    pub step_id: String,
    pub instructions: String,
    /// Epoch ms
    pub deadline: Option<i64>,
}

/// A manual step waiting for someone, for the Work view
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingManualStep {
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub step_id: String,
    pub instructions: String,
    /// Epoch ms
    pub deadline: Option<Timestamp>,
    /// When the run reached the step (epoch ms)
    pub ready_at: Timestamp,
}

impl NandiEvent for PendingManualStep {
    const NAME: &'static str = MANUAL_STEP_READY;
}

impl PendingManualStep {
    /// A step of run `run_id` that became ready at `ready_at` (the
    /// workflow's name is filled in by `record_ready`)
    pub fn new(run_id: &str, workflow_id: &str, config: ManualStepConfig, ready_at: i64) -> Self {
        Self {
            run_id: run_id.to_string(),
            workflow_id: workflow_id.to_string(),
            workflow_name: String::new(),
            step_id: config.step_id,
            instructions: config.instructions,
            deadline: config.deadline.map(db::ts),
            ready_at: db::ts(ready_at),
        }
    }
}

/// What the person hands back; `output` becomes `{{step.output}}`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualStepCompletion {
    pub output: Option<String>,
    /// Staged attachment IDs (from the file picker)
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

/// Payload of `workflow://manual_step_completed`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManualStepCompleted {
    pub run_id: String,
    pub workflow_id: String,
    pub step_id: String,
    pub completed_at: Timestamp,
}

impl NandiEvent for ManualStepCompleted {
    const NAME: &'static str = MANUAL_STEP_COMPLETED;
}

// ---------------------------------------------------------------------------
// `manual_steps`
// ---------------------------------------------------------------------------

const COLUMNS: &str = "m.run_id, m.workflow_id, COALESCE(w.name, m.workflow_id), m.step_id,
                       m.instructions, m.deadline, m.created_at";

fn pending_row(row: &Row) -> rusqlite::Result<PendingManualStep> {
    Ok(PendingManualStep {
        run_id: row.get(0)?,
        workflow_id: row.get(1)?,
        workflow_name: row.get(2)?,
        step_id: row.get(3)?,
        instructions: row.get(4)?,
        deadline: row.get::<_, Option<i64>>(5)?.map(db::ts),
        ready_at: db::ts(row.get(6)?),
    })
}

/// Records a step that just became ready (idempotent: a re-recorded step
/// stays pending with its first `ready_at`), returning it with the
/// workflow's name
pub(super) async fn record_ready(
    db: &Db,
    step: PendingManualStep,
) -> Result<PendingManualStep, String> {
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO manual_steps
               (run_id, step_id, workflow_id, status, instructions, deadline, created_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)
             ON CONFLICT(run_id, step_id) DO UPDATE SET
               status = 'pending', instructions = excluded.instructions,
               deadline = excluded.deadline, completed_at = NULL",
            params![
                step.run_id,
                step.step_id,
                step.workflow_id,
                step.instructions,
                step.deadline.map(|d| d as i64),
                step.ready_at as i64,
            ],
        )?;
        conn.query_row(
            &format!(
                "SELECT {COLUMNS} FROM manual_steps m LEFT JOIN workflows w ON w.id = m.workflow_id
                  WHERE m.run_id = ?1 AND m.step_id = ?2"
            ),
            params![step.run_id, step.step_id],
            pending_row,
        )
    })
    .await
}

/// Marks one step `status` (completed or cancelled)
pub(super) async fn close(
    db: &Db,
    run_id: &str,
    step_id: &str,
    status: &'static str,
) -> Result<(), String> {
    let (run_id, step_id) = (run_id.to_string(), step_id.to_string());
    db.call(move |conn| {
        conn.execute(
            "UPDATE manual_steps SET status = ?3, completed_at = ?4
              WHERE run_id = ?1 AND step_id = ?2",
            params![run_id, step_id, status, db::now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// Marks every pending step of a run `status` (e.g. the run was cancelled)
pub(super) async fn close_pending(
    db: &Db,
    run_id: &str,
    status: &'static str,
) -> Result<(), String> {
    let run_id = run_id.to_string();
    db.call(move |conn| {
        conn.execute(
            "UPDATE manual_steps SET status = ?2, completed_at = ?3
              WHERE run_id = ?1 AND status = 'pending'",
            params![run_id, status, db::now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// Pending steps of runs still waiting, oldest first
pub fn pending_conn(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<PendingManualStep>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM manual_steps m
           JOIN workflow_runs r ON r.id = m.run_id AND r.status = 'waiting'
           LEFT JOIN workflows w ON w.id = m.workflow_id
          WHERE m.status = 'pending'
          ORDER BY m.created_at ASC, m.rowid ASC"
    ))?;
    let rows = stmt.query_map([], pending_row)?;
    rows.collect()
}

pub async fn pending(db: &Db) -> Result<Vec<PendingManualStep>, String> {
    db.call(|conn| pending_conn(conn)).await
}

/// The step's output: the person's text, then where each attached file now
/// lives (`~/.nandi/attachments/workflow-<run>-<step>/…`), so later agent
/// steps can read them
async fn output_of(
    run_id: &str,
    step_id: &str,
    completion: ManualStepCompletion,
) -> Result<String, String> {
    let mut output = completion.output.unwrap_or_default().trim().to_string();
    let staged = attachments::resolve_staged(&completion.attachment_ids).await?;
    if staged.is_empty() {
        return Ok(output);
    }
    let turn = format!("workflow-{run_id}-{step_id}");
    let root = crate::utils::nandi_config::nandi_dir()?.join("attachments");
    let files: Vec<String> = staged
        .iter()
        .map(|file| {
            root.join(&turn)
                .join(&file.info.id)
                .join(&file.info.name)
                .display()
                .to_string()
        })
        .collect();
    attachments::commit_staged(staged, &turn).await?;
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str("Attached files:\n");
    for file in files {
        output.push_str(&format!("- {file}\n"));
    }
    Ok(output.trim_end().to_string())
}

// ---------------------------------------------------------------------------
// Service API (shared by the Tauri commands and HTTP routes)
// ---------------------------------------------------------------------------

pub async fn list_pending_manual_steps(state: &AppState) -> Result<Vec<PendingManualStep>, String> {
    pending(state.db()).await
}

pub async fn complete_manual_step(
    state: &AppState,
    run_id: &str,
    step_id: &str,
    completion: ManualStepCompletion,
) -> Result<(), String> {
    let note = completion.output.clone();
    let output = output_of(run_id, step_id, completion).await?;
    state
        .workflows()
        .complete_manual_step(run_id, step_id, output)
        .await?;
    crate::services::inbox::audit::record(
        state,
        crate::services::inbox::audit::Decided::new(
            "manualStep",
            format!("{run_id}/{step_id}"),
            "done",
        )
        .note(note.as_deref()),
    );
    Ok(())
}
