//! Nandi core: business logic shared by every transport.
//!
//! Nothing in `services/` may use Tauri types (`AppHandle`, `State`, …).
//! Transports are thin adapters over these functions:
//! - `rpc/`: each command declared once, served over Tauri IPC and
//!   `POST /api/rpc/{name}`
//! - `desktop/`: the desktop-only Tauri commands
//! - `http/`: the web server (RPC, webhooks, `/ws`, the frontend)
//!
//! Every service takes `&AppState`.

pub mod a2a;
pub mod access;
pub mod agent_memory;
pub mod agent_overview;
pub mod agents;
pub mod ask;
pub mod attachments;
pub mod auth;
pub mod builtin_agents;
pub mod capability;
pub mod capability_index;
pub mod chat;
pub mod companion;
pub mod config;
pub mod connectors;
pub mod domain;
pub mod egress;
pub mod email;
pub mod events;
pub mod grants;
// Protocol 7: groups, their signed policies, my access levels
pub mod gateway;
pub mod groups;
pub mod harness;
pub mod help;
pub mod history;
pub mod identity;
pub mod images;
pub mod inbox;
pub mod intake;
pub mod knowme;
pub mod live;
pub mod llm_keys;
pub mod llm_trial;
pub mod machine;
pub mod marketplace;
pub mod mascot;
pub mod network;
pub mod notes;
pub mod orchestrator;
pub mod palette;
pub mod pilots;
pub mod playbooks;
// Policies applied to agents
pub mod policies;
pub mod preferences;
pub mod projection;
pub mod queue;
pub mod recovery;
pub mod relay;
pub mod remote;
pub mod repo_bootstrap;
pub mod repo_service;
pub mod repo_store;
pub mod repos;
pub mod run_analysis;
pub mod run_plan;
pub mod run_waiter;
pub mod runs;
pub mod runtimes;
// v1: Requests and their Steps, projected from the task tables (task 3)
pub mod requests;
pub mod secrets;
pub mod selection;
pub mod shared;
pub mod snapshots;
pub mod studio;
pub mod team;
pub mod tools;
pub mod triggers;
pub mod trust;
pub mod watcher;
pub mod work;
pub mod workflow_graph;
pub mod workflows;
pub mod workspace;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use crate::audit::AuditStore;
use crate::db::Db;
use crate::model::STUDIO_AGENT_ID;
use crate::runtime::ProxyTokens;
use crate::utils::nandi_config;

