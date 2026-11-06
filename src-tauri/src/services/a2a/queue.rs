//! Requester side: the outbox. Provider side: the slot queue.
//!
//! **Outbox.** The relay holds work for someone offline, fails it over,
//! and ends it at its due date (relay design step 11). This Nandi only
//! keeps what the relay hasn't taken yet: a question or an assignment
//! (`ask-me`, `assign-me`) made while the relay was unreachable waits
//! `Submitted` here, with no `_accepted` message, and is sent (priority,
//! then age) when the relay connects and on every sweep. Anything else
//! made with no relay is refused at once (`RelayUnavailable`). Once the
//! relay takes a request it's marked `_accepted` and never sent again
//! (the relay deduplicates a request sent twice by its ID anyway).
//!
//! **Slot queue.** Inbound tasks over `max_concurrent` stay `Delivered`
//! until a slot opens.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::json;

use super::task::{self, TaskRecord, TaskService, TaskState};
use super::task_state::{self, Change};
use crate::services::relay::client_api::REFUSED_BY_RELAY;

/// How often the provider side looks for a free slot
pub const DEQUEUE_INTERVAL_SECS: u64 = 5;
/// A live wait stretches to this many ETAs after the acknowledgement
pub const TIMEOUT_ETA_FACTOR: u32 = 3;
/// `task_messages.kind` (internal) once the relay took a request
pub const ACCEPTED_KIND: &str = "_accepted";

/// Our requests the relay hasn't taken yet: priority first, then the
/// oldest
pub fn outbox(conn: &Connection, local_instance: &str) -> rusqlite::Result<Vec<TaskRecord>> {
    task::select(
        conn,
        "requester_instance = ?1 AND provider_instance != ?1 AND state = 'submitted'
         AND NOT EXISTS (SELECT 1 FROM task_messages m WHERE m.task_id = tasks.id AND m.kind = '_accepted')
         ORDER BY priority DESC, created_at ASC, rowid ASC",
        &[&local_instance],
    )
}

/// Records that the relay took `task_id` (`held`: its provider is offline,
/// it waits at the relay for them), and moves it on: `Delivered` when the
/// provider has it, else a note saying the relay holds it
pub async fn accepted(service: &TaskService, task_id: &str, held: bool) {
    let (db, bus) = (service.db(), service.bus());
    if let Err(e) = task::add_message(db, task_id, ACCEPTED_KIND, json!({ "held": held })).await {
        log::warn!("Task {task_id}: {e}");
    }
    let outcome = if held {
        let mut change = Change::reason("waiting at the relay until they're online");
        change.reason_kind = Some(super::task::ReasonKind::Offline);
        change.stage = Some(super::task::TaskStage::Dispatch);
        task_state::note(db, bus, task_id, change).await
    } else {
        task_state::advance(
            db,
            bus,
            task_id,
            TaskState::Delivered,
            Change::reason("with the provider"),
        )
        .await
        .map(|_| ())
    };
    if let Err(e) = outcome {
        log::warn!("Task {task_id}: {e}");
    }
}

impl TaskService {
    /// Sends what waits in the outbox, once the relay is connected; the
    /// IDs the relay took, in order. Stops at the first send that fails
    /// (the relay went away again).
    pub async fn flush(&self) -> Vec<String> {
        let Some(relay) = self.relay().cloned() else {
            return Vec::new();
        };
        if !relay.status().connected {
            return Vec::new();
        }
        // Held to the end: the next flush reads the outbox after this one
        // marked what it sent
        let _flushing = self.flushing.lock().await;
        let me = self.instance_id().to_string();
        let rows = match self.db().call(move |conn| outbox(conn, &me)).await {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("Couldn't read the outbox: {e}");
                return Vec::new();
            }
        };
        let mut sent = Vec::new();
        for row in rows {
            let Ok(Some(request)) = task::request_of(self.db(), &row.id).await else {
                continue;
            };
            match relay.delegate_task(&request, &[]).await {
                Ok(submitted) => {
                    accepted(self, &row.id, submitted.held).await;
                    sent.push(row.id);
                }
                Err(e) if e.starts_with(REFUSED_BY_RELAY) => {
                    // The relay refused it: it ends here
                    let change = Change::error(e.clone()).kind(
                        super::task::ReasonKind::Offline,
                        super::task::TaskStage::Dispatch,
                    );
                    if let Err(e) = task_state::advance(
                        self.db(),
                        self.bus(),
                        &row.id,
                        TaskState::Abandoned,
                        change,
                    )
                    .await
                    {
                        log::warn!("Task {}: {e}", row.id);
                    }
                    let _ = super::history::close(
                        self.db(),
                        self.bus(),
                        &row.id,
                        super::TaskStatus::Failed,
                        serde_json::Value::Null,
                        Some(e),
                    )
                    .await;
                }
                // It didn't get there (the relay went again, or isn't
                // ready yet): the next flush tries again
                Err(e) => {
                    log::info!("Outbox: {} waits ({e})", row.id);
                    break;
                }
            }
        }
        sent
    }
}

