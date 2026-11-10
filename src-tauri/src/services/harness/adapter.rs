//! The harness adapter contract (wave B, frozen): one adapter instance per
//! live session. `start` spawns the CLI and opens an ACP session; `send`
//! prompts it and streams `AcpSessionUpdate`s until the turn finishes.
//!
//! Credentials travel only in `HarnessCredentials`, which is neither
//! `Serialize` nor `Debug`-printable: they reach the CLI's environment and
//! nowhere else (no logs, no config files, no events).

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::mpsc;

use super::acp::AcpSessionUpdate;
use crate::model::Timestamp;

/// Turn updates from `HarnessAdapter::send`, ending with `TurnFinished` or
/// `Error` (then the channel closes)
pub type EventStream = mpsc::Receiver<AcpSessionUpdate>;

#[async_trait]
pub trait HarnessAdapter: Send + Sync {
    /// Spawns the CLI, runs the ACP handshake, and opens a session
    async fn start(&self, config: HarnessConfig) -> Result<HarnessSession, HarnessError>;
    /// Prompts the open session; the stream ends when the turn does
    async fn send(&mut self, message: UserMessage) -> Result<EventStream, HarnessError>;
    /// Cancels the turn in flight (ACP `session/cancel`); the session stays
    async fn cancel(&mut self) -> Result<(), HarnessError>;
    /// Ends the session and the child process. Boxed so adapters stay
    /// object-safe (`Box<dyn HarnessAdapter>`)
    async fn stop(self: Box<Self>) -> Result<(), HarnessError>;
    /// Whether the CLI is installed and authenticated, without a session
    async fn health(&self) -> Result<HarnessHealth, HarnessError>;
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn supported_models(&self) -> Vec<ModelRef>;
}

/// A model a harness can run, as the harness names it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    /// What the CLI takes, e.g. `claude-sonnet-5` or a Bedrock model ID
    pub id: String,
    pub display_name: String,
    /// `anthropic`, `bedrock`, `azure`, `vertex`, `openai`, `google`
    pub provider: String,
}

impl ModelRef {
    pub fn new(id: &str, display_name: &str, provider: &str) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            provider: provider.to_string(),
        }
    }
}

/// One prompt to the harness
#[derive(Debug, Clone, PartialEq)]
pub struct UserMessage {
    pub text: String,
}

impl UserMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

/// Everything a session needs. Holds credentials: never serialized, and
/// `Debug` redacts them.
#[derive(Clone, Default)]
pub struct HarnessConfig {
    /// The CLI's cwd (inside the container: the mounted workdir)
    pub working_dir: PathBuf,
    /// Non-secret environment for the CLI (provider switches, model)
    pub env: HashMap<String, String>,
    pub mcp_servers: Vec<McpServerRef>,
    pub max_turns: u32,
    /// Empty: every tool the harness has
    pub allowed_tools: Vec<String>,
    /// Kinds a policy denies, whatever `allowed_tools` says
    pub denied_tools: Vec<String>,
    /// Calls a person approves first (`requires_approval`)
    pub approvals: super::acp::Approvals,
    pub model: Option<String>,
    pub gateway: Option<GatewayConfig>,
    pub credentials: HarnessCredentials,
    /// The system prompt (skill body + instructions), appended to the
    /// harness's own
    pub system_prompt: Option<String>,
    /// Where the CLI runs
    pub placement: Placement,
    /// Per-request ACP timeout (`None`: `acp::DEFAULT_TIMEOUT`)
    pub request_timeout: Option<Duration>,
}

impl fmt::Debug for HarnessConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HarnessConfig")
            .field("working_dir", &self.working_dir)
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("mcp_servers", &self.mcp_servers)
            .field("max_turns", &self.max_turns)
            .field("allowed_tools", &self.allowed_tools)
            .field("denied_tools", &self.denied_tools)
            .field("approvals", &self.approvals)
            .field("model", &self.model)
            .field("gateway", &self.gateway)
            .field("credentials", &self.credentials)
            .field("placement", &self.placement)
            .finish()
    }
}

