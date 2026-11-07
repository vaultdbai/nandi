//! Calendar handovers: the backup takes over when the primary doesn't do
//! the work.
//!
//! **Across Nandis the relay watches and decides** (relay design step 14).
//! The backup's Nandi registers a watch at the relay (`DutyWatch`); the
//! relay asks the primary's Nandi how the duty is going a grace period
//! after it's due, decides, and sends the backup its call: settled, or
//! hand over (with why). The primary's Nandi answers from the duty's fire
//! row and its run's status ([`duty_state`], the same answer either way),
//! a failure's error sealed for the backup. The relay keeps the watch on
//! disk, so a relay restart loses nothing.
//!
//! **With the relay down, the backup decides alone** (the owner's decision
//! 6): once the grace period has passed and the relay can't be reached,
//! it hands over, as for a primary that doesn't answer.
//!
//! **One Nandi watching the run it started** (an owner who is also the
//! backup's contact) needs nobody else, so it decides here with
//! [`decide`]:
//!
//! | Answer                                   | Then                                      |
//! | ---------------------------------------- | ----------------------------------------- |
//! | Completed                                | Done                                      |
//! | Running, within `backupAfterMinutes`     | Ask again later                           |
//! | Running past it                          | Hand over                                 |
//! | Failed, or never started                 | Hand over                                 |
//! | Upcoming (the clock is behind)           | Ask again `GRACE` after it's due          |
//!
//! Handing over fires the trigger again with `role: backup` (rate limits
//! don't apply to it). This Nandi's side of a watch lives in memory: a
//! decision that arrives after a restart has nobody waiting and is logged.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::calendar;
use super::schema::{TriggerEvent, TriggerFireStatus, TriggerSourceType};
use super::store;
use crate::db::{self, Db};
use crate::services::domain::calendar::{is_one_of, DutyId};
use crate::services::relay::protocol::{DutyDecision, DutyDelivery, DutyState};
use crate::services::relay::RelayClient;
use crate::services::WeakAppState;

/// How long after the scheduled time the primary's run may take to start
/// (the relay's grace before it asks; the backup's before it decides alone)
pub const GRACE: Duration = Duration::from_secs(5 * 60);
/// How often a watch looks again
pub const POLL: Duration = Duration::from_secs(2 * 60);
/// A watch gives up this long after the scheduled time
pub const GIVE_UP_MS: i64 = 24 * 60 * 60 * 1000;

/// The relay side of handovers.
pub trait DutyNetwork: Send + Sync {
    /// The names this Nandi answers to in a calendar: its instance ID,
    /// display name, and owner's name and handle
    fn my_names(&self) -> Vec<String>;

    /// Asks the relay to watch `person`'s `duty_id`, due at `due_at`
    /// (epoch ms); its call comes on the receiver. `Err` when the relay
    /// can't be reached or doesn't know `person`.
    fn watch<'a>(
        &'a self,
        person: &'a str,
        duty_id: &'a str,
        due_at: i64,
        after_minutes: Option<u32>,
    ) -> BoxFuture<'a, Result<oneshot::Receiver<Step>, String>>;

    /// Whether the relay is connected now
    fn connected(&self) -> bool;
}

