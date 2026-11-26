//! The Lock box: keys agents can use but never see.
//!
//! ```text
//! Settings › Lock box ─▶ add: the value goes straight into the OS keychain
//!                         (`nandi.lockbox.<name>`); state.db keeps the rules
//!
//! agent ─▶ POST /fetch { url, key: "<name>" } ─▶ egress proxy ─▶ release():
//!   known key?                     no ─▶ refused
//!   agent allowed?                 no ─▶ refused, on the record
//!   https to a bound host on 443?  no ─▶ refused, on the record
//!   a header to put it in?         no ─▶ refused
//!   lock level  Open    ─▶ released
//!               Ask     ─▶ approval card (key, agent, host), waits: yes ─▶ released
//!               Locked  ─▶ unlocked now? ─▶ released
//!                          else the OS prompt (Windows Hello) ─▶ unlocked for its time
//!                          no OS prompt here ─▶ Ask each time, and says so
//!   ─▶ the record (`inbox::audit`): key, agent, run, host, lock, who
//! ```
//!
//! The rules are enforced here, in code, at release time: never in a
//! prompt. The value is read from the keychain only after every rule
//! passed, and only the proxy holds it, for that one request. It never
//! goes into a log, a run record, the gold copy, the relay, a backup or an
//! error message.

pub mod approvals;
mod store;
pub mod unlock;

#[cfg(test)]
pub(crate) mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db;
use crate::services::domain::lock_box::{self as rules, Destination, KeyRules, LockLevel};
use crate::services::inbox::audit::{self, Decided};
use crate::services::AppState;

use unlock::{OsUnlock, SystemUnlock, UnlockOutcome};

/// The keychain entry of a key: `nandi.lockbox.<name>`
const KEYCHAIN_PREFIX: &str = "nandi.lockbox.";
/// The synthetic tool of a key release's approval card
pub const KEY_TOOL: &str = "lock_box_key";
/// The decision record's kind for a release
pub const RECORD_KIND: &str = "keyRelease";
/// How long a request waits for an Ask each time card
const ASK_WAIT: Duration = Duration::from_secs(10 * 60);
/// Who released an Open key
const OPEN_RULE: &str = "Lock box rule (open)";
/// Who refused a release the rules don't allow
const RULES: &str = "Lock box rules";

fn keychain_entry(name: &str) -> String {
    format!("{KEYCHAIN_PREFIX}{name}")
}

/// A Locked key's open unlock
#[derive(Debug, Clone)]
struct Unlocked {
    until: i64,
    /// `Dev Chechi with Windows Hello`
    by: String,
}

/// What the Lock box keeps in memory: open unlocks (never on disk, so a
/// restart locks every key again) and the OS prompt
pub struct LockBoxState {
    unlocks: Mutex<HashMap<String, Unlocked>>,
    unlocker: RwLock<Arc<dyn SystemUnlock>>,
    /// One OS prompt at a time
    prompting: tokio::sync::Mutex<()>,
    /// For tests: time moved forward, in ms
    #[cfg(test)]
    skew_ms: std::sync::atomic::AtomicI64,
}

impl Default for LockBoxState {
    fn default() -> Self {
        Self {
            unlocks: Mutex::default(),
            unlocker: RwLock::new(Arc::new(OsUnlock)),
            prompting: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            skew_ms: std::sync::atomic::AtomicI64::new(0),
        }
    }
}

impl LockBoxState {
    fn now(&self) -> i64 {
        #[cfg(test)]
        {
            db::now_ms() + self.skew_ms.load(std::sync::atomic::Ordering::Relaxed)
        }
        #[cfg(not(test))]
        {
            db::now_ms()
        }
    }

