//! Nandi core: business logic shared by every transport.
//!
//! Nothing in `services/` may use Tauri types (`AppHandle`, `State`, …).
//! Transports are thin adapters over these functions:
//! - `commands/` — Tauri IPC for the desktop webview
//! - `http/` — REST + WebSocket for browsers
//!
//! Every service takes `&AppState`.

// Not every API has a caller yet
#[allow(dead_code)]
pub mod a2a;
pub mod access;
// Wave final (Track C)
#[allow(dead_code)]
pub mod agent_memory;
pub mod agent_overview;
pub mod agents;
// Wave CD: the ask box (see docs/developer/interaction-layer.md)
#[allow(dead_code)]
pub mod ask;
pub mod attachments;
// Wave final (Track A): local identity
#[allow(dead_code)]
pub mod auth;
pub mod builtin_agents;
// Wave A: capabilities as entities
#[allow(dead_code)]
pub mod capability;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod capability_index;
pub mod chat;
// Wave 2B redesign (Track D)
#[allow(dead_code)]
pub mod companion;
// Wave EF (Track E): repo-level configuration
#[allow(dead_code)]
pub mod config;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod connectors;
// The domain model's API is complete by design (entities, value objects,
// behavior); not every accessor has a caller yet.
#[allow(dead_code)]
pub mod domain;
pub mod egress;
pub mod email;
pub mod events;
pub mod grants;
// Protocol 7: groups, their signed policies, my access levels
pub mod groups;
// Wave final (Track D): the gateway agent (NANDI.md)
#[allow(dead_code)]
pub mod gateway;
// Wave B: harness adapters
#[allow(dead_code)]
pub mod harness;
pub mod history;
pub mod images;
pub mod inbox;
pub mod intake;
pub mod live;
pub mod llm_keys;
pub mod llm_trial;
pub mod marketplace;
pub mod mascot;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod network;
pub mod notes;
pub mod orchestrator;
// Wave 2B redesign (Track E)
#[allow(dead_code)]
pub mod palette;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod pilots;
pub mod playbooks;
// Policies applied to agents (enforced-policies wave 3)
pub mod policies;
pub mod preferences;
pub mod projection;
pub mod queue;
pub mod recovery;
// Wave 2B redesign wiring (frozen)
pub mod redesign;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod relay;
// Wave final (Track A): the optional git remote
#[allow(dead_code)]
pub mod remote;
pub mod repo_bootstrap;
pub mod repo_service;
pub mod repo_store;
pub mod repos;
// Wave 2B redesign (Track A)
#[allow(dead_code)]
pub mod run_analysis;
pub mod run_plan;
pub mod runs;
pub mod runtimes;
// v1: Requests and their Steps, projected from the task tables (task 3)
pub mod requests;
pub mod secrets;
// Wave final (Track D): capability selection and circuit breakers
#[allow(dead_code)]
pub mod selection;
pub mod shared;
pub mod snapshots;
// Wave CD: Studio's artifact editor
#[allow(dead_code)]
pub mod studio;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod team;
// Wave A (Track C): the unified tool catalog
#[allow(dead_code)]
pub mod tools;
pub mod triggers;
// Not every API has a caller yet
#[allow(dead_code)]
pub mod trust;
pub mod watcher;
// Wave CD: work in flight
#[allow(dead_code)]
pub mod work;
// Wave 2B redesign (Track B)
#[allow(dead_code)]
pub mod workflow_graph;
pub mod workflows;
pub mod workspace;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use crate::audit::AuditStore;
use crate::db::{self, Db};
use crate::model::STUDIO_AGENT_ID;
use crate::runtime::ProxyTokens;
use crate::utils::nandi_config;

use a2a::A2ADelegator;
use capability_index::CapabilityIndex;
use companion::CompanionService;
use connectors::ConnectorRegistry;
use egress::EgressState;
use events::{BroadcastSink, EventBus, EventSink, FanoutSink};
use orchestrator::{Orchestrators, RunDeps};
use palette::PaletteService;
use pilots::PilotRegistry;
use relay::RelayClient;
use repo_service::GitService;
use repo_store::RepoStore;
use repos::{RepoLayout, Repos};
use run_analysis::RunAnalyzer;
use runtimes::AgentRuntimes;
use secrets::Secrets;
use team::TeamManager;
use triggers::TriggerEngine;
use workflows::WorkflowEngine;

/// The app identifier from `tauri.conf.json`. Used as the keychain service
/// and the app data folder name, so desktop and headless share both.
pub const APP_IDENTIFIER: &str = "com.nandi.app";

