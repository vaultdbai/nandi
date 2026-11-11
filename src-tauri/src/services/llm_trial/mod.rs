//! The free trial: [`TRIAL_MODEL`] through Nandi's trial gateway, for a
//! new user who has no model of their own yet.
//!
//! The DeepSeek key lives only on the gateway ([`gateway`]); the app
//! signs each call with this instance's device key and holds no key and no
//! counter. The gateway gives each computer 100,000 tokens (shared by the
//! Nandis on it) and, once linked to the person's connected email
//! ([`link`], with the mailbox's ID token; no sign-in), one 100,000 across
//! all their computers. It is the only count: Settings shows what `GET
//! /v1/trial` says. Like every model call the trial runs only on the host
//! and never enters a container.
//!
//! The trial has three [`TrialMode`]s, kept in the keychain. It starts in
//! `Auto`: calls use it only when the user has nothing of their own
//! (`llm_keys::usable_model` tries their keys and sign-ins first). The
//! first time a call on a model of their own works (a saved key, a Claude
//! Code sign-in, Ollama, a model from `llm.yaml`) it turns `Off` for good.
//! A sign-in that's only found, not proven, doesn't count: a stale login
//! would leave them with nothing. The user can turn it back `On` in
//! Settings › Models & runtimes while tokens are left; then the default
//! model runs on it again until they're used up or the user saves another
//! key. When the keychain can't be read, the trial counts as off.
//!
//! What the gateway last said is kept in memory ([`Health`]): used up,
//! paused until a time (its daily cap), or unreachable for a minute. While
//! any of those holds, calls don't offer the trial, so a computer with no
//! model at all gets the built-in help guide (`services::help`) at once
//! instead of an error after a timeout.

pub mod gateway;

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use tokio::sync::mpsc;

use crate::llm::{LlmRequest, LlmResponse};
use crate::services::identity::{self, machine_id};
use crate::services::secrets::Secrets;
use crate::services::AppState;

pub use gateway::{GatewayClient, GatewayError, TrialAccount, TrialLink};

/// The one model the trial runs (the cheapest DeepSeek model)
pub const TRIAL_MODEL: &str = "deepseek:deepseek-flash";
/// Keychain entry holding the default model from before the user turned
/// the trial on, so turning it off gives it back
const PREVIOUS_ENTRY: &str = "nandi.trial.deepseek.previous-model";
/// Keychain entry holding the [`TrialMode`] (`on` / `off`; none is `Auto`)
const MODE_ENTRY: &str = "nandi.trial.deepseek.mode";

pub const TRIAL_USED_UP: &str = "Your free tokens are used up. Add your own key (Anthropic, OpenAI, or DeepSeek) in Settings › Models & runtimes, sign in to Claude Code (`claude`), or run a local model with Ollama.";
const TRIAL_PAUSED: &str = "The free trial is paused for today: it has a daily limit shared by everyone. It's back after midnight UTC.";
const TRIAL_OFFLINE: &str = "Can't reach the free trial right now. Check this computer's internet connection, or set up your own model in Settings › Models & runtimes.";

/// How long an unreachable gateway isn't offered before it's tried again
const UNREACHABLE_FOR: Duration = Duration::from_secs(60);
/// How long a paused trial (per `GET /v1/trial`, which has no
/// `Retry-After`) isn't offered before it's tried again
const PAUSED_RECHECK: Duration = Duration::from_secs(5 * 60);

/// Whether model calls may use the trial
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialMode {
    /// Only while the user has no model of their own (the start)
    Auto,
    /// Never: the user has their own model, or turned it off
    Off,
    /// The user turned it back on: the default model runs on it
    On,
}

/// What the gateway last said
#[derive(Debug, Default)]
struct Health {
    used_up: bool,
    paused_until: Option<Instant>,
    unreachable_until: Option<Instant>,
}

