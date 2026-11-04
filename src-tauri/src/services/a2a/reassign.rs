//! Supervisor reassignment (wave EF, Track C): a provider with too much
//! queued hands a task to a teammate who serves the same capability, and
//! the requester never resubmits.
//!
//! ```text
//! Alice (provider)            requester                   Bob
//!  reassign_task ──ReassignTask──▶ accept
//!  row → Reassigned                old attempt → Reassigned
//!                                  new attempt (source_kind reassign)
//!                                     ──────── delegate_task ────────▶ Delivered, acks
//! ```
//!
//! The requester's instance makes the new attempt, so results, trust,
//! timeouts, and failover work as for any other attempt, and a delegation
//! still waiting follows it (`TaskService::follow`). A task never goes to
//! an instance that already had it: the provider checks what it knows,
//! and the requester checks the whole attempt chain; a reassignment that
//! would loop fails the attempt instead, and normal failover picks
//! someone who hasn't had it.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;

use super::delegator::instance_of;
use super::failover;
use super::task::{self, TaskRecord, TaskService, TaskSourceKind, TaskState};
use super::task_state::{self, Change};
use super::Task;
use crate::db::now_ms;
use crate::model::Timestamp;
use crate::services::domain::agent_card::OwnerRef;
use crate::services::domain::AgentCard;
use crate::services::events::{self, NandiEvent};
use crate::services::relay::protocol::PeerMessage;
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
    /// The requester's new attempt, once created (requester side)
    pub new_task_id: Option<String>,
    pub reassigned_at: Timestamp,
}

impl NandiEvent for TaskReassigned {
    const NAME: &'static str = TASK_REASSIGNED;
}

/// What the provider handed over (a `ReassignTask` peer message)
pub(super) struct Handoff {
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
                   (task_id, from_instance, to_instance, reason, new_task_id, reassigned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(task_id, to_instance) DO UPDATE SET
                   new_task_id = COALESCE(excluded.new_task_id, new_task_id)",
                params![
                    event.task_id,
                    event.from_instance,
                    event.to_instance,
                    event.reason,
                    event.new_task_id,
                    event.reassigned_at as i64,
                ],
            )?;
            Ok(())
        })
        .await
}

/// Every instance a task (and the attempts before it) has been with:
/// providers of the attempt chain, and both ends of each reassignment
async fn visited(service: &TaskService, task: &TaskRecord) -> Result<HashSet<String>, String> {
    let mut chain = Vec::new();
    let mut at = Some(task.clone());
    while let Some(attempt) = at {
        at = match &attempt.previous_attempt_id {
            Some(previous) => task::get_row(service.db(), previous).await?,
            None => None,
        };
        chain.push(attempt);
    }
    let ids: Vec<String> = chain.iter().map(|a| a.id.clone()).collect();
    let mut instances: HashSet<String> = chain.into_iter().map(|a| a.provider_instance).collect();
    let hops: Vec<(String, String)> = service
        .db()
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT from_instance, to_instance FROM reassigned_tasks WHERE task_id = ?1",
            )?;
            let mut hops = Vec::new();
            for id in &ids {
                let rows = stmt.query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?;
                hops.extend(rows.collect::<rusqlite::Result<Vec<(String, String)>>>()?);
            }
            Ok(hops)
        })
        .await?;
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

/// Hands a task we're serving to `request.new_provider_instance`: the
/// requester is told (and makes the new attempt), our row goes
/// `Reassigned`, and our run of it stops. Returns our row.
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
    let by = our_name(service);
    let reason = request
        .reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    // The requester first: if it can't be told, nothing changes here
    service
        .send(
            &task.requester_instance,
            PeerMessage::ReassignTask {
                task_id: task.id.clone(),
                new_provider_instance: candidate.instance_id.clone(),
                new_provider_card: candidate.card_id.clone(),
                new_provider_name: Some(candidate.display_name.clone()),
                by: by.clone(),
                reason: reason.clone(),
            },
        )
        .await?;
    task_state::advance(
        service.db(),
        service.bus(),
        &task.id,
        TaskState::Reassigned,
        Change::reason(format!("Reassigned to {} by {by}", candidate.display_name)),
    )
    .await?;
    service.stop_run(&task.id).await;
    let event = TaskReassigned {
        task_id: task.id.clone(),
        from_instance: service.instance_id().to_string(),
        to_instance: candidate.instance_id,
        to_name: Some(candidate.display_name),
        by,
        reason,
        new_task_id: None,
        reassigned_at: now_ms() as Timestamp,
    };
    record(service, &event).await?;
    events::emit(service.bus(), &event);
    log::info!("Task {} reassigned to {}", task.id, event.to_instance);
    service.get(&task.id).await
}

