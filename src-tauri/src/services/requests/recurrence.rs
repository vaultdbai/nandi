//! Recurring work becomes a workflow proposal (task-x step 4): when a
//! Request finishes well and the same work finished well before, the Chief
//! of Staff offers to save it as a workflow. It is never saved silently:
//! **Save as workflow** drafts it (`local::draft_workflow_from_request`)
//! and the user checks and saves the file.
//!
//! The rule ([`proposal`], pure): a Request that ended `done`, has a step
//! someone other than the user does, and isn't already one workflow,
//! recurs when an earlier `done` Request has
//!
//! | Reason     | Match                                                          |
//! | ---------- | -------------------------------------------------------------- |
//! | `sameWork` | the same title, ignoring case, numbers, dates and month names |
//! | `samePlan` | the same steps (kind and who, in order), two steps or more     |
//!
//! The proposal shows on the Request in its intake thread (a note there,
//! and the button on its card) and on Home's "Since you were away".
//!
//! The same signal proposes a **playbook** (step 5): when the Chief of
//! Staff does a step itself and did the same work itself before, its step's
//! prompt carries [`playbook_hint`]. Its skill.md and `build-a-workflow`
//! playbook say the rest: one line in `suggestions.md`, never repeated, and
//! `skill_propose` only once the user says yes.

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::local::{self, LocalRequest};
use super::RequestStatus;
use crate::services::ask::PlanStepKind;
use crate::services::intake;
use crate::services::AppState;

/// Earlier finished Requests compared, newest first
const MAX_EARLIER: usize = 200;

const MONTHS: [&str; 24] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "jan",
    "feb",
    "mar",
    "apr",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
];

/// Why the work counts as recurring
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RecurrenceReason {
    /// The same work (title) finished before
    SameWork,
    /// The same plan succeeded before
    SamePlan,
}

/// The Chief of Staff's offer to save a finished Request as a workflow
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowProposal {
    pub request_id: String,
    pub reason: RecurrenceReason,
    /// The earlier finished Request it repeats
    pub like_request_id: String,
    pub like_title: String,
}

/// The title's words without numbers, dates or month names. Pure.
pub fn work_key(title: &str) -> Vec<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter(|word| !word.chars().any(|c| c.is_ascii_digit()))
        .filter(|word| !MONTHS.contains(word))
        .map(str::to_string)
        .collect()
}

fn plan_key(request: &LocalRequest) -> Vec<(PlanStepKind, &str)> {
    request
        .steps
        .iter()
        .map(|s| (s.kind, s.target.as_str()))
        .collect()
}

/// Whether a Request could become a workflow: it ended well, someone other
/// than the user does a step, and it isn't already one workflow. Pure.
pub fn worth_a_workflow(request: &LocalRequest) -> bool {
    let already_one = request.steps.len() == 1 && request.steps[0].kind == PlanStepKind::Workflow;
    request.status == RequestStatus::Done
        && !already_one
        && request
            .steps
            .iter()
            .any(|s| !matches!(s.kind, PlanStepKind::Manual | PlanStepKind::Human))
}

/// The proposal for `request`, if an earlier finished Request matches it
/// (newest first, see the module docs). Pure.
pub fn proposal(request: &LocalRequest, earlier: &[LocalRequest]) -> Option<WorkflowProposal> {
    if !worth_a_workflow(request) {
        return None;
    }
    let work = work_key(&request.title);
    let plan = plan_key(request);
    earlier
        .iter()
        .filter(|e| e.id != request.id && e.created_at < request.created_at)
        .filter(|e| e.status == RequestStatus::Done)
        .find_map(|e| {
            let reason = if !work.is_empty() && work_key(&e.title) == work {
                RecurrenceReason::SameWork
            } else if plan.len() >= 2 && plan_key(e) == plan {
                RecurrenceReason::SamePlan
            } else {
                return None;
            };
            Some(WorkflowProposal {
                request_id: request.id.clone(),
                reason,
                like_request_id: e.id.clone(),
                like_title: e.title.clone(),
            })
        })
}

/// The Chief of Staff's note for the proposal. Pure.
pub fn note(proposal: &WorkflowProposal, title: &str) -> String {
    let why = match proposal.reason {
        RecurrenceReason::SameWork => "the same work",
        RecurrenceReason::SamePlan => "done the same way",
    };
    format!(
        "\"{title}\" went well, and it's {why} as \"{}\" before. Save it as a workflow, so it \
runs the same steps next time? Press **Save as workflow** on it: you check the draft before \
anything is saved, and your parts (signing in, paying) stay steps for you.",
        proposal.like_title
    )
}

