//! The GitHub security check: with the GitHub sign-in (`auth::github`),
//! Nandi reads the account's and its owned orgs' settings for the
//! weaknesses attackers use (two-factor sign-in off, a public Backup
//! repository, secret scanning off, default branches open to force-push)
//! and keeps one `check` to-do per finding (`gateway::todos`, source
//! `security_check`, a stable `source_ref` per finding).
//!
//! A host-side, read-only check: [`reader`] makes `GET` calls to
//! `api.github.com` only, for settings and metadata; nothing runs in a
//! container. The rules and the to-do steps are pure
//! (`domain::github_security`). It runs in the background after a sign-in
//! ([`check_soon`]) and from Settings › Account ([`check`]); it never
//! blocks either, and it needs no relay.
//!
//! | Step                     | To-do                                         |
//! | ------------------------ | --------------------------------------------- |
//! | a new finding            | added by Nandi (`added`)                      |
//! | still there              | unchanged                                     |
//! | fixed, or gone           | Nandi closes it: "Fixed: checked again on …"  |
//! | back after Nandi closed  | Nandi opens it again (`reopened`)             |
//! | GitHub didn't show it    | unchanged                                     |

pub mod reader;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::model::Timestamp;
use crate::services::auth::github;
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::domain::github_security::{
    self as rules, fixed_note, github_repo_of, ExistingCheck, Step,
};
use crate::services::domain::todo::{NewTodo, TodoKind, TodoRefs, TodoSource};
use crate::services::gateway::assignments::{self, AssignmentStatus};
use crate::services::gateway::todos::{self, Actor};
use crate::services::AppState;
use crate::utils::nandi_config;

/// The last check, kept in `config.json` (`githubSecurity`) for Settings
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GithubSecurityStatus {
    /// Epoch ms; `None` before the first check
    pub checked_at: Option<Timestamp>,
    /// What it found, open as to-dos
    pub findings: u32,
}

/// The last check's time and findings
pub fn status(state: &AppState) -> GithubSecurityStatus {
    nandi_config::load_from(state.nandi_dir())
        .ok()
        .and_then(|config| config.github_security)
        .unwrap_or_default()
}

/// Checks now, against `api.github.com`
pub async fn check(state: &AppState) -> Result<GithubSecurityStatus, String> {
    check_at(state, reader::API_URL).await
}

/// Checks in the background (after a sign-in); never waited on
pub fn check_soon(state: &AppState) {
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        if let Some(state) = weak.upgrade() {
            if let Err(e) = check(&state).await {
                log::warn!("GitHub security check failed: {e}");
            }
        }
    });
}

/// Checks against `api` (a fake GitHub in tests)
pub(crate) async fn check_at(state: &AppState, api: &str) -> Result<GithubSecurityStatus, String> {
    let token = github::token(state.secrets()).ok_or("Sign in to GitHub first")?;
    let backup = nandi_config::load_from(state.nandi_dir())?
        .remote
        .and_then(|remote| github_repo_of(&remote.url));
    let mut reader = reader::Reader::new(api, token);
    let snapshot = reader.read(backup).await;
    let run = rules::evaluate(&snapshot);
    log::info!(
        "GitHub security check: {} findings, {} API calls",
        run.failed(),
        reader.calls()
    );
    apply(state, &run).await?;
    let status = GithubSecurityStatus {
        checked_at: Some(now_ms() as f64),
        findings: run.failed() as u32,
    };
    let saved = status.clone();
    nandi_config::update_in(state.nandi_dir(), |config| {
        config.github_security = Some(saved);
    })?;
    Ok(status)
}

/// Brings the security-check to-dos in line with `run`
async fn apply(state: &AppState, run: &rules::CheckRun) -> Result<(), String> {
    let existing: Vec<ExistingCheck> = state
        .db()
        .call(|conn| assignments::all_from(conn, TodoSource::SecurityCheck))
        .await?
        .into_iter()
        .filter_map(|todo| {
            Some(ExistingCheck {
                source_ref: todo.source_ref?,
                task_id: todo.task_id,
                open: todo.status == AssignmentStatus::Open,
                note: todo.note,
            })
        })
        .collect();
    let today = chrono::Local::now().date_naive();
    let nandi = || Actor::Agent(DEFAULT_AGENT.to_string());
    for step in rules::plan(run, &existing) {
        // One step failing (closed meanwhile) doesn't stop the others
        let done = match step {
            Step::Add { source_ref, text } => {
                let refs = TodoRefs {
                    source: Some(&source_ref),
                    related: None,
                };
                match NewTodo::new(
                    &text,
                    TodoKind::Check,
                    None,
                    TodoSource::SecurityCheck,
                    refs,
                    now_ms(),
                    &chrono::Local,
                ) {
                    Ok(todo) => todos::add(state, todo, nandi()).await.map(|_| ()),
                    Err(e) => Err(e.to_string()),
                }
            }
            Step::Close { task_id } => {
                todos::done(state, task_id, nandi(), Some(fixed_note(today)))
                    .await
                    .map(|_| ())
            }
            Step::Reopen { task_id } => todos::reopen(state, task_id, nandi()).await.map(|_| ()),
        };
        if let Err(e) = done {
            log::warn!("GitHub security check: couldn't update a to-do: {e}");
        }
    }
    Ok(())
}
