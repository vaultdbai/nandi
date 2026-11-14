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
//! - `plan_test_session` plans Settings' "Test harness" session the same
//!   way, so it runs where and how a run does
//! - `execute_harness_run` drives one run through an adapter
//!
//! A harness that can't start fails the run, unless the agent opts into
//! `runtime_config.fallback: native`: silently switching to Nandi's own
//! model would also switch the provider the user chose.

use std::path::PathBuf;

use crate::db::{self, role};
use crate::model::RunStatus;
use crate::sandbox::{DEFAULT_CPU, DEFAULT_MEMORY_MB};
use crate::services::config::repo_config::{self, HarnessDefaults};
use crate::services::domain::{Agent, AgentId};
use crate::services::egress::model_proxy::{ModelUpstream, Secret, UpstreamAuth};
use crate::services::events::{EventBus, RunContext};
use crate::services::harness::acp::{AcpSessionUpdate, AcpUsage};
use crate::services::harness::adapter::{
    HarnessAdapter, HarnessConfig, ModelRoute, Placement, UserMessage,
};
use crate::services::harness::detection::DetectEnv;
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
    /// How the CLI reaches its model through the egress proxy, and what the
    /// host signs it with ([`claude_code_access`])
    pub model_access: Option<ModelAccess>,
    /// The running LiteLLM gateway as an upstream the proxy signs for
    /// (`GatewayService::session_upstream`), when the agent wants one
    pub gateway: Option<ModelUpstream>,
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

