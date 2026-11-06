//! Reassignment: a provider hands a task to a teammate who serves the
//! same capability, and the requester never resubmits. The relay moves
//! it (relay design step 13).
//!
//! ```text
//! Alice (provider)              relay                          requester    Bob
//!  reassign_task ── Reassign ──▶ checks the chain
//!  (request resealed for Bob)    old attempt → reassigned
//!  row → Reassigned  ◀ ReassignResult    Deliver task ─────────────────────▶ runs it
//!                                Deliver event reassigned ──▶ row follows Bob
//! ```
//!
//! The request stays sealed end to end. Alice's relay client reseals the
//! request it opened for Bob (`nandi_seal::reseal`: the requester's
//! signature is kept) and the relay delivers it to Bob, who checks the
//! requester signed it. The task keeps its ID; the requester's row moves
//! to Bob (`task_state::hand_on`) with "Reassigned to Bob by Alice" in
//! its thread. When Alice no longer holds the opened request (her Nandi
//! restarted since it came), the relay asks the requester, its author, to
//! seal it again ([`reseal`]), and Bob gets it then.
//!
//! A manager's move (the relay's `POST /team/tasks/{id}/reassign`) comes
//! to the provider's Nandi as a reseal request, and it does the same
//! handoff. Nothing moves until a Nandi that can open the request reseals
//! it: there's no team key (decision 4, 2026-11-01).
//!
//! The relay keeps the attempt chain: a task never goes back to its
//! requester or to anyone who had it. The candidates offered here leave
//! out those this Nandi knows of.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;

use super::failover;
use super::task::{self, TaskRecord, TaskService, TaskState};
use super::task_state::{self, Change};
use crate::db::now_ms;
use crate::model::Timestamp;
use crate::services::domain::agent_card::OwnerRef;
use crate::services::domain::AgentCard;
use crate::services::events::{self, NandiEvent};
use crate::services::relay::protocol::ResealRequest;
use crate::services::relay::unavailable;
use crate::services::AppState;

/// `task://reassigned`
pub const TASK_REASSIGNED: &str = "task://reassigned";
/// `task_messages.kind` of the note on a reassigned attempt
pub const REASSIGNED_KIND: &str = "reassigned";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReassignRequest {
    pub task_id: String,
    pub new_provider_instance: String,
    pub reason: Option<String>,
}

/// An online teammate who could take a task
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReassignCandidate {
    pub instance_id: String,
    pub display_name: String,
    /// `<instance>/<agent>` card that serves the capability
    pub card_id: String,
}

/// Payload of `task://reassigned`, and a `reassigned_tasks` row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskReassigned {
    pub task_id: String,
    pub from_instance: String,
    pub to_instance: String,
    /// The new provider's display name, when known
    pub to_name: Option<String>,
    /// Display name of who reassigned it
    pub by: String,
    pub reason: Option<String>,
    /// We asked for the task (it moved away from its provider), not served
    /// it
    pub requester: bool,
    pub reassigned_at: Timestamp,
}

impl NandiEvent for TaskReassigned {
    const NAME: &'static str = TASK_REASSIGNED;
}

/// A move the relay made with one of our requests (`TaskEvent::Reassigned`)
pub(super) struct Move {
    pub from_instance: String,
    pub to_instance: String,
    pub to_card: String,
    pub to_name: Option<String>,
    pub by: String,
    pub reason: Option<String>,
}

/// States a task can be handed on from: it reached us and isn't over
fn reassignable(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Delivered
            | TaskState::Acknowledged
            | TaskState::Working
            | TaskState::InputRequired
    )
}

/// Who we are, for "Reassigned to Bob by Alice"
fn our_name(service: &TaskService) -> String {
    let OwnerRef {
        display_name,
        github_handle,
    } = service.deps.owner();
    [display_name, github_handle]
        .into_iter()
        .map(|name| name.trim().to_string())
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| service.instance_id().to_string())
}

fn card_name(card: &AgentCard) -> String {
    [&card.owner.display_name, &card.owner.github_handle]
        .into_iter()
        .map(|name| name.trim())
        .find(|name| !name.is_empty())
        .unwrap_or(&card.instance_id)
        .to_string()
}

// ---------------------------------------------------------------------------
// `reassigned_tasks`
// ---------------------------------------------------------------------------

