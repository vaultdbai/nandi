//! Delegation end to end: local in-process, two instances over a mock
//! relay, the tool's approval, timeouts, and rejections; and the task
//! lifecycle over it (wave final, Track B).
//!
//! The mock relay seals and opens like the real client: every task,
//! result and peer message between instances goes through each side's
//! `Sealer` (`relay::sealing`), so these tests run with sealing on. It
//! also stores and forwards like the relay: what's for an instance that's
//! offline waits until it's back, a refusal goes to the next candidate,
//! and a reassignment moves the task. The real relay's frames are checked
//! in `relay/sealed_tests.rs`; its rules are its own tests.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::delegator::{instance_of, A2aDeps, Delegator};
use super::history::{self, AuditLog};
use super::task::TaskState;
use super::*;
use crate::db;
use crate::engine::CANNED_SCRIPT_KEY;
use crate::model::{ApprovalDecision, RunStatus};
use crate::services::chat;
use crate::services::domain::{Agent, AgentId};
use crate::services::events::tests::RecordingSink;
use crate::services::relay::client_api::{Handler, Submitted};
use crate::services::relay::protocol::{
    PeerDelivery, PeerMessage, TaskAssigned, TaskCompleted, TaskFailed, ALL_CAPABILITIES,
};
use crate::services::relay::sealing::{Opening, Sealer};
use crate::services::relay::{RelayClient, RelayStatus};
use crate::services::trust;
use nandi_protocol::{Envelope, PublicKeys, SealedTask};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn save_agent(state: &AppState, id: &str, frontmatter: &str) {
    let agent = Agent::from_skill_md(
        AgentId::parse(id).unwrap(),
        &format!("---\nname: {id}\n{frontmatter}---\nHelp.\n"),
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, id).unwrap();
}

/// Every agent replays `steps` (no model, no delay)
async fn canned(state: &AppState, steps: Value) {
    state
        .db()
        .call(move |conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(
                conn,
                CANNED_SCRIPT_KEY,
                &json!({ "agent": steps, "studio": [], "tokenDelayMs": 0 }),
            )
        })
        .await
        .unwrap();
}

/// Each agent answers with its input in brackets
async fn echo_agents(state: &AppState) {
    canned(state, json!([{ "text": "[{{input}}]" }])).await;
}

/// The delegation row of `task_id`
async fn delegation(state: &AppState, task_id: &str) -> TaskResult {
    history::get(state.db(), task_id).await.unwrap().unwrap()
}

fn instance(state: &AppState) -> String {
    crate::services::network::local_instance_id(state.db()).unwrap()
}

fn deps(state: &AppState, relay: Option<Arc<dyn RelayClient>>) -> A2aDeps {
    A2aDeps {
        relay,
        ..A2aDeps::from_state(state).unwrap()
    }
}

/// A task service on `relay`, listening and registered for the state's
/// instance (no background loops: tests call the sweeps)
pub(crate) async fn lifecycle(
    state: &AppState,
    relay: Arc<MockRelay>,
    config: task::TaskConfig,
) -> Arc<task::TaskService> {
    let service = task::TaskService::new(
        deps(state, Some(relay)),
        None,
        task::TaskConfig {
            status_timeout: Duration::from_millis(500),
            ..config
        },
        &tokio::runtime::Handle::current(),
    );
    service.listen().await;
    service.register();
    service
}

async fn wait_until<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

// ---------------------------------------------------------------------------
// A mock relay: instances on one hub, stored and forwarded like the relay
// ---------------------------------------------------------------------------

/// Something the hub holds for an instance until it's online, as the
/// relay's mailbox does
enum Held {
    Task { task_id: String },
    Result(nandi_protocol::ResultReport, String),
    Peer(String, nandi_protocol::PeerMessage),
    Event(crate::services::relay::protocol::TaskEvent),
}

/// A task as the hub carries it: who asked, and the request sealed for
/// each candidate (the current one first among those left)
struct Carried {
    requester: String,
    requester_card: AgentCard,
    capability: String,
    created_at: f64,
    deadline: Option<f64>,
    attempts: Vec<(String, Envelope)>,
    current: usize,
}

impl Carried {
    fn provider(&self) -> String {
        instance_of(&self.attempts[self.current].0).to_string()
    }
}

#[derive(Default)]
pub(crate) struct Hub {
    relays: Mutex<HashMap<String, Weak<MockRelay>>>,
    tasks: Mutex<HashMap<String, Carried>>,
    /// Joined, but unreachable (what's for them waits)
    offline: Mutex<HashSet<String>>,
    /// Waiting for instances that are offline, in order
    held: Mutex<Vec<(String, Held)>>,
}

impl Hub {
    /// `instance`'s client while it's online
    fn relay(&self, instance: &str) -> Option<Arc<MockRelay>> {
        if self.offline.lock().unwrap().contains(instance) {
            return None;
        }
        self.relays
            .lock()
            .unwrap()
            .get(instance)
            .and_then(Weak::upgrade)
    }

    /// `instance`'s keys, online or not (a relay keeps them pinned)
    fn keys(&self, instance: &str) -> Result<PublicKeys, String> {
        self.relays
            .lock()
            .unwrap()
            .get(instance)
            .and_then(Weak::upgrade)
            .map(|relay| relay.sealer.public_keys().clone())
            .ok_or_else(|| crate::services::relay::sealing::missing(instance))
    }

    /// Takes `instance` off line or back; back, it gets what waited
    fn set_online(&self, instance: &str, online: bool) {
        {
            let mut offline = self.offline.lock().unwrap();
            if online {
                offline.remove(instance);
            } else {
                offline.insert(instance.to_string());
            }
        }
        if online {
            let waiting: Vec<Held> = {
                let mut held = self.held.lock().unwrap();
                let (theirs, rest) = std::mem::take(&mut *held)
                    .into_iter()
                    .partition(|(to, _)| to == instance);
                *held = rest;
                theirs.into_iter().map(|(_, item)| item).collect()
            };
            for item in waiting {
                self.post(instance, item);
            }
        }
    }

    /// Delivers `item` to `to` now, or keeps it until they're online
    fn post(&self, to: &str, item: Held) {
        let Some(target) = self.relay(to) else {
            self.held.lock().unwrap().push((to.to_string(), item));
            return;
        };
        match item {
            Held::Task { task_id } => self.hand_over(&target, &task_id),
            Held::Result(report, from) => {
                let Ok(keys) = self.keys(&from) else { return };
                if let Ok(Opening::Opened(result)) =
                    target.sealer.open_result(report, (&from, &keys))
                {
                    for handler in target.completed.lock().unwrap().iter() {
                        handler(TaskCompleted {
                            task_id: result.task_id.clone(),
                            result: result.clone(),
                        });
                    }
                }
            }
            Held::Peer(from, wire) => {
                let Ok(keys) = self.keys(&from) else { return };
                if let Ok(Opening::Opened(body)) = target.sealer.open_peer(&from, wire, Some(&keys))
                {
                    let delivery = PeerDelivery {
                        from_instance: from,
                        target_instance: to.to_string(),
                        body,
                    };
                    for handler in target.peer.lock().unwrap().iter() {
                        handler(delivery.clone());
                    }
                }
            }
            Held::Event(event) => {
                for handler in target.events.lock().unwrap().iter() {
                    handler(event.clone());
                }
            }
        }
    }

    /// Opens the current attempt of `task_id` as `provider` does and
    /// hands it over
    fn hand_over(&self, provider: &Arc<MockRelay>, task_id: &str) {
        let sealed = {
            let tasks = self.tasks.lock().unwrap();
            let Some(carried) = tasks.get(task_id) else {
                return;
            };
            let (target, envelope) = carried.attempts[carried.current].clone();
            (
                SealedTask {
                    id: task_id.to_string(),
                    requester: carried.requester_card.clone(),
                    target,
                    capability: carried.capability.clone(),
                    created_at: carried.created_at,
                    deadline: carried.deadline,
                    envelope,
                },
                carried.requester.clone(),
            )
        };
        let (sealed, requester) = sealed;
        let Ok(keys) = self.keys(&requester) else {
            return;
        };
        let Ok(Opening::Opened(opened)) = provider.sealer.open_task(sealed, &keys) else {
            return;
        };
        provider
            .delegated_here
            .lock()
            .unwrap()
            .push(task_id.to_string());
        for handler in provider.assigned.lock().unwrap().iter() {
            handler(TaskAssigned {
                task_id: task_id.to_string(),
                provider_instance: provider.instance.clone(),
                task: opened.clone(),
            });
        }
    }

    /// Gives `task_id` to its next candidate, as the relay does after a
    /// refusal; false when there's none
    fn fail_over(&self, task_id: &str, reason_kind: Option<task::ReasonKind>) -> bool {
        let moved = {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(carried) = tasks.get_mut(task_id) else {
                return false;
            };
            if carried.current + 1 >= carried.attempts.len() {
                return false;
            }
            let from = carried.provider();
            carried.current += 1;
            (
                from,
                carried.provider(),
                carried.attempts[carried.current].0.clone(),
                carried.requester.clone(),
            )
        };
        let (from, to, target, requester) = moved;
        self.post(
            &to,
            Held::Task {
                task_id: task_id.to_string(),
            },
        );
        self.post(
            &requester,
            Held::Event(crate::services::relay::protocol::TaskEvent::FailedOver {
                task_id: task_id.to_string(),
                from_instance: from,
                provider_instance: to,
                target,
                cause: nandi_protocol::FailoverCause::Rejected,
                reason_kind,
            }),
        );
        true
    }

    /// Ends `task_id` for both sides, as the relay does past its due date
    fn end(&self, task_id: &str, state: TaskState) {
        let parties = {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(carried) = tasks.remove(task_id) else {
                return;
            };
            (carried.requester.clone(), carried.provider())
        };
        let event = crate::services::relay::protocol::TaskEvent::Ended {
            task_id: task_id.to_string(),
            state,
            reason_kind: None,
        };
        self.post(&parties.1, Held::Event(event.clone()));
        self.post(&parties.0, Held::Event(event));
    }

    /// The other side of `task_id` for `from`; `None` when `from` isn't a
    /// side of it (any more)
    fn other_side(&self, task_id: &str, from: &str) -> Option<String> {
        let tasks = self.tasks.lock().unwrap();
        let carried = tasks.get(task_id)?;
        let provider = carried.provider();
        if from == carried.requester {
            Some(provider)
        } else if from == provider {
            Some(carried.requester.clone())
        } else {
            None
        }
    }
}

pub(crate) struct MockRelay {
    instance: String,
    hub: Arc<Hub>,
    cards: Mutex<Vec<AgentCard>>,
    assigned: Mutex<Vec<Handler<TaskAssigned>>>,
    completed: Mutex<Vec<Handler<TaskCompleted>>>,
    failed: Mutex<Vec<Handler<TaskFailed>>>,
    peer: Mutex<Vec<Handler<PeerDelivery>>>,
    events: Mutex<Vec<Handler<crate::services::relay::protocol::TaskEvent>>>,
    /// Every `send_peer`: (target, body)
    sent_peer: Mutex<Vec<(String, PeerMessage)>>,
    /// Task IDs this client submitted, in order
    delegated: Mutex<Vec<String>>,
    /// Task IDs handed to this instance, in order
    delegated_here: Mutex<Vec<String>>,
    /// Whether this client counts as connected (an outbox test turns it
    /// off)
    connected: std::sync::atomic::AtomicBool,
    /// This instance's keys, sealing and opening as the real client does
    sealer: Sealer,
}

