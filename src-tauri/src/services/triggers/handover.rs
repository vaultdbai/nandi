//! Calendar handovers: the backup takes over when the primary doesn't do
//! the work.
//!
//! A watch asks how the primary's duty is going until it's settled. It
//! asks the local database (an owner Nandi watching the run it started)
//! or, over the relay, the primary's own Nandi (`DutyQuery` →
//! `DutyStatus`). Both answers come from the same place: the duty's fire
//! row and its run's status (`duty_state`), so they survive restarts.
//!
//! | Answer                                   | Then                                      |
//! | ---------------------------------------- | ----------------------------------------- |
//! | Completed                                | Done                                      |
//! | Running, within `backupAfterMinutes`     | Ask again later                           |
//! | Running past it                          | Hand over                                 |
//! | Failed, or never started                 | Hand over                                 |
//! | Upcoming (the primary's clock is behind) | Ask again `GRACE` after it's due          |
//! | No answer (offline, Nandi not running)  | Hand over                                 |
//!
//! Handing over fires the trigger again with `role: backup` (rate limits
//! don't apply to it). Watches live in memory: one interrupted by a restart
//! is not resumed.

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
use crate::services::relay::protocol::{DutyState, PeerDelivery, PeerMessage};
use crate::services::relay::RelayClient;
use crate::services::WeakAppState;

/// How long a backup waits after the scheduled time before the first
/// question (the primary's run needs a moment to start)
pub const GRACE: Duration = Duration::from_secs(5 * 60);
/// Between questions while the primary's run is going
pub const POLL: Duration = Duration::from_secs(2 * 60);
/// A watch gives up this long after the scheduled time
pub const GIVE_UP_MS: i64 = 24 * 60 * 60 * 1000;
/// How long the primary's Nandi has to answer
const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

/// The relay side of handovers.
pub trait DutyNetwork: Send + Sync {
    /// The names this Nandi answers to in a calendar: its instance ID,
    /// display name, and owner's name and handle
    fn my_names(&self) -> Vec<String>;

    /// How `person`'s Nandi is doing with `duty_id`. `Err` when it can't
    /// be asked or doesn't answer (offline, unknown, Nandi not running).
    fn ask<'a>(
        &'a self,
        person: &'a str,
        duty_id: &'a str,
    ) -> BoxFuture<'a, Result<DutyState, String>>;
}

/// Where a watch gets its answers.
pub enum Watch {
    /// This Nandi started the primary's run
    Local { db: Db },
    /// The primary's own Nandi, over the relay
    Remote {
        network: Arc<dyn DutyNetwork>,
        primary: String,
    },
}

impl Watch {
    async fn observe(&self, duty_id: &DutyId) -> Result<DutyState, String> {
        match self {
            Self::Local { db } => Ok(duty_state(db, duty_id).await),
            Self::Remote { network, primary } => network.ask(primary, &duty_id.to_string()).await,
        }
    }

