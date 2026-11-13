//! Native or harness, per run.
//!
//! An agent's `runtime:` picks who runs its turns. `native` (or none) is
//! Nandi's own loop, unchanged. A harness runtime starts the CLI in the
//! harness container, sends the run's input as one ACP prompt, and writes
//! the streamed updates into the run as native steps would be
//! (`harness::event_translation`), so chat, history, and evidence work
//! the same.
//!
//! - `plan` is pure: it turns the agent and what the machine has into a
//!   decision (tested without Docker)
//! - `resolve_runtime` gathers those inputs: the image, the detected CLI,
//!   the default harness, credentials, and the gateway
//! - `execute_harness_run` drives one run through an adapter
//!
//! A harness that can't start fails the run, unless the agent opts into
//! `runtime_config.fallback: native`: silently switching to Nandi's own
//! model would also switch the provider the user chose.

use std::path::PathBuf;

use crate::db::{self, role};
use crate::model::RunStatus;
use crate::runtime::{DEFAULT_CPU, DEFAULT_MEMORY_MB};
use crate::services::config::repo_config::{self, HarnessDefaults};
use crate::services::domain::{Agent, AgentRuntimeKind};
use crate::services::events::{EventBus, RunContext};
use crate::services::harness::acp::{AcpSessionUpdate, AcpUsage};
use crate::services::harness::adapter::{
    GatewayConfig, HarnessAdapter, HarnessConfig, HarnessCredentials, Placement, UserMessage,
};
use crate::services::harness::event_translation::{TurnOutcome, TurnTranslator};
use crate::services::harness::{
    evidence, gateway, HarnessErrorEvent, HarnessService, HarnessSessionStatus,
    HarnessTurnFinished, SessionMeta, HARNESS_IMAGE,
};
use crate::services::images::tools::VolumeMount;
use crate::services::orchestrator::middleware::TurnPolicy;
use crate::services::{events, AppState};

/// Nandi's tool names → the ACP tool kinds harnesses report
const TOOL_KINDS: &[(&str, &[&str])] = &[
    ("shell", &["execute"]),
    ("read_file", &["read", "search"]),
    ("list_dir", &["read", "search"]),
    ("write_file", &["edit", "delete", "move"]),
    ("fetch", &["fetch"]),
];

/// The harness sandbox for a run: ready, with the tool volumes the harness
/// mounts (Claude Code's, `images::tools`), or why it isn't
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sandbox {
    Ready(Vec<VolumeMount>),
    Unavailable(String),
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::Ready(Vec::new())
    }
}

/// What a run needs to know about the machine to pick a runtime
#[derive(Debug, Clone, Default)]
pub struct RouterInputs {
    /// The harness image and this harness's tool volumes
    pub sandbox: Sandbox,
    /// The default harness's model, when it's this harness
    pub default_model: Option<String>,
    /// The agent's workdir on the host
    pub workdir: PathBuf,
    pub credentials: HarnessCredentials,
    /// The running gateway (endpoint, key), when the agent wants one
    pub gateway: Option<(GatewayConfig, String)>,
    /// `.nandi/llm.yaml`'s choices the environment left open;
    /// the agent's own `runtime_config` still wins
    pub repo: HarnessDefaults,
    /// The CLI is signed in only on the host: its own settings give a
    /// container nothing to authenticate with
    pub host_only_sign_in: bool,
    /// The agent's shared folders, mounted into its container
    pub folders: Vec<crate::runtime::FolderMount>,
    /// What its prompt says about them (`folders::prompt`)
    pub folder_prompt: String,
    /// The agent's egress proxy token (`ProxyTokens::issue`)
    pub proxy_token: String,
    /// Its tools once its policies applied (`RepoStore::skill`); `None`:
    /// skill.md's own
    pub tools: Option<ToolGrant>,
    /// Its policies' `models.allow` lists (`SkillConfig::model_rules`)
    pub model_rules: Vec<(Vec<String>, String)>,
}

/// A CLI's model in Nandi's `provider:model` names, for `models.allow`:
/// `claude:` for Claude Code, `gpt:` for Codex, `gemini:` for Gemini CLI;
/// an unset model is `<provider>:default`
pub fn cli_model_ref(harness_id: &str, model: Option<&str>) -> String {
    let provider = match harness_id {
        "claude-code" => "claude",
        "codex" => "gpt",
        "gemini-cli" => "gemini",
        other => other,
    };
    format!("{provider}:{}", model.unwrap_or("default"))
}

/// An agent's tools by Nandi's names: allowed (`None`: all), denied by a
/// policy, and the ones a person approves first
#[derive(Debug, Clone, Default)]
pub struct ToolGrant {
    pub allowed: Option<Vec<String>>,
    pub denied: Vec<String>,
    pub approval: Vec<String>,
}

