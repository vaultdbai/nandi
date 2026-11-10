//! Calendars and files: Outlook calendar, Google Calendar, OneDrive and
//! Google Drive, read-only, on the shared sign-in layer
//! (`services::auth::oauth`) and the mail API's GET-only client
//! (`email::api`). Like mail, these are our own connectors: tokens stay in
//! the host's keyring and the tools run on the host, never in a container.
//!
//! ```text
//! Settings › Calendar & files › Connect ─ sign-in (only that account's scopes)
//!     ─▶ keyring nandi.<id>.oauth + config.json workspace.accounts + a saved
//!        read grant for the Chief of Staff
//! every hour (or Check now), batched:
//!   each granted calendar ─ change feed since cursor ─▶ ONE Chief of Staff
//!   run (source calendar_check, untrusted <calendar-data>) ─▶ cursors saved
//! tools: calendar_* and files_* for agents with a grant (or that name the
//! connector in skill.md, asking first)
//! ```
//!
//! Nothing is kept but the cursors, fingerprints of the events already sent
//! (`seen`) and the notes the Chief of Staff writes.

pub mod accounts;
pub mod batch;
pub mod calendar;
pub mod files;
mod seen;
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

pub use accounts::{AccountFamily, AccountKind};

use crate::db;
use crate::model::{SourceType, Timestamp};
use crate::services::auth::oauth::{self, DeviceFlowState, OAuthFlows};
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::domain::grant::{ConnectorGrant, GrantAccess, GrantScope};
use crate::services::email::api::{MailApi, MailError};
use crate::services::email::{classify_sign_in_error, EmailSignInPoll, ProviderSetup, SignInPhase};
use crate::services::events::{self, NandiEvent};
use crate::services::grants::{self, GrantInfo, GrantsChanged};
use crate::services::orchestrator::{RunSource, PRIORITY_SCHEDULED};
use crate::services::AppState;
use crate::utils::nandi_config;

/// The calendar check's default: hourly (owner, task Wave 4), batched
pub const DEFAULT_CALENDAR_MINUTES: u32 = 60;
const TICK: Duration = Duration::from_secs(30);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// Settings (config.json `workspace`, this computer)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAccount {
    pub kind: AccountKind,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    #[serde(default)]
    pub accounts: Vec<WorkspaceAccount>,
    #[serde(default = "default_minutes")]
    pub calendar_interval_minutes: u32,
}

fn default_minutes() -> u32 {
    DEFAULT_CALENDAR_MINUTES
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            calendar_interval_minutes: DEFAULT_CALENDAR_MINUTES,
        }
    }
}

impl WorkspaceSettings {
    pub fn account(&self, kind: AccountKind) -> Option<&WorkspaceAccount> {
        self.accounts.iter().find(|a| a.kind == kind)
    }
}

fn settings(state: &AppState) -> WorkspaceSettings {
    nandi_config::load_from(state.nandi_dir())
        .ok()
        .and_then(|c| c.workspace)
        .unwrap_or_default()
}

fn update(state: &AppState, change: impl FnOnce(&mut WorkspaceSettings)) -> Result<(), String> {
    nandi_config::update_in(state.nandi_dir(), |config| {
        change(
            config
                .workspace
                .get_or_insert_with(WorkspaceSettings::default),
        )
    })
}

/// The connected accounts (granted or not)
pub fn connected(state: &AppState) -> Vec<AccountKind> {
    settings(state).accounts.iter().map(|a| a.kind).collect()
}

// ---------------------------------------------------------------------------
// What clients see
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAccountStatus {
    pub kind: AccountKind,
    pub id: String,
    pub family: AccountFamily,
    /// This build can sign in to its cloud
    pub available: bool,
    pub connected: bool,
    pub account_email: Option<String>,
    pub needs_sign_in: bool,
    pub error: Option<String>,
    pub last_checked_at: Option<Timestamp>,
    /// Calendar: events the last check handed to the Chief of Staff
    pub last_found: u32,
    pub last_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    pub accounts: Vec<WorkspaceAccountStatus>,
    pub calendar_interval_minutes: u32,
    pub next_calendar_check_at: Option<Timestamp>,
    pub checking: bool,
}

