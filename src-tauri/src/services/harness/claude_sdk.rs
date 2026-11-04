//! Claude Code through the Claude Agent SDK (`@anthropic-ai/claude-agent-sdk`).
//!
//! Nandi's own LLM calls (Studio, chat, compaction, native agents' turns)
//! for `claude:` models, signed in the way Claude Code is: the user's
//! `claude` executable answers through the SDK, so a company's Claude
//! login, or its Bedrock identity, works with no API key in Nandi.
//!
//! A completion here is a plain model call, like an HTTPS request to a
//! provider: no tools, no MCP servers, no hooks, no settings or project
//! files, one turn. Nothing an agent wrote runs on the host; agents'
//! code runs only in the sandbox container (`runtime: claude-code` agents
//! too, over ACP: `claude_code`).
//!
//! The SDK is a Node package, so Nandi runs a small bridge script
//! (`runtimes/claude-sdk/bridge.mjs`, embedded here) under the user's Node.
//! It is installed on first use into `~/.nandi/local/runtimes/claude-sdk`
//! (`npm install --omit=optional`: the SDK's bundled CLI is skipped, the
//! user's `claude` is the one that runs).
//!
//! The bridge speaks JSON lines on stdio: a config line in, then
//! `AcpSessionUpdate`-shaped lines out (text deltas, then `turnFinished`).
//! `llm::claude_code` builds the request on top of `complete`.
//!
//! Sign-in, in order (API keys last: Nandi is for teams, whose identity is
//! their company's):
//! 1. The team's Bedrock choice (`.nandi/llm.yaml` `provider: bedrock`, its
//!    `aws_profile` and `aws_region`): Claude Code runs on Bedrock with
//!    `CLAUDE_CODE_USE_BEDROCK=1` and that profile. Claude Code resolves it
//!    on the host from `~/.aws` itself: SSO (`aws sso login`), SAML or
//!    `credential_process` helpers, assume-role, or keys.
//! 2. Claude Code's own setup: its account login (Pro, Max, Team,
//!    Enterprise), or the Bedrock, Vertex, or Foundry switches in its
//!    settings (including ones IT manages).
//! 3. An API key, only through Claude Code's own environment.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::ChildStdout;
use tokio::sync::mpsc;

use super::acp::{AcpSessionUpdate, AcpUsage};
use super::adapter::{spawn_with_env, HarnessError, Invocation, Placement, SpawnedHarness};
use super::detection::{self, DetectEnv};

/// The SDK version Nandi is tested with (also in
/// `runtimes/claude-sdk/package.json`)
pub const SDK_PACKAGE: &str = "@anthropic-ai/claude-agent-sdk";
pub const SDK_VERSION: &str = "0.3.304";

const BRIDGE_JS: &str = include_str!("../../../../runtimes/claude-sdk/bridge.mjs");
const PACKAGE_JSON: &str = include_str!("../../../../runtimes/claude-sdk/package.json");

/// The bridge must load the SDK within this
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// `npm install` of the SDK (about 50 MB without the bundled CLI)
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// Generous, as for `llm`: a completion on a large model can take minutes
const COMPLETE_TIMEOUT: Duration = Duration::from_secs(300);
/// How long `usable()` trusts its last look at the machine
const READINESS_TTL: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Readiness
// ---------------------------------------------------------------------------

/// The team's Bedrock choice: Claude Code runs on Bedrock with this AWS
/// profile and region (`None`: the AWS defaults)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BedrockRoute {
    pub aws_profile: Option<String>,
    pub aws_region: Option<String>,
}

impl BedrockRoute {
    /// Claude Code's switches for it. No keys: Claude Code resolves the
    /// profile from the host's `~/.aws`.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env = vec![("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())];
        if let Some(profile) = &self.aws_profile {
            env.push(("AWS_PROFILE".to_string(), profile.clone()));
        }
        if let Some(region) = &self.aws_region {
            env.push(("AWS_REGION".to_string(), region.clone()));
        }
        env
    }
}

type RouteSource = Box<dyn Fn() -> Option<BedrockRoute> + Send + Sync>;

static BEDROCK_ROUTE: std::sync::OnceLock<RouteSource> = std::sync::OnceLock::new();