async fn record(service: &TaskService, event: &TaskReassigned) -> Result<(), String> {
    let event = event.clone();
    service
        .db()
        .call(move |conn| {
            conn.execute(
                "INSERT INTO reassigned_tasks
                   (task_id, from_instance, to_instance, reason, reassigned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(task_id, to_instance) DO NOTHING",
                params![
                    event.task_id,
                    event.from_instance,
                    event.to_instance,
                    event.reason,
                    event.reassigned_at as i64,
                ],
            )?;
            Ok(())
        })
        .await
}

/// Every instance this Nandi knows a task has been with: both ends of
/// each reassignment it saw (the relay keeps the whole chain)
async fn visited(service: &TaskService, task: &TaskRecord) -> Result<HashSet<String>, String> {
    let id = task.id.clone();
    let hops: Vec<(String, String)> = service
        .db()
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT from_instance, to_instance FROM reassigned_tasks WHERE task_id = ?1",
            )?;
            let rows = stmt.query_map([&id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<(String, String)>>>()
        })
        .await?;
    let mut instances = HashSet::from([task.provider_instance.clone()]);
    for (from, to) in hops {
        instances.insert(from);
        instances.insert(to);
    }
    Ok(instances)
}

// ---------------------------------------------------------------------------
// Provider side
// ---------------------------------------------------------------------------

/// Online instances serving `task`'s capability that could take it: not
/// us, not the requester, not anyone who already had it. One per instance.
async fn candidates(
    service: &TaskService,
    task: &TaskRecord,
) -> Result<Vec<(ReassignCandidate, AgentCard)>, String> {
    let relay = unavailable::connected(service.relay())?;
    let mut excluded = visited(service, task).await?;
    excluded.insert(service.instance_id().to_string());
    excluded.insert(task.requester_instance.clone());
    let cards = relay
        .query_capability(&task.capability)
        .await
        .unwrap_or_default();
    let cards = failover::without_untrusted(service.db(), service.instance_id(), cards);
    let mut seen = HashSet::new();
    Ok(cards
        .into_iter()
        .filter(|card| card.accepts_delegation && !excluded.contains(&card.instance_id))
        .filter(|card| seen.insert(card.instance_id.clone()))
        .map(|card| {
            let candidate = ReassignCandidate {
                instance_id: card.instance_id.clone(),
                display_name: card_name(&card),
                card_id: card.id.clone(),
            };
            (candidate, card)
        })
        .collect())
}

/// The provider task `task_id`, if it's one we could still hand on
async fn ours(service: &TaskService, task_id: &str) -> Result<TaskRecord, String> {
    let task = service.get(task_id).await?;
    if task.provider_instance != service.instance_id() || task.is_requester(service.instance_id()) {
        return Err("Only the provider handling a task can reassign it".to_string());
    }
    if !reassignable(task.state) {
        return Err(format!(
            "This task is {}, so it can't be reassigned",
            task.state.as_str()
        ));
    }
    Ok(task)
}

/// Hands a task we're serving to `request.new_provider_instance` through
/// the relay: once it moved it, our row goes `Reassigned` and our run of
/// it stops. Returns our row.
pub async fn reassign(
    service: &TaskService,
    request: ReassignRequest,
) -> Result<TaskRecord, String> {
    let task = ours(service, &request.task_id).await?;
    let to = request.new_provider_instance.trim();
    if to.is_empty() {
        return Err("Choose who to hand it to".to_string());
    }
    if to == service.instance_id() {
        return Err("This task is already yours".to_string());
    }
    if to == task.requester_instance {
        return Err("A task can't go back to whoever asked for it".to_string());
    }
    if visited(service, &task).await?.contains(to) {
        return Err(format!("{to} already had this task"));
    }
    let Some((candidate, _)) = candidates(service, &task)
        .await?
        .into_iter()
        .find(|(c, _)| c.instance_id == to)
    else {
        return Err(format!(
            "{to} isn't online or doesn't serve '{}'",
            task.capability
        ));
    };
    let reason = request
        .reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    hand_over(
        service,
        &task,
        (&candidate.card_id, Some(candidate.display_name)),
        &our_name(service),
        reason,
    )
    .await
}