impl MockRelay {
    fn join(hub: &Arc<Hub>, instance: &str) -> Arc<Self> {
        let relay = Arc::new(Self {
            instance: instance.to_string(),
            hub: hub.clone(),
            cards: Mutex::default(),
            assigned: Mutex::default(),
            completed: Mutex::default(),
            failed: Mutex::default(),
            peer: Mutex::default(),
            events: Mutex::default(),
            sent_peer: Mutex::default(),
            delegated: Mutex::default(),
            delegated_here: Mutex::default(),
            connected: std::sync::atomic::AtomicBool::new(true),
            sealer: Sealer::new(instance, nandi_seal::DeviceKeys::generate().unwrap()),
        });
        hub.relays
            .lock()
            .unwrap()
            .insert(instance.to_string(), Arc::downgrade(&relay));
        relay
    }

    fn set_connected(&self, connected: bool) {
        self.connected
            .store(connected, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl RelayClient for MockRelay {
    fn connect<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn register(&self, cards: Vec<AgentCard>) -> BoxFuture<'_, Result<(), String>> {
        *self.cards.lock().unwrap() = cards;
        Box::pin(async { Ok(()) })
    }

    fn heartbeat(&self, _: HashMap<String, u32>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn query_capability<'a>(
        &'a self,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
        let relays: Vec<_> = self.hub.relays.lock().unwrap().values().cloned().collect();
        let cards = relays
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|relay| relay.instance != self.instance)
            .filter(|relay| !self.hub.offline.lock().unwrap().contains(&relay.instance))
            .flat_map(|relay| {
                let cards = relay.cards.lock().unwrap().clone();
                cards
            })
            .filter(|card| capability == ALL_CAPABILITIES || card.serves(capability))
            .collect();
        Box::pin(async move { Ok(cards) })
    }

    /// A stand-in for the relay's plan (the ranking is the relay's, see
    /// `nandi-relay`): a named card alone while its instance is online;
    /// else every online card that serves the capability and takes
    /// requests, not excluded, pinned first, then free before full, then
    /// by card ID
    fn route<'a>(
        &'a self,
        query: &'a crate::services::relay::protocol::RoutingQuery,
    ) -> BoxFuture<'a, Result<crate::services::relay::protocol::RoutePlan, String>> {
        use crate::services::relay::protocol::{
            RouteCandidate, RoutePlan, RouteReason, RouteTarget,
        };
        let relays: Vec<_> = self.hub.relays.lock().unwrap().values().cloned().collect();
        let offline = self.hub.offline.lock().unwrap().clone();
        let mut cards: Vec<AgentCard> = relays
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|relay| relay.instance != self.instance && !offline.contains(&relay.instance))
            .flat_map(|relay| relay.cards.lock().unwrap().clone())
            .collect();
        let hints = &query.hints;
        let listed = |list: &[String], card: &AgentCard| {
            list.iter()
                .any(|item| *item == card.id || *item == card.instance_id)
        };
        match &query.target {
            RouteTarget::Agent { card_id } => cards.retain(|card| card.id == *card_id),
            RouteTarget::Person { instance_id } => {
                cards.retain(|card| card.instance_id == *instance_id)
            }
            RouteTarget::Anyone => cards.retain(|card| {
                card.serves(&query.capability)
                    && card.accepts_delegation
                    && !listed(&hints.excluded, card)
            }),
        }
        cards.sort_by(|a, b| {
            let full = |card: &AgentCard| card.current_load >= card.max_concurrent.max(1);
            (!listed(&hints.preferred, a))
                .cmp(&!listed(&hints.preferred, b))
                .then(full(a).cmp(&full(b)))
                .then(a.id.cmp(&b.id))
        });
        let candidates = cards
            .into_iter()
            .take(hints.max_candidates.unwrap_or(3) as usize)
            .map(|card| RouteCandidate {
                target: card.id.clone(),
                instance_id: card.instance_id.clone(),
                name: card.owner.display_name.clone(),
                handle: Some(card.owner.github_handle.clone()).filter(|h| !h.is_empty()),
                agent_name: card.name.clone(),
                provider_kind: Default::default(),
                online: true,
                load: card.current_load,
                max_concurrent: card.max_concurrent,
                wait_seconds: None,
                redirected_from: None,
                reasons: vec![RouteReason::Free {
                    name: card.owner.display_name.clone(),
                }],
                public_keys: None,
            })
            .collect();
        Box::pin(async move {
            Ok(RoutePlan {
                plan_id: "mock".into(),
                candidates,
                ..RoutePlan::default()
            })
        })
    }

    fn delegate_task<'a>(
        &'a self,
        task: &'a Task,
        alternates: &'a [String],
    ) -> BoxFuture<'a, Result<Submitted, String>> {
        Box::pin(async move {
            if !self.is_connected() {
                return Err(crate::services::relay::RelayUnavailable.to_string());
            }
            let mut attempts = Vec::new();
            for target in std::iter::once(&task.target).chain(alternates) {
                let provider = instance_of(target).to_string();
                let keys = self.hub.keys(&provider)?;
                attempts.push((
                    target.clone(),
                    self.sealer.seal_task(task, (&provider, &keys))?,
                ));
            }
            let provider = instance_of(&task.target).to_string();
            {
                let mut tasks = self.hub.tasks.lock().unwrap();
                if tasks.contains_key(&task.id) {
                    return Ok(Submitted {
                        held: self.hub.relay(&provider).is_none(),
                    });
                }
                tasks.insert(
                    task.id.clone(),
                    Carried {
                        requester: self.instance.clone(),
                        requester_card: task.requester.clone(),
                        capability: task.capability.clone(),
                        created_at: task.created_at,
                        deadline: task.deadline,
                        attempts,
                        current: 0,
                    },
                );
            }
            self.delegated.lock().unwrap().push(task.id.clone());
            let held = self.hub.relay(&provider).is_none();
            self.hub.post(
                &provider,
                Held::Task {
                    task_id: task.id.clone(),
                },
            );
            Ok(Submitted { held })
        })
    }

    fn send_result(&self, result: TaskResult) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            // The relay knows who asked
            let requester = self
                .hub
                .other_side(&result.task_id, &self.instance)
                .ok_or("No route back")?;
            let report = self
                .sealer
                .seal_result(&result, (&requester, &self.hub.keys(&requester)?))?;
            self.hub.tasks.lock().unwrap().remove(&result.task_id);
            self.hub
                .post(&requester, Held::Result(report, self.instance.clone()));
            Ok(())
        })
    }

    fn on_task_assigned(&self, handler: Handler<TaskAssigned>) -> BoxFuture<'_, ()> {
        self.assigned.lock().unwrap().push(handler);
        Box::pin(async {})
    }

    fn on_task_completed(&self, handler: Handler<TaskCompleted>) -> BoxFuture<'_, ()> {
        self.completed.lock().unwrap().push(handler);
        Box::pin(async {})
    }

    fn on_task_failed(&self, handler: Handler<TaskFailed>) -> BoxFuture<'_, ()> {
        self.failed.lock().unwrap().push(handler);
        Box::pin(async {})
    }

    fn on_task_event(
        &self,
        handler: Handler<crate::services::relay::protocol::TaskEvent>,
    ) -> BoxFuture<'_, ()> {
        self.events.lock().unwrap().push(handler);
        Box::pin(async {})
    }

    fn status(&self) -> RelayStatus {
        RelayStatus {
            connected: self.is_connected(),
            instance_id: self.instance.clone(),
            relay_url: Some("wss://relay.test/relay".into()),
            agents_online: 0,
            ..RelayStatus::default()
        }
    }

    fn send_peer(
        &self,
        target_instance: String,
        body: PeerMessage,
    ) -> BoxFuture<'_, Result<(), String>> {
        self.sent_peer
            .lock()
            .unwrap()
            .push((target_instance.clone(), body.clone()));
        Box::pin(async move {
            let task_id = body.task_id().to_string();
            if self.hub.other_side(&task_id, &self.instance).as_deref() != Some(&target_instance) {
                return Err(format!("{} isn't a side of task {task_id}", self.instance));
            }
            // A refusal goes to the next candidate while there is one
            if let PeerMessage::RejectTask { reason_kind, .. } = &body {
                let lasting = self
                    .hub
                    .tasks
                    .lock()
                    .unwrap()
                    .get(&task_id)
                    .is_some_and(|c| assignment::is_lasting(&c.capability));
                if !lasting
                    && *reason_kind != Some(task::ReasonKind::Declined)
                    && self.hub.fail_over(&task_id, *reason_kind)
                {
                    return Ok(());
                }
            }
            let keys = if Sealer::has_content(&body) {
                Some(self.hub.keys(&target_instance)?)
            } else {
                None
            };
            let live_only = matches!(
                body,
                PeerMessage::StatusQuery { .. } | PeerMessage::StatusResponse { .. }
            );
            let wire = self
                .sealer
                .seal_peer(&target_instance, body, keys.as_ref())?;
            if live_only && self.hub.relay(&target_instance).is_none() {
                return Ok(());
            }
            self.hub
                .post(&target_instance, Held::Peer(self.instance.clone(), wire));
            Ok(())
        })
    }

    fn on_peer(&self, handler: Handler<PeerDelivery>) -> BoxFuture<'_, ()> {
        self.peer.lock().unwrap().push(handler);
        Box::pin(async {})
    }

    /// The relay's move, with the request resealed for the new provider:
    /// never back to the requester or to anyone who had it
    fn reassign<'a>(
        &'a self,
        task_id: &'a str,
        requester: &'a str,
        target: &'a str,
        by: &'a str,
        reason: Option<String>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let to = instance_of(target).to_string();
            let keys = self.hub.keys(&to)?;
            let request = self
                .sealer
                .reseal_request(task_id, (&to, &keys))?
                .ok_or("The request isn't held here")?;
            let from = {
                let mut tasks = self.hub.tasks.lock().unwrap();
                let carried = tasks.get_mut(task_id).ok_or("No such task")?;
                if carried.provider() != self.instance {
                    return Err("Only the task's provider can hand it on".to_string());
                }
                let had = carried.attempts[..=carried.current]
                    .iter()
                    .any(|(card, _)| instance_of(card) == to);
                if to == carried.requester || had {
                    return Err(format!("{to} already had this task"));
                }
                carried.attempts.truncate(carried.current + 1);
                carried.attempts.push((target.to_string(), request));
                carried.current += 1;
                self.instance.clone()
            };
            self.hub.post(
                &to,
                Held::Task {
                    task_id: task_id.to_string(),
                },
            );
            self.hub.post(
                requester,
                Held::Event(crate::services::relay::protocol::TaskEvent::Reassigned {
                    task_id: task_id.to_string(),
                    from_instance: from,
                    provider_instance: to.clone(),
                    target: target.to_string(),
                    provider_name: Some(to),
                    by: by.to_string(),
                    note: reason,
                }),
            );
            Ok(())
        })
    }
}

impl MockRelay {
    /// Carries `task` (ours) as if it had been submitted earlier: the
    /// relay knows where its result goes
    fn track(&self, task: &Task) {
        let provider = instance_of(&task.target).to_string();
        let keys = self.hub.keys(&provider).unwrap();
        let envelope = self.sealer.seal_task(task, (&provider, &keys)).unwrap();
        self.hub.tasks.lock().unwrap().insert(
            task.id.clone(),
            Carried {
                requester: self.instance.clone(),
                requester_card: task.requester.clone(),
                capability: task.capability.clone(),
                created_at: task.created_at,
                deadline: task.deadline,
                attempts: vec![(task.target.clone(), envelope)],
                current: 0,
            },
        );
    }

