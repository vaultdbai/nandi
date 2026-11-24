//! The weekly retro: Nandi gets better from the person's corrections
//! (task "Weekly retro"). Nandi's code never changes itself; what gets
//! better is state already in the gold copy.
//!
//! | Step    | What happens                                                    | Who     |
//! | ------- | --------------------------------------------------------------- | ------- |
//! | Observe | Last week's runs: refusals, edited drafts, retries, failures     | records |
//! | Reflect | Nandi reads them on the weekly trigger (`weekly-retro` playbook) | model   |
//! | Propose | At most three changes, each an Inbox item with its diff and runs | model   |
//! | Apply   | A yes is one gold copy commit with the reason and the runs       | Nandi   |
//! | Undo    | One click: one more commit puts the file back                    | Nandi   |
//! | Verify  | The agent's correction rate after vs before ([`check`])          | Nandi   |
//!
//! The model reads and proposes; this module keeps the record, the limits
//! and the check. The limits (`domain::retro`) are checked in code when a
//! change is proposed and again when it is applied: a change may edit an
//! agent's `agent.md`, a playbook, a workflow or a trigger, and narrow
//! access, never widen it. Agents from a signed pack are never edited.
//!
//! The retro's tools ([`tool`]) are offered only in the weekly trigger's
//! run, which runs at most once a week ([`guard`]), and it proposes at most
//! [`MAX_PER_WEEK`] changes a week. Proposals live in `state.db`
//! ([`proposals`]); a change and its undo live in the gold copy.

pub mod check;
pub mod guard;
pub mod proposals;
pub mod records;
pub mod target;
pub mod tool;

#[cfg(test)]
mod tests;

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;

use self::check::{RetroRates, RetroVerdict};
use self::proposals::{Proposal, ProposalStatus};
use self::target::Target;
use crate::db::{self, Db};
use crate::model::Timestamp;
use crate::services::events::{self, NandiEvent};
use crate::services::repo_service::GitService;
use crate::services::repo_store::RepoStore;
use crate::services::AppState;

pub use crate::services::builtin_agents::WEEKLY_RETRO;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// A week
pub const WEEK_MS: i64 = 7 * DAY_MS;
/// Proposals a week, at most
pub const MAX_PER_WEEK: u32 = 3;
/// How long a proposal waits for the person
pub const WAITS_MS: i64 = WEEK_MS;
/// How long an applied change stays in the Inbox with its Undo
pub const SHOWN_AFTER_APPLY_MS: i64 = WEEK_MS;
/// The longest proposed file
const MAX_CONTENT_CHARS: usize = 64 * 1024;
/// The most runs one proposal cites
const MAX_RUNS: usize = 10;
/// The longest reason
const MAX_REASON_CHARS: usize = 300;

/// One apply or undo at a time
static WRITING: Mutex<()> = Mutex::new(());

/// `retro://changed`: a proposal was made, decided or checked
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RetroChanged {}

impl NandiEvent for RetroChanged {
    const NAME: &'static str = "retro://changed";
}

/// A proposal as the Inbox shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RetroItem {
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    /// `agent`, `playbook:<name>`, `workflow:<id>` or `trigger:<id>`
    pub target: String,
    /// Its file in the gold copy
    pub path: String,
    pub reason: String,
    /// The runs it learned from
    pub runs: Vec<String>,
    /// What changes, as a unified diff
    pub diff: String,
    pub status: ProposalStatus,
    pub created_at: Timestamp,
    /// When the person said yes (or no)
    pub decided_at: Option<Timestamp>,
    /// When a pending proposal stops waiting
    pub expires_at: Option<Timestamp>,
    /// The gold copy commit of the change
    pub commit_id: Option<String>,
    /// The check's verdict, once there is one
    pub verdict: Option<RetroVerdict>,
    pub rates: Option<RetroRates>,
    /// The change didn't help: the note offering the undo
    pub needs_note: bool,
}

/// What the retro works with: the gold copy, its git and `state.db`
#[derive(Clone)]
pub struct Retro {
    pub store: RepoStore,
    pub git: std::sync::Arc<GitService>,
    pub db: Db,
}

impl Retro {
    pub fn of(state: &AppState) -> Self {
        Self {
            store: state.store().clone(),
            git: state.git_service().clone(),
            db: state.db().clone(),
        }
    }

