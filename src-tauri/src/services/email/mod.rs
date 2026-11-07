//! Email: the Chief of Staff reads the user's mail (Outlook or Gmail) and
//! keeps My notes up to date. Read-only: Nandi reads mail and writes notes;
//! nothing is ever sent, moved, deleted or marked read.
//!
//! ```text
//! Connect (Settings › Email) ─ sign-in ─▶ keyring + account + grant (chief-of-staff, read)
//! every N minutes (or Check now):
//!   each mailbox ─ changes since cursor ─▶ filter (bulk, notes to self, watch list)
//!   all new mail ─▶ ONE Chief of Staff run (headers + previews, untrusted) ─▶ cursors saved
//! the run: mail_* tools (read-only, on the host) + notes tools, nothing else
//! ```
//!
//! - **Our own connector, not an MCP server.** Tokens stay in the host's
//!   keyring; the agent's tool calls run here, never in a container.
//! - **Notes, not copies.** Only the change cursor and the notes the Chief
//!   of Staff writes are kept. See [`settings`] for what lives where.
//! - **One run per check**, and only when there's new mail (model cost).
//!   The first check after connecting reads the last [`READ_WINDOW_DAYS`]
//!   days, capped at [`batch::MAX_BATCH`] emails.
//! - A refused token refresh marks the mailbox "needs sign-in" (Google's
//!   Testing mode ends sign-ins every 7 days); the UI offers Reconnect.

mod api;
pub mod backfill;
pub mod batch;
mod filter;
mod gmail;
mod mail;
mod mailbox;
mod outlook;
pub mod provider;
pub mod settings;
mod text;
pub mod tool;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

pub use backfill::{BackfillAction, BackfillEstimate, BackfillStatus};
pub use provider::{MailProvider, ProviderSetup};
pub use settings::{BackfillRange, ConnectorGrant, EmailSettings, GrantAccess};

use crate::db;
use crate::model::{SourceType, Timestamp};
use crate::services::auth::oauth::{self, DeviceFlowState, OAuthFlows};
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::events::{self, NandiEvent};
use crate::services::orchestrator::{RunSource, PRIORITY_SCHEDULED};
use crate::services::AppState;
use crate::utils::nandi_config::{self, NandiConfig};
use api::MailError;
use mail::Identity;
use mailbox::Mailbox;
use settings::{CheckState, EmailAccount};