    fn unlocker(&self) -> Arc<dyn SystemUnlock> {
        self.unlocker
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn unlocks(&self) -> std::sync::MutexGuard<'_, HashMap<String, Unlocked>> {
        self.unlocks.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The open unlock of `name`; an expired one is dropped
    fn unlocked(&self, name: &str) -> Option<Unlocked> {
        let now = self.now();
        let mut unlocks = self.unlocks();
        match unlocks.get(name) {
            Some(open) if open.until > now => Some(open.clone()),
            Some(_) => {
                unlocks.remove(name);
                None
            }
            None => None,
        }
    }

    fn unlock(&self, name: &str, minutes: u32, by: String) -> i64 {
        let until = self.now() + i64::from(minutes) * 60_000;
        self.unlocks()
            .insert(name.to_string(), Unlocked { until, by });
        until
    }

    fn lock(&self, name: &str) {
        self.unlocks().remove(name);
    }

    #[cfg(test)]
    pub fn set_unlocker(&self, unlocker: Arc<dyn SystemUnlock>) {
        *self.unlocker.write().unwrap_or_else(|e| e.into_inner()) = unlocker;
    }

    #[cfg(test)]
    pub fn advance(&self, ms: i64) {
        self.skew_ms
            .fetch_add(ms, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One key as the Lock box page shows it. Never its value.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyView {
    pub name: String,
    pub rules: KeyRules,
    /// Whether its value is in the keychain
    pub stored: bool,
    #[specta(type = f64)]
    pub created_at: i64,
    #[specta(type = Option<f64>)]
    pub last_used_at: Option<i64>,
    /// The agent that used it last
    pub last_used_by: Option<String>,
    pub last_used_host: Option<String>,
    /// A Locked key's open unlock ends then (epoch ms)
    #[specta(type = Option<f64>)]
    pub unlocked_until: Option<i64>,
}

/// The Lock box page
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LockBoxOverview {
    pub keys: Vec<KeyView>,
    /// The OS prompt a Locked key uses here (`Windows Hello`); `None`:
    /// Locked keys ask each time on this computer
    pub system_unlock: Option<String>,
}

/// A key to add: its value is typed once and goes straight into the
/// keychain
#[derive(Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewKey {
    pub name: String,
    pub value: String,
    pub rules: KeyRules,
}

impl std::fmt::Debug for NewKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewKey")
            .field("name", &self.name)
            .field("value", &"[hidden]")
            .field("rules", &self.rules)
            .finish()
    }
}

/// A key released for one request: the header and its value. Only the
/// egress proxy holds it, and only while it sends that request.
#[derive(Clone)]
pub struct ReleasedKey {
    /// Lowercase header name
    pub header: String,
    /// `<prefix><key>`
    pub header_value: String,
    /// The key alone, so the proxy can take it out of what comes back
    pub secret: String,
}

impl std::fmt::Debug for ReleasedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleasedKey")
            .field("header", &self.header)
            .field("value", &"[hidden]")
            .finish()
    }
}

async fn rows(state: &AppState) -> Result<Vec<store::KeyRow>, String> {
    state.db().call(|conn| store::list(conn)).await
}

async fn row(state: &AppState, name: &str) -> Result<Option<store::KeyRow>, String> {
    let name = name.to_string();
    state.db().call(move |conn| store::get(conn, &name)).await
}

/// Every key, by name, and the OS prompt Locked keys use here
pub async fn overview(state: &AppState) -> Result<LockBoxOverview, String> {
    let lock_box = state.lock_box();
    let secrets = state.secrets().clone();
    let keys = rows(state)
        .await?
        .into_iter()
        .map(|row| KeyView {
            stored: secrets
                .get(&keychain_entry(&row.name))
                .is_ok_and(|v| v.is_some()),
            unlocked_until: lock_box.unlocked(&row.name).map(|u| u.until),
            name: row.name,
            rules: row.rules,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
            last_used_by: row.last_used_by,
            last_used_host: row.last_used_host,
        })
        .collect();
    let system_unlock = lock_box.unlocker().available().await.map(str::to_string);
    Ok(LockBoxOverview {
        keys,
        system_unlock,
    })
}

/// Checks that every allowed agent exists
fn known_agents(state: &AppState, rules: &KeyRules) -> Result<(), String> {
    for agent in &rules.agents {
        state.store().require_agent(agent)?;
    }
    Ok(())
}

/// Adds a key: its value into the keychain, its rules into state.db
pub async fn add(state: &AppState, key: NewKey) -> Result<(), String> {
    let name = rules::validate_name(&key.name)?;
    let rules = key.rules.validated()?;
    known_agents(state, &rules)?;
    let value = key.value.trim();
    if value.is_empty() {
        return Err("Type the key's value".to_string());
    }
    if value.contains(['\r', '\n']) {
        return Err("A key's value is one line".to_string());
    }
    if row(state, &name).await?.is_some() {
        return Err(format!("There is already a key named '{name}'"));
    }
    state.secrets().set(&keychain_entry(&name), value)?;
    let (n, r, now) = (name.clone(), rules.clone(), db::now_ms());
    let added = state
        .db()
        .call(move |conn| store::insert(conn, &n, &r, now))
        .await;
    match added {
        Ok(true) => {}
        Ok(false) => return Err(format!("There is already a key named '{name}'")),
        Err(e) => {
            // No rules, so no value either
            let _ = state.secrets().delete(&keychain_entry(&name));
            return Err(e);
        }
    }
    log::info!("Lock box: key '{name}' added");
    audit::record(
        state,
        Decided::new(RECORD_KIND, &name, "added").subject(rules_subject(&name, &rules)),
    );
    Ok(())
}

