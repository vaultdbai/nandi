//! Offline queue (wave final, Track B). Requester side: tasks for an
//! offline provider stay `Submitted` and flush (priority, then age) when it
//! comes online. Provider side: tasks over `max_concurrent` stay
//! `Delivered` until a slot opens. Queued over [`ABANDON_AFTER_SECS`] with
//! the provider still offline: `Abandoned`.
//!
//! Also the requester's sweeps (run every `TaskConfig::sweep_interval`):
//! - flush: queued tasks go to providers that are online again (also on
//!   `relay://agent_online`)
//! - timeouts: `Acknowledged` / `Working` longer than
//!   [`TIMEOUT_ETA_FACTOR`] × the ETA since the acknowledgement →
//!   `TimedOut`, and the provider gets `TaskTimedOut`
//! - abandons: with the provider offline, `Submitted` (never sent) after
//!   [`ABANDON_AFTER_SECS`], `Delivered` (sent, never acknowledged) after
//!   [`ABANDON_DELIVERED_AFTER_SECS`] → `Abandoned`
//!
//! Assignments (`assignment::ASSIGN_ME`) last until their person ends them:
//! neither timeouts nor abandons touch them.
//!
//! Each sweep takes `now`, so tests move the clock instead of waiting.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::eta::DEFAULT_MEDIAN_SECONDS;
use super::task::{self, TaskRecord, TaskService, TaskState};
use super::task_state::{self, Change};
use super::{assignment, failover, history, TaskStatus};
use crate::services::relay::protocol::PeerMessage;

/// Seconds a queued task waits for an offline provider
pub const ABANDON_AFTER_SECS: u64 = 300;
/// How often the provider side looks for a free slot
pub const DEQUEUE_INTERVAL_SECS: u64 = 5;
/// Seconds a sent but unacknowledged task waits for an offline provider
pub const ABANDON_DELIVERED_AFTER_SECS: u64 = 90;
/// A task times out this many ETAs after it was acknowledged
pub const TIMEOUT_ETA_FACTOR: u32 = 3;

/// Our queued tasks (requester rows still `Submitted`), for one provider
/// instance or all: priority first, then the oldest.
pub fn queued(
    conn: &Connection,
    local_instance: &str,
    provider_instance: Option<&str>,
) -> rusqlite::Result<Vec<TaskRecord>> {
    let order = "ORDER BY priority DESC, created_at ASC, rowid ASC";
    match provider_instance {
        Some(provider) => task::select(
            conn,
            &format!(
                "requester_instance = ?1 AND provider_instance = ?2 AND state = 'submitted' {order}"
            ),
            &[&local_instance, &provider],
        ),
        None => task::select(
            conn,
            &format!("requester_instance = ?1 AND provider_instance != ?1 AND state = 'submitted' {order}"),
            &[&local_instance],
        ),
    }
}

/// Instance presence, asked at most once per sweep
#[derive(Default)]
struct PresenceCache(HashMap<String, bool>);

impl PresenceCache {
    async fn online(&mut self, service: &TaskService, instance: &str) -> bool {
        if let Some(&online) = self.0.get(instance) {
            return online;
        }
        let online = service.is_online(instance).await;
        self.0.insert(instance.to_string(), online);
        online
    }
}