    /// What `send_peer` sent, by message kind (`acknowledge_task`, …)
    fn sent_kinds(&self) -> Vec<String> {
        self.sent_peer
            .lock()
            .unwrap()
            .iter()
            .map(|(_, body)| {
                serde_json::to_value(body).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    fn delegated(&self) -> Vec<String> {
        self.delegated.lock().unwrap().clone()
    }
}

/// Two app instances on one mock relay: a requester with `writer`, and a
/// provider with `papers` serving `summarize`.
struct TwoInstances {
    requester_state: AppState,
    provider_state: AppState,
    requester: Arc<Delegator>,
    _provider: Arc<Delegator>,
    _relays: (Arc<MockRelay>, Arc<MockRelay>),
    _log_dir: tempfile::TempDir,
    hub: Arc<Hub>,
    /// Requester's and provider's task services (the registry holds them
    /// weakly)
    _services: (Arc<task::TaskService>, Arc<task::TaskService>),
}

async fn two_instances(provider_frontmatter: &str) -> TwoInstances {
    let requester_state = AppState::for_tests(vec![]);
    let provider_state = AppState::for_tests(vec![]);
    echo_agents(&provider_state).await;
    save_agent(&requester_state, "writer", "delegation_outbound: true\n");
    save_agent(&provider_state, "papers", provider_frontmatter);

    let hub = Arc::new(Hub::default());
    let requester_relay = MockRelay::join(&hub, &instance(&requester_state));
    // Each side's task service on the mock relay (replacing the state's)
    let requester_service = lifecycle(
        &requester_state,
        requester_relay.clone(),
        Default::default(),
    )
    .await;
    let provider_relay = MockRelay::join(&hub, &instance(&provider_state));
    let provider_service =
        lifecycle(&provider_state, provider_relay.clone(), Default::default()).await;
    let log_dir = tempfile::tempdir().unwrap();
    let requester = Delegator::new(A2aDeps {
        audit: AuditLog::at(log_dir.path().join("requester.log")),
        ..deps(&requester_state, Some(requester_relay.clone()))
    });
    let provider = Delegator::new(A2aDeps {
        audit: AuditLog::at(log_dir.path().join("provider.log")),
        ..deps(&provider_state, Some(provider_relay.clone()))
    });
    requester.clone().register_relay_handlers().await;
    provider.clone().register_relay_handlers().await;
    provider_relay
        .register(provider.local_cards(""))
        .await
        .unwrap();
    TwoInstances {
        requester_state,
        provider_state,
        requester,
        _provider: provider,
        _relays: (requester_relay, provider_relay),
        _log_dir: log_dir,
        hub,
        _services: (requester_service, provider_service),
    }
}

/// The user's requester card on `instance`
pub(crate) fn user_card(instance: &str) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(instance, delegator::USER_REQUESTER),
        name: "Nandi user".into(),
        description: String::new(),
        instance_id: instance.into(),
        endpoint: String::new(),
        capabilities: Vec::new(),
        accepts_delegation: false,
        max_concurrent: 0,
        current_load: 0,
        owner: Default::default(),
        signature: None,
        issued_at: 1.0,
    }
}

fn from_writer(capability: &str, input: Value) -> DelegationRequest {
    DelegationRequest {
        requester_agent: Some("writer".into()),
        capability: capability.into(),
        input,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_instance_delegates_to_another_through_the_relay() {
    let world = two_instances("serves: [{name: summarize}]\naccepts_delegation: true\n").await;
    trust::set_policy(
        world.provider_state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Allow,
    )
    .unwrap();
    let requester_instance = instance(&world.requester_state);
    let provider_instance = instance(&world.provider_state);

    let providers = world.requester.providers("summarize").await;
    assert_eq!(providers.len(), 1, "the provider's card, through the relay");
    assert_eq!(providers[0].id, format!("{provider_instance}/papers"));

    let result = world
        .requester
        .run(from_writer(
            "summarize",
            json!({"text": "Attention is all you need"}),
        ))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.direction, DelegationDirection::Outbound);
    assert_eq!(result.provider_agent, format!("{provider_instance}/papers"));
    assert_eq!(
        result.requester_agent,
        format!("{requester_instance}/writer")
    );
    let output = result.output.as_str().unwrap();
    assert!(output.contains("Attention is all you need"), "{output}");

    // Both sides keep history
    let asked = history::get(world.requester_state.db(), &result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked.status, TaskStatus::Completed);
    let served = history::list(world.provider_state.db(), 10).await.unwrap();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].direction, DelegationDirection::Inbound);
    assert_eq!(served[0].status, TaskStatus::Completed);
    assert_eq!(served[0].requester_agent, result.requester_agent);

    // The provider ran it as a delegated turn
    let source: (String, Option<String>) = world
        .provider_state
        .db()
        .call(|conn| {
            conn.query_row(
                "SELECT source_type, source_id FROM runs WHERE agent_id = 'papers'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(source.0, "agent");
    assert_eq!(source.1.as_deref(), Some(result.requester_agent.as_str()));

    // Audit: started + finished on each side
    let log = std::fs::read_to_string(world._log_dir.path().join("provider.log")).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert!(!log.contains("Attention"), "no task content in the log");
    world.requester_state.shutdown();
    world.provider_state.shutdown();
}

#[tokio::test]
async fn remote_requesters_need_an_allowing_trust_policy() {
    let world = two_instances("serves: [{name: summarize}]\naccepts_delegation: true\n").await;
    // No rule: `ask`, which rejects
    let result = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result.error.as_deref().unwrap().contains("needs to trust"),
        "{:?}",
        result.error
    );

    trust::set_policy(
        world.provider_state.db(),
        &result.requester_agent,
        "papers",
        trust::TrustPolicy::Deny,
    )
    .unwrap();
    let denied = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert!(denied.error.unwrap().contains("doesn't take requests from"));
    world.requester_state.shutdown();
    world.provider_state.shutdown();
}

#[tokio::test]
async fn agents_that_dont_accept_delegation_reject_tasks() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let delegator = state.a2a_delegator().unwrap().clone();

    // Scoring skips it. Not "free right now": it isn't busy, it doesn't
    // take delegation
    let result = delegator.delegate("summarize", json!("hi")).await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    // In plain words: no YAML key in the advice (task 5)
    let error = result.error.unwrap();
    assert!(
        error.contains("don't take requests from other Nandis"),
        "{error}"
    );
    assert!(!error.contains("accepts_delegation"), "{error}");

    // Asked by name, it refuses
    let target = format!("{}/papers", instance(&state));
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap()
            .contains("doesn't take requests from other Nandis"),
        "{:?}",
        result.error
    );
    let rows = history::list(state.db(), 10).await.unwrap();
    assert_eq!(rows.len(), 2, "failures are recorded too");
    state.shutdown();
}

#[tokio::test]
async fn an_override_can_open_or_close_one_capability() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}, {name: translate}]\naccepts_delegation: true\n\
         delegation_overrides: {translate: deny}\n",
    );
    let delegator = state.a2a_delegator().unwrap().clone();
    let target = format!("{}/papers", instance(&state));
    let denied = delegator
        .delegate_to(&target, "translate", json!("hi"))
        .await
        .unwrap();
    assert!(denied.error.unwrap().contains("doesn't take 'translate'"));
    let done = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error);
    state.shutdown();
}

#[tokio::test]
async fn local_delegation_works_without_a_relay() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    // A relay client may exist; with no relay URL it never connects
    assert!(!state.relay_client().is_some_and(|c| c.status().connected));

    let providers = list_available_providers(&state, "summarize").await.unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(list_available_providers(&state, "").await.unwrap().len(), 1);
    assert!(list_available_providers(&state, "other")
        .await
        .unwrap()
        .is_empty());

    let task_id = delegate_task(&state, "summarize", json!({"text": "hello"}), None)
        .await
        .unwrap();
    assert_eq!(
        delegation(&state, &task_id).await.status,
        TaskStatus::Running
    );
    wait_until("the delegation", || {
        let state = state.clone();
        let task_id = task_id.clone();
        async move { delegation(&state, &task_id).await.status != TaskStatus::Running }
    })
    .await;
    let result = delegation(&state, &task_id).await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert!(result.output.as_str().unwrap().contains("hello"));
    assert!(result.requester_agent.ends_with("/_user"));

    let history = list_delegation_history(&state, 50).await.unwrap();
    assert_eq!(history.len(), 1, "one row for an in-process delegation");
    let names = recorder.names();
    assert!(names.contains(&"delegation://task_started"));
    assert!(names.contains(&"delegation://task_completed"));

    // A capability only others serve needs the relay's plan: not connected
    let delegator = state.a2a_delegator().unwrap().clone();
    let result = delegator.delegate("translate", json!("hi")).await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    let error = result.error.unwrap();
    assert!(
        crate::services::relay::unavailable::RelayUnavailable::is(&error),
        "{error}"
    );
    state.shutdown();
}

#[tokio::test]
async fn the_delegation_tool_waits_for_approval() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    canned(
        &state,
        json!([
            {
                "text": "Let me ask a colleague.",
                "toolCalls": [{
                    "tool": "delegate_to_agent",
                    "args": {"capability": "summarize", "input": {"text": "hello"}},
                    "result": "unused"
                }]
            },
            { "text": "[{{input}}]" }
        ]),
    )
    .await;
    save_agent(&state, "writer", "delegation_outbound: true\n");
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );

    let sent = chat::send_message(&state, "writer".into(), "Summarize".into(), vec![], None)
        .await
        .unwrap();
    let pending = || {
        let db = state.db().clone();
        async move {
            db.call(|conn| db::pending_approvals(conn, Some("writer")))
                .await
                .unwrap()
        }
    };
    wait_until("the approval request", || async move {
        !pending().await.is_empty()
    })
    .await;
    let approval = pending().await.remove(0);
    assert_eq!(approval.tool, "delegate_to_agent");
    assert!(
        history::list(state.db(), 10).await.unwrap().is_empty(),
        "nothing is delegated before approval"
    );

    chat::approve_action(&state, None, approval.id, ApprovalDecision::Approved)
        .await
        .unwrap();
    let run_id = sent.run_id.clone();
    wait_until("the writer's run", || {
        let db = state.db().clone();
        let id = run_id.clone();
        async move {
            db.call(move |conn| db::get_run(conn, &id))
                .await
                .unwrap()
                .is_some_and(|run| run.status.is_finished())
        }
    })
    .await;
    let id = sent.run_id.clone();
    let run = state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let rows = history::list(state.db(), 10).await.unwrap();
    assert_eq!(rows.len(), 1, "papers (no outbound) couldn't delegate");
    assert_eq!(rows[0].status, TaskStatus::Completed, "{:?}", rows[0].error);
    assert!(rows[0].provider_agent.ends_with("/papers"));
    assert!(rows[0].requester_agent.ends_with("/writer"));

    // The tool result carries the provider's answer back to the writer
    let id = sent.run_id.clone();
    let messages = state
        .db()
        .call(move |conn| db::list_run_messages(conn, &id))
        .await
        .unwrap();
    let tool_result = messages
        .iter()
        .find(|m| m.role == db::role::TOOL_CALL)
        .expect("a tool call message");
    assert!(
        tool_result.metadata.to_string().contains("hello") || tool_result.content.contains("hello"),
        "{tool_result:?}"
    );
    state.shutdown();
}