/// How a transport wants the core started.
pub struct Config {
    /// Where preferences and recovery files live
    pub app_data_dir: PathBuf,
    /// Runtime the long-lived tasks are spawned on
    pub runtime: tokio::runtime::Handle,
    /// Extra event sinks (e.g. the Tauri webview). The WebSocket broadcast
    /// sink is always present.
    pub sinks: Vec<Arc<dyn EventSink>>,
    pub secrets: Secrets,
    /// The relay this build comes with (`BuiltInRelay::of_this_build`)
    pub built_in_relay: relay::BuiltInRelay,
}

struct Inner {
    db: Db,
    built_in_relay: relay::BuiltInRelay,
    bus: EventBus,
    orchestrators: Orchestrators,
    secrets: Secrets,
    proxy_tokens: ProxyTokens,
    runtimes: AgentRuntimes,
    broadcast: BroadcastSink,
    app_data_dir: PathBuf,
    nandi_dir: PathBuf,
    runtime: tokio::runtime::Handle,
    http_port: OnceLock<u16>,
    egress: Arc<EgressState>,
    workflows: Arc<WorkflowEngine>,
    triggers: Arc<TriggerEngine>,
    store: RepoStore,
    /// Applies edits to the repo files as they happen
    watcher: std::sync::Mutex<Option<watcher::RepoWatcher>>,
    /// Who changed which file, and the notes and work folder watcher
    live: live::LiveService,
    marketplace: marketplace::Marketplace,
    /// Host-side OS-control MCP servers
    pilots: Option<Arc<PilotRegistry>>,
    /// Runtime MCP connectors
    connectors: Option<Arc<ConnectorRegistry>>,
    /// The GitHub sign-in in progress (`auth::github`)
    github_sign_in_flows: auth::oauth::OAuthFlows,
    /// Reading the user's mail for the Chief of Staff
    email: Arc<email::EmailService>,
    workspace: Arc<workspace::WorkspaceService>,
    audit_store: Arc<AuditStore>,
    git_service: Arc<GitService>,
    // Set once by `network::init`
    capability_index: OnceLock<Arc<RwLock<CapabilityIndex>>>,
    relay_client: OnceLock<Arc<dyn RelayClient>>,
    a2a_delegator: OnceLock<Arc<dyn A2ADelegator>>,
    team_manager: OnceLock<Arc<dyn TeamManager>>,
    // Wave 2B redesign: set once by each track's `init` (`redesign::init`)
    analysis: OnceLock<Arc<dyn RunAnalyzer>>,
    companion: OnceLock<Arc<CompanionService>>,
    palette: OnceLock<Arc<PaletteService>>,
    // Wave final: set once by each track's `init` (`init_wave_final`)
    remote: OnceLock<Arc<dyn remote::RemoteService>>,
    tasks: OnceLock<Arc<a2a::task::TaskService>>,
    clarifications: OnceLock<Arc<a2a::clarification::ClarificationService>>,
    gateway: OnceLock<Arc<gateway::GatewayService>>,
    selection: OnceLock<Arc<dyn selection::SelectionService>>,
    network: OnceLock<Arc<network::NetworkService>>,
    team_roster: OnceLock<Arc<team::roster::RosterService>>,
    trust: OnceLock<Arc<team::trust::TrustService>>,
    // Wave A: set once by each track's `init` (`init_wave_a`)
    capabilities: OnceLock<Arc<capability::CapabilityService>>,
    tools: OnceLock<Arc<tools::ToolService>>,
    capability_registries: OnceLock<Arc<capability::registry::CapabilityRegistryService>>,
    // Wave B: set once by each track's `init` (`init_wave_b`)
    harnesses: OnceLock<Arc<harness::HarnessService>>,
    harness_detector: OnceLock<Arc<harness::detection::HarnessDetector>>,
    aws_auth: OnceLock<Arc<harness::aws_auth::AwsAuthService>>,
    bedrock_models: OnceLock<Arc<harness::bedrock_models::BedrockModelService>>,
    litellm_gateway: OnceLock<Arc<harness::gateway::GatewayService>>,
    // Wave CD: set once by `init_wave_cd`
    ask: OnceLock<Arc<ask::AskService>>,
    studio: OnceLock<Arc<studio::StudioService>>,
    work: OnceLock<Arc<work::WorkService>>,
    // Wave EF: set once by `init_wave_ef` (network and trust are wave
    // final's slots above)
    repo_config: OnceLock<Arc<config::repo_config::RepoConfigService>>,
}

/// Shared handle to everything the services need. Cheap to clone.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

