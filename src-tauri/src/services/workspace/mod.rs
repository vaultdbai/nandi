//! Calendars and files: Outlook calendar, Google Calendar, OneDrive and
//! Google Drive, read-only, with each connected account (`services::email`)
//! and the mail API's GET-only client (`email::api`). Like mail, these are
//! our own connectors: tokens stay in the host's keyring and the tools run
//! on the host, never in a container.
//!
//! ```text
//! Settings › Email › Connect, Also read my files / my calendar (ticked)
//!     ─▶ ONE sign-in for mail + the ticked read-only scopes
//!     ─▶ config.json email.accounts[].files / .calendar + a saved read
//!        grant for Nandi on the account's drive and calendar connectors
//! every hour (or Check now), batched:
//!   each account's granted calendar ─ change feed since its cursor ─▶ ONE
//!   Nandi run (source calendar_check, untrusted <calendar-data>) ─▶ cursors saved
//! tools: calendar_* and files_* over every granted account, each result
//! naming its account when there are several
//! ```
//!
//! Nothing is kept but each account's cursors (`files.<provider>.<hash>.*`,
//! `calendar.<provider>.<hash>.*`), fingerprints of the events already sent
//! (`seen`) and the notes Nandi writes.

pub mod accounts;
pub mod batch;
pub mod calendar;
pub mod files;
pub mod keys_from_0_2;
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
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::domain::grant::{ConnectorGrant, GrantAccess, GrantScope};
use crate::services::email::api::{MailApi, MailError};
use crate::services::email::settings::EmailAccount;
use crate::services::email::{EmailSettings, MailboxId, ProviderSetup, Reads};
use crate::services::events::{self, NandiEvent};
use crate::services::grants::{self, GrantInfo, GrantsChanged};
use crate::services::orchestrator::{RunSource, PRIORITY_SCHEDULED};
use crate::services::AppState;
use crate::utils::nandi_config;

/// The calendar check's default: hourly (owner's decision), batched
pub const DEFAULT_CALENDAR_MINUTES: u32 = 60;
const TICK: Duration = Duration::from_secs(30);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// Settings (config.json `workspace`, this computer)
// ---------------------------------------------------------------------------

/// How often the calendars are checked. Which accounts read files and
/// calendars lives with each account (`email.accounts`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettings {
    #[serde(default = "default_minutes")]
    pub calendar_interval_minutes: u32,
}

fn default_minutes() -> u32 {
    DEFAULT_CALENDAR_MINUTES
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            calendar_interval_minutes: DEFAULT_CALENDAR_MINUTES,
        }
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

// ---------------------------------------------------------------------------
// Sources: one account's drive or calendar
// ---------------------------------------------------------------------------

/// One account's drive or calendar, and the sign-in it reads with
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub mailbox: MailboxId,
    pub kind: AccountKind,
    pub name: Option<String>,
    entry: String,
    reads: Reads,
}

impl Source {
    /// `account`'s drive or calendar, while Nandi reads it
    pub fn of(account: &EmailAccount, family: AccountFamily) -> Option<Self> {
        let (entry, reads) = account.read_sign_in(family)?;
        Some(Self {
            mailbox: account.id(),
            kind: AccountKind::of(account.provider, family),
            name: account.name.clone(),
            entry,
            reads,
        })
    }

    /// The account's address, as results name it
    pub fn account(&self) -> &str {
        &self.mailbox.email
    }

    pub fn cursor_key(&self) -> String {
        self.mailbox.read_key(self.kind.family(), "cursor")
    }

    fn state_key(&self) -> String {
        self.mailbox.read_key(self.kind.family(), "state")
    }

    fn seen_key(&self) -> String {
        self.mailbox.read_key(self.kind.family(), "seen")
    }
}

/// Every account's drive or calendar, in the order the accounts were
/// connected
pub fn sources_of(settings: &EmailSettings, family: AccountFamily) -> Vec<Source> {
    settings
        .accounts
        .iter()
        .filter_map(|account| Source::of(account, family))
        .collect()
}

pub fn sources(state: &AppState, family: AccountFamily) -> Vec<Source> {
    sources_of(&crate::services::email::email_settings(state), family)
}

