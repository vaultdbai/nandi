//! Triggers: rules that start agent or workflow runs when something
//! happens — a schedule (`cron`), a calendar with a rota of who is
//! responsible (`calendar`), a file arriving (`file`), an HTTP call
//! (`webhook`), a device event (`device`), or one of Nandi's own events
//! (`event`).
//!
//! ```text
//! cron task ─┐
//! calendar  ─┤ (+ handover watches)
//! file watch ┤                       ┌─ rate limit / storm guard
//! webhook  ──┼─▶ TriggerEvent ─▶ dispatcher ─▶ trigger_fires row ─▶ agent queue (run)
//! device   ──┤      (mpsc)           └─ trigger://fired | rate_limited | error
//! event bus ─┘
//! ```
//!
//! Rules are files, `triggers/<id>.yaml` in the gold copy; the `triggers`
//! table is their projection. Webhook and device secrets live in the OS
//! keyring (named by the trigger's `secretRef`, default
//! `nandi.trigger.<id>`), never in the files or SQLite. A change, whether a
//! command here or an edited file, goes repo → projection → `reload`, which
//! re-reads the rules (attaching their secrets) and re-registers cron jobs
//! and file watchers, so edits apply without a restart. Every fire is a
//! `trigger_fires` row; a dispatched one is also a run whose `source_type`
//! is the trigger's (`cron`, `file`, …) and `source_id` the trigger's ID.
//!
//! No Tauri types here (see `services/`).

pub mod calendar;
pub mod cron;
pub mod device;
pub mod dispatch;
pub mod event;
pub mod file;
pub mod handover;
pub mod repo;
pub mod schema;
pub mod store;
pub mod template;
pub mod webhook;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio::task::AbortHandle;

use crate::db::{self, Db};
use crate::model::Timestamp;
use crate::services::agents;
use crate::services::domain::{ChangedBy, EntityKind, TriggerId};
use crate::services::events::{BroadcastSink, EventBus, NandiEvent};
use crate::services::orchestrator::Orchestrators;
use crate::services::repo_store::RepoStore;
use crate::services::repos::Repository;
use crate::services::secrets::Secrets;
use crate::services::workflows::WorkflowEngine;
use crate::services::AppState;

use device::{DeviceEvent, DeviceMatch};
use dispatch::StormDetector;
use file::FileWatch;
use schema::{SavedTrigger, Trigger, TriggerEvent, TriggerFire, TriggerInput, TriggerSourceType};
use webhook::{ReceiveError, RequestLimiter};

/// Fires waiting for the dispatcher before receivers answer `503`
const QUEUE_CAPACITY: usize = 1024;
/// Largest webhook/device payload accepted (it's stored with the fire)
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
const MAX_NAME_CHARS: usize = 100;
const MAX_TEMPLATE_CHARS: usize = 20_000;
/// Fire history kept per page
pub const DEFAULT_FIRE_LIMIT: u32 = 50;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

macro_rules! event {
    ($ty:ident, $name:literal) => {
        impl NandiEvent for $ty {
            const NAME: &'static str = $name;
        }
    };
}

/// A trigger fired and dispatched a run.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerFired {
    pub trigger_id: String,
    pub fire_id: String,
    pub source_type: TriggerSourceType,
    pub payload: Value,
    /// The agent run, or workflow run, it started
    pub run_id: Option<String>,
}
event!(TriggerFired, "trigger://fired");

/// A trigger fired too soon after its last run (`rate_limit_seconds`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRateLimited {
    pub trigger_id: String,
    pub fire_id: String,
    pub next_allowed_at: Timestamp,
}
event!(TriggerRateLimited, "trigger://rate_limited");

/// A fire couldn't dispatch, or the trigger was disabled for storming.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerError {
    pub trigger_id: String,
    pub fire_id: Option<String>,
    pub error: String,
}
event!(TriggerError, "trigger://error");

/// Triggers were added, changed, enabled/disabled, or removed.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggersChanged {}
event!(TriggersChanged, "trigger://changed");

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Sources {
    /// Cron and calendar jobs
    schedules: HashMap<String, AbortHandle>,
    files: HashMap<String, FileWatch>,
    /// The dispatcher and event-bus subscriber
    tasks: Vec<AbortHandle>,
}