/// Whether two Requests are the same work: the title (see [`work_key`])
/// or the same steps, two or more. Pure.
fn same_work(a: &LocalRequest, b: &LocalRequest) -> bool {
    let work = work_key(&a.title);
    let plan = plan_key(a);
    (!work.is_empty() && work_key(&b.title) == work) || (plan.len() >= 2 && plan_key(b) == plan)
}

fn has_self_step(request: &LocalRequest) -> bool {
    request
        .steps
        .iter()
        .any(|s| super::self_step::is_self_step(s.kind, &s.target))
}

/// The self-step prompt's hint to propose a playbook, when `request` has a
/// step the Chief of Staff does itself and earlier `done` Requests (newest
/// first) were the same work, done by it too. Pure.
pub fn playbook_hint(request: &LocalRequest, earlier: &[LocalRequest]) -> Option<String> {
    if !has_self_step(request) {
        return None;
    }
    let like: Vec<&LocalRequest> = earlier
        .iter()
        .filter(|e| e.id != request.id && e.created_at < request.created_at)
        .filter(|e| e.status == RequestStatus::Done && has_self_step(e))
        .filter(|e| same_work(request, e))
        .collect();
    let first = like.first()?;
    let count = like.len() + 1;
    Some(format!(
        "

You did this kind of work yourself before (\"{}\"; {count} similar tasks with this one). Once your step is done, if none of your playbooks covers it and `my-notes/suggestions.md` doesn't have it yet (open, done or dismissed), add one line there: `- [ ] Add a playbook \"<name>\" (from {count} similar tasks)`. Draft it with `skill_propose` only when the user ticks it or says yes.
",
        first.title
    ))
}

/// [`playbook_hint`] for a stored Request
pub fn playbook_hint_for(conn: &Connection, request: &LocalRequest) -> Option<String> {
    if !has_self_step(request) {
        return None;
    }
    playbook_hint(request, &earlier_done(conn, request).ok()?)
}

/// Requests that ended `done` before `request` started, newest first
fn earlier_done(conn: &Connection, request: &LocalRequest) -> rusqlite::Result<Vec<LocalRequest>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM requests WHERE status = 'done' AND created_at < ?1 AND id <> ?2
          ORDER BY created_at DESC LIMIT ?3",
    )?;
    let ids = stmt
        .query_map(
            rusqlite::params![request.created_at, request.id, MAX_EARLIER as i64],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(earlier) = local::load(conn, &id)? {
            out.push(earlier);
        }
    }
    Ok(out)
}

/// The proposal for a stored Request, if it recurs
pub fn for_request(
    conn: &Connection,
    request: &LocalRequest,
) -> rusqlite::Result<Option<WorkflowProposal>> {
    if !worth_a_workflow(request) {
        return Ok(None);
    }
    Ok(proposal(request, &earlier_done(conn, request)?))
}

/// The proposals for these Requests, skipping any that can't be read
pub fn for_ids(conn: &Connection, ids: &[String]) -> Vec<WorkflowProposal> {
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| local::load(conn, id).ok().flatten())
        .filter_map(|request| for_request(conn, &request).ok().flatten())
        .collect()
}

