//! Runtime connectors: MCP servers that give
//! agents access to external services. The orchestrator merges a running
//! connector's tools into an agent's tool list.
//!
//! Connector *definitions* are gold-copy files (`services::repos::connector_repo`);
//! this module owns their runtime side: install, run and health.
//!
//! ```text
//! .nandi/connectors.yaml ──install──▶ declared ──start (or first use)──▶ running ──▶ tools
//! agent skill.md `connectors: [filesystem]` ──▶ that agent sees `filesystem__read_file`, …
//! ```
//!
//! - Connectors are disabled by default: nothing runs until it's installed,
//!   and an agent only sees the tools of connectors its skill.md declares.
//! - Tool names are prefixed `<connector_id>__` (see `tools`), so they can't
//!   collide with the container's tools.
//! - Sensitive tools (marked in [`ConnectorDefinition`]) go through the
//!   orchestrator's approval flow.
//! - Health checks time out after [`HEALTH_TIMEOUT`]; an unhealthy
//!   connector's tools are left out of the turn and the agent gets a warning.
//! - Every call is audited to `~/.nandi/local/logs/connectors.log` (JSONL,
//!   argument names only).
//! - Secrets (API keys, tokens) live in the OS keyring only.
//!
//! FROZEN INTERFACE (wave 1): the types below are the contract between the
//! backend and the UI. Track D (Connectors) fills in `ConnectorRegistry` and
//! the service functions.

pub mod audit;
mod aws;
mod backup_triggers;
mod catalog;
mod filesystem;
pub mod folders;
mod github;
mod mcp;
mod s3_backup;
mod tools;
mod web_search;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::services::domain::connector::Connector as ConnectorDecl;
use crate::services::domain::{self, ConnectorId, ConnectorManifest, EntityKind};
use crate::services::events::{self, NandiEvent};
use crate::services::repos::{Repository, SingletonRepository};
use crate::services::secrets::Secrets;
use crate::services::AppState;

pub use catalog::{definition_for, ConnectorDefinition, MCP as MCP_KIND};

/// Connectors that are the person's own accounts, signed in under
/// Settings › Email / Calendar / Files rather than installed: their tools
/// run on the host (`services::email`, `services::workspace`). An agent
/// names them in skill.md `connectors:` like any other
pub const ACCOUNT_CONNECTORS: &[&str] = &[
    "outlook",
    "gmail",
    "outlook-calendar",
    "google-calendar",
    "onedrive",
    "google-drive",
];

pub fn is_account_connector(id: &str) -> bool {
    ACCOUNT_CONNECTORS.contains(&id)
}

/// The instructions of the installed packages `agent` declares and holds a
/// grant on, for its system prompt
pub fn package_instructions(state: &AppState, agent: &str, declared: &[String]) -> String {
    let grants = state.grants();
    let mut out = String::new();
    for id in declared {
        let granted = [
            domain::grant::GrantAccess::Read,
            domain::grant::GrantAccess::Write,
        ]
        .into_iter()
        .any(|access| grants.allows(agent, id, access));
        let Ok(Some(package)) = state.store().repos().connectors.load_package(id) else {
            continue;
        };
        if granted && !package.instructions.is_empty() {
            out.push_str(&format!(
                "\n\n## Connector: {} (tools `{}__*`)\n\n{}",
                package.name,
                tools::sanitize(id),
                package.instructions
            ));
        } else if !granted {
            out.push_str(&format!(
                "\n\nThe {} connector's tools ask the person for access the first time you use them.",
                package.name
            ));
        }
    }
    out
}

/// The IDs of the connectors the catalog offers (`github`, `s3_backup`, …)
#[cfg(test)]
pub(crate) fn builtin_ids() -> Vec<String> {
    catalog::builtins().into_iter().map(|d| d.id).collect()
}
pub use filesystem::expand_home;
pub use mcp::connector_of_identity;
/// Shared with the browser pilot (`pilots::browser`)
pub(crate) use mcp::{host_command, LineTransport};

/// A list in a connector's `config` (`config.args`, `config.network`)
pub fn catalog_config_list(decl: &ConnectorDecl, key: &str) -> Vec<String> {
    catalog::config_list(decl, key)
}
pub use tools::{parse_prefixed, prefixed_name};

/// How long a health check may take before the connector counts as unhealthy
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
/// Upper bound on one connector tool call
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Tool output beyond this is truncated before it reaches the model
const MAX_RESULT_CHARS: usize = 100_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatus {
    pub installed: bool,
    pub running: bool,
    pub healthy: bool,
    /// Why the connector isn't healthy (last health check or start)
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `mcp`
    pub kind: String,
    pub status: ConnectorStatus,
    /// The tools the connector exposes (unprefixed names)
    #[serde(default)]
    pub tools: Vec<ConnectorToolInfo>,
    /// Non-secret configuration from `.nandi/connectors.yaml`
    #[serde(default)]
    pub config: BTreeMap<String, Value>,
    /// Keyring entries the connector reads (never their values)
    #[serde(default)]
    pub secrets: Vec<ConnectorSecretInfo>,
    /// What's missing before the connector can start, if anything
    #[serde(default)]
    pub setup_hint: Option<String>,
    /// A published connector (`connector.md`): where it came from and what
    /// it can reach
    #[serde(default)]
    pub package: Option<PackageInfo>,
}

/// An installed connector package, as the Library and the consent dialog
/// show it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub version: String,
    /// The registry it was installed from; `None`: made here (Studio)
    pub registry: Option<String>,
    pub publisher: Option<String>,
    /// `npx <package>@<version>`, `uvx …`, or the server's URL
    pub runs: String,
    pub placement: domain::connector_package::Placement,
    /// Its own image, when it needs something unusual
    pub image: Option<String>,
    pub auth: domain::connector_package::AuthSpec,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub hosts: Vec<String>,
    pub tools: Vec<domain::connector_package::PackageTool>,
}