pub struct TriggerEngine {
    /// Webhook and device secrets
    secrets: Secrets,
    db: Db,
    bus: EventBus,
    orchestrators: Orchestrators,
    workflows: Arc<WorkflowEngine>,
    broadcast: BroadcastSink,
    runtime: tokio::runtime::Handle,
    /// The gold copy; triggers live in `{agents repo}/triggers/*.yaml`
    pub(crate) store: RepoStore,
    /// Enabled and disabled rules, as of the last reload
    triggers: RwLock<HashMap<String, Trigger>>,
    tx: mpsc::Sender<TriggerEvent>,
    rx: Mutex<Option<mpsc::Receiver<TriggerEvent>>>,
    sources: Mutex<Sources>,
    requests: RequestLimiter,
    storms: Mutex<StormDetector>,
    /// One fire at a time (the dispatcher and manual test fires)
    dispatching: tokio::sync::Mutex<()>,
    /// Calendar handovers over the relay (set once it's up)
    duty_network: Arc<OnceLock<Arc<dyn handover::DutyNetwork>>>,
}

impl TriggerEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Db,
        bus: EventBus,
        orchestrators: Orchestrators,
        workflows: Arc<WorkflowEngine>,
        broadcast: BroadcastSink,
        runtime: tokio::runtime::Handle,
        store: RepoStore,
        secrets: Secrets,
    ) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        Self {
            secrets,
            db,
            bus,
            orchestrators,
            workflows,
            broadcast,
            runtime,
            store,
            triggers: RwLock::default(),
            tx,
            rx: Mutex::new(Some(rx)),
            sources: Mutex::default(),
            requests: RequestLimiter::default(),
            storms: Mutex::default(),
            dispatching: tokio::sync::Mutex::const_new(()),
            duty_network: Arc::default(),
        }
    }

    /// Brings the projection up to date with the trigger files, loads the
    /// rules, registers sources, and starts dispatching. Call once.
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.rescan(EntityKind::Trigger, ChangedBy::Watcher))
            .await
            .map_err(|e| format!("Trigger scan failed: {e}"))?;
        self.reload().await?;

        let Some(mut rx) = self.rx.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return Ok(()); // already started
        };
        let weak = Arc::downgrade(self);
        let dispatcher = self.runtime.spawn(async move {
            while let Some(event) = rx.recv().await {
                let Some(engine) = weak.upgrade() else { return };
                let _turn = engine.dispatching.lock().await;
                dispatch::process(&engine, event).await;
            }
        });
        let subscriber = self.runtime.spawn(watch_bus(Arc::downgrade(self)));
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        sources.tasks.push(dispatcher.abort_handle());
        sources.tasks.push(subscriber.abort_handle());
        Ok(())
    }

    /// Stops every source and the dispatcher.
    pub fn stop(&self) {
        let mut guard = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        let sources = &mut *guard;
        for task in sources
            .tasks
            .drain(..)
            .chain(sources.schedules.drain().map(|(_, t)| t))
        {
            task.abort();
        }
        sources.files.clear();
    }

    /// Lets calendar triggers hand over to backups on other Nandis.
    pub fn set_duty_network(&self, network: Arc<dyn handover::DutyNetwork>) {
        let _ = self.duty_network.set(network);
    }

    /// Re-reads the rules (from the projection, secrets from the keyring)
    /// and re-registers cron and calendar jobs and file watchers.
    pub async fn reload(&self) -> Result<(), String> {
        let mut list = self.db.call(|conn| store::list_triggers(conn)).await?;
        for trigger in list.iter_mut() {
            self.attach_secret(trigger);
        }
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        for (_, job) in sources.schedules.drain() {
            job.abort();
        }
        sources.files.clear();
        for trigger in list.iter().filter(|t| t.enabled) {
            match trigger.source_type {
                TriggerSourceType::Cron => {
                    let job = schema::cron_config(&trigger.source_config)
                        .and_then(|c| cron::parse_schedule(&c.schedule));
                    match job {
                        Ok(schedule) => {
                            let handle = cron::spawn(
                                &self.runtime,
                                trigger.id.clone(),
                                schedule,
                                self.tx.clone(),
                            );
                            sources.schedules.insert(trigger.id.clone(), handle);
                        }
                        Err(e) => log::warn!("Cron trigger '{}' not scheduled: {e}", trigger.id),
                    }
                }
                TriggerSourceType::Calendar => match calendar::parse(&trigger.source_config) {
                    Ok(spec) => {
                        let deps = calendar::CalendarDeps {
                            runtime: self.runtime.clone(),
                            events: self.tx.clone(),
                            db: self.db.clone(),
                            network: self.duty_network.clone(),
                        };
                        let handle = calendar::spawn(deps, trigger.id.clone(), spec);
                        sources.schedules.insert(trigger.id.clone(), handle);
                    }
                    Err(e) => log::warn!("Calendar trigger '{}' not scheduled: {e}", trigger.id),
                },
                TriggerSourceType::File => {
                    let watch = schema::file_config(&trigger.source_config).and_then(|config| {
                        file::spawn(&self.runtime, trigger.id.clone(), config, self.tx.clone())
                    });
                    match watch {
                        Ok(watch) => {
                            sources.files.insert(trigger.id.clone(), watch);
                        }
                        // E.g. the folder was removed: keep the rule, warn
                        Err(e) => log::warn!("File trigger '{}' not watching: {e}", trigger.id),
                    }
                }
                _ => {}
            }
        }
        drop(sources);
        *self.triggers.write().unwrap_or_else(|e| e.into_inner()) =
            list.into_iter().map(|t| (t.id.clone(), t)).collect();
        Ok(())
    }

    fn trigger(&self, id: &str) -> Option<Trigger> {
        self.triggers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    fn all(&self) -> Vec<Trigger> {
        self.triggers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn record_fire(&self, id: &str, now: i64) -> bool {
        self.storms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(id, now)
    }

    /// Keeps the in-memory copy's `last_fired_at` current for rate limits.
    fn note_fired(&self, id: &str, at: i64) {
        if let Some(t) = self
            .triggers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
        {
            t.last_fired_at = Some(db::ts(at));
        }
    }

    /// Whether any cron job or file watcher is registered for `id`.
    #[cfg(test)]
    fn is_watching(&self, id: &str) -> bool {
        let sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        sources.schedules.contains_key(id) || sources.files.contains_key(id)
    }

    // --- Rules --------------------------------------------------------------

    /// Validates input into a rule. `existing` is the rule being updated.
    fn build(
        &self,
        input: TriggerInput,
        existing: Option<&Trigger>,
        check_paths: bool,
    ) -> Result<(Trigger, Option<String>), String> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err("Trigger name is empty".to_string());
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(format!(
                "Trigger name is longer than {MAX_NAME_CHARS} characters"
            ));
        }
        let id = match (existing, input.id) {
            (Some(existing), _) => existing.id.clone(),
            (None, Some(id)) => {
                if self.trigger(&id).is_some() {
                    return Err(format!("A trigger with id '{id}' already exists"));
                }
                id
            }
            (None, None) => {
                let base = schema::slugify(&name);
                let mut id = base.clone();
                let mut n = 2;
                while self.trigger(&id).is_some() {
                    id = format!("{base}-{n}");
                    n += 1;
                }
                id
            }
        };
        agents::validate_agent_id(&id).map_err(|_| format!("Invalid trigger id '{id}'"))?;
        agents::validate_agent_id(&input.target_id)
            .map_err(|_| format!("Invalid target '{}'", input.target_id))?;

        let mut config =
            schema::validate_config(input.source_type, &input.source_config, check_paths)?;
        // Secrets are generated here, never taken from clients or files
        let mut new_secret = None;
        if let Some(map) = config.as_object_mut() {
            map.remove("secret");
            // A watched folder is stored portably, like shared folders
            if input.source_type == TriggerSourceType::File {
                if let Some(Value::String(path)) = map.get_mut("path") {
                    *path = crate::services::connectors::folders::portable(
                        path,
                        crate::utils::nandi_config::nandi_dir().ok().as_deref(),
                        dirs::home_dir().as_deref(),
                    );
                }
            }
            if input.source_type.has_secret() {
                let kept = existing
                    .filter(|e| e.source_type.has_secret())
                    .and_then(|e| e.secret().map(str::to_string));
                let secret = kept.unwrap_or_else(|| {
                    let secret = schema::generate_secret();
                    new_secret = Some(secret.clone());
                    secret
                });
                map.insert("secret".into(), Value::String(secret));
            }
        }

        let template = input.input_template.filter(|t| !t.trim().is_empty());
        if template
            .as_ref()
            .is_some_and(|t| t.chars().count() > MAX_TEMPLATE_CHARS)
        {
            return Err(format!(
                "Input template is longer than {MAX_TEMPLATE_CHARS} characters"
            ));
        }
        let now = db::now_ms();
        let mut trigger = Trigger {
            id,
            name,
            enabled: input.enabled.unwrap_or(true),
            source_type: input.source_type,
            source_config: config,
            target_type: input.target_type,
            target_id: input.target_id,
            input_template: template,
            priority: input
                .priority
                .unwrap_or_else(|| input.source_type.default_priority()),
            rate_limit_seconds: input.rate_limit_seconds.filter(|s| *s > 0),
            created_at: existing.map_or(db::ts(now), |e| e.created_at),
            updated_at: db::ts(now),
            last_fired_at: existing.and_then(|e| e.last_fired_at),
            has_secret: false,
        };
        trigger.has_secret = trigger.secret().is_some();
        Ok((trigger, new_secret))
    }

    /// The write path: the domain validates the rule, its secret goes to
    /// the keyring, the repository writes (and stages) the file, and the
    /// commit updates the aggregate and projection and publishes
    /// `repo://trigger-*`. Then the rules are reloaded.
    async fn save(&self, trigger: Trigger) -> Result<(), String> {
        let entity = repo::to_domain(&trigger)?;
        if let (Some(secret_ref), Some(secret)) = (entity.secret_ref(), trigger.secret()) {
            self.secrets.set(secret_ref.as_str(), secret)?;
        }
        // Inbound fires see the change before the projection does: once
        // SQLite says a trigger is disabled, its webhook already refuses
        self.triggers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(trigger.id.clone(), trigger);
        let store = self.store.clone();
        let written = tokio::task::spawn_blocking(move || {
            store.repos().triggers.save(&entity)?;
            store.commit(EntityKind::Trigger, entity.id().as_str())?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| format!("Trigger write failed: {e}"))
        .and_then(|written| written);
        // The files are the truth: a failed write puts the old rule back
        let reloaded = self.reload().await;
        written.and(reloaded)
    }

    /// Puts the trigger's secret (from the keyring) into its config. A
    /// webhook/device trigger without one (e.g. a repo cloned onto a new
    /// machine) gets a fresh one.
    fn attach_secret(&self, trigger: &mut Trigger) {
        if !trigger.source_type.has_secret() {
            return;
        }
        let Some(secret_ref) = repo::to_domain(trigger).ok().and_then(|t| t.secret_ref()) else {
            return;
        };
        let secret = match self.secrets.get(secret_ref.as_str()) {
            Ok(Some(secret)) => secret,
            Ok(None) => {
                // A reload that read the trigger list before a delete must
                // not bring the deleted trigger's secret back
                if !self.store.layout().trigger_path(&trigger.id).exists() {
                    return;
                }
                let secret = schema::generate_secret();
                if let Err(e) = self.secrets.set(secret_ref.as_str(), &secret) {
                    log::warn!(
                        "Trigger '{}' has no secret and one couldn't be stored: {e}",
                        trigger.id
                    );
                    return;
                }
                log::info!("Generated a secret for trigger '{}'", trigger.id);
                secret
            }
            Err(e) => {
                log::warn!("Failed to read trigger '{}' secret: {e}", trigger.id);
                return;
            }
        };
        if let Some(map) = trigger.source_config.as_object_mut() {
            map.insert("secret".into(), Value::String(secret));
        }
        trigger.has_secret = true;
    }

    pub async fn create(&self, input: TriggerInput) -> Result<SavedTrigger, String> {
        let (trigger, secret) = self.build(input, None, true)?;
        self.save(trigger.clone()).await?;
        log::info!("Created trigger '{}'", trigger.id);
        Ok(SavedTrigger {
            trigger: trigger.public(),
            secret,
        })
    }

    pub async fn update(&self, id: &str, input: TriggerInput) -> Result<SavedTrigger, String> {
        let existing = self.get(id)?;
        let (trigger, secret) = self.build(input, Some(&existing), true)?;
        self.save(trigger.clone()).await?;
        Ok(SavedTrigger {
            trigger: trigger.public(),
            secret,
        })
    }

    /// Removes the file (staged), the projection row, the secret, and the
    /// fire history.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let trigger = self.get(id)?;
        let entity = repo::to_domain(&trigger).ok();
        let (store, trigger_id) = (self.store.clone(), TriggerId::parse(id)?);
        tokio::task::spawn_blocking(move || {
            store.repos().triggers.delete(&trigger_id)?;
            store.commit(EntityKind::Trigger, trigger_id.as_str())?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| format!("Trigger delete failed: {e}"))??;
        let id_owned = id.to_string();
        self.db
            .call(move |conn| store::delete_history(conn, &id_owned))
            .await?;
        self.requests.forget(id);
        self.storms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget(id);
        let reloaded = self.reload().await;
        // After the reload, so it can't attach (and so re-create) the secret
        if let Some(secret_ref) = entity.and_then(|t| t.secret_ref()) {
            if let Err(e) = self.secrets.delete(secret_ref.as_str()) {
                log::warn!("Trigger '{id}' deleted, but not its secret: {e}");
            }
        }
        log::info!("Deleted trigger '{id}'");
        reloaded
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let mut trigger = self.get(id)?;
        trigger.enabled = enabled;
        trigger.updated_at = db::ts(db::now_ms());
        if enabled {
            // A fresh start after a storm
            self.storms
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .forget(id);
        }
        self.save(trigger).await
    }

    /// Replaces a webhook/device trigger's secret; the new one is returned
    /// (shown once).
    pub async fn rotate_secret(&self, id: &str) -> Result<SavedTrigger, String> {
        let mut trigger = self.get(id)?;
        if !trigger.source_type.has_secret() {
            return Err("Only webhook and device triggers have a secret".to_string());
        }
        let secret = schema::generate_secret();
        if let Some(map) = trigger.source_config.as_object_mut() {
            map.insert("secret".into(), Value::String(secret.clone()));
        }
        trigger.updated_at = db::ts(db::now_ms());
        trigger.has_secret = true;
        self.save(trigger.clone()).await?;
        Ok(SavedTrigger {
            trigger: trigger.public(),
            secret: Some(secret),
        })
    }

    /// The full rule (with its secret), for internal use.
    fn get(&self, id: &str) -> Result<Trigger, String> {
        self.trigger(id)
            .ok_or_else(|| format!("Trigger '{id}' not found"))
    }

    /// Fires a trigger by hand (even if disabled; no storm or rate limit).
    pub async fn test_fire(&self, id: &str, payload: Value) -> Result<TriggerFire, String> {
        let trigger = self.get(id)?;
        let _turn = self.dispatching.lock().await;
        dispatch::process(
            self,
            TriggerEvent {
                trigger_id: trigger.id,
                source: trigger.source_type,
                payload,
                test: true,
            },
        )
        .await
        .ok_or_else(|| "The fire couldn't be recorded".to_string())
    }

    // --- Receivers ----------------------------------------------------------

    fn accept(&self, event: TriggerEvent) -> Result<(), ReceiveError> {
        if !self.requests.allow(&event.trigger_id) {
            return Err(ReceiveError::RateLimited);
        }
        self.tx.try_send(event).map_err(|_| ReceiveError::Busy)
    }

    /// A webhook call. Hands the fire to the dispatcher and returns at once.
    pub fn receive_webhook(
        &self,
        id: &str,
        secret: Option<&str>,
        payload: Value,
    ) -> Result<(), ReceiveError> {
        let trigger = self
            .trigger(id)
            .filter(|t| t.enabled && t.source_type == TriggerSourceType::Webhook)
            .ok_or(ReceiveError::NotFound)?;
        let authorized = matches!(
            (trigger.secret(), secret),
            (Some(expected), Some(given)) if schema::secrets_match(expected, given)
        );
        if !authorized {
            return Err(ReceiveError::Unauthorized);
        }
        self.accept(TriggerEvent {
            trigger_id: trigger.id,
            source: TriggerSourceType::Webhook,
            payload,
            test: false,
        })
    }

    /// A device event. Returns how many triggers it fired.
    pub fn receive_device(
        &self,
        device_id: &str,
        secret: Option<&str>,
        event: DeviceEvent,
    ) -> Result<usize, ReceiveError> {
        let all = self.all();
        let targets = match device::match_triggers(&all, device_id, secret, &event.event) {
            DeviceMatch::UnknownDevice => return Err(ReceiveError::NotFound),
            DeviceMatch::WrongSecret => return Err(ReceiveError::Unauthorized),
            DeviceMatch::Fire(targets) => targets,
        };
        let payload = device::payload(device_id, &event);
        let mut fired = 0;
        let mut limited = false;
        for trigger in targets {
            match self.accept(TriggerEvent {
                trigger_id: trigger.id.clone(),
                source: TriggerSourceType::Device,
                payload: payload.clone(),
                test: false,
            }) {
                Ok(()) => fired += 1,
                Err(ReceiveError::RateLimited) => limited = true,
                Err(e) => return Err(e),
            }
        }
        if fired == 0 && limited {
            return Err(ReceiveError::RateLimited);
        }
        Ok(fired)
    }
}