/// The first read (and a restart after an expired cursor) goes back this far
pub const READ_WINDOW_DAYS: i64 = 14;
/// How often the scheduler looks whether a check is due
const TICK: Duration = Duration::from_secs(20);
/// The first check after launch catches up from the cursor
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// What clients see
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EmailAccountState {
    NotConnected,
    Ready,
    Paused,
    /// The sign-in ended or was refused: Reconnect
    NeedsSignIn,
    /// The last check failed (network, the provider); it tries again
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EmailProviderStatus {
    pub provider: MailProvider,
    /// This build has the provider's client ID
    pub available: bool,
    pub connected: bool,
    pub account_name: Option<String>,
    pub account_email: Option<String>,
    pub state: EmailAccountState,
    pub error: Option<String>,
    pub last_checked_at: Option<Timestamp>,
    /// Emails the last check handed to the Chief of Staff
    pub last_found: u32,
    pub last_run_id: Option<String>,
    /// The background read of older mail
    pub backfill: BackfillStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EmailStatus {
    /// Outlook, then Gmail
    pub providers: Vec<EmailProviderStatus>,
    pub interval_minutes: u32,
    pub watch: Vec<String>,
    pub next_check_at: Option<Timestamp>,
    pub checking: bool,
    /// How far back the background read of older mail goes
    pub backfill_range: BackfillRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EmailSettingsInput {
    pub interval_minutes: u32,
    pub watch: Vec<String>,
    pub backfill: BackfillRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SignInPhase {
    Waiting,
    Connected,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SignInProblemKind {
    /// The company needs an admin to approve Nandi first
    AdminApproval,
    /// The user said no on the consent page
    Declined,
    Network,
    /// The sign-in took too long
    Expired,
    /// Nandi's app registration is set up for another kind of app (the
    /// redirect under "Web" or "Single-page application" instead of
    /// "Mobile and desktop applications"): a fix for whoever runs it
    AppRegistration,
    /// Signed in, but the sign-in couldn't be saved to the keychain
    Keychain,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SignInProblem {
    pub kind: SignInProblemKind,
    /// The provider's words, for the details line
    pub detail: String,
    /// For `adminApproval`: the page to forward to IT
    pub admin_consent_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EmailSignInPoll {
    pub phase: SignInPhase,
    pub problem: Option<SignInProblem>,
}

/// `email://changed`: a mailbox connected, disconnected, or was checked
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EmailChanged {}

impl NandiEvent for EmailChanged {
    const NAME: &'static str = "email://changed";
}

/// Sorts a sign-in failure into something the UI can explain calmly
pub fn classify_sign_in_error(error: &str, setup: &ProviderSetup) -> SignInProblem {
    let lower = error.to_ascii_lowercase();
    let admin = [
        "aadsts65001",
        "aadsts90094",
        "aadsts90095",
        "admin_consent",
        "admin approval",
    ]
    .iter()
    .any(|code| lower.contains(code));
    let registration = [
        "aadsts7000218",
        "aadsts9002327",
        "aadsts50011",
        "aadsts700025",
    ]
    .iter()
    .any(|code| lower.contains(code));
    let kind = if admin {
        SignInProblemKind::AdminApproval
    } else if registration {
        SignInProblemKind::AppRegistration
    } else if lower.contains("couldn't save your sign-in") {
        SignInProblemKind::Keychain
    } else if lower.contains("could not reach") || lower.contains("couldn't reach") {
        SignInProblemKind::Network
    } else if lower.contains("access_denied")
        || lower.contains("cancelled")
        || lower.contains("aadsts65004")
        || lower.contains("consent_required")
    {
        SignInProblemKind::Declined
    } else if lower.contains("expired") {
        SignInProblemKind::Expired
    } else {
        SignInProblemKind::Other
    };
    SignInProblem {
        kind,
        detail: error.to_string(),
        admin_consent_url: (kind == SignInProblemKind::AdminApproval)
            .then(|| setup.admin_consent_url())
            .flatten(),
    }
}

// ---------------------------------------------------------------------------
// Where a check's batch goes
// ---------------------------------------------------------------------------

/// Hands one check's message to the agent that keeps the notes; returns
/// the run's ID
pub trait BatchSink: Send + Sync {
    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        title: String,
        text: String,
    ) -> BoxFuture<'a, Result<String, String>>;
}

/// The Chief of Staff, in a conversation of the check's own
struct ChiefOfStaff;

impl BatchSink for ChiefOfStaff {
    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        title: String,
        text: String,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            state.store().require_agent(CHIEF_OF_STAFF)?;
            let check_id = uuid::Uuid::new_v4().to_string();
            let label = title.clone();
            let conversation = state
                .db()
                .call(move |conn| {
                    db::ensure_email_conversation(conn, CHIEF_OF_STAFF, &check_id, &title)
                })
                .await?;
            let source = RunSource {
                source_type: SourceType::Email,
                source_id: Some("email".into()),
                source_label: Some(label),
                priority: PRIORITY_SCHEDULED,
            };
            crate::services::chat::submit(
                state.orchestrators(),
                CHIEF_OF_STAFF,
                conversation,
                text,
                Vec::new(),
                None,
                source,
            )
            .await
            .map(|sent| sent.run_id)
        })
    }
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

pub struct EmailService {
    setup: RwLock<ProviderSetup>,
    sink: RwLock<Arc<dyn BatchSink>>,
    flows: OAuthFlows,
    /// One check at a time
    check_lock: tokio::sync::Mutex<()>,
    checking: AtomicBool,
    /// Check right after connecting or resuming (off in tests, which
    /// check by hand)
    auto_check: AtomicBool,
    next_check: Mutex<Option<Timestamp>>,
    wake: tokio::sync::Notify,
}

impl Default for EmailService {
    fn default() -> Self {
        Self::new(ProviderSetup::of_this_build())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> Timestamp {
    db::now_ms() as Timestamp
}

impl EmailService {
    pub fn new(setup: ProviderSetup) -> Self {
        Self {
            setup: RwLock::new(setup),
            sink: RwLock::new(Arc::new(ChiefOfStaff)),
            flows: OAuthFlows::default(),
            check_lock: tokio::sync::Mutex::new(()),
            checking: AtomicBool::new(false),
            auto_check: AtomicBool::new(true),
            next_check: Mutex::new(None),
            wake: tokio::sync::Notify::new(),
        }
    }

    pub fn setup(&self) -> ProviderSetup {
        self.setup.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[cfg(test)]
    pub fn set_setup(&self, setup: ProviderSetup) {
        *self.setup.write().unwrap_or_else(|e| e.into_inner()) = setup;
    }

    #[cfg(test)]
    pub fn set_auto_check(&self, on: bool) {
        self.auto_check.store(on, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn set_sink(&self, sink: Arc<dyn BatchSink>) {
        *self.sink.write().unwrap_or_else(|e| e.into_inner()) = sink;
    }

    fn is_checking(&self) -> bool {
        self.checking.load(Ordering::SeqCst)
    }

    fn sink(&self) -> Arc<dyn BatchSink> {
        self.sink.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn schedule_next(&self, minutes: u32) {
        *lock(&self.next_check) = Some(now() + f64::from(minutes) * 60_000.0);
    }
}

fn config(state: &AppState) -> NandiConfig {
    nandi_config::load_from(state.nandi_dir()).unwrap_or_else(|e| {
        log::warn!("Reading the email settings without config.json: {e}");
        NandiConfig::default()
    })
}

fn email_settings(state: &AppState) -> EmailSettings {
    config(state).email.unwrap_or_default()
}

fn update_config(state: &AppState, change: impl FnOnce(&mut NandiConfig)) -> Result<(), String> {
    nandi_config::update_in(state.nandi_dir(), change)
}

async fn load_setting<T: serde::de::DeserializeOwned + Default>(
    state: &AppState,
    key: String,
) -> T {
    state
        .db()
        .call(move |conn| db::get_setting(conn, &key))
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

async fn save_setting(state: &AppState, key: String, value: Value) -> Result<(), String> {
    state
        .db()
        .call(move |conn| db::set_setting(conn, &key, &value))
        .await
}

async fn check_state(state: &AppState, provider: MailProvider) -> CheckState {
    load_setting(state, provider.state_key()).await
}

async fn save_check_state(
    state: &AppState,
    provider: MailProvider,
    check: &CheckState,
) -> Result<(), String> {
    save_setting(
        state,
        provider.state_key(),
        serde_json::to_value(check).unwrap_or_default(),
    )
    .await
}

fn emit_changed(state: &AppState) {
    events::emit(state.bus(), &EmailChanged {});
}

/// The mailboxes the Chief of Staff may read
fn granted(state: &AppState, agent: &str) -> Vec<EmailAccount> {
    let config = config(state);
    config
        .email
        .unwrap_or_default()
        .accounts
        .into_iter()
        .filter(|a| {
            settings::allows(
                &config.connector_grants,
                agent,
                a.provider.id(),
                GrantAccess::Read,
            )
        })
        .collect()
}

pub async fn status(state: &AppState) -> EmailStatus {
    let email = state.email();
    let setup = email.setup();
    let settings = email_settings(state);
    let mut providers = Vec::new();
    for provider in MailProvider::ALL {
        let account = settings.account(provider);
        let check = check_state(state, provider).await;
        let connected = account.is_some();
        let state_now = match account {
            None => EmailAccountState::NotConnected,
            Some(_) if check.needs_sign_in => EmailAccountState::NeedsSignIn,
            Some(a) if a.paused => EmailAccountState::Paused,
            Some(_) if check.error.is_some() => EmailAccountState::Error,
            Some(_) => EmailAccountState::Ready,
        };
        providers.push(EmailProviderStatus {
            provider,
            available: setup.available(provider),
            connected,
            account_name: account.and_then(|a| a.name.clone()),
            account_email: account.map(|a| a.email.clone()),
            state: state_now,
            error: if connected { check.error } else { None },
            last_checked_at: check.last_checked_at,
            last_found: check.last_found,
            last_run_id: check.last_run_id,
            backfill: backfill::status_of(
                settings.backfill,
                connected,
                !load_setting::<Value>(state, provider.cursor_key())
                    .await
                    .is_null(),
                &backfill::load(state, provider).await,
            ),
        });
    }
    let any_active = settings.accounts.iter().any(|a| !a.paused);
    EmailStatus {
        providers,
        interval_minutes: settings.interval_minutes,
        watch: settings.watch,
        next_check_at: if any_active {
            *lock(&email.next_check)
        } else {
            None
        },
        checking: email.checking.load(Ordering::SeqCst),
        backfill_range: settings.backfill,
    }
}

/// Starts the sign-in: open `verificationUri` (or show `userCode`)
pub async fn start_sign_in(
    state: &AppState,
    provider: MailProvider,
) -> Result<DeviceFlowState, String> {
    let setup = state.email().setup();
    if !setup.available(provider) {
        return Err(format!(
            "{} isn't available in this build of Nandi",
            provider.name()
        ));
    }
    state
        .email()
        .flows
        .start(
            &provider.token_entry(),
            setup.oauth(provider),
            state.secrets().clone(),
        )
        .await
}

/// Checks the sign-in once. On success: who the mailbox belongs to, the
/// account, the Chief of Staff's read grant, and the first check.
pub async fn poll_sign_in(state: &AppState, provider: MailProvider) -> EmailSignInPoll {
    let setup = state.email().setup();
    let failed = |error: String| EmailSignInPoll {
        phase: SignInPhase::Failed,
        problem: Some(classify_sign_in_error(&error, &setup)),
    };
    match state
        .email()
        .flows
        .poll(&provider.token_entry(), state.secrets())
        .await
    {
        Ok(false) => EmailSignInPoll {
            phase: SignInPhase::Waiting,
            problem: None,
        },
        Err(error) => failed(error),
        Ok(true) => match connect(state, provider).await {
            Ok(()) => EmailSignInPoll {
                phase: SignInPhase::Connected,
                problem: None,
            },
            Err(error) => {
                let _ = oauth::forget(&provider.token_entry(), state.secrets());
                failed(error)
            }
        },
    }
}

async fn connect(state: &AppState, provider: MailProvider) -> Result<(), String> {
    let setup = state.email().setup();
    let identity = Mailbox::new(provider, &setup, state.secrets())
        .identity()
        .await
        .map_err(|e| {
            log::warn!(
                "{} signed in, but reading the mailbox owner failed: {e}",
                provider.name()
            );
            format!("Signed in, but Nandi couldn't read whose mailbox this is: {e}")
        })?;
    // Reconnecting the same mailbox (after a sign-in ended) carries on from
    // its cursor; a new one starts with the window
    let same_mailbox = email_settings(state)
        .account(provider)
        .is_some_and(|a| a.email == identity.email);
    update_config(state, |config| {
        let email = config.email.get_or_insert_with(EmailSettings::default);
        email.accounts.retain(|a| a.provider != provider);
        email.accounts.push(EmailAccount {
            provider,
            email: identity.email.clone(),
            name: identity.name.clone(),
            paused: false,
        });
        if !settings::allows(
            &config.connector_grants,
            CHIEF_OF_STAFF,
            provider.id(),
            GrantAccess::Read,
        ) {
            config.connector_grants.push(ConnectorGrant {
                agent: CHIEF_OF_STAFF.into(),
                connector: provider.id().into(),
                access: GrantAccess::Read,
            });
        }
    })?;
    if same_mailbox {
        let mut check = check_state(state, provider).await;
        check.needs_sign_in = false;
        check.error = None;
        save_check_state(state, provider, &check).await?;
    } else {
        forget_progress(state, provider).await?;
    }
    // Who "me" is, in every Chief of Staff run (editable in My role)
    let store = state.store().clone();
    let (name, address) = (identity.name.clone(), identity.email.clone());
    match tokio::task::spawn_blocking(move || {
        crate::services::builtin_agents::note_identity(&store, name.as_deref(), &address)
    })
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log::warn!("Couldn't note the mailbox owner in My role: {e}"),
        Err(e) => log::warn!("Noting the mailbox owner failed: {e}"),
    }
    log::info!("Connected {} for the Chief of Staff", provider.name());
    emit_changed(state);
    check_soon(state);
    Ok(())
}

async fn forget_progress(state: &AppState, provider: MailProvider) -> Result<(), String> {
    let keys = [
        provider.cursor_key(),
        provider.state_key(),
        provider.backfill_key(),
    ];
    state
        .db()
        .call(move |conn| {
            for key in &keys {
                db::delete_setting(conn, key)?;
            }
            Ok(())
        })
        .await
}

/// Signs out: the keyring entry, the cursor, the account and its grants
pub async fn disconnect(state: &AppState, provider: MailProvider) -> Result<EmailStatus, String> {
    state.email().flows.cancel(&provider.token_entry());
    oauth::forget(&provider.token_entry(), state.secrets())?;
    forget_progress(state, provider).await?;
    update_config(state, |config| {
        if let Some(email) = config.email.as_mut() {
            email.accounts.retain(|a| a.provider != provider);
        }
        config
            .connector_grants
            .retain(|g| g.connector != provider.id());
    })?;
    log::info!("Disconnected {}", provider.name());
    emit_changed(state);
    Ok(status(state).await)
}

pub async fn set_paused(
    state: &AppState,
    provider: MailProvider,
    paused: bool,
) -> Result<EmailStatus, String> {
    let mut found = false;
    update_config(state, |config| {
        if let Some(account) = config.email.as_mut().and_then(|e| e.account_mut(provider)) {
            account.paused = paused;
            found = true;
        }
    })?;
    if !found {
        return Err(format!("{} isn't connected", provider.name()));
    }
    emit_changed(state);
    if !paused {
        check_soon(state);
    }
    Ok(status(state).await)
}

pub async fn set_settings(
    state: &AppState,
    input: EmailSettingsInput,
) -> Result<EmailStatus, String> {
    let watch = settings::validate(input.interval_minutes, &input.watch)?;
    update_config(state, |config| {
        let email = config.email.get_or_insert_with(EmailSettings::default);
        email.interval_minutes = input.interval_minutes;
        email.watch = watch;
        email.backfill = input.backfill;
    })?;
    state.email().schedule_next(input.interval_minutes);
    emit_changed(state);
    Ok(status(state).await)
}

/// Checks now, in the background
pub async fn check_now(state: &AppState) -> EmailStatus {
    check_soon(state);
    let mut status = status(state).await;
    status.checking = true;
    status
}

fn check_soon(state: &AppState) {
    if !state.email().auto_check.load(Ordering::SeqCst) {
        return;
    }
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        if let Some(state) = weak.upgrade() {
            check_all(&state).await;
        }
    });
}

/// What one check did
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    /// Emails handed over
    pub found: usize,
    pub run_id: Option<String>,
}

/// Reads every connected, unpaused mailbox the Chief of Staff may read
/// and hands all new mail over in one run. Cursors move only once the run
/// is queued, so mail is never skipped.
pub async fn check_all(state: &AppState) -> CheckOutcome {
    let email = state.email();
    let Ok(_one_check) = email.check_lock.try_lock() else {
        return CheckOutcome::default();
    };
    let settings = email_settings(state);
    let accounts: Vec<EmailAccount> = granted(state, CHIEF_OF_STAFF)
        .into_iter()
        .filter(|a| !a.paused)
        .collect();
    email.schedule_next(settings.interval_minutes);
    if accounts.is_empty() {
        return CheckOutcome::default();
    }
    email.checking.store(true, Ordering::SeqCst);
    emit_changed(state);
    let outcome = run_check(state, &settings, &accounts).await;
    email.checking.store(false, Ordering::SeqCst);
    email.schedule_next(settings.interval_minutes);
    emit_changed(state);
    outcome
}

async fn run_check(
    state: &AppState,
    settings: &EmailSettings,
    accounts: &[EmailAccount],
) -> CheckOutcome {
    let setup = state.email().setup();
    let window_start = db::now_ms() / 1000 - READ_WINDOW_DAYS * 24 * 60 * 60;
    let me: Vec<Identity> = settings
        .accounts
        .iter()
        .map(|a| Identity {
            name: a.name.clone(),
            email: a.email.clone(),
        })
        .collect();
    let my_addresses: Vec<String> = me.iter().map(|i| i.email.clone()).collect();
    let filter = filter::MailFilter::new(&my_addresses, &settings.watch);

    let mut headers = Vec::new();
    let mut cursors = Vec::new();
    let mut first_read = false;
    let mut checked = Vec::new();
    for account in accounts {
        let provider = account.provider;
        let mut check = check_state(state, provider).await;
        let cursor: Value = load_setting(state, provider.cursor_key()).await;
        let mailbox = Mailbox::new(provider, &setup, state.secrets());
        match mailbox.changes(&cursor, window_start).await {
            Ok(changes) => {
                let mut found = changes.headers;
                // Cheap signals first, then the per-message header lookup
                // for what's left (Outlook only)
                found.retain(|h| filter.keep(h));
                found.truncate(batch::MAX_BATCH * 2);
                mailbox.mark_bulk(&mut found).await;
                found.retain(|h| filter.keep(h));
                first_read |= changes.restarted;
                headers.extend(found);
                cursors.push((provider, changes.cursor));
                check.needs_sign_in = false;
                check.error = None;
            }
            Err(MailError::NeedsSignIn(reason)) => {
                log::warn!("{} needs a new sign-in: {reason}", provider.name());
                check.needs_sign_in = true;
                check.error = Some(reason);
            }
            Err(e) => {
                log::warn!("Checking {} failed: {e}", provider.name());
                check.error = Some(e.to_string());
            }
        }
        check.last_checked_at = Some(now());
        checked.push((provider, check));
    }

    headers.sort_by_key(|h| std::cmp::Reverse(h.at));
    let omitted = headers.len().saturating_sub(batch::MAX_BATCH);
    headers.truncate(batch::MAX_BATCH);
    let mut outcome = CheckOutcome {
        found: headers.len(),
        run_id: None,
    };
    let handed_over = if headers.is_empty() {
        true
    } else {
        let text = batch::Batch {
            me: &me,
            headers: &headers,
            omitted,
            first_read,
            window_days: READ_WINDOW_DAYS,
            backfill: false,
        }
        .prompt();
        let title = format!(
            "Email: {} new email{}",
            headers.len(),
            if headers.len() == 1 { "" } else { "s" }
        );
        match state.email().sink().submit(state, title, text).await {
            Ok(run_id) => {
                log::info!("Handed {} emails to the Chief of Staff", headers.len());
                outcome.run_id = Some(run_id);
                true
            }
            Err(e) => {
                log::warn!("The Chief of Staff couldn't take the email check: {e}");
                for (_, check) in checked.iter_mut().filter(|(_, c)| c.error.is_none()) {
                    check.error = Some(format!("The Chief of Staff couldn't start: {e}"));
                }
                outcome.found = 0;
                false
            }
        }
    };
    if handed_over {
        for (provider, cursor) in cursors {
            if let Err(e) = save_setting(state, provider.cursor_key(), cursor).await {
                log::warn!("Couldn't save the {} cursor: {e}", provider.name());
            }
        }
    }
    for (provider, mut check) in checked {
        if check.error.is_none() {
            check.last_found = u32::try_from(
                headers
                    .iter()
                    .filter(|h| h.id.starts_with(provider.id()))
                    .count(),
            )
            .unwrap_or(u32::MAX);
            if outcome.run_id.is_some() && check.last_found > 0 {
                check.last_run_id = outcome.run_id.clone();
            }
        }
        if let Err(e) = save_check_state(state, provider, &check).await {
            log::warn!("Couldn't save the {} check: {e}", provider.name());
        }
    }
    outcome
}

/// Checks every `interval` minutes while any mailbox is connected; the
/// first check comes shortly after launch and catches up from the cursor
pub fn start_scheduler(state: &AppState) {
    let email = state.email();
    *lock(&email.next_check) = Some(now() + FIRST_CHECK_AFTER.as_millis() as f64);
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        let mut next_backfill = tokio::time::Instant::now() + backfill::BACKFILL_EVERY;
        loop {
            let Some(state) = weak.upgrade() else {
                break;
            };
            let due = lock(&state.email().next_check).is_some_and(|at| at <= now());
            if due {
                check_all(&state).await;
            } else if tokio::time::Instant::now() >= next_backfill {
                next_backfill = tokio::time::Instant::now() + backfill::BACKFILL_EVERY;
                backfill::pass(&state).await;
            }
            let email = state.email().clone();
            drop(state);
            let _ = tokio::time::timeout(TICK, email.wake.notified()).await;
        }
    });
}