/// Where the team's Bedrock choice is read from (`harness::init`: the
/// repo's `.nandi/llm.yaml`)
pub fn set_bedrock_route_source(source: RouteSource) {
    let _ = BEDROCK_ROUTE.set(source);
}

/// The team's Bedrock choice, if it made one
pub fn bedrock_route() -> Option<BedrockRoute> {
    BEDROCK_ROUTE.get().and_then(|source| source())
}

/// What the SDK route needs on this machine
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Readiness {
    /// The user's `claude`
    pub claude: Option<PathBuf>,
    /// Node, to run the bridge
    pub node: Option<PathBuf>,
    /// Claude Code looks signed in (its own login, a key, or a cloud
    /// provider it is set up for)
    pub signed_in: bool,
    /// The team chose Bedrock: Claude Code signs in with the AWS profile
    pub bedrock: Option<BedrockRoute>,
}

impl Readiness {
    pub fn check(env: &DetectEnv) -> Self {
        let claude = detection::find_executable(
            detection::spec("claude-code")
                .expect("built-in")
                .executables,
            env,
        );
        let signed_in = claude.is_some() && detection::provider_and_auth("claude-code", env).1;
        Self {
            claude,
            node: detection::find_executable(&["node"], env),
            signed_in,
            bedrock: bedrock_route(),
        }
    }

    pub fn usable(&self) -> bool {
        self.claude.is_some() && self.node.is_some() && (self.signed_in || self.bedrock.is_some())
    }

    /// Claude Code's environment for a session: the team's Bedrock route,
    /// else nothing (Claude Code signs in its own way)
    pub fn sign_in_env(&self) -> Vec<(String, String)> {
        self.bedrock
            .as_ref()
            .map(BedrockRoute::env)
            .unwrap_or_default()
    }

    /// What's missing, in words the user can act on
    pub fn problem(&self) -> Option<HarnessError> {
        if self.claude.is_none() {
            Some(HarnessError::NotInstalled("Claude Code".to_string()))
        } else if self.node.is_none() {
            Some(HarnessError::NotInstalled(
                "Node.js (the Claude Agent SDK runs on it)".to_string(),
            ))
        } else if !self.signed_in && self.bedrock.is_none() {
            Some(HarnessError::Auth(
                "run `claude` once and sign in, or set up AWS Bedrock in Settings, \
                 then try again"
                    .to_string(),
            ))
        } else {
            None
        }
    }
}

static READINESS: Mutex<Option<(Instant, Readiness)>> = Mutex::new(None);

/// This machine's readiness, re-checked at most every `READINESS_TTL`
pub fn readiness() -> Readiness {
    let mut cached = READINESS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, readiness)) = cached.as_ref() {
        if at.elapsed() < READINESS_TTL {
            return readiness.clone();
        }
    }
    let readiness = Readiness::check(&DetectEnv::current());
    *cached = Some((Instant::now(), readiness.clone()));
    readiness
}