/// Feeds Nandi's own events to `event` triggers.
async fn watch_bus(engine: Weak<TriggerEngine>) {
    let Some(mut events) = engine.upgrade().map(|e| e.broadcast.subscribe()) else {
        return;
    };
    loop {
        let wire = match events.recv().await {
            Ok(wire) => wire,
            Err(RecvError::Lagged(missed)) => {
                log::warn!("Event triggers missed {missed} events");
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        let Some(engine) = engine.upgrade() else {
            return;
        };
        let watching: Vec<Trigger> = engine
            .all()
            .into_iter()
            .filter(|t| t.enabled && t.source_type == TriggerSourceType::Event)
            .collect();
        if watching.is_empty() {
            continue;
        }
        let Some((name, payload)) = event::translate(&engine.db, &wire).await else {
            continue;
        };
        for trigger in watching {
            let Ok(config) = schema::event_config(&trigger.source_config) else {
                continue;
            };
            if config.event_name == name && event::filter_matches(&config.filter, &payload) {
                let fire = TriggerEvent {
                    trigger_id: trigger.id,
                    source: TriggerSourceType::Event,
                    payload: payload.clone(),
                    test: false,
                };
                if engine.tx.send(fire).await.is_err() {
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Service API (shared by the Tauri commands and HTTP routes)
// ---------------------------------------------------------------------------

pub async fn list_triggers(state: &AppState) -> Result<Vec<Trigger>, String> {
    let list = state.db().call(|conn| store::list_triggers(conn)).await?;
    Ok(list.iter().map(Trigger::public).collect())
}

pub async fn get_trigger(state: &AppState, id: String) -> Result<Trigger, String> {
    state
        .db()
        .call(move |conn| store::get_trigger(conn, &id))
        .await?
        .map(|t| t.public())
        .ok_or_else(|| "Trigger not found".to_string())
}

pub async fn create_trigger(state: &AppState, input: TriggerInput) -> Result<SavedTrigger, String> {
    state.triggers().create(input).await
}

pub async fn update_trigger(
    state: &AppState,
    id: String,
    input: TriggerInput,
) -> Result<SavedTrigger, String> {
    state.triggers().update(&id, input).await
}

pub async fn delete_trigger(state: &AppState, id: String) -> Result<(), String> {
    state.triggers().delete(&id).await
}

pub async fn set_trigger_enabled(
    state: &AppState,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state.triggers().set_enabled(&id, enabled).await
}

pub async fn rotate_trigger_secret(state: &AppState, id: String) -> Result<SavedTrigger, String> {
    state.triggers().rotate_secret(&id).await
}

pub async fn test_fire_trigger(
    state: &AppState,
    id: String,
    payload: Option<Value>,
) -> Result<TriggerFire, String> {
    let payload = payload.unwrap_or_else(|| serde_json::json!({ "test": true }));
    if payload.to_string().len() > MAX_PAYLOAD_BYTES {
        return Err("Test payload is too large".to_string());
    }
    state.triggers().test_fire(&id, payload).await
}

pub async fn list_trigger_fires(
    state: &AppState,
    trigger_id: String,
    limit: Option<u32>,
) -> Result<Vec<TriggerFire>, String> {
    let limit = limit.unwrap_or(DEFAULT_FIRE_LIMIT).clamp(1, 500);
    state
        .db()
        .call(move |conn| store::list_fires(conn, &trigger_id, limit))
        .await
}