impl PackageInfo {
    pub fn of(
        package: &domain::connector_package::ConnectorPackage,
        decl: Option<&ConnectorDecl>,
    ) -> Self {
        let source = |key: &str| {
            decl.and_then(|d| d.config.get("package"))
                .and_then(|p| p.get(key))
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_string)
        };
        let runs = match &package.run {
            domain::connector_package::RunSpec::Url { url } => url.clone(),
            run => run
                .command_line()
                .map(|(command, args)| format!("{command} {}", args.join(" ")))
                .unwrap_or_default(),
        };
        Self {
            version: package.version.to_string(),
            registry: source("registry"),
            publisher: source("publisher"),
            runs,
            placement: package.placement,
            image: package.image.clone(),
            auth: package.auth.clone(),
            reads: package.reads.clone(),
            writes: package.writes.clone(),
            hosts: package.hosts.clone(),
            tools: package.tools.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorToolInfo {
    pub name: String,
    pub description: String,
    /// Calls need the user's approval
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSecretInfo {
    pub key: String,
    pub keyring_entry: String,
    pub is_set: bool,
    /// Start needs it set; `false` when the connector has another way in
    /// (GitHub: the GitHub CLI's sign-in)
    pub required: bool,
}

/// One connector was installed, started, stopped, signed in, or changed
/// health (`connector://changed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorChanged {
    pub connector_id: String,
    pub status: ConnectorStatus,
}

impl NandiEvent for ConnectorChanged {
    const NAME: &'static str = "connector://changed";
}

/// Connectors were added or removed; reload the list (`connectors://changed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorsChanged {}

impl NandiEvent for ConnectorsChanged {
    const NAME: &'static str = "connectors://changed";
}

// ---------------------------------------------------------------------------
// Connector trait
// ---------------------------------------------------------------------------

/// One tool a connector exposes, before prefixing.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object
    pub input_schema: Value,
    /// Calls need the user's approval
    pub sensitive: bool,
}

/// A running connector: an MCP server (in-process or external) that the
/// registry can list tools from, call, and health-check.
pub trait Connector: Send + Sync {
    fn list_tools(&self) -> Vec<ToolDef>;

    /// `Err` is a tool failure the model sees (`is_error`), not a crash.
    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>>;

    /// `call_tool` for one agent: `options` is its `connectors:` item in
    /// skill.md (the filesystem connector reads its `folders`). Most
    /// connectors are the same for every agent.
    fn call_tool_for<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
        options: Option<&'a serde_yaml::Value>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        let _ = options;
        self.call_tool(name, args)
    }

    fn health_check(&self) -> BoxFuture<'_, Result<(), String>>;