// ---------------------------------------------------------------------------
// Provider side: waiting for a slot
// ---------------------------------------------------------------------------

struct Waiting {
    priority: i32,
    created_at: i64,
    task_id: String,
}

/// Inbound tasks waiting for a free slot on their agent. The executor polls
/// every [`DEQUEUE_INTERVAL_SECS`]; only the head of an agent's line
/// (priority, then the oldest) may take a slot.
#[derive(Default)]
pub struct SlotQueue(Mutex<HashMap<String, Vec<Waiting>>>);

impl SlotQueue {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Waiting>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn join(&self, agent_id: &str, task_id: &str, priority: i32, created_at: i64) {
        let mut queues = self.lock();
        let line = queues.entry(agent_id.to_string()).or_default();
        if line.iter().any(|w| w.task_id == task_id) {
            return;
        }
        line.push(Waiting {
            priority,
            created_at,
            task_id: task_id.to_string(),
        });
        line.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then(a.created_at.cmp(&b.created_at))
        });
    }

    pub fn is_next(&self, agent_id: &str, task_id: &str) -> bool {
        self.lock()
            .get(agent_id)
            .and_then(|line| line.first())
            .is_none_or(|w| w.task_id == task_id)
    }

    pub fn leave(&self, agent_id: &str, task_id: &str) {
        let mut queues = self.lock();
        if let Some(line) = queues.get_mut(agent_id) {
            line.retain(|w| w.task_id != task_id);
            if line.is_empty() {
                queues.remove(agent_id);
            }
        }
    }

    pub fn waiting(&self, agent_id: &str) -> usize {
        self.lock().get(agent_id).map_or(0, Vec::len)
    }

    /// 1-based place of `task_id` in `agent_id`'s line, if it waits
    pub fn position(&self, agent_id: &str, task_id: &str) -> Option<usize> {
        self.lock()
            .get(agent_id)?
            .iter()
            .position(|w| w.task_id == task_id)
            .map(|index| index + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slot_line_is_priority_then_age() {
        let slots = SlotQueue::default();
        slots.join("a", "old", 0, 1);
        slots.join("a", "new", 0, 2);
        slots.join("a", "urgent", 5, 3);
        assert!(slots.is_next("a", "urgent"));
        assert!(!slots.is_next("a", "old"));
        slots.leave("a", "urgent");
        assert!(slots.is_next("a", "old"));
        assert_eq!(slots.waiting("a"), 2);
        assert!(slots.is_next("b", "anything"), "nobody waiting");
    }

    #[test]
    fn positions_are_one_based_and_follow_the_line() {
        let slots = SlotQueue::default();
        slots.join("a", "first", 0, 1);
        slots.join("a", "second", 0, 2);
        assert_eq!(slots.position("a", "first"), Some(1));
        assert_eq!(slots.position("a", "second"), Some(2));
        slots.leave("a", "first");
        assert_eq!(slots.position("a", "second"), Some(1));
        assert_eq!(slots.position("a", "gone"), None);
        assert_eq!(slots.position("nobody", "x"), None);
    }
}