/// An `AppState` that doesn't keep it alive, for handlers stored inside the
/// services it owns (e.g. the gateway's executor hook)
#[derive(Clone)]
pub struct WeakAppState(std::sync::Weak<Inner>);

impl WeakAppState {
    pub fn upgrade(&self) -> Option<AppState> {
        self.0.upgrade().map(AppState)
    }
}

impl AppState {
    pub fn downgrade(&self) -> WeakAppState {
        WeakAppState(Arc::downgrade(&self.0))
    }

    /// Sets up `~/.nandi/repo` if needed, opens `~/.nandi/local/state.db`,
    /// recovers interrupted runs, loads the repo files (bringing the SQLite
    /// projection up to date), and starts one orchestrator per agent (and
    /// Studio) and the file watcher.
    pub fn start(config: Config) -> Result<Self, String> {
        let layout = repo_bootstrap::install_layout()?;
        if let Err(e) = repo_bootstrap::bootstrap(&layout) {
            log::error!("Failed to set up {}: {e}", layout.root.display());
        }
        let db = Db::open(&nandi_config::state_db_path()?)?;
        // Before anything reads it: the instance ID outlives the database
        match crate::utils::nandi_config::local_dir() {
            Ok(local) => network::keep_instance_id(&db, &local),
            Err(e) => log::warn!("Instance ID not kept on disk: {e}"),
        }
        match db
            .conn()
            .and_then(|conn| db::fail_interrupted_runs(&conn).map_err(|e| e.to_string()))
        {
            Ok(runs) if !runs.is_empty() => {
                log::warn!("Marked {} interrupted runs as failed", runs.len());
            }
            Ok(_) => {}
            Err(e) => log::error!("Failed to recover interrupted runs: {e}"),
        }

        let state = Self::build(db, config, layout);
        match state.store().load() {
            Ok(stats) => log::info!(
                "Loaded the repo: {} agents, {} workflows re-projected",
                stats.agents,
                stats.workflows
            ),
            Err(e) => log::error!("Failed to project the repo files: {e}"),
        }
        let agent_ids: Vec<String> = state
            .store()
            .read(|repo| repo.agents().map(|a| a.id().to_string()).collect());
        for id in &agent_ids {
            state.orchestrators().ensure(id);
        }
        log::info!("Started {} agent orchestrators", agent_ids.len());
        state.orchestrators().ensure(STUDIO_AGENT_ID);
        state.watch_repo();
        images::init(state.bus());
        network::init(&state);
        redesign::init(&state);
        state.init_wave_final();
        state.init_wave_a();
        state.init_wave_b();
        state.init_wave_cd();
        state.init_wave_ef();

        let workflows = state.workflows().clone();
        state.runtime().spawn(async move {
            if let Err(e) = workflows.sync_from_disk().await {
                log::error!("Failed to load workflows: {e}");
            }
            if let Err(e) = workflows.resume_interrupted().await {
                log::error!("Failed to resume workflow runs: {e}");
            }
            // Ask steps waiting on a colleague: answers that came while
            // the app was closed, then every few seconds
            workflows.watch_asks();
        });
        // My notes exists and is shared from the first launch, so Chief of
        // Staff can keep notes before anyone opens the My notes page
        let weak = state.downgrade();
        state.runtime().spawn(async move {
            let Some(state) = weak.upgrade() else {
                return;
            };
            match notes::folder(&state).await {
                Ok((folder, _)) => notes::watch(&state, &folder),
                Err(e) => log::warn!("The notes folder isn't ready: {e}"),
            }
        });
        // Local asks in progress pick up where they were (task 14)
        let weak = state.downgrade();
        state.runtime().spawn(async move {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if let Err(e) = requests::local::resume_all(&state).await {
                log::error!("Failed to resume requests: {e}");
            }
        });
        // Screenshots in runs' files are kept 30 days
        run_plan::screenshots::start_cleanup(state.runtime());
        // The email check, every few minutes while a mailbox is connected
        email::start_scheduler(&state);
        workspace::start_scheduler(&state);
        let triggers = state.triggers().clone();
        state.runtime().spawn(async move {
            if let Err(e) = triggers.start().await {
                log::error!("Failed to start triggers: {e}");
            }
        });
        // Approvals nobody decided in time expire, so runs don't stall
        let weak = state.downgrade();
        state.runtime().spawn(async move {
            let mut tick = tokio::time::interval(chat::APPROVAL_SWEEP);
            loop {
                tick.tick().await;
                let Some(state) = weak.upgrade() else {
                    break;
                };
                if let Err(e) = chat::expire_overdue_approvals(&state).await {
                    log::warn!("Expiring overdue approvals failed: {e}");
                }
            }
        });
        Ok(state)
    }