use a2a::{A2aDeps, Delegator};
use capability_index::CapabilityIndex;
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
    pilots: Arc<PilotRegistry>,
    /// Runtime MCP connectors
    connectors: Arc<ConnectorRegistry>,
    /// The GitHub sign-in in progress (`auth::github`)
    github_sign_in_flows: auth::oauth::OAuthFlows,
    /// Reading the user's mail for the Chief of Staff
    email: Arc<email::EmailService>,
    workspace: Arc<workspace::WorkspaceService>,
    audit_store: Arc<AuditStore>,
    git_service: Arc<GitService>,
    /// Local agent cards by capability. `None` without an instance ID.
    capability_index: Option<Arc<RwLock<CapabilityIndex>>>,
    /// `None` without an instance ID. Present even when no relay is
    /// configured: it then reports "not connected".
    relay_client: Option<Arc<dyn RelayClient>>,
    /// Cross-user delegation. `None` without an instance ID.
    a2a_delegator: Option<Arc<Delegator>>,
    team_manager: Arc<dyn TeamManager>,
    analysis: Arc<dyn RunAnalyzer>,
    palette: Arc<PaletteService>,
    remote: Arc<dyn remote::RemoteService>,
    selection: Arc<dyn selection::SelectionService>,
    network: Arc<network::NetworkService>,
    studio: Arc<studio::StudioService>,
    work: Arc<work::WorkService>,
    ask: Arc<ask::AskService>,
    repo_config: Arc<config::repo_config::RepoConfigService>,
    aws_auth: Arc<harness::aws_auth::AwsAuthService>,
    bedrock_models: Arc<harness::bedrock_models::BedrockModelService>,
    litellm_gateway: Arc<harness::gateway::GatewayService>,
    // Set once at startup by each service's `init` (`init_wave_final`)
    tasks: OnceLock<Arc<a2a::task::TaskService>>,
    clarifications: OnceLock<Arc<a2a::clarification::ClarificationService>>,
    gateway: OnceLock<Arc<gateway::GatewayService>>,
    capability_registries: Arc<capability::registry::CapabilityRegistryService>,
    harnesses: Arc<harness::HarnessService>,
    // Set once at startup by each service's `init` (`init_wave_a`)
    capabilities: OnceLock<Arc<capability::CapabilityService>>,
    tools: OnceLock<Arc<tools::ToolService>>,
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
    /// loads the repo files (bringing the SQLite projection up to date),
    /// fails the runs a previous process left running, and starts one
    /// orchestrator per agent (and Studio) and the file watcher.
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
        // The free trial signs its calls as this instance
        match network::local_instance_id(&db) {
            Ok(id) => llm_trial::init(id),
            Err(e) => log::warn!("No free trial without an instance ID: {e}"),
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
        // After the load, so the closing records carry agent names
        let store = state.store().clone();
        match state
            .bus()
            .fail_interrupted_runs(|agent_id| orchestrator::display_name(&store, agent_id))
        {
            Ok(0) => {}
            Ok(count) => log::warn!("Marked {count} interrupted runs as failed"),
            Err(e) => log::error!("Failed to recover interrupted runs: {e}"),
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
        network::start(&state);
        state.init_wave_final();
        state.init_wave_a();
        state.init_wave_b();

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
        let analysis: Arc<dyn RunAnalyzer> = Arc::new(run_analysis::Analyzer::with_llm(
            db.clone(),
            store.clone(),
            config.secrets.clone(),
        ));
        let remote: Arc<dyn remote::RemoteService> = Arc::new(remote::GitRemoteService::new(
            bus.clone(),
            config.secrets.clone(),
            store.clone(),
            nandi_dir.clone(),
        ));
        let selection: Arc<dyn selection::SelectionService> =
            Arc::new(selection::DbSelection::new(db.clone(), Some(bus.clone())));
        let network = Arc::new(network::NetworkService::new(&db));
        let repo_config = Arc::new(config::repo_config::RepoConfigService::new(
            store.clone(),
            db.clone(),
            bus.clone(),
        ));
        let aws_auth = Arc::new(harness::aws_auth::AwsAuthService::for_app(
            db.clone(),
            bus.clone(),
            &config.app_data_dir,
        ));
        let bedrock_models = Arc::new(harness::bedrock_models::BedrockModelService::new(
            db.clone(),
            bus.clone(),
        ));
        let litellm_gateway = Arc::new(harness::gateway::GatewayService::new(
            bus.clone(),
            config.app_data_dir.join("local").join("gateway"),
        ));
        let capability_registries = Arc::new(capability::registry::CapabilityRegistryService::new(
            db.clone(),
            bus.clone(),
            config.secrets.clone(),
            nandi_dir.join("local").join("cache"),
            Some(store.repos().layout().capabilities_dir()),
        ));
        let harnesses = Arc::new(harness::HarnessService::for_app(
            db.clone(),
            bus.clone(),
            &config.app_data_dir,
        ));
        let ask_llm: Arc<dyn ask::AskLlm> =
            Arc::new(ask::ModelAskLlm::new(config.secrets.clone(), store.clone()));
        // The network services need this install's instance ID. Without
        // one (the database can't give it), they stay off.
        let instance_id = network::local_instance_id(&db)
            .inspect_err(|e| log::error!("Network services disabled: no instance ID ({e})"))
            .ok();
        let capability_index = instance_id.as_ref().map(|id| {
            let relay_url = relay::configured_url_in(&nandi_dir).unwrap_or_default();
            Arc::new(RwLock::new(CapabilityIndex::new(
                db.clone(),
                id.clone(),
                relay_url,
            )))
        });
        let relay_client = instance_id.as_ref().map(|id| {
            relay::client(
                &db,
                &bus,
                &config.secrets,
                &config.built_in_relay,
                id.clone(),
            )
        });
        let team_manager: Arc<dyn TeamManager> = Arc::new(team::TeamService::new(
            db.clone(),
            bus.clone(),
            store.clone(),
            config.secrets.clone(),
            team::TeamConfig::in_repo(&store),
        ));
        let a2a_delegator = instance_id.as_ref().map(|id| {
            Delegator::new(A2aDeps::new(
                db.clone(),
                bus.clone(),
                orchestrators.clone(),
                broadcast.clone(),
                &config.secrets,
                id.clone(),
                capability_index.clone(),
                relay_client.clone(),
            ))
        });
        Self(Arc::new_cyclic(|inner| Inner {
            // Services that call back into the app hold it weakly
            studio: Arc::new(studio::StudioService::new(WeakAppState(inner.clone()))),
            work: Arc::new(work::WorkService::new(WeakAppState(inner.clone()))),
            ask: Arc::new(ask::AskService::new(WeakAppState(inner.clone()), ask_llm)),
            analysis,
            palette: Arc::default(),
            remote,
            selection,
            network,
            repo_config,
            aws_auth,
            bedrock_models,
            litellm_gateway,
            capability_registries,
            harnesses,
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
            pilots,
            connectors,
            github_sign_in_flows: Default::default(),
            email: Arc::new(email::EmailService::default()),
            workspace: Arc::new(workspace::WorkspaceService::default()),
            audit_store: Arc::new(AuditStore::new()),
            git_service,
            capability_index,
            relay_client,
            a2a_delegator,
            team_manager,
            tasks: OnceLock::new(),
            clarifications: OnceLock::new(),
            gateway: OnceLock::new(),
            capabilities: OnceLock::new(),
            tools: OnceLock::new(),
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
        network::start(&state);
        state.init_wave_final();
        state.init_wave_a();
        state.init_wave_b();
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

    /// Host-side pilots
    pub fn pilots(&self) -> &Arc<PilotRegistry> {
        &self.0.pilots
    }

    /// Runtime connectors
    pub fn connectors(&self) -> &Arc<ConnectorRegistry> {
        &self.0.connectors
    }

    /// Connector grants: saved in the repo, or on this computer only
    pub fn grants(&self) -> grants::GrantStore {
        grants::GrantStore::new(&self.0.store, &self.0.nandi_dir)
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
        self.0.capability_index.as_ref()
    }

    /// The relay this build comes with
    pub fn built_in_relay(&self) -> &relay::BuiltInRelay {
        &self.0.built_in_relay
    }

    /// Connection to the relay
    pub fn relay_client(&self) -> Option<&Arc<dyn RelayClient>> {
        self.0.relay_client.as_ref()
    }

    /// Cross-user delegation
    pub fn a2a_delegator(&self) -> Option<&Arc<Delegator>> {
        self.0.a2a_delegator.as_ref()
    }

    /// Teams and their private registries
    pub fn team_manager(&self) -> &Arc<dyn TeamManager> {
        &self.0.team_manager
    }

    /// "Ask about this run"
    pub fn analysis(&self) -> &Arc<dyn RunAnalyzer> {
        &self.0.analysis
    }

    /// The command palette
    pub fn palette(&self) -> &Arc<PaletteService> {
        &self.0.palette
    }

    /// The optional git remote for the repo
    pub fn remote(&self) -> &Arc<dyn remote::RemoteService> {
        &self.0.remote
    }

    /// The task lifecycle
    #[allow(dead_code)]
    pub fn tasks(&self) -> Option<&Arc<a2a::task::TaskService>> {
        self.0.tasks.get()
    }

    /// Clarification and escalation
    #[allow(dead_code)]
    pub fn clarifications(&self) -> Option<&Arc<a2a::clarification::ClarificationService>> {
        self.0.clarifications.get()
    }

    /// The gateway agent
    #[allow(dead_code)]
    pub fn gateway(&self) -> Option<&Arc<gateway::GatewayService>> {
        self.0.gateway.get()
    }

    /// Capability selection and circuit breakers
    pub fn selection(&self) -> &Arc<dyn selection::SelectionService> {
        &self.0.selection
    }

    /// The network directory
    pub fn network(&self) -> &Arc<network::NetworkService> {
        &self.0.network
    }

    /// Starts the delegation services, in dependency order (tasks before
    /// the gateway, which dispatches child tasks). Must not block.
    fn init_wave_final(&self) {
        if let Some(service) = a2a::task::init(self) {
            let _ = self.0.tasks.set(service);
        }
        if let Some(service) = a2a::clarification::init(self) {
            let _ = self.0.clarifications.set(service);
        }
        if let Some(service) = gateway::init(self) {
            let _ = self.0.gateway.set(service);
        }
        network::watch_directory(self);
        triggers::handover::init(self);
        team::roster::watch(self);
    }

    /// The capability service
    #[allow(dead_code)]
    pub fn capabilities(&self) -> Option<&Arc<capability::CapabilityService>> {
        self.0.capabilities.get()
    }

    /// The unified tool catalog
    #[allow(dead_code)]
    pub fn tools(&self) -> Option<&Arc<tools::ToolService>> {
        self.0.tools.get()
    }

    /// Subscribed capability registries
    pub fn capability_registries(&self) -> &Arc<capability::registry::CapabilityRegistryService> {
        &self.0.capability_registries
    }

    /// Starts the tool and capability services, after `init_wave_final`, in dependency
    /// order (tools and registries before capabilities, which resolve
    /// required tools and merge registry definitions). Must not block.
    fn init_wave_a(&self) {
        capability::ensure_capabilities_dir(self);
        if let Some(service) = tools::init(self) {
            let _ = self.0.tools.set(service);
        }
        capability::registry::start(self);
        if let Some(service) = capability::init(self) {
            let _ = self.0.capabilities.set(service);
        }
        capability::aggregation::start(self);
    }

    /// Harness sessions, adapters and the installed harness CLIs
    pub fn harnesses(&self) -> &Arc<harness::HarnessService> {
        &self.0.harnesses
    }

    /// AWS auth for Bedrock
    pub fn aws_auth(&self) -> &Arc<harness::aws_auth::AwsAuthService> {
        &self.0.aws_auth
    }

    /// The Bedrock model catalog
    pub fn bedrock_models(&self) -> &Arc<harness::bedrock_models::BedrockModelService> {
        &self.0.bedrock_models
    }

    /// The LiteLLM gateway
    pub fn litellm_gateway(&self) -> &Arc<harness::gateway::GatewayService> {
        &self.0.litellm_gateway
    }

    /// Starts the harnesses and the browser pilot, after `init_wave_a`. Must
    /// not block.
    fn init_wave_b(&self) {
        harness::start(self);
        pilots::browser::init(pilots::browser::Host {
            proxy_tokens: self.proxy_tokens().clone(),
            secrets: self.secrets().clone(),
            store: self.store().clone(),
        });
        self.egress()
            .credentials()
            .set_sign_in(harness::aws_auth::agent_sign_in(self.aws_auth().clone()));
        // Harness runs reach the services through the orchestrators' deps
        let _ = self.orchestrators().deps().harness.set(self.downgrade());
    }

    /// The ask box
    pub fn ask(&self) -> &Arc<ask::AskService> {
        &self.0.ask
    }

    /// Studio's artifact editor
    pub fn studio(&self) -> &Arc<studio::StudioService> {
        &self.0.studio
    }

    /// Work in flight
    pub fn work(&self) -> &Arc<work::WorkService> {
        &self.0.work
    }

    /// `.nandi/llm.yaml`
    pub fn repo_config(&self) -> &Arc<config::repo_config::RepoConfigService> {
        &self.0.repo_config
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

    /// Stops every orchestrator. Queued runs and runs waiting on an
    /// approval carry on at the next launch; a run still `running` is
    /// failed then (`EventBus::fail_interrupted_runs`).
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
        let connectors = self.connectors().clone();
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