/// Where a harness CLI runs
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Placement {
    /// `docker run -i <image>` with the CLI as the entrypoint's child
    Container {
        image: String,
        /// Container name (for `docker kill`)
        name: String,
        /// Host directory mounted at `/agent/workdir`
        host_workdir: PathBuf,
        memory_mb: Option<u32>,
        cpu: Option<f32>,
        /// A Docker network to join (`nandi-harness`, to reach the gateway)
        network: Option<String>,
        /// The agent's shared folders (`runtime::folder_mounts`)
        folders: Vec<crate::runtime::FolderMount>,
        /// Tool volumes fetched on first use (`images::tools`), read-only
        tools: Vec<crate::services::images::tools::VolumeMount>,
    },
    /// Directly on the host (health checks, explicit opt-in)
    #[default]
    Host,
}

/// An MCP server the harness should connect to (ACP `session/new`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "transport")]
pub enum McpServerRef {
    /// Spawned by the CLI (inside the container, for the MCP bridge)
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        /// Non-secret
        env: Vec<(String, String)>,
    },
    Http {
        name: String,
        url: String,
        /// e.g. `Authorization`; values are per-session tokens
        headers: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    pub kind: GatewayKind,
    /// `None` with `Litellm`: the session's own sidecar in the container
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum GatewayKind {
    /// A LiteLLM proxy translating Anthropic Messages to Bedrock Converse
    Litellm,
    /// Any Anthropic-compatible endpoint the user runs
    External,
}

/// Secrets for the CLI's environment only. Read from the keyring or the
/// host environment; never logged, never written to disk.
#[derive(Clone, Default, PartialEq)]
pub struct HarnessCredentials {
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub aws_profile: Option<String>,
    pub aws_region: Option<String>,
    /// Bedrock API key (`AWS_BEARER_TOKEN_BEDROCK`)
    pub aws_bearer_token: Option<String>,
    /// Temporary STS credentials resolved on the host (SSO or the
    /// standard chain), so the container never sees `~/.aws`
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub azure_endpoint: Option<String>,
    pub azure_api_key: Option<String>,
    pub azure_tenant_id: Option<String>,
    pub azure_client_id: Option<String>,
    pub custom_env: HashMap<String, String>,
}

impl HarnessCredentials {
    /// Names (never values) of the credentials that are set
    pub fn present(&self) -> Vec<&'static str> {
        let fields: [(&'static str, bool); 13] = [
            ("anthropic_api_key", self.anthropic_api_key.is_some()),
            ("openai_api_key", self.openai_api_key.is_some()),
            ("google_api_key", self.google_api_key.is_some()),
            ("aws_profile", self.aws_profile.is_some()),
            ("aws_region", self.aws_region.is_some()),
            ("aws_bearer_token", self.aws_bearer_token.is_some()),
            ("aws_access_key_id", self.aws_access_key_id.is_some()),
            (
                "aws_secret_access_key",
                self.aws_secret_access_key.is_some(),
            ),
            ("aws_session_token", self.aws_session_token.is_some()),
            ("azure_endpoint", self.azure_endpoint.is_some()),
            ("azure_api_key", self.azure_api_key.is_some()),
            ("azure_tenant_id", self.azure_tenant_id.is_some()),
            ("azure_client_id", self.azure_client_id.is_some()),
        ];
        fields
            .into_iter()
            .filter(|(_, set)| *set)
            .map(|(name, _)| name)
            .collect()
    }
}

impl fmt::Debug for HarnessCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HarnessCredentials")
            .field("present", &self.present())
            .field(
                "custom_env_keys",
                &self.custom_env.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// A live session, as the UI sees it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSession {
    pub id: String,
    pub adapter_id: String,
    pub child_pid: u32,
    pub started_at: Timestamp,
    #[specta(type = String)]
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessHealth {
    pub installed: bool,
    pub version: Option<String>,
    /// Signed in for sessions on the host (its own login counts)
    pub host_authenticated: bool,
    /// Sessions in the harness container can sign in: the host's login
    /// files stay on the host, so this needs a key, a provider, or a
    /// forwardable token
    pub container_authenticated: bool,
    /// `anthropic`, `bedrock`, `vertex`, `azure`, `openai`, `google`
    pub configured_provider: Option<String>,
    /// How the CLI signs in, as far as its own setup shows (not stored:
    /// re-read on load, like `container_authenticated`)
    pub auth: Option<HarnessAuth>,
    pub last_check: Timestamp,
    pub error: Option<String>,
}

/// What a harness CLI signs in with, from its own config and environment.
/// Never holds a secret.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessAuth {
    pub kind: HarnessAuthKind,
    /// The subscription plan, e.g. `max`, `team`, `enterprise`
    pub plan: Option<String>,
    /// The organization the sign-in belongs to
    pub organization: Option<String>,
    /// Bedrock: the AWS profile (SSO, assume-role, keys…) and region
    pub aws_profile: Option<String>,
    pub aws_region: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum HarnessAuthKind {
    /// The CLI's own account login (Claude Pro/Max/Team/Enterprise, a
    /// ChatGPT or Google account)
    Subscription,
    /// AWS Bedrock with the user's AWS identity
    Bedrock,
    /// Google Vertex AI
    Vertex,
    /// Azure AI Foundry / Azure OpenAI
    Azure,
    /// An Anthropic-compatible gateway (`ANTHROPIC_BASE_URL`)
    Gateway,
    /// A provider API key
    ApiKey,
    #[default]
    None,
}

/// How an agent runs, resolved for one run
#[derive(Debug, Clone)]
pub enum AgentRuntime {
    /// Nandi's own loop (`orchestrator::drive_run`)
    Native,
    Harness {
        adapter_id: String,
        config: Box<HarnessConfig>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HarnessRuntime {
    ClaudeCode,
    Codex,
    GeminiCli,
    /// A user-registered ACP executable
    Custom(String),
}

impl HarnessRuntime {
    pub const BUILT_IN: [HarnessRuntime; 3] = [Self::ClaudeCode, Self::Codex, Self::GeminiCli];

    /// `claude-code`, `codex`, `gemini-cli`, `custom:<id>`
    pub fn id(&self) -> String {
        match self {
            Self::ClaudeCode => "claude-code".to_string(),
            Self::Codex => "codex".to_string(),
            Self::GeminiCli => "gemini-cli".to_string(),
            Self::Custom(id) => format!("custom:{id}"),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "claude-code" | "claude_code" | "claude" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "gemini-cli" | "gemini_cli" | "gemini" => Some(Self::GeminiCli),
            other => other
                .strip_prefix("custom:")
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| Self::Custom(id.to_string())),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::ClaudeCode => "Claude Code".to_string(),
            Self::Codex => "Codex".to_string(),
            Self::GeminiCli => "Gemini CLI".to_string(),
            Self::Custom(id) => id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HarnessError {
    /// The CLI (or Docker, for container placement) isn't there
    NotInstalled(String),
    Spawn(String),
    /// The child broke the ACP contract (bad JSON, unexpected reply)
    Protocol(String),
    /// A JSON-RPC error response
    Rpc {
        code: i64,
        message: String,
    },
    Timeout(Duration),
    /// The CLI says it isn't logged in
    Auth(String),
    Cancelled,
    /// The child exited (code if it had one)
    Exited(Option<i32>),
    /// No session is open on this adapter
    NoSession,
    Io(String),
    Other(String),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled(what) => write!(f, "{what} is not installed"),
            Self::Spawn(e) => write!(f, "Could not start the harness: {e}"),
            Self::Protocol(e) => write!(f, "The harness broke the ACP protocol: {e}"),
            Self::Rpc { code, message } => {
                write!(f, "The harness returned an error ({code}): {message}")
            }
            Self::Timeout(after) => {
                write!(f, "The harness did not answer within {}s", after.as_secs())
            }
            Self::Auth(e) => write!(f, "The harness is not signed in: {e}"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Exited(Some(code)) => write!(f, "The harness exited with code {code}"),
            Self::Exited(None) => write!(f, "The harness exited"),
            Self::NoSession => write!(f, "No harness session is open"),
            Self::Io(e) => write!(f, "Harness I/O failed: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for HarnessError {}

impl From<HarnessError> for String {
    fn from(error: HarnessError) -> Self {
        error.to_string()
    }
}

impl From<std::io::Error> for HarnessError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

// ---------------------------------------------------------------------------
// Helpers shared by the adapters
// ---------------------------------------------------------------------------

/// The workdir inside the harness container
pub const CONTAINER_WORKDIR: &str = "/agent/workdir";
/// The harness image's home, a tmpfs (the image's `node` user's)
const CONTAINER_HOME: &str = "/home/node";
/// Lines of the child's stderr kept for error messages (never logged)
const STDERR_TAIL_LINES: usize = 20;

/// How to launch a harness CLI that speaks ACP on stdio
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
}

impl Invocation {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }
}

/// A spawned harness process (`docker run -i …` for container placement)
pub struct SpawnedHarness {
    pub child: tokio::process::Child,
    pub pid: u32,
    pub container_name: Option<String>,
    /// The last lines the child wrote to stderr
    pub stderr_tail: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl SpawnedHarness {
    pub fn stderr_summary(&self) -> Option<String> {
        let tail = self.stderr_tail.lock().ok()?;
        (!tail.is_empty()).then(|| tail.join("\n"))
    }
}

/// The `docker run` arguments for container placement. Every environment
/// variable is passed as `-e NAME` (no value): Docker takes the value from
/// its own environment, so secrets never appear in a command line. The
/// only values given are the agent user's `HOME` and `USER`, which aren't
/// secret.
pub fn docker_args(
    invocation: &Invocation,
    env_names: &[&str],
    placement: &Placement,
) -> Vec<String> {
    let Placement::Container {
        image,
        name,
        host_workdir,
        memory_mb,
        cpu,
        network,
        folders,
        tools,
    } = placement
    else {
        return Vec::new();
    };
    // The same hardening as the native runtime container
    let mut args: Vec<String> = [
        "run",
        "-i",
        "--rm",
        "--init",
        "--name",
        name,
        "--label",
        "nandi.harness=1",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pids-limit=512",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // The agent user (the host user's uid on a Linux engine, the image's
    // `node` on Docker Desktop); the CLIs' state lives in a home tmpfs
    let user = crate::runtime::AgentUser::current();
    args.push(format!("--user={}", user.user()));
    args.push("--read-only".to_string());
    args.push("--tmpfs=/tmp:rw,nosuid,nodev,size=512m".to_string());
    args.push(format!(
        "--tmpfs={CONTAINER_HOME}:{}",
        user.home_tmpfs("rw,nosuid,nodev,size=512m")
    ));
    for var in crate::runtime::AgentUser::env(CONTAINER_HOME, "node") {
        args.push("-e".to_string());
        args.push(var);
    }
    args.push("-w".to_string());
    args.push(CONTAINER_WORKDIR.to_string());
    args.push("-v".to_string());
    args.push(format!("{}:{CONTAINER_WORKDIR}", host_workdir.display()));
    for folder in folders {
        args.push("-v".to_string());
        let mode = if folder.read_only { ":ro" } else { "" };
        args.push(format!("{}:{}{mode}", folder.source, folder.target));
    }
    for tool in tools {
        args.push("-v".to_string());
        args.push(format!("{}:{}:ro", tool.volume, tool.target));
    }
    if let Some(memory) = memory_mb {
        args.push(format!("--memory={memory}m"));
    }
    if let Some(cpu) = cpu {
        args.push(format!("--cpus={cpu}"));
    }
    if let Some(network) = network {
        args.push(format!("--network={network}"));
    }
    for name in env_names {
        args.push("-e".to_string());
        args.push(name.to_string());
    }
    args.push(image.to_string());
    args.push(invocation.program.clone());
    args.extend(invocation.args.iter().cloned());
    args
}

/// Spawns `invocation` with `env` (which may hold credentials) where
/// `placement` says, stdio piped. Only the child process gets `env`.
pub fn spawn_with_env(
    invocation: &Invocation,
    env: &[(String, String)],
    placement: &Placement,
    working_dir: &std::path::Path,
) -> Result<SpawnedHarness, HarnessError> {
    use std::process::Stdio;

    let (mut command, container_name) = match placement {
        Placement::Host => {
            let mut command = tokio::process::Command::new(&invocation.program);
            command.args(&invocation.args).current_dir(working_dir);
            (command, None)
        }
        Placement::Container { name, .. } => {
            let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
            let mut command = tokio::process::Command::new("docker");
            command.args(docker_args(invocation, &names, placement));
            (command, Some(name.clone()))
        }
    };
    command
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window flashing up for the child
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => HarnessError::NotInstalled(match placement {
            Placement::Host => invocation.program.clone(),
            Placement::Container { .. } => "Docker".to_string(),
        }),
        _ => HarnessError::Spawn(e.to_string()),
    })?;
    let pid = child.id().unwrap_or_default();
    let stderr_tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    if let Some(stderr) = child.stderr.take() {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut tail) = tail.lock() {
                    tail.push(line);
                    let excess = tail.len().saturating_sub(STDERR_TAIL_LINES);
                    tail.drain(..excess);
                }
            }
        });
    }
    Ok(SpawnedHarness {
        child,
        pid,
        container_name,
        stderr_tail,
    })
}

/// A spawned CLI with its open ACP session: what every adapter holds
/// between `start` and `stop`.
pub struct LiveSession {
    pub client: super::acp::AcpClient,
    /// `None` in tests, where the agent is an in-memory pipe
    pub process: Option<SpawnedHarness>,
    pub session: HarnessSession,
}

impl LiveSession {
    /// Spawns the CLI and opens an ACP session on it. `cwd` is the agent's
    /// view of the workdir (the container path for container placement).
    pub async fn open(
        adapter_id: &str,
        invocation: &Invocation,
        env: &[(String, String)],
        config: &HarnessConfig,
        access: SessionAccess,
    ) -> Result<Self, HarnessError> {
        use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

        let cwd = match &config.placement {
            Placement::Host => config.working_dir.clone(),
            Placement::Container { .. } => PathBuf::from(CONTAINER_WORKDIR),
        };
        // Its network and the relay out of it, or it doesn't start at all
        if let Placement::Container { .. } = &config.placement {
            crate::services::egress::relay::ensure_ready(
                crate::services::egress::relay::RelayImage::Harness,
            )
            .await
            .map_err(HarnessError::Spawn)?;
        }
        let mut process = spawn_with_env(invocation, env, &config.placement, &config.working_dir)?;
        let (Some(stdin), Some(stdout)) = (process.child.stdin.take(), process.child.stdout.take())
        else {
            return Err(HarnessError::Spawn("no stdio".to_string()));
        };
        let options = super::acp::SessionOptions {
            cwd,
            mcp_servers: config.mcp_servers.clone(),
            timeout: config
                .request_timeout
                .unwrap_or(super::acp::DEFAULT_TIMEOUT),
            permissions: access.permissions,
            approvals: access.approvals,
            auth_method: access.auth_method,
        };
        let connected =
            super::acp::AcpClient::connect(stdin.compat_write(), stdout.compat(), options).await;
        let client = match connected {
            Ok(client) => client,
            Err(error) => {
                let detail = process.stderr_summary();
                let _ = process.child.start_kill();
                if let Some(name) = &process.container_name {
                    crate::runtime::stop_container(name).await;
                }
                return Err(with_stderr(error, detail));
            }
        };
        let session = HarnessSession {
            id: uuid::Uuid::new_v4().to_string(),
            adapter_id: adapter_id.to_string(),
            child_pid: process.pid,
            started_at: crate::db::ts(crate::db::now_ms()),
            working_dir: config.working_dir.clone(),
        };
        Ok(Self {
            client,
            process: Some(process),
            session,
        })
    }

    /// `open` over an in-memory agent
    #[cfg(test)]
    pub(crate) async fn open_streams(
        adapter_id: &str,
        agent: super::acp::tests::ClientEnds,
        config: &HarnessConfig,
        access: SessionAccess,
    ) -> Result<Self, HarnessError> {
        use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

        let options = super::acp::SessionOptions {
            cwd: config.working_dir.clone(),
            mcp_servers: config.mcp_servers.clone(),
            timeout: config
                .request_timeout
                .unwrap_or(super::acp::DEFAULT_TIMEOUT),
            permissions: access.permissions,
            approvals: access.approvals,
            auth_method: access.auth_method,
        };
        let client = super::acp::AcpClient::connect(
            agent.write.compat_write(),
            agent.read.compat(),
            options,
        )
        .await?;
        Ok(Self {
            client,
            process: None,
            session: HarnessSession {
                id: uuid::Uuid::new_v4().to_string(),
                adapter_id: adapter_id.to_string(),
                child_pid: 0,
                started_at: crate::db::ts(crate::db::now_ms()),
                working_dir: config.working_dir.clone(),
            },
        })
    }

    pub fn send(&self, message: &UserMessage) -> Result<EventStream, HarnessError> {
        if !self.client.is_alive() {
            return Err(HarnessError::Exited(None));
        }
        self.client.prompt(&message.text)
    }

    pub fn cancel(&self) -> Result<(), HarnessError> {
        self.client.cancel()
    }

    /// Closes the session, then the process (and its container)
    pub async fn stop(self) {
        self.client.shutdown().await;
        let Some(mut process) = self.process else {
            return;
        };
        let exited = tokio::time::timeout(Duration::from_secs(5), process.child.wait()).await;
        if exited.is_err() {
            let _ = process.child.start_kill();
        }
        if let Some(name) = &process.container_name {
            crate::runtime::stop_container(name).await;
        }
    }
}

/// Adds what the CLI printed to stderr, which usually says why it failed
/// (not signed in, unknown model)
fn with_stderr(error: HarnessError, stderr: Option<String>) -> HarnessError {
    match (error, stderr) {
        (HarnessError::Exited(code), Some(detail)) => {
            HarnessError::Other(format!("{}: {detail}", HarnessError::Exited(code)))
        }
        (HarnessError::Protocol(e), Some(detail)) => {
            HarnessError::Protocol(format!("{e} ({detail})"))
        }
        (HarnessError::Timeout(after), Some(detail)) => {
            HarnessError::Other(format!("{}: {detail}", HarnessError::Timeout(after)))
        }
        (error, _) => error,
    }
}

/// Opens a session with `config` and closes it: the end-to-end check
/// behind Test connection. Sends no prompt (no tokens spent).
pub async fn test_session(
    adapter: Box<dyn HarnessAdapter>,
    config: HarnessConfig,
) -> Result<String, HarnessError> {
    let working_dir = config.working_dir.clone();
    let started = adapter.start(config).await;
    let stopped = adapter.stop().await;
    let _ = tokio::fs::remove_dir_all(&working_dir).await;
    let session = started?;
    stopped?;
    Ok(format!(
        "Opened and closed an ACP session (pid {})",
        session.child_pid
    ))
}

/// Merges the config's plain env with the credential env (credentials win)
pub fn merge_env(
    plain: &HashMap<String, String>,
    credentials: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut merged: std::collections::BTreeMap<String, String> =
        plain.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    merged.extend(credentials);
    merged.into_iter().collect()
}

/// The first `x.y.z` in `--version` output
pub fn parse_version(output: &str) -> Option<semver::Version> {
    let re = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)(-[0-9A-Za-z.\-]+)?").ok()?;
    let found = re.find(output)?;
    semver::Version::parse(found.as_str()).ok()
}

// ---------------------------------------------------------------------------
// One adapter for every ACP CLI
// ---------------------------------------------------------------------------

/// What differs between harness CLIs: how to launch them and which
/// environment they read. `CliAdapter` does the rest.
pub trait CliProfile: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn supported_models(&self) -> Vec<ModelRef>;
    /// The ACP command for `placement`; `host_version` is the CLI's version
    /// on the host (detected), for invocations that changed over time
    fn invocation(
        &self,
        placement: &Placement,
        host_version: Option<&semver::Version>,
        model: Option<&str>,
    ) -> Invocation;
    /// The CLI's environment: provider switches, model, credentials. Only
    /// the child process sees it.
    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)>;
    /// Installed, version, provider, signed in
    fn health(&self) -> futures_util::future::BoxFuture<'_, HarnessHealth>;
    /// The ACP `authenticate` method for the credentials in `env`, for CLIs
    /// that want one before `session/new`
    fn auth_method(&self, _env: &[(String, String)]) -> Option<String> {
        None
    }
}