    fn conn(&self) -> Result<crate::db::DbConn, String> {
        self.db.conn()
    }
}

/// What `retro_propose` asks for
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProposeInput {
    pub agent: String,
    pub target: String,
    pub content: String,
    pub reason: String,
    pub runs: Vec<String>,
}

fn sql(e: rusqlite::Error) -> String {
    format!("Database error: {e}")
}

/// Records a proposal from retro run `run_id`, checked against the limits
/// first. Nothing in the gold copy changes.
pub fn propose(
    retro: &Retro,
    run_id: &str,
    input: ProposeInput,
    now: i64,
) -> Result<Proposal, String> {
    let reason = input.reason.trim().to_string();
    if reason.is_empty() || reason.chars().count() > MAX_REASON_CHARS {
        return Err(format!(
            "reason is required: one line for the person, {MAX_REASON_CHARS} characters at most"
        ));
    }
    let mut runs: Vec<String> = input
        .runs
        .iter()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    runs.dedup();
    if runs.is_empty() || runs.len() > MAX_RUNS {
        return Err(format!(
            "runs is required: the IDs of the runs it learned from (1 to {MAX_RUNS})"
        ));
    }
    if input.content.chars().count() > MAX_CONTENT_CHARS {
        return Err("content is too long".to_string());
    }
    let conn = retro.conn()?;
    let made = proposals::count_since(&conn, now - WEEK_MS).map_err(sql)?;
    if made >= MAX_PER_WEEK {
        return Err(format!(
            "You've made {MAX_PER_WEEK} proposals this week, the most there are. Stop here."
        ));
    }
    let known = records::runs(&conn, now - WEEK_MS - DAY_MS, now + 1).map_err(sql)?;
    if let Some(unknown) = runs.iter().find(|r| !known.iter().any(|k| &k.id == *r)) {
        return Err(format!(
            "There is no run '{unknown}' last week: cite runs from `retro_runs`"
        ));
    }
    let agent = input.agent.trim();
    let target = Target::parse(&input.target)?;
    let resolved = target::resolve(&retro.store, agent, &target)?;
    if resolved.current.as_deref() == Some(input.content.as_str()) {
        return Err("That is the file as it is now: nothing would change".to_string());
    }
    target::check(agent, &target, resolved.current.as_deref(), &input.content)?;
    let waiting = proposals::since(&conn, now - WAITS_MS)
        .map_err(sql)?
        .into_iter()
        .any(|p| {
            p.status == ProposalStatus::Pending
                && p.agent_id == agent
                && p.target == target.as_string()
        });
    if waiting {
        return Err(format!(
            "A proposal for {agent}'s {} already waits for the person",
            target.as_string()
        ));
    }
    let diff = crate::services::ask::editor::unified_diff(
        &resolved.relative,
        resolved.current.as_deref().unwrap_or_default(),
        &input.content,
    )
    .map_err(|e| e.to_string())?;
    let proposal = Proposal {
        id: uuid::Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        agent_id: agent.to_string(),
        target: target.as_string(),
        path: resolved.relative,
        reason,
        runs,
        before: resolved.current,
        after: input.content,
        diff,
        status: ProposalStatus::Pending,
        created_at: now,
        decided_at: None,
        commit_id: None,
        undo_commit_id: None,
        verdict: None,
        rates: None,
    };
    proposals::insert(&conn, &proposal).map_err(sql)?;
    Ok(proposal)
}

fn load(retro: &Retro, id: &str) -> Result<Proposal, String> {
    proposals::get(&*retro.conn()?, id)
        .map_err(sql)?
        .ok_or_else(|| "That proposal wasn't found".to_string())
}

/// The commit message of an applied change: the reason and the runs
pub fn commit_message(proposal: &Proposal) -> String {
    format!(
        "Weekly retro: {}\n\nChanges {} of agent {} ({}), as the person approved.\nRuns it learned from: {}\nRetro proposal: {}\n",
        proposal.reason,
        proposal.target,
        proposal.agent_id,
        proposal.path,
        proposal.runs.join(", "),
        proposal.id
    )
}