/// Where a watch gets its answers.
pub enum Watch {
    /// This Nandi started the primary's run
    Local { db: Db },
    /// The primary's own Nandi, through the relay, which decides
    Remote {
        network: Arc<dyn DutyNetwork>,
        primary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Wait(Duration),
    Handover(String),
    Done,
}

/// What to do about a local answer at `now` (epoch ms).
pub fn decide(answer: DutyState, now: i64, after_minutes: Option<u32>) -> Step {
    let until = |at: i64| Duration::from_millis(at.saturating_sub(now).max(0) as u64);
    match answer {
        DutyState::Completed => Step::Done,
        DutyState::Upcoming { due_at } => Step::Wait(until(due_at) + GRACE),
        DutyState::Running { since } => match after_minutes {
            Some(minutes) => {
                let deadline = since + i64::from(minutes) * 60_000;
                if now >= deadline {
                    Step::Handover(format!("not finished after {minutes} minutes"))
                } else {
                    Step::Wait(until(deadline).min(POLL))
                }
            }
            None => Step::Wait(POLL),
        },
        DutyState::Failed { error } => Step::Handover(match error {
            Some(error) => format!("the primary's run failed: {error}"),
            None => "the primary's run failed".to_string(),
        }),
        DutyState::NotRun => Step::Handover("the primary's run never started".to_string()),
    }
}

/// The relay's call, as a step
pub fn step_of(decision: DutyDecision, error: Option<String>) -> Step {
    match decision {
        DutyDecision::Settled => Step::Done,
        DutyDecision::Handover { reason } => Step::Handover(match error {
            Some(error) => format!("{}: {error}", reason.sentence()),
            None => reason.sentence(),
        }),
    }
}

/// Watches one duty until it's done, handed over, or given up.
pub fn spawn(
    runtime: &tokio::runtime::Handle,
    watch: Watch,
    duty_id: DutyId,
    after_minutes: Option<u32>,
    primary_payload: Value,
    events: mpsc::Sender<TriggerEvent>,
) {
    runtime.spawn(async move {
        let scheduled = calendar::local(duty_id.at)
            .map(|at| at.timestamp_millis())
            .unwrap_or_else(db::now_ms);
        let step = match watch {
            Watch::Local { db } => watch_locally(&db, &duty_id, scheduled, after_minutes).await,
            Watch::Remote { network, primary } => {
                watch_remotely(
                    network.as_ref(),
                    &primary,
                    &duty_id,
                    scheduled,
                    after_minutes,
                )
                .await
            }
        };
        let Some(reason) = step else {
            return;
        };
        log::info!("Handing {duty_id} to the backup: {reason}");
        let event = TriggerEvent {
            trigger_id: duty_id.trigger_id.clone(),
            source: TriggerSourceType::Calendar,
            payload: calendar::handover_payload(&primary_payload, &reason),
            test: false,
        };
        let _ = events.send(event).await;
    });
}

/// Watches our own run of the duty; why to hand over, if it comes to it
async fn watch_locally(
    db: &Db,
    duty_id: &DutyId,
    scheduled: i64,
    after_minutes: Option<u32>,
) -> Option<String> {
    let mut wait = POLL;
    loop {
        tokio::time::sleep(wait).await;
        let now = db::now_ms();
        if now - scheduled > GIVE_UP_MS {
            log::warn!("Handover watch for {duty_id} gave up after a day");
            return None;
        }
        match decide(duty_state(db, duty_id).await, now, after_minutes) {
            Step::Done => return None,
            Step::Wait(next) => wait = next,
            Step::Handover(reason) => return Some(reason),
        }
    }
}

/// The relay watches `primary`'s duty and decides; with the relay down
/// past the grace period, we hand over ourselves. Why to hand over, if
/// it comes to it.
async fn watch_remotely(
    network: &dyn DutyNetwork,
    primary: &str,
    duty_id: &DutyId,
    scheduled: i64,
    after_minutes: Option<u32>,
) -> Option<String> {
    let id = duty_id.to_string();
    let grace_ends = scheduled + GRACE.as_millis() as i64;
    let mut call: Option<oneshot::Receiver<Step>> = None;
    loop {
        let now = db::now_ms();
        if now - scheduled > GIVE_UP_MS {
            log::warn!("Handover watch for {duty_id} gave up after a day");
            return None;
        }
        if call.is_none() {
            match network.watch(primary, &id, scheduled, after_minutes).await {
                Ok(receiver) => call = Some(receiver),
                // Nobody to ask once it's due: as a primary that doesn't
                // answer
                Err(e) if now >= grace_ends => return Some(e),
                Err(e) => log::info!("The relay can't watch {duty_id} yet: {e}"),
            }
        }
        let wait = if now < grace_ends {
            Duration::from_millis((grace_ends - now) as u64).min(POLL)
        } else {
            POLL
        };
        match call.as_mut() {
            Some(receiver) => match tokio::time::timeout(wait, receiver).await {
                Ok(Ok(Step::Handover(reason))) => return Some(reason),
                Ok(Ok(_)) => return None,
                // The relay connection went (the watch stays at the relay;
                // ours is asked again)
                Ok(Err(_)) => call = None,
                Err(_) => {}
            },
            None => tokio::time::sleep(wait).await,
        }
        if db::now_ms() >= grace_ends && !network.connected() {
            return Some("the relay couldn't be reached to ask the primary".to_string());
        }
    }
}

/// How this Nandi is doing with `duty_id` as its primary, from the fire
/// row and its run's status.
pub async fn duty_state(db: &Db, duty_id: &DutyId) -> DutyState {
    let (trigger_id, id) = (duty_id.trigger_id.clone(), duty_id.to_string());
    let found = db
        .call(move |conn| {
            let Some(trigger) = store::get_trigger(conn, &trigger_id)? else {
                return Ok(None);
            };
            let Some(fire) = store::duty_fire(conn, &trigger_id, &id, "primary")? else {
                return Ok(Some((None, None)));
            };
            let status = match &fire.run_id {
                Some(run) => store::run_status(conn, trigger.target_type, run)?,
                None => None,
            };
            Ok(Some((Some(fire), status)))
        })
        .await;
    let due_at = calendar::local(duty_id.at).map(|at| at.timestamp_millis());
    match found {
        Ok(Some((Some(fire), status))) => fire_state(&fire, status.as_deref()),
        // No fire yet: still to come on this clock, or it never ran
        Ok(Some((None, _))) => match due_at.filter(|at| *at > db::now_ms()) {
            Some(due_at) => DutyState::Upcoming { due_at },
            None => DutyState::NotRun,
        },
        Ok(None) => DutyState::NotRun,
        Err(e) => DutyState::Failed {
            error: Some(format!("couldn't read the run: {e}")),
        },
    }
}

fn fire_state(fire: &super::schema::TriggerFire, run_status: Option<&str>) -> DutyState {
    let since = fire.fired_at as i64;
    match fire.status {
        TriggerFireStatus::Error => DutyState::Failed {
            error: fire.error.clone(),
        },
        TriggerFireStatus::RateLimited | TriggerFireStatus::Skipped => DutyState::NotRun,
        TriggerFireStatus::Queued => match run_status {
            // Dispatched, run not written yet
            None => DutyState::Running { since },
            Some("completed") => DutyState::Completed,
            Some("failed") | Some("cancelled") => DutyState::Failed { error: None },
            // queued, running, awaiting approval, compacted (continues),
            // paused or waiting (workflows)
            Some(_) => DutyState::Running { since },
        },
    }
}

// ---------------------------------------------------------------------------
// Over the relay
// ---------------------------------------------------------------------------

/// Registers watches with the relay and answers its duty questions.
pub struct RelayDutyNetwork {
    state: WeakAppState,
    relay: Arc<dyn RelayClient>,
    /// Watches waiting for the relay's call, by duty ID
    pending: Mutex<HashMap<String, oneshot::Sender<Step>>>,
}

impl RelayDutyNetwork {
    /// The instance a calendar name refers to (online or not: the relay
    /// decides what an offline primary means)
    async fn find(&self, person: &str) -> Result<String, String> {
        let state = self.state.upgrade().ok_or("Nandi is shutting down")?;
        let snapshot = crate::services::network::get_network_snapshot(&state).await?;
        let names = [person.to_string()];
        snapshot
            .members
            .into_iter()
            .find(|m| {
                is_one_of(&m.instance_id, &names)
                    || is_one_of(&m.display_name, &names)
                    || m.handle.as_deref().is_some_and(|h| is_one_of(h, &names))
            })
            .map(|m| m.instance_id)
            .ok_or_else(|| format!("{person} isn't on the relay"))
    }