impl Health {
    fn paused(&self) -> bool {
        self.paused_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn unreachable(&self) -> bool {
        self.unreachable_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn usable(&self) -> bool {
        !self.used_up && !self.paused() && !self.unreachable()
    }
}

/// This instance's trial: the gateway it signs calls for, and what the
/// gateway last said
pub struct Trial {
    base_url: String,
    instance: String,
    client: OnceLock<Arc<GatewayClient>>,
    health: Mutex<Health>,
}

impl Trial {
    pub fn new(base_url: String, instance: String) -> Self {
        Self {
            base_url,
            instance,
            client: OnceLock::new(),
            health: Mutex::new(Health::default()),
        }
    }

    fn health(&self) -> std::sync::MutexGuard<'_, Health> {
        self.health.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The signed client, with this device's keys (made on first use, like
    /// the relay's)
    fn client(&self, secrets: &Secrets) -> Result<Arc<GatewayClient>, GatewayError> {
        if let Some(client) = self.client.get() {
            return Ok(client.clone());
        }
        let keys = crate::services::relay::device_keys::load_or_create(secrets)
            .map_err(GatewayError::Keys)?;
        let machine =
            machine_id::this_machine(machine_id::Service::Trial).map_err(GatewayError::Machine)?;
        let client = Arc::new(GatewayClient::new(
            self.base_url.clone(),
            self.instance.clone(),
            machine,
            keys,
        ));
        Ok(self.client.get_or_init(|| client).clone())
    }

    /// Keeps what an answer says about the trial
    fn note<T>(&self, result: &Result<T, GatewayError>) {
        let mut health = self.health();
        match result {
            Ok(_) => {
                health.unreachable_until = None;
                health.paused_until = None;
            }
            Err(GatewayError::Unreachable(_)) => {
                health.unreachable_until = Some(Instant::now() + UNREACHABLE_FOR);
            }
            Err(error) => match error.code() {
                Some("trial_used_up") => health.used_up = true,
                Some("trial_paused") => {
                    let seconds = error.retry_after().unwrap_or(PAUSED_RECHECK.as_secs());
                    health.paused_until = Some(Instant::now() + Duration::from_secs(seconds));
                }
                _ => health.unreachable_until = None,
            },
        }
    }

    /// Keeps what `GET /v1/trial` says
    fn note_account(&self, account: &TrialAccount) {
        let mut health = self.health();
        health.used_up = account.used >= account.limit;
        health.unreachable_until = None;
        health.paused_until = account.paused.then(|| Instant::now() + PAUSED_RECHECK);
    }
}

/// The trial the app runs, set up at start (`init`)
static TRIAL: OnceLock<Arc<Trial>> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// Tests use their own trial (a fake gateway), on their own thread
    static TEST_TRIAL: std::cell::RefCell<Option<Arc<Trial>>> =
        const { std::cell::RefCell::new(None) };
}

/// Sets up the trial for this instance, on the gateway from
/// `NANDI_TRIAL_GATEWAY_URL` or the default one
pub fn init(instance_id: String) {
    let url = gateway::gateway_url(crate::utils::env::var(gateway::GATEWAY_URL_ENV).as_deref());
    log::info!("Free trial gateway: {url}");
    let _ = TRIAL.set(Arc::new(Trial::new(url, instance_id)));
}

/// The trial, once `init` ran. Tests never reach the real gateway: they
/// see only the trial they set ([`use_test_trial`]).
fn trial() -> Option<Arc<Trial>> {
    #[cfg(test)]
    {
        TEST_TRIAL.with(|cell| cell.borrow().clone())
    }
    #[cfg(not(test))]
    {
        TRIAL.get().cloned()
    }
}

/// Makes this test thread's calls use `trial` (`None`: no trial)
#[cfg(test)]
pub fn use_test_trial(trial: Option<Arc<Trial>>) {
    TEST_TRIAL.with(|cell| *cell.borrow_mut() = trial);
}

/// The trial's state for Settings
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrialStatus {
    /// Not turned off (by the user, or by their own model)
    pub enabled: bool,
    /// The default model runs on the trial right now
    pub active: bool,
    /// The gateway said the tokens are used up
    pub used_up: bool,
    /// The gateway said the trial is paused for today
    pub paused: bool,
    /// The gateway couldn't be reached a moment ago
    pub unreachable: bool,
}

fn mode(secrets: &Secrets) -> TrialMode {
    match secrets.read(MODE_ENTRY) {
        Ok(None) => TrialMode::Auto,
        Ok(Some(value)) => match value.as_str() {
            "on" => TrialMode::On,
            "off" => TrialMode::Off,
            _ => TrialMode::Auto,
        },
        Err(e) => {
            log::warn!("Free trial: {e}");
            TrialMode::Off
        }
    }
}

fn set_mode(secrets: &Secrets, mode: TrialMode) -> Result<(), String> {
    match mode {
        TrialMode::Auto => secrets.delete(MODE_ENTRY),
        TrialMode::Off => secrets.set(MODE_ENTRY, "off"),
        TrialMode::On => secrets.set(MODE_ENTRY, "on"),
    }
}

/// Calls may use the trial (set up, not off, and the gateway didn't say
/// it's used up, paused or unreachable)
pub fn available(secrets: &Secrets) -> bool {
    available_with(trial().as_deref(), secrets)
}

fn available_with(trial: Option<&Trial>, secrets: &Secrets) -> bool {
    trial.is_some_and(|trial| trial.health().usable()) && mode(secrets) != TrialMode::Off
}

/// The user turned the trial back on and it's usable: `deepseek:` calls
/// run on it before any key
pub fn chosen(secrets: &Secrets) -> bool {
    chosen_with(trial().as_deref(), secrets)
}

fn chosen_with(trial: Option<&Trial>, secrets: &Secrets) -> bool {
    trial.is_some_and(|trial| trial.health().usable()) && mode(secrets) == TrialMode::On
}

/// Why calls can't use a trial that isn't off: used up, paused, or
/// unreachable. `None` when it's usable, off, or there's none.
pub fn why_unavailable(secrets: &Secrets) -> Option<&'static str> {
    why_unavailable_with(trial().as_deref(), secrets)
}

