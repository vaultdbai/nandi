//! Pilots: host-side MCP servers that let agents control the OS (screen,
//! input, apps). Each pilot is installed and run on the host; agents reach
//! it only through capabilities the user grants per agent.
//!
//! FROZEN INTERFACE (wave 1): the types below are the contract between the
//! backend and the UI. Track A (Pilots) fills in `PilotRegistry` and the
//! service functions.
//!
//! How it fits together:
//!
//! - [`catalog`]: the built-in pilot definitions, filtered to the host OS.
//!   Nothing is installed or started until the user asks (never automatic).
//! - [`install`]: runs a pilot's `install_command`, streaming its output as
//!   `pilot://install_output`.
//! - [`detect`]: finds a pilot already installed some other way (npm global,
//!   Docker image) and records it as installed. Read-only.
//! - [`runtime`]: spawns `start_command` and health-checks
//!   `GET http://127.0.0.1:<port>/health` (2s timeout) every 30s. A failed
//!   check marks the pilot unhealthy and its tools stop being offered.
//! - [`store`]: machine-local runtime state in SQLite `local_settings`
//!   (`pilots.<id>.<field>`) and per-agent consent
//!   (`consent.<agent_id>.<pilot_id>.<capability>`). Neither is a gold-copy
//!   definition: they describe this machine, not the agent.
//! - [`tools`]: what an agent may use. An agent reaches a pilot only if its
//!   skill.md `pilots:` block names it **and** the user granted every
//!   capability the pilot exposes. The orchestrator talks to the pilot's
//!   MCP server from the host, so one agent's grant never opens another
//!   pilot to it. Every call is audited to `pilots.log` ([`audit`]).
//!
//! Containers are never told a pilot's URL: a direct connection would skip
//! consent, approval, and the audit log.

pub mod audit;
pub mod browser;
pub mod catalog;
pub mod detect;
pub mod handover;
pub mod install;
mod mcp;
pub mod refusal;
pub mod runtime;
pub mod sites;
pub mod store;
pub mod tools;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::process::Child;
use tokio::task::AbortHandle;

use crate::services::agents::validate_agent_id;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;

/// A pilot the user can install, as listed in the pilot catalog.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PilotDefinition {
    pub id: String,
    pub name: String,
    /// `macos` | `windows` | `linux` | `any`
    pub platform: String,
    pub install_command: String,
    pub start_command: String,
    /// Host loopback port the pilot's MCP server listens on
    pub mcp_port: u16,
    pub description: String,
    /// What an agent can be granted, e.g. `screen.read`, `input.keyboard`
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PilotStatus {
    pub installed: bool,
    pub running: bool,
    pub healthy: bool,
    pub pid: Option<u32>,
}

/// A pilot was installed, started, stopped, or changed health
/// (`pilot://changed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PilotChanged {
    pub pilot_id: String,
    pub status: PilotStatus,
}

impl NandiEvent for PilotChanged {
    const NAME: &'static str = "pilot://changed";
}

/// A run left out a pilot the agent declares because the user hasn't
/// granted it yet (`pilot://consent_needed`). The UI asks for consent.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PilotConsentNeeded {
    pub agent_id: String,
    pub pilot_id: String,
    /// The capabilities not granted yet
    pub capabilities: Vec<String>,
}

impl NandiEvent for PilotConsentNeeded {
    const NAME: &'static str = "pilot://consent_needed";
}

/// Installed pilots and their host processes. `AppState::pilots()` is
/// `None` until Track A initializes it.
///
/// Holds only what can't live in SQLite: the child processes (killed when
/// the app exits, so OS control never outlives Nandi) and their health
/// monitors.
#[derive(Debug, Default)]
pub struct PilotRegistry {
    running: Mutex<HashMap<String, RunningPilot>>,
    installing: Mutex<HashSet<String>>,
    /// Tells a monitor its pilot was stopped or restarted since it began
    generation: AtomicU64,
}

#[derive(Debug)]
struct RunningPilot {
    /// `None` when the server was already listening and Nandi adopted it
    child: Option<Child>,
    pid: Option<u32>,
    generation: u64,
    monitor: Option<AbortHandle>,
}