/// How a session answers permission requests and authenticates
#[derive(Debug, Clone, Default)]
pub struct SessionAccess {
    pub permissions: super::acp::PermissionPolicy,
    pub approvals: super::acp::Approvals,
    pub auth_method: Option<String>,
}

/// A `HarnessAdapter` over any `CliProfile`: one live session at a time
pub struct CliAdapter<P: CliProfile> {
    profile: P,
    live: tokio::sync::Mutex<Option<LiveSession>>,
    /// Tests: what `start` would have spawned, and the in-memory agent it
    /// talks to instead
    #[cfg(test)]
    pub(crate) test_spawn: std::sync::Mutex<Option<TestSpawn>>,
}

#[cfg(test)]
pub(crate) struct TestSpawn {
    pub agent: Option<super::acp::tests::ClientEnds>,
    pub invocation: Option<Invocation>,
    pub env: Vec<(String, String)>,
}

impl<P: CliProfile> CliAdapter<P> {
    pub fn new(profile: P) -> Self {
        Self {
            profile,
            live: tokio::sync::Mutex::new(None),
            #[cfg(test)]
            test_spawn: std::sync::Mutex::new(None),
        }
    }

    pub fn profile(&self) -> &P {
        &self.profile
    }

    #[cfg(test)]
    async fn open(
        &self,
        invocation: &Invocation,
        env: &[(String, String)],
        config: &HarnessConfig,
        access: SessionAccess,
    ) -> Result<LiveSession, HarnessError> {
        let Some(agent) = self.test_spawn.lock().unwrap().as_mut().and_then(|spawn| {
            spawn.invocation = Some(invocation.clone());
            spawn.env = env.to_vec();
            spawn.agent.take()
        }) else {
            return LiveSession::open(self.profile.id(), invocation, env, config, access).await;
        };
        LiveSession::open_streams(self.profile.id(), agent, config, access).await
    }