/// Writes `text` (or removes the file) and reloads its entity; puts
/// `restore` back when the reload refuses it
fn write(
    retro: &Retro,
    resolved: &target::Resolved,
    text: Option<&str>,
    restore: Option<&str>,
) -> Result<(), String> {
    let repos = retro.store.repos();
    repos
        .write_definition_text(&resolved.path, text)
        .map_err(|e| e.to_string())?;
    if let Err(e) = retro.store.commit(resolved.kind, &resolved.entity_id) {
        let _ = repos.write_definition_text(&resolved.path, restore);
        let _ = retro.store.commit(resolved.kind, &resolved.entity_id);
        return Err(format!(
            "The change doesn't load, so it wasn't applied: {e}"
        ));
    }
    Ok(())
}

/// The person's yes: the limits are checked again, against the file as it
/// is now, then the change is written and committed on its own
pub fn apply(retro: &Retro, id: &str, now: i64) -> Result<Proposal, String> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let proposal = load(retro, id)?;
    if proposal.status != ProposalStatus::Pending {
        return Err("This proposal no longer waits".to_string());
    }
    if now - proposal.created_at > WAITS_MS {
        return Err("This proposal waited over a week, so it expired".to_string());
    }
    let target = Target::parse(&proposal.target)?;
    let resolved = target::resolve(&retro.store, &proposal.agent_id, &target)?;
    if resolved.current != proposal.before {
        return Err(format!(
            "{} changed since Nandi proposed this, so it wasn't applied. The next retro can look again.",
            proposal.path
        ));
    }
    target::check(
        &proposal.agent_id,
        &target,
        proposal.before.as_deref(),
        &proposal.after,
    )?;
    // Uncommitted edits to the file are the person's: kept in history first
    retro.git.commit_files(
        std::slice::from_ref(&proposal.path),
        &format!("Keep {} before the weekly retro's change", proposal.path),
        None,
    )?;
    write(
        retro,
        &resolved,
        Some(&proposal.after),
        proposal.before.as_deref(),
    )?;
    let commit = retro
        .git
        .commit_files(
            std::slice::from_ref(&proposal.path),
            &commit_message(&proposal),
            None,
        )?
        .ok_or("Nothing changed")?;
    proposals::decide(
        &*retro.conn()?,
        id,
        ProposalStatus::Pending,
        ProposalStatus::Applied,
        Some(&commit),
        now,
    )
    .map_err(sql)?;
    log::info!(
        "Applied retro proposal {id} to {} in {commit}",
        proposal.path
    );
    load(retro, id)
}

/// Undo: the file goes back to what it was before the change, in one more
/// commit. Refused when the person edited it since (their edit stays).
pub fn undo(retro: &Retro, id: &str, now: i64) -> Result<Proposal, String> {
    let _writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let proposal = load(retro, id)?;
    if !matches!(
        proposal.status,
        ProposalStatus::Applied | ProposalStatus::Kept
    ) {
        return Err("This change isn't applied".to_string());
    }
    let target = Target::parse(&proposal.target)?;
    let resolved = target::resolve(&retro.store, &proposal.agent_id, &target)?;
    if resolved.current.as_deref() != Some(proposal.after.as_str()) {
        return Err(format!(
            "{} changed since this was applied, so Nandi leaves it as it is. Change it on its page.",
            proposal.path
        ));
    }
    write(
        retro,
        &resolved,
        proposal.before.as_deref(),
        Some(&proposal.after),
    )?;
    let commit = retro
        .git
        .commit_files(
            std::slice::from_ref(&proposal.path),
            &format!(
                "Undo the weekly retro's change: {}\n\nPuts {} back as it was before {}.\nRetro proposal: {}\n",
                proposal.reason,
                proposal.path,
                proposal.commit_id.as_deref().unwrap_or("the change"),
                proposal.id
            ),
            None,
        )?
        .ok_or("Nothing changed")?;
    proposals::decide(
        &*retro.conn()?,
        id,
        proposal.status,
        ProposalStatus::Undone,
        Some(&commit),
        now,
    )
    .map_err(sql)?;
    log::info!("Undid retro proposal {id} in {commit}");
    load(retro, id)
}

/// The person's no
pub fn refuse(retro: &Retro, id: &str, now: i64) -> Result<Proposal, String> {
    let changed = proposals::decide(
        &*retro.conn()?,
        id,
        ProposalStatus::Pending,
        ProposalStatus::Refused,
        None,
        now,
    )
    .map_err(sql)?;
    if !changed {
        return Err("This proposal no longer waits".to_string());
    }
    load(retro, id)
}