impl PilotRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_running(&self, id: &str) -> bool {
        self.lock().contains_key(id)
    }

    fn pid(&self, id: &str) -> Option<u32> {
        self.lock().get(id).and_then(|p| p.pid)
    }

    fn generation_of(&self, id: &str) -> Option<u64> {
        self.lock().get(id).map(|p| p.generation)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunningPilot>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the pilot's process has exited (adopted servers never do;
    /// their health check notices instead).
    fn has_exited(&self, id: &str) -> bool {
        let mut running = self.lock();
        match running.get_mut(id).and_then(|p| p.child.as_mut()) {
            Some(child) => !matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn insert(&self, id: &str, child: Option<Child>, pid: Option<u32>) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.lock().insert(
            id.to_string(),
            RunningPilot {
                child,
                pid,
                generation,
                monitor: None,
            },
        );
        generation
    }

    fn set_monitor(&self, id: &str, generation: u64, monitor: AbortHandle) {
        match self
            .lock()
            .get_mut(id)
            .filter(|p| p.generation == generation)
        {
            Some(pilot) => pilot.monitor = Some(monitor),
            None => monitor.abort(),
        }
    }

    /// Forgets the pilot and stops its monitor; the caller kills the child.
    fn remove(&self, id: &str) -> Option<Child> {
        let pilot = self.lock().remove(id)?;
        if let Some(monitor) = pilot.monitor {
            monitor.abort();
        }
        pilot.child
    }

    /// Forgets the pilot only if `generation` is still the current start.
    fn remove_if(&self, id: &str, generation: u64) -> Option<Option<Child>> {
        let mut running = self.lock();
        if running.get(id).map(|p| p.generation) != Some(generation) {
            return None;
        }
        running.remove(id).map(|p| p.child)
    }

    fn begin_install(&self, id: &str) -> bool {
        self.installing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string())
    }

    fn end_install(&self, id: &str) {
        self.installing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
}

fn registry(state: &AppState) -> Result<&Arc<PilotRegistry>, String> {
    state
        .pilots()
        .ok_or_else(|| "Pilots aren't available: the pilot registry isn't initialized".to_string())
}

// ---------------------------------------------------------------------------
// Service functions (one per command / route)
// ---------------------------------------------------------------------------

/// The pilots available on this OS.
pub fn list_pilots() -> Vec<PilotDefinition> {
    catalog::for_platform(catalog::current_platform())
}

/// Runs the pilot's install command. Only ever on the user's request.
pub async fn install_pilot(state: &AppState, id: &str) -> Result<(), String> {
    let pilot = catalog::find(id)?;
    let guard = state.pilots().cloned();
    if let Some(registry) = &guard {
        if !registry.begin_install(id) {
            return Err(format!("{} is already being installed", pilot.name));
        }
    }
    let result = install_definition(state.bus(), &pilot).await;
    if let Some(registry) = &guard {
        registry.end_install(id);
    }
    result
}

pub(crate) async fn install_definition(
    bus: &EventBus,
    pilot: &PilotDefinition,
) -> Result<(), String> {
    install::run(bus, &pilot.id, &pilot.install_command).await?;
    let id = pilot.id.clone();
    bus.db
        .call(move |conn| store::save_installed(conn, &id, true))
        .await?;
    announce(bus, &pilot.id).await;
    log::info!("Installed pilot '{}'", pilot.id);
    Ok(())
}

/// Starts the pilot's MCP server and returns its process ID (`0` if a
/// server was already listening on the pilot's port and Nandi adopted it).
pub async fn start_pilot(state: &AppState, id: &str) -> Result<u32, String> {
    let registry = registry(state)?.clone();
    let pilot = catalog::find(id)?;
    if !installed(state.bus(), &detect::SystemProbe, &pilot).await? {
        return Err(format!("Install {} first", pilot.name));
    }
    start_definition(state.bus(), &registry, &pilot).await
}

pub(crate) async fn start_definition(
    bus: &EventBus,
    registry: &Arc<PilotRegistry>,
    pilot: &PilotDefinition,
) -> Result<u32, String> {
    if browser::is_browser(&pilot.id) {
        turn_on_browser(bus, pilot).await?;
        return Ok(0);
    }
    if registry.is_running(&pilot.id) && !registry.has_exited(&pilot.id) {
        return Ok(registry.pid(&pilot.id).unwrap_or(0));
    }
    // A stale entry whose process died
    if let Some(child) = registry.remove(&pilot.id) {
        runtime::kill(child).await;
    }

    let adopted = runtime::health_check(pilot.mcp_port).await;
    let (child, pid) = if adopted {
        log::info!(
            "Pilot '{}' is already listening on {}; not spawning another",
            pilot.id,
            pilot.mcp_port
        );
        (None, None)
    } else {
        let child = runtime::spawn(pilot)?;
        let pid = child.id();
        (Some(child), pid)
    };
    let generation = registry.insert(&pilot.id, child, pid);

    let (id, port) = (pilot.id.clone(), pilot.mcp_port);
    bus.db
        .call(move |conn| {
            store::save_running(conn, &id, pid, port)?;
            store::save_healthy(conn, &id, adopted)
        })
        .await?;
    announce(bus, &pilot.id).await;

    let monitor = tokio::spawn(runtime::monitor(
        bus.clone(),
        registry.clone(),
        pilot.clone(),
        generation,
        adopted,
    ));
    registry.set_monitor(&pilot.id, generation, monitor.abort_handle());
    log::info!("Started pilot '{}' (pid {pid:?})", pilot.id);
    Ok(pid.unwrap_or(0))
}

/// A browser runs one server per agent, started on that agent's first use
/// ([`tools::for_agent`] turns it on too): turning it on checks it can run
/// here (Node.js and Chrome or Edge, or Docker and the image) and lets
/// agents use it.
pub(crate) async fn turn_on_browser(bus: &EventBus, pilot: &PilotDefinition) -> Result<(), String> {
    browser::prerequisites(pilot).await?;
    let id = pilot.id.clone();
    bus.db
        .call(move |conn| {
            store::save_running(conn, &id, None, 0)?;
            store::save_healthy(conn, &id, true)
        })
        .await?;
    announce(bus, &pilot.id).await;
    log::info!("Pilot '{}' is on", pilot.id);
    Ok(())
}

/// Whether `pilot` is installed: through Nandi, or found on this computer
/// ([`detect`]); a find is recorded and announced.
pub(crate) async fn installed(
    bus: &EventBus,
    probe: &impl detect::InstallProbe,
    pilot: &PilotDefinition,
) -> Result<bool, String> {
    let found = detect::installed(&bus.db, probe, pilot).await?;
    if found == detect::Found::Detected {
        announce(bus, &pilot.id).await;
    }
    Ok(found.yes())
}

pub async fn stop_pilot(state: &AppState, id: &str) -> Result<(), String> {
    let registry = registry(state)?.clone();
    let pilot = catalog::find(id)?;
    stop_definition(state.bus(), &registry, &pilot).await
}

pub(crate) async fn stop_definition(
    bus: &EventBus,
    registry: &PilotRegistry,
    pilot: &PilotDefinition,
) -> Result<(), String> {
    if browser::is_browser(&pilot.id) {
        browser::stop_all(&pilot.id).await;
    }
    if let Some(child) = registry.remove(&pilot.id) {
        runtime::kill(child).await;
    }
    let id = pilot.id.clone();
    bus.db
        .call(move |conn| store::save_stopped(conn, &id))
        .await?;
    announce(bus, &pilot.id).await;
    log::info!("Stopped pilot '{}'", pilot.id);
    Ok(())
}

pub async fn pilot_status(state: &AppState, id: &str) -> Result<PilotStatus, String> {
    let pilot = catalog::find(id)?;
    // Installed some other way counts too (read-only look)
    installed(state.bus(), &detect::SystemProbe, &pilot).await?;
    status_of(state.db(), state.pilots().map(Arc::as_ref), &pilot.id).await
}

pub(crate) async fn status_of(
    db: &crate::db::Db,
    registry: Option<&PilotRegistry>,
    id: &str,
) -> Result<PilotStatus, String> {
    let pilot_id = id.to_string();
    let persisted = db.call(move |conn| store::load(conn, &pilot_id)).await?;
    let mut status = persisted.status();
    // A browser has no process of its own in the registry (one per agent):
    // its rows, from this session, say whether it's on
    if let Some(registry) = registry.filter(|_| !browser::is_browser(id)) {
        // The registry, not a row from an earlier session, says what runs
        status.running = registry.is_running(id);
        status.healthy &= status.running;
        status.pid = registry.pid(id);
    }
    Ok(status)
}

/// Records the user's consent for `agent_id` to use one capability of a
/// pilot. The agent must also declare the pilot in its skill.md.
pub async fn grant_pilot_capability(
    state: &AppState,
    agent_id: &str,
    pilot_id: &str,
    capability: &str,
) -> Result<(), String> {
    validate_agent_id(agent_id)?;
    if state.store().agent_info(agent_id).is_none() {
        return Err(format!("Agent '{agent_id}' not found"));
    }
    let pilot = catalog::find(pilot_id)?;
    grant(state.db(), agent_id, &pilot, capability).await
}

pub(crate) async fn grant(
    db: &crate::db::Db,
    agent_id: &str,
    pilot: &PilotDefinition,
    capability: &str,
) -> Result<(), String> {
    if !pilot.capabilities.iter().any(|c| c == capability) {
        return Err(format!("{} has no capability '{capability}'", pilot.name));
    }
    log::info!("Granting '{agent_id}' {}.{capability}", pilot.id);
    let (agent, id, capability) = (
        agent_id.to_string(),
        pilot.id.clone(),
        capability.to_string(),
    );
    db.call(move |conn| store::grant(conn, &agent, &id, &capability))
        .await
}

/// Withdraws a capability granted with [`grant_pilot_capability`]. The
/// agent loses the pilot's tools from its next step.
pub async fn revoke_pilot_capability(
    state: &AppState,
    agent_id: &str,
    pilot_id: &str,
    capability: &str,
) -> Result<(), String> {
    validate_agent_id(agent_id)?;
    let pilot = catalog::find(pilot_id)?;
    revoke(state.db(), agent_id, &pilot, capability).await
}

pub(crate) async fn revoke(
    db: &crate::db::Db,
    agent_id: &str,
    pilot: &PilotDefinition,
    capability: &str,
) -> Result<(), String> {
    if !pilot.capabilities.iter().any(|c| c == capability) {
        return Err(format!("{} has no capability '{capability}'", pilot.name));
    }
    log::info!("Revoking '{agent_id}' {}.{capability}", pilot.id);
    let (agent, id, capability) = (
        agent_id.to_string(),
        pilot.id.clone(),
        capability.to_string(),
    );
    db.call(move |conn| store::revoke(conn, &agent, &id, &capability))
        .await
}

/// What an agent may do with each pilot it declares (for the agent's
/// Permissions view).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPilotPermission {
    pub pilot_id: String,
    /// Declared in skill.md but not in this OS's catalog
    pub available: bool,
    pub capabilities: Vec<String>,
    pub granted: Vec<String>,
    /// What skill.md lets it do without asking (`allow:`)
    #[serde(default)]
    pub allows: Vec<String>,
    /// The secrets skill.md names for it, and whether each is in the
    /// keychain (never the value)
    #[serde(default)]
    pub secrets: Vec<PilotSecretStatus>,
    /// A browser pilot: it takes site secrets and keeps a profile
    #[serde(default)]
    pub browser: bool,
    /// Its browser has a profile, so sites may remember the agent
    #[serde(default)]
    pub has_profile: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PilotSecretStatus {
    /// What the agent types, e.g. `SITE_PASSWORD`
    pub name: String,
    pub is_set: bool,
}

/// The pilots `agent_id` declares, and which capabilities the user granted.
pub async fn agent_pilot_permissions(
    state: &AppState,
    agent_id: &str,
) -> Result<Vec<AgentPilotPermission>, String> {
    validate_agent_id(agent_id)?;
    let definition = state.store().agent(agent_id)?;
    let declared = tools::declared(&definition);
    let mut permissions = Vec::new();
    for pilot_id in declared {
        let pilot = catalog::find(&pilot_id).ok();
        let (agent, id) = (agent_id.to_string(), pilot_id.clone());
        let granted = state
            .db()
            .call(move |conn| store::granted(conn, &agent, &id))
            .await?;
        let options = definition.capabilities().pilot_options(&pilot_id);
        let secrets = options
            .map(|o| {
                o.secrets
                    .iter()
                    .map(|(name, entry)| PilotSecretStatus {
                        name: name.clone(),
                        is_set: state
                            .secrets()
                            .get(entry.as_str())
                            .ok()
                            .flatten()
                            .is_some_and(|v| !v.is_empty()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let browser = browser::is_browser(&pilot_id);
        permissions.push(AgentPilotPermission {
            allows: options.map(|o| o.allow.clone()).unwrap_or_default(),
            secrets,
            browser,
            has_profile: browser && browser::has_profile(agent_id, &pilot_id),
            pilot_id,
            available: pilot.is_some(),
            capabilities: pilot.map(|p| p.capabilities).unwrap_or_default(),
            granted,
        });
    }
    Ok(permissions)
}

/// Saves the value of a secret `agent_id`'s skill.md names for `pilot_id`
/// (`secrets: {NAME: <keychain entry>}`) in the keychain; empty removes it.
/// Write-only: the value is never read back.
pub async fn set_pilot_secret(
    state: &AppState,
    agent_id: &str,
    pilot_id: &str,
    name: &str,
    value: &str,
) -> Result<(), String> {
    validate_agent_id(agent_id)?;
    let agent = state.store().agent(agent_id)?;
    let entry = agent
        .capabilities()
        .pilot_options(pilot_id)
        .and_then(|o| o.secrets.get(name))
        .ok_or_else(|| {
            format!("This agent's skill.md doesn't name a secret '{name}' for {pilot_id}")
        })?
        .clone();
    if value.is_empty() {
        state.secrets().delete(entry.as_str())?;
    } else {
        state.secrets().set(entry.as_str(), value)?;
    }
    log::info!("Pilot secret '{name}' for {agent_id}/{pilot_id} updated");
    Ok(())
}

/// Adds a site secret to `agent_id`'s `pilot_id` browser: its name goes
/// into skill.md (`secrets: {NAME: nandi.pilot.<agent>.<name>}`, through
/// the repository) and its value into the keychain. Only for agents made
/// on this computer: a pack's agent names its secrets itself.
pub async fn add_pilot_secret(
    state: &AppState,
    agent_id: &str,
    pilot_id: &str,
    name: &str,
    value: &str,
) -> Result<(), String> {
    use crate::services::domain::EntityKind;
    use crate::services::repos::agent_locator::{self, Origin};
    use crate::services::repos::Repository;

    validate_agent_id(agent_id)?;
    if !browser::is_browser(pilot_id) {
        return Err(format!(
            "{pilot_id} doesn't type secrets: only a browser does"
        ));
    }
    if value.is_empty() {
        return Err("Give the secret a value".to_string());
    }
    let store = state.store();
    let local = agent_locator::locate(store.layout(), agent_id)
        .is_none_or(|place| place.origin == Origin::Local);
    if !local {
        return Err(
            "This agent came with a pack: it names its own secrets, and you can set their values"
                .to_string(),
        );
    }
    let mut agent = store.agent(agent_id)?;
    let name = name.trim();
    let entry = agent
        .add_pilot_secret(pilot_id, name)
        .map_err(|e| e.to_string())?;
    store
        .repos()
        .agents
        .save(&agent)
        .map_err(|e| e.to_string())?;
    store.commit(EntityKind::Agent, agent_id)?;
    state.secrets().set(entry.as_str(), value)?;
    log::info!("Site secret '{name}' added for {agent_id}/{pilot_id}");
    Ok(())
}

/// "Sign out everywhere": stops `agent_id`'s `pilot_id` browser and clears
/// its profile, so no site remembers the agent. `false`: it had none
pub async fn clear_browser_profile(
    state: &AppState,
    agent_id: &str,
    pilot_id: &str,
) -> Result<bool, String> {
    validate_agent_id(agent_id)?;
    if state.store().agent_info(agent_id).is_none() {
        return Err(format!("Agent '{agent_id}' not found"));
    }
    if !browser::is_browser(pilot_id) {
        return Err(format!("{pilot_id} has no browser profile"));
    }
    browser::sign_out(agent_id, pilot_id).await
}

/// Emits `pilot://changed` with the persisted status.
async fn announce(bus: &EventBus, id: &str) {
    match status_of(&bus.db, None, id).await {
        Ok(status) => events::emit(
            bus,
            &PilotChanged {
                pilot_id: id.to_string(),
                status,
            },
        ),
        Err(e) => log::warn!("Failed to read pilot '{id}' status: {e}"),
    }
}