impl ToolGrant {
    fn of(agent: &Agent) -> Self {
        let tools = agent.capabilities().tools();
        Self {
            allowed: tools.allowed().map(<[String]>::to_vec),
            denied: Vec::new(),
            approval: tools.approval_list(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HarnessPlan {
    /// `claude-code`, `codex`, `gemini-cli`, `custom:<id>`
    pub harness_id: String,
    pub config: HarnessConfig,
    pub provider: Option<String>,
    /// The model the CLI is asked for (the gateway alias through LiteLLM)
    pub model: Option<String>,
    /// Hosts the CLI's own model calls go to, allowed through the egress
    /// proxy while the run lasts (`egress::allow_session_hosts`)
    pub egress_hosts: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum RuntimeDecision {
    Native,
    Harness(Box<HarnessPlan>),
    /// The harness can't run here, and the agent doesn't fall back
    Unavailable(String),
}

/// `.nandi/llm.yaml`'s provider kind for a team gateway
const REPO_GATEWAY: &str = "litellm";

/// The provider a harness session uses: the agent's choice, Bedrock when it
/// goes through the gateway, else what the CLI is set up for
pub fn provider(agent: &Agent, detected: Option<&str>) -> Option<String> {
    let config = agent.runtime_config();
    config
        .and_then(|c| c.provider.clone())
        .or_else(|| {
            config
                .and_then(|c| c.gateway.as_ref())
                .map(|_| "bedrock".to_string())
        })
        .or_else(|| detected.map(str::to_string))
}

/// ACP tool kinds for `allowed-tools` (`None` → empty: everything)
pub fn allowed_tool_kinds(allowed: Option<&[String]>) -> Vec<String> {
    let Some(allowed) = allowed else {
        return Vec::new();
    };
    let mut kinds = tool_kinds(allowed);
    // Read-only by default when every listed tool is unknown to harnesses
    if kinds.is_empty() {
        kinds.push("read".to_string());
    }
    kinds
}

/// ACP tool kinds a person approves first: `requires_approval` (`shell`
/// and `write_file` unless skill.md says otherwise, plus a policy's `ask`),
/// as for native runs
pub fn approval_tool_kinds(approval: &[String]) -> Vec<String> {
    tool_kinds(approval)
}

/// Nandi's tool names as ACP kinds; other names stay, matched by title to
/// deny or ask first (a title never allows, `acp::PermissionPolicy`)
fn tool_kinds(tools: &[String]) -> Vec<String> {
    let mut kinds: Vec<String> = tools
        .iter()
        .flat_map(|tool| {
            TOOL_KINDS
                .iter()
                .find(|(name, _)| name == tool)
                .map(|(_, kinds)| kinds.iter().map(|k| k.to_string()).collect())
                // Anything else is matched by name against the tool's title
                .unwrap_or_else(|| vec![tool.clone()])
        })
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

/// `nandi-harness-<instance tag>-<agent id>`: per agent and per Nandi
pub fn container_name(agent_id: &str) -> String {
    format!(
        "nandi-harness-{}-{agent_id}",
        crate::utils::nandi_config::instance_tag()
    )
}

/// The decision for `agent` given `inputs` (pure)
pub fn plan(agent: &Agent, inputs: RouterInputs) -> RuntimeDecision {
    let runtime = agent.runtime();
    if runtime.is_native() {
        return RuntimeDecision::Native;
    }
    let config = agent.runtime_config().cloned().unwrap_or_default();
    let fallback = |reason: String| {
        if config.falls_back_to_native() {
            log::info!("Agent '{}' runs natively: {reason}", agent.id().as_str());
            RuntimeDecision::Native
        } else {
            RuntimeDecision::Unavailable(reason)
        }
    };
    let harness_id = runtime.as_str().into_owned();
    let tools = match &inputs.sandbox {
        Sandbox::Ready(tools) => tools.clone(),
        Sandbox::Unavailable(reason) => return fallback(reason.clone()),
    };
    let placement = Placement::Container {
        image: HARNESS_IMAGE.to_string(),
        name: container_name(agent.id().as_str()),
        host_workdir: inputs.workdir.clone(),
        // The native runtime's limits when skill.md sets none
        memory_mb: Some(agent.container().memory_mb().unwrap_or(DEFAULT_MEMORY_MB)),
        cpu: Some(agent.container().cpu().unwrap_or(DEFAULT_CPU)),
        // Internal: its way out is the egress relay (`HTTPS_PROXY` below)
        network: Some(crate::services::egress::relay::AGENT_NETWORK.to_string()),
        folders: inputs.folders.clone(),
        tools,
    };
    let mut credentials = inputs.credentials;
    let mut env: std::collections::HashMap<String, String> =
        crate::services::egress::proxy_env(&inputs.proxy_token)
            .into_iter()
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=')?;
                Some((name.to_string(), value.to_string()))
            })
            .collect();
    let repo = inputs.repo;
    let mut model = config
        .model
        .clone()
        .or(repo.model.clone())
        .or(inputs.default_model);
    // The team's own Anthropic-compatible gateway (`kind: litellm`)
    let repo_gateway = repo
        .endpoint
        .clone()
        .filter(|_| repo.provider.as_deref() == Some(REPO_GATEWAY))
        .map(|endpoint| GatewayConfig {
            kind: crate::services::harness::adapter::GatewayKind::External,
            endpoint: Some(endpoint),
        });
    let gateway = if config.gateway.is_some() {
        let Some((gateway, key)) = inputs.gateway else {
            return fallback("The LiteLLM gateway isn't running".to_string());
        };
        // The gateway's aliases stand for the Bedrock model
        model = Some(gateway::MODEL_ALIAS.to_string());
        env.insert(
            "ANTHROPIC_SMALL_FAST_MODEL".to_string(),
            gateway::SMALL_MODEL_ALIAS.to_string(),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
            gateway::SMALL_MODEL_ALIAS.to_string(),
        );
        credentials
            .custom_env
            .insert("ANTHROPIC_AUTH_TOKEN".to_string(), key);
        // The gateway holds the AWS identity; the CLI doesn't need it
        credentials.aws_access_key_id = None;
        credentials.aws_secret_access_key = None;
        credentials.aws_session_token = None;
        credentials.aws_bearer_token = None;
        credentials.aws_profile = None;
        Some(gateway)
    } else {
        repo_gateway
    };
    if inputs.host_only_sign_in && gateway.is_none() && !has_secret(&credentials) {
        return fallback(
            "Claude Code needs an API key to run in the container: your sign-in on this \
             computer can't be passed in. Add an Anthropic key in Settings › Models & runtimes."
                .to_string(),
        );
    }
    let provider = provider(agent, None)
        .or_else(|| repo.provider.clone().filter(|p| p != REPO_GATEWAY))
        .or_else(|| {
            (credentials.aws_access_key_id.is_some() || credentials.aws_bearer_token.is_some())
                .then(|| "bedrock".to_string())
        });
    // The model the CLI really uses: through the gateway, its Bedrock model
    let named = if config.gateway.is_some() {
        config.model.as_deref()
    } else {
        model.as_deref()
    };
    let model_ref = cli_model_ref(&harness_id, named);
    if let Err(by) = crate::services::domain::policy::model_allowed(&inputs.model_rules, &model_ref)
    {
        // Not a fallback: running natively would quietly change the model
        return RuntimeDecision::Unavailable(format!(
            "The policy '{by}' doesn't allow the model {model_ref} for agent '{}'",
            agent.id().as_str()
        ));
    }
    let egress_hosts = model_hosts(&harness_id, &credentials, gateway.as_ref());
    let grant = inputs.tools.unwrap_or_else(|| ToolGrant::of(agent));
    RuntimeDecision::Harness(Box::new(HarnessPlan {
        egress_hosts,
        config: HarnessConfig {
            working_dir: inputs.workdir,
            env,
            mcp_servers: Vec::new(),
            max_turns: max_turns(agent),
            allowed_tools: allowed_tool_kinds(grant.allowed.as_deref()),
            denied_tools: tool_kinds(&grant.denied),
            // The asker joins in `execute_harness_run`, which knows the run
            approvals: crate::services::harness::acp::Approvals {
                kinds: approval_tool_kinds(&grant.approval),
                asker: None,
            },
            model: model.clone(),
            gateway,
            credentials,
            system_prompt: Some(format!("{}{}", agent.system_prompt(), inputs.folder_prompt))
                .filter(|p| !p.trim().is_empty()),
            placement,
            request_timeout: None,
        },
        harness_id,
        provider,
        model,
    }))
}

/// Where a CLI's own model calls go, given how it's signed in: allowed
/// through the egress proxy on top of the agent's `network.allow`. Nothing
/// for the LiteLLM gateway, which coding agents reach on their network.
fn model_hosts(
    harness_id: &str,
    credentials: &HarnessCredentials,
    gateway: Option<&GatewayConfig>,
) -> Vec<String> {
    use crate::services::harness::adapter::GatewayKind;
    let host_of = |url: &str| {
        let url = reqwest::Url::parse(url).ok()?;
        let host = url.host_str()?.to_string();
        Some(match url.port() {
            Some(port) if port != 443 => format!("{host}:{port}"),
            _ => host,
        })
    };
    if let Some(gateway) = gateway {
        return match gateway.kind {
            GatewayKind::Litellm => Vec::new(),
            GatewayKind::External => gateway
                .endpoint
                .as_deref()
                .and_then(host_of)
                .into_iter()
                .collect(),
        };
    }
    let hosts: &[&str] = match harness_id {
        "claude-code" => {
            if let Some(host) = credentials.azure_endpoint.as_deref().and_then(host_of) {
                return vec![host];
            }
            if credentials.aws_access_key_id.is_some() || credentials.aws_bearer_token.is_some() {
                &["*.amazonaws.com"]
            } else {
                &["api.anthropic.com"]
            }
        }
        "codex" => &["api.openai.com", "chatgpt.com", "auth.openai.com"],
        "gemini-cli" => &[
            "generativelanguage.googleapis.com",
            "cloudcode-pa.googleapis.com",
            "oauth2.googleapis.com",
        ],
        // A custom CLI gets what its skill.md allows
        _ => &[],
    };
    hosts.iter().map(|h| h.to_string()).collect()
}

/// Gathers `plan`'s inputs for `agent` and decides. Starts the gateway for
/// an agent that needs it.
pub async fn resolve_runtime(app: &AppState, agent: &Agent) -> RuntimeDecision {
    if agent.runtime().is_native() {
        return RuntimeDecision::Native;
    }
    let harness_id = agent.runtime().as_str().into_owned();
    let config = agent.runtime_config().cloned().unwrap_or_default();
    let service = app.harnesses();
    let detected = service
        .detector()
        .stored_one(&harness_id)
        .await
        .ok()
        .flatten();
    let workdir = match crate::utils::nandi_config::work_dir() {
        Ok(dir) => dir.join(agent.id().as_str()),
        Err(e) => return RuntimeDecision::Unavailable(e),
    };
    if let Err(e) = tokio::fs::create_dir_all(&workdir).await {
        return RuntimeDecision::Unavailable(format!(
            "Could not create the workdir {}: {e}",
            workdir.display()
        ));
    }
    let default_model = service
        .default()
        .await
        .ok()
        .flatten()
        .filter(|d| d.harness_id == harness_id)
        .and_then(|d| d.model);
    // Agent's choice > the team's `.nandi/llm.yaml` > what the CLI is set up for
    let repo = repo_config::harness_defaults(app.store());
    let provider = provider(
        agent,
        repo.provider
            .as_deref()
            .filter(|p| *p != REPO_GATEWAY)
            .or_else(|| {
                detected
                    .as_ref()
                    .and_then(|d| d.health.configured_provider.as_deref())
            }),
    );
    let credentials = match credentials(app, agent, provider.as_deref(), &repo).await {
        Ok(credentials) => credentials,
        Err(e) => return RuntimeDecision::Unavailable(e),
    };
    // skill.md narrowed by its policies (folders, tools); a broken policy
    // or a refused model stops it
    let skill = match app.store().skill(agent.id().as_str()) {
        Ok(skill) => skill,
        Err(e) => return RuntimeDecision::Unavailable(e),
    };
    let shared = skill.folders.clone();
    let gateway = match &config.gateway {
        Some(_) => {
            let gateway = app.litellm_gateway();
            let Some(bedrock_model) = config.model.clone() else {
                return RuntimeDecision::Unavailable(
                    "runtime_config.model must name the Bedrock model for the gateway".to_string(),
                );
            };
            if gateway.model().as_deref() != Some(bedrock_model.as_str()) {
                if let Err(e) = gateway.start(&bedrock_model, &credentials).await {
                    return RuntimeDecision::Unavailable(e);
                }
            }
            gateway.session_gateway(true)
        }
        None => None,
    };
    plan(
        agent,
        RouterInputs {
            sandbox: sandbox_ready(&harness_id).await,
            default_model,
            workdir,
            credentials,
            gateway,
            repo,
            host_only_sign_in: harness_id == "claude-code"
                && !crate::services::harness::detection::container_auth(
                    &harness_id,
                    &crate::services::harness::detection::DetectEnv::current(),
                ),
            folders: crate::runtime::folder_mounts(&shared),
            proxy_token: app.proxy_tokens().issue(agent.id().as_str()),
            tools: Some(ToolGrant {
                allowed: skill.allowed_tools.clone(),
                denied: skill.denied_tools.clone(),
                approval: skill.requires_approval.clone(),
            }),
            model_rules: skill.model_rules.clone(),
            folder_prompt: crate::services::connectors::folders::prompt(
                &shared,
                crate::runtime::FOLDERS_MOUNT,
            ),
        },
    )
}

/// The steps a harness run may take: `runtime_config.max_turns`, else
/// skill.md `max_turns` (as for native runs). ACP has no turn limit, so
/// `execute_harness_run` enforces it.
pub fn max_turns(agent: &Agent) -> u32 {
    agent
        .runtime_config()
        .and_then(|c| c.extra.get("max_turns"))
        .and_then(serde_yaml::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| TurnPolicy::from_extra(agent.extra()).max_turns)
}

/// Counts a harness run's model steps against `max_turns`. ACP reports one
/// prompt response per prompt, however many steps the CLI takes, so a step
/// is a tool call (each one sends the model another request).
#[derive(Debug, Clone, Copy)]
pub struct TurnBudget {
    max_turns: u32,
    steps: u32,
}

impl TurnBudget {
    /// `0`: no limit
    pub fn new(max_turns: u32) -> Self {
        Self {
            max_turns,
            steps: 0,
        }
    }

    /// Counts `update`; `Some(message)` once the run is over its limit
    pub fn observe(&mut self, update: &AcpSessionUpdate) -> Option<String> {
        if !matches!(update, AcpSessionUpdate::ToolCallStarted { .. }) {
            return None;
        }
        self.steps += 1;
        let max = self.max_turns;
        (max > 0 && self.steps > max).then(|| {
            format!("Stopped: this run reached its limit of {max} turns (skill.md `max_turns`).")
        })
    }
}

/// A key or token the CLI can sign in with (a region or profile name alone
/// isn't one)
fn has_secret(credentials: &HarnessCredentials) -> bool {
    credentials.anthropic_api_key.is_some()
        || credentials.aws_access_key_id.is_some()
        || credentials.aws_bearer_token.is_some()
        || credentials.azure_api_key.is_some()
        || !credentials.custom_env.is_empty()
}

/// Credentials for `provider` from the user's own setup: the AWS identity
/// for Bedrock (the agent's profile and region, else the repo's), resolved
/// on the host to temporary keys (SSO, SAML/`credential_process`,
/// assume-role), so `~/.aws` never enters the container; Nandi's keyring
/// keys for Anthropic (only when Claude Code has no sign-in of its own)
/// and OpenAI; the repo's Azure endpoint.
/// Nothing for providers the CLI signs into itself.
/// Gets the harness sandbox ready for `harness_id`, waiting for it: the
/// image (pulled when missing, progress on `images://pull`), then its tool
/// volume on first use (Claude Code, progress on `images://tools`)
async fn sandbox_ready(harness_id: &str) -> Sandbox {
    use crate::services::images::{self, tools::ToolBundle, SandboxImage};
    let ready = match ToolBundle::for_harness(harness_id) {
        Some(bundle) => images::tools::ensure(bundle).await.map(|mount| vec![mount]),
        None => images::ensure(SandboxImage::Harness)
            .await
            .map(|()| Vec::new()),
    };
    match ready {
        Ok(tools) => Sandbox::Ready(tools),
        Err(reason) => Sandbox::Unavailable(reason),
    }
}

async fn credentials(
    app: &AppState,
    agent: &Agent,
    provider: Option<&str>,
    repo: &HarnessDefaults,
) -> Result<HarnessCredentials, String> {
    use crate::llm::Provider;
    use crate::services::llm_keys::require_api_key;

    let config = agent.runtime_config();
    match provider {
        Some("bedrock") => {
            crate::services::harness::aws_auth::bedrock_credentials(
                app,
                config
                    .and_then(|c| c.aws_profile.clone())
                    .or_else(|| repo.aws_profile.clone()),
                config
                    .and_then(|c| c.aws_region.clone())
                    .or_else(|| repo.aws_region.clone()),
            )
            .await
        }
        // The key stays in the environment (`ANTHROPIC_FOUNDRY_API_KEY`)
        Some("azure") => Ok(HarnessCredentials {
            azure_endpoint: repo.endpoint.clone(),
            ..HarnessCredentials::default()
        }),
        // Keys last: Claude Code's own sign-in (its settings, or the host
        // login's token) reaches the container first
        Some("anthropic") if *agent.runtime() == AgentRuntimeKind::ClaudeCode => {
            let own = crate::services::harness::detection::container_auth(
                "claude-code",
                &crate::services::harness::detection::DetectEnv::current(),
            );
            Ok(HarnessCredentials {
                anthropic_api_key: (!own)
                    .then(|| require_api_key(app.secrets(), Provider::Anthropic).ok())
                    .flatten(),
                ..HarnessCredentials::default()
            })
        }
        Some("openai") if *agent.runtime() == AgentRuntimeKind::Codex => Ok(HarnessCredentials {
            openai_api_key: require_api_key(app.secrets(), Provider::OpenAi).ok(),
            ..HarnessCredentials::default()
        }),
        _ => Ok(HarnessCredentials::default()),
    }
}

/// The first prompt: the agent's system prompt, then the run's input (ACP
/// has no stable system-prompt field, and this works with every harness)
pub fn first_prompt(system: Option<&str>, input: &str) -> String {
    match system.map(str::trim).filter(|s| !s.is_empty()) {
        Some(system) => format!("<instructions>\n{system}\n</instructions>\n\n{input}"),
        None => input.to_string(),
    }
}

/// Closes the session row if the run's task is dropped (cancelled)
struct SessionGuard<'a> {
    service: &'a HarnessService,
    session_id: String,
    done: bool,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let db = self.service.db().clone();
        let id = self.session_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = evidence::session_ended(
                    &db,
                    &id,
                    HarnessSessionStatus::Killed,
                    Some("cancelled".to_string()),
                    Some("The run was cancelled".to_string()),
                    AcpUsage::default(),
                )
                .await;
            });
        }
    }
}