    fn build(db: Db, config: Config, layout: RepoLayout) -> Self {
        // `~/.nandi/local/cache` (the repo is `~/.nandi/repo`)
        let cache_dir = layout.root.parent().map_or_else(
            || layout.root.join(".cache"),
            |nandi| nandi.join("local").join("cache"),
        );
        let marketplace = marketplace::Marketplace::new(cache_dir);
        let git_service = Arc::new(GitService::new(layout.root.clone()));
        let store = RepoStore::sqlite(Repos::new(layout), db.clone());
        let broadcast = BroadcastSink::new();
        let mut sinks = config.sinks;
        sinks.push(Arc::new(broadcast.clone()));
        let bus = EventBus::new(db.clone(), Arc::new(FanoutSink::new(sinks)));
        store.subscribe(Arc::new(bus.clone()));
        let feed = live::ChangeFeed::new(bus.clone());
        live::StoreChanges::subscribe(&feed, &store);
        let proxy_tokens = ProxyTokens::default();
        let egress = Arc::new(EgressState::new(config.secrets.clone()));
        let runtimes =
            AgentRuntimes::new(proxy_tokens.clone(), egress.clone()).with_events(bus.clone());
        let pilots = Arc::new(PilotRegistry::new());
        let connectors =
            Arc::new(ConnectorRegistry::default().with_proxy_tokens(proxy_tokens.clone()));
        let orchestrators = Orchestrators::new(
            RunDeps {
                bus: bus.clone(),
                secrets: config.secrets.clone(),
                runtimes: runtimes.clone(),
                store: store.clone(),
                connectors: Some(connectors.clone()),
                harness: Default::default(),
            },
            config.runtime.clone(),
        );
        let workflows = Arc::new(WorkflowEngine::new(
            db.clone(),
            bus.clone(),
            orchestrators.clone(),
            broadcast.clone(),
            config.runtime.clone(),
            store.clone(),
        ));
        let triggers = Arc::new(TriggerEngine::new(
            db.clone(),
            bus.clone(),
            orchestrators.clone(),
            workflows.clone(),
            broadcast.clone(),
            config.runtime.clone(),
            store.clone(),
            config.secrets.clone(),
        ));
        store.subscribe(Arc::new(Reactor {
            orchestrators: orchestrators.clone(),
            triggers: Arc::downgrade(&triggers),
            runtime: config.runtime.clone(),
        }));
        // Tests keep `config.json` in their temporary folder
        #[cfg(test)]
        let nandi_dir = config.app_data_dir.clone();
        #[cfg(not(test))]
        let nandi_dir = nandi_config::nandi_dir().unwrap_or_else(|_| config.app_data_dir.clone());
        Self(Arc::new(Inner {
            nandi_dir,
            db,
            built_in_relay: config.built_in_relay,
            bus,
            orchestrators,
            secrets: config.secrets,
            proxy_tokens,
            runtimes,
            broadcast,
            app_data_dir: config.app_data_dir,
            runtime: config.runtime,
            http_port: OnceLock::new(),
            egress,
            workflows,
            triggers,
            store,
            watcher: std::sync::Mutex::new(None),
            live: live::LiveService::new(feed),
            marketplace,
            pilots: Some(pilots),
            connectors: Some(connectors),
            github_sign_in_flows: Default::default(),
            email: Arc::new(email::EmailService::default()),
            workspace: Arc::new(workspace::WorkspaceService::default()),
            audit_store: Arc::new(AuditStore::new()),
            git_service,
            capability_index: OnceLock::new(),
            relay_client: OnceLock::new(),
            a2a_delegator: OnceLock::new(),
            team_manager: OnceLock::new(),
            analysis: OnceLock::new(),
            companion: OnceLock::new(),
            palette: OnceLock::new(),
            remote: OnceLock::new(),
            tasks: OnceLock::new(),
            clarifications: OnceLock::new(),
            gateway: OnceLock::new(),
            selection: OnceLock::new(),
            network: OnceLock::new(),
            team_roster: OnceLock::new(),
            trust: OnceLock::new(),
            capabilities: OnceLock::new(),
            tools: OnceLock::new(),
            capability_registries: OnceLock::new(),
            harnesses: OnceLock::new(),
            harness_detector: OnceLock::new(),
            aws_auth: OnceLock::new(),
            bedrock_models: OnceLock::new(),
            litellm_gateway: OnceLock::new(),
            ask: OnceLock::new(),
            studio: OnceLock::new(),
            work: OnceLock::new(),
            repo_config: OnceLock::new(),
        }))
    }

