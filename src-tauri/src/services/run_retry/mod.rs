//! Nandi's own work heals itself. A run Nandi started on its own (a
//! schedule like the morning digest, a file or webhook trigger, the email
//! or calendar check) that failed is tried again a few times before it is
//! left for the user.
//!
//! - **What is tried again:** failed runs from [`RETRIED`] sources that
//!   ended in the last [`RETRY_WINDOW`]. Never the user's own messages
//!   (they are there to see it), workflow steps (workflows resume
//!   themselves), A2A tasks (their own failover) or Request steps.
//! - **What isn't:** a run a middleware stopped (a turn, cost or token
//!   cap, read-only): those are the user's limits. A cancelled run isn't
//!   failed.
//! - **How:** a new run in the same conversation with the first try's
//!   input and a note on what went wrong, so the model sees the earlier
//!   attempt and works around it ([`retry_input`]). It names the run it
//!   tries again (`runs.retry_of`, `runs.attempt`), and
//!   `run_waiter::newest_run` follows it: Work, the Inbox and a schedule's
//!   overlap check see the latest attempt, so a failure a retry fixed is
//!   gone from the Inbox.
//! - **When:** [`BACKOFF`] after the failed attempt, [`MAX_ATTEMPTS`] tries
//!   in all. The database is the schedule: the sweep every [`SWEEP_EVERY`]
//!   finds what is due, so a retry due while the app was closed runs at the
//!   next start, and a run the app was closed in
//!   (`EventBus::fail_interrupted_runs`) is tried again too.
//! - **By hand:** [`retry_run`] tries a failed run again now, whatever its
//!   attempt (the Inbox's Retry).

pub mod store;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use self::store::FailedAttempt;
use crate::db;
use crate::model::SourceType;
use crate::services::events::truncate;
use crate::services::orchestrator::RunSource;
use crate::services::{chat, AppState};

/// Tries in all, the first one included
pub const MAX_ATTEMPTS: u32 = 3;
/// The wait after the first and the second failed attempt
pub const BACKOFF: [Duration; 2] = [Duration::from_secs(2 * 60), Duration::from_secs(10 * 60)];
/// Failures older than this are left alone (a digest from yesterday is
/// no use today)
pub const RETRY_WINDOW: Duration = Duration::from_secs(6 * 60 * 60);
/// How often the sweep looks for retries that are due
pub const SWEEP_EVERY: Duration = Duration::from_secs(60);
/// How long a retry that couldn't start (the agent is gone) waits
const HOLD_AFTER_ERROR: Duration = Duration::from_secs(10 * 60);
/// The error quoted in a retry's note
const ERROR_CHARS: usize = 300;

/// The sources whose runs Nandi started on its own
pub const RETRIED: &[SourceType] = &[
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

/// One retry starts at a time, so the sweep and Retry never both try the
/// same run again
static STARTING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));
/// Runs whose retry couldn't start, and when to try again
static HELD: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Default::default);

fn ms(duration: Duration) -> i64 {
    duration.as_millis() as i64
}

/// When attempt `attempt`, which failed at `ended_at`, is tried again;
/// `None` once it was the last
pub fn due_at(ended_at: i64, attempt: u32) -> Option<i64> {
    let wait = BACKOFF.get(usize::try_from(attempt.checked_sub(1)?).ok()?)?;
    Some(ended_at + ms(*wait))
}

/// What a retry asks: the first try's input, then what went wrong.
/// `asked`: the user asked for it (Retry), not the sweep.
pub fn retry_input(first_input: &str, attempt: u32, error: Option<&str>, asked: bool) -> String {
    let why = error.map(str::trim).filter(|e| !e.is_empty()).map_or_else(
        || "no reason was recorded".to_string(),
        |e| truncate(e, ERROR_CHARS),
    );
    let who = if asked {
        "The user asked to try this again".to_string()
    } else {
        format!("Attempt {attempt} of {MAX_ATTEMPTS}")
    };
    format!(
        "{first_input}\n\n(Nandi: {who}. The last attempt failed: {why}. If it got partway, \
check what was already done before you repeat anything.)"
    )
}