/// The record's few words on a key's rules
fn rules_subject(name: &str, rules: &KeyRules) -> String {
    format!(
        "key '{name}': sites {}; agents {}; {}",
        list_or_none(&rules.hosts),
        list_or_none(&rules.agents),
        rules.lock.label()
    )
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

/// New rules for a key (its value stays as it is). A key that is no
/// longer Locked loses its open unlock.
pub async fn update(state: &AppState, name: String, rules: KeyRules) -> Result<(), String> {
    let name = rules::validate_name(&name)?;
    let rules = rules.validated()?;
    known_agents(state, &rules)?;
    let (n, r, now) = (name.clone(), rules.clone(), db::now_ms());
    let changed = state
        .db()
        .call(move |conn| store::update(conn, &n, &r, now))
        .await?;
    if !changed {
        return Err(format!("There is no key named '{name}'"));
    }
    // A new unlock time or lock level starts from locked
    state.lock_box().lock(&name);
    audit::record(
        state,
        Decided::new(RECORD_KIND, &name, "changed").subject(rules_subject(&name, &rules)),
    );
    Ok(())
}

/// Removes a key: its keychain entry first, then its rules
pub async fn delete(state: &AppState, name: String) -> Result<(), String> {
    let name = rules::validate_name(&name)?;
    if row(state, &name).await?.is_none() {
        return Err(format!("There is no key named '{name}'"));
    }
    state.secrets().delete(&keychain_entry(&name))?;
    let n = name.clone();
    state.db().call(move |conn| store::delete(conn, &n)).await?;
    state.lock_box().lock(&name);
    log::info!("Lock box: key '{name}' deleted");
    audit::record(state, Decided::new(RECORD_KIND, &name, "deleted"));
    Ok(())
}

/// Unlocks a Locked key now, through the OS prompt, for its unlock time.
/// Returns when the unlock ends.
pub async fn unlock_now(state: &AppState, name: String) -> Result<f64, String> {
    let name = rules::validate_name(&name)?;
    let row = row(state, &name)
        .await?
        .ok_or_else(|| format!("There is no key named '{name}'"))?;
    if row.rules.lock != LockLevel::Locked {
        return Err(format!("Key '{name}' isn't Locked"));
    }
    let lock_box = state.lock_box();
    let unlocker = lock_box.unlocker();
    let Some(prompt) = unlocker.available().await else {
        return Err(
            "This computer has no system unlock Nandi can use: this key asks each time instead"
                .to_string(),
        );
    };
    // One OS prompt at a time, held until it is answered
    let _prompt_guard = lock_box.prompting.lock().await;
    let message = format!(
        "Unlock the key '{name}' for {} minutes",
        row.rules.unlock_minutes
    );
    match unlocker.verify(&message).await {
        UnlockOutcome::Verified => {
            let by = format!("{} with {prompt}", crate::services::chat::decider(state));
            let until = lock_box.unlock(&name, row.rules.unlock_minutes, by.clone());
            audit::record(
                state,
                Decided::new(RECORD_KIND, &name, "unlocked")
                    .subject(format!("key '{name}'"))
                    .note(Some(&format!(
                        "for {} minutes, by {by}",
                        row.rules.unlock_minutes
                    ))),
            );
            Ok(until as f64)
        }
        UnlockOutcome::Declined => Err(format!("{prompt} didn't unlock it")),
        UnlockOutcome::Unavailable => Err(format!("{prompt} couldn't ask right now")),
    }
}

/// Locks a key again now
pub async fn lock_now(state: &AppState, name: String) -> Result<(), String> {
    let name = rules::validate_name(&name)?;
    state.lock_box().lock(&name);
    Ok(())
}

/// The few lines an agent's prompt gets about the keys it may use: names
/// and sites only, and how to ask the proxy for one. Empty without keys.
pub async fn prompt(state: &AppState, agent: &str) -> String {
    let Ok(rows) = rows(state).await else {
        return String::new();
    };
    let usable: Vec<String> = rows
        .iter()
        .filter(|row| row.rules.allows_agent(agent) && row.rules.header.is_some())
        .filter(|row| !row.rules.hosts.is_empty())
        .map(|row| format!("- `{}`: {}", row.name, row.rules.hosts.join(", ")))
        .collect();
    if usable.is_empty() {
        return String::new();
    }
    format!(
        "\n\n## Keys from the Lock box\n\n\
         You may use these keys, by name, at their sites only. You never see a key's value. \
         To call an API with one, send the request through the egress proxy: \
         `POST $NANDI_FETCH_PROXY` with the header `Authorization: Bearer $NANDI_LLM_PROXY_TOKEN` \
         and a JSON body `{{\"url\": \"https://...\", \"method\": \"GET\", \"headers\": {{}}, \"body\": \"...\", \"key\": \"<name>\"}}`. \
         The proxy adds the key; the person may be asked first.\n\n{}\n",
        usable.join("\n")
    )
}

/// Why a release was refused, for the agent (never with the value)
fn refused(name: &str, why: &str) -> String {
    format!("Key '{name}' wasn't released: {why}")
}

/// A refusal recorded once in a run, and how often it came again since
struct Repeat {
    count: u32,
    decisions: crate::services::repos::DecisionRepository,
    /// The entry recorded the first time
    first: Decided,
}

/// What the Lock box remembers about one run until it ends, so a looping
/// agent can neither raise the OS prompt again nor fill the record
#[derive(Default)]
struct RunMemory {
    /// (key, host) whose OS unlock the person cancelled or failed
    unlock_declined: std::collections::HashSet<(String, String)>,
    /// Refusals by what they say: recorded once, then counted
    repeated: HashMap<String, Repeat>,
}

static RUNS: std::sync::LazyLock<Mutex<HashMap<String, RunMemory>>> =
    std::sync::LazyLock::new(Mutex::default);

fn runs() -> std::sync::MutexGuard<'static, HashMap<String, RunMemory>> {
    RUNS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The run ended (`EventBus::turn_done`): one summary line for each
/// refusal that came again, then the run's memory goes
pub fn end_run(run_id: &str) {
    let Some(memory) = runs().remove(run_id) else {
        return;
    };
    for repeat in memory.repeated.into_values().filter(|r| r.count > 0) {
        let Repeat {
            count,
            decisions,
            first,
        } = repeat;
        let note = format!(
            "{}; the same refusal {count} more time{} in this run",
            first.note.clone().unwrap_or_default(),
            if count == 1 { "" } else { "s" }
        );
        audit::record_in(&decisions, first.note(Some(&note)));
    }
}

/// What the record says about one release or refusal
struct Release<'a> {
    name: &'a str,
    agent: &'a str,
    run_id: Option<&'a str>,
    host: &'a str,
    lock: LockLevel,
}