    /// An in-memory core for tests: no orchestrators are pre-spawned, and
    /// no built-in relay.
    #[cfg(test)]
    pub fn for_tests(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self::for_tests_with_relay(sinks, relay::BuiltInRelay::default())
    }

    /// [`for_tests`](Self::for_tests) as a build that comes with `built_in_relay`
    #[cfg(test)]
    pub fn for_tests_with_relay(
        sinks: Vec<Arc<dyn EventSink>>,
        built_in_relay: relay::BuiltInRelay,
    ) -> Self {
        let app_data_dir =
            std::env::temp_dir().join(format!("nandi-test-{}", uuid::Uuid::new_v4()));
        // The repo layout inside the temporary folder: repo/.nandi/...
        let layout = RepoLayout::new(app_data_dir.join("repo"), app_data_dir.join("trash"));
        let state = Self::build(
            Db::in_memory(),
            Config {
                app_data_dir,
                runtime: tokio::runtime::Handle::current(),
                sinks,
                secrets: Secrets::in_memory(),
                built_in_relay,
            },
            layout,
        );
        network::init(&state);
        redesign::init(&state);
        state.init_wave_final();
        state.init_wave_a();
        state.init_wave_b();
        state.init_wave_cd();
        state.init_wave_ef();
        state
    }

    pub fn db(&self) -> &Db {
        &self.0.db
    }

    pub fn bus(&self) -> &EventBus {
        &self.0.bus
    }

    pub fn orchestrators(&self) -> &Orchestrators {
        &self.0.orchestrators
    }

    pub fn secrets(&self) -> &Secrets {
        &self.0.secrets
    }

    /// The GitHub sign-in in progress
    pub fn email(&self) -> &Arc<email::EmailService> {
        &self.0.email
    }

    /// Calendars and files (`services::workspace`)
    pub fn workspace(&self) -> &Arc<workspace::WorkspaceService> {
        &self.0.workspace
    }

    pub fn github_sign_in_flows(&self) -> &auth::oauth::OAuthFlows {
        &self.0.github_sign_in_flows
    }

    /// Per-agent tokens for the container-facing LLM proxy
    pub fn proxy_tokens(&self) -> &ProxyTokens {
        &self.0.proxy_tokens
    }

    /// AWS credentials and STS sessions for the egress proxy
    pub fn egress(&self) -> &Arc<EgressState> {
        &self.0.egress
    }

    /// Agents' runtime containers and MCP sessions
    pub fn runtimes(&self) -> &AgentRuntimes {
        &self.0.runtimes
    }

    pub fn workflows(&self) -> &Arc<WorkflowEngine> {
        &self.0.workflows
    }

    pub fn triggers(&self) -> &Arc<TriggerEngine> {
        &self.0.triggers
    }

    /// The gold copy in memory, kept in step with the files and projection
    pub fn marketplace(&self) -> &marketplace::Marketplace {
        &self.0.marketplace
    }

    pub fn store(&self) -> &RepoStore {
        &self.0.store
    }

    /// Host-side pilots (always set)
    pub fn pilots(&self) -> Option<&Arc<PilotRegistry>> {
        self.0.pilots.as_ref()
    }

    /// Runtime connectors (always set)
    pub fn connectors(&self) -> Option<&Arc<ConnectorRegistry>> {
        self.0.connectors.as_ref()
    }

    /// Connector grants: saved in the repo, or on this computer only
    pub fn grants(&self) -> grants::GrantStore {
        grants::GrantStore::new(&self.0.store, &self.0.nandi_dir)
    }

    /// Where grant decisions are audited (the connector log)
    pub fn connector_audit(&self) -> connectors::audit::AuditSink {
        self.connectors()
            .map(|registry| registry.audit_sink())
            .unwrap_or_default()
    }

    /// Readers for the audit logs
    pub fn audit_store(&self) -> &Arc<AuditStore> {
        &self.0.audit_store
    }

    /// Status, diff, and commit for `~/.nandi/repo`
    pub fn git_service(&self) -> &Arc<GitService> {
        &self.0.git_service
    }

    /// Local agent cards by capability
    pub fn capability_index(&self) -> Option<&Arc<RwLock<CapabilityIndex>>> {
        self.0.capability_index.get()
    }

    /// The relay this build comes with
    pub fn built_in_relay(&self) -> &relay::BuiltInRelay {
        &self.0.built_in_relay
    }

    /// Connection to the relay
    pub fn relay_client(&self) -> Option<&Arc<dyn RelayClient>> {
        self.0.relay_client.get()
    }

    /// Cross-user delegation
    pub fn a2a_delegator(&self) -> Option<&Arc<dyn A2ADelegator>> {
        self.0.a2a_delegator.get()
    }