    /// Whether `instance` may ask about this trigger: its owner is named
    /// in the calendar.
    async fn may_ask(
        &self,
        state: &crate::services::AppState,
        instance: &str,
        trigger_id: &str,
    ) -> bool {
        let names = match crate::services::network::get_network_snapshot(state).await {
            Ok(snapshot) => snapshot
                .members
                .into_iter()
                .find(|m| m.instance_id == instance)
                .map(|m| {
                    [Some(m.instance_id), Some(m.display_name), m.handle]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            Err(_) => return false,
        };
        let id = trigger_id.to_string();
        let trigger = state
            .db()
            .call(move |conn| store::get_trigger(conn, &id))
            .await
            .ok()
            .flatten();
        trigger
            .filter(|t| t.source_type == TriggerSourceType::Calendar)
            .and_then(|t| calendar::parse(&t.source_config).ok())
            .is_some_and(|spec| spec.mentions(&names))
    }

    async fn on_duty(&self, delivery: DutyDelivery) {
        match delivery {
            // As the primary: a backup named in the calendar watches
            DutyDelivery::Query {
                duty_id,
                backup_instance,
            } => {
                let Some(state) = self.state.upgrade() else {
                    return;
                };
                let Some(parsed) = DutyId::parse(&duty_id) else {
                    return;
                };
                if !self
                    .may_ask(&state, &backup_instance, &parsed.trigger_id)
                    .await
                {
                    log::warn!(
                        "Not reporting {duty_id} for {backup_instance}: they aren't named in it"
                    );
                    return;
                }
                let answer = duty_state(state.db(), &parsed).await;
                if let Err(e) = self
                    .relay
                    .duty_report(&duty_id, &backup_instance, answer)
                    .await
                {
                    log::warn!("Couldn't report {duty_id} to the relay: {e}");
                }
            }
            // As the backup: the relay's call
            DutyDelivery::Decision {
                duty_id,
                decision,
                error,
            } => {
                let waiting = self
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&duty_id);
                match waiting {
                    Some(waiting) => {
                        let _ = waiting.send(step_of(decision, error));
                    }
                    None => log::info!("The relay decided {duty_id}, but nothing here watches it"),
                }
            }
        }
    }
}

impl DutyNetwork for RelayDutyNetwork {
    fn my_names(&self) -> Vec<String> {
        let Some(state) = self.state.upgrade() else {
            return Vec::new();
        };
        let mut names = Vec::new();
        if let Ok(id) = crate::services::network::local_instance_id(state.db()) {
            names.push(id);
        }
        if let Some(manifest) = state.gateway().and_then(|g| g.manifest().ok()) {
            names.extend([
                manifest.display_name,
                manifest.owner.display_name,
                manifest.owner.github_handle,
                manifest.user_slug,
            ]);
        }
        names.retain(|n| !n.trim().is_empty());
        names
    }