/// When Nandi tries `run_id` again on its own, if it will: a failed run
/// from a [`RETRIED`] source, inside the window, not stopped by a policy,
/// not yet tried again and not on its last attempt. Work shows it waiting
/// on that time instead of failed.
pub fn next_try(conn: &Connection, run_id: &str, now: i64) -> rusqlite::Result<Option<i64>> {
    let Some(failed) = store::failed_attempt(conn, run_id)? else {
        return Ok(None);
    };
    let Some(ended_at) = failed.run.ended_at.map(|at| at as i64) else {
        return Ok(None);
    };
    if !RETRIED.contains(&failed.run.source_type)
        || ended_at < now - ms(RETRY_WINDOW)
        || store::retry_for(conn, run_id)?.is_some()
        || store::stopped_by_policy(conn, run_id)?
    {
        return Ok(None);
    }
    Ok(due_at(ended_at, failed.attempt).map(|due| due.max(now)))
}

fn held(run_id: &str, now: i64) -> bool {
    let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.get(run_id).is_some_and(|until| *until > now)
}

fn hold(run_id: &str, until: i64) {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.retain(|_, at| *at > db::now_ms());
    held.insert(run_id.to_string(), until);
}

/// Starts every retry that is due at `now`; returns the new runs' IDs
pub async fn sweep(state: &AppState, now: i64) -> Vec<String> {
    let _one = STARTING.lock().await;
    let since = now - ms(RETRY_WINDOW);
    let failures = state
        .db()
        .call(move |conn| store::open_failures(conn, since, RETRIED))
        .await
        .unwrap_or_else(|e| {
            log::warn!("Retries: failed runs not read: {e}");
            Vec::new()
        });
    let mut started = Vec::new();
    for failed in failures {
        let Some(due) = failed
            .run
            .ended_at
            .and_then(|at| due_at(at as i64, failed.attempt))
        else {
            continue;
        };
        if due > now || held(&failed.run.id, now) {
            continue;
        }
        match start(state, &failed, false).await {
            Ok(run_id) => started.push(run_id),
            Err(e) => {
                log::warn!("Couldn't try run {} again: {e}", failed.run.id);
                hold(&failed.run.id, now + ms(HOLD_AFTER_ERROR));
            }
        }
    }
    started
}

/// Tries failed run `run_id` again now, at the user's request; returns
/// the new run's ID
pub async fn retry_run(state: &AppState, run_id: String) -> Result<String, String> {
    let _one = STARTING.lock().await;
    let id = run_id.clone();
    let (failed, retried) = state
        .db()
        .call(move |conn| {
            Ok((
                store::failed_attempt(conn, &id)?,
                store::retry_for(conn, &id)?,
            ))
        })
        .await?;
    let Some(failed) = failed else {
        return Err(format!(
            "Run {run_id} didn't fail, so there is nothing to try again"
        ));
    };
    // A line of attempts never forks: Retry acts on its latest attempt
    if retried.is_some() {
        return Err("This run was already tried again".to_string());
    }
    start(state, &failed, true).await
}

/// Starts the retry of `failed`: the next attempt, in the same
/// conversation, from the same source
async fn start(state: &AppState, failed: &FailedAttempt, asked: bool) -> Result<String, String> {
    let run = &failed.run;
    let attempt = failed.attempt + 1;
    let text = retry_input(&failed.first_input, attempt, run.summary.as_deref(), asked);
    let source = RunSource {
        source_type: run.source_type,
        source_id: run.source_id.clone(),
        source_label: run.source_label.clone(),
        priority: run.priority,
    };
    let sent = chat::submit(
        state.orchestrators(),
        &run.agent_id,
        run.conversation_id.clone(),
        text,
        Vec::new(),
        None,
        source,
    )
    .await?;
    let (id, of) = (sent.run_id.clone(), run.id.clone());
    state
        .db()
        .call(move |conn| store::link(conn, &id, &of, attempt))
        .await?;
    log::info!(
        "Trying run {} of '{}' again (attempt {attempt}): run {}",
        run.id,
        run.agent_id,
        sent.run_id
    );
    Ok(sent.run_id)
}

/// Sweeps every [`SWEEP_EVERY`] while the app runs, the first time at once
pub fn start_sweeper(state: &AppState) {
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            sweep(&state, db::now_ms()).await;
        }
    });
}