fn why_unavailable_with(trial: Option<&Trial>, secrets: &Secrets) -> Option<&'static str> {
    let trial = trial?;
    let health = trial.health();
    if health.used_up {
        return Some(TRIAL_USED_UP);
    }
    if mode(secrets) == TrialMode::Off {
        return None;
    }
    if health.paused() {
        Some(TRIAL_PAUSED)
    } else if health.unreachable() {
        Some(TRIAL_OFFLINE)
    } else {
        None
    }
}

/// A call on the user's own model just worked: an `Auto` trial turns off
/// for good (a trial the user turned `On` stays on). Checked once per run
/// of the app, so calls don't each read the keychain.
pub fn own_model_worked(secrets: &Secrets) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        own_model_found_with(trial().is_some(), secrets);
    }
}

fn own_model_found_with(has_trial: bool, secrets: &Secrets) {
    if has_trial && mode(secrets) == TrialMode::Auto {
        match set_mode(secrets, TrialMode::Off) {
            Ok(()) => log::info!("The user has their own model; the free trial is off"),
            Err(e) => log::warn!("Could not turn off the free trial: {e}"),
        }
    }
}

/// The user just set up a model of their own (saved a key, chose Ollama):
/// the trial turns off, even one they had turned back on
pub fn own_model_set_up(secrets: &Secrets) {
    own_model_set_up_with(trial().is_some(), secrets);
}

fn own_model_set_up_with(has_trial: bool, secrets: &Secrets) {
    if has_trial && mode(secrets) != TrialMode::Off {
        match set_mode(secrets, TrialMode::Off) {
            Ok(()) => log::info!("The user set up their own model; the free trial is off"),
            Err(e) => log::warn!("Could not turn off the free trial: {e}"),
        }
    }
}

/// Turns the trial on (unless the gateway said it's used up) or off
pub fn set_enabled(secrets: &Secrets, enabled: bool) -> Result<(), String> {
    set_enabled_with(trial().as_deref(), secrets, enabled)
}

fn set_enabled_with(trial: Option<&Trial>, secrets: &Secrets, enabled: bool) -> Result<(), String> {
    let Some(trial) = trial else {
        return Err("The free trial isn't set up in this Nandi.".to_string());
    };
    if !enabled {
        return set_mode(secrets, TrialMode::Off);
    }
    if trial.health().used_up {
        return Err(TRIAL_USED_UP.to_string());
    }
    set_mode(secrets, TrialMode::On)
}