/// Takes a run's model hosts back when it ends, however it ends
struct SessionHostsGuard(String);

impl Drop for SessionHostsGuard {
    fn drop(&mut self) {
        crate::services::egress::clear_session_hosts(&self.0);
    }
}

/// Runs one run through `adapter` and ends it (completed or failed). The
/// run's input is the prompt; updates stream into its messages.
pub async fn execute_harness_run(
    bus: &EventBus,
    service: &HarnessService,
    adapter: Box<dyn HarnessAdapter>,
    plan: &HarnessPlan,
    ctx: &RunContext,
) -> Result<(), String> {
    let id = ctx.run_id.clone();
    let (run, messages) = bus
        .db
        .call(move |conn| Ok((db::get_run(conn, &id)?, db::list_run_messages(conn, &id)?)))
        .await?;
    let run = run.ok_or_else(|| format!("Run {} not found", ctx.run_id))?;
    if run.status != RunStatus::Running {
        return Ok(());
    }
    let first_step = messages
        .iter()
        .filter(|m| m.role == role::ASSISTANT)
        .count() as u32;
    evidence::attribute_run(&bus.db, &ctx.run_id, &plan.harness_id, None, None).await?;

    let mut adapter = adapter;
    // Calls in `requires_approval` wait for the user in the Inbox; they stop
    // waiting when this run ends, however it ends (the guard drops)
    let ended = tokio_util::sync::CancellationToken::new();
    let _ended_guard = ended.clone().drop_guard();
    // Its CLI's model hosts, through the egress proxy, for this run only
    crate::services::egress::allow_session_hosts(&ctx.agent_id, &plan.egress_hosts);
    let _hosts_guard = SessionHostsGuard(ctx.agent_id.clone());
    let mut config = plan.config.clone();
    config.approvals.asker = Some(std::sync::Arc::new(
        crate::services::harness::approvals::RunApprover::new(bus.clone(), ctx.clone(), ended),
    ));
    let container_name = match &plan.config.placement {
        Placement::Container { name, .. } => Some(name.clone()),
        Placement::Host => None,
    };
    let started = service
        .start_session(
            adapter.as_ref(),
            config,
            SessionMeta {
                agent_id: Some(ctx.agent_id.clone()),
                run_id: Some(ctx.run_id.clone()),
                model: plan.model.clone(),
                provider: plan.provider.clone(),
                container_name,
            },
        )
        .await;
    let (session, kill) = match started {
        Ok(started) => started,
        Err(error) => {
            let _ = adapter.stop().await;
            let message = format!("{} could not start: {error}", plan.harness_id);
            return bus.turn_done(ctx, RunStatus::Failed, Some(message)).await;
        }
    };
    let mut guard = SessionGuard {
        service,
        session_id: session.id.clone(),
        done: false,
    };

    let prompt = first_prompt(plan.config.system_prompt.as_deref(), &run.input);
    let outcome = match adapter.send(UserMessage::text(prompt)).await {
        Err(error) => TurnOutcome::Failed(error.to_string()),
        Ok(mut stream) => {
            let mut turn = TurnTranslator::new(bus.clone(), ctx.clone(), &session.id, first_step);
            let mut budget = TurnBudget::new(plan.config.max_turns);
            loop {
                tokio::select! {
                    update = stream.recv() => match update {
                        Some(update) => {
                            service.record(&session.id, &update).await;
                            if let Some(message) = budget.observe(&update) {
                                // `session/cancel`: the CLI stops before running the tool
                                let _ = adapter.cancel().await;
                                break TurnOutcome::Failed(message);
                            }
                            if let Some(outcome) = turn.apply(&update).await? {
                                break outcome;
                            }
                        }
                        // The CLI went away without finishing the turn
                        None => break TurnOutcome::Failed(format!(
                            "{} exited before finishing the turn",
                            plan.harness_id
                        )),
                    },
                    _ = kill.cancelled() => {
                        let _ = adapter.cancel().await;
                        break TurnOutcome::Failed("Stopped by the user".to_string());
                    }
                }
            }
        }
    };
    let _ = adapter.stop().await;

    guard.done = true;
    match outcome {
        TurnOutcome::Finished { stop_reason, usage } => {
            evidence::attribute_run(
                &bus.db,
                &ctx.run_id,
                &plan.harness_id,
                plan.model.as_deref(),
                usage,
            )
            .await?;
            service
                .end_session(
                    &session.id,
                    HarnessSessionStatus::Finished,
                    Some(stop_reason.clone()),
                    None,
                    usage.unwrap_or_default(),
                )
                .await;
            events::emit(
                bus,
                &HarnessTurnFinished {
                    session_id: session.id.clone(),
                    run_id: Some(ctx.run_id.clone()),
                    stop_reason: Some(stop_reason.clone()),
                    usage,
                },
            );
            match stop_reason.as_str() {
                "cancelled" => bus.turn_done(ctx, RunStatus::Cancelled, None).await,
                "refusal" => {
                    bus.turn_done(
                        ctx,
                        RunStatus::Failed,
                        Some("The model refused to continue".to_string()),
                    )
                    .await
                }
                _ => bus.turn_done(ctx, RunStatus::Completed, None).await,
            }
        }
        TurnOutcome::Failed(message) => {
            service
                .end_session(
                    &session.id,
                    HarnessSessionStatus::Failed,
                    None,
                    Some(message.clone()),
                    AcpUsage::default(),
                )
                .await;
            events::emit(
                bus,
                &HarnessErrorEvent {
                    harness_id: plan.harness_id.clone(),
                    session_id: Some(session.id.clone()),
                    run_id: Some(ctx.run_id.clone()),
                    message: message.clone(),
                },
            );
            bus.turn_done(ctx, RunStatus::Failed, Some(message)).await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::services::domain::AgentId;
    use crate::services::events::tests::{bus, started};
    use crate::services::harness::adapter::{
        CliProfile, EventStream, HarnessError, HarnessSession, ModelRef,
    };
    use crate::services::harness::detection::{DetectEnv, HarnessDetector};

    fn agent(frontmatter: &str) -> Agent {
        Agent::from_skill_md(
            AgentId::parse("writer").unwrap(),
            &format!("---\nname: Writer\n{frontmatter}---\n\nYou write.\n"),
        )
        .unwrap()
    }

    fn inputs() -> RouterInputs {
        RouterInputs {
            sandbox: Sandbox::default(),
            workdir: PathBuf::from("/work/writer"),
            ..RouterInputs::default()
        }
    }

    fn harness_plan(decision: RuntimeDecision) -> HarnessPlan {
        match decision {
            RuntimeDecision::Harness(plan) => *plan,
            other => panic!("expected a harness, got {other:?}"),
        }
    }

    #[test]
    fn agents_without_a_runtime_stay_native_and_unchanged() {
        let agent = agent("");
        assert!(agent.runtime().is_native());
        assert!(matches!(plan(&agent, inputs()), RuntimeDecision::Native));
        assert!(!agent.to_skill_md().contains("runtime"));
    }

    #[test]
    fn runtime_claude_code_resolves_to_a_container_harness() {
        let agent = agent(
            "runtime: claude-code\nruntime_config:\n  model: us.anthropic.claude-sonnet-5\n  provider: bedrock\n  team_note: kept\nallowed-tools: [read_file, shell]\n",
        );
        assert_eq!(*agent.runtime(), AgentRuntimeKind::ClaudeCode);
        let rendered = agent.to_skill_md();
        assert!(rendered.contains("runtime: claude-code"), "{rendered}");
        assert!(rendered.contains("team_note: kept"), "unknown keys survive");

        let plan = harness_plan(plan(&agent, inputs()));
        assert_eq!(plan.harness_id, "claude-code");
        assert_eq!(plan.model.as_deref(), Some("us.anthropic.claude-sonnet-5"));
        assert_eq!(plan.provider.as_deref(), Some("bedrock"));
        assert_eq!(plan.config.allowed_tools, vec!["execute", "read", "search"]);
        assert_eq!(plan.config.system_prompt.as_deref(), Some("You write."));
        let Placement::Container {
            name,
            host_workdir,
            network,
            ..
        } = &plan.config.placement
        else {
            panic!("container placement")
        };
        assert_eq!(name, &container_name("writer"));
        assert_eq!(host_workdir, &PathBuf::from("/work/writer"));
        // Always the internal network, gateway or not
        assert_eq!(network.as_deref(), Some("nandi-agents"));
    }

    #[test]
    fn harness_containers_get_the_native_limits_unless_skill_md_sets_them() {
        let cases = [
            ("", (DEFAULT_MEMORY_MB, DEFAULT_CPU)),
            (
                "memory_mb: 4096
cpu: 2.5
",
                (4096, 2.5),
            ),
            (
                "memory_mb: 512
",
                (512, DEFAULT_CPU),
            ),
        ];
        for (limits, expected) in cases {
            let agent = agent(&format!(
                "runtime: claude-code
{limits}"
            ));
            let plan = harness_plan(plan(&agent, inputs()));
            let Placement::Container { memory_mb, cpu, .. } = &plan.config.placement else {
                panic!("container placement")
            };
            assert_eq!(
                (*memory_mb, *cpu),
                (Some(expected.0), Some(expected.1)),
                "{limits:?}"
            );
        }
    }

    #[test]
    fn the_container_gets_the_providers_env_and_nothing_else() {
        let agent =
            agent("runtime: claude-code\nruntime_config:\n  model: us.anthropic.claude-sonnet-5\n");
        let mut inputs = inputs();
        inputs.credentials = HarnessCredentials {
            aws_region: Some("us-west-2".into()),
            aws_access_key_id: Some("ASIA".into()),
            aws_secret_access_key: Some("secret".into()),
            aws_session_token: Some("token".into()),
            aws_profile: Some("work".into()),
            ..HarnessCredentials::default()
        };
        let plan = harness_plan(plan(&agent, inputs));
        let home = tempfile::tempdir().unwrap();
        let profile_env = DetectEnv {
            home: home.path().to_path_buf(),
            path: Some(home.path().as_os_str().to_owned()),
            vars: HashMap::from([("UNRELATED_SECRET".to_string(), "x".to_string())]),
        };
        let adapter =
            crate::services::harness::claude_code::ClaudeCodeAdapter::with_env(profile_env);
        let env: HashMap<_, _> = adapter.profile().env(&plan.config).into_iter().collect();
        assert_eq!(env["CLAUDE_CODE_USE_BEDROCK"], "1");
        assert_eq!(env["AWS_REGION"], "us-west-2");
        assert_eq!(env["AWS_SESSION_TOKEN"], "token");
        assert_eq!(env["ANTHROPIC_MODEL"], "us.anthropic.claude-sonnet-5");
        assert!(
            !env.contains_key("AWS_PROFILE"),
            "no ~/.aws in the container"
        );
        assert!(!env.contains_key("UNRELATED_SECRET"));
    }

    #[test]
    fn the_repo_config_fills_in_what_the_agent_leaves_open() {
        let mut inputs = inputs();
        inputs.default_model = Some("sonnet".into());
        inputs.repo = HarnessDefaults {
            provider: Some("bedrock".into()),
            aws_profile: Some("acme-dev".into()),
            aws_region: Some("us-east-1".into()),
            endpoint: None,
            model: Some("us.anthropic.claude-sonnet-5".into()),
        };
        let open = harness_plan(plan(&agent("runtime: claude-code\n"), inputs.clone()));
        assert_eq!(open.provider.as_deref(), Some("bedrock"));
        assert_eq!(
            open.model.as_deref(),
            Some("us.anthropic.claude-sonnet-5"),
            "the repo beats the local default harness model"
        );

        // The agent's own runtime_config wins
        let own =
            agent("runtime: claude-code\nruntime_config:\n  provider: anthropic\n  model: opus\n");
        let own = harness_plan(plan(&own, inputs));
        assert_eq!(own.provider.as_deref(), Some("anthropic"));
        assert_eq!(own.model.as_deref(), Some("opus"));
    }

    #[test]
    fn a_repo_litellm_gateway_is_an_external_anthropic_endpoint() {
        let mut inputs = inputs();
        inputs.repo = HarnessDefaults {
            provider: Some("litellm".into()),
            endpoint: Some("https://llm.acme.internal".into()),
            ..HarnessDefaults::default()
        };
        let plan = harness_plan(plan(&agent("runtime: claude-code\n"), inputs));
        let gateway = plan.config.gateway.as_ref().unwrap();
        assert_eq!(
            gateway.kind,
            crate::services::harness::adapter::GatewayKind::External
        );
        assert_eq!(
            gateway.endpoint.as_deref(),
            Some("https://llm.acme.internal")
        );
        assert_eq!(plan.provider, None, "the gateway owns the provider");
    }

    #[test]
    fn a_gateway_agent_uses_the_alias_and_the_gateway_key() {
        let agent = agent(
            "runtime: claude-code\nruntime_config:\n  model: meta.llama3-3-70b-instruct-v1:0\n  gateway: litellm\n",
        );
        assert!(matches!(
            plan(&agent, inputs()),
            RuntimeDecision::Unavailable(reason) if reason.contains("gateway")
        ));
        let mut inputs = inputs();
        inputs.credentials.aws_access_key_id = Some("ASIA".into());
        inputs.gateway = Some((
            GatewayConfig {
                kind: crate::services::harness::adapter::GatewayKind::Litellm,
                endpoint: Some("http://nandi-litellm:4000".into()),
            },
            "sk-nandi-key".into(),
        ));
        let plan = harness_plan(plan(&agent, inputs));
        assert_eq!(plan.model.as_deref(), Some(gateway::MODEL_ALIAS));
        assert_eq!(plan.provider.as_deref(), Some("bedrock"));
        assert_eq!(
            plan.config.credentials.custom_env["ANTHROPIC_AUTH_TOKEN"],
            "sk-nandi-key"
        );
        assert_eq!(
            plan.config.credentials.aws_access_key_id, None,
            "the gateway holds AWS"
        );
        assert!(matches!(
            &plan.config.placement,
            Placement::Container { network: Some(n), .. } if n == "nandi-agents"
        ));
        assert!(
            plan.egress_hosts.is_empty(),
            "the gateway is on its network"
        );
    }

    #[test]
    fn an_unavailable_harness_falls_back_only_when_asked() {
        let mut missing = inputs();
        missing.sandbox =
            Sandbox::Unavailable(format!("Couldn't download {HARNESS_IMAGE} (offline)"));
        let strict = agent("runtime: codex\n");
        assert!(matches!(
            plan(&strict, missing.clone()),
            RuntimeDecision::Unavailable(reason) if reason.contains(HARNESS_IMAGE)
        ));
        let lenient = agent("runtime: codex\nruntime_config:\n  fallback: native\n");
        assert!(matches!(plan(&lenient, missing), RuntimeDecision::Native));
    }

    #[test]
    fn harness_calls_ask_for_approval_like_native_tools() {
        let kinds = |frontmatter: &str| {
            harness_plan(plan(&agent(frontmatter), inputs()))
                .config
                .approvals
                .kinds
        };
        // Default: shell and write_file
        assert_eq!(
            kinds("runtime: codex\n"),
            ["delete", "edit", "execute", "move"]
        );
        assert_eq!(
            kinds("runtime: codex\nrequires_approval: [fetch, deploy]\n"),
            ["deploy", "fetch"]
        );
        assert!(kinds("runtime: codex\nrequires_approval: []\n").is_empty());
    }

    #[test]
    fn a_policy_s_tools_reach_the_cli() {
        let mut narrowed = inputs();
        narrowed.tools = Some(ToolGrant {
            allowed: None,
            denied: vec!["shell".into()],
            approval: vec!["fetch".into()],
        });
        let plan = harness_plan(plan(&agent("runtime: codex\n"), narrowed));
        assert_eq!(plan.config.denied_tools, ["execute"]);
        assert_eq!(plan.config.approvals.kinds, ["fetch"]);
        assert!(plan.config.allowed_tools.is_empty(), "everything else");
    }

    #[test]
    fn a_policy_s_models_allow_covers_the_cli_model() {
        let rules = vec![(
            vec!["claude:sonnet*".to_string()],
            "only-sonnet".to_string(),
        )];
        let mut narrowed = inputs();
        narrowed.model_rules = rules;
        let sonnet = agent("runtime: claude-code\nruntime_config:\n  model: sonnet\n");
        assert!(matches!(
            plan(&sonnet, narrowed.clone()),
            RuntimeDecision::Harness(_)
        ));
        let opus = agent("runtime: claude-code\nruntime_config:\n  model: opus\n");
        assert!(matches!(
            plan(&opus, narrowed.clone()),
            RuntimeDecision::Unavailable(reason)
                if reason.contains("'only-sonnet'") && reason.contains("claude:opus")
        ));
        // A fallback doesn't get around it
        let lenient =
            agent("runtime: codex\nruntime_config:\n  model: gpt-5\n  fallback: native\n");
        assert!(matches!(
            plan(&lenient, narrowed),
            RuntimeDecision::Unavailable(_)
        ));
        assert_eq!(cli_model_ref("codex", None), "gpt:default");
    }

    #[test]
    fn coding_agents_leave_only_through_the_proxy_to_their_model() {
        let mut keyed = inputs();
        keyed.proxy_token = "tok".into();
        keyed.credentials.anthropic_api_key = Some("sk-ant".into());
        let plan = harness_plan(plan(&agent("runtime: claude-code\n"), keyed));
        assert!(matches!(
            &plan.config.placement,
            Placement::Container { network: Some(n), .. } if n == "nandi-agents"
        ));
        assert_eq!(
            plan.config.env["HTTPS_PROXY"],
            "http://nandi:tok@nandi-egress:11435"
        );
        assert_eq!(plan.egress_hosts, ["api.anthropic.com"]);

        let codex = harness_plan(plan_for("runtime: codex\n"));
        assert!(codex.egress_hosts.contains(&"api.openai.com".to_string()));
        let custom = harness_plan(plan_for("runtime: custom:mine\n"));
        assert!(custom.egress_hosts.is_empty(), "its network.allow only");
    }

    fn plan_for(frontmatter: &str) -> RuntimeDecision {
        plan(&agent(frontmatter), inputs())
    }

    #[test]
    fn a_container_without_credentials_is_refused_before_it_starts() {
        let mut host_login = inputs();
        host_login.host_only_sign_in = true;
        let strict = agent("runtime: claude-code\n");
        assert!(matches!(
            plan(&strict, host_login.clone()),
            RuntimeDecision::Unavailable(reason) if reason.contains("API key")
        ));
        let lenient = agent("runtime: claude-code\nruntime_config:\n  fallback: native\n");
        assert!(matches!(
            plan(&lenient, host_login.clone()),
            RuntimeDecision::Native
        ));
        // A Nandi key is enough
        let mut keyed = host_login;
        keyed.credentials.anthropic_api_key = Some("sk-ant".into());
        assert!(matches!(plan(&strict, keyed), RuntimeDecision::Harness(_)));
    }

    #[test]
    fn invalid_runtimes_are_rejected() {
        for bad in [
            "runtime: cursor\n",
            "runtime: custom:\n",
            "runtime: claude-code\nruntime_config:\n  gateway: other\n",
            "runtime: claude-code\nruntime_config:\n  fallback: always\n",
            "runtime: claude-code\nruntime_config:\n  model: two words\n",
        ] {
            assert!(
                Agent::from_skill_md(
                    AgentId::parse("a").unwrap(),
                    &format!("---\nname: A\n{bad}---\n")
                )
                .is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            *agent("runtime: custom:my-agent\n").runtime(),
            AgentRuntimeKind::Custom("my-agent".into())
        );
    }

    #[test]
    fn the_editor_writes_runtime_and_keeps_runtime_config() {
        use crate::services::repo_service::{definition_agent, AgentDefinitionInput};
        let existing = "---\nname: Writer\nruntime: codex\nruntime_config:\n  model: old\n  gateway: litellm\n---\n\nBody\n";
        let input = AgentDefinitionInput {
            name: "Writer".into(),
            description: String::new(),
            skill_body: "Body".into(),
            instructions: String::new(),
            model_provider: None,
            model_name: None,
            image: None,
            memory_mb: None,
            cpu: None,
            capabilities: Default::default(),
            runtime: Some("claude-code".into()),
            runtime_model: Some("sonnet".into()),
            aws_profile: None,
        };
        let id = AgentId::parse("writer").unwrap();
        let agent = definition_agent(&id, Some(existing), &input).unwrap();
        assert_eq!(*agent.runtime(), AgentRuntimeKind::ClaudeCode);
        let config = agent.runtime_config().unwrap();
        assert_eq!(config.model.as_deref(), Some("sonnet"));
        assert_eq!(config.gateway.as_deref(), Some("litellm"));

        let native = AgentDefinitionInput {
            runtime: Some("native".into()),
            runtime_model: None,
            ..input
        };
        let agent = definition_agent(&id, Some(existing), &native).unwrap();
        assert!(agent.runtime().is_native());
        assert!(!agent.to_skill_md().contains("runtime"));
    }

    #[test]
    fn the_first_prompt_carries_the_system_prompt() {
        assert_eq!(first_prompt(None, "hi"), "hi");
        assert_eq!(first_prompt(Some("  "), "hi"), "hi");
        assert!(first_prompt(Some("Be brief"), "hi").starts_with("<instructions>\nBe brief"));
    }

    /// A harness that streams `script` once, then closes
    struct ScriptedAdapter {
        script: Mutex<Vec<AcpSessionUpdate>>,
    }

    #[async_trait]
    impl HarnessAdapter for ScriptedAdapter {
        async fn start(&self, config: HarnessConfig) -> Result<HarnessSession, HarnessError> {
            Ok(HarnessSession {
                id: uuid::Uuid::new_v4().to_string(),
                adapter_id: "claude-code".into(),
                child_pid: 7,
                started_at: 0.0,
                working_dir: config.working_dir,
            })
        }

        async fn send(&mut self, _: UserMessage) -> Result<EventStream, HarnessError> {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            let script = std::mem::take(&mut *self.script.lock().unwrap());
            for update in script {
                tx.send(update).await.unwrap();
            }
            Ok(rx)
        }

        async fn cancel(&mut self) -> Result<(), HarnessError> {
            Ok(())
        }

        async fn stop(self: Box<Self>) -> Result<(), HarnessError> {
            Ok(())
        }

        fn id(&self) -> &'static str {
            "claude-code"
        }

        fn supported_models(&self) -> Vec<ModelRef> {
            Vec::new()
        }
    }

    async fn run_scripted(script: Vec<AcpSessionUpdate>) -> (EventBus, Vec<&'static str>, String) {
        run_scripted_as("runtime: claude-code\n", script).await
    }

    async fn run_scripted_as(
        frontmatter: &str,
        script: Vec<AcpSessionUpdate>,
    ) -> (EventBus, Vec<&'static str>, String) {
        let (bus, sink) = bus();
        let ctx = started(&bus, "r1").await;
        let detector = Arc::new(HarnessDetector::new(bus.db.clone(), DetectEnv::default()));
        let service = HarnessService::new(bus.db.clone(), bus.clone(), detector);
        let plan = harness_plan(plan(&agent(frontmatter), inputs()));
        let adapter = Box::new(ScriptedAdapter {
            script: Mutex::new(script),
        });
        execute_harness_run(&bus, &service, adapter, &plan, &ctx)
            .await
            .unwrap();
        let names = sink.names();
        let status = bus
            .db
            .call(|conn| {
                let run = db::get_run(conn, "r1")?.unwrap();
                let runtime: Option<String> =
                    conn.query_row("SELECT runtime FROM runs WHERE id = 'r1'", [], |r| r.get(0))?;
                Ok(format!("{:?} {}", run.status, runtime.unwrap_or_default()))
            })
            .await
            .unwrap();
        (bus, names, status)
    }

    #[tokio::test]
    async fn a_finished_turn_completes_the_run_with_usage_and_runtime() {
        let (bus, names, status) = run_scripted(vec![
            AcpSessionUpdate::MessageDelta {
                text: "Done".into(),
            },
            AcpSessionUpdate::TurnFinished {
                stop_reason: "end_turn".into(),
                usage: Some(AcpUsage {
                    input_tokens: 12,
                    output_tokens: 4,
                    ..AcpUsage::default()
                }),
            },
        ])
        .await;
        assert_eq!(status, "Completed claude-code");
        assert!(names.contains(&"agent://token"));
        assert!(names.contains(&"harness://session_started"));
        assert!(names.contains(&"harness://turn_finished"));
        let tokens: i64 = bus
            .db
            .call(|conn| {
                conn.query_row("SELECT input_tokens FROM runs WHERE id = 'r1'", [], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(tokens, 12);
    }

    fn tool_call(id: &str) -> AcpSessionUpdate {
        AcpSessionUpdate::ToolCallStarted {
            tool_call_id: id.into(),
            title: "ls".into(),
            tool_kind: Some("execute".into()),
            input: serde_json::Value::Null,
        }
    }

    #[test]
    fn max_turns_comes_from_runtime_config_then_skill_md() {
        let default = agent("runtime: claude-code\n");
        assert_eq!(
            harness_plan(plan(&default, inputs())).config.max_turns,
            crate::services::orchestrator::middleware::DEFAULT_MAX_TURNS
        );
        let skill = agent("runtime: claude-code\nmax_turns: 7\n");
        assert_eq!(harness_plan(plan(&skill, inputs())).config.max_turns, 7);
        let own = agent("runtime: claude-code\nmax_turns: 7\nruntime_config:\n  max_turns: 3\n");
        assert_eq!(harness_plan(plan(&own, inputs())).config.max_turns, 3);
    }

    #[test]
    fn the_budget_counts_tool_calls() {
        let mut budget = TurnBudget::new(2);
        let text = AcpSessionUpdate::MessageDelta { text: "x".into() };
        assert!(budget.observe(&text).is_none());
        assert!(budget.observe(&tool_call("a")).is_none());
        assert!(budget.observe(&tool_call("b")).is_none());
        let over = budget.observe(&tool_call("c")).unwrap();
        assert!(over.contains("limit of 2 turns"), "{over}");
        let mut unlimited = TurnBudget::new(0);
        assert!((0..100).all(|i| unlimited.observe(&tool_call(&i.to_string())).is_none()));
    }

    #[tokio::test]
    async fn a_harness_run_over_max_turns_is_cancelled_and_fails() {
        let (bus, _names, status) = run_scripted_as(
            "runtime: claude-code\nruntime_config:\n  max_turns: 1\n",
            vec![
                tool_call("a"),
                tool_call("b"),
                AcpSessionUpdate::TurnFinished {
                    stop_reason: "end_turn".into(),
                    usage: None,
                },
            ],
        )
        .await;
        assert_eq!(status, "Failed claude-code");
        let error: String = bus
            .db
            .call(|conn| conn.query_row("SELECT error FROM harness_sessions", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!(error.contains("max_turns"), "{error}");
    }

    #[tokio::test]
    async fn a_cli_crash_fails_the_run_with_an_error_event() {
        // The stream closes before TurnFinished: the CLI died
        let (bus, names, status) = run_scripted(vec![AcpSessionUpdate::MessageDelta {
            text: "Half".into(),
        }])
        .await;
        assert_eq!(status, "Failed claude-code");
        assert!(names.contains(&"harness://error"), "{names:?}");
        assert!(names.contains(&"harness://session_ended"));
        let session_status: String = bus
            .db
            .call(|conn| conn.query_row("SELECT status FROM harness_sessions", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(session_status, "failed");
    }

    /// The run's plan for `harness_id` on this machine's sandbox, as a run
    /// gets it, in a throwaway workdir on Docker's default network
    async fn live_plan(harness_id: &str, workdir: &std::path::Path) -> HarnessPlan {
        let Sandbox::Ready(tools) = sandbox_ready(harness_id).await else {
            panic!("the {harness_id} sandbox isn't ready");
        };
        let mut inputs = inputs();
        inputs.sandbox = Sandbox::Ready(tools);
        inputs.workdir = workdir.to_path_buf();
        let mut plan = harness_plan(plan(
            &agent(&format!(
                "runtime: {harness_id}
"
            )),
            inputs,
        ));
        if let Placement::Container { network, name, .. } = &mut plan.config.placement {
            *network = None;
            *name = format!("nandi-harness-live-{harness_id}");
        }
        plan.config.working_dir = workdir.to_path_buf();
        plan
    }

    /// No sign-in of this computer's reaches the container
    fn signed_out() -> DetectEnv {
        DetectEnv {
            home: tempfile::tempdir().unwrap().keep(),
            path: None,
            vars: HashMap::new(),
        }
    }

    /// Live (Docker and the harness image): the first Claude Code run
    /// installs it into its volume, a second run reuses it, and the CLI
    /// starts in the container with the volume mounted read-only.
    /// `cargo test --lib first_use_claude_code_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn first_use_claude_code_live() {
        use crate::services::harness::claude_code::ClaudeCodeAdapter;
        use crate::services::images::tools::{self, ToolBundle};
        let bundle = ToolBundle::ClaudeCode;
        // A fresh machine: no tools volume
        let _ = tokio::process::Command::new("docker")
            .args(["volume", "rm", "-f", &bundle.volume()])
            .status()
            .await;
        tools::forget(bundle);
        let first = std::time::Instant::now();
        let workdir = tempfile::tempdir().unwrap();
        let plan = live_plan("claude-code", workdir.path()).await;
        println!("first use: Claude Code installed in {:?}", first.elapsed());
        let installed = tools::status(bundle).await;
        assert!(installed.ready, "{installed:?}");
        println!(
            "install log tail: {:?}",
            installed.log.iter().rev().take(3).collect::<Vec<_>>()
        );
        let Placement::Container { tools: mounts, .. } = &plan.config.placement else {
            panic!("runs in the container");
        };
        assert_eq!(mounts, &vec![bundle.mount()]);
        let args = crate::services::harness::adapter::docker_args(
            &crate::services::harness::adapter::Invocation::new("claude-agent-acp", &[]),
            &[],
            &plan.config.placement,
        );
        assert!(args.contains(&format!(
            "{}:/opt/nandi-tools/claude-code:ro",
            bundle.volume()
        )));

        // The next app start: checked, not installed again
        tools::forget(bundle);
        let second = std::time::Instant::now();
        live_plan("claude-code", workdir.path()).await;
        let reused = tools::status(bundle).await;
        println!("second run: reused the volume in {:?}", second.elapsed());
        assert!(reused.ready && reused.log.is_empty(), "{reused:?}");
        assert!(second.elapsed() < std::time::Duration::from_secs(30));

        // The CLI starts in the sandbox (no sign-in: as far as that goes)
        let adapter = ClaudeCodeAdapter::with_env(signed_out());
        let outcome =
            crate::services::harness::adapter::test_session(Box::new(adapter), plan.config).await;
        println!("Claude Code in the sandbox: {outcome:?}");
        assert!(
            !matches!(
                &outcome,
                Err(HarnessError::NotInstalled(_) | HarnessError::Spawn(_))
            ),
            "{outcome:?}"
        );
    }

    /// Live: a Codex run needs no extra step (it's in the image)
    /// `cargo test --lib codex_runs_with_no_extra_step_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn codex_runs_with_no_extra_step_live() {
        use crate::services::harness::codex::CodexAdapter;
        let workdir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let plan = live_plan("codex", workdir.path()).await;
        let Placement::Container { tools, .. } = &plan.config.placement else {
            panic!("runs in the container");
        };
        assert!(tools.is_empty(), "Codex mounts nothing extra");
        let adapter = CodexAdapter::with_env(signed_out());
        let outcome =
            crate::services::harness::adapter::test_session(Box::new(adapter), plan.config).await;
        println!(
            "Codex in the sandbox after {:?}: {outcome:?}",
            started.elapsed()
        );
        assert!(
            !matches!(
                &outcome,
                Err(HarnessError::NotInstalled(_) | HarnessError::Spawn(_))
            ),
            "{outcome:?}"
        );
    }
}