    /// Teams and their private registries
    pub fn team_manager(&self) -> Option<&Arc<dyn TeamManager>> {
        self.0.team_manager.get()
    }

    // Set once, by `network::init`
    fn set_capability_index(&self, index: Arc<RwLock<CapabilityIndex>>) {
        let _ = self.0.capability_index.set(index);
    }

    fn set_relay_client(&self, client: Arc<dyn RelayClient>) {
        let _ = self.0.relay_client.set(client);
    }

    fn set_a2a_delegator(&self, delegator: Arc<dyn A2ADelegator>) {
        let _ = self.0.a2a_delegator.set(delegator);
    }

    fn set_team_manager(&self, manager: Arc<dyn TeamManager>) {
        let _ = self.0.team_manager.set(manager);
    }

    /// "Ask about this run" (wave 2B redesign, Track A)
    #[allow(dead_code)]
    pub fn analysis(&self) -> Option<&Arc<dyn RunAnalyzer>> {
        self.0.analysis.get()
    }

    /// The floating companion (wave 2B redesign, Track D)
    #[allow(dead_code)]
    pub fn companion(&self) -> Option<&Arc<CompanionService>> {
        self.0.companion.get()
    }

    /// The command palette (wave 2B redesign, Track E)
    #[allow(dead_code)]
    pub fn palette(&self) -> Option<&Arc<PaletteService>> {
        self.0.palette.get()
    }

    // Set once, by `redesign::init`
    fn set_analysis(&self, analyzer: Arc<dyn RunAnalyzer>) {
        let _ = self.0.analysis.set(analyzer);
    }

    fn set_companion(&self, companion: Arc<CompanionService>) {
        let _ = self.0.companion.set(companion);
    }

    fn set_palette(&self, palette: Arc<PaletteService>) {
        let _ = self.0.palette.set(palette);
    }

    /// The git remote (wave final, Track A); `None` until it lands
    #[allow(dead_code)]
    pub fn remote(&self) -> Option<&Arc<dyn remote::RemoteService>> {
        self.0.remote.get()
    }

    /// The task lifecycle (wave final, Track B)
    #[allow(dead_code)]
    pub fn tasks(&self) -> Option<&Arc<a2a::task::TaskService>> {
        self.0.tasks.get()
    }

    /// Clarification and escalation (wave final, Track C)
    #[allow(dead_code)]
    pub fn clarifications(&self) -> Option<&Arc<a2a::clarification::ClarificationService>> {
        self.0.clarifications.get()
    }

    /// The gateway agent (wave final, Track D)
    #[allow(dead_code)]
    pub fn gateway(&self) -> Option<&Arc<gateway::GatewayService>> {
        self.0.gateway.get()
    }

    /// Capability selection (wave final, Track D)
    #[allow(dead_code)]
    pub fn selection(&self) -> Option<&Arc<dyn selection::SelectionService>> {
        self.0.selection.get()
    }

    /// The network directory (wave final, Track E)
    #[allow(dead_code)]
    pub fn network(&self) -> Option<&Arc<network::NetworkService>> {
        self.0.network.get()
    }

    /// Team rosters and member profiles (wave final, Track E)
    #[allow(dead_code)]
    pub fn team_roster(&self) -> Option<&Arc<team::roster::RosterService>> {
        self.0.team_roster.get()
    }

    /// Trust toward other members (wave final, Track E)
    #[allow(dead_code)]
    pub fn trust(&self) -> Option<&Arc<team::trust::TrustService>> {
        self.0.trust.get()
    }

    /// Wave final: each track's `init`, after `redesign::init`, in
    /// dependency order (selection before tasks, which fail over through
    /// it; tasks before the gateway, which dispatches child tasks). Must
    /// not block.
    fn init_wave_final(&self) {
        if let Some(service) = remote::init(self) {
            let _ = self.0.remote.set(service);
        }
        if let Some(service) = team::trust::init(self) {
            let _ = self.0.trust.set(service);
        }
        if let Some(service) = selection::init(self) {
            let _ = self.0.selection.set(service);
        }
        if let Some(service) = a2a::task::init(self) {
            let _ = self.0.tasks.set(service);
        }
        if let Some(service) = a2a::clarification::init(self) {
            let _ = self.0.clarifications.set(service);
        }
        if let Some(service) = gateway::init(self) {
            let _ = self.0.gateway.set(service);
        }
        if let Some(service) = network::init_directory(self) {
            let _ = self.0.network.set(service);
        }
        triggers::handover::init(self);
        if let Some(service) = team::roster::init(self) {
            let _ = self.0.team_roster.set(service);
        }
    }