/// `workspace://changed`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChanged {}

impl NandiEvent for WorkspaceChanged {
    const NAME: &'static str = "workspace://changed";
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckState {
    #[serde(default)]
    last_checked_at: Option<Timestamp>,
    #[serde(default)]
    last_found: u32,
    #[serde(default)]
    last_run_id: Option<String>,
    #[serde(default)]
    needs_sign_in: bool,
    #[serde(default)]
    error: Option<String>,
}

fn state_key(kind: AccountKind) -> String {
    format!("workspace.{}.state", kind.id())
}

fn seen_key(kind: AccountKind) -> String {
    format!("workspace.{}.seen", kind.id())
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// Hands one calendar check to the agent that keeps the notes
pub trait CalendarSink: Send + Sync {
    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        title: String,
        text: String,
    ) -> BoxFuture<'a, Result<String, String>>;
}

struct ChiefOfStaff;

impl CalendarSink for ChiefOfStaff {
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
                    db::ensure_calendar_conversation(conn, CHIEF_OF_STAFF, &check_id, &title)
                })
                .await?;
            let source = RunSource {
                source_type: SourceType::CalendarCheck,
                source_id: Some("calendar".into()),
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

pub struct WorkspaceService {
    setup: RwLock<ProviderSetup>,
    sink: RwLock<Arc<dyn CalendarSink>>,
    flows: OAuthFlows,
    check_lock: tokio::sync::Mutex<()>,
    checking: AtomicBool,
    auto_check: AtomicBool,
    next_check: Mutex<Option<Timestamp>>,
    wake: tokio::sync::Notify,
}

impl Default for WorkspaceService {
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

impl WorkspaceService {
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
    pub fn set_sink(&self, sink: Arc<dyn CalendarSink>) {
        *self.sink.write().unwrap_or_else(|e| e.into_inner()) = sink;
    }

    fn sink(&self) -> Arc<dyn CalendarSink> {
        self.sink.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn schedule_next(&self, minutes: u32) {
        *lock(&self.next_check) = Some(now() + f64::from(minutes) * 60_000.0);
    }

    /// The account's GET-only API
    pub fn api(&self, kind: AccountKind, secrets: &crate::services::secrets::Secrets) -> MailApi {
        let setup = self.setup();
        MailApi::for_account(
            kind.name(),
            setup.account_oauth(kind.cloud(), kind.id(), kind.scopes()),
            kind.token_entry(),
            kind.api_bases(&setup),
            secrets,
        )
    }

    /// Calendar's or Drive's base address
    pub fn base(&self, kind: AccountKind) -> String {
        let e = self.setup().endpoints;
        match kind {
            AccountKind::OutlookCalendar | AccountKind::OneDrive => e.graph,
            AccountKind::GoogleCalendar => e.google_calendar,
            AccountKind::GoogleDrive => e.google_drive,
        }
    }
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

pub(crate) async fn save_setting(
    state: &AppState,
    key: String,
    value: Value,
) -> Result<(), String> {
    state
        .db()
        .call(move |conn| db::set_setting(conn, &key, &value))
        .await
}

pub(crate) async fn load_cursor(state: &AppState, kind: AccountKind) -> Value {
    load_setting(state, kind.cursor_key()).await
}

fn emit_changed(state: &AppState) {
    events::emit(state.bus(), &WorkspaceChanged {});
}

pub async fn status(state: &AppState) -> WorkspaceStatus {
    let service = state.workspace();
    let setup = service.setup();
    let settings = settings(state);
    let mut accounts = Vec::new();
    for kind in AccountKind::ALL {
        let account = settings.account(kind);
        let check: CheckState = load_setting(state, state_key(kind)).await;
        accounts.push(WorkspaceAccountStatus {
            kind,
            id: kind.id().to_string(),
            family: kind.family(),
            available: setup.cloud_available(kind.cloud()),
            connected: account.is_some(),
            account_email: account.map(|a| a.email.clone()),
            needs_sign_in: account.is_some() && check.needs_sign_in,
            error: account.and(check.error),
            last_checked_at: check.last_checked_at,
            last_found: check.last_found,
            last_run_id: check.last_run_id,
        });
    }
    let any_calendar = settings
        .accounts
        .iter()
        .any(|a| a.kind.family() == AccountFamily::Calendar);
    WorkspaceStatus {
        accounts,
        calendar_interval_minutes: settings.calendar_interval_minutes,
        next_calendar_check_at: if any_calendar {
            *lock(&service.next_check)
        } else {
            None
        },
        checking: service.checking.load(Ordering::SeqCst),
    }
}

/// Starts connecting one account: only its own scopes are asked for
pub async fn start_sign_in(state: &AppState, kind: AccountKind) -> Result<DeviceFlowState, String> {
    let service = state.workspace();
    let setup = service.setup();
    if !setup.cloud_available(kind.cloud()) {
        return Err(format!(
            "{} isn't available in this build of Nandi",
            kind.name()
        ));
    }
    service
        .flows
        .start(
            &kind.token_entry(),
            setup.account_oauth(kind.cloud(), kind.id(), kind.scopes()),
            state.secrets().clone(),
        )
        .await
}

pub async fn poll_sign_in(state: &AppState, kind: AccountKind) -> EmailSignInPoll {
    let setup = state.workspace().setup();
    let failed = |error: String| EmailSignInPoll {
        phase: SignInPhase::Failed,
        problem: Some(classify_sign_in_error(&error, &setup)),
    };
    match state
        .workspace()
        .flows
        .poll(&kind.token_entry(), state.secrets())
        .await
    {
        Ok(false) => EmailSignInPoll {
            phase: SignInPhase::Waiting,
            problem: None,
        },
        Err(error) => failed(error),
        Ok(true) => match connect(state, kind).await {
            Ok(()) => EmailSignInPoll {
                phase: SignInPhase::Connected,
                problem: None,
            },
            Err(error) => {
                let _ = oauth::forget(&kind.token_entry(), state.secrets());
                failed(error)
            }
        },
    }
}

/// Who signed in: Graph `/me`, or Google's userinfo
async fn identity(
    state: &AppState,
    kind: AccountKind,
) -> Result<(String, Option<String>), MailError> {
    let service = state.workspace();
    let api = service.api(kind, state.secrets());
    let setup = service.setup();
    let (url, mail, name) = match kind.cloud() {
        crate::services::email::Cloud::Microsoft => (
            api.url(
                &setup.endpoints.graph,
                &["me"],
                &[("$select", "mail,userPrincipalName,displayName")],
            ),
            ["mail", "userPrincipalName"],
            "displayName",
        ),
        crate::services::email::Cloud::Google => (
            setup.endpoints.google_userinfo.clone(),
            ["email", "email"],
            "name",
        ),
    };
    let me = api.get(&url, &[]).await?;
    let email = mail
        .iter()
        .find_map(|key| {
            me.get(*key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| MailError::Failed("no address in the profile".into()))?
        .to_lowercase();
    let name = me.get(name).and_then(Value::as_str).map(str::to_string);
    Ok((email, name))
}

async fn connect(state: &AppState, kind: AccountKind) -> Result<(), String> {
    let (email, name) = identity(state, kind).await.map_err(|e| {
        log::warn!(
            "{} signed in, but reading the profile failed: {e}",
            kind.name()
        );
        format!("Signed in, but Nandi couldn't read whose account this is: {e}")
    })?;
    let same = settings(state)
        .account(kind)
        .is_some_and(|a| a.email == email);
    update(state, |s| {
        s.accounts.retain(|a| a.kind != kind);
        s.accounts.push(WorkspaceAccount { kind, email, name });
    })?;
    if !same {
        save_setting(state, kind.cursor_key(), Value::Null).await?;
        save_setting(state, seen_key(kind), Value::Null).await?;
    }
    save_setting(state, state_key(kind), Value::Null).await?;
    // Connecting is the person's say-so for the Chief of Staff (saved, like
    // mail; removable in Settings › Connectors › Access)
    let grant = ConnectorGrant::new(CHIEF_OF_STAFF, kind.id(), GrantAccess::Read)
        .map_err(|e| e.to_string())?;
    if state.grants().grant(grant, GrantScope::Saved)? {
        let info = GrantInfo {
            agent: CHIEF_OF_STAFF.into(),
            connector: kind.id().into(),
            access: GrantAccess::Read,
            scope: GrantScope::Saved,
        };
        grants::record(state, "granted", &info, Some("The account was connected"));
        events::emit(state.bus(), &GrantsChanged {});
    }
    log::info!("Connected {}", kind.name());
    emit_changed(state);
    if kind.family() == AccountFamily::Calendar {
        check_soon(state);
    }
    Ok(())
}

/// Signs out: the keyring entry, the cursor, the events seen, the account
/// and its grants
pub async fn disconnect(state: &AppState, kind: AccountKind) -> Result<WorkspaceStatus, String> {
    state.workspace().flows.cancel(&kind.token_entry());
    oauth::forget(&kind.token_entry(), state.secrets())?;
    for key in [kind.cursor_key(), state_key(kind), seen_key(kind)] {
        state
            .db()
            .call(move |conn| db::delete_setting(conn, &key))
            .await?;
    }
    update(state, |s| s.accounts.retain(|a| a.kind != kind))?;
    for gone in state.grants().revoke_connector(kind.id())? {
        grants::record(state, "removed", &gone, Some("The account was signed out"));
    }
    events::emit(state.bus(), &GrantsChanged {});
    emit_changed(state);
    Ok(status(state).await)
}

pub async fn set_calendar_interval(
    state: &AppState,
    minutes: u32,
) -> Result<WorkspaceStatus, String> {
    if !(15..=24 * 60).contains(&minutes) {
        return Err("Check the calendar every 15 minutes to 24 hours".into());
    }
    update(state, |s| s.calendar_interval_minutes = minutes)?;
    state.workspace().schedule_next(minutes);
    emit_changed(state);
    Ok(status(state).await)
}

pub async fn check_now(state: &AppState) -> WorkspaceStatus {
    check_soon(state);
    let mut status = status(state).await;
    status.checking = true;
    status
}

fn check_soon(state: &AppState) {
    if !state.workspace().auto_check.load(Ordering::SeqCst) {
        return;
    }
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        if let Some(state) = weak.upgrade() {
            check_calendars(&state).await;
        }
    });
}

/// What one calendar check did
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    pub found: usize,
    pub run_id: Option<String>,
}

/// Reads the change feed of every calendar the Chief of Staff may read and
/// hands what changed to it in ONE run. Cursors move only once the run is
/// queued, so nothing is skipped.
pub async fn check_calendars(state: &AppState) -> CheckOutcome {
    let service = state.workspace();
    let Ok(_one) = service.check_lock.try_lock() else {
        return CheckOutcome::default();
    };
    let settings = settings(state);
    service.schedule_next(settings.calendar_interval_minutes);
    let grants = state.grants();
    let accounts: Vec<WorkspaceAccount> = settings
        .accounts
        .iter()
        .filter(|a| a.kind.family() == AccountFamily::Calendar)
        .filter(|a| grants.allows(CHIEF_OF_STAFF, a.kind.id(), GrantAccess::Read))
        .cloned()
        .collect();
    if accounts.is_empty() {
        return CheckOutcome::default();
    }
    service.checking.store(true, Ordering::SeqCst);
    emit_changed(state);
    let now_secs = db::now_ms() / 1000;
    let mut events = Vec::new();
    let mut cursors = Vec::new();
    let mut seens = Vec::new();
    let mut checked = Vec::new();
    let mut first_read = false;
    for account in &accounts {
        let kind = account.kind;
        let mut check: CheckState = load_setting(state, state_key(kind)).await;
        let cursor = load_cursor(state, kind).await;
        let mut seen: seen::Seen = load_setting(state, seen_key(kind)).await;
        let api = service.api(kind, state.secrets());
        let base = service.base(kind);
        let calendar = calendar::Calendar {
            kind,
            api: &api,
            base: &base,
        };
        match calendar.changes(&cursor, now_secs).await {
            Ok(changes) => {
                // Only the very first read is one; a feed starting over later
                // sends only what changed since the last hand-over
                first_read |= changes.restarted && seen.is_empty();
                let fresh: Vec<_> = changes
                    .events
                    .iter()
                    .filter(|e| !seen.knows(e))
                    .cloned()
                    .collect();
                seen.record(&changes.events, changes.restarted);
                let found = fresh.len();
                events.extend(fresh);
                // A feed that ended without a token keeps the last cursor
                let next = if changes.cursor.is_null() {
                    cursor
                } else {
                    changes.cursor
                };
                cursors.push((kind, next));
                seens.push((kind, seen));
                check.needs_sign_in = false;
                check.error = None;
                check.last_found = u32::try_from(found).unwrap_or(u32::MAX);
            }
            Err(MailError::NeedsSignIn(reason)) => {
                check.needs_sign_in = true;
                check.error = Some(reason);
            }
            Err(e) => {
                log::warn!("Checking {} failed: {e}", kind.name());
                check.error = Some(e.to_string());
            }
        }
        check.last_checked_at = Some(now());
        checked.push((kind, check));
    }
    let batch = batch::CalendarBatch::new(&accounts, events, first_read, now_secs);
    let mut outcome = CheckOutcome {
        found: batch.len(),
        run_id: None,
    };
    let handed_over = if batch.is_empty() {
        true
    } else {
        match service
            .sink()
            .submit(state, batch.title(), batch.prompt())
            .await
        {
            Ok(run_id) => {
                outcome.run_id = Some(run_id);
                true
            }
            Err(e) => {
                log::warn!("The Chief of Staff couldn't take the calendar check: {e}");
                for (_, check) in checked.iter_mut().filter(|(_, c)| c.error.is_none()) {
                    check.error = Some(format!("The Chief of Staff couldn't start: {e}"));
                }
                outcome.found = 0;
                false
            }
        }
    };
    if handed_over {
        for (kind, cursor) in cursors {
            if let Err(e) = save_setting(state, kind.cursor_key(), cursor).await {
                log::warn!("Couldn't save the {} cursor: {e}", kind.name());
            }
        }
        for (kind, seen) in seens {
            let value = serde_json::to_value(&seen).unwrap_or_default();
            if let Err(e) = save_setting(state, seen_key(kind), value).await {
                log::warn!("Couldn't save the {} events seen: {e}", kind.name());
            }
        }
    }
    for (kind, mut check) in checked {
        if outcome.run_id.is_some() && check.last_found > 0 {
            check.last_run_id = outcome.run_id.clone();
        }
        let value = serde_json::to_value(&check).unwrap_or_default();
        if let Err(e) = save_setting(state, state_key(kind), value).await {
            log::warn!("Couldn't save the {} check: {e}", kind.name());
        }
    }
    service.checking.store(false, Ordering::SeqCst);
    service.schedule_next(settings.calendar_interval_minutes);
    emit_changed(state);
    outcome
}

/// Checks the calendars every `calendar_interval_minutes` while one is
/// connected; the first check comes shortly after launch
pub fn start_scheduler(state: &AppState) {
    let service = state.workspace();
    *lock(&service.next_check) = Some(now() + FIRST_CHECK_AFTER.as_millis() as f64);
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else {
                break;
            };
            let due = lock(&state.workspace().next_check).is_some_and(|at| at <= now());
            if due {
                check_calendars(&state).await;
            }
            let service = state.workspace().clone();
            drop(state);
            let _ = tokio::time::timeout(TICK, service.wake.notified()).await;
        }
    });
}