    /// Releases processes and sessions. Dropping also cleans up.
    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// What a turn gets from the connectors its agent declares.
#[derive(Debug, Default)]
pub struct AgentToolset {
    /// Prefixed tool specs, ready to append to the container's tools
    pub tools: Vec<ToolSpec>,
    /// Prefixed names of tools that need approval
    pub sensitive: HashSet<String>,
    /// For `agent://warning`: connectors left out and why
    pub warnings: Vec<String>,
    /// For `connector://changed`: connectors whose status changed
    pub changed: Vec<ConnectorChanged>,
}

struct Running {
    connector: Arc<dyn Connector>,
    /// Tool names (unprefixed) that need approval
    sensitive: HashSet<String>,
}

/// Installed connectors and their MCP sessions. `AppState::connectors()` is
/// `None` until Track D initializes it.
#[derive(Default)]
pub struct ConnectorRegistry {
    running: Mutex<HashMap<String, Arc<Running>>>,
    /// Stopped by the user this session: not started on first use
    stopped: Mutex<HashSet<String>>,
    /// Last health-check or start failure per connector
    errors: Mutex<HashMap<String, String>>,
    audit: audit::AuditSink,
    /// Egress identities for connector containers (`connector:<id>`)
    proxy_tokens: Option<crate::runtime::ProxyTokens>,
}

/// The registry key of `id` run on the host (`connectors.run_on_host`); a
/// connector in its container is keyed by its id
fn host_key(id: &str) -> String {
    format!("{id}#host")
}

impl std::fmt::Debug for ConnectorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorRegistry")
            .field("running", &lock(&self.running).keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl ConnectorRegistry {
    /// A registry that audits to `path` instead of `~/.nandi/local/logs`.
    pub fn with_audit_log(path: PathBuf) -> Self {
        Self {
            audit: audit::AuditSink::at(path),
            ..Self::default()
        }
    }

    /// Lets stdio connectors' containers through the egress proxy
    pub fn with_proxy_tokens(mut self, tokens: crate::runtime::ProxyTokens) -> Self {
        self.proxy_tokens = Some(tokens);
        self
    }

    /// The registry key of `id`'s instance for an agent that may run
    /// `on_host` connectors on the host, and whether that's the host one
    fn instance(id: &str, on_host: &[String]) -> (String, bool) {
        if on_host.iter().any(|c| c == id) {
            (host_key(id), true)
        } else {
            (id.to_string(), false)
        }
    }

    /// A sink writing where this registry's calls are audited
    pub fn audit_sink(&self) -> audit::AuditSink {
        self.audit.clone()
    }

    pub fn is_running(&self, id: &str) -> bool {
        lock(&self.running).contains_key(id)
    }

    fn running(&self, id: &str) -> Option<Arc<Running>> {
        lock(&self.running).get(id).cloned()
    }

    fn set_error(&self, id: &str, error: Option<String>) {
        let mut errors = lock(&self.errors);
        match error {
            Some(error) => errors.insert(id.to_string(), error),
            None => errors.remove(id),
        };
    }

    /// The connector's status, from what the registry last saw (no I/O).
    pub fn cached_status(&self, id: &str, installed: bool) -> ConnectorStatus {
        let running = self.is_running(id);
        let error = lock(&self.errors).get(id).cloned();
        ConnectorStatus {
            installed,
            running,
            healthy: running && error.is_none(),
            error,
        }
    }

    /// Adds an already-built connector (built-ins, tests).
    pub fn insert(
        &self,
        id: &str,
        connector: Arc<dyn Connector>,
        definition: &ConnectorDefinition,
    ) {
        let sensitive = definition
            .sensitive_tools
            .iter()
            .map(|t| t.to_string())
            .chain(
                connector
                    .list_tools()
                    .into_iter()
                    .filter(|t| t.sensitive)
                    .map(|t| t.name),
            )
            .collect();
        lock(&self.running).insert(
            id.to_string(),
            Arc::new(Running {
                connector,
                sensitive,
            }),
        );
        lock(&self.stopped).remove(id);
        self.set_error(id, None);
    }

    /// Starts `decl` if it isn't running. A connector the user stopped
    /// starts again here, since this is an explicit start.
    pub async fn start(&self, decl: &ConnectorDecl, secrets: &Secrets) -> Result<(), String> {
        self.start_placed(decl, secrets, &[]).await.map(|_| ())
    }

    /// Starts the instance of `decl` an agent with `on_host` uses, if it
    /// isn't running; returns its registry key
    async fn start_placed(
        &self,
        decl: &ConnectorDecl,
        secrets: &Secrets,
        on_host: &[String],
    ) -> Result<String, String> {
        let id = decl.id.as_str();
        let (key, on_this_host) = Self::instance(id, on_host);
        if self.is_running(&key) {
            lock(&self.stopped).remove(id);
            return Ok(key);
        }
        let placement = if on_this_host {
            mcp::Placement::Host
        } else {
            // A fresh egress identity for the new container
            mcp::Placement::Container {
                proxy_token: self
                    .proxy_tokens
                    .as_ref()
                    .map(|tokens| tokens.issue(&mcp::proxy_identity(id)))
                    .unwrap_or_default(),
            }
        };
        let definition = catalog::definition_for(decl);
        let built = catalog::build(decl, &definition, secrets, &placement).await;
        let connector = match built {
            Ok(connector) => connector,
            Err(e) => {
                self.set_error(&key, Some(e.clone()));
                return Err(e);
            }
        };
        if self.is_running(&key) {
            // Started concurrently; keep the first one
            connector.shutdown().await;
            return Ok(key);
        }
        self.insert(&key, connector, &definition);
        log::info!("Started connector '{id}' ({placement:?})");
        Ok(key)
    }

    /// Starts `decl` once in its container, lists its tools and stops it
    /// (Studio, making a connector from an MCP server)
    pub async fn discover_tools(
        &self,
        decl: &ConnectorDecl,
        secrets: &Secrets,
    ) -> Result<Vec<ToolDef>, String> {
        let placement = mcp::Placement::Container {
            proxy_token: self
                .proxy_tokens
                .as_ref()
                .map(|tokens| tokens.issue(&mcp::proxy_identity(decl.id.as_str())))
                .unwrap_or_default(),
        };
        let definition = catalog::definition_for(decl);
        let connector = catalog::build(decl, &definition, secrets, &placement).await?;
        let tools = connector.list_tools();
        connector.shutdown().await;
        Ok(tools)
    }

    /// Stops the connector and keeps it stopped until started again.
    pub async fn stop(&self, id: &str) {
        lock(&self.stopped).insert(id.to_string());
        for key in [id.to_string(), host_key(id)] {
            let running = lock(&self.running).remove(&key);
            self.set_error(&key, None);
            if let Some(running) = running {
                running.connector.shutdown().await;
                log::info!("Stopped connector '{key}'");
            }
        }
        if let Some(tokens) = &self.proxy_tokens {
            tokens.revoke(&mcp::proxy_identity(id));
        }
    }

    pub fn is_stopped(&self, id: &str) -> bool {
        lock(&self.stopped).contains(id)
    }

    /// Runs a health check (at most [`HEALTH_TIMEOUT`]) and records it.
    pub async fn check_health(&self, id: &str) -> Result<(), String> {
        let Some(running) = self.running(id) else {
            return Err(format!("Connector '{id}' isn't running"));
        };
        let result =
            match tokio::time::timeout(HEALTH_TIMEOUT, running.connector.health_check()).await {
                Ok(result) => result,
                Err(_) => Err(format!(
                    "Health check timed out after {}s",
                    HEALTH_TIMEOUT.as_secs()
                )),
            };
        self.set_error(id, result.as_ref().err().cloned());
        result
    }

    /// The tools a turn gets from the connectors `declared` by its agent.
    /// Installed connectors start on first use unless the user stopped
    /// them; unhealthy ones are left out with a warning. `taken` holds the
    /// container's tool names, which always win a collision.
    pub async fn agent_tools(
        &self,
        secrets: &Secrets,
        manifest: &ConnectorManifest,
        declared: &[String],
        taken: &HashSet<String>,
    ) -> AgentToolset {
        self.agent_tools_placed(secrets, manifest, declared, &[], taken)
            .await
    }

    /// [`Self::agent_tools`] for an agent whose policies let `on_host`
    /// connectors run on the host
    pub async fn agent_tools_placed(
        &self,
        secrets: &Secrets,
        manifest: &ConnectorManifest,
        declared: &[String],
        on_host: &[String],
        taken: &HashSet<String>,
    ) -> AgentToolset {
        let mut set = AgentToolset::default();
        let mut names: HashSet<String> = taken.clone();
        // Two IDs with the same prefix (`a.b`, `a_b`) can't both be routed:
        // the first declared one wins, as in `parse_prefixed`
        let mut prefixes: HashMap<String, &str> = HashMap::new();
        for id in declared {
            if let Some(first) = prefixes.get(&tools::sanitize(id)) {
                set.warnings.push(format!(
                    "Connector '{id}' has the same tool prefix as '{first}'; its tools are unavailable"
                ));
                continue;
            }
            prefixes.insert(tools::sanitize(id), id);
            // The person's accounts aren't installed: their tools come
            // from `services::email` and `services::workspace`
            if is_account_connector(id) {
                continue;
            }
            let Some(decl) = manifest.get(id) else {
                set.warnings.push(format!(
                    "Connector '{id}' isn't installed; its tools are unavailable"
                ));
                continue;
            };
            if self.is_stopped(id) {
                set.warnings.push(format!(
                    "Connector '{id}' is stopped; its tools are unavailable"
                ));
                continue;
            }
            let before = self.cached_status(id, true);
            let key = match self.start_placed(decl, secrets, on_host).await {
                Ok(key) => key,
                Err(e) => {
                    set.warnings
                        .push(format!("Connector '{id}' couldn't start: {e}"));
                    push_changed(&mut set, id, &before, self.cached_status(id, true));
                    continue;
                }
            };
            let health = self.check_health(&key).await;
            push_changed(&mut set, id, &before, self.cached_status(id, true));
            if let Err(e) = health {
                set.warnings.push(format!(
                    "Connector '{id}' is unhealthy, so its tools are left out: {e}"
                ));
                continue;
            }
            let Some(running) = self.running(&key) else {
                continue;
            };
            for tool in running.connector.list_tools() {
                let Some(name) = prefixed_name(id, &tool.name) else {
                    log::warn!("Connector '{id}' tool '{}' has an unusable name", tool.name);
                    continue;
                };
                if !names.insert(name.clone()) {
                    log::warn!("Connector tool '{name}' collides with another tool; skipped");
                    continue;
                }
                if tool.sensitive || running.sensitive.contains(&tool.name) {
                    set.sensitive.insert(name.clone());
                }
                set.tools.push(ToolSpec {
                    name,
                    description: tool.description,
                    input_schema: tool.input_schema,
                });
            }
        }
        set
    }

    /// Runs `name` if it's a tool of a connector `declared` by the agent.
    /// `None` means it isn't a connector tool (the caller tries the
    /// container). Every call is audited.
    pub async fn call_agent_tool(
        &self,
        agent_id: &str,
        declared: &[String],
        name: &str,
        args: &Value,
    ) -> Option<ToolOutcome> {
        self.call_agent_tool_with(agent_id, declared, &HashMap::new(), name, args)
            .await
    }

    /// `call_agent_tool` with the agent's per-connector `options`
    /// (`declared_options`)
    pub async fn call_agent_tool_with(
        &self,
        agent_id: &str,
        declared: &[String],
        options: &HashMap<String, serde_yaml::Value>,
        name: &str,
        args: &Value,
    ) -> Option<ToolOutcome> {
        self.call_agent_tool_placed(agent_id, declared, options, &[], name, args)
            .await
    }

    /// [`Self::call_agent_tool_with`] for an agent whose policies let
    /// `on_host` connectors run on the host
    pub async fn call_agent_tool_placed(
        &self,
        agent_id: &str,
        declared: &[String],
        options: &HashMap<String, serde_yaml::Value>,
        on_host: &[String],
        name: &str,
        args: &Value,
    ) -> Option<ToolOutcome> {
        let (connector_id, tool) = parse_prefixed(name, declared)?;
        let unavailable = |reason: String| ToolOutcome {
            result: format!("Tool '{name}' is not available: {reason}"),
            is_error: true,
        };
        let (key, _) = Self::instance(&connector_id, on_host);
        let Some(running) = self.running(&key) else {
            self.audit
                .record(agent_id, &connector_id, &tool, args, "unavailable", 0)
                .await;
            return Some(unavailable(format!(
                "connector '{connector_id}' isn't running"
            )));
        };
        if !running
            .connector
            .list_tools()
            .iter()
            .any(|t| t.name == tool)
        {
            return Some(unavailable(format!(
                "connector '{connector_id}' doesn't provide '{tool}'"
            )));
        }
        let started = Instant::now();
        let call = running
            .connector
            .call_tool_for(&tool, args, options.get(&connector_id));
        let result = tokio::time::timeout(CALL_TIMEOUT, call)
            .await
            .unwrap_or_else(|_| Err(format!("Tool '{name}' timed out")));
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let outcome = if result.is_ok() { "ok" } else { "error" };
        self.audit
            .record(agent_id, &connector_id, &tool, args, outcome, elapsed)
            .await;
        Some(match result {
            Ok(value) => ToolOutcome {
                result: truncate(render(value)),
                is_error: false,
            },
            Err(error) => ToolOutcome {
                result: truncate(error),
                is_error: true,
            },
        })
    }

    /// Stops everything (app shutdown).
    pub async fn shutdown_all(&self) {
        let all: Vec<_> = lock(&self.running).drain().collect();
        for (_, running) in all {
            running.connector.shutdown().await;
        }
    }
}

fn push_changed(
    set: &mut AgentToolset,
    id: &str,
    before: &ConnectorStatus,
    after: ConnectorStatus,
) {
    if before.running != after.running || before.healthy != after.healthy {
        set.changed.push(ConnectorChanged {
            connector_id: id.to_string(),
            status: after,
        });
    }
}

/// Strings go to the model as-is; anything else as JSON.
fn render(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

fn truncate(mut text: String) -> String {
    if let Some((index, _)) = text.char_indices().nth(MAX_RESULT_CHARS) {
        let omitted = text[index..].chars().count();
        text.truncate(index);
        text.push_str(&format!("\n[... {omitted} characters truncated]"));
    }
    text
}

/// A connector an agent uses, as its page shows and sets it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectorChoice {
    pub id: String,
    /// Filesystem connectors: the shared folders it gets
    #[serde(default)]
    pub folders: Vec<FolderChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderChoice {
    pub id: String,
    /// Narrower than the folder's own access; `None` keeps it
    #[serde(default)]
    pub access: Option<folders::Access>,
}

/// The connectors `agent_id`'s skill.md declares, with its folders
pub fn agent_connectors(
    state: &AppState,
    agent_id: &str,
) -> Result<Vec<AgentConnectorChoice>, String> {
    let agent = state.store().agent(agent_id)?;
    let options = declared_options(&agent);
    declared_connectors(&agent)
        .into_iter()
        .map(|id| {
            let folders = folders::grants(options.get(&id))?
                .into_iter()
                .map(|g| FolderChoice {
                    id: g.id,
                    access: g.access,
                })
                .collect();
            Ok(AgentConnectorChoice { id, folders })
        })
        .collect()
}

/// The agents a shared folder is given to, by folder ID
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderUsers {
    pub folder: String,
    /// Agent names, sorted
    pub agents: Vec<String>,
}

/// Which agents get each of `connector_id`'s shared folders, so its page
/// can say so and ask before removing one in use
pub fn folder_users(state: &AppState, connector_id: &str) -> Vec<FolderUsers> {
    let manifest = manifest(state);
    let Some(decl) = manifest.get(connector_id) else {
        return Vec::new();
    };
    let list = folders::shared_folders(decl).unwrap_or_default();
    let agents: Vec<domain::Agent> = state.store().read(|repo| repo.agents().cloned().collect());
    list.iter()
        .map(|folder| {
            let mut names: Vec<String> = agents
                .iter()
                .filter(|agent| {
                    declared_connectors(agent)
                        .iter()
                        .any(|id| id == connector_id)
                })
                .filter(|agent| {
                    let grants = folders::grants(declared_options(agent).get(connector_id))
                        .unwrap_or_default();
                    folders::granted(&list, &grants)
                        .iter()
                        .any(|f| f.id == folder.id)
                })
                .map(|agent| agent.name().to_string())
                .collect();
            names.sort();
            FolderUsers {
                folder: folder.id.clone(),
                agents: names,
            }
        })
        .collect()
}

/// `choices` as skill.md's `connectors:` (plain IDs where there are no
/// folders); `None` when there are no connectors
pub fn connectors_yaml(choices: &[AgentConnectorChoice]) -> Option<serde_yaml::Value> {
    if choices.is_empty() {
        return None;
    }
    let items = choices
        .iter()
        .map(|choice| match choice.folders.as_slice() {
            [] => serde_yaml::Value::String(choice.id.clone()),
            folders => {
                let folders = folders
                    .iter()
                    .map(|folder| match folder.access {
                        None => serde_yaml::Value::String(folder.id.clone()),
                        Some(access) => {
                            let mut map = serde_yaml::Mapping::new();
                            map.insert("id".into(), folder.id.clone().into());
                            map.insert("access".into(), access.as_str().into());
                            serde_yaml::Value::Mapping(map)
                        }
                    })
                    .collect();
                let mut map = serde_yaml::Mapping::new();
                map.insert("id".into(), choice.id.clone().into());
                map.insert("folders".into(), serde_yaml::Value::Sequence(folders));
                serde_yaml::Value::Mapping(map)
            }
        })
        .collect();
    Some(serde_yaml::Value::Sequence(items))
}

/// Replaces the connectors `agent_id` uses (its skill.md, through the
/// agent repository). Folders must be on their connector's list.
pub async fn set_agent_connectors(
    state: &AppState,
    agent_id: &str,
    choices: Vec<AgentConnectorChoice>,
) -> Result<(), String> {
    let manifest = manifest(state);
    for choice in &choices {
        parse_id(&choice.id)?;
        let Some(decl) = manifest.get(&choice.id) else {
            continue;
        };
        let chosen = &choice.folders;
        if chosen.is_empty() {
            continue;
        }
        if decl.kind != catalog::FILESYSTEM {
            return Err(format!("'{}' has no folders", choice.id));
        }
        let list = folders::shared_folders(decl)?;
        let grants: Vec<folders::FolderGrant> = chosen
            .iter()
            .map(|f| folders::FolderGrant {
                id: f.id.clone(),
                access: f.access,
            })
            .collect();
        if let Some(id) = folders::unknown(&list, &grants).first() {
            return Err(format!("There's no shared folder '{id}'"));
        }
    }
    let mut agent = state.store().agent(agent_id)?;
    agent.set_extra("connectors", connectors_yaml(&choices));
    let store = state.store().clone();
    tokio::task::spawn_blocking(move || {
        store.repos().agents.save(&agent).map_err(String::from)?;
        store.commit(EntityKind::Agent, agent.id().as_str())
    })
    .await
    .map_err(|e| format!("Saving the agent failed: {e}"))??;
    log::info!("Agent '{agent_id}' connectors updated");
    Ok(())
}

/// Each declared connector's item when it's a mapping (`{id: filesystem,
/// folders: [docs]}`), by connector ID
pub fn declared_options(agent: &domain::Agent) -> HashMap<String, serde_yaml::Value> {
    let Some(serde_yaml::Value::Sequence(items)) = agent.extra().get("connectors") else {
        return HashMap::new();
    };
    items
        .iter()
        .filter(|item| item.is_mapping())
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.trim().to_string();
            Some((id, item.clone()))
        })
        .collect()
}

/// `options` with every declared connector's `allowed_sites` set to the
/// agent's effective `network.allow` (web search reads it). Whatever
/// skill.md says there is replaced, so an agent can't widen its own sites.
pub fn with_allowed_sites(
    mut options: HashMap<String, serde_yaml::Value>,
    declared: &[String],
    allow: &[crate::services::egress::capabilities::HostRule],
) -> HashMap<String, serde_yaml::Value> {
    let sites = serde_yaml::to_value(allow).unwrap_or_default();
    for id in declared {
        let item = options
            .entry(id.clone())
            .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
        if let serde_yaml::Value::Mapping(map) = item {
            map.insert(web_search::ALLOWED_SITES.into(), sites.clone());
        }
    }
    options
}

/// The shared folders `agent` gets, with their access: from every
/// `filesystem` connector it declares (for the container's mounts)
pub fn agent_folders(
    agent: &domain::Agent,
    manifest: &ConnectorManifest,
) -> Vec<folders::SharedFolder> {
    let options = declared_options(agent);
    let mut granted: Vec<folders::SharedFolder> = Vec::new();
    for id in declared_connectors(agent) {
        let Some(decl) = manifest.get(&id).filter(|d| d.kind == catalog::FILESYSTEM) else {
            continue;
        };
        let Ok(list) = folders::shared_folders(decl) else {
            continue;
        };
        let grants = folders::grants(options.get(&id)).unwrap_or_default();
        for folder in folders::granted(&list, &grants) {
            if !granted.iter().any(|f| f.id == folder.id) {
                granted.push(folder);
            }
        }
    }
    granted
}

/// The connector IDs an agent's skill.md declares:
///
/// ```yaml
/// connectors: [filesystem, web_search]
/// ```
pub fn declared_connectors(agent: &domain::Agent) -> Vec<String> {
    let skill_md = agent.to_skill_md();
    let Ok((Some(frontmatter), _)) = domain::agent::split_frontmatter(&skill_md) else {
        return Vec::new();
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    if let Some(serde_yaml::Value::Sequence(items)) = value.get("connectors") {
        for item in items {
            // `- filesystem` or `- {id: filesystem}`
            let id = item
                .as_str()
                .or_else(|| item.get("id").and_then(serde_yaml::Value::as_str));
            if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids
}

// ---------------------------------------------------------------------------
// Services (one per command / route)
// ---------------------------------------------------------------------------

fn registry(state: &AppState) -> Result<&Arc<ConnectorRegistry>, String> {
    state.connectors().ok_or_else(|| {
        "Connectors aren't available yet: the connector registry isn't running".to_string()
    })
}

fn manifest(state: &AppState) -> ConnectorManifest {
    state.store().read(|repo| repo.connectors().clone())
}

fn installed(state: &AppState, id: &str) -> Result<ConnectorDecl, String> {
    manifest(state)
        .get(id)
        .cloned()
        .ok_or_else(|| format!("Connector '{id}' isn't installed"))
}

fn emit_status(state: &AppState, id: &str, status: ConnectorStatus) {
    events::emit(
        state.bus(),
        &ConnectorChanged {
            connector_id: id.to_string(),
            status,
        },
    );
}

/// The built-in catalog plus everything declared in `.nandi/connectors.yaml`.
pub async fn list_connectors(state: &AppState) -> Result<Vec<ConnectorInfo>, String> {
    let manifest = manifest(state);
    let mut infos = connector_infos(
        &manifest,
        state.connectors().map(Arc::as_ref),
        state.secrets(),
    );
    // Packages: what connector.md says, before the server has started
    for info in &mut infos {
        let Ok(Some(package)) = state.store().repos().connectors.load_package(&info.id) else {
            continue;
        };
        if info.tools.is_empty() {
            info.tools = package
                .tools
                .iter()
                .map(|t| ConnectorToolInfo {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    sensitive: t.access == domain::grant::GrantAccess::Write,
                })
                .collect();
        }
        info.package = Some(PackageInfo::of(&package, manifest.get(&info.id)));
    }
    Ok(infos)
}

/// Installs (or updates) a connector package: its `connector.md` in the
/// repo, its declaration in `connectors.yaml`, and its API key, if it has
/// one, in the keyring. Installing grants nothing: each agent asks first
pub async fn install_package(
    state: &AppState,
    package: &domain::connector_package::ConnectorPackage,
    source: Option<&domain::connector_package::PackageSource>,
    api_key: Option<String>,
) -> Result<(), String> {
    let id = package.id.as_str();
    if is_account_connector(id) || catalog::builtin(id).is_some() {
        return Err(format!("'{id}' is one of Nandi's own connectors"));
    }
    let decl = package.declaration(source);
    if let Some(key) = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        state.secrets().set(&package.key_entry(), &key)?;
    }
    let store = state.store();
    store
        .repos()
        .connectors
        .save_package(package)
        .map_err(|e| e.to_string())?;
    let mut connectors: Vec<ConnectorDecl> = manifest(state)
        .connectors()
        .iter()
        .filter(|c| c.id.as_str() != id)
        .cloned()
        .collect();
    connectors.push(decl);
    save_manifest(state, connectors)?;
    if let Some(registry) = state.connectors() {
        if registry.is_running(id) {
            registry.stop(id).await;
            lock(&registry.stopped).remove(id);
        }
    }
    events::emit(state.bus(), &ConnectorsChanged {});
    log::info!("Installed connector '{id}' {}", package.version);
    Ok(())
}

/// Removes an installed connector: its declaration, its package file, its
/// key and every grant on it
pub async fn remove_connector(state: &AppState, id: &str) -> Result<(), String> {
    let current = manifest(state);
    let decl = current
        .get(id)
        .cloned()
        .ok_or_else(|| format!("Connector '{id}' isn't installed"))?;
    if let Some(registry) = state.connectors() {
        registry.stop(id).await;
        lock(&registry.stopped).remove(id);
    }
    let connectors: Vec<ConnectorDecl> = current
        .connectors()
        .iter()
        .filter(|c| c.id.as_str() != id)
        .cloned()
        .collect();
    save_manifest(state, connectors)?;
    state
        .store()
        .repos()
        .connectors
        .delete_package(id)
        .map_err(|e| e.to_string())?;
    for entry in decl.secret_refs.values() {
        let _ = state.secrets().delete(entry.as_str());
    }
    for gone in state.grants().revoke_connector(id)? {
        crate::services::grants::audit(&state.connector_audit(), "removed", &gone, None, None)
            .await;
    }
    events::emit(state.bus(), &crate::services::grants::GrantsChanged {});
    events::emit(state.bus(), &ConnectorsChanged {});
    Ok(())
}

fn connector_infos(
    manifest: &ConnectorManifest,
    registry: Option<&ConnectorRegistry>,
    secrets: &Secrets,
) -> Vec<ConnectorInfo> {
    let mut infos: Vec<ConnectorInfo> = manifest
        .connectors()
        .iter()
        .map(|decl| connector_info(decl, true, registry, secrets))
        .collect();
    for definition in catalog::builtins() {
        if manifest.get(&definition.id).is_none() {
            let decl = catalog::declaration(&definition);
            infos.push(connector_info(&decl, false, registry, secrets));
        }
    }
    infos
}

fn connector_info(
    decl: &ConnectorDecl,
    installed: bool,
    registry: Option<&ConnectorRegistry>,
    secrets: &Secrets,
) -> ConnectorInfo {
    let id = decl.id.as_str();
    let definition = catalog::definition_for(decl);
    let status = registry.map_or_else(
        || ConnectorStatus {
            installed,
            ..ConnectorStatus::default()
        },
        |registry| registry.cached_status(id, installed),
    );
    let running = registry.and_then(|r| r.running(id));
    let tools = match &running {
        Some(running) => running
            .connector
            .list_tools()
            .into_iter()
            .map(|t| ConnectorToolInfo {
                sensitive: t.sensitive || running.sensitive.contains(&t.name),
                name: t.name,
                description: t.description,
            })
            .collect(),
        None => catalog::static_tools(decl.kind.as_str())
            .into_iter()
            .map(|t| ConnectorToolInfo {
                sensitive: t.sensitive || definition.sensitive_tools.contains(&t.name),
                name: t.name,
                description: t.description,
            })
            .collect(),
    };
    let secret_infos = catalog::secret_entries(decl)
        .into_iter()
        .map(|(key, entry)| ConnectorSecretInfo {
            is_set: secrets.get(&entry).ok().flatten().is_some(),
            required: catalog::secret_required(decl, &key),
            key,
            keyring_entry: entry,
        })
        .collect();
    ConnectorInfo {
        id: id.to_string(),
        name: definition.name.clone(),
        description: definition.description.clone(),
        kind: definition.kind.as_str().to_string(),
        status,
        tools,
        config: decl
            .config
            .iter()
            .filter_map(|(k, v)| serde_json::to_value(v).ok().map(|v| (k.clone(), v)))
            .collect(),
        secrets: secret_infos,
        setup_hint: catalog::setup_hint(decl, secrets),
        package: None,
    }
}

/// Declares a built-in connector in `.nandi/connectors.yaml`. Installing
/// doesn't start it, and no agent uses it until its skill.md says so.
pub async fn install_connector(state: &AppState, id: &str) -> Result<(), String> {
    let current = manifest(state);
    if current.get(id).is_some() {
        return Ok(());
    }
    let definition = catalog::builtin(id).ok_or_else(|| format!("Unknown connector '{id}'"))?;
    let mut connectors = current.connectors().to_vec();
    connectors.push(catalog::declaration(&definition));
    save_manifest(state, connectors)?;
    events::emit(state.bus(), &ConnectorsChanged {});
    emit_status(
        state,
        id,
        ConnectorStatus {
            installed: true,
            ..ConnectorStatus::default()
        },
    );
    log::info!("Installed connector '{id}'");
    Ok(())
}

/// Replaces an installed connector's (non-secret) config, e.g. the
/// filesystem connector's `path`. Credential-like keys are refused by the
/// domain. A running connector is stopped so the next start uses it.
pub async fn configure_connector(
    state: &AppState,
    id: &str,
    config: BTreeMap<String, serde_yaml::Value>,
) -> Result<(), String> {
    let current = manifest(state);
    let Some(existing) = current.get(id) else {
        return Err(format!("Connector '{id}' isn't installed"));
    };
    // Shared folders are stored with portable paths (`folders::portable`),
    // however the Setup form wrote them
    let mut config = config;
    if existing.kind == catalog::FILESYSTEM {
        let probe = ConnectorDecl {
            config: config.clone(),
            ..existing.clone()
        };
        config.extend(folders::folders_config(&folders::shared_folders(&probe)?));
    }
    let connectors = current
        .connectors()
        .iter()
        .cloned()
        .map(|mut decl| {
            if decl.id.as_str() == id {
                decl.config = config.clone();
            }
            decl
        })
        .collect::<Vec<_>>();
    // The folder list is checked here, not first at start
    if let Some(decl) = connectors
        .iter()
        .find(|d| d.id.as_str() == id && d.kind == catalog::FILESYSTEM)
    {
        folders::shared_folders(decl)?;
    }
    // S3 Backup's folders and bucket decide its triggers
    let backup = connectors
        .iter()
        .find(|d| d.id.as_str() == id && d.kind == catalog::S3_BACKUP)
        .map(s3_backup::BackupSettings::from_decl)
        .transpose()?;
    save_manifest(state, connectors)?;
    if let Some(settings) = backup {
        backup_triggers::sync(state, &settings).await?;
    }
    // The running instance has the old config; restart on next use
    if let Some(registry) = state.connectors() {
        if registry.is_running(id) {
            registry.stop(id).await;
            lock(&registry.stopped).remove(id);
        }
    }
    events::emit(state.bus(), &ConnectorsChanged {});
    Ok(())
}

/// The folder shared as `id` on the `filesystem` connector, if there is one
pub fn shared_folder(state: &AppState, id: &str) -> Option<folders::SharedFolder> {
    let decl = manifest(state).get(catalog::FILESYSTEM)?.clone();
    folders::shared_folders(&decl)
        .ok()?
        .into_iter()
        .find(|folder| folder.id == id)
}

/// Shares `folder` with agents: installs the `filesystem` connector when it
/// isn't, and replaces a folder with the same ID. The connector's other
/// config is kept.
pub async fn share_folder(state: &AppState, folder: folders::SharedFolder) -> Result<(), String> {
    install_connector(state, catalog::FILESYSTEM).await?;
    let decl = installed(state, catalog::FILESYSTEM)?;
    let mut list = folders::shared_folders(&decl)?;
    list.retain(|f| f.id != folder.id);
    list.push(folder);
    let mut config = decl.config.clone();
    config.extend(folders::folders_config(&list));
    configure_connector(state, catalog::FILESYSTEM, config).await
}

/// Brings S3 Backup's triggers in line with its saved Setup, e.g. once its
/// agent (Backup Keeper) is installed after the Setup was saved. Nothing to
/// do when S3 Backup isn't installed.
pub async fn resync_backup_triggers(state: &AppState) -> Result<(), String> {
    let Some(decl) = manifest(state)
        .connectors()
        .iter()
        .find(|d| d.kind == catalog::S3_BACKUP)
        .cloned()
    else {
        return Ok(());
    };
    let settings = s3_backup::BackupSettings::from_decl(&decl)?;
    backup_triggers::sync(state, &settings).await
}

/// A connector as Ask's chat declares it (`studio::save`)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectorDeclaration {
    pub id: String,
    /// `None` for a built-in, which is its own kind; `mcp` for a server
    pub kind: Option<String>,
    /// `None` keeps the current config (or a built-in's empty one)
    pub config: Option<BTreeMap<String, serde_yaml::Value>>,
    /// Name → keyring entry; the values are set in Library › Browse › Tools
    pub secret_refs: BTreeMap<String, String>,
}

/// Installs or replaces a connector's declaration: a built-in by ID, or
/// any kind the catalog runs (`mcp`) with its config. The domain refuses
/// credential-like config keys, so secrets stay in the keyring. A running
/// connector is stopped so its next start uses the new declaration.
pub async fn declare_connector(
    state: &AppState,
    declaration: ConnectorDeclaration,
) -> Result<ConnectorInfo, String> {
    let id = parse_id(&declaration.id)?;
    let current = manifest(state);
    let existing = current.get(id.as_str()).cloned();
    let builtin = catalog::builtin(id.as_str());
    let kind = match (&declaration.kind, &existing, &builtin) {
        (Some(kind), _, _) => kind.clone(),
        (None, Some(existing), _) => existing.kind.clone(),
        (None, None, Some(builtin)) => builtin.id.clone(),
        (None, None, None) => {
            return Err(format!(
                "'{id}' isn't a built-in connector: give its kind (`mcp`) and config"
            ))
        }
    };
    if catalog::builtin(&kind).is_none() && kind != catalog::MCP {
        return Err(format!(
            "Connector kind '{kind}' has no runtime: use a built-in or `mcp`"
        ));
    }
    let secret_refs = declaration
        .secret_refs
        .into_iter()
        .map(|(key, entry)| {
            domain::settings::SecretRef::parse(entry)
                .map(|entry| (key, entry))
                .map_err(|e| e.to_string())
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let decl = ConnectorDecl {
        id: id.clone(),
        kind,
        config: declaration
            .config
            .or_else(|| existing.as_ref().map(|e| e.config.clone()))
            .unwrap_or_default(),
        secret_refs: if secret_refs.is_empty() {
            existing.map(|e| e.secret_refs).unwrap_or_default()
        } else {
            secret_refs
        },
    };
    let mut connectors: Vec<ConnectorDecl> = current
        .connectors()
        .iter()
        .filter(|c| c.id != id)
        .cloned()
        .collect();
    connectors.push(decl);
    save_manifest(state, connectors)?;
    if let Some(registry) = state.connectors() {
        if registry.is_running(id.as_str()) {
            registry.stop(id.as_str()).await;
            lock(&registry.stopped).remove(id.as_str());
        }
    }
    events::emit(state.bus(), &ConnectorsChanged {});
    log::info!("Declared connector '{id}'");
    list_connectors(state)
        .await?
        .into_iter()
        .find(|info| info.id == id.as_str())
        .ok_or_else(|| format!("Connector '{id}' was saved but couldn't be read back"))
}

/// Settings as the UI sends them (JSON) to how `connectors.yaml` holds them
pub fn yaml_config(
    config: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, serde_yaml::Value>, String> {
    config
        .into_iter()
        .map(|(key, value)| {
            serde_yaml::to_value(value)
                .map(|value| (key, value))
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// Saves (or, with an empty `value`, removes) one of a connector's
/// secrets in the OS keyring, under the entry the connector reads. Write
/// only: values never come back, and nothing is written to the repo.
pub async fn set_connector_secret(
    state: &AppState,
    id: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let decl = match manifest(state).get(id) {
        Some(decl) => decl.clone(),
        None => catalog::declaration(
            &catalog::builtin(id).ok_or_else(|| format!("Unknown connector '{id}'"))?,
        ),
    };
    let entry = secret_entry(&decl, key)?;
    let value = value.trim();
    if value.is_empty() {
        state.secrets().delete(&entry)?;
        log::info!("Removed secret '{key}' of connector '{id}'");
    } else {
        state.secrets().set(&entry, value)?;
        log::info!("Saved secret '{key}' of connector '{id}'");
    }
    events::emit(state.bus(), &ConnectorsChanged {});
    Ok(())
}

/// The keyring entry behind a connector's secret `key`
fn secret_entry(decl: &ConnectorDecl, key: &str) -> Result<String, String> {
    catalog::secret_entries(decl)
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, entry)| entry)
        .ok_or_else(|| format!("Connector '{}' has no secret '{key}'", decl.id))
}

/// Domain validates, the repository writes the file, commit projects it.
fn save_manifest(state: &AppState, connectors: Vec<ConnectorDecl>) -> Result<(), String> {
    let manifest = ConnectorManifest::new(connectors).map_err(|e| e.to_string())?;
    let store = state.store();
    store
        .repos()
        .connectors
        .save(&manifest)
        .map_err(|e| e.to_string())?;
    store.commit(EntityKind::Connectors, "connectors")?;
    Ok(())
}

pub async fn start_connector(state: &AppState, id: &str) -> Result<(), String> {
    let registry = registry(state)?;
    let decl = installed(state, id)?;
    // Missing a secret, a folder, or a sign-in: say what, don't try
    if let Some(hint) = catalog::setup_hint(&decl, state.secrets()) {
        return Err(hint);
    }
    let result = registry.start(&decl, state.secrets()).await;
    if result.is_ok() {
        // A first health check so the UI shows a real status
        let _ = registry.check_health(id).await;
    }
    emit_status(state, id, registry.cached_status(id, true));
    result
}

pub async fn stop_connector(state: &AppState, id: &str) -> Result<(), String> {
    let registry = registry(state)?;
    installed(state, id)?;
    registry.stop(id).await;
    emit_status(state, id, registry.cached_status(id, true));
    Ok(())
}

/// Live status: a running connector is health-checked (at most 2s).
pub async fn connector_status(state: &AppState, id: &str) -> Result<ConnectorStatus, String> {
    let is_installed = manifest(state).get(id).is_some();
    if !is_installed && catalog::builtin(id).is_none() {
        return Err(format!("Unknown connector '{id}'"));
    }
    let Some(registry) = state.connectors() else {
        return Ok(ConnectorStatus {
            installed: is_installed,
            ..ConnectorStatus::default()
        });
    };
    let before = registry.cached_status(id, is_installed);
    if registry.is_running(id) {
        let _ = registry.check_health(id).await;
    }
    let after = registry.cached_status(id, is_installed);
    if before.healthy != after.healthy {
        emit_status(state, id, after.clone());
    }
    Ok(after)
}

/// A connector ID from a client, checked before it touches anything.
pub fn parse_id(id: &str) -> Result<ConnectorId, String> {
    ConnectorId::parse(id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