/// Forgets the cached readiness (after detection, or a sign-in)
pub fn invalidate() {
    *READINESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
thread_local! {
    static TEST_USABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: whether `usable()` says yes on this thread (never the real machine)
#[cfg(test)]
pub fn set_usable_for_tests(usable: bool) {
    TEST_USABLE.with(|cell| cell.set(usable));
}

/// Whether LLM calls can go through Claude Code on this machine
pub fn usable() -> bool {
    #[cfg(test)]
    {
        TEST_USABLE.with(|cell| cell.get())
    }
    #[cfg(not(test))]
    {
        readiness().usable()
    }
}

// ---------------------------------------------------------------------------
// The bridge
// ---------------------------------------------------------------------------

/// `~/.nandi/local/runtimes/claude-sdk`
pub fn bridge_dir() -> Result<PathBuf, String> {
    Ok(crate::utils::nandi_config::local_dir()?
        .join("runtimes")
        .join("claude-sdk"))
}

/// One install at a time (runs and LLM calls may race on first use)
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Writes the bridge and installs the SDK if it isn't (at `SDK_VERSION`).
/// Returns the bridge script's path.
pub async fn ensure_bridge(env: &DetectEnv) -> Result<PathBuf, HarnessError> {
    let dir = bridge_dir().map_err(HarnessError::Other)?;
    let _install = INSTALL.lock().await;
    tokio::fs::create_dir_all(&dir).await?;
    write_if_changed(&dir.join("bridge.mjs"), BRIDGE_JS).await?;
    write_if_changed(&dir.join("package.json"), PACKAGE_JSON).await?;
    if installed_version(&dir).as_deref() != Some(SDK_VERSION) {
        install(&dir, env).await?;
    }
    Ok(dir.join("bridge.mjs"))
}

async fn write_if_changed(path: &Path, content: &str) -> Result<(), HarnessError> {
    if tokio::fs::read_to_string(path).await.ok().as_deref() != Some(content) {
        tokio::fs::write(path, content).await?;
    }
    Ok(())
}

fn installed_version(dir: &Path) -> Option<String> {
    let manifest = dir
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-agent-sdk")
        .join("package.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(manifest).ok()?).ok()?;
    json.get("version")?.as_str().map(str::to_string)
}

async fn install(dir: &Path, env: &DetectEnv) -> Result<(), HarnessError> {
    let npm = detection::find_executable(&["npm"], env)
        .ok_or_else(|| HarnessError::NotInstalled("npm (it comes with Node.js)".to_string()))?;
    log::info!(
        "Installing {SDK_PACKAGE}@{SDK_VERSION} into {}",
        dir.display()
    );
    let mut command = tokio::process::Command::new(npm);
    command
        .args([
            "install",
            "--omit=optional",
            "--no-audit",
            "--no-fund",
            "--loglevel=error",
        ])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    let output = tokio::time::timeout(INSTALL_TIMEOUT, command.output())
        .await
        .map_err(|_| HarnessError::Timeout(INSTALL_TIMEOUT))?
        .map_err(|e| HarnessError::Spawn(format!("npm install: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: String = stderr.chars().take(600).collect();
        return Err(HarnessError::Other(format!(
            "Installing the Claude Agent SDK failed: {}",
            detail.trim()
        )));
    }
    log::info!("Installed {SDK_PACKAGE}@{SDK_VERSION}");
    Ok(())
}

/// The bridge's first line
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeConfig<'a> {
    claude_path: String,
    cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    system_prompt: &'a str,
    prompt: &'a str,
}

/// One line from the bridge
#[derive(Debug, Clone, PartialEq)]
enum BridgeLine {
    /// The SDK loaded
    Ready,
    Update(AcpSessionUpdate),
    /// Informational (`sessionInfo`) or unknown
    Other,
}

fn parse_line(line: &str) -> Result<BridgeLine, HarnessError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| HarnessError::Protocol(format!("bad line from the bridge: {e}")))?;
    Ok(match value.get("kind").and_then(Value::as_str) {
        Some("ready") => BridgeLine::Ready,
        Some(
            "messageDelta" | "thinkingDelta" | "toolCallStarted" | "toolCallFinished"
            | "turnFinished" | "permissionRequest" | "error",
        ) => BridgeLine::Update(
            serde_json::from_value(value)
                .map_err(|e| HarnessError::Protocol(format!("bad update from the bridge: {e}")))?,
        ),
        _ => BridgeLine::Other,
    })
}

/// A running bridge, past its `ready` line
struct Bridge {
    process: SpawnedHarness,
    lines: Lines<BufReader<ChildStdout>>,
}

async fn spawn_bridge(
    env: &DetectEnv,
    working_dir: &Path,
    extra_env: &[(String, String)],
    config: BridgeConfig<'_>,
) -> Result<Bridge, HarnessError> {
    let readiness = Readiness::check(env);
    if let Some(problem) = readiness.problem() {
        return Err(problem);
    }
    let (Some(node), Some(claude)) = (readiness.node, readiness.claude) else {
        unreachable!("checked by problem()");
    };
    let script = ensure_bridge(env).await?;
    let config = BridgeConfig {
        claude_path: claude.display().to_string(),
        ..config
    };
    let invocation = Invocation {
        program: node.display().to_string(),
        args: vec![script.display().to_string()],
    };
    let mut process = spawn_with_env(&invocation, extra_env, &Placement::Host, working_dir)?;
    let (Some(mut stdin), Some(stdout)) = (process.child.stdin.take(), process.child.stdout.take())
    else {
        return Err(HarnessError::Spawn("no stdio".to_string()));
    };
    let line = serde_json::to_string(&config).map_err(|e| HarnessError::Other(e.to_string()))?;
    stdin.write_all(format!("{line}\n").as_bytes()).await?;
    stdin.flush().await?;
    // The config is all the bridge reads
    drop(stdin);
    let mut lines = BufReader::new(stdout).lines();
    let ready = tokio::time::timeout(READY_TIMEOUT, async {
        loop {
            match lines.next_line().await? {
                None => return Err(HarnessError::Exited(None)),
                Some(line) if line.trim().is_empty() => continue,
                Some(line) => match parse_line(&line)? {
                    BridgeLine::Ready => return Ok(()),
                    BridgeLine::Update(AcpSessionUpdate::Error { message }) => {
                        return Err(HarnessError::Other(message))
                    }
                    _ => continue,
                },
            }
        }
    })
    .await
    .unwrap_or(Err(HarnessError::Timeout(READY_TIMEOUT)));
    if let Err(error) = ready {
        let detail = process.stderr_summary();
        let _ = process.child.start_kill();
        return Err(match detail {
            Some(detail) => HarnessError::Other(format!("{error}: {detail}")),
            None => error,
        });
    }
    Ok(Bridge { process, lines })
}

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

/// One completion's answer
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Completion {
    pub text: String,
    pub stop_reason: String,
    pub usage: AcpUsage,
}

/// One tool-less completion on Claude Code: `system` as the whole system
/// prompt, `prompt` as the user turn. Text deltas go to `text` as they
/// arrive. Dropping the future kills the bridge.
pub async fn complete(
    system: &str,
    prompt: &str,
    model: Option<&str>,
    text: Option<mpsc::Sender<String>>,
) -> Result<Completion, String> {
    let env = DetectEnv::current();
    // An empty folder: Claude Code finds no project there
    let cwd = bridge_dir()?.join("scratch");
    tokio::fs::create_dir_all(&cwd)
        .await
        .map_err(|e| format!("Could not create {}: {e}", cwd.display()))?;
    let sign_in = Readiness::check(&env).sign_in_env();
    let mut bridge = spawn_bridge(
        &env,
        &cwd,
        &sign_in,
        BridgeConfig {
            claude_path: String::new(),
            cwd: cwd.display().to_string(),
            model,
            system_prompt: system,
            prompt,
        },
    )
    .await
    .map_err(|e| format!("Claude Code: {e}"))?;
    let read = async {
        let mut completion = Completion::default();
        while let Some(line) = bridge.lines.next_line().await.map_err(|e| e.to_string())? {
            if line.trim().is_empty() {
                continue;
            }
            match parse_line(&line).map_err(|e| e.to_string())? {
                BridgeLine::Update(AcpSessionUpdate::MessageDelta { text: delta }) => {
                    if let Some(text) = &text {
                        let _ = text.send(delta.clone()).await;
                    }
                    completion.text.push_str(&delta);
                }
                BridgeLine::Update(AcpSessionUpdate::TurnFinished { stop_reason, usage }) => {
                    completion.stop_reason = stop_reason;
                    completion.usage = usage.unwrap_or_default();
                    return Ok(completion);
                }
                BridgeLine::Update(AcpSessionUpdate::Error { message }) => return Err(message),
                _ => {}
            }
        }
        Err(match bridge.process.stderr_summary() {
            Some(detail) => format!("Claude Code exited: {detail}"),
            None => "Claude Code exited before it answered".to_string(),
        })
    };
    let result = tokio::time::timeout(COMPLETE_TIMEOUT, read)
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "Claude Code did not answer within {}s",
                COMPLETE_TIMEOUT.as_secs()
            ))
        });
    let _ = bridge.process.child.start_kill();
    result.map_err(|e| format!("Claude Code: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_lines_parse_into_updates() {
        assert_eq!(
            parse_line(r#"{"kind":"ready"}"#).unwrap(),
            BridgeLine::Ready
        );
        assert_eq!(
            parse_line(r#"{"kind":"sessionInfo","sessionId":"s"}"#).unwrap(),
            BridgeLine::Other
        );
        assert_eq!(
            parse_line(r#"{"kind":"messageDelta","text":"hi"}"#).unwrap(),
            BridgeLine::Update(AcpSessionUpdate::MessageDelta { text: "hi".into() })
        );
        assert_eq!(
            parse_line(
                r#"{"kind":"turnFinished","stopReason":"end_turn","usage":{"inputTokens":3,"outputTokens":2,"cachedInputTokens":1,"costUsd":null}}"#
            )
            .unwrap(),
            BridgeLine::Update(AcpSessionUpdate::TurnFinished {
                stop_reason: "end_turn".into(),
                usage: Some(AcpUsage {
                    input_tokens: 3,
                    output_tokens: 2,
                    cached_input_tokens: 1,
                    cost_usd: None,
                }),
            })
        );
        assert!(matches!(
            parse_line(
                r#"{"kind":"toolCallStarted","toolCallId":"t1","title":"Bash: ls","toolKind":"execute","input":{"command":"ls"}}"#
            )
            .unwrap(),
            BridgeLine::Update(AcpSessionUpdate::ToolCallStarted { tool_kind: Some(k), .. }) if k == "execute"
        ));
        assert!(parse_line("not json").is_err());
    }

    #[test]
    fn readiness_says_what_is_missing() {
        let home = tempfile::tempdir().unwrap();
        let env = DetectEnv {
            home: home.path().to_path_buf(),
            path: Some(home.path().as_os_str().to_owned()),
            vars: Default::default(),
        };
        let readiness = Readiness::check(&env);
        assert!(!readiness.usable());
        assert!(matches!(
            readiness.problem(),
            Some(HarnessError::NotInstalled(what)) if what == "Claude Code"
        ));
        let no_node = Readiness {
            claude: Some("claude".into()),
            node: None,
            signed_in: true,
            bedrock: None,
        };
        assert!(matches!(
            no_node.problem(),
            Some(HarnessError::NotInstalled(what)) if what.contains("Node.js")
        ));
        let signed_out = Readiness {
            node: Some("node".into()),
            signed_in: false,
            ..no_node.clone()
        };
        assert!(matches!(signed_out.problem(), Some(HarnessError::Auth(_))));
        let ready = Readiness {
            signed_in: true,
            ..signed_out.clone()
        };
        assert!(ready.usable() && ready.problem().is_none());
        assert!(ready.sign_in_env().is_empty(), "Claude Code's own sign-in");
        // The team's Bedrock choice is enough without a Claude login
        let bedrock = Readiness {
            bedrock: Some(BedrockRoute {
                aws_profile: Some("acme-dev".into()),
                aws_region: Some("us-east-1".into()),
            }),
            ..signed_out
        };
        assert!(bedrock.usable() && bedrock.problem().is_none());
        assert_eq!(
            bedrock.sign_in_env(),
            vec![
                ("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string()),
                ("AWS_PROFILE".to_string(), "acme-dev".to_string()),
                ("AWS_REGION".to_string(), "us-east-1".to_string()),
            ]
        );
    }

    #[test]
    fn the_embedded_package_pins_the_sdk_version() {
        let package: Value = serde_json::from_str(PACKAGE_JSON).unwrap();
        assert_eq!(package["dependencies"][SDK_PACKAGE], SDK_VERSION);
        assert!(BRIDGE_JS.contains(SDK_PACKAGE));
    }

    /// Live, on a machine where `claude` is signed in (installs the SDK):
    /// `NANDI_HOME=<tmp> cargo test live_completion -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_completion_on_this_machine() {
        let (tx, mut rx) = mpsc::channel(64);
        let completion = complete(
            "Answer in one word.",
            "What is the capital of France?",
            Some("haiku"),
            Some(tx),
        )
        .await
        .unwrap();
        assert!(completion.text.contains("Paris"), "{completion:?}");
        assert_eq!(completion.stop_reason, "end_turn");
        assert!(completion.usage.output_tokens > 0);
        let mut streamed = String::new();
        while let Ok(delta) = rx.try_recv() {
            streamed.push_str(&delta);
        }
        assert_eq!(streamed, completion.text);
    }

    #[test]
    fn usable_is_off_in_tests_unless_a_test_turns_it_on() {
        assert!(!usable());
        set_usable_for_tests(true);
        assert!(usable());
        set_usable_for_tests(false);
    }
}