    /// The capability service (wave A, Track A)
    #[allow(dead_code)]
    pub fn capabilities(&self) -> Option<&Arc<capability::CapabilityService>> {
        self.0.capabilities.get()
    }

    /// The unified tool catalog (wave A, Track C)
    #[allow(dead_code)]
    pub fn tools(&self) -> Option<&Arc<tools::ToolService>> {
        self.0.tools.get()
    }

    /// Subscribed capability registries (wave A, Track B)
    #[allow(dead_code)]
    pub fn capability_registries(
        &self,
    ) -> Option<&Arc<capability::registry::CapabilityRegistryService>> {
        self.0.capability_registries.get()
    }

    /// Wave A: each track's `init`, after `init_wave_final`, in dependency
    /// order (tools and registries before capabilities, which resolve
    /// required tools and merge registry definitions). Must not block.
    fn init_wave_a(&self) {
        capability::ensure_capabilities_dir(self);
        if let Some(service) = tools::init(self) {
            let _ = self.0.tools.set(service);
        }
        if let Some(service) = capability::registry::init(self) {
            let _ = self.0.capability_registries.set(service);
        }
        if let Some(service) = capability::init(self) {
            let _ = self.0.capabilities.set(service);
        }
        capability::aggregation::start(self);
    }

    /// Harness sessions and adapters (wave B, Tracks A and E)
    #[allow(dead_code)]
    pub fn harnesses(&self) -> Option<&Arc<harness::HarnessService>> {
        self.0.harnesses.get()
    }

    /// Installed harness CLIs (wave B, Track A)
    #[allow(dead_code)]
    pub fn harness_detector(&self) -> Option<&Arc<harness::detection::HarnessDetector>> {
        self.0.harness_detector.get()
    }

    /// AWS auth for Bedrock (wave B, Track F)
    #[allow(dead_code)]
    pub fn aws_auth(&self) -> Option<&Arc<harness::aws_auth::AwsAuthService>> {
        self.0.aws_auth.get()
    }

    /// The Bedrock model catalog (wave B, Track F)
    #[allow(dead_code)]
    pub fn bedrock_models(&self) -> Option<&Arc<harness::bedrock_models::BedrockModelService>> {
        self.0.bedrock_models.get()
    }

    /// The LiteLLM gateway (wave B, Track C)
    #[allow(dead_code)]
    pub fn litellm_gateway(&self) -> Option<&Arc<harness::gateway::GatewayService>> {
        self.0.litellm_gateway.get()
    }

    /// Wave B: each track's `init`, after `init_wave_a`. Must not block.
    fn init_wave_b(&self) {
        if let Some((service, detector)) = harness::init(self) {
            let _ = self.0.harness_detector.set(detector);
            let _ = self.0.harnesses.set(service);
        }
        if let Some(gateway) = harness::gateway::init(self) {
            let _ = self.0.litellm_gateway.set(gateway);
        }
        pilots::browser::init(pilots::browser::Host {
            proxy_tokens: self.proxy_tokens().clone(),
            secrets: self.secrets().clone(),
            store: self.store().clone(),
        });
        if let Some(aws) = harness::aws_auth::init(self) {
            self.egress()
                .credentials()
                .set_sign_in(harness::aws_auth::agent_sign_in(aws.clone()));
            let _ = self.0.aws_auth.set(aws);
        }
        if let Some(models) = harness::bedrock_models::init(self) {
            let _ = self.0.bedrock_models.set(models);
        }
        // Harness runs reach the services through the orchestrators' deps
        let _ = self.orchestrators().deps().harness.set(self.downgrade());
    }

    /// The ask box (wave CD, Tracks A, B, H)
    #[allow(dead_code)]
    pub fn ask(&self) -> Option<&Arc<ask::AskService>> {
        self.0.ask.get()
    }

    /// Studio's artifact editor (wave CD, Track C)
    #[allow(dead_code)]
    pub fn studio(&self) -> Option<&Arc<studio::StudioService>> {
        self.0.studio.get()
    }

    /// Work in flight (wave CD, Track F)
    #[allow(dead_code)]
    pub fn work(&self) -> Option<&Arc<work::WorkService>> {
        self.0.work.get()
    }

    /// Wave CD: after `init_wave_b`. Must not block.
    fn init_wave_cd(&self) {
        if let Some(service) = studio::init(self) {
            let _ = self.0.studio.set(service);
        }
        if let Some(service) = work::init(self) {
            let _ = self.0.work.set(service);
        }
        if let Some(service) = ask::init(self) {
            let _ = self.0.ask.set(service);
        }
    }