    #[cfg(not(test))]
    async fn open(
        &self,
        invocation: &Invocation,
        env: &[(String, String)],
        config: &HarnessConfig,
        access: SessionAccess,
    ) -> Result<LiveSession, HarnessError> {
        LiveSession::open(self.profile.id(), invocation, env, config, access).await
    }
}

#[async_trait]
impl<P: CliProfile> HarnessAdapter for CliAdapter<P> {
    async fn start(&self, config: HarnessConfig) -> Result<HarnessSession, HarnessError> {
        let mut live = self.live.lock().await;
        if live.is_some() {
            return Err(HarnessError::Other(
                "This adapter already has a session".to_string(),
            ));
        }
        let host_version = match config.placement {
            Placement::Host => self
                .profile
                .health()
                .await
                .version
                .and_then(|v| semver::Version::parse(&v).ok()),
            Placement::Container { .. } => None,
        };
        let invocation = self.profile.invocation(
            &config.placement,
            host_version.as_ref(),
            config.model.as_deref(),
        );
        let env = self.profile.env(&config);
        let access = SessionAccess {
            permissions: super::acp::PermissionPolicy::from_tools(
                &config.allowed_tools,
                &config.denied_tools,
            ),
            approvals: config.approvals.clone(),
            auth_method: self.profile.auth_method(&env),
        };
        let session = self.open(&invocation, &env, &config, access).await?;
        let summary = session.session.clone();
        *live = Some(session);
        Ok(summary)
    }