/// The trial's state, `None` when there's none (tests). `active` says
/// whether calls use it now (from `llm_keys`).
pub fn status(secrets: &Secrets, active: bool) -> Option<TrialStatus> {
    let trial = trial()?;
    let health = trial.health();
    Some(TrialStatus {
        enabled: mode(secrets) != TrialMode::Off,
        active,
        used_up: health.used_up,
        paused: health.paused(),
        unreachable: health.unreachable(),
    })
}

fn require(secrets: &Secrets) -> Result<(Arc<Trial>, Arc<GatewayClient>), String> {
    let trial = trial().ok_or_else(|| "The free trial isn't set up in this Nandi.".to_string())?;
    let client = trial.client(secrets).map_err(|e| e.user_message())?;
    Ok((trial, client))
}

/// One call on the trial's model through the gateway, streamed like a
/// DeepSeek call (`llm::dispatch`)
pub async fn chat(
    secrets: &Secrets,
    http: &reqwest::Client,
    request: &LlmRequest,
    max_tokens: u32,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    if !available(secrets) {
        return Err(why_unavailable(secrets)
            .unwrap_or("DeepSeek API key not configured. Add it in Settings › Models & runtimes.")
            .to_string());
    }
    let (trial, client) = require(secrets)?;
    let (_, model) = crate::llm::parse_model(TRIAL_MODEL)?;
    let result = client.chat(http, model, request, max_tokens, text).await;
    trial.note(&result);
    result.map_err(|e| e.user_message())
}

/// `GET /v1/trial`: the allowance as the gateway counts it, for Settings.
/// Not linked yet while an email is connected: links first.
pub async fn account(state: &AppState) -> Result<TrialAccount, String> {
    let (trial, client) = require(state.secrets())?;
    let mut result = client.account().await;
    let unlinked = matches!(&result, Ok(account) if account.account.is_none());
    if unlinked && identity::identity(state).is_some() {
        match link_with(state, &trial, &client).await {
            Ok(_) => result = client.account().await,
            Err(e) => log::info!("Free trial not linked to the email yet: {e}"),
        }
    }
    match &result {
        Ok(account) => trial.note_account(account),
        Err(_) => trial.note(&result),
    }
    result.map_err(|e| e.user_message())
}

/// Links this computer's trial to the person's connected email (the
/// mailbox's fresh ID token). Linking again is a no-op at the gateway.
pub async fn link(state: &AppState) -> Result<TrialLink, String> {
    let (trial, client) = require(state.secrets())?;
    link_with(state, &trial, &client).await
}

async fn link_with(
    state: &AppState,
    trial: &Trial,
    client: &GatewayClient,
) -> Result<TrialLink, String> {
    let (id_token, _) = identity::fresh_id_token(state)
        .await
        .map_err(|e| e.message())?;
    let linked = client.link(&id_token).await.map_err(|e| e.user_message())?;
    trial.health().used_up = linked.used >= linked.limit;
    Ok(linked)
}

/// Links in the background (after a mailbox connected); a miss is tried
/// again the next time Settings reads the trial
pub fn link_soon(state: &AppState) {
    if trial().is_none() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        match link(&state).await {
            Ok(linked) => log::info!(
                "Free trial linked to the connected email ({})",
                linked.account
            ),
            Err(e) => log::info!("Free trial not linked to the email yet: {e}"),
        }
    });
}

/// Keeps the default model from before the trial was turned on
pub fn remember_default(secrets: &Secrets, model: &str) {
    if model == TRIAL_MODEL {
        return;
    }
    if let Err(e) = secrets.set(PREVIOUS_ENTRY, model) {
        log::warn!("Could not remember the default model: {e}");
    }
}

/// The default model from before the trial was turned on, once
pub fn take_remembered_default(secrets: &Secrets) -> Option<String> {
    let model = secrets.read(PREVIOUS_ENTRY).ok().flatten()?;
    let _ = secrets.delete(PREVIOUS_ENTRY);
    Some(model)
}

#[cfg(test)]
mod tests;