impl Release<'_> {
    fn record(&self, state: &AppState, decision: &str, by: Option<String>, note: String) {
        let mut decided = Decided::new(RECORD_KIND, self.run_id.unwrap_or("no run"), decision)
            .subject(format!(
                "key '{}' for agent '{}' to {}",
                self.name, self.agent, self.host
            ))
            .note(Some(&format!("lock: {}; {note}", self.lock.label())));
        if let Some(by) = by {
            decided = decided.by(by);
        }
        // A refusal repeated in the same run is counted, not written again
        if let ("refused", Some(run_id)) = (decision, self.run_id) {
            let signature = format!("{}\n{}\n{}\n{note}", self.name, self.agent, self.host);
            let mut runs = runs();
            let memory = runs.entry(run_id.to_string()).or_default();
            if let Some(repeat) = memory.repeated.get_mut(&signature) {
                repeat.count += 1;
                return;
            }
            let who = decided
                .by
                .clone()
                .unwrap_or_else(|| crate::services::chat::decider(state));
            memory.repeated.insert(
                signature,
                Repeat {
                    count: 0,
                    decisions: state.store().repos().decisions.clone(),
                    first: decided.clone().by(who),
                },
            );
        }
        audit::record(state, decided);
    }
}

/// Releases key `name` for one request of `agent` to `url`, if every rule
/// allows it now. Waits for the person when its lock level asks.
pub async fn release(
    state: &AppState,
    agent: &str,
    name: &str,
    url: &Url,
) -> Result<ReleasedKey, String> {
    let shown = name.trim();
    let Ok(name) = rules::validate_name(shown) else {
        return Err(refused(
            shown,
            "there is no key of that name in the Lock box",
        ));
    };
    let Some(row) = row(state, &name).await? else {
        return Err(refused(
            &name,
            "there is no key of that name in the Lock box",
        ));
    };
    let key_rules = &row.rules;
    let run = {
        let agent = agent.to_string();
        state
            .db()
            .call(move |conn| db::active_agent_run(conn, &agent))
            .await
            .ok()
            .flatten()
    };
    let host_shown = url.host_str().unwrap_or_default().to_string();
    let release = Release {
        name: &name,
        agent,
        run_id: run.as_ref().map(|r| r.id.as_str()),
        host: &host_shown,
        lock: key_rules.lock,
    };
    if !key_rules.allows_agent(agent) {
        release.record(
            state,
            "refused",
            Some(RULES.into()),
            "agent not allowed".into(),
        );
        return Err(refused(&name, "this agent isn't allowed to use it"));
    }
    let host = match rules::destination(key_rules, url) {
        Destination::Bound { host, .. } => host,
        Destination::NotHttps => {
            release.record(
                state,
                "refused",
                Some(RULES.into()),
                "not https on port 443".into(),
            );
            return Err(refused(
                &name,
                "keys go only over https, on the default port, to a site name",
            ));
        }
        Destination::NotBound(host) => {
            release.record(
                state,
                "refused",
                Some(RULES.into()),
                "site not bound".into(),
            );
            return Err(refused(
                &name,
                &format!("it works only at its own sites, and {host} isn't one of them"),
            ));
        }
    };
    let Some(header) = key_rules.header.clone() else {
        return Err(refused(
            &name,
            "it has no header for the proxy to put it in (set one in the Lock box)",
        ));
    };

    // Never unlocked for an ask from a phone (Phone Controls): only a key
    // left open works in a phone-started run
    if key_rules.lock != LockLevel::Open && phone_run(run.as_ref()) {
        release.record(
            state,
            "refused",
            Some(RULES.into()),
            "asked from a phone".into(),
        );
        return Err(refused(
            &name,
            "a key that asks first is never unlocked for an ask from a phone",
        ));
    }
    let (decided_by, note) = match key_rules.lock {
        LockLevel::Open => (OPEN_RULE.to_string(), "released on its site".to_string()),
        LockLevel::Ask => match ask(state, &release, run.as_ref()).await {
            Ok(by) => (by, "approved on its card".to_string()),
            Err(e) => return Err(e),
        },
        LockLevel::Locked => locked(state, &release, run.as_ref(), key_rules).await?,
    };

    let value = match state.secrets().read(&keychain_entry(&name)) {
        Ok(Some(value)) => value,
        Ok(None) => return Err(refused(&name, "its value isn't in the keychain any more")),
        Err(_) => {
            return Err(refused(
                &name,
                &format!(
                    "{} couldn't read it",
                    crate::services::secrets::keychain_name()
                ),
            ))
        }
    };
    let header_value = format!("{}{value}", header.prefix);
    if axum::http::HeaderValue::from_str(&header_value).is_err() {
        return Err(refused(&name, "its value can't go in a request header"));
    }
    release.record(state, "released", Some(decided_by), note);
    let (n, a, h, now) = (name.clone(), agent.to_string(), host, db::now_ms());
    if let Err(e) = state
        .db()
        .call(move |conn| store::used(conn, &n, &a, &h, now))
        .await
    {
        log::warn!("Lock box: couldn't note the use of '{name}': {e}");
    }
    Ok(ReleasedKey {
        header: header.name,
        header_value,
        secret: value,
    })
}