/// How a session reaches its model through the egress proxy: what the CLI
/// is told (no secret) and the upstream the host signs its calls for
#[derive(Debug, Clone, PartialEq)]
pub struct ModelAccess {
    pub route: ModelRoute,
    pub upstream: ModelUpstream,
    /// The AWS region the CLI names its Bedrock models in (`AWS_REGION`)
    pub region: Option<String>,
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
    /// The model session the proxy serves while the run lasts
    /// (`ModelSessions::open`); `None`: the CLI signs in another way
    pub upstream: Option<ModelUpstream>,
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
    let placement = Placement {
        image: HARNESS_IMAGE.to_string(),
        name: container_name(agent.id().as_str()),
        host_workdir: inputs.workdir.clone(),
        // The native runtime's limits when skill.md sets none
        memory_mb: agent.container().memory_mb().unwrap_or(DEFAULT_MEMORY_MB),
        cpu: agent.container().cpu().unwrap_or(DEFAULT_CPU),
        // Internal: its way out is the egress relay (`HTTPS_PROXY` below)
        network: crate::services::egress::relay::AGENT_NETWORK.to_string(),
        folders: inputs.folders.clone(),
        tools,
    };
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
    // Gemini CLI has no proxy route yet: its credentials would go in
    if harness_id == "gemini-cli" {
        return fallback(crate::services::harness::gemini_cli::UNSUPPORTED.to_string());
    }
    let through_gateway = config.gateway.is_some();
    let model_access = if through_gateway {
        let Some(upstream) = inputs.gateway else {
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
        // The proxy fronts the gateway with its key; the gateway holds
        // the AWS identity
        Some(ModelAccess {
            route: ModelRoute::Anthropic,
            upstream,
            region: None,
        })
    } else {
        inputs.model_access
    };
    if inputs.host_only_sign_in && model_access.is_none() {
        return fallback(
            "Claude Code has nothing to sign in with: sign in to Claude Code on this computer, \
             or add an Anthropic key in Settings › Models & runtimes."
                .to_string(),
        );
    }
    let provider = provider(agent, None)
        .or_else(|| repo.provider.clone().filter(|p| p != REPO_GATEWAY))
        .or_else(|| {
            let bedrock = model_access
                .as_ref()
                .is_some_and(|access| access.route == ModelRoute::Bedrock);
            bedrock.then(|| "bedrock".to_string())
        });
    // The model the CLI really uses: through the gateway, its Bedrock model
    let named = if through_gateway {
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
    if harness_id == "codex" && model_access.is_none() {
        return fallback(
            "Codex has no OpenAI key to sign in with: add one in Settings › Models & runtimes. \
             A ChatGPT login can't be used in the container."
                .to_string(),
        );
    }
    let egress_hosts = if model_access.is_some() {
        // Its model calls go to the proxy, which is always reachable
        Vec::new()
    } else {
        model_hosts(&harness_id)
    };
    let (model_route, upstream) = match model_access {
        Some(access) => {
            if let Some(region) = access.region {
                env.insert("AWS_REGION".to_string(), region);
            }
            (Some(access.route), Some(access.upstream))
        }
        None => (None, None),
    };
    let grant = inputs.tools.unwrap_or_else(|| ToolGrant::of(agent));
    RuntimeDecision::Harness(Box::new(HarnessPlan {
        egress_hosts,
        upstream,
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
            model_route,
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

/// Where a CLI's own model calls go when the proxy doesn't serve them:
/// allowed through the egress proxy on top of the agent's `network.allow`
fn model_hosts(harness_id: &str) -> Vec<String> {
    let hosts: &[&str] = match harness_id {
        // Vertex from Claude Code's own settings
        "claude-code" => &["api.anthropic.com"],
        // A custom CLI gets what its skill.md allows
        _ => &[],
    };
    hosts.iter().map(|h| h.to_string()).collect()
}

/// What a run takes from the agent's own setup (its workdir, its policies,
/// its proxy identity), on top of what the machine has
struct AgentSetup {
    workdir: PathBuf,
    folders: Vec<crate::runtime::FolderMount>,
    folder_prompt: String,
    /// Who the egress proxy token is issued to (`ProxyTokens::issue`)
    proxy_identity: String,
    tools: Option<ToolGrant>,
    model_rules: Vec<(Vec<String>, String)>,
}

/// Gathers `plan`'s inputs for `agent` and decides. Starts the gateway for
/// an agent that needs it.
pub async fn resolve_runtime(app: &AppState, agent: &Agent) -> RuntimeDecision {
    if agent.runtime().is_native() {
        return RuntimeDecision::Native;
    }
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
    // skill.md narrowed by its policies (folders, tools); a broken policy
    // or a refused model stops it
    let skill = match app.store().skill(agent.id().as_str()) {
        Ok(skill) => skill,
        Err(e) => return RuntimeDecision::Unavailable(e),
    };
    let shared = &skill.folders;
    let setup = AgentSetup {
        workdir,
        folders: crate::runtime::folder_mounts(shared),
        folder_prompt: crate::services::connectors::folders::prompt(
            shared,
            crate::runtime::FOLDERS_MOUNT,
        ),
        proxy_identity: agent.id().as_str().to_string(),
        tools: Some(ToolGrant {
            allowed: skill.allowed_tools.clone(),
            denied: skill.denied_tools.clone(),
            approval: skill.requires_approval.clone(),
        }),
        model_rules: skill.model_rules.clone(),
    };
    match inputs_for(app, agent, setup).await {
        Ok(inputs) => plan(agent, inputs),
        Err(reason) => RuntimeDecision::Unavailable(reason),
    }
}

/// The agent ID the "Test harness" session plans as. Only its runtime is
/// read: the session gets its own container name and proxy identity.
const TEST_AGENT_ID: &str = "harness-test";

/// How long the "Test harness" session waits for each ACP answer
const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// The throwaway agent the "Test harness" session plans as: `harness_id`'s
/// runtime and nothing else (no folders, no `network.allow`, no tool limits)
fn test_agent(harness_id: &str) -> Result<Agent, String> {
    let id = AgentId::parse(TEST_AGENT_ID).map_err(|e| e.to_string())?;
    Agent::from_skill_md(
        id,
        &format!("---\nname: Test harness\nruntime: {harness_id}\n---\n"),
    )
    .map_err(|e| e.to_string())
}

/// Settings' "Test harness" session for `harness_id`, planned the way a run
/// is (`inputs_for`, then [`test_session_plan`]). Its proxy identity is
/// `harness::test_identity`, which reaches only the CLI's model hosts, and
/// only while the session is open (`HarnessService::test`).
pub async fn plan_test_session(app: &AppState, harness_id: &str) -> Result<HarnessPlan, String> {
    let agent = test_agent(harness_id)?;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let workdir = std::env::temp_dir().join(format!("nandi-harness-test-{suffix}"));
    tokio::fs::create_dir_all(&workdir)
        .await
        .map_err(|e| format!("Could not create {}: {e}", workdir.display()))?;
    let setup = AgentSetup {
        workdir: workdir.clone(),
        folders: Vec::new(),
        folder_prompt: String::new(),
        proxy_identity: crate::services::harness::test_identity(harness_id),
        tools: None,
        model_rules: Vec::new(),
    };
    let planned = match inputs_for(app, &agent, setup).await {
        Ok(inputs) => test_session_plan(harness_id, inputs, &suffix),
        Err(reason) => Err(reason),
    };
    if planned.is_err() {
        let _ = tokio::fs::remove_dir_all(&workdir).await;
    }
    planned
}

/// The "Test harness" session given a run's `inputs` (pure): `plan` for a
/// throwaway agent with `harness_id`'s runtime, so it runs in the harness
/// container on the run network, with the egress proxy env, the default
/// limits and the credentials a run would get. Then its own container name
/// (`nandi-harness-test-<suffix>`, never an agent's), one turn and a
/// shorter timeout.
pub fn test_session_plan(
    harness_id: &str,
    inputs: RouterInputs,
    suffix: &str,
) -> Result<HarnessPlan, String> {
    match plan(&test_agent(harness_id)?, inputs) {
        RuntimeDecision::Harness(plan) => {
            let mut plan = *plan;
            plan.config.placement.name = format!("nandi-harness-test-{}", &suffix[..12]);
            plan.config.max_turns = 1;
            plan.config.request_timeout = Some(TEST_TIMEOUT);
            Ok(plan)
        }
        RuntimeDecision::Unavailable(reason) => Err(reason),
        // The test agent sets no fallback, so a harness never plans native
        RuntimeDecision::Native => Err("This runtime runs natively".to_string()),
    }
}

/// `plan`'s inputs: the agent's `setup` and the machine's side (the image,
/// the detected CLI, the default harness, credentials, the gateway).
/// Starts the gateway for an agent that needs it.
async fn inputs_for(
    app: &AppState,
    agent: &Agent,
    setup: AgentSetup,
) -> Result<RouterInputs, String> {
    let harness_id = agent.runtime().as_str().into_owned();
    let config = agent.runtime_config().cloned().unwrap_or_default();
    let service = app.harnesses();
    let detected = service
        .detector()
        .stored_one(&harness_id)
        .await
        .ok()
        .flatten();
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
    let model_access = match (harness_id.as_str(), provider.as_deref()) {
        ("claude-code", _) if config.gateway.is_some() => None,
        ("claude-code", Some("bedrock")) => Some(bedrock_access(app, agent, &repo).await?),
        ("claude-code", provider) => {
            claude_code_access(provider, &repo, &DetectEnv::current(), || {
                crate::services::llm_keys::require_api_key(
                    app.secrets(),
                    crate::llm::Provider::Anthropic,
                )
                .ok()
            })
        }
        ("codex", _) => codex_access(&DetectEnv::current(), || {
            crate::services::llm_keys::require_api_key(app.secrets(), crate::llm::Provider::OpenAi)
                .ok()
        }),
        _ => None,
    };
    let gateway = match &config.gateway {
        Some(_) => {
            let gateway = app.litellm_gateway();
            let bedrock_model = config.model.clone().ok_or_else(|| {
                "runtime_config.model must name the Bedrock model for the gateway".to_string()
            })?;
            if gateway.model().as_deref() != Some(bedrock_model.as_str()) {
                // Only the gateway's own container holds AWS keys
                let (profile, region) = aws_choice(agent, &repo);
                let keys =
                    crate::services::harness::aws_auth::gateway_keys(app, profile, region).await?;
                gateway.start(&bedrock_model, &keys).await?;
            }
            gateway.session_upstream()
        }
        None => None,
    };
    Ok(RouterInputs {
        sandbox: sandbox_ready(&harness_id).await,
        default_model,
        workdir: setup.workdir,
        model_access,
        gateway,
        repo,
        host_only_sign_in: harness_id == "claude-code"
            && !crate::services::harness::detection::container_auth(
                &harness_id,
                &crate::services::harness::detection::DetectEnv::current(),
            ),
        folders: setup.folders,
        proxy_token: app.proxy_tokens().issue(&setup.proxy_identity),
        tools: setup.tools,
        model_rules: setup.model_rules,
        folder_prompt: setup.folder_prompt,
    })
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

/// The AWS profile and region a Bedrock session uses: the agent's, else
/// the repo's (`.nandi/llm.yaml`)
fn aws_choice(agent: &Agent, repo: &HarnessDefaults) -> (Option<String>, Option<String>) {
    let config = agent.runtime_config();
    (
        config
            .and_then(|c| c.aws_profile.clone())
            .or_else(|| repo.aws_profile.clone()),
        config
            .and_then(|c| c.aws_region.clone())
            .or_else(|| repo.aws_region.clone()),
    )
}

/// A Bedrock session's model access: the egress proxy signs its calls on
/// the host as the user's AWS identity (`aws_auth::bedrock_upstream`), so
/// no AWS key enters the container
async fn bedrock_access(
    app: &AppState,
    agent: &Agent,
    repo: &HarnessDefaults,
) -> Result<ModelAccess, String> {
    let (profile, region) = aws_choice(agent, repo);
    let (upstream, region) = app.aws_auth().bedrock_upstream(profile, region).await?;
    Ok(ModelAccess {
        route: ModelRoute::Bedrock,
        upstream,
        region: Some(region),
    })
}

/// How a Claude Code session reaches its model through the egress proxy
/// (pure but for reading the host's Claude Code setup from `env`), first
/// match wins:
/// - the team's Anthropic-compatible gateway (`.nandi/llm.yaml`
///   `kind: litellm` with an `endpoint`), with Claude Code's gateway token
/// - Foundry for `provider: azure`, at the repo's endpoint or Claude Code's
/// - Claude Code's own setup on the host (`claude_code::host_upstream`)
/// - Nandi's Anthropic key (`nandi_key`): keys last
///
/// `None` for Bedrock, which signs in its own way.
pub fn claude_code_access(
    provider: Option<&str>,
    repo: &HarnessDefaults,
    env: &DetectEnv,
    nandi_key: impl FnOnce() -> Option<String>,
) -> Option<ModelAccess> {
    use crate::services::harness::{claude_code, detection};
    let access = |route, upstream| {
        Some(ModelAccess {
            route,
            upstream,
            region: None,
        })
    };
    let own = detection::claude_code_env(env);
    let setting = |name: &str| own.get(name).filter(|v| !v.trim().is_empty()).cloned();
    if provider == Some("bedrock") {
        return None;
    }
    if let Some(endpoint) = repo
        .endpoint
        .clone()
        .filter(|_| repo.provider.as_deref() == Some(REPO_GATEWAY))
    {
        let auth = match (
            setting("ANTHROPIC_AUTH_TOKEN"),
            setting("ANTHROPIC_API_KEY"),
        ) {
            (Some(token), _) => UpstreamAuth::Bearer(Secret::new(token)),
            (None, Some(key)) => UpstreamAuth::ApiKey(Secret::new(key)),
            // A gateway that checks no key still wants one sent
            (None, None) => UpstreamAuth::Bearer(Secret::new("nandi-gateway")),
        };
        return access(
            ModelRoute::Anthropic,
            ModelUpstream::anthropic(endpoint, auth),
        );
    }
    if provider == Some("azure") {
        let endpoint = repo
            .endpoint
            .clone()
            .or_else(|| claude_code::foundry_endpoint(&own))?;
        let key = setting("ANTHROPIC_FOUNDRY_API_KEY")?;
        let upstream = ModelUpstream::anthropic(endpoint, UpstreamAuth::ApiKey(Secret::new(key)));
        return access(ModelRoute::Foundry, upstream);
    }
    if let Some((route, upstream)) = claude_code::host_upstream(env) {
        return access(route, upstream);
    }
    // Bedrock or Vertex in Claude Code's own settings sign in their own way
    if setting("CLAUDE_CODE_USE_BEDROCK").is_some() || setting("CLAUDE_CODE_USE_VERTEX").is_some() {
        return None;
    }
    let key = nandi_key()?;
    access(
        ModelRoute::Anthropic,
        ModelUpstream::anthropic(
            ModelUpstream::ANTHROPIC_URL,
            UpstreamAuth::ApiKey(Secret::new(key)),
        ),
    )
}

/// How a Codex session reaches OpenAI through the egress proxy: Nandi's
/// OpenAI key, else one in Nandi's environment (`codex::host_upstream`)
pub fn codex_access(
    env: &DetectEnv,
    nandi_key: impl FnOnce() -> Option<String>,
) -> Option<ModelAccess> {
    let upstream = match nandi_key() {
        Some(key) => ModelUpstream::openai(UpstreamAuth::Bearer(Secret::new(key))),
        None => crate::services::harness::codex::host_upstream(env)?,
    };
    Some(ModelAccess {
        route: ModelRoute::OpenAi,
        upstream,
        region: None,
    })
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

/// Takes a session's model hosts back when it ends, however it ends
pub(crate) struct SessionHostsGuard(pub(crate) String);

impl Drop for SessionHostsGuard {
    fn drop(&mut self) {
        crate::services::egress::clear_session_hosts(&self.0);
    }
}

/// Opens the plan's model session on the egress proxy for `identity`,
/// until the guard drops
pub(crate) fn open_model_session(
    identity: &str,
    plan: &HarnessPlan,
) -> Option<crate::services::egress::model_proxy::ModelSessionGuard> {
    let sessions = crate::services::egress::model_proxy::ModelSessions::global();
    plan.upstream
        .clone()
        .map(|upstream| sessions.open(identity, upstream))
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
    // Its model calls, signed on the host, for this run only
    let _model_session = open_model_session(&ctx.agent_id, plan);
    let mut config = plan.config.clone();
    config.approvals.asker = Some(std::sync::Arc::new(
        crate::services::harness::approvals::RunApprover::new(bus.clone(), ctx.clone(), ended),
    ));
    let container_name = Some(plan.config.placement.name.clone());
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
    use crate::services::domain::AgentRuntimeKind;
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
        let Placement {
            name,
            host_workdir,
            network,
            ..
        } = &plan.config.placement;
        assert_eq!(name, &container_name("writer"));
        assert_eq!(host_workdir, &PathBuf::from("/work/writer"));
        // Always the internal network, gateway or not
        assert_eq!(network, "nandi-agents");
    }

    #[test]
    fn the_test_session_is_planned_like_a_run_in_its_own_container() {
        for harness_id in ["claude-code", "codex"] {
            let mut inputs = inputs();
            inputs.proxy_token = "test-token".to_string();
            inputs.model_access = Some(anthropic_key("sk-ant"));
            let plan = test_session_plan(harness_id, inputs, "0123456789abcdef").unwrap();
            let placement = &plan.config.placement;
            assert_eq!(placement.network, "nandi-agents", "{harness_id}");
            assert_eq!(
                (placement.memory_mb, placement.cpu),
                (DEFAULT_MEMORY_MB, DEFAULT_CPU),
                "{harness_id}"
            );
            assert_eq!(
                placement.name, "nandi-harness-test-0123456789ab",
                "{harness_id}: never an agent's container"
            );
            assert!(placement.folders.is_empty(), "{harness_id}");
            assert!(
                plan.config.env["HTTPS_PROXY"].contains("test-token"),
                "{harness_id}: through the egress proxy"
            );
            assert_eq!(plan.config.max_turns, 1, "{harness_id}");
            assert_eq!(plan.config.request_timeout, Some(TEST_TIMEOUT));
        }
    }

    #[test]
    fn a_test_session_that_cannot_start_says_why() {
        let mut inputs = inputs();
        inputs.sandbox = Sandbox::Unavailable("Docker isn't running".to_string());
        assert_eq!(
            test_session_plan("claude-code", inputs, "0123456789abcdef").unwrap_err(),
            "Docker isn't running"
        );
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
            let Placement { memory_mb, cpu, .. } = &plan.config.placement;
            assert_eq!((*memory_mb, *cpu), expected, "{limits:?}");
        }
    }

    #[test]
    fn the_container_gets_the_providers_env_and_nothing_else() {
        let agent =
            agent("runtime: claude-code\nruntime_config:\n  model: us.anthropic.claude-sonnet-5\n");
        let mut inputs = inputs();
        inputs.proxy_token = "tok".into();
        inputs.model_access = Some(ModelAccess {
            route: ModelRoute::Bedrock,
            upstream: ModelUpstream::bedrock(
                "us-west-2",
                UpstreamAuth::Bearer(Secret::new("bedrock-key")),
            ),
            region: Some("us-west-2".into()),
        });
        let plan = harness_plan(plan(&agent, inputs));
        assert_eq!(plan.provider.as_deref(), Some("bedrock"));
        assert!(plan.egress_hosts.is_empty(), "only the proxy");
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
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "tok");
        assert_eq!(env["ANTHROPIC_MODEL"], "us.anthropic.claude-sonnet-5");
        assert!(!env.values().any(|v| v.contains("bedrock-key")));
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

    /// A host where Claude Code has `settings` and nothing else
    fn claude_code_host(settings: Option<&str>) -> (DetectEnv, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        if let Some(settings) = settings {
            std::fs::create_dir_all(home.path().join(".claude")).unwrap();
            std::fs::write(home.path().join(".claude/settings.json"), settings).unwrap();
        }
        let env = DetectEnv {
            home: home.path().to_path_buf(),
            path: Some(home.path().as_os_str().to_owned()),
            vars: HashMap::new(),
        };
        (env, home)
    }

    fn anthropic_key(key: &str) -> ModelAccess {
        ModelAccess {
            route: ModelRoute::Anthropic,
            upstream: ModelUpstream::anthropic(
                ModelUpstream::ANTHROPIC_URL,
                UpstreamAuth::ApiKey(Secret::new(key)),
            ),
            region: None,
        }
    }

    #[test]
    fn a_repo_litellm_gateway_is_signed_for_on_the_host() {
        let repo = HarnessDefaults {
            provider: Some("litellm".into()),
            endpoint: Some("https://llm.acme.internal".into()),
            ..HarnessDefaults::default()
        };
        let (host, _home) = claude_code_host(Some(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-team"}}"#));
        let access = claude_code_access(None, &repo, &host, || None).unwrap();
        assert_eq!(
            access,
            ModelAccess {
                route: ModelRoute::Anthropic,
                upstream: ModelUpstream::anthropic(
                    "https://llm.acme.internal",
                    UpstreamAuth::Bearer(Secret::new("sk-team")),
                ),
                region: None,
            }
        );

        let mut inputs = inputs();
        inputs.repo = repo;
        inputs.model_access = Some(access.clone());
        let plan = harness_plan(plan(&agent("runtime: claude-code\n"), inputs));
        assert_eq!(plan.config.model_route, Some(ModelRoute::Anthropic));
        assert_eq!(plan.upstream, Some(access.upstream));
        assert!(plan.egress_hosts.is_empty(), "only the proxy");
        assert_eq!(plan.provider, None, "the gateway owns the provider");
    }

    #[test]
    fn claude_codes_own_setup_comes_before_nandis_key() {
        let none = HarnessDefaults::default();
        let (keyed, _a) = claude_code_host(Some(r#"{"env":{"ANTHROPIC_API_KEY":"sk-own"}}"#));
        assert_eq!(
            claude_code_access(None, &none, &keyed, || Some("sk-nandi".into())),
            Some(anthropic_key("sk-own"))
        );
        let (bare, _b) = claude_code_host(None);
        assert_eq!(
            claude_code_access(Some("anthropic"), &none, &bare, || Some("sk-nandi".into())),
            Some(anthropic_key("sk-nandi"))
        );
        assert_eq!(claude_code_access(None, &none, &bare, || None), None);
        // Bedrock signs in its own way
        assert_eq!(
            claude_code_access(Some("bedrock"), &none, &keyed, || Some("sk-nandi".into())),
            None
        );
        let (bedrock, _c) = claude_code_host(Some(r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#));
        assert_eq!(
            claude_code_access(None, &none, &bedrock, || Some("sk-nandi".into())),
            None
        );
    }

    #[test]
    fn azure_is_foundry_at_the_repos_endpoint() {
        let repo = HarnessDefaults {
            provider: Some("azure".into()),
            endpoint: Some("https://team.services.ai.azure.com/anthropic".into()),
            ..HarnessDefaults::default()
        };
        let (host, _home) =
            claude_code_host(Some(r#"{"env":{"ANTHROPIC_FOUNDRY_API_KEY":"az-key"}}"#));
        assert_eq!(
            claude_code_access(Some("azure"), &repo, &host, || None),
            Some(ModelAccess {
                route: ModelRoute::Foundry,
                upstream: ModelUpstream::anthropic(
                    "https://team.services.ai.azure.com/anthropic",
                    UpstreamAuth::ApiKey(Secret::new("az-key")),
                ),
                region: None,
            })
        );
    }

    #[test]
    fn a_gateway_agent_uses_the_alias_and_the_proxy_keeps_the_master_key() {
        let agent = agent(
            "runtime: claude-code\nruntime_config:\n  model: meta.llama3-3-70b-instruct-v1:0\n  gateway: litellm\n",
        );
        assert!(matches!(
            plan(&agent, inputs()),
            RuntimeDecision::Unavailable(reason) if reason.contains("gateway")
        ));
        let mut inputs = inputs();
        let upstream = ModelUpstream::anthropic(
            "http://127.0.0.1:4000",
            UpstreamAuth::Bearer(Secret::new("sk-nandi-master")),
        );
        inputs.gateway = Some(upstream.clone());
        let plan = harness_plan(plan(&agent, inputs));
        assert_eq!(plan.model.as_deref(), Some(gateway::MODEL_ALIAS));
        assert_eq!(plan.provider.as_deref(), Some("bedrock"));
        assert_eq!(plan.config.model_route, Some(ModelRoute::Anthropic));
        assert_eq!(plan.upstream, Some(upstream), "the host signs for it");
        assert!(!plan
            .config
            .env
            .values()
            .any(|v| v.contains("sk-nandi-master")));
        assert_eq!(plan.config.placement.network, "nandi-agents");
        assert!(plan.egress_hosts.is_empty(), "only the proxy");
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
            kinds("runtime: custom:mine\n"),
            ["delete", "edit", "execute", "move"]
        );
        assert_eq!(
            kinds("runtime: custom:mine\nrequires_approval: [fetch, deploy]\n"),
            ["deploy", "fetch"]
        );
        assert!(kinds("runtime: custom:mine\nrequires_approval: []\n").is_empty());
    }

    #[test]
    fn a_policy_s_tools_reach_the_cli() {
        let mut narrowed = inputs();
        narrowed.tools = Some(ToolGrant {
            allowed: None,
            denied: vec!["shell".into()],
            approval: vec!["fetch".into()],
        });
        let plan = harness_plan(plan(&agent("runtime: custom:mine\n"), narrowed));
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
        keyed.model_access = Some(anthropic_key("sk-ant"));
        let plan = harness_plan(plan(&agent("runtime: claude-code\n"), keyed));
        assert_eq!(plan.config.placement.network, "nandi-agents");
        assert_eq!(
            plan.config.env["HTTPS_PROXY"],
            "http://nandi:tok@nandi-egress:11435"
        );
        assert!(plan.egress_hosts.is_empty(), "its model is the proxy's");

        let mut keyed = inputs();
        keyed.model_access = codex_access(&DetectEnv::default(), || Some("sk-openai".into()));
        let codex = harness_plan(super::plan(&agent("runtime: codex\n"), keyed));
        assert_eq!(codex.config.model_route, Some(ModelRoute::OpenAi));
        assert!(codex.egress_hosts.is_empty(), "its model is the proxy's");
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
            RuntimeDecision::Unavailable(reason) if reason.contains("nothing to sign in with")
        ));
        let lenient = agent("runtime: claude-code\nruntime_config:\n  fallback: native\n");
        assert!(matches!(
            plan(&lenient, host_login.clone()),
            RuntimeDecision::Native
        ));
        // A Nandi key is enough
        let mut keyed = host_login;
        keyed.model_access = Some(anthropic_key("sk-ant"));
        assert!(matches!(plan(&strict, keyed), RuntimeDecision::Harness(_)));
    }

    #[test]
    fn codex_without_a_key_and_gemini_cli_are_refused_with_a_reason() {
        let codex = plan(&agent("runtime: codex\n"), inputs());
        assert!(matches!(
            codex,
            RuntimeDecision::Unavailable(reason) if reason.contains("OpenAI key")
        ));
        let gemini = plan(&agent("runtime: gemini-cli\n"), inputs());
        assert!(matches!(
            gemini,
            RuntimeDecision::Unavailable(reason) if reason.contains("Gemini CLI can't run")
        ));
        let lenient = agent("runtime: gemini-cli\nruntime_config:\n  fallback: native\n");
        assert!(matches!(plan(&lenient, inputs()), RuntimeDecision::Native));
    }

    /// Every credential a machine can hold where a harness could pick it
    /// up, each with a value that names it
    const HOST_SECRETS: &[(&str, &str)] = &[
        ("ANTHROPIC_API_KEY", "secret-anthropic-key"),
        ("ANTHROPIC_AUTH_TOKEN", "secret-anthropic-token"),
        ("CLAUDE_CODE_OAUTH_TOKEN", "secret-setup-token"),
        ("ANTHROPIC_FOUNDRY_API_KEY", "secret-foundry-key"),
        ("AWS_ACCESS_KEY_ID", "secret-aws-access-key"),
        ("AWS_SECRET_ACCESS_KEY", "secret-aws-secret-key"),
        ("AWS_SESSION_TOKEN", "secret-aws-session"),
        ("AWS_BEARER_TOKEN_BEDROCK", "secret-bedrock-key"),
        ("OPENAI_API_KEY", "secret-openai-key"),
        ("CODEX_API_KEY", "secret-codex-key"),
        ("AZURE_OPENAI_API_KEY", "secret-azure-openai-key"),
        ("GEMINI_API_KEY", "secret-gemini-key"),
        ("GOOGLE_API_KEY", "secret-google-key"),
    ];

    /// A host with every credential source set: the environment, Claude
    /// Code's settings and its sign-in
    fn host_with_every_credential() -> (DetectEnv, tempfile::TempDir) {
        let settings = serde_json::json!({
            "env": HOST_SECRETS.iter().cloned().collect::<HashMap<_, _>>()
        });
        let (mut host, home) = claude_code_host(Some(&settings.to_string()));
        let expires = crate::db::now_ms() + 60 * 60 * 1000;
        std::fs::write(
            home.path().join(".claude/.credentials.json"),
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"secret-oauth-access","refreshToken":"secret-oauth-refresh","expiresAt":{expires}}}}}"#
            ),
        )
        .unwrap();
        host.vars = HOST_SECRETS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        (host, home)
    }

    /// An upstream the host signs with `secret`, on `api`'s route
    fn upstream_with(route: ModelRoute, secret: &str) -> ModelAccess {
        let auth = UpstreamAuth::Bearer(Secret::new(secret));
        let upstream = match route {
            ModelRoute::Anthropic | ModelRoute::Foundry => {
                ModelUpstream::anthropic(ModelUpstream::ANTHROPIC_URL, auth)
            }
            ModelRoute::Bedrock => ModelUpstream::bedrock("us-east-1", auth),
            ModelRoute::OpenAi => ModelUpstream::openai(auth),
        };
        ModelAccess {
            route,
            upstream,
            region: (route == ModelRoute::Bedrock).then(|| "us-east-1".to_string()),
        }
    }

    #[test]
    fn no_harness_container_gets_a_secret_whatever_the_host_holds() {
        use crate::services::harness::adapter::{docker_args, CliProfile, Invocation};
        use crate::services::harness::{claude_code, codex, custom, gemini_cli};

        let (host, _home) = host_with_every_credential();
        const PROXY_TOKEN: &str = "proxy-token-of-the-session";
        let planned = |frontmatter: &str, access: Option<ModelAccess>, gateway| {
            let mut inputs = inputs();
            inputs.proxy_token = PROXY_TOKEN.to_string();
            inputs.model_access = access;
            inputs.gateway = gateway;
            harness_plan(plan(&agent(frontmatter), inputs))
        };
        let anthropic_from_host =
            claude_code_access(None, &HarnessDefaults::default(), &host, || {
                Some("secret-nandi-anthropic-key".to_string())
            });
        let litellm = ModelUpstream::anthropic(
            "http://127.0.0.1:4000",
            UpstreamAuth::Bearer(Secret::new("secret-litellm-master")),
        );
        let gateway_agent =
            "runtime: claude-code\nruntime_config:\n  model: meta.llama3\n  gateway: litellm\n";
        let claude = claude_code::ClaudeCodeAdapter::with_env(host.clone());
        let gemini = gemini_cli::GeminiCliAdapter::with_env(host.clone());
        let codex = codex::CodexAdapter::create();
        let custom = custom::CustomAdapter::with_command(Invocation::new("my-agent", &["--acp"]));
        let sessions: Vec<(&str, &dyn CliProfile, HarnessPlan)> = vec![
            (
                "Claude Code, the host's own setup",
                claude.profile(),
                planned("runtime: claude-code\n", anthropic_from_host, None),
            ),
            (
                "Claude Code, a subscription",
                claude.profile(),
                planned(
                    "runtime: claude-code\n",
                    Some(upstream_with(ModelRoute::Anthropic, "secret-subscription")),
                    None,
                ),
            ),
            (
                "Claude Code, Foundry",
                claude.profile(),
                planned(
                    "runtime: claude-code\n",
                    Some(upstream_with(
                        ModelRoute::Foundry,
                        "secret-foundry-upstream",
                    )),
                    None,
                ),
            ),
            (
                "Claude Code, Bedrock",
                claude.profile(),
                planned(
                    "runtime: claude-code\n",
                    Some(upstream_with(
                        ModelRoute::Bedrock,
                        "secret-bedrock-upstream",
                    )),
                    None,
                ),
            ),
            (
                "Claude Code, the LiteLLM gateway",
                claude.profile(),
                planned(gateway_agent, None, Some(litellm)),
            ),
            (
                "Codex",
                codex.profile(),
                planned(
                    "runtime: codex\n",
                    Some(upstream_with(ModelRoute::OpenAi, "secret-openai-upstream")),
                    None,
                ),
            ),
            (
                "Gemini CLI (refused, but never with a key)",
                gemini.profile(),
                planned("runtime: custom:mine\n", None, None),
            ),
            (
                "a custom CLI",
                custom.profile(),
                planned("runtime: custom:mine\n", None, None),
            ),
        ];

        for (session, profile, plan) in sessions {
            let env = profile.env(&plan.config);
            let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
            let args = docker_args(
                &profile.invocation(plan.model.as_deref()),
                &names,
                &plan.config.placement,
            );
            let printed = format!("{:?} {:?}", plan.config, plan.upstream);
            let everything = env
                .iter()
                .map(|(_, v)| v.as_str())
                .chain(args.iter().map(String::as_str))
                .chain(std::iter::once(printed.as_str()));
            for text in everything {
                assert!(!text.contains("secret-"), "{session}: {text}");
            }
            // A provider key variable, if any, carries the proxy token
            for (name, value) in &env {
                if HOST_SECRETS.iter().any(|(secret, _)| secret == name) {
                    assert_eq!(value, PROXY_TOKEN, "{session}: {name}");
                }
            }
        }
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
        // Live CLIs reach their providers directly, not through a proxy
        plan.config.placement.network = crate::sandbox::BRIDGE_NETWORK.to_string();
        plan.config.placement.name = format!("nandi-harness-live-{harness_id}");
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
        assert_eq!(plan.config.placement.tools, vec![bundle.mount()]);
        let args = crate::services::harness::adapter::docker_args(
            &crate::services::harness::adapter::Invocation::new("claude-agent-acp", &[]),
            &[],
            &plan.config.placement,
        );
        assert!(args.contains(&format!(
            "type=volume,source={},target=/opt/nandi-tools/claude-code,readonly",
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
        assert!(
            plan.config.placement.tools.is_empty(),
            "Codex mounts nothing extra"
        );
        let adapter = CodexAdapter::create();
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