    async fn send(&mut self, message: UserMessage) -> Result<EventStream, HarnessError> {
        let live = self.live.lock().await;
        live.as_ref().ok_or(HarnessError::NoSession)?.send(&message)
    }

    async fn cancel(&mut self) -> Result<(), HarnessError> {
        let live = self.live.lock().await;
        live.as_ref().ok_or(HarnessError::NoSession)?.cancel()
    }

    async fn stop(self: Box<Self>) -> Result<(), HarnessError> {
        if let Some(session) = self.live.lock().await.take() {
            session.stop().await;
        }
        Ok(())
    }

    async fn health(&self) -> Result<HarnessHealth, HarnessError> {
        Ok(self.profile.health().await)
    }

    fn id(&self) -> &'static str {
        self.profile.id()
    }

    fn display_name(&self) -> &'static str {
        self.profile.display_name()
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        self.profile.supported_models()
    }
}

/// Environment entries for the credentials every CLI reads under the same
/// names (`AWS_*`, `custom_env`). Provider-specific ones are the profile's.
pub fn common_credential_env(credentials: &HarnessCredentials) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let mut put = |name: &str, value: &Option<String>| {
        if let Some(value) = value.as_ref().filter(|v| !v.is_empty()) {
            env.push((name.to_string(), value.clone()));
        }
    };
    put("AWS_REGION", &credentials.aws_region);
    put("AWS_ACCESS_KEY_ID", &credentials.aws_access_key_id);
    put("AWS_SECRET_ACCESS_KEY", &credentials.aws_secret_access_key);
    put("AWS_SESSION_TOKEN", &credentials.aws_session_token);
    put("AWS_BEARER_TOKEN_BEDROCK", &credentials.aws_bearer_token);
    // Exported keys make the profile unnecessary (and the container has no
    // ~/.aws to find it in)
    if credentials.aws_access_key_id.is_none() {
        put("AWS_PROFILE", &credentials.aws_profile);
    }
    env.extend(
        credentials
            .custom_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    env
}