#[tokio::test]
async fn declaring_approvals_without_the_tool_skips_approval() {
    let state = AppState::for_tests(vec![]);
    save_agent(
        &state,
        "auto",
        "delegation_outbound: true\nrequires_approval: [shell]\n",
    );
    save_agent(&state, "careful", "delegation_outbound: true\n");
    save_agent(&state, "plain", "");
    let gate = |id: &str| tool::policy(state.store(), id);
    assert!(gate("auto").offered && !gate("auto").requires_approval_for(tool::NAME));
    assert!(gate("careful").requires_approval_for(tool::NAME));
    assert!(!gate("careful").requires_approval_for("shell"));
    assert!(!gate("plain").offered);
    state.shutdown();
}

#[tokio::test]
async fn a_remote_request_without_an_answer_stays_open_until_it_is_due() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    let hub = Arc::new(Hub::default());
    let relay = MockRelay::join(&hub, &instance(&state));
    // Online, but nothing on it answers within the live wait
    let _silent = MockRelay::join(&hub, "silent");
    let delegator = Delegator::new(A2aDeps {
        timeout: Duration::from_millis(150),
        ..deps(&state, Some(relay.clone()))
    });
    delegator.clone().register_relay_handlers().await;
    let service = lifecycle(&state, relay.clone(), task::TaskConfig::default()).await;

    let started = crate::db::now_ms();
    let result = delegator
        .delegate_to("silent/papers", "summarize", json!("hi"))
        .await
        .unwrap();
    // The live wait ended without an answer: still open, waiting
    assert_eq!(result.status, TaskStatus::Running);
    assert_eq!(result.output["waiting"], json!(true));
    let row = task::get_row(state.db(), &result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!row.state.is_terminal(), "{:?}", row.state);
    let due = row.deadline.unwrap() as i64;
    assert!(due >= started + assignment::DEFAULT_REQUEST_DUE_DAYS * 86_400_000);
    assert_eq!(
        delegation(&state, &result.task_id).await.status,
        TaskStatus::Running
    );

    // The relay ends it past its due date: so does its history row
    let _ = &service;
    hub.end(&result.task_id, TaskState::TimedOut);
    wait_until("the end", || async {
        delegation(&state, &result.task_id).await.status == TaskStatus::TimedOut
    })
    .await;
    let closed = delegation(&state, &result.task_id).await;
    assert_eq!(closed.status, TaskStatus::TimedOut);
    assert!(closed.error.unwrap().contains("due date"));
    assert!(recorder.names().contains(&"delegation://task_failed"));
    state.shutdown();
}

#[tokio::test]
async fn an_answer_that_comes_after_the_live_wait_is_kept() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    let hub = Arc::new(Hub::default());
    let relay = MockRelay::join(&hub, &instance(&state));
    let late = MockRelay::join(&hub, "late");
    let delegator = Delegator::new(A2aDeps {
        timeout: Duration::from_millis(150),
        ..deps(&state, Some(relay.clone()))
    });
    delegator.clone().register_relay_handlers().await;
    let _service = lifecycle(&state, relay.clone(), task::TaskConfig::default()).await;

    let result = delegator
        .delegate_to("late/papers", "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Running);

    // The answer arrives later, when nobody waits on it live
    let row = task::get_row(state.db(), &result.task_id)
        .await
        .unwrap()
        .unwrap();
    let answer = TaskResult {
        task_id: row.id.clone(),
        provider_agent: "late/papers".into(),
        output: json!({"summary": "done later"}),
        status: TaskStatus::Completed,
        duration_ms: 1,
        tokens_in: 0,
        tokens_out: 0,
        error: None,
        direction: DelegationDirection::Inbound,
        capability: "summarize".into(),
        requester_agent: row.requester_agent.clone(),
        provider_owner: None,
        requester_owner: None,
        input: json!("hi"),
        started_at: row.created_at,
        ended_at: Some(row.created_at + 1.0),
    };
    late.send_result(answer).await.unwrap();
    let ended = wait_for_history_end(&state, &result.task_id).await;
    assert_eq!(ended.status, TaskStatus::Completed);
    assert_eq!(ended.output["summary"], "done later");
    assert!(recorder.names().contains(&"delegation://task_completed"));
    // It's an Inbox item, and the companion hears about it
    let returns = returns::list(state.db()).await.unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].outcome, returns::ReturnOutcome::Answered);
    assert_eq!(returns[0].summary, "done later");
    assert!(recorder.names().contains(&"request://returned"));
    state.shutdown();
}

#[tokio::test]
async fn settings_are_written_to_skill_md() {
    let state = AppState::for_tests(vec![]);
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let settings = AgentDelegationSettings {
        allow_outbound: true,
        accept_inbound: true,
        max_concurrent: Some(2),
        overrides: vec![DelegationOverride {
            capability: "summarize".into(),
            policy: TrustPolicy::Allow,
        }],
    };
    set_agent_delegation(&state, "papers", settings.clone())
        .await
        .unwrap();
    assert_eq!(
        get_agent_delegation(&state, "papers").await.unwrap(),
        settings
    );
    let md = std::fs::read_to_string(state.store().layout().skill_path("papers")).unwrap();
    assert!(md.contains("accepts_delegation: true"), "{md}");
    assert!(md.contains("delegation_outbound: true"), "{md}");
    assert!(state
        .store()
        .agent("papers")
        .unwrap()
        .capabilities()
        .accepts_delegation());
    assert!(get_agent_delegation(&state, "nope").await.is_err());
    state.shutdown();
}

#[tokio::test]
async fn a_rejected_pick_falls_back_to_the_next_provider() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "closed",
        "serves: [{name: summarize}]\naccepts_delegation: true\n\
         delegation_overrides: {summarize: deny}\n",
    );
    save_agent(
        &state,
        "open",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let me = instance(&state);
    // Pinned, so it's tried first; it rejects the task without running it
    crate::services::shared::set_provider_pin(&state, "summarize", Some(&format!("{me}/closed")))
        .await
        .unwrap();

    let overview = crate::services::shared::list_capability_providers(&state, Some("summarize"))
        .await
        .unwrap();
    assert_eq!(overview.len(), 1);
    assert_eq!(
        overview[0].pinned.as_deref(),
        Some(format!("{me}/closed").as_str())
    );
    assert_eq!(overview[0].auto_choice, overview[0].pinned);
    assert_eq!(overview[0].providers.len(), 2);

    let delegator = state.a2a_delegator().unwrap().clone();
    let result = delegator.delegate("summarize", json!("hi")).await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.provider_agent, format!("{me}/open"));

    let history = list_delegation_history(&state, 50).await.unwrap();
    assert_eq!(history.len(), 2, "the final row and the rejected attempt");
    let attempt = history
        .iter()
        .find(|row| row.task_id.ends_with("-attempt1"))
        .expect("the rejected attempt keeps a row");
    assert_eq!(attempt.provider_agent, format!("{me}/closed"));
    assert!(attempt
        .error
        .as_deref()
        .unwrap()
        .contains("passed to the next provider"));
    state.shutdown();
}