// ---------------------------------------------------------------------------
// Requester side
// ---------------------------------------------------------------------------

/// The provider handed our attempt `task` on: it goes `Reassigned`, and a
/// new attempt goes to the new provider. A handoff to an instance that
/// already had the task (a loop) fails the attempt instead, so failover
/// picks someone new.
pub(super) async fn accept(
    service: &Arc<TaskService>,
    task: &TaskRecord,
    handoff: Handoff,
) -> Result<(), String> {
    if task.state.is_terminal() {
        log::debug!("Task {}: ignoring a reassignment, it's over", task.id);
        return Ok(());
    }
    let (db, bus) = (service.db(), service.bus());
    let to = handoff.to_instance.clone();
    let looped = to == service.instance_id()
        || instance_of(&handoff.to_card) != to
        || visited(service, task).await?.contains(&to);
    if looped {
        let error = format!(
            "{} tried to reassign it to {to}, which already had it",
            handoff.by
        );
        log::warn!("Task {}: {error}", task.id);
        if let Some(ended) =
            task_state::advance(db, bus, &task.id, TaskState::Failed, Change::error(error)).await?
        {
            service.after_failure(ended);
        }
        return Ok(());
    }
    let Some(request) = task::request_of(db, &task.id).await? else {
        return Err(format!(
            "Task {}: the request to resend is missing",
            task.id
        ));
    };
    let relay = unavailable::connected(service.relay())?.clone();

    // The successor first: a delegation waiting on `task` looks for it as
    // soon as `task` reads `Reassigned`
    let id = uuid::Uuid::new_v4().to_string();
    let mut record_row = failover::next_attempt(task, &id, &handoff.to_card);
    record_row.source_kind = TaskSourceKind::Reassign;
    let now = now_ms() as f64;
    let wire = Task {
        id: id.clone(),
        target: handoff.to_card.clone(),
        created_at: now,
        ..request
    };
    service.create(&record_row, Some(&wire)).await?;
    // Before sending: the result may come back before `delegate_task` does
    service.follow(&task.id, &id);

    let to_name = handoff.to_name.clone().unwrap_or_else(|| to.clone());
    let note = format!("Reassigned to {to_name} by {}", handoff.by);
    task_state::advance(
        db,
        bus,
        &task.id,
        TaskState::Reassigned,
        Change::reason(match &handoff.reason {
            Some(reason) => format!("{note}: {reason}"),
            None => note,
        }),
    )
    .await?;

    let event = TaskReassigned {
        task_id: task.id.clone(),
        from_instance: task.provider_instance.clone(),
        to_instance: to.clone(),
        to_name: handoff.to_name,
        by: handoff.by,
        reason: handoff.reason,
        new_task_id: Some(id.clone()),
        reassigned_at: now,
    };
    record(service, &event).await?;
    let payload = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
    task::add_message(db, &task.id, REASSIGNED_KIND, payload).await?;
    events::emit(bus, &event);

    match relay.delegate_task(&wire).await {
        Ok(_) => {
            task_state::advance(
                db,
                bus,
                &id,
                TaskState::Delivered,
                Change::reason("routed by the relay"),
            )
            .await?;
        }
        // Queued: the next flush sends it
        Err(e) => log::info!("Task {id} waits for {to}: {e}"),
    }
    log::info!("Task {} reassigned to {to} as {id}", task.id);
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