/// Whether `run` was asked from a paired phone
fn phone_run(run: Option<&crate::model::RunSummary>) -> bool {
    run.and_then(crate::services::own_devices::controls::asked_from)
        .is_some()
}

/// The approval card's arguments: the key, the agent and the host. Never
/// the value.
fn card(release: &Release<'_>, locked_fallback: bool) -> serde_json::Value {
    serde_json::json!({
        "title": format!("Use the key '{}' at {}", release.name, release.host),
        "key": release.name,
        "agent": release.agent,
        "host": release.host,
        "lock": release.lock.as_str(),
        "noSystemUnlock": locked_fallback,
    })
}

/// Ask each time: the card, then the person's answer
async fn ask(
    state: &AppState,
    release: &Release<'_>,
    run: Option<&crate::model::RunSummary>,
) -> Result<String, String> {
    ask_card(state, release, run, false).await
}

async fn ask_card(
    state: &AppState,
    release: &Release<'_>,
    run: Option<&crate::model::RunSummary>,
    locked_fallback: bool,
) -> Result<String, String> {
    let Some(run) = run else {
        release.record(
            state,
            "refused",
            Some(RULES.into()),
            "no run to ask in".into(),
        );
        return Err(refused(
            release.name,
            "it asks each time, and there is no run to ask in",
        ));
    };
    // Declined once in this run: not asked again, so a looping agent
    // can't wear the person down with cards
    if approvals::declined_in_run(state, &run.id, release.name, release.host).await {
        return Err(refused(
            release.name,
            "the person didn't allow it earlier in this task. Don't ask for it again",
        ));
    }
    match approvals::ask(state, run, card(release, locked_fallback), ASK_WAIT).await {
        approvals::Answer::Approved(by) => Ok(by),
        approvals::Answer::Declined(by) => {
            let by = by.unwrap_or_else(|| "nobody: the card expired".to_string());
            release.record(state, "refused", Some(by), "declined on its card".into());
            Err(refused(
                release.name,
                "the person didn't allow it. Don't ask for it again in this task",
            ))
        }
    }
}

