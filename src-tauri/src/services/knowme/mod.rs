//! Getting to know you: Nandi learns what the person does and what they'd
//! like help with, a little at a time, in an Ask chat with the Chief of
//! Staff. The questions are never "done"; nothing blocks and nothing nags.
//!
//! - **The to-do** (Inbox kind `knowMe`): the first one comes after
//!   onboarding, then at most one a month ([`cadence`], `local_settings`).
//!   Later and Skip close it.
//! - **The chat** is one standing Inbox thread, [`THREAD_ID`], opened from
//!   the to-do or any time from Network › Sharing › What you offer ("Tell
//!   Nandi about you"). The Chief of Staff asks (its `know-me` playbook)
//!   and proposes changes with `knowme_propose` (`inbox::tool`), offered
//!   only there and always confirmed on its card, where the person can edit
//!   it first ([`confirm`]).
//! - **On confirm** ([`apply`]) the proposal ([`proposal`]) writes NANDI.md
//!   through the gateway service and My notes through `services::notes`,
//!   records the decision (`inbox::audit`, kind `knowMe`) and closes the
//!   to-do. Personal items never reach NANDI.md; one may be shared with one
//!   group I'm in, which only that group's members' requests read
//!   ([`sharing`]).
//!
//! Everything here works with no relay.

pub mod cadence;
pub mod proposal;
pub mod sharing;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use self::cadence::Cadence;
use self::proposal::{add_lines, apply_to_manifest, GroupRef, KnowMeProposal};
use crate::model::{ApprovalDecision, ApprovalStatus};
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::domain::decision::{select, DecisionFilter, DecisionRecord};
use crate::services::events::{self, NandiEvent};
use crate::services::inbox::audit::{self, Decided};
use crate::services::AppState;

/// The Inbox kind, and the decisions file's
pub const KIND: &str = "knowMe";
/// The one know-me chat
pub const THREAD_ID: &str = "inbox:knowMe:you";
/// How many confirmed changes the What you offer entry shows
const RECENT: usize = 5;

/// `knowme://changed`: the to-do or the answers changed
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KnowMeChanged {}

impl NandiEvent for KnowMeChanged {
    const NAME: &'static str = "knowme://changed";
}

/// The to-do and what the answers changed lately
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KnowMeStatus {
    /// The to-do is in the Inbox
    pub todo_open: bool,
    /// The person answered before: the to-do asks if it's still right
    pub recheck: bool,
    /// Epoch ms the to-do last came
    #[specta(type = Option<f64>)]
    pub shown_at: Option<i64>,
    /// Epoch ms a change was last confirmed
    #[specta(type = Option<f64>)]
    pub answered_at: Option<i64>,
    /// The chat (`inbox:knowMe:you`)
    pub thread_id: String,
    /// Groups I'm in, for sharing one personal item
    pub groups: Vec<GroupRef>,
    /// The latest confirmed changes, newest first
    pub recent: Vec<DecisionRecord>,
}

/// What the to-do's buttons do
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum KnowMeTodoAction {
    /// Back in a week
    Later,
    /// Back in a month at the soonest
    Skip,
}