    fn first_check(&self) -> Duration {
        match self {
            Self::Local { .. } => POLL,
            Self::Remote { .. } => GRACE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Wait(Duration),
    Handover(String),
    Done,
}

/// What to do about an answer at `now` (epoch ms).
pub fn decide(answer: Result<DutyState, String>, now: i64, after_minutes: Option<u32>) -> Step {
    let until = |at: i64| Duration::from_millis(at.saturating_sub(now).max(0) as u64);
    match answer {
        Ok(DutyState::Completed) => Step::Done,
        Ok(DutyState::Upcoming { due_at }) => Step::Wait(until(due_at) + GRACE),
        Ok(DutyState::Running { since }) => match after_minutes {
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
        Ok(DutyState::Failed { error }) => Step::Handover(match error {
            Some(error) => format!("the primary's run failed: {error}"),
            None => "the primary's run failed".to_string(),
        }),
        Ok(DutyState::NotRun) => Step::Handover("the primary's run never started".to_string()),
        Err(reason) => Step::Handover(reason),
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
        let mut wait = watch.first_check();
        loop {
            tokio::time::sleep(wait).await;
            let now = db::now_ms();
            if now - scheduled > GIVE_UP_MS {
                log::warn!("Handover watch for {duty_id} gave up after a day");
                return;
            }
            match decide(watch.observe(&duty_id).await, now, after_minutes) {
                Step::Done => return,
                Step::Wait(next) => wait = next,
                Step::Handover(reason) => {
                    log::info!("Handing {duty_id} to the backup: {reason}");
                    let event = TriggerEvent {
                        trigger_id: duty_id.trigger_id.clone(),
                        source: TriggerSourceType::Calendar,
                        payload: calendar::handover_payload(&primary_payload, &reason),
                        test: false,
                    };
                    let _ = events.send(event).await;
                    return;
                }
            }
        }
    });
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

/// Asks and answers duty questions through the relay.
pub struct RelayDutyNetwork {
    state: WeakAppState,
    relay: Arc<dyn RelayClient>,
    /// Questions waiting for an answer, by duty ID
    pending: Mutex<HashMap<String, oneshot::Sender<Result<DutyState, String>>>>,
}

impl RelayDutyNetwork {
    /// The instance a calendar name refers to, if it's online.
    async fn find(&self, person: &str) -> Result<String, String> {
        let state = self.state.upgrade().ok_or("Nandi is shutting down")?;
        let snapshot = crate::services::network::get_network_snapshot(&state).await?;
        let names = [person.to_string()];
        let member = snapshot
            .members
            .into_iter()
            .find(|m| {
                is_one_of(&m.instance_id, &names)
                    || is_one_of(&m.display_name, &names)
                    || m.handle.as_deref().is_some_and(|h| is_one_of(h, &names))
            })
            .ok_or_else(|| format!("{person} isn't on the relay"))?;
        if !member.online {
            return Err(format!("{person}'s Nandi is offline"));
        }
        Ok(member.instance_id)
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

    async fn on_peer(&self, delivery: PeerDelivery) {
        match delivery.body {
            PeerMessage::DutyQuery { duty_id } if delivery.undeliverable => {
                self.resolve(&duty_id, Err("the primary's Nandi is offline".into()));
            }
            PeerMessage::DutyQuery { duty_id } => {
                let Some(state) = self.state.upgrade() else {
                    return;
                };
                let Some(parsed) = DutyId::parse(&duty_id) else {
                    return;
                };
                if !self
                    .may_ask(&state, &delivery.from_instance, &parsed.trigger_id)
                    .await
                {
                    log::warn!(
                        "Ignoring a duty question about {duty_id} from {}: not named in it",
                        delivery.from_instance
                    );
                    return;
                }
                let answer = PeerMessage::DutyStatus {
                    state: duty_state(state.db(), &parsed).await,
                    duty_id,
                };
                if let Err(e) = self.relay.send_peer(delivery.from_instance, answer).await {
                    log::warn!("Couldn't answer a duty question: {e}");
                }
            }
            PeerMessage::DutyStatus { duty_id, state } if !delivery.undeliverable => {
                self.resolve(&duty_id, Ok(state));
            }
            _ => {}
        }
    }

    fn resolve(&self, duty_id: &str, answer: Result<DutyState, String>) {
        let waiting = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(duty_id);
        if let Some(waiting) = waiting {
            let _ = waiting.send(answer);
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

    fn ask<'a>(
        &'a self,
        person: &'a str,
        duty_id: &'a str,
    ) -> BoxFuture<'a, Result<DutyState, String>> {
        Box::pin(async move {
            let instance = self.find(person).await?;
            let (tx, rx) = oneshot::channel();
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(duty_id.to_string(), tx);
            let query = PeerMessage::DutyQuery {
                duty_id: duty_id.to_string(),
            };
            if let Err(e) = self.relay.send_peer(instance, query).await {
                self.resolve(duty_id, Err(e));
            }
            match tokio::time::timeout(ANSWER_TIMEOUT, rx).await {
                Ok(Ok(answer)) => answer,
                _ => {
                    self.resolve(duty_id, Err(String::new()));
                    Err(format!("{person}'s Nandi didn't answer"))
                }
            }
        })
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
            .on_peer(Box::new(move |delivery: PeerDelivery| {
                if matches!(
                    delivery.body,
                    PeerMessage::DutyQuery { .. } | PeerMessage::DutyStatus { .. }
                ) {
                    let handler = handler.clone();
                    tokio::spawn(async move { handler.on_peer(delivery).await });
                }
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
        assert_eq!(decide(Ok(DutyState::Completed), now, Some(30)), Step::Done);
        // Running: wait, then hand over once it's late
        assert_eq!(
            decide(
                Ok(DutyState::Running {
                    since: now - 10 * MIN
                }),
                now,
                Some(30)
            ),
            Step::Wait(POLL)
        );
        assert_eq!(
            decide(
                Ok(DutyState::Running {
                    since: now - MIN * 29 - 30_000
                }),
                now,
                Some(30)
            ),
            Step::Wait(Duration::from_secs(30))
        );
        assert!(matches!(
            decide(Ok(DutyState::Running { since: now - 30 * MIN }), now, Some(30)),
            Step::Handover(reason) if reason.contains("30 minutes")
        ));
        // Without a deadline, only a failure hands over
        assert_eq!(
            decide(
                Ok(DutyState::Running {
                    since: now - 600 * MIN
                }),
                now,
                None
            ),
            Step::Wait(POLL)
        );
        assert!(matches!(
            decide(Ok(DutyState::Failed { error: Some("boom".into()) }), now, None),
            Step::Handover(reason) if reason.contains("boom")
        ));
        assert!(matches!(
            decide(Ok(DutyState::NotRun), now, None),
            Step::Handover(_)
        ));
        assert!(matches!(
            decide(Err("bob's Nandi is offline".into()), now, None),
            Step::Handover(reason) if reason.contains("offline")
        ));
        // The primary's clock is behind: ask again after it's due
        assert_eq!(
            decide(
                Ok(DutyState::Upcoming {
                    due_at: now + 60 * MIN
                }),
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