    /// `.nandi/llm.yaml` (wave EF, Track E)
    #[allow(dead_code)]
    pub fn repo_config(&self) -> Option<&Arc<config::repo_config::RepoConfigService>> {
        self.0.repo_config.get()
    }

    /// Wave EF: after `init_wave_cd`. Must not block. The network
    /// directory and trust service are wave final's (`init_wave_final`).
    fn init_wave_ef(&self) {
        if let Some(service) = config::repo_config::init(self) {
            let _ = self.0.repo_config.set(service);
        }
    }

    /// Events for WebSocket subscribers
    pub fn broadcast(&self) -> &BroadcastSink {
        &self.0.broadcast
    }

    /// Runtime for background tasks (servers, orchestrators)
    pub fn runtime(&self) -> &tokio::runtime::Handle {
        &self.0.runtime
    }

    pub fn app_data_dir(&self) -> &std::path::Path {
        &self.0.app_data_dir
    }

    /// `~/.nandi` (or `NANDI_HOME`), where `config.json` lives
    pub fn nandi_dir(&self) -> &std::path::Path {
        &self.0.nandi_dir
    }

    /// The web server's bound port, once it's listening.
    pub fn http_port(&self) -> Option<u16> {
        self.0.http_port.get().copied()
    }

    pub fn set_http_port(&self, port: u16) {
        let _ = self.0.http_port.set(port);
    }

    /// Starts (or restarts) the repo file watcher. Without it, the app still
    /// works; listing then re-scans the files.
    pub fn watch_repo(&self) {
        let mut slot = self.0.watcher.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        match watcher::spawn(
            self.store().clone(),
            self.bus().clone(),
            self.live().feed().clone(),
            self.runtime(),
        ) {
            Ok(watcher) => *slot = Some(watcher),
            Err(e) => {
                self.store().set_watching(false);
                log::warn!("Not watching the repo files: {e}");
            }
        }
    }

    /// Recent file changes and the notes and work folder watcher
    pub fn live(&self) -> &live::LiveService {
        &self.0.live
    }

    /// Stops every orchestrator. Persisted runs resume on next launch.
    pub fn shutdown(&self) {
        self.live().stop();
        if let Some(watcher) = self
            .0
            .watcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            drop(watcher);
            self.store().set_watching(false);
        }
        self.triggers().stop();
        self.orchestrators().shutdown_all();
        if let Some(connectors) = self.connectors().cloned() {
            let stop = async move { connectors.shutdown_all().await };
            match tokio::runtime::Handle::try_current() {
                // Inside a runtime (tests, the HTTP server): don't block it
                Ok(handle) => {
                    handle.spawn(stop);
                }
                // App exit runs on the main thread
                Err(_) => tauri::async_runtime::block_on(stop),
            }
        }
    }
}

/// Keeps the running parts in step with gold-copy changes, whether from a
/// command or an edited file: new agents get an orchestrator, and changed
/// triggers re-arm their sources.
struct Reactor {
    orchestrators: Orchestrators,
    triggers: std::sync::Weak<TriggerEngine>,
    runtime: tokio::runtime::Handle,
}

impl repo_store::EventPublisher for Reactor {
    fn changed(&self, event: &domain::DomainEvent) {
        use domain::{ChangeAction, EntityKind};
        match (event.kind, event.action) {
            (EntityKind::Agent, ChangeAction::Created | ChangeAction::Updated) => {
                self.orchestrators.ensure(&event.id);
            }
            (EntityKind::Trigger, _) => {
                if let Some(engine) = self.triggers.upgrade() {
                    self.runtime.spawn(async move {
                        if let Err(e) = engine.reload().await {
                            log::error!("Failed to reload triggers: {e}");
                        }
                    });
                }
            }
            _ => {}
        }
    }

    fn invalid(&self, _problem: &domain::Problem, _changed_by: domain::ChangedBy) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;

    #[test]
    fn identifier_matches_tauri_config() {
        // Keychain entries and the app data folder are keyed by it
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_IDENTIFIER);
    }

    #[tokio::test]
    async fn events_reach_every_sink() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let mut subscriber = state.broadcast().subscribe();

        let conversation = chat::create_studio_conversation(&state).await.unwrap();
        chat::send_studio_message(&state, conversation.id, "hi".into(), vec![], None)
            .await
            .unwrap();

        assert!(recorder.names().contains(&"agent://message"));
        let first = subscriber.recv().await.unwrap();
        assert_eq!(first.event, "agent://run");
        assert_eq!(first.payload["agentId"], STUDIO_AGENT_ID);
        state.shutdown();
    }
}