/// Our provider `task` goes to `card` through the relay (the request
/// resealed), as `by` asked; then our row ends `Reassigned`
async fn hand_over(
    service: &TaskService,
    task: &TaskRecord,
    (card, name): (&str, Option<String>),
    by: &str,
    reason: Option<String>,
) -> Result<TaskRecord, String> {
    let relay = unavailable::connected(service.relay())?;
    // The relay first: if it won't move it, nothing changes here
    relay
        .reassign(&task.id, &task.requester_instance, card, by, reason.clone())
        .await?;
    let to_instance = card.split('/').next().unwrap_or_default().to_string();
    let to_name = name.unwrap_or_else(|| to_instance.clone());
    task_state::advance(
        service.db(),
        service.bus(),
        &task.id,
        TaskState::Reassigned,
        Change::reason(format!("Reassigned to {to_name} by {by}")),
    )
    .await?;
    service.stop_run(&task.id).await;
    let event = TaskReassigned {
        task_id: task.id.clone(),
        from_instance: service.instance_id().to_string(),
        to_instance,
        to_name: Some(to_name),
        by: by.to_string(),
        reason,
        requester: false,
        reassigned_at: now_ms() as Timestamp,
    };
    record(service, &event).await?;
    events::emit(service.bus(), &event);
    log::info!("Task {} reassigned to {}", task.id, event.to_instance);
    service.get(&task.id).await
}

/// The relay asks us to reseal a task for a new provider. Our provider
/// row: a manager moved it, so we hand it over as they asked. Our
/// requester row: its provider no longer held the request, so we seal our
/// own again.
pub(super) async fn reseal(
    service: &Arc<TaskService>,
    request: ResealRequest,
) -> Result<(), String> {
    let Some(task) = task::get_row(service.db(), &request.task_id).await? else {
        return Err(format!(
            "Task {}: asked to reseal a task we don't have",
            request.task_id
        ));
    };
    if task.state.is_terminal() {
        log::info!("Task {}: asked to reseal it, but it's over", task.id);
        return Ok(());
    }
    if task.is_requester(service.instance_id()) {
        let Some(original) = task::request_of(service.db(), &task.id).await? else {
            return Err(format!(
                "Task {}: the request to reseal is missing",
                task.id
            ));
        };
        let relay = unavailable::connected(service.relay())?;
        relay
            .reseal_as_author(&original, &request.target, &request.by)
            .await?;
        log::info!("Task {}: resealed for {}", task.id, request.target);
        return Ok(());
    }
    hand_over(service, &task, (&request.target, None), &request.by, None).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Requester side
// ---------------------------------------------------------------------------

/// The relay moved our request `task` to another provider: the row
/// follows it, and the thread says who moved it
pub(super) async fn moved(
    service: &Arc<TaskService>,
    task: &TaskRecord,
    change: Move,
) -> Result<(), String> {
    let (db, bus) = (service.db(), service.bus());
    let to_name = change
        .to_name
        .clone()
        .unwrap_or_else(|| change.to_instance.clone());
    let note = format!("Reassigned to {to_name} by {}", change.by);
    let reason = match &change.reason {
        Some(reason) => format!("{note}: {reason}"),
        None => note,
    };
    task_state::hand_on(db, bus, &task.id, &change.to_card, Change::reason(reason)).await?;
    let event = TaskReassigned {
        task_id: task.id.clone(),
        from_instance: change.from_instance,
        to_instance: change.to_instance,
        to_name: change.to_name,
        by: change.by,
        reason: change.reason,
        requester: true,
        reassigned_at: now_ms() as Timestamp,
    };
    record(service, &event).await?;
    let payload = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
    task::add_message(db, &task.id, REASSIGNED_KIND, payload).await?;
    events::emit(bus, &event);
    log::info!("Task {} moved to {}", task.id, event.to_instance);
    Ok(())
}

// ---------------------------------------------------------------------------
// Service API (shared by the Tauri commands and HTTP routes)
// ---------------------------------------------------------------------------

fn service(state: &AppState) -> Result<&Arc<TaskService>, String> {
    state
        .tasks()
        .ok_or_else(|| "Task tracking is not available".to_string())
}

/// Hands a task we're serving to `new_provider_instance`
pub async fn reassign_task(
    state: &AppState,
    request: ReassignRequest,
) -> Result<TaskRecord, String> {
    reassign(service(state)?, request).await
}

/// Online instances (not us, not the requester, not anyone who already
/// had the task) serving the task's capability
pub async fn reassign_candidates(
    state: &AppState,
    task_id: &str,
) -> Result<Vec<ReassignCandidate>, String> {
    reassign_candidates_for(service(state)?, task_id).await
}

pub async fn reassign_candidates_for(
    service: &TaskService,
    task_id: &str,
) -> Result<Vec<ReassignCandidate>, String> {
    let task = ours(service, task_id).await?;
    Ok(candidates(service, &task)
        .await?
        .into_iter()
        .map(|(candidate, _)| candidate)
        .collect())
}