/// Copies the `names` that are set in `source` (the CLI's own config or
/// Nandi's environment) into the child's environment
pub fn forward_env(source: &HashMap<String, String>, names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .filter_map(|name| {
            source
                .get(*name)
                .filter(|v| !v.trim().is_empty())
                .map(|v| (name.to_string(), v.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_from_cli_output() {
        let v = |s: &str| parse_version(s).map(|v| v.to_string());
        assert_eq!(v("2.1.301 (Claude Code)").as_deref(), Some("2.1.301"));
        assert_eq!(v("codex-cli 0.163.0").as_deref(), Some("0.163.0"));
        assert_eq!(
            v("0.64.0-nightly.20261001").as_deref(),
            Some("0.64.0-nightly.20261001")
        );
        assert_eq!(v("no version here"), None);
    }

    #[test]
    fn docker_run_names_variables_but_never_their_values() {
        let args = docker_args(
            &Invocation::new("claude-agent-acp", &[]),
            &["ANTHROPIC_API_KEY", "AWS_SESSION_TOKEN"],
            &Placement::Container {
                image: "nandi-harness:latest".into(),
                name: "nandi-harness-writer".into(),
                host_workdir: "/work/writer".into(),
                memory_mb: Some(2048),
                cpu: Some(1.5),
                network: Some("nandi-harness".into()),
                folders: vec![crate::runtime::FolderMount {
                    source: "/home/a/docs".into(),
                    target: "/agent/folders/docs".into(),
                    read_only: true,
                }],
                tools: vec![crate::services::images::tools::VolumeMount {
                    volume: "nandi-tools-claude-code-abc".into(),
                    target: "/opt/nandi-tools/claude-code".into(),
                }],
            },
        );
        let joined = args.join(" ");
        assert!(joined.starts_with("run -i --rm --init --name nandi-harness-writer"));
        assert!(joined.contains("-v nandi-tools-claude-code-abc:/opt/nandi-tools/claude-code:ro"));
        assert!(joined.contains("--cap-drop=ALL --security-opt=no-new-privileges"));
        let user = crate::runtime::AgentUser::current();
        assert!(joined.contains(&format!("--user={} --read-only", user.user())));
        assert!(joined.contains("--tmpfs=/tmp:"));
        assert!(joined.contains(&format!(
            "--tmpfs=/home/node:rw,nosuid,nodev,size=512m,uid={},gid={}",
            user.uid, user.gid
        )));
        assert!(joined.contains("-e HOME=/home/node -e USER=node"));
        assert!(joined.contains("--network=nandi-harness"));
        assert!(joined.contains("-e ANTHROPIC_API_KEY -e AWS_SESSION_TOKEN"));
        assert!(joined.contains("/work/writer:/agent/workdir"));
        assert!(joined.contains("-v /home/a/docs:/agent/folders/docs:ro"));
        assert!(joined.contains("--memory=2048m --cpus=1.5"));
        assert!(joined.ends_with("nandi-harness:latest claude-agent-acp"));
        let passed: Vec<&String> = args
            .windows(2)
            .filter(|pair| pair[0] == "-e")
            .map(|pair| &pair[1])
            .collect();
        assert!(
            passed.iter().all(|name| !name.contains('=')
                || name.starts_with("HOME=")
                || name.starts_with("USER=")),
            "{passed:?}"
        );
    }

    #[test]
    fn credentials_never_print() {
        let credentials = HarnessCredentials {
            anthropic_api_key: Some("sk-ant-secret".into()),
            aws_secret_access_key: Some("aws-secret".into()),
            custom_env: [("TOKEN".to_string(), "custom-secret".to_string())].into(),
            ..HarnessCredentials::default()
        };
        let config = HarnessConfig {
            credentials: credentials.clone(),
            env: [("PLAIN".to_string(), "value-secret".to_string())].into(),
            ..HarnessConfig::default()
        };
        let printed = format!("{credentials:?} {config:?}");
        for secret in [
            "sk-ant-secret",
            "aws-secret",
            "custom-secret",
            "value-secret",
        ] {
            assert!(!printed.contains(secret), "{printed}");
        }
        assert!(printed.contains("anthropic_api_key"), "names are fine");
    }

    #[test]
    fn exported_aws_keys_replace_the_profile() {
        let env = common_credential_env(&HarnessCredentials {
            aws_profile: Some("work".into()),
            aws_access_key_id: Some("AKIA".into()),
            aws_secret_access_key: Some("s".into()),
            ..HarnessCredentials::default()
        });
        let names: Vec<_> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"]);
        let env = common_credential_env(&HarnessCredentials {
            aws_profile: Some("work".into()),
            ..HarnessCredentials::default()
        });
        assert_eq!(env, vec![("AWS_PROFILE".to_string(), "work".to_string())]);
    }

    #[tokio::test]
    async fn a_missing_program_is_not_installed() {
        let result = spawn_with_env(
            &Invocation::new("definitely-not-a-real-harness-cli", &[]),
            &[],
            &Placement::Host,
            &std::env::temp_dir(),
        );
        assert!(matches!(result, Err(HarnessError::NotInstalled(_))));
    }
}