/// The drives and calendars at least one account reads (granted or not)
pub fn connected(state: &AppState) -> Vec<AccountKind> {
    let settings = crate::services::email::email_settings(state);
    AccountKind::ALL
        .into_iter()
        .filter(|kind| {
            sources_of(&settings, kind.family())
                .iter()
                .any(|s| s.kind == *kind)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// What clients see
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    /// Connector IDs of the drives and calendars an account reads
    pub connected: Vec<String>,
    pub calendar_interval_minutes: u32,
    pub next_calendar_check_at: Option<Timestamp>,
    pub checking: bool,
}

/// One account's calendar check, for its row in Settings › Email
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarCheckStatus {
    pub needs_sign_in: bool,
    pub error: Option<String>,
    pub last_checked_at: Option<Timestamp>,
    /// Events the last check handed to Nandi
    pub last_found: u32,
    pub last_run_id: Option<String>,
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

struct Nandi;

impl CalendarSink for Nandi {
    fn submit<'a>(
        &'a self,
        state: &'a AppState,
        title: String,
        text: String,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            state.store().require_agent(DEFAULT_AGENT)?;
            let check_id = uuid::Uuid::new_v4().to_string();
            let label = title.clone();
            let conversation = state
                .db()
                .call(move |conn| {
                    db::ensure_calendar_conversation(conn, DEFAULT_AGENT, &check_id, &title)
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
                DEFAULT_AGENT,
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
            sink: RwLock::new(Arc::new(Nandi)),
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

    /// The source's GET-only API, on its account's sign-in
    pub fn api(&self, source: &Source, secrets: &crate::services::secrets::Secrets) -> MailApi {
        let setup = self.setup();
        let provider = source.kind.provider();
        MailApi::for_account(
            source.kind.name(),
            setup.oauth(provider, source.reads),
            source.entry.clone(),
            source.kind.api_bases(&setup),
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

pub(crate) async fn load_cursor(state: &AppState, source: &Source) -> Value {
    load_setting(state, source.cursor_key()).await
}

fn emit_changed(state: &AppState) {
    events::emit(state.bus(), &WorkspaceChanged {});
}

pub async fn status(state: &AppState) -> WorkspaceStatus {
    let service = state.workspace();
    let settings = settings(state);
    let connected = connected(state);
    let any_calendar = connected
        .iter()
        .any(|k| k.family() == AccountFamily::Calendar);
    WorkspaceStatus {
        connected: connected.iter().map(|k| k.id().to_string()).collect(),
        calendar_interval_minutes: settings.calendar_interval_minutes,
        next_calendar_check_at: if any_calendar {
            *lock(&service.next_check)
        } else {
            None
        },
        checking: service.checking.load(Ordering::SeqCst),
    }
}

/// `mailbox`'s last calendar check
pub async fn calendar_check(state: &AppState, mailbox: &MailboxId) -> CalendarCheckStatus {
    let check: CheckState =
        load_setting(state, mailbox.read_key(AccountFamily::Calendar, "state")).await;
    CalendarCheckStatus {
        needs_sign_in: check.needs_sign_in,
        error: check.error,
        last_checked_at: check.last_checked_at,
        last_found: check.last_found,
        last_run_id: check.last_run_id,
    }
}

// ---------------------------------------------------------------------------
// An account starts or stops reading its files or calendar
// (`email::connect`, `email::set_reads`, `email::disconnect`)
// ---------------------------------------------------------------------------

/// The account now reads `kind`: Nandi gets a saved read grant on it (the
/// person ticked it), and a calendar is checked soon
pub(crate) fn started(state: &AppState, kind: AccountKind) -> Result<(), String> {
    let grant = ConnectorGrant::new(DEFAULT_AGENT, kind.id(), GrantAccess::Read)
        .map_err(|e| e.to_string())?;
    if state.grants().grant(grant, GrantScope::Saved)? {
        let info = GrantInfo {
            agent: DEFAULT_AGENT.into(),
            connector: kind.id().into(),
            access: GrantAccess::Read,
            scope: GrantScope::Saved,
        };
        grants::record(state, "granted", &info, Some("The account was connected"));
        events::emit(state.bus(), &GrantsChanged {});
    }
    log::info!("An account reads its {}", kind.name());
    emit_changed(state);
    if kind.family() == AccountFamily::Calendar {
        check_soon(state);
    }
    Ok(())
}

/// Forgets what `mailbox` kept for `family`: its cursor, last check, the
/// events seen, and a sign-in it had apart
pub(crate) async fn forget(
    state: &AppState,
    mailbox: &MailboxId,
    family: AccountFamily,
) -> Result<(), String> {
    crate::services::auth::oauth::forget(&mailbox.read_token_entry(family), state.secrets())?;
    let keys = ["cursor", "state", "seen"].map(|what| mailbox.read_key(family, what));
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

/// An account stopped reading `kind`: the grants on it go with the last
/// account that read it
pub(crate) fn stopped(state: &AppState, kind: AccountKind) -> Result<(), String> {
    if !connected(state).contains(&kind) {
        for gone in state.grants().revoke_connector(kind.id())? {
            grants::record(
                state,
                "removed",
                &gone,
                Some("No account reads it any more"),
            );
        }
        events::emit(state.bus(), &GrantsChanged {});
    }
    emit_changed(state);
    Ok(())
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

/// Reads the change feed of every account's calendar Nandi may read and
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
    let sources: Vec<Source> = sources(state, AccountFamily::Calendar)
        .into_iter()
        .filter(|s| grants.allows(DEFAULT_AGENT, s.kind.id(), GrantAccess::Read))
        .collect();
    if sources.is_empty() {
        return CheckOutcome::default();
    }
    service.checking.store(true, Ordering::SeqCst);
    emit_changed(state);
    let now_secs = db::now_ms() / 1000;
    let several = sources.len() > 1;
    let mut events = Vec::new();
    let mut cursors = Vec::new();
    let mut seens = Vec::new();
    let mut checked = Vec::new();
    let mut first_read = false;
    for source in &sources {
        let kind = source.kind;
        let mut check: CheckState = load_setting(state, source.state_key()).await;
        let cursor = load_cursor(state, source).await;
        let mut seen: seen::Seen = load_setting(state, source.seen_key()).await;
        let api = service.api(source, state.secrets());
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
                    .map(|mut e| {
                        if several {
                            e.account = Some(source.account().to_string());
                        }
                        e
                    })
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
                cursors.push((source, next));
                seens.push((source, seen));
                check.needs_sign_in = false;
                check.error = None;
                check.last_found = u32::try_from(found).unwrap_or(u32::MAX);
            }
            Err(MailError::NeedsSignIn(reason)) => {
                check.needs_sign_in = true;
                check.error = Some(reason);
            }
            Err(e) => {
                log::warn!("Checking a {} failed: {e}", kind.name());
                check.error = Some(e.to_string());
            }
        }
        check.last_checked_at = Some(now());
        checked.push((source, check));
    }
    let batch = batch::CalendarBatch::new(&sources, events, first_read, now_secs);
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
                log::warn!("Nandi couldn't take the calendar check: {e}");
                for (_, check) in checked.iter_mut().filter(|(_, c)| c.error.is_none()) {
                    check.error = Some(format!("Nandi couldn't start: {e}"));
                }
                outcome.found = 0;
                false
            }
        }
    };
    if handed_over {
        for (source, cursor) in cursors {
            if let Err(e) = save_setting(state, source.cursor_key(), cursor).await {
                log::warn!("Couldn't save a {} cursor: {e}", source.kind.name());
            }
        }
        for (source, seen) in seens {
            let value = serde_json::to_value(&seen).unwrap_or_default();
            if let Err(e) = save_setting(state, source.seen_key(), value).await {
                log::warn!("Couldn't save a {}'s events seen: {e}", source.kind.name());
            }
        }
    }
    for (source, mut check) in checked {
        if outcome.run_id.is_some() && check.last_found > 0 {
            check.last_run_id = outcome.run_id.clone();
        }
        let value = serde_json::to_value(&check).unwrap_or_default();
        if let Err(e) = save_setting(state, source.state_key(), value).await {
            log::warn!("Couldn't save a {} check: {e}", source.kind.name());
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