/// Keep it: the person keeps a change the check said didn't help
pub fn keep(retro: &Retro, id: &str, now: i64) -> Result<Proposal, String> {
    let changed = proposals::decide(
        &*retro.conn()?,
        id,
        ProposalStatus::Applied,
        ProposalStatus::Kept,
        None,
        now,
    )
    .map_err(sql)?;
    if !changed {
        return Err("This change isn't applied".to_string());
    }
    load(retro, id)
}

/// The check, for every applied change without a verdict: its agent's
/// rates from the run records, and a verdict once there are enough
/// decisions. Returns how many got one.
pub fn refresh_checks(retro: &Retro, now: i64) -> Result<usize, String> {
    let conn = retro.conn()?;
    let waiting = proposals::awaiting_check(&conn).map_err(sql)?;
    let Some(earliest) = waiting.iter().filter_map(|p| p.decided_at).min() else {
        return Ok(0);
    };
    let decisions = records::decisions(&conn, earliest - check::BEFORE_MS, now + 1).map_err(sql)?;
    let mut decided = 0;
    for proposal in &waiting {
        let Some(applied_at) = proposal.decided_at else {
            continue;
        };
        let rates = check::rates(&decisions, &proposal.agent_id, applied_at);
        if let Some(verdict) = check::verdict(&rates, applied_at, now) {
            proposals::set_verdict(&conn, &proposal.id, verdict, &rates).map_err(sql)?;
            decided += 1;
        }
    }
    Ok(decided)
}

/// What the Inbox shows: proposals waiting (a week at most), changes
/// applied this week (with their Undo), and changes the check says didn't
/// help (the note), after running the check
pub fn items(retro: &Retro, now: i64) -> Result<Vec<RetroItem>, String> {
    refresh_checks(retro, now)?;
    let conn = retro.conn()?;
    let mut items = Vec::new();
    for p in proposals::since(&conn, now - WEEK_MS).map_err(sql)? {
        let needs_note =
            p.status == ProposalStatus::Applied && check::needs_note(p.verdict, p.rates.as_ref());
        let shown = match p.status {
            ProposalStatus::Pending => now - p.created_at <= WAITS_MS,
            ProposalStatus::Applied => {
                needs_note
                    || p.decided_at
                        .is_some_and(|at| now - at <= SHOWN_AFTER_APPLY_MS)
            }
            _ => false,
        };
        if !shown {
            continue;
        }
        items.push(RetroItem {
            agent_name: retro.store.display_name(&p.agent_id),
            expires_at: (p.status == ProposalStatus::Pending)
                .then(|| db::ts(p.created_at + WAITS_MS)),
            created_at: db::ts(p.created_at),
            decided_at: p.decided_at.map(db::ts),
            id: p.id,
            agent_id: p.agent_id,
            target: p.target,
            path: p.path,
            reason: p.reason,
            runs: p.runs,
            diff: p.diff,
            status: p.status,
            commit_id: p.commit_id,
            verdict: p.verdict,
            rates: p.rates,
            needs_note,
        });
    }
    Ok(items)
}

/// One of the actions on a proposal, from its Inbox card or chat
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RetroAction {
    Apply,
    Refuse,
    Undo,
    Keep,
}

async fn blocking<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce(&Retro) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let retro = Retro::of(state);
    tokio::task::spawn_blocking(move || f(&retro))
        .await
        .map_err(|e| format!("The retro stopped: {e}"))?
}

/// The Inbox's retro items now
pub async fn list(state: &AppState) -> Result<Vec<RetroItem>, String> {
    blocking(state, |retro| items(retro, db::now_ms())).await
}

/// Applies, refuses, undoes or keeps proposal `id`, then tells the views
pub async fn act(state: &AppState, id: String, action: RetroAction) -> Result<(), String> {
    blocking(state, move |retro| {
        let now = db::now_ms();
        match action {
            RetroAction::Apply => apply(retro, &id, now),
            RetroAction::Refuse => refuse(retro, &id, now),
            RetroAction::Undo => undo(retro, &id, now),
            RetroAction::Keep => keep(retro, &id, now),
        }
        .map(|_| ())
    })
    .await?;
    events::emit(state.bus(), &RetroChanged {});
    Ok(())
}