#[tokio::test]
async fn a_named_provider_is_never_swapped() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "closed",
        "serves: [{name: summarize}]\naccepts_delegation: true\n\
         delegation_overrides: {summarize: deny}\n",
    );
    save_agent(
        &state,
        "open",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let target = format!("{}/closed", instance(&state));
    let delegator = state.a2a_delegator().unwrap().clone();
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert_eq!(result.provider_agent, target);
    assert_eq!(list_delegation_history(&state, 50).await.unwrap().len(), 1);
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Task lifecycle (wave final, Track B)
// ---------------------------------------------------------------------------

mod lifecycle_tests {
    use super::*;
    use crate::services::a2a::task::{
        self, TaskFilter, TaskRecord, TaskService, TaskSourceKind, TaskState,
    };
    use crate::services::a2a::task_state;

    const SERVES: &str = "serves: [{name: summarize}]\naccepts_delegation: true\n";

    fn allow(world: &TwoInstances) {
        trust::set_policy(
            world.provider_state.db(),
            trust::ANY_REQUESTER,
            "papers",
            trust::TrustPolicy::Allow,
        )
        .unwrap();
    }

    fn wire(id: &str, requester_instance: &str, target: &str, input: Value) -> Task {
        Task {
            id: id.into(),
            requester: user_card(requester_instance),
            target: target.into(),
            capability: "summarize".into(),
            input,
            created_at: db::now_ms() as f64,
            deadline: None,
        }
    }

    fn serving(id: &str) -> AgentCard {
        let (instance, agent) = id.split_once('/').unwrap();
        AgentCard {
            id: id.into(),
            name: agent.into(),
            capabilities: vec![crate::services::domain::capability::ServesCapability {
                name: "summarize".into(),
                description: String::new(),
                input_schema: None,
                output_schema: None,
            }],
            accepts_delegation: true,
            max_concurrent: 2,
            ..user_card(instance)
        }
    }

    fn states(events: &[task::TaskEvent]) -> Vec<TaskState> {
        events.iter().map(|e| e.to_state).collect()
    }

    async fn row(state: &AppState, id: &str) -> TaskRecord {
        task::get_row(state.db(), id).await.unwrap().unwrap()
    }

    async fn state_of(state: &AppState, id: &str) -> Option<TaskState> {
        task::get_row(state.db(), id)
            .await
            .unwrap()
            .map(|r| r.state)
    }

    fn service(state: &AppState) -> Arc<TaskService> {
        task::service_for(&instance(state)).unwrap()
    }

    /// One requester state on a hub, with "them" joined (online)
    async fn requester_alone(
        recorder: Option<Arc<RecordingSink>>,
    ) -> (
        AppState,
        Arc<Hub>,
        Arc<MockRelay>,
        Arc<TaskService>,
        Arc<MockRelay>,
    ) {
        let sinks: Vec<Arc<dyn crate::services::events::EventSink>> = recorder
            .map(|r| vec![r as Arc<dyn crate::services::events::EventSink>])
            .unwrap_or_default();
        let state = AppState::for_tests(sinks);
        let hub = Arc::new(Hub::default());
        let relay = MockRelay::join(&hub, &instance(&state));
        let them = MockRelay::join(&hub, "them");
        let service = lifecycle(&state, relay.clone(), Default::default()).await;
        (state, hub, relay, service, them)
    }

    #[tokio::test]
    async fn a_cross_instance_delegation_walks_the_lifecycle_on_both_sides() {
        let world = two_instances(SERVES).await;
        allow(&world);
        let provider = instance(&world.provider_state);
        let result = world
            .requester
            .run(from_writer("summarize", json!({"text": "hi"})))
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);

        let asked = row(&world.requester_state, &result.task_id).await;
        assert_eq!(asked.state, TaskState::Completed);
        assert_eq!(asked.requester_agent, "writer");
        assert_eq!(asked.provider_instance, provider);
        assert_eq!(asked.provider_agent, "papers");
        assert_eq!(asked.attempt_number, 1);
        assert_eq!(asked.source_kind, TaskSourceKind::Direct);
        assert!(asked.result.is_some());
        let path = states(
            &task::events_of(world.requester_state.db(), &result.task_id)
                .await
                .unwrap(),
        );
        assert_eq!(
            &path[..3],
            [
                TaskState::Submitted,
                TaskState::Delivered,
                TaskState::Acknowledged
            ]
        );
        assert_eq!(path.last(), Some(&TaskState::Completed));

        let served = row(&world.provider_state, &result.task_id).await;
        assert_eq!(served.state, TaskState::Completed);
        assert_eq!(served.provider_instance, provider);
        assert!(served.eta_seconds.is_some(), "acknowledged with an ETA");
        let path = states(
            &task::events_of(world.provider_state.db(), &result.task_id)
                .await
                .unwrap(),
        );
        assert_eq!(
            path,
            [
                TaskState::Submitted,
                TaskState::Delivered,
                TaskState::Acknowledged,
                TaskState::Working,
                TaskState::Completed
            ]
        );
        let samples: u32 = world
            .provider_state
            .db()
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM eta_history", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(samples, 1, "the completion feeds the ETA");
        let kinds = world._relays.1.sent_kinds();
        assert!(kinds.contains(&"task_delivered".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"acknowledge_task".to_string()), "{kinds:?}");
        // One "started" update always: it's how the requester sees `Working`
        assert_eq!(
            kinds.iter().filter(|k| *k == "progress_update").count(),
            1,
            "{kinds:?}"
        );
        // The delegation history is unchanged
        assert_eq!(
            history::get(world.requester_state.db(), &result.task_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Completed
        );
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn a_gateway_task_gets_a_provider_row_with_the_routing_reason() {
        let world = two_instances(SERVES).await;
        let provider = instance(&world.provider_state);
        let gateway = AgentCard::card_id(&provider, crate::services::gateway::GATEWAY_AGENT_ID);
        // A gateway that routes, as `handle_gateway_task` does
        let state = world.provider_state.clone();
        executor::set_gateway_handler(
            &provider,
            Arc::new(move |task: Task| {
                let state = state.clone();
                Box::pin(async move {
                    executor::gateway_dispatched(
                        &state,
                        &task,
                        "Routed to papers: math".into(),
                        None,
                    )
                    .await;
                    let mut result = history::placeholder(
                        &task.id,
                        &task.capability,
                        &task.input,
                        Some(&task.target),
                    );
                    result.output = json!("4");
                    history::finish(&mut result, TaskStatus::Completed, None);
                    Ok(result)
                })
            }),
        );
        let result = world
            .requester
            .run(DelegationRequest {
                capability: "gateway".into(),
                input: json!({"text": "What is 2+2?"}),
                target: Some(gateway.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        assert_eq!(result.output, json!("4"));

        let served = row(&world.provider_state, &result.task_id).await;
        assert_eq!(served.state, TaskState::Completed);
        let events = task::events_of(world.provider_state.db(), &result.task_id)
            .await
            .unwrap();
        assert_eq!(
            states(&events),
            [
                TaskState::Submitted,
                TaskState::Delivered,
                TaskState::Acknowledged,
                TaskState::Working,
                TaskState::Completed
            ]
        );
        assert_eq!(events[2].reason.as_deref(), Some("Routed to papers: math"));
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    /// An assignment lasts: the requester's call returns once the person
    /// has it, sweeps leave it open for days, no one else gets it, and the
    /// result comes back however late: pushed when it's done, or fetched
    /// by the requester's poll when that was missed
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_assignment_stays_open_until_its_person_ends_it() {
        use crate::services::a2a::assignment::ASSIGN_ME;
        const DAY: i64 = 24 * 60 * 60 * 1000;
        let world = two_instances(SERVES).await;
        let provider = instance(&world.provider_state);
        let gateway = AgentCard::card_id(&provider, crate::services::gateway::GATEWAY_AGENT_ID);
        // The person's gateway: on their list (acknowledged on the mock
        // relay: the state's own lifecycle isn't on it), done when
        // `release` says
        let release = Arc::new(tokio::sync::Notify::new());
        let relay = world._relays.1.clone();
        let (state, done) = (world.provider_state.clone(), release.clone());
        executor::set_gateway_handler(
            &provider,
            Arc::new(move |task: Task| {
                let (state, done, relay) = (state.clone(), done.clone(), relay.clone());
                Box::pin(async move {
                    relay
                        .send_peer(
                            task.requester.instance_id.clone(),
                            PeerMessage::AcknowledgeTask {
                                task_id: task.id.clone(),
                                eta_seconds: None,
                                note: Some("On Lena's list".into()),
                            },
                        )
                        .await
                        .unwrap();
                    executor::gateway_dispatched(
                        &state,
                        &task,
                        "On Lena's list".into(),
                        Some(DAY as u32 / 1000),
                    )
                    .await;
                    done.notified().await;
                    let mut result = history::placeholder(
                        &task.id,
                        &task.capability,
                        &task.input,
                        Some(&task.target),
                    );
                    result.output = json!({"status": "done", "note": "Uploaded"});
                    history::finish(&mut result, TaskStatus::Completed, None);
                    Ok(result)
                })
            }),
        );
        let assign = |text: &str| DelegationRequest {
            capability: ASSIGN_ME.into(),
            input: json!({"text": text, "due": "3 days"}),
            target: Some(gateway.clone()),
            ..Default::default()
        };

        let started = db::now_ms();
        let result = world
            .requester
            .run(assign("Upload the SOC 1 evidence"))
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);
        assert!(
            db::now_ms() - started < 5_000,
            "it didn't wait for the work"
        );
        let asked = row(&world.requester_state, &result.task_id).await;
        assert!(!asked.state.is_terminal(), "{:?}", asked.state);
        let due = asked.deadline.unwrap() as i64;
        assert!((due - started - 3 * DAY).abs() < 60_000, "due in 3 days");
        assert_eq!(
            history::get(world.requester_state.db(), &result.task_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Running
        );

        // Done while we're offline: the result waits at the relay
        let me = instance(&world.requester_state);
        world.hub.set_online(&me, false);
        release.notify_one();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!row(&world.requester_state, &result.task_id)
            .await
            .state
            .is_terminal());
        world.hub.set_online(&me, true);
        wait_until("the result", || async {
            state_of(&world.requester_state, &result.task_id).await == Some(TaskState::Completed)
        })
        .await;
        let ended = row(&world.requester_state, &result.task_id).await;
        assert_eq!(
            ended.result,
            Some(json!({"status": "done", "note": "Uploaded"}))
        );
        // The history row ends just after the task row
        wait_until("the history row", || async {
            history::get(world.requester_state.db(), &result.task_id)
                .await
                .unwrap()
                .is_some_and(|h| h.status == TaskStatus::Completed)
        })
        .await;
        let history = history::get(world.requester_state.db(), &result.task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            history.output,
            json!({"status": "done", "note": "Uploaded"})
        );

        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    /// Declined while it waited for the person's approval: the requester's
    /// open history row ends and the Inbox gets "{name} declined it"
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_assignment_declined_while_held_comes_back_as_declined() {
        use crate::services::a2a::assignment::ASSIGN_ME;
        use crate::services::a2a::returns::{self, ReturnOutcome};
        let world = two_instances(SERVES).await;
        let provider = instance(&world.provider_state);
        let gateway = AgentCard::card_id(&provider, crate::services::gateway::GATEWAY_AGENT_ID);
        let decline = Arc::new(tokio::sync::Notify::new());
        let relay = world._relays.1.clone();
        let declined = decline.clone();
        executor::set_gateway_handler(
            &provider,
            Arc::new(move |task: Task| {
                let (declined, relay) = (declined.clone(), relay.clone());
                Box::pin(async move {
                    let requester = task.requester.instance_id.clone();
                    relay
                        .send_peer(
                            requester.clone(),
                            PeerMessage::ProgressUpdate {
                                task_id: task.id.clone(),
                                percent: None,
                                note: Some("Waiting for the owner to approve".into()),
                                reason_kind: Some(task::ReasonKind::NeedsTheirApproval),
                                queue_position: None,
                            },
                        )
                        .await
                        .unwrap();
                    declined.notified().await;
                    relay
                        .send_peer(
                            requester,
                            PeerMessage::RejectTask {
                                task_id: task.id.clone(),
                                reason: "The owner declined this request".into(),
                                reason_kind: Some(task::ReasonKind::Declined),
                            },
                        )
                        .await
                        .unwrap();
                    std::future::pending::<()>().await;
                    unreachable!()
                })
            }),
        );
        let result = world
            .requester
            .run(DelegationRequest {
                capability: ASSIGN_ME.into(),
                input: json!({"text": "Collect the screenshots", "due": "3 days"}),
                target: Some(gateway.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);

        decline.notify_one();
        wait_until("the history row", || async {
            history::get(world.requester_state.db(), &result.task_id)
                .await
                .unwrap()
                .is_some_and(|h| h.status == TaskStatus::Failed)
        })
        .await;
        assert_eq!(
            row(&world.requester_state, &result.task_id).await.state,
            TaskState::Rejected
        );
        let returned = returns::list(world.requester_state.db()).await.unwrap();
        let item = returned
            .iter()
            .find(|r| r.task_id == result.task_id)
            .expect("a return for the declined request");
        assert_eq!(item.outcome, ReturnOutcome::Declined);
        assert_eq!(item.request, "Collect the screenshots");
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn my_own_agent_wins_unless_i_pinned_someone_elses() {
        let world = two_instances(SERVES).await;
        allow(&world);
        echo_agents(&world.requester_state).await;
        save_agent(&world.requester_state, "local", SERVES);
        let me = instance(&world.requester_state);
        let remote = format!("{}/papers", instance(&world.provider_state));

        // Mine runs here, with no relay
        let result = world
            .requester
            .run(from_writer("summarize", json!("hi")))
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        assert_eq!(result.provider_agent, format!("{me}/local"));
        assert!(world._relays.0.delegated().is_empty());

        // Pinned and free at the relay: theirs first
        crate::services::shared::set_provider_pin(
            &world.requester_state,
            "summarize",
            Some(&remote),
        )
        .await
        .unwrap();
        let result = world
            .requester
            .run(from_writer("summarize", json!("hi")))
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        assert_eq!(result.provider_agent, remote);
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn progress_is_sent_only_when_the_task_wants_it() {
        let world = two_instances(SERVES).await;
        allow(&world);
        let result = world
            .requester
            .run(from_writer(
                "summarize",
                json!({"text": "hi", "wants_progress": true}),
            ))
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        assert!(world
            ._relays
            .1
            .sent_kinds()
            .contains(&"progress_update".to_string()));
        let db = world.requester_state.db().clone();
        let id = result.task_id.clone();
        wait_until("the progress message", || {
            let (db, id) = (db.clone(), id.clone());
            async move {
                task::messages_of(&db, &id)
                    .await
                    .unwrap()
                    .iter()
                    .any(|m| m.kind == task::PROGRESS_KIND)
            }
        })
        .await;
        // The relay's reasons come first: why it went there
        let messages = task::messages_of(&db, &id).await.unwrap();
        assert_eq!(messages[0].kind, route::ROUTE_KIND);
        assert_eq!(messages[0].payload["reasons"][0]["code"], "free");

        // Heartbeats: only working tasks that want progress
        let requester = instance(&world.requester_state);
        let provider = instance(&world.provider_state);
        let target = format!("{provider}/papers");
        for (id, input) in [
            ("hb-wants", json!({"wants_progress": true})),
            ("hb-quiet", json!({"text": "x"})),
        ] {
            let task = wire(id, &requester, &target, input);
            world._relays.0.track(&task);
            let mut record = TaskRecord::provider(&task, &provider);
            record.state = TaskState::Working;
            task::create(
                world.provider_state.db(),
                world.provider_state.bus(),
                &record,
                Some(&task),
            )
            .await
            .unwrap();
        }
        let provider_service = service(&world.provider_state);
        let before = world._relays.1.sent_kinds().len();
        assert_eq!(provider_service.heartbeat().await.unwrap(), 1);
        let sent = world._relays.1.sent_peer.lock().unwrap()[before..].to_vec();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            &sent[0].1,
            PeerMessage::ProgressUpdate { task_id, note: Some(note), .. }
                if task_id == "hb-wants" && note == "still working"
        ));
        assert!(!provider_service
            .send_progress("hb-quiet", Some(50), None)
            .await
            .unwrap());
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn the_outbox_flushes_in_priority_order_once_the_relay_is_back() {
        let (state, _hub, relay, service, _them) = requester_alone(None).await;
        relay.set_connected(false);
        let me = instance(&state);
        for (id, priority, created_at) in [("old", 0, 1.0), ("urgent", 5, 3.0), ("new", 0, 2.0)] {
            let mut record = TaskRecord::requester(
                id,
                &me,
                "writer",
                "them/papers",
                "summarize",
                &json!({ "priority": priority }),
                None,
            );
            record.created_at = created_at;
            assert_eq!(record.priority, priority);
            let task = wire(id, &me, "them/papers", record.input.clone());
            task::create(state.db(), state.bus(), &record, Some(&task))
                .await
                .unwrap();
        }
        // No relay: everything waits
        assert!(service.flush().await.is_empty());
        assert!(relay.delegated().is_empty());
        assert_eq!(state_of(&state, "old").await, Some(TaskState::Submitted));

        relay.set_connected(true);
        // Two flushes at once (the relay connecting + the sweep) send each
        // request once
        let (first, second) = tokio::join!(service.flush(), service.flush());
        assert_eq!([first, second].concat(), ["urgent", "old", "new"]);
        assert_eq!(relay.delegated(), ["urgent", "old", "new"]);
        for id in ["urgent", "old", "new"] {
            assert_eq!(state_of(&state, id).await, Some(TaskState::Delivered));
        }
        assert!(service.flush().await.is_empty(), "nothing left");
        state.shutdown();
    }

    #[tokio::test]
    async fn a_delegation_to_an_offline_provider_waits_at_the_relay() {
        let world = two_instances(SERVES).await;
        allow(&world);
        let provider = instance(&world.provider_state);
        world.hub.set_online(&provider, false);
        let requester = world.requester.clone();
        let target = format!("{provider}/papers");
        let running = tokio::spawn(async move {
            requester
                .delegate_to(&target, "summarize", json!("queued work"))
                .await
        });
        let state = world.requester_state.clone();
        let queued = || {
            let state = state.clone();
            async move {
                task::list_tasks(
                    &state,
                    TaskFilter {
                        state: Some("submitted".into()),
                        ..TaskFilter::default()
                    },
                )
                .await
                .unwrap()
            }
        };
        wait_until("the held task", || async { !queued().await.is_empty() }).await;
        // The relay has it: nothing for the outbox
        assert_eq!(world._relays.0.delegated().len(), 1);
        assert!(service(&world.requester_state).flush().await.is_empty());

        world.hub.set_online(&provider, true);
        let result = running.await.unwrap().unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        assert_eq!(
            state_of(&world.requester_state, &result.task_id).await,
            Some(TaskState::Completed)
        );
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn a_restart_resumes_in_flight_tasks() {
        let world = two_instances(SERVES).await;
        allow(&world);
        let requester = instance(&world.requester_state);
        let provider = instance(&world.provider_state);
        let target = format!("{provider}/papers");

        // Provider: a task that was working when the app stopped
        let task = wire("resumed", &requester, &target, json!("again"));
        let mut served = TaskRecord::provider(&task, &provider);
        served.state = TaskState::Working;
        task::create(
            world.provider_state.db(),
            world.provider_state.bus(),
            &served,
            Some(&task),
        )
        .await
        .unwrap();
        // ...and the requester's row of it, waiting for the result
        let mut asked = TaskRecord::requester(
            "resumed",
            &requester,
            delegator::USER_REQUESTER,
            &target,
            "summarize",
            &task.input,
            None,
        );
        asked.state = TaskState::Acknowledged;
        task::create(
            world.requester_state.db(),
            world.requester_state.bus(),
            &asked,
            None,
        )
        .await
        .unwrap();
        world._relays.0.track(&task);
        // Requester: a queued task
        let queued_task = wire("queued", &requester, &target, json!("later"));
        let queued = TaskRecord::requester(
            "queued",
            &requester,
            delegator::USER_REQUESTER,
            &target,
            "summarize",
            &queued_task.input,
            None,
        );
        task::create(
            world.requester_state.db(),
            world.requester_state.bus(),
            &queued,
            Some(&queued_task),
        )
        .await
        .unwrap();

        let resumed = service(&world.provider_state).resume().await.unwrap();
        assert_eq!(resumed, ["resumed"]);
        let events = task::events_of(world.provider_state.db(), "resumed")
            .await
            .unwrap();
        let resume = events.last().unwrap();
        assert_eq!(resume.from_state, Some(TaskState::Working));
        assert_eq!(resume.to_state, TaskState::Working);
        assert_eq!(resume.reason.as_deref(), Some(task_state::RESUMED_REASON));

        let mut ids = service(&world.requester_state).resume().await.unwrap();
        ids.sort();
        assert_eq!(ids, ["queued", "resumed"]);

        for (state, id) in [
            (&world.provider_state, "resumed"),
            (&world.requester_state, "resumed"),
            (&world.requester_state, "queued"),
        ] {
            wait_until(&format!("{id} to complete"), || async {
                state_of(state, id).await == Some(TaskState::Completed)
            })
            .await;
        }
        let done = row(&world.requester_state, "resumed").await;
        assert!(done.result.unwrap().as_str().unwrap().contains("again"));
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn a_retry_creates_the_next_attempt_elsewhere() {
        let (state, hub, relay, service, them) = requester_alone(None).await;
        them.register(vec![serving("them/papers")]).await.unwrap();
        let other = MockRelay::join(&hub, "other");
        other.register(vec![serving("other/papers")]).await.unwrap();
        let me = instance(&state);

        let task = wire("first", &me, "them/papers", json!("x"));
        let mut failed = TaskRecord::requester(
            "first",
            &me,
            "writer",
            "them/papers",
            "summarize",
            &json!("x"),
            None,
        );
        failed.state = TaskState::Failed;
        task::create(state.db(), state.bus(), &failed, Some(&task))
            .await
            .unwrap();

        // The relay has nobody else to plan (the other one is offline):
        // nowhere to go
        hub.set_online("other", false);
        assert!(service.retry(&failed).await.unwrap().is_none());
        hub.set_online("other", true);

        let next = service.retry(&failed).await.unwrap().unwrap();
        assert_eq!(next.attempt_number, 2);
        assert_eq!(next.previous_attempt_id.as_deref(), Some("first"));
        assert_eq!(next.source_kind, TaskSourceKind::Failover);
        assert_eq!(next.provider_instance, "other", "never the failed one");
        assert_eq!(next.state, TaskState::Delivered);
        assert_eq!(relay.delegated(), std::slice::from_ref(&next.id));
        let sent = task::request_of(state.db(), &next.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sent.target, "other/papers");
        assert_eq!(sent.input, json!("x"));
        state.shutdown();
    }

    #[tokio::test]
    async fn status_queries_and_cancels_reach_the_provider() {
        let world = two_instances(SERVES).await;
        let requester = instance(&world.requester_state);
        let provider = instance(&world.provider_state);
        let target = format!("{provider}/papers");
        let task = wire("c", &requester, &target, json!("x"));
        let mut asked = TaskRecord::requester(
            "c",
            &requester,
            "writer",
            &target,
            "summarize",
            &task.input,
            None,
        );
        asked.state = TaskState::Acknowledged;
        task::create(
            world.requester_state.db(),
            world.requester_state.bus(),
            &asked,
            Some(&task),
        )
        .await
        .unwrap();
        world._relays.0.track(&task);
        let mut served = TaskRecord::provider(&task, &provider);
        served.state = TaskState::Working;
        served.eta_seconds = Some(42);
        task::create(
            world.provider_state.db(),
            world.provider_state.bus(),
            &served,
            None,
        )
        .await
        .unwrap();

        let live = service(&world.requester_state)
            .query_status("c")
            .await
            .unwrap();
        assert!(live.live);
        assert_eq!(live.state, TaskState::Working);
        assert_eq!(live.eta_seconds, Some(42));
        assert_eq!(row(&world.requester_state, "c").await.eta_seconds, Some(42));

        world.hub.set_online(&provider, false);
        let local = service(&world.requester_state)
            .query_status("c")
            .await
            .unwrap();
        assert!(!local.live);
        assert_eq!(local.state, TaskState::Acknowledged);
        world.hub.set_online(&provider, true);

        let cancelled = service(&world.requester_state)
            .cancel("c", Some("no longer needed".into()))
            .await
            .unwrap();
        assert_eq!(cancelled.state, TaskState::Cancelled);
        wait_until("the provider's cancel", || async {
            state_of(&world.provider_state, "c").await == Some(TaskState::Cancelled)
        })
        .await;
        assert!(service(&world.requester_state)
            .cancel("c", None)
            .await
            .is_err());

        let mine = task::list_tasks(
            &world.requester_state,
            TaskFilter {
                role: Some("requester".into()),
                ..TaskFilter::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(mine.len(), 1);
        let events = task::get_task_events(&world.requester_state, "c")
            .await
            .unwrap();
        assert_eq!(
            events.last().unwrap().reason.as_deref(),
            Some("no longer needed")
        );
        assert!(task::get_task(&world.requester_state, "nope")
            .await
            .is_err());
        world.requester_state.shutdown();
        world.provider_state.shutdown();
    }

    #[tokio::test]
    async fn peers_can_only_move_their_own_tasks() {
        let (state, _hub, relay, _service, them) = requester_alone(None).await;
        let me = instance(&state);
        let mut record = TaskRecord::requester(
            "t",
            &me,
            "writer",
            "them/papers",
            "summarize",
            &json!(1),
            None,
        );
        record.state = TaskState::Delivered;
        task::create(state.db(), state.bus(), &record, None)
            .await
            .unwrap();
        relay.track(&wire("t", &me, "them/papers", json!(1)));
        // "intruder" isn't the provider: the relay refuses it
        let intruder = MockRelay::join(&_hub, "intruder");
        intruder
            .send_peer(
                me.clone(),
                PeerMessage::RejectTask {
                    task_id: "t".into(),
                    reason: "no".into(),
                    reason_kind: None,
                },
            )
            .await
            .unwrap_err();
        them.send_peer(
            me.clone(),
            PeerMessage::AcknowledgeTask {
                task_id: "t".into(),
                eta_seconds: Some(7),
                note: None,
            },
        )
        .await
        .unwrap();
        wait_until("the acknowledgement", || async {
            state_of(&state, "t").await == Some(TaskState::Acknowledged)
        })
        .await;
        assert_eq!(row(&state, "t").await.eta_seconds, Some(7));
        state.shutdown();
    }

    #[tokio::test]
    async fn eta_estimates_come_from_history() {
        let state = AppState::for_tests(vec![]);
        let none = task::get_eta_estimate(&state, "summarize", None)
            .await
            .unwrap();
        assert_eq!(none.sample_count, 0);
        assert_eq!(
            none.median_seconds,
            crate::services::a2a::eta::DEFAULT_MEDIAN_SECONDS
        );
        assert!(task::get_eta_estimate(&state, " ", None).await.is_err());
        state.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Wave final merge wiring: gateway tasks, trust, clarification pauses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gateway_tasks_go_to_the_gateway_handler() {
    let state = AppState::for_tests(vec![]);
    let me = instance(&state);
    let target = AgentCard::card_id(&me, crate::services::gateway::GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();

    // The hook `gateway::init` registered: the gateway answers (whatever it
    // says without NANDI.md or a model), never "no such agent"
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    let error = result.error.unwrap_or_default();
    assert!(!error.contains("no agent"), "{error}");
    assert!(!error.contains("isn't an agent"), "{error}");

    // A handler that routes, then one that refuses
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let handled = seen.clone();
    executor::set_gateway_handler(
        &me,
        Arc::new(move |task: Task| {
            handled.lock().unwrap().push(task.target.clone());
            Box::pin(async move {
                let mut result = history::placeholder(
                    &task.id,
                    &task.capability,
                    &task.input,
                    Some(&task.target),
                );
                result.output = json!("routed");
                history::finish(&mut result, TaskStatus::Completed, None);
                Ok(result)
            })
        }),
    );
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.output, json!("routed"));
    assert_eq!(*seen.lock().unwrap(), std::slice::from_ref(&target));

    executor::set_gateway_handler(
        &me,
        Arc::new(|_task: Task| {
            Box::pin(async {
                Err(task::Refusal::new(
                    task::ReasonKind::NotAccepting,
                    task::TaskStage::Policy,
                    "Not for you",
                ))
            })
        }),
    );
    let refused = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(refused.status, TaskStatus::Failed);
    assert_eq!(refused.error.as_deref(), Some("Not for you"));
    state.shutdown();
}

#[tokio::test]
async fn the_gateway_reaches_agents_that_dont_take_direct_delegation() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    // Behind the gateway only: no `accepts_delegation`
    save_agent(
        &state,
        "quiet",
        "serves: [{name: summarize}]
",
    );
    let me = instance(&state);
    let delegator = state.a2a_delegator().unwrap().clone();
    let request = |requester_agent: Option<&str>| DelegationRequest {
        task_id: None,
        requester_agent: requester_agent.map(str::to_string),
        // The gateway passes what the caller asked for, maybe "gateway"
        capability: crate::services::gateway::GATEWAY_AGENT_ID.to_string(),
        input: json!("What is 2+2?"),
        target: Some(AgentCard::card_id(&me, "quiet")),
    };

    let routed = delegator
        .delegate_request(request(Some(crate::services::gateway::GATEWAY_AGENT_ID)))
        .await
        .unwrap();
    assert_eq!(routed.status, TaskStatus::Completed, "{:?}", routed.error);
    assert!(routed.output.to_string().contains("What is 2+2?"));

    // Anyone else still needs the agent to accept delegation
    let direct = delegator.delegate_request(request(None)).await.unwrap();
    assert_eq!(direct.status, TaskStatus::Failed);
    state.shutdown();
}

#[tokio::test]
async fn each_delegated_task_runs_in_its_own_conversation() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]
accepts_delegation: true
",
    );
    let me = instance(&state);
    let delegator = state.a2a_delegator().unwrap().clone();
    let mut tasks = Vec::new();
    for text in ["Alice's private request", "Carol's request"] {
        let result = delegator
            .delegate_request(DelegationRequest {
                capability: "summarize".into(),
                input: json!(text),
                target: Some(AgentCard::card_id(&me, "papers")),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        tasks.push(result.task_id);
    }
    let conversations: Vec<String> = state
        .db()
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT conversation_id FROM runs WHERE agent_id = 'papers' ORDER BY started_at",
            )?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    // Neither sees the other, nor the owner's chat with the agent
    let expected: Vec<String> = tasks.iter().map(|t| db::task_conversation_id(t)).collect();
    assert_eq!(conversations, expected);
    assert!(!conversations.contains(&db::agent_conversation_id("papers")));
    state.shutdown();
}

/// The pending gateway holds once one appears (or panics after ~5 s)
async fn wait_for_hold(state: &AppState) -> crate::services::gateway::holds::GatewayHold {
    for _ in 0..400 {
        let holds = crate::services::gateway::list_gateway_holds(state)
            .await
            .unwrap();
        if let Some(hold) = holds.into_iter().next() {
            return hold;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no hold appeared");
}

/// Task 5: "Needs my approval" holds the request instead of refusing it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_capability_needing_approval_waits_for_the_owner() {
    use crate::services::gateway::{self, holds::HoldStatus, GATEWAY_AGENT_ID};
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec!["summarize".into()];
    gateway::set_gateway_config(&state, config).await.unwrap();
    let target = AgentCard::card_id(&instance(&state), GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();
    let ask = |text: &str| {
        let (delegator, target) = (delegator.clone(), target.clone());
        let input = json!({ "text": text });
        tokio::spawn(async move {
            delegator
                .delegate_request(DelegationRequest {
                    capability: "summarize".into(),
                    input,
                    target: Some(target),
                    ..Default::default()
                })
                .await
                .unwrap()
        })
    };

    // Held, listed for the Inbox, and it goes on once approved
    let approved = ask("Summarize Q3 risk");
    let hold = wait_for_hold(&state).await;
    assert_eq!(hold.status, HoldStatus::Pending);
    assert_eq!(hold.capability.as_deref(), Some("summarize"));
    assert_eq!(hold.summary, "Summarize Q3 risk");
    assert!(!approved.is_finished(), "it waits");
    gateway::decide_gateway_hold(&state, hold.task_id.clone(), true)
        .await
        .unwrap();
    let result = approved.await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    // Decided once only
    assert!(gateway::decide_gateway_hold(&state, hold.task_id, false)
        .await
        .is_err());

    // Declined: refused with the owner's no, not a policy reason
    let declined = ask("Summarize Q4 risk");
    let hold = wait_for_hold(&state).await;
    gateway::decide_gateway_hold(&state, hold.task_id, false)
        .await
        .unwrap();
    let result = declined.await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("declined"),
        "{:?}",
        result.error
    );
    assert!(gateway::list_gateway_holds(&state)
        .await
        .unwrap()
        .is_empty());
    state.shutdown();
}

/// The pending assignments once one appears (or panics after ~5 s)
async fn wait_for_assignment(
    state: &AppState,
) -> crate::services::gateway::assignments::GatewayAssignment {
    for _ in 0..400 {
        let open = crate::services::gateway::list_gateway_assignments(state)
            .await
            .unwrap();
        if let Some(assignment) = open.into_iter().next() {
            return assignment;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no assignment appeared");
}

/// The delegation's history row once it has ended
async fn wait_for_history_end(state: &AppState, task_id: &str) -> TaskResult {
    for _ in 0..200 {
        let row = history::get(state.db(), task_id).await.unwrap();
        if let Some(row) = row.filter(|r| r.status != TaskStatus::Running) {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the delegation {task_id} didn't end");
}

/// Work assigned to the owner (your own gateway): the call returns once
/// it's held or on the list, as it does for someone else. Approved, it
/// isn't routed to an agent but waits in the Inbox until the owner marks it
/// Done (the note is the result) or Declines it (refused with the note)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn assigned_work_waits_for_the_owners_done_or_decline() {
    use crate::services::a2a::assignment::ASSIGN_ME;
    use crate::services::gateway::{self, assignments::AssignmentStatus, GATEWAY_AGENT_ID};
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: assign-me}]
",
    );
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec![ASSIGN_ME.into()];
    gateway::set_gateway_config(&state, config).await.unwrap();
    let target = AgentCard::card_id(&instance(&state), GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();
    let assign = |text: &str, links: Value| {
        let (delegator, target) = (delegator.clone(), target.clone());
        let input = json!({ "text": text, "due": "2027-03-01", "links": links });
        async move {
            let started = db::now_ms();
            let result = delegator
                .delegate_request(DelegationRequest {
                    capability: ASSIGN_ME.into(),
                    input,
                    target: Some(target),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);
            assert!(
                db::now_ms() - started < 5_000,
                "it didn't wait for the owner"
            );
            result
        }
    };

    // Held for approval: the call has returned already
    let asked = assign("Upload the SOC 1 evidence", json!([])).await;
    assert_eq!(asked.output["state"], "delivered");
    let hold = wait_for_hold(&state).await;
    assert_eq!(hold.task_id, asked.task_id);
    gateway::decide_gateway_hold(&state, hold.task_id.clone(), true)
        .await
        .unwrap();
    let assignment = wait_for_assignment(&state).await;
    assert_eq!(assignment.task_id, asked.task_id);
    assert_eq!(assignment.request, "Upload the SOC 1 evidence");
    assert_eq!(assignment.status, AssignmentStatus::Open);
    assert!(assignment.due_at.is_some());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        history::get(state.db(), &asked.task_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Running,
        "no agent did it for the owner"
    );
    let closed = gateway::close_gateway_assignment(
        &state,
        assignment.task_id.clone(),
        true,
        Some("Uploaded to the audit folder".into()),
    )
    .await
    .unwrap();
    assert_eq!(closed.status, AssignmentStatus::Done);
    let result = wait_for_history_end(&state, &asked.task_id).await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.output["status"], "done");
    assert_eq!(result.output["note"], "Uploaded to the audit folder");
    // Closed once only
    assert!(
        gateway::close_gateway_assignment(&state, assignment.task_id, false, None)
            .await
            .is_err()
    );

    // Declined: refused with the owner's note
    let declined = assign("Rotate the keys", json!([])).await;
    let hold = wait_for_hold(&state).await;
    gateway::decide_gateway_hold(&state, hold.task_id, true)
        .await
        .unwrap();
    let assignment = wait_for_assignment(&state).await;
    gateway::close_gateway_assignment(
        &state,
        assignment.task_id,
        false,
        Some("Not my system".into()),
    )
    .await
    .unwrap();
    let result = wait_for_history_end(&state, &declined.task_id).await;
    assert_eq!(result.status, TaskStatus::Failed);
    let error = result.error.unwrap_or_default();
    assert!(error.contains("declined it: Not my system"), "{error}");
    assert!(gateway::list_gateway_assignments(&state)
        .await
        .unwrap()
        .is_empty());

    // Without approval it goes straight onto the list
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec![];
    gateway::set_gateway_config(&state, config).await.unwrap();
    // with the links the requester attached
    let listed = assign(
        "Book the auditor",
        json!([{ "url": "https://cal.example.com/audit", "name": "Auditor's calendar" }]),
    )
    .await;
    assert_eq!(listed.output["state"], "acknowledged");
    let assignment = wait_for_assignment(&state).await;
    assert_eq!(assignment.task_id, listed.task_id);
    assert_eq!(assignment.links.len(), 1);
    assert_eq!(assignment.links[0].name, "Auditor's calendar");
    assert_eq!(assignment.links[0].url, "https://cal.example.com/audit");
    state.shutdown();
}

/// Task 5: a busy gateway queues the request instead of refusing it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_gateway_queues_the_request_until_a_slot_frees() {
    use crate::services::gateway::{self, GATEWAY_AGENT_ID, GATEWAY_LINE};
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.max_concurrent = 1;
    gateway::set_gateway_config(&state, config).await.unwrap();
    let me = instance(&state);
    let service = state.gateway().unwrap().clone();
    let manifest = service.manifest().unwrap();
    // Another request holds the only slot
    let caller = service.caller_policy(&me, "").await.unwrap();
    let busy = service.acquire(&caller, &manifest).unwrap();

    let delegator = state.a2a_delegator().unwrap().clone();
    let target = AgentCard::card_id(&me, GATEWAY_AGENT_ID);
    let queued = tokio::spawn(async move {
        delegator
            .delegate_request(DelegationRequest {
                capability: "summarize".into(),
                input: json!({ "text": "Summarize this" }),
                target: Some(target),
                ..Default::default()
            })
            .await
            .unwrap()
    });
    // It waits first in line, not refused
    let mut waiting = 0;
    for _ in 0..400 {
        waiting = service.slots().waiting(GATEWAY_LINE);
        if waiting > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(waiting, 1, "queued, alone in line (position 1)");
    assert!(!queued.is_finished());

    drop(busy);
    let result = queued.await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(service.slots().waiting(GATEWAY_LINE), 0);
    state.shutdown();
}

#[tokio::test]
async fn a_trusted_peer_passes_the_ask_policy() {
    use crate::services::team::trust::{update_relationship, TrustUpdate};
    let world = two_instances(
        "serves: [{name: summarize}]
accepts_delegation: true
",
    )
    .await;
    // Default policy `ask`: a stranger is turned away
    let refused = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(refused.status, TaskStatus::Failed);
    assert!(refused.error.unwrap_or_default().contains("needs to trust"));

    // Marked trusted in Network: let in
    update_relationship(
        world.provider_state.db(),
        &instance(&world.requester_state),
        &TrustUpdate {
            added_as_trusted_peer: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let allowed = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(allowed.status, TaskStatus::Completed, "{:?}", allowed.error);

    // An explicit deny still wins
    trust::set_policy(
        world.provider_state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Deny,
    )
    .unwrap();
    let denied = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(denied.status, TaskStatus::Failed);
    world.requester_state.shutdown();
    world.provider_state.shutdown();
}

#[tokio::test]
async fn a_paused_requester_instance_is_rejected() {
    use crate::services::team::trust::{update_relationship, TrustUpdate};
    let world = two_instances("serves: [{name: summarize}]\naccepts_delegation: true\n").await;
    trust::set_policy(
        world.provider_state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Allow,
    )
    .unwrap();
    update_relationship(
        world.provider_state.db(),
        &instance(&world.requester_state),
        &TrustUpdate {
            delegations_paused: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let result = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("paused"),
        "{:?}",
        result.error
    );
    world.requester_state.shutdown();
    world.provider_state.shutdown();
}

#[tokio::test]
async fn a_blocked_requester_instance_is_rejected_on_arrival() {
    use crate::services::team::trust::{set_blocked, ALL_AGENTS};
    let world = two_instances("serves: [{name: summarize}]\naccepts_delegation: true\n").await;
    trust::set_policy(
        world.provider_state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Allow,
    )
    .unwrap();
    let requester = instance(&world.requester_state);
    set_blocked(world.provider_state.db(), &requester, ALL_AGENTS, true).unwrap();
    let result = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("blocked"),
        "{:?}",
        result.error
    );

    // Unblocked: the same request goes through
    set_blocked(world.provider_state.db(), &requester, ALL_AGENTS, false).unwrap();
    let result = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    world.requester_state.shutdown();
    world.provider_state.shutdown();
}

#[tokio::test]
async fn blocked_agents_and_paused_instances_are_never_picked() {
    use crate::services::team::trust::{set_blocked, update_relationship, TrustUpdate};
    let world = two_instances("serves: [{name: summarize}]\naccepts_delegation: true\n").await;
    let provider = instance(&world.provider_state);
    let db = world.requester_state.db();
    assert_eq!(world.requester.providers("summarize").await.len(), 1);

    set_blocked(db, &provider, "papers", true).unwrap();
    assert!(world.requester.providers("summarize").await.is_empty());
    let result = world
        .requester
        .run(from_writer("summarize", json!("hi")))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(result.error.unwrap().contains("No agent that serves"));
    // Named, it's refused before it's sent
    let named = world
        .requester
        .run(DelegationRequest {
            target: Some(format!("{provider}/papers")),
            ..from_writer("summarize", json!("hi"))
        })
        .await
        .unwrap();
    assert!(named.error.unwrap().contains("blocked"));

    set_blocked(db, &provider, "papers", false).unwrap();
    update_relationship(
        db,
        &provider,
        &TrustUpdate {
            delegations_paused: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(world.requester.providers("summarize").await.is_empty());
    // Our own instance is never filtered
    let cards = vec![user_card(&instance(&world.requester_state))];
    let me = instance(&world.requester_state);
    assert_eq!(failover::without_untrusted(db, &me, cards).len(), 1);
    world.requester_state.shutdown();
    world.provider_state.shutdown();
}

#[tokio::test]
async fn a_task_waiting_for_an_answer_doesnt_time_out() {
    let state = AppState::for_tests(vec![]);
    // A turn of ~12 × 40ms, well past the 150ms timeout
    state
        .db()
        .call(|conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(
                conn,
                CANNED_SCRIPT_KEY,
                &json!({
                    "agent": [{ "text": "one two three four five six seven eight nine ten eleven twelve" }],
                    "studio": [],
                    "tokenDelayMs": 40
                }),
            )
        })
        .await
        .unwrap();
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let me = instance(&state);
    let delegator = Delegator::new(A2aDeps {
        timeout: Duration::from_millis(150),
        ..deps(&state, None)
    });
    let task = |id: &str| Task {
        id: id.into(),
        requester: user_card(&me),
        target: format!("{me}/papers"),
        capability: "summarize".into(),
        input: json!("hi"),
        created_at: db::now_ms() as f64,
        deadline: None,
    };

    // Not waiting: the timeout fires
    let timed_out = delegator.executor().execute(&task("plain"), false).await;
    assert_eq!(timed_out.status, TaskStatus::TimedOut);

    // Its row is `InputRequired` (the requester's human is answering)
    let waiting = task("waiting");
    let record = task::TaskRecord::requester(
        &waiting.id,
        &me,
        "_user",
        &waiting.target,
        &waiting.capability,
        &waiting.input,
        None,
    );
    task::create(state.db(), state.bus(), &record, None)
        .await
        .unwrap();
    task_state::advance(
        state.db(),
        state.bus(),
        &waiting.id,
        task::TaskState::InputRequired,
        task_state::Change::reason("asked"),
    )
    .await
    .unwrap()
    .unwrap();
    let done = delegator.executor().execute(&waiting, false).await;
    assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error);
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Supervisor reassignment (wave EF, Track C)
// ---------------------------------------------------------------------------

/// Another provider on `hub` with `papers` serving `summarize`, allowing
/// any requester
async fn join_provider(
    hub: &Arc<Hub>,
    log_dir: &std::path::Path,
) -> (
    AppState,
    Arc<Delegator>,
    Arc<MockRelay>,
    Arc<task::TaskService>,
) {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    trust::set_policy(
        state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Allow,
    )
    .unwrap();
    let relay = MockRelay::join(hub, &instance(&state));
    let service = lifecycle(&state, relay.clone(), Default::default()).await;
    let delegator = Delegator::new(A2aDeps {
        audit: AuditLog::at(log_dir.join(format!("{}.log", instance(&state)))),
        ..deps(&state, Some(relay.clone()))
    });
    delegator.clone().register_relay_handlers().await;
    relay.register(delegator.local_cards("")).await.unwrap();
    (state, delegator, relay, service)
}

#[tokio::test]
async fn a_provider_reassigns_a_task_and_the_requester_gets_the_result() {
    let world = two_instances("serves: [{name: summarize}]\naccepts_delegation: true\n").await;
    trust::set_policy(
        world.provider_state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Allow,
    )
    .unwrap();
    let alice = instance(&world.provider_state);
    // Alice is slow: her agent streams a long reply
    let long = "word ".repeat(400);
    world
        .provider_state
        .db()
        .call(move |conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(
                conn,
                CANNED_SCRIPT_KEY,
                &json!({ "agent": [{ "text": long }], "studio": [], "tokenDelayMs": 50 }),
            )
        })
        .await
        .unwrap();
    let log_dir = tempfile::tempdir().unwrap();
    let (bob_state, _bob, _bob_relay, _bob_service) =
        join_provider(&world.hub, log_dir.path()).await;
    let bob = instance(&bob_state);

    let requester = world.requester.clone();
    let target = format!("{alice}/papers");
    let run = tokio::spawn(async move {
        requester
            .run(DelegationRequest {
                target: Some(target),
                ..from_writer("summarize", json!("hand me on"))
            })
            .await
    });

    // Alice has it and is on it
    let alice_service = task::service_for(&alice).unwrap();
    let provider_db = world.provider_state.db().clone();
    let mut task_id = String::new();
    for _ in 0..500 {
        let rows = task::list_rows(&provider_db, &alice, task::TaskFilter::default())
            .await
            .unwrap();
        if let Some(row) = rows.iter().find(|r| {
            matches!(
                r.state,
                task::TaskState::Acknowledged | task::TaskState::Working
            )
        }) {
            task_id = row.id.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!task_id.is_empty(), "Alice never picked the task up");

    // Bob is the only teammate she can hand it to
    let candidates = reassign::reassign_candidates_for(&alice_service, &task_id)
        .await
        .unwrap();
    let ids: Vec<&str> = candidates.iter().map(|c| c.instance_id.as_str()).collect();
    assert_eq!(ids, [bob.as_str()]);
    let not_back = reassign::reassign(
        &alice_service,
        reassign::ReassignRequest {
            task_id: task_id.clone(),
            new_provider_instance: instance(&world.requester_state),
            reason: None,
        },
    )
    .await;
    assert!(not_back.unwrap_err().contains("back to"));

    let row = reassign::reassign(
        &alice_service,
        reassign::ReassignRequest {
            task_id: task_id.clone(),
            new_provider_instance: bob.clone(),
            reason: Some("swamped".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(row.state, task::TaskState::Reassigned);

    // The requester never resubmitted, and Bob's echo is the result
    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("the delegation followed the reassignment")
        .unwrap()
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert!(
        result.output.to_string().contains("hand me on"),
        "{}",
        result.output
    );
    assert!(
        result.provider_agent.starts_with(&format!("{bob}/")),
        "the answer is Bob's, not Alice's: {}",
        result.provider_agent
    );

    // One task, one row: it followed Bob, with the move in its thread
    let requester_db = world.requester_state.db();
    let asked = task::get_row(requester_db, &task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked.state, task::TaskState::Completed);
    assert_eq!(asked.provider_instance, bob);
    let notes = task::messages_of(requester_db, &task_id).await.unwrap();
    let note = notes
        .iter()
        .find(|m| m.kind == reassign::REASSIGNED_KIND)
        .expect("a reassigned note in the thread");
    assert_eq!(note.payload["toInstance"], json!(bob));
    assert_eq!(note.payload["reason"], json!("swamped"));
    // Bob saw an ordinary delivered task, under the same ID
    let served = task::get_row(bob_state.db(), &task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(served.state, task::TaskState::Completed);

    world.requester_state.shutdown();
    world.provider_state.shutdown();
    bob_state.shutdown();
}