/// A Request just finished well: when it recurs and came through intake,
/// the Chief of Staff proposes saving it in its thread. Best effort.
pub async fn on_done(state: &AppState, request: &LocalRequest) {
    let id = request.id.clone();
    let request = request.clone();
    let noted = state
        .db()
        .call(move |conn| {
            let Some(thread) = intake::thread::for_request(conn, &request.id)? else {
                return Ok(false);
            };
            let Some(proposal) = for_request(conn, &request)? else {
                return Ok(false);
            };
            crate::services::inbox::thread::note(conn, &thread, &note(&proposal, &request.title))?;
            Ok(true)
        })
        .await;
    match noted {
        Ok(true) => crate::services::events::emit(state.bus(), &local::RequestUpdated { id }),
        Ok(false) => {}
        Err(e) => log::warn!("Workflow proposal: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ask::{ActionPlan, PlanStep};
    use crate::services::requests::local::RequestSource;

    fn plan(steps: &[(PlanStepKind, &str)]) -> ActionPlan {
        ActionPlan {
            steps: steps
                .iter()
                .map(|(kind, target)| PlanStep {
                    kind: *kind,
                    target: (*target).to_string(),
                    description: format!("Do {target}"),
                })
                .collect(),
            ..ActionPlan::default()
        }
    }

    fn done(id: &str, title: &str, steps: &[(PlanStepKind, &str)], at: i64) -> LocalRequest {
        let mut r = local::new_request(id, title, &plan(steps), RequestSource::Email, at);
        r.status = RequestStatus::Done;
        r
    }

    const BILL: &[(PlanStepKind, &str)] = &[(PlanStepKind::LocalAgent, "chief-of-staff")];

    #[test]
    fn the_work_key_ignores_numbers_dates_and_months() {
        assert_eq!(
            work_key("Pay the Acme Energy bill for October ($84, 2026-10-05)"),
            vec!["pay", "the", "acme", "energy", "bill", "for"]
        );
        assert_eq!(
            work_key("Pay the Acme Energy bill for November"),
            work_key("pay the acme energy bill for Oct")
        );
    }

    #[test]
    fn the_same_work_finished_before_recurs() {
        let earlier = done("r1", "Pay the Acme bill for October", BILL, 10);
        let now = done("r2", "Pay the Acme bill for November", BILL, 20);
        let found = proposal(&now, std::slice::from_ref(&earlier)).unwrap();
        assert_eq!(found.reason, RecurrenceReason::SameWork);
        assert_eq!(found.like_request_id, "r1");
        assert!(note(&found, &now.title).contains("Save as workflow"));
    }

    #[test]
    fn the_same_plan_succeeding_twice_recurs() {
        let steps = &[
            (PlanStepKind::LocalAgent, "researcher"),
            (PlanStepKind::LocalAgent, "writer"),
        ];
        let earlier = done("r1", "Brief on Acme", steps, 10);
        let now = done("r2", "Brief on Globex", steps, 20);
        assert_eq!(
            proposal(&now, &[earlier]).map(|p| p.reason),
            Some(RecurrenceReason::SamePlan)
        );
        // One step alone is too common to count
        let one = done(
            "r3",
            "Write a poem",
            &[(PlanStepKind::LocalAgent, "writer")],
            10,
        );
        let other = done(
            "r4",
            "Write a toast",
            &[(PlanStepKind::LocalAgent, "writer")],
            20,
        );
        assert_eq!(proposal(&other, &[one]), None);
    }

    #[test]
    fn work_the_chief_of_staff_repeats_itself_hints_a_playbook() {
        let a = done("r1", "Pay the Acme bill for October", BILL, 10);
        let b = done("r2", "Pay the Acme bill for November", BILL, 20);
        let mut now = done("r3", "Pay the Acme bill for December", BILL, 30);
        now.status = RequestStatus::Running;
        let hint = playbook_hint(&now, &[b.clone(), a.clone()]).unwrap();
        assert!(hint.contains("\"Pay the Acme bill for November\""));
        assert!(hint.contains("(from 3 similar tasks)"));
        assert!(hint.contains("`- [ ] Add a playbook"));
        assert!(hint.contains("`skill_propose`") && hint.contains("dismissed"));
        // The first time: nothing to go on
        assert_eq!(playbook_hint(&now, &[]), None);
        // Work it didn't do itself, now or before
        let agent = &[(PlanStepKind::LocalAgent, "writer")];
        let theirs = done("r4", "Pay the Acme bill", agent, 40);
        assert_eq!(playbook_hint(&theirs, std::slice::from_ref(&a)), None);
        let mine = done("r5", "Pay the Acme bill", BILL, 50);
        assert_eq!(playbook_hint(&mine, &[theirs]), None);
        // Different work, or failed before
        assert_eq!(
            playbook_hint(&now, &[done("r6", "Book a dentist", BILL, 5)]),
            None
        );
        let mut failed = a;
        failed.status = RequestStatus::Failed;
        assert_eq!(playbook_hint(&now, &[failed]), None);
    }

    #[test]
    fn only_work_that_went_well_and_could_run_alone_is_proposed() {
        let earlier = done("r1", "Pay the Acme bill", BILL, 10);
        // Failed now, or failed before
        let mut failed = done("r2", "Pay the Acme bill", BILL, 20);
        failed.status = RequestStatus::Failed;
        assert_eq!(proposal(&failed, std::slice::from_ref(&earlier)), None);
        let mut failed_before = earlier.clone();
        failed_before.status = RequestStatus::Failed;
        let now = done("r3", "Pay the Acme bill", BILL, 30);
        assert_eq!(proposal(&now, &[failed_before]), None);
        // Only the user's own steps: nothing to automate
        let manual = &[(PlanStepKind::Manual, "")];
        let a = done("r4", "Water the plants", manual, 10);
        let b = done("r5", "Water the plants", manual, 20);
        assert_eq!(proposal(&b, &[a]), None);
        // Already a workflow
        let wf = &[(PlanStepKind::Workflow, "pay-acme")];
        let a = done("r6", "Pay the Acme bill", wf, 10);
        let b = done("r7", "Pay the Acme bill", wf, 20);
        assert_eq!(proposal(&b, &[a]), None);
        // Later work doesn't count as before
        let later = done("r8", "Pay the Acme bill", BILL, 99);
        assert_eq!(proposal(&now, &[later]), None);
    }
}