    fn watch<'a>(
        &'a self,
        person: &'a str,
        duty_id: &'a str,
        due_at: i64,
        after_minutes: Option<u32>,
    ) -> BoxFuture<'a, Result<oneshot::Receiver<Step>, String>> {
        Box::pin(async move {
            let primary = self.find(person).await?;
            let (tx, rx) = oneshot::channel();
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(duty_id.to_string(), tx);
            if let Err(e) = self
                .relay
                .duty_watch(duty_id, &primary, due_at, after_minutes)
                .await
            {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(duty_id);
                return Err(e);
            }
            Ok(rx)
        })
    }

    fn connected(&self) -> bool {
        self.relay.status().connected
    }
}

/// Connects the trigger engine's calendar handovers to the relay. Call
/// once the relay client is set.
pub fn init(state: &crate::services::AppState) {
    let Some(relay) = state.relay_client().cloned() else {
        return;
    };
    let network = Arc::new(RelayDutyNetwork {
        state: state.downgrade(),
        relay: relay.clone(),
        pending: Mutex::default(),
    });
    let handler = network.clone();
    let runtime = state.runtime();
    runtime.spawn(async move {
        relay
            .on_duty(Box::new(move |delivery: DutyDelivery| {
                let handler = handler.clone();
                tokio::spawn(async move { handler.on_duty(delivery).await });
            }))
            .await;
    });
    state.triggers().set_duty_network(network);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn decides_when_the_backup_takes_over() {
        let now = 1_000 * MIN;
        assert_eq!(decide(DutyState::Completed, now, Some(30)), Step::Done);
        // Running: wait, then hand over once it's late
        assert_eq!(
            decide(
                DutyState::Running {
                    since: now - 10 * MIN
                },
                now,
                Some(30)
            ),
            Step::Wait(POLL)
        );
        assert_eq!(
            decide(
                DutyState::Running {
                    since: now - MIN * 29 - 30_000
                },
                now,
                Some(30)
            ),
            Step::Wait(Duration::from_secs(30))
        );
        assert!(matches!(
            decide(DutyState::Running { since: now - 30 * MIN }, now, Some(30)),
            Step::Handover(reason) if reason.contains("30 minutes")
        ));
        // Without a deadline, only a failure hands over
        assert_eq!(
            decide(
                DutyState::Running {
                    since: now - 600 * MIN
                },
                now,
                None
            ),
            Step::Wait(POLL)
        );
        assert!(matches!(
            decide(DutyState::Failed { error: Some("boom".into()) }, now, None),
            Step::Handover(reason) if reason.contains("boom")
        ));
        assert!(matches!(
            decide(DutyState::NotRun, now, None),
            Step::Handover(_)
        ));
        // The primary's clock is behind: ask again after it's due
        assert_eq!(
            decide(
                DutyState::Upcoming {
                    due_at: now + 60 * MIN
                },
                now,
                None
            ),
            Step::Wait(Duration::from_secs(3600) + GRACE)
        );
    }

    fn fire(status: TriggerFireStatus) -> super::super::schema::TriggerFire {
        super::super::schema::TriggerFire {
            id: "f".into(),
            trigger_id: "t".into(),
            fired_at: 5.0,
            payload: Value::Null,
            run_id: Some("r".into()),
            status,
            error: Some("no agent".into()),
            run_status: None,
        }
    }

    #[test]
    fn reads_the_duty_from_the_fire_and_its_run() {
        use TriggerFireStatus::*;
        assert_eq!(
            fire_state(&fire(Queued), Some("completed")),
            DutyState::Completed
        );
        assert_eq!(
            fire_state(&fire(Queued), Some("running")),
            DutyState::Running { since: 5 }
        );
        assert_eq!(
            fire_state(&fire(Queued), None),
            DutyState::Running { since: 5 }
        );
        assert_eq!(
            fire_state(&fire(Queued), Some("failed")),
            DutyState::Failed { error: None }
        );
        assert_eq!(
            fire_state(&fire(Error), None),
            DutyState::Failed {
                error: Some("no agent".into())
            }
        );
        assert_eq!(fire_state(&fire(RateLimited), None), DutyState::NotRun);
    }
}
