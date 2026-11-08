//! "Since you were away": what the Chief of Staff did with what arrived
//! (work it started, news it added, work that finished) since the user last
//! looked, and the work waiting on them, which stays until it's done.
//! What arrived from outside and awaits the user's approval comes with
//! it (`queue`).
//!
//! [`build`] is pure; [`get`] and [`mark_seen`] do the I/O.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;

use super::local::{self, LocalRequest, RequestSource, RequestUpdate};
use super::recurrence::{self, WorkflowProposal};
use super::RequestStatus;
use crate::db;
use crate::services::ask::PlanStepKind;
use crate::services::intake::{self, QueuedWork};
use crate::services::AppState;

/// When the user last marked it seen, epoch ms (setting)
const SEEN_KEY: &str = "chief_of_staff.digest_seen_at";
/// Longest list
const MAX_ITEMS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DigestKind {
    /// Waiting on the user (shown until it's done)
    NeedsYou,
    Started,
    /// News added to work in progress
    Updated,
    /// Done, failed or cancelled
    Finished,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DigestItem {
    pub request_id: String,
    pub title: String,
    pub source: RequestSource,
    pub kind: DigestKind,
    pub status: RequestStatus,
    /// The news, for `updated`
    pub text: Option<String>,
    /// Epoch ms
    pub at: f64,
    /// Its Chief of Staff thread, when it came through intake
    pub thread_id: Option<String>,
    /// The step waiting on the user (a manual step or a person), for
    /// "Mark done" (`complete_request_step`)
    pub waiting_step: Option<WaitingStep>,
    /// Finished work that recurs: the Chief of Staff offers **Save as
    /// workflow** (`recurrence`)
    pub workflow_proposal: Option<WorkflowProposal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WaitingStep {
    pub position: u32,
    pub label: String,
}

/// The first step that waits on the user. Pure.
pub fn waiting_step(request: &LocalRequest) -> Option<WaitingStep> {
    request
        .steps
        .iter()
        .find(|s| {
            s.status == super::StepStatus::Waiting
                && matches!(s.kind, PlanStepKind::Manual | PlanStepKind::Human)
        })
        .map(|s| WaitingStep {
            position: s.position,
            label: s.label.clone(),
        })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChiefOfStaffDigest {
    /// When the user last marked it seen, epoch ms (`None`: never)
    pub since: Option<f64>,
    /// Waiting on the user first, then newest first
    pub items: Vec<DigestItem>,
    /// From outside, screened, waiting for the user's approval before it
    /// starts or reaches its Request (`services::intake`), oldest first
    pub queue: Vec<QueuedWork>,
}

fn ended(status: RequestStatus) -> bool {
    matches!(
        status,
        RequestStatus::Done | RequestStatus::Failed | RequestStatus::Cancelled
    )
}

/// The digest from the Requests open or changed since `seen` and the news
/// added since. Work the user started themselves is left out. Pure.
pub fn build(requests: &[LocalRequest], updates: &[RequestUpdate], seen: i64) -> Vec<DigestItem> {
    let item = |r: &LocalRequest, kind: DigestKind, at: i64, text: Option<String>| DigestItem {
        request_id: r.id.clone(),
        title: r.title.clone(),
        source: r.source,
        kind,
        status: r.status,
        text,
        at: at as f64,
        thread_id: None,
        waiting_step: (kind == DigestKind::NeedsYou)
            .then(|| waiting_step(r))
            .flatten(),
        workflow_proposal: None,
    };
    let mut items = Vec::new();
    let mut by_id = HashMap::new();
    for r in requests.iter().filter(|r| r.source != RequestSource::Me) {
        by_id.insert(r.id.as_str(), r);
        if r.status == RequestStatus::Waiting {
            items.push(item(r, DigestKind::NeedsYou, r.updated_at, None));
        } else if ended(r.status) && r.ended_at.is_some_and(|at| at > seen) {
            items.push(item(r, DigestKind::Finished, r.ended_at.unwrap_or(0), None));
        } else if r.created_at > seen {
            items.push(item(r, DigestKind::Started, r.created_at, None));
        }
    }
    for u in updates.iter().filter(|u| u.at > seen) {
        if let Some(r) = by_id.get(u.request_id.as_str()) {
            items.push(item(r, DigestKind::Updated, u.at, Some(u.text.clone())));
        }
    }
    items.sort_by(|a, b| {
        (b.kind == DigestKind::NeedsYou)
            .cmp(&(a.kind == DigestKind::NeedsYou))
            .then(b.at.total_cmp(&a.at))
    });
    items.truncate(MAX_ITEMS);
    items
}

pub async fn get(state: &AppState) -> Result<ChiefOfStaffDigest, String> {
    state
        .db()
        .call(|conn| {
            let seen = db::get_setting(conn, SEEN_KEY)?.and_then(|v| v.as_i64());
            let since = seen.unwrap_or(0);
            let requests = local::recent(conn, since)?;
            let updates = local::updates_since(conn, since)?;
            let mut items = build(&requests, &updates, since);
            for item in &mut items {
                item.thread_id = intake::thread::for_request(conn, &item.request_id)?;
                if item.kind == DigestKind::Finished {
                    if let Some(request) = requests.iter().find(|r| r.id == item.request_id) {
                        item.workflow_proposal = recurrence::for_request(conn, request)?;
                    }
                }
            }
            Ok(ChiefOfStaffDigest {
                since: seen.map(|at| at as f64),
                items,
                queue: intake::pending(conn)?,
            })
        })
        .await
}

/// The user has seen it: only what waits on them stays
pub async fn mark_seen(state: &AppState) -> Result<(), String> {
    state
        .db()
        .call(|conn| db::set_setting(conn, SEEN_KEY, &json!(db::now_ms())))
        .await?;
    crate::services::events::emit(state.bus(), &local::RequestUpdated { id: String::new() });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ask::ActionPlan;

    fn request(
        id: &str,
        source: RequestSource,
        status: RequestStatus,
        created: i64,
    ) -> LocalRequest {
        let mut r = local::new_request(id, id, &ActionPlan::default(), source, created);
        r.status = status;
        r.updated_at = created;
        if ended(status) {
            r.ended_at = Some(created + 10);
        }
        r
    }

    fn update(id: &str, at: i64) -> RequestUpdate {
        RequestUpdate {
            request_id: id.to_string(),
            at,
            source: RequestSource::Email,
            text: "The due date moved to Friday".to_string(),
        }
    }

    #[test]
    fn shows_what_happened_since_the_user_looked() {
        let requests = [
            request("new", RequestSource::Email, RequestStatus::Running, 200),
            request("old", RequestSource::Email, RequestStatus::Running, 50),
            request(
                "finished",
                RequestSource::Calendar,
                RequestStatus::Done,
                150,
            ),
            request("mine", RequestSource::Me, RequestStatus::Running, 200),
        ];
        let items = build(&requests, &[update("old", 300)], 100);
        let got: Vec<(&str, DigestKind)> = items
            .iter()
            .map(|i| (i.request_id.as_str(), i.kind))
            .collect();
        assert_eq!(
            got,
            vec![
                ("old", DigestKind::Updated),
                ("new", DigestKind::Started),
                ("finished", DigestKind::Finished),
            ]
        );
        assert_eq!(
            items[0].text.as_deref(),
            Some("The due date moved to Friday")
        );
    }

    #[test]
    fn work_waiting_on_the_user_stays_on_top_after_it_is_seen() {
        let requests = [
            request("bill", RequestSource::Email, RequestStatus::Waiting, 10),
            request(
                "post",
                RequestSource::ChiefOfStaff,
                RequestStatus::Running,
                500,
            ),
        ];
        let items = build(&requests, &[], 100);
        assert_eq!(items[0].request_id, "bill");
        assert_eq!(items[0].kind, DigestKind::NeedsYou);
        assert_eq!(items[1].kind, DigestKind::Started);
    }

    #[test]
    fn a_step_waiting_on_the_user_can_be_marked_done_from_home() {
        let plan = ActionPlan {
            steps: vec![crate::services::ask::PlanStep {
                kind: PlanStepKind::Manual,
                target: String::new(),
                description: "For you: Pay the power bill".to_string(),
            }],
            ..ActionPlan::default()
        };
        let mut r = local::new_request("bill", "Pay it", &plan, RequestSource::Email, 10);
        r.status = RequestStatus::Waiting;
        r.steps[0].status = super::super::StepStatus::Waiting;
        let items = build(&[r], &[], 100);
        assert_eq!(
            items[0].waiting_step,
            Some(WaitingStep {
                position: 0,
                label: "For you: Pay the power bill".to_string()
            })
        );
    }

    #[test]
    fn nothing_new_is_empty() {
        let requests = [request(
            "old",
            RequestSource::Email,
            RequestStatus::Done,
            10,
        )];
        assert!(build(&requests, &[update("old", 20)], 100).is_empty());
    }
}