impl TaskService {
    /// Sends queued tasks to providers that are online: `Some(instance)`
    /// just came online (no presence check). A provider whose first send
    /// fails keeps the rest queued. Returns the IDs sent, in order.
    pub async fn flush(&self, provider_instance: Option<&str>) -> Vec<String> {
        let Some(relay) = self.relay().cloned() else {
            return Vec::new();
        };
        // Held to the end: the next flush reads the queue after this one
        // moved what it sent to `Delivered`
        let _flushing = self.flushing.lock().await;
        let me = self.instance_id().to_string();
        let provider = provider_instance.map(str::to_string);
        let rows = match self
            .db()
            .call(move |conn| queued(conn, &me, provider.as_deref()))
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("Couldn't read the offline queue: {e}");
                return Vec::new();
            }
        };
        let mut presence = PresenceCache::default();
        let mut stalled: HashSet<String> = HashSet::new();
        let mut sent = Vec::new();
        for row in rows {
            let provider = row.provider_instance.clone();
            if stalled.contains(&provider) {
                continue;
            }
            if provider_instance.is_none() && !presence.online(self, &provider).await {
                continue;
            }
            let Ok(Some(request)) = task::request_of(self.db(), &row.id).await else {
                continue;
            };
            match relay.delegate_task(&request).await {
                Ok(_) => {
                    let delivered = task_state::advance(
                        self.db(),
                        self.bus(),
                        &row.id,
                        TaskState::Delivered,
                        Change::reason("sent from the offline queue"),
                    )
                    .await;
                    if let Err(e) = delivered {
                        log::warn!("Task {}: {e}", row.id);
                    }
                    sent.push(row.id);
                }
                Err(e) => {
                    log::debug!("Task {} stays queued for {provider}: {e}", row.id);
                    stalled.insert(provider);
                }
            }
        }
        sent
    }

    /// `Acknowledged` / `Working` requester tasks past their due date (the
    /// task's deadline; without one, [`TIMEOUT_ETA_FACTOR`] × their ETA
    /// since the acknowledgement) → `TimedOut`, `TaskTimedOut` to the
    /// provider, one failure on its circuit, and failover when nobody waits
    /// on it.
    pub async fn sweep_timeouts(self: &Arc<Self>, now: i64) -> Result<Vec<String>, String> {
        let me = self.instance_id().to_string();
        let rows = self
            .db()
            .call(move |conn| {
                task::select(
                    conn,
                    "requester_instance = ?1 AND provider_instance != ?1 AND capability != ?2
                     AND state IN ('acknowledged', 'working') AND acknowledged_at IS NOT NULL",
                    &[&me, &assignment::ASSIGN_ME],
                )
            })
            .await?;
        let mut timed_out = Vec::new();
        for row in rows {
            let reason = match row.deadline {
                Some(due) => {
                    if now <= due as i64 {
                        continue;
                    }
                    "No answer by its due date".to_string()
                }
                None => {
                    let eta = row.eta_seconds.unwrap_or(DEFAULT_MEDIAN_SECONDS).max(1);
                    let limit = TIMEOUT_ETA_FACTOR as i64 * eta as i64 * 1000;
                    let since = row.acknowledged_at.unwrap_or(row.created_at) as i64;
                    if now - since <= limit {
                        continue;
                    }
                    format!(
                        "No result within {}s ({TIMEOUT_ETA_FACTOR}× the {eta}s ETA)",
                        limit / 1000
                    )
                }
            };
            let Some(ended) = task_state::advance(
                self.db(),
                self.bus(),
                &row.id,
                TaskState::TimedOut,
                Change::error(reason.clone()),
            )
            .await?
            else {
                continue;
            };
            // A request nobody waited on live: its history row was open
            history::close(
                self.db(),
                self.bus(),
                &row.id,
                TaskStatus::TimedOut,
                serde_json::Value::Null,
                Some(reason.clone()),
            )
            .await?;
            if let Some(relay) = self.relay() {
                let told = relay
                    .send_peer(
                        row.provider_instance.clone(),
                        PeerMessage::TaskTimedOut {
                            task_id: row.id.clone(),
                            reason: Some(reason),
                        },
                    )
                    .await;
                if let Err(e) = told {
                    log::debug!(
                        "Couldn't tell {} task {} timed out: {e}",
                        row.provider_instance,
                        row.id
                    );
                }
            }
            failover::record_failure(
                self.db(),
                self.bus(),
                &row.provider_instance,
                &row.capability,
            )
            .await?;
            timed_out.push(row.id.clone());
            self.after_failure(ended);
        }
        Ok(timed_out)
    }

    /// Requester tasks the provider never picked up by their due date →
    /// `Abandoned` (then failover when nobody waits on them). A task
    /// without a due date is abandoned sooner, while its provider is
    /// offline.
    pub async fn sweep_abandoned(self: &Arc<Self>, now: i64) -> Result<Vec<String>, String> {
        let me = self.instance_id().to_string();
        let rows = self
            .db()
            .call(move |conn| {
                task::select(
                    conn,
                    "requester_instance = ?1 AND provider_instance != ?1 AND capability != ?2
                     AND state IN ('submitted', 'delivered')",
                    &[&me, &assignment::ASSIGN_ME],
                )
            })
            .await?;
        let mut presence = PresenceCache::default();
        let mut abandoned = Vec::new();
        for row in rows {
            let reason = match row.deadline {
                // Queued or delivered and waiting for them, however long
                // they're away, until it's due
                Some(due) => {
                    if now <= due as i64 {
                        continue;
                    }
                    format!(
                        "{} didn't pick it up by its due date",
                        row.provider_instance
                    )
                }
                None => {
                    let after = if row.state == TaskState::Submitted {
                        ABANDON_AFTER_SECS
                    } else {
                        ABANDON_DELIVERED_AFTER_SECS
                    };
                    if now - (row.created_at as i64) <= after as i64 * 1000 {
                        continue;
                    }
                    if presence.online(self, &row.provider_instance).await {
                        continue;
                    }
                    format!(
                        "{} stayed offline for more than {after}s",
                        row.provider_instance
                    )
                }
            };
            if let Some(ended) = task_state::advance(
                self.db(),
                self.bus(),
                &row.id,
                TaskState::Abandoned,
                Change::error(reason.clone()),
            )
            .await?
            {
                history::close(
                    self.db(),
                    self.bus(),
                    &row.id,
                    TaskStatus::TimedOut,
                    serde_json::Value::Null,
                    Some(reason),
                )
                .await?;
                abandoned.push(row.id.clone());
                self.after_failure(ended);
            }
        }
        Ok(abandoned)
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