/// Locked: an open unlock, else the OS prompt, else Ask each time
async fn locked(
    state: &AppState,
    release: &Release<'_>,
    run: Option<&crate::model::RunSummary>,
    key_rules: &KeyRules,
) -> Result<(String, String), String> {
    let lock_box = state.lock_box();
    if let Some(open) = lock_box.unlocked(release.name) {
        return Ok((open.by, "unlocked".to_string()));
    }
    let unlocker = lock_box.unlocker();
    let Some(prompt) = unlocker.available().await else {
        let by = ask_card(state, release, run, true).await?;
        return Ok((
            by,
            "approved on its card (no system unlock on this computer)".to_string(),
        ));
    };
    let prompt_guard = lock_box.prompting.lock().await;
    // Unlocked while this request waited for the prompt
    if let Some(open) = lock_box.unlocked(release.name) {
        return Ok((open.by, "unlocked".to_string()));
    }
    // Cancelled or failed once in this run: the prompt isn't raised again
    let declined = (release.name.to_string(), release.host.to_string());
    if release.run_id.is_some_and(|run| {
        runs()
            .get(run)
            .is_some_and(|memory| memory.unlock_declined.contains(&declined))
    }) {
        release.record(
            state,
            "refused",
            Some(RULES.into()),
            format!("{prompt} declined earlier in this run"),
        );
        return Err(refused(
            release.name,
            "the person didn't unlock it earlier in this task. Don't ask for it again",
        ));
    }
    let agent_name = state
        .store()
        .agent(release.agent)
        .map(|a| a.name().to_string())
        .unwrap_or_else(|_| release.agent.to_string());
    let message = format!(
        "{agent_name} wants to use the key '{}' at {}. Unlock it for {} minutes?",
        release.name, release.host, key_rules.unlock_minutes
    );
    match unlocker.verify(&message).await {
        UnlockOutcome::Verified => {
            let by = format!("{} with {prompt}", crate::services::chat::decider(state));
            lock_box.unlock(release.name, key_rules.unlock_minutes, by.clone());
            Ok((
                by,
                format!("unlocked for {} minutes", key_rules.unlock_minutes),
            ))
        }
        UnlockOutcome::Declined => {
            // Outside a run there is nothing to remember it for
            if let Some(run) = release.run_id {
                runs()
                    .entry(run.to_string())
                    .or_default()
                    .unlock_declined
                    .insert(declined);
            }
            release.record(
                state,
                "refused",
                Some(crate::services::chat::decider(state)),
                format!("{prompt} not unlocked"),
            );
            Err(refused(
                release.name,
                "the person didn't unlock it. Don't ask for it again in this task",
            ))
        }
        UnlockOutcome::Unavailable => {
            drop(prompt_guard);
            let by = ask_card(state, release, run, true).await?;
            Ok((by, format!("approved on its card ({prompt} couldn't ask)")))
        }
    }
}

/// The egress proxy's way to the Lock box
pub struct Releaser(pub AppState);

#[async_trait::async_trait]
impl crate::services::egress::KeyRelease for Releaser {
    async fn release(&self, agent: &str, key: &str, url: &Url) -> Result<ReleasedKey, String> {
        release(&self.0, agent, key, url).await
    }
}