async fn load(state: &AppState) -> Result<Cadence, String> {
    let value = state
        .db()
        .call(|conn| crate::db::get_setting(conn, cadence::KEY))
        .await?;
    Ok(value
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

async fn save(state: &AppState, before: Cadence, after: Cadence) -> Result<(), String> {
    if before == after {
        return Ok(());
    }
    let value = serde_json::to_value(after).map_err(|e| e.to_string())?;
    state
        .db()
        .call(move |conn| crate::db::set_setting(conn, cadence::KEY, &value))
        .await?;
    events::emit(state.bus(), &KnowMeChanged {});
    Ok(())
}

async fn update(
    state: &AppState,
    change: impl FnOnce(Cadence) -> Cadence,
) -> Result<Cadence, String> {
    let before = load(state).await?;
    let after = change(before);
    save(state, before, after).await?;
    Ok(after)
}

/// The first to-do, once onboarding is done (again: nothing new)
pub async fn start(state: &AppState) -> Result<(), String> {
    let now = crate::db::now_ms();
    update(state, |c| c.start(now)).await.map(|_| ())
}

async fn onboarded(state: &AppState) -> bool {
    use crate::services::auth::{OnboardingStep, ONBOARDING_STEP_KEY};
    state
        .db()
        .call(|conn| crate::db::get_setting(conn, ONBOARDING_STEP_KEY))
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<OnboardingStep>(v).ok())
        == Some(OnboardingStep::Done)
}

/// The to-do (brought back when its time came) and the recent answers
pub async fn status(state: &AppState) -> Result<KnowMeStatus, String> {
    let now = crate::db::now_ms();
    let started = onboarded(state).await;
    let cadence = update(state, |c| {
        let c = if started { c.start(now) } else { c };
        c.tick(now)
    })
    .await?;
    Ok(KnowMeStatus {
        todo_open: cadence.open,
        recheck: cadence.answered_at.is_some(),
        shown_at: cadence.shown_at,
        answered_at: cadence.answered_at,
        thread_id: THREAD_ID.to_string(),
        groups: my_groups(state),
        recent: recent(state),
    })
}

/// Later or Skip
pub async fn todo(state: &AppState, action: KnowMeTodoAction) -> Result<KnowMeStatus, String> {
    let now = crate::db::now_ms();
    update(state, |c| match action {
        KnowMeTodoAction::Later => c.later(now),
        KnowMeTodoAction::Skip => c.skip(now),
    })
    .await?;
    status(state).await
}

/// The latest know-me decisions, across months
fn recent(state: &AppState) -> Vec<DecisionRecord> {
    let decisions = &state.store().repos().decisions;
    let filter = DecisionFilter {
        kind: Some(KIND.to_string()),
        ..DecisionFilter::default()
    };
    let mut out = Vec::new();
    for month in decisions.months().unwrap_or_default() {
        if let Ok(entries) = decisions.entries(&month) {
            out.extend(select(&entries, &filter));
        }
        if out.len() >= RECENT {
            break;
        }
    }
    out.truncate(RECENT);
    out
}

/// The groups this Nandi is in (its group copies that list it)
pub fn my_groups(state: &AppState) -> Vec<GroupRef> {
    let me = crate::services::network::local_instance_id(state.db()).ok();
    let (copies, _) = state.store().repos().groups.load_all().unwrap_or_default();
    copies
        .into_iter()
        .filter(|c| {
            me.as_ref()
                .is_none_or(|me| c.members.iter().any(|m| &m.instance_id == me))
        })
        .map(|c| GroupRef {
            name: if c.name.trim().is_empty() {
                c.group_id.clone()
            } else {
                c.name
            },
            id: c.group_id,
        })
        .collect()
}

/// The capabilities agents here serve
pub fn served(state: &AppState) -> Vec<String> {
    let Some(index) = state.capability_index() else {
        return Vec::new();
    };
    let index = index.read().unwrap_or_else(|e| e.into_inner());
    index
        .list_capabilities()
        .into_iter()
        .filter(|c| !index.find_local(c).is_empty())
        .collect()
}

/// The person confirms a proposal on its card, maybe edited: the edited
/// proposal replaces the call's arguments, then the call goes ahead like
/// any approval (the dispatcher reads the confirmed arguments)
pub async fn confirm(
    state: &AppState,
    approval_id: String,
    proposal: KnowMeProposal,
) -> Result<(), String> {
    let id = approval_id.clone();
    let approval = state
        .db()
        .call(move |conn| crate::db::get_approval(conn, &id))
        .await?
        .ok_or("That proposal wasn't found")?;
    if approval.tool != crate::services::inbox::tool::PROPOSE || approval.agent_id != CHIEF_OF_STAFF
    {
        return Err("That isn't a know-me proposal".to_string());
    }
    if approval.status != ApprovalStatus::Pending {
        return Err("This proposal was already decided".to_string());
    }
    // Checked now, so a mistake shows on the card, not in the chat
    let checked = proposal.checked(&my_groups(state), &served(state))?;
    let args = serde_json::to_value(&checked).map_err(|e| e.to_string())?;
    let row = approval.clone();
    state
        .db()
        .call(move |conn| crate::db::set_approval_args(conn, &row, &args))
        .await?;
    crate::services::chat::approve_action(state, None, approval_id, ApprovalDecision::Approved)
        .await
}

/// A confirmed proposal: NANDI.md and My notes written through their own
/// paths, the decision recorded, the to-do closed. Returns what changed.
pub async fn apply(state: &AppState, args: &Value) -> Result<String, String> {
    let proposal: KnowMeProposal = serde_json::from_value(args.clone())
        .map_err(|e| format!("The proposal doesn't read: {e}"))?;
    let groups = my_groups(state);
    let proposal = proposal.checked(&groups, &served(state))?;

    if proposal.touches_manifest() {
        let gateway = state.gateway().ok_or("The gateway is unavailable")?;
        let mut manifest = gateway.manifest()?;
        apply_to_manifest(&mut manifest, &proposal);
        gateway.save_manifest(&manifest)?;
    }

    let lines = proposal.note_lines(&groups);
    if !lines.is_empty() {
        let (folder, _) = crate::services::notes::folder(state).await?;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        for note in lines {
            let existing = folder.read(&note.note).unwrap_or_default();
            let text = add_lines(&existing, &note.title, &note.section, &note.lines, &today);
            crate::services::notes::save_note(state, note.note.clone(), text).await?;
        }
    }

    let now = crate::db::now_ms();
    update(state, |c| c.answered(now)).await?;
    // The decisions file is in the repo: a fixed subject and counts, so a
    // personal answer never lands there (`KnowMeProposal::record`)
    let changed = proposal.record(&groups);
    audit::record(
        state,
        Decided::new(KIND, "you", "approved")
            .subject("Getting to know you")
            .note(Some(&changed)),
    );
    events::emit(state.bus(), &KnowMeChanged {});
    Ok(changed)
}

#[cfg(test)]
mod tests;
