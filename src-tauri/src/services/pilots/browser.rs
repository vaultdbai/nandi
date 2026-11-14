//! The browser pilots: Microsoft's Playwright MCP server (pinned, unmodified)
//! driving a real Chromium, one browser per agent.
//!
//! - `browser-headless` runs in a container from Microsoft's image on the
//!   internal `nandi-agents` network, so the only way out is the egress
//!   proxy. `browser` is a visible window on this computer (system Chrome,
//!   else Edge) so the user can watch, forced through the same proxy.
//! - **Only the agent's sites load.** Chromium's proxy is the egress proxy,
//!   authenticated as `<pilot>:<agent>` ([`identity`]): the proxy applies
//!   that agent's `network.allow`, narrowed by its policies, to every
//!   request the page makes, and logs them in `egress.log`. Loopback goes
//!   through it too (`<-loopback>`), and WebRTC can't go around it.
//!   Navigation to a site outside the rules is refused before it's tried,
//!   so the agent says why. (Playwright's own `allowedOrigins` is "not a
//!   security boundary" and has no `*.domain` patterns, so it isn't used.)
//! - Everything else is Playwright MCP's own: its tools, its persistent
//!   profile (`~/.nandi/local/browser/<agent>/<pilot>/profile`, never the
//!   user's own), its secrets (the agent types a secret's name; the server
//!   types the value and hides it in every reply), and its screenshots
//!   (saved into the agent's work folder, `browser/…`).
//! - What runs without asking is decided per call ([`action`], [`needs_approval`]):
//!   reading and moving between allowed pages never ask; clicking and
//!   typing, submitting, typing a secret, files and paying ask unless the
//!   agent's skill.md allows them (`pilots: [{id: browser, allow: [...]}]`).
//!   The arbitrary-code tools are never offered.
//! - Policies can refuse controls (`browser.refuse`, [`super::refusal`]):
//!   before a call that acts on the page, Nandi asks the page what the
//!   control is and where it is, and refuses it with the policy's words.
//! - The visible browser also has `browser_hand_over` ([`super::handover`]):
//!   the agent prepares a page and a person finishes it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, OnceLock};
use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;

use super::detect::Probe;
use super::mcp::PilotTool;
use super::{handover, refusal, sites, PilotDefinition};
use crate::llm::ToolSpec;
use crate::runtime::ProxyTokens;
use crate::sandbox::{Access, SandboxSpec};
use crate::services::domain::capability::PilotOptions;
use crate::services::domain::policy::Refusal;
use crate::services::domain::Agent;
use crate::services::egress::capabilities::Capabilities;
use crate::services::repo_store::RepoStore;
use crate::services::secrets::Secrets;

pub const BROWSER: &str = "browser";
pub const HEADLESS: &str = "browser-headless";
/// Playwright MCP, as npm and Microsoft's registry name it (same release)
pub const PACKAGE: &str = "@playwright/mcp@0.0.83";
/// [`PACKAGE`]'s name and pinned version, as `npm ls` reports them
const PACKAGE_NAME: &str = "@playwright/mcp";
const PACKAGE_VERSION: &str = "0.0.83";
pub const IMAGE: &str = "mcr.microsoft.com/playwright/mcp:v0.0.83";

/// Tools that run arbitrary code (in the page, or in the MCP server
/// itself): never offered
const HIDDEN_TOOLS: &[&str] = &["browser_evaluate", "browser_run_code_unsafe"];
const START_TIMEOUT: Duration = Duration::from_secs(60);
const CALL_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_RESULT_CHARS: usize = 60_000;
/// Inside the headless container: the agent's work folder and config
const CONTAINER_WORKDIR: &str = "/agent/workdir";
const CONTAINER_CONFIG: &str = "/config/config.json";
const CONTAINER_PROFILE: &str = "/profile";

pub fn is_browser(pilot_id: &str) -> bool {
    pilot_id == BROWSER || pilot_id == HEADLESS
}

/// Playwright's screenshot tool (named per run step by the orchestrator)
pub const SCREENSHOT_TOOL: &str = "browser_take_screenshot";
/// Playwright's tools that only look (its read-only hint), for deciding
/// outside a running browser what a call does
const READ_TOOLS: &[&str] = &[
    "browser_snapshot",
    SCREENSHOT_TOOL,
    "browser_console_messages",
    "browser_network_requests",
    "browser_wait_for",
];

pub fn is_read_tool(tool: &str) -> bool {
    READ_TOOLS.contains(&tool)
}

/// The egress proxy's name for `agent`'s `pilot` browser
pub fn identity(pilot: &str, agent: &str) -> String {
    format!("{pilot}:{agent}")
}

/// The agent behind an [`identity`], if it is one
pub fn agent_of_identity(identity: &str) -> Option<&str> {
    [BROWSER, HEADLESS].iter().find_map(|pilot| {
        identity
            .strip_prefix(pilot)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|agent| !agent.is_empty())
    })
}

/// What a browser needs from the app (set once at start-up)
pub struct Host {
    pub proxy_tokens: ProxyTokens,
    pub secrets: Secrets,
    pub store: RepoStore,
}

static HOST: OnceLock<Host> = OnceLock::new();

pub fn init(host: Host) {
    let _ = HOST.set(host);
}

fn host() -> Result<&'static Host, String> {
    HOST.get()
        .ok_or_else(|| "The browser isn't set up yet".to_string())
}

/// A running browser server for one agent
struct Instance {
    service: RunningService<RoleClient, ()>,
    child: Child,
    /// The settings it started with (no token), to restart on change
    fingerprint: String,
    tools: Vec<PilotTool>,
    /// The secrets it types, by name, to mask in its replies (memory only)
    secrets: HashMap<String, String>,
}

/// `(agent, pilot)` → its browser
static POOL: LazyLock<tokio::sync::Mutex<HashMap<(String, String), Instance>>> =
    LazyLock::new(Default::default);

/// `(agent, pilot)` → the URL its browser last showed, for deciding before
/// a call runs whether a policy refuses it
static PAGES: LazyLock<std::sync::Mutex<HashMap<(String, String), String>>> =
    LazyLock::new(Default::default);

pub(super) fn remember_page(agent: &str, pilot: &str, url: &str) {
    if let Ok(mut pages) = PAGES.lock() {
        pages.insert((agent.to_string(), pilot.to_string()), url.to_string());
    }
}

/// `(agent, pilot)` browsers with pages open: set by any call but
/// `browser_close`, cleared by it (the run plan's finish check and the
/// close at run end)
static OPEN_PAGES: LazyLock<std::sync::Mutex<std::collections::HashSet<(String, String)>>> =
    LazyLock::new(Default::default);

pub(crate) fn set_pages_open(agent: &str, pilot: &str, open: bool) {
    let mut pages = OPEN_PAGES.lock().unwrap_or_else(|e| e.into_inner());
    let key = (agent.to_string(), pilot.to_string());
    if open {
        pages.insert(key);
    } else {
        pages.remove(&key);
    }
}

/// Whether one of `agent`'s browsers still has pages open
pub fn has_open_pages(agent: &str) -> bool {
    OPEN_PAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(a, _)| a == agent)
}

/// Closes `agent`'s browser pages (a run of its ended). The profile stays,
/// and so does the browser's server, ready for the next run.
pub async fn close_pages(agent: &str) {
    let open: Vec<String> = OPEN_PAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(a, _)| a == agent)
        .map(|(_, pilot)| pilot.clone())
        .collect();
    for pilot in open {
        set_pages_open(agent, &pilot, false);
        let mut pool = POOL.lock().await;
        if let Some(instance) = pool.get_mut(&(agent.to_string(), pilot.clone())) {
            match run_tool(instance, "browser_close", &json!({})).await {
                Ok(_) => log::info!("Closed the {pilot} pages of '{agent}': its run ended"),
                Err(e) => log::warn!("Couldn't close the {pilot} pages of '{agent}': {e}"),
            }
        }
    }
}

fn last_page(agent: &str, pilot: &str) -> Option<String> {
    PAGES
        .lock()
        .ok()?
        .get(&(agent.to_string(), pilot.to_string()))
        .cloned()
}

// ---------------------------------------------------------------------------
// What may run without asking
// ---------------------------------------------------------------------------

/// What a browser tool call does, most sensitive kind first when several apply
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Snapshots, screenshots, waiting: never asks
    Read,
    /// Opening allowed pages, tabs, going back: never asks (the proxy
    /// refuses other sites)
    Navigate,
    Interact,
    Submit,
    Secret,
    Upload,
    Pay,
    /// `browser_hand_over`: always waits for the person
    HandOver,
}

const PAY_WORDS: &[&str] = &[
    "pay",
    "buy",
    "purchase",
    "checkout",
    "check out",
    "place order",
    "order now",
    "subscribe",
    "donate",
    "payment",
];
const SUBMIT_WORDS: &[&str] = &[
    "submit", "send", "post", "publish", "save", "confirm", "delete", "remove", "sign up",
    "signup", "register", "log in", "login", "sign in", "apply", "approve", "accept", "transfer",
];
const FILE_WORDS: &[&str] = &["download", "export"];
const SECRET_WORDS: &[&str] = &[
    "password",
    "passcode",
    "passphrase",
    "secret",
    "token",
    "pin code",
];

fn says(text: &str, words: &[&str]) -> bool {
    crate::services::domain::browser_rule::says(text, words)
}

/// Playwright MCP asks for an element's human-readable description exactly
/// so a client can decide on permission: that, the tool and its arguments
/// decide the kind
pub fn action(tool: &str, args: &Value, read_only: bool, secret_names: &[String]) -> Action {
    let element = args["element"].as_str().unwrap_or_default();
    let has_secret = |text: &str| secret_names.iter().any(|name| text.contains(name.as_str()));
    let typed: Vec<&str> = match tool {
        "browser_type" => args["text"].as_str().into_iter().collect(),
        "browser_fill_form" => args["fields"]
            .as_array()
            .map(|fields| fields.iter().filter_map(|f| f["value"].as_str()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let field_names: Vec<&str> = match tool {
        "browser_fill_form" => args["fields"]
            .as_array()
            .map(|fields| fields.iter().filter_map(|f| f["name"].as_str()).collect())
            .unwrap_or_default(),
        _ => vec![element],
    };
    let secret = typed.iter().any(|t| has_secret(t))
        || (!typed.is_empty() && field_names.iter().any(|n| says(n, SECRET_WORDS)));
    match tool {
        handover::TOOL => Action::HandOver,
        "browser_navigate"
        | "browser_navigate_back"
        | "browser_tabs"
        | "browser_resize"
        | "browser_close" => Action::Navigate,
        "browser_file_upload" | "browser_drop" => Action::Upload,
        _ if secret => Action::Secret,
        "browser_click" if says(element, PAY_WORDS) => Action::Pay,
        "browser_click" if says(element, FILE_WORDS) => Action::Upload,
        "browser_click" if says(element, SUBMIT_WORDS) => Action::Submit,
        "browser_type" if args["submit"].as_bool() == Some(true) => Action::Submit,
        // Enter, and Ctrl+Enter (which posts on many sites)
        "browser_press_key" if args["key"].as_str().is_some_and(|k| k.ends_with("Enter")) => {
            Action::Submit
        }
        "browser_handle_dialog" if args["accept"].as_bool() == Some(true) => Action::Submit,
        _ if read_only => Action::Read,
        _ => Action::Interact,
    }
}

/// Whether `action` waits for the user, given what skill.md allows
pub fn needs_approval(action: Action, options: Option<&PilotOptions>) -> bool {
    let allows = |kind: &str| options.is_some_and(|o| o.allows(kind));
    match action {
        Action::Read | Action::Navigate => false,
        Action::Interact => !allows("interact"),
        Action::Submit => !allows("submit"),
        Action::Secret => !allows("secrets"),
        Action::Upload => !allows("upload"),
        Action::Pay => !allows("pay"),
        Action::HandOver => true,
    }
}

/// Whether a policy refuses this call on the page `agent`'s browser last
/// showed: such a call isn't asked about, it's refused when it runs
pub fn refused_before(
    agent: &str,
    pilot: &str,
    refusals: &[Refusal],
    tool: &str,
    args: &Value,
    action: Action,
) -> bool {
    !refusals.is_empty()
        && last_page(agent, pilot)
            .is_some_and(|url| refusal::refused(refusals, &url, tool, args, action, None).is_some())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Where a browser runs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// In a container on the agents' network
    Container,
    /// A window on this computer, with this browser channel
    Window(&'static str),
}

/// Playwright MCP's config file. `proxy` is the egress proxy URL as the
/// browser reaches it; the token is its password (never on a command line).
fn config(
    mode: Mode,
    proxy: &str,
    token: &str,
    profile: &str,
    output: &str,
    secrets: &HashMap<String, String>,
) -> Value {
    let mut launch = json!({
        "headless": mode == Mode::Container,
        "proxy": {
            "server": proxy,
            // Chromium skips the proxy for loopback unless told not to
            "bypass": "<-loopback>",
            "username": "nandi",
            "password": token,
        },
        "args": [
            "--proxy-bypass-list=<-loopback>",
            // WebRTC could otherwise reach the network around the proxy
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
        ],
    });
    if let Mode::Window(channel) = mode {
        launch["channel"] = Value::from(channel);
        // Playwright MCP adds --disable-blink-features=AutomationControlled,
        // and Chrome and Edge then show an "unsupported command-line flag"
        // bar in the window; --test-type (as ChromeDriver passes) hides it
        if let Some(args) = launch["args"].as_array_mut() {
            args.push(Value::from("--test-type"));
        }
    }
    json!({
        "browser": {
            "browserName": "chromium",
            "userDataDir": profile,
            "launchOptions": launch,
        },
        "outputDir": output,
        // The model reads the accessibility tree; screenshots are files
        "imageResponses": "omit",
        "codegen": "none",
        "secrets": secrets,
    })
}

/// The system browser Playwright drives for the visible window
fn window_channel() -> Option<&'static str> {
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[
            (
                "chrome",
                &[
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                ],
            ),
            (
                "msedge",
                &[
                    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                ],
            ),
        ]
    } else if cfg!(target_os = "macos") {
        &[
            ("chrome", &["/Applications/Google Chrome.app"]),
            ("msedge", &["/Applications/Microsoft Edge.app"]),
        ]
    } else {
        &[
            (
                "chrome",
                &["/usr/bin/google-chrome", "/opt/google/chrome/chrome"],
            ),
            ("msedge", &["/usr/bin/microsoft-edge"]),
        ]
    };
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    candidates.iter().find_map(|(channel, paths)| {
        let user_chrome = Path::new(&local).join(r"Google\Chrome\Application\chrome.exe");
        let found = paths.iter().any(|p| Path::new(p).exists())
            || (*channel == "chrome" && cfg!(windows) && user_chrome.exists());
        found.then_some(*channel)
    })
}

/// What the pilot needs before it can start: Docker and the image, or npx
/// and Chrome/Edge
pub async fn prerequisites(pilot: &PilotDefinition) -> Result<(), String> {
    if pilot.id == HEADLESS {
        let present = tokio::process::Command::new("docker")
            .args(["image", "inspect", IMAGE])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|_| "Docker is not installed".to_string())?;
        if !present.success() {
            return Err(format!(
                "The browser image isn't here yet: Install it first ({IMAGE})"
            ));
        }
        return Ok(());
    }
    let env = crate::services::harness::detection::DetectEnv::current();
    if crate::services::harness::detection::find_executable(&["npx"], &env).is_none() {
        return Err("The visible browser needs Node.js (npx) on this computer".to_string());
    }
    window_channel()
        .map(|_| ())
        .ok_or_else(|| "The visible browser needs Google Chrome or Microsoft Edge".to_string())
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// The agent's secrets, read from the keychain, by the names it types
fn read_secrets(
    secrets: &Secrets,
    options: Option<&PilotOptions>,
) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for (name, entry) in options.map(|o| o.secrets.clone()).unwrap_or_default() {
        match secrets.get(entry.as_str())? {
            Some(value) if !value.is_empty() => {
                out.insert(name, value);
            }
            _ => log::warn!(
                "Browser secret '{name}' isn't in the keychain ({})",
                entry.as_str()
            ),
        }
    }
    Ok(out)
}

/// The folder under `~/.nandi/local` that holds the browsers' own files
const BROWSER_DIR: &str = "browser";
/// Each browser's persistent profile, in its `(agent, pilot)` folder
const PROFILE_DIR: &str = "profile";

/// `agent`'s `pilot` browser folder under `local` (`~/.nandi/local`): never
/// under the repo, which is the gold copy
fn browser_base(local: &Path, agent: &str, pilot: &str) -> PathBuf {
    local.join(BROWSER_DIR).join(agent).join(pilot)
}

/// Where `agent`'s `pilot` browser keeps its sign-ins and cookies
fn profile_dir(local: &Path, agent: &str, pilot: &str) -> PathBuf {
    browser_base(local, agent, pilot).join(PROFILE_DIR)
}

fn dirs(agent: &str, pilot: &str) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    use crate::utils::nandi_config;
    let local = nandi_config::local_dir()?;
    let base = browser_base(&local, agent, pilot);
    let profile = profile_dir(&local, agent, pilot);
    let workdir = nandi_config::work_dir()?.join(agent);
    for dir in [&base, &profile, &workdir.join("browser")] {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    Ok((base, profile, workdir))
}

/// The browser for `agent`, started (or restarted, when its settings
/// changed) as needed
async fn ensure<'a>(
    pool: &'a mut HashMap<(String, String), Instance>,
    agent: &Agent,
    pilot: &str,
) -> Result<&'a mut Instance, String> {
    let host = host()?;
    let agent_id = agent.id().as_str().to_string();
    let options = agent.capabilities().pilot_options(pilot);
    let secrets = read_secrets(&host.secrets, options)?;
    let mode = if pilot == HEADLESS {
        Mode::Container
    } else {
        Mode::Window(window_channel().ok_or("No Chrome or Edge on this computer")?)
    };
    let (base, profile, workdir) = dirs(&agent_id, pilot)?;
    let (proxy, profile_arg, output_arg) = match mode {
        Mode::Container => {
            use crate::services::egress::relay;
            (
                format!("http://{}:{}", relay::RELAY_NAME, relay::RELAY_PORT),
                CONTAINER_PROFILE.to_string(),
                format!("{CONTAINER_WORKDIR}/browser"),
            )
        }
        Mode::Window(_) => (
            format!("http://127.0.0.1:{}", crate::services::egress::proxy_port()),
            profile.display().to_string(),
            workdir.join("browser").display().to_string(),
        ),
    };
    let fingerprint = config(mode, &proxy, "-", &profile_arg, &output_arg, &secrets).to_string();
    let key = (agent_id.clone(), pilot.to_string());
    let stale = pool.get_mut(&key).is_some_and(|instance| {
        instance.fingerprint != fingerprint || !matches!(instance.child.try_wait(), Ok(None))
    });
    if stale {
        if let Some(old) = pool.remove(&key) {
            shutdown(old).await;
        }
    }
    if !pool.contains_key(&key) {
        let token = host.proxy_tokens.issue(&identity(pilot, &agent_id));
        let settings = config(mode, &proxy, &token, &profile_arg, &output_arg, &secrets);
        let config_path = base.join("config.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&settings).unwrap_or_default(),
        )
        .map_err(|e| format!("Couldn't write the browser's settings: {e}"))?;
        let started = start(
            mode,
            pilot,
            &agent_id,
            &base,
            &profile,
            &workdir,
            &config_path,
        )
        .await;
        // The settings hold the proxy token and secrets: not left on disk
        let _ = std::fs::remove_file(&config_path);
        let (service, child) = started?;
        let tools = tokio::time::timeout(START_TIMEOUT, service.list_all_tools())
            .await
            .map_err(|_| "The browser didn't list its tools in time".to_string())?
            .map_err(|e| format!("The browser didn't list its tools: {e}"))?
            .into_iter()
            .filter(|tool| !HIDDEN_TOOLS.contains(&tool.name.as_ref()))
            .map(|tool| PilotTool {
                read_only: tool
                    .annotations
                    .as_ref()
                    .and_then(|a| a.read_only_hint)
                    .unwrap_or(false),
                spec: ToolSpec {
                    name: tool.name.to_string(),
                    description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
                    input_schema: Value::Object((*tool.input_schema).clone()),
                },
            })
            .collect();
        log::info!("Started the {pilot} pilot for agent '{agent_id}'");
        pool.insert(
            key.clone(),
            Instance {
                service,
                child,
                fingerprint,
                tools,
                secrets,
            },
        );
    }
    pool.get_mut(&key)
        .ok_or_else(|| "The browser stopped".to_string())
}

/// Spawns Playwright MCP over stdio: in a container, or on the host
async fn start(
    mode: Mode,
    pilot: &str,
    agent: &str,
    base: &Path,
    profile: &Path,
    workdir: &Path,
    config_path: &Path,
) -> Result<(RunningService<RoleClient, ()>, Child), String> {
    let mut cmd = match mode {
        Mode::Container => {
            use crate::services::egress::relay::{self, RelayImage};
            relay::ensure_ready(RelayImage::Runtime).await?;
            let name = container_name(agent);
            let _ = tokio::process::Command::new("docker")
                .args(["rm", "-f", &name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            let mut cmd = tokio::process::Command::new("docker");
            cmd.args(container_args(&name, pilot, agent, base, profile, workdir));
            #[cfg(windows)]
            {
                cmd.creation_flags(0x0800_0000);
            }
            cmd
        }
        Mode::Window(_) => {
            let args = vec![
                "-y".to_string(),
                PACKAGE.to_string(),
                "--config".to_string(),
                config_path.display().to_string(),
            ];
            let mut cmd = crate::services::connectors::host_command("npx", &args);
            cmd.current_dir(workdir);
            cmd
        }
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound if mode == Mode::Container => {
            "Docker is not installed".to_string()
        }
        _ => format!("Couldn't start the browser: {e}"),
    })?;
    let stdin = child.stdin.take().ok_or("No stdin for the browser")?;
    let stdout = child.stdout.take().ok_or("No stdout for the browser")?;
    if let Some(stderr) = child.stderr.take() {
        let label = format!("{pilot}:{agent}");
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log::debug!("[{label}] {line}");
            }
        });
    }
    let transport = crate::services::connectors::LineTransport::new(stdout, stdin);
    let service = tokio::time::timeout(START_TIMEOUT, ().serve(transport))
        .await
        .map_err(|_| "The browser didn't start in time".to_string())?
        .map_err(|e| format!("Couldn't talk to the browser: {e}"))?;
    Ok((service, child))
}

fn container_name(agent: &str) -> String {
    let safe: String = agent
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!(
        "nandi-browser-{}-{safe}",
        crate::utils::nandi_config::instance_tag()
    )
}

/// `docker run` for the headless browser: Microsoft's image as is, on the
/// agents' network, its settings read-only, its profile and the agent's
/// work folder mounted
fn container_args(
    name: &str,
    pilot: &str,
    agent: &str,
    base: &Path,
    profile: &Path,
    workdir: &Path,
) -> Vec<String> {
    let path = |path: &Path| path.display().to_string();
    SandboxSpec::new(IMAGE)
        .interactive()
        .remove_on_exit()
        .name(name)
        .label("nandi.pilot", pilot)
        .label("nandi.agent", agent)
        // Microsoft's image runs as its own user and writes outside its
        // mounts (Chrome's caches)
        .image_user()
        .writable_root()
        .pids(512)
        .memory_mb(2048)
        .cpus(BROWSER_CPUS)
        .shm_size_mb(1024)
        .network(crate::services::egress::relay::AGENT_NETWORK)
        .bind(&path(base), "/config", Access::ReadOnly)
        .bind(&path(profile), CONTAINER_PROFILE, Access::ReadWrite)
        .bind(&path(workdir), CONTAINER_WORKDIR, Access::ReadWrite)
        .workdir(CONTAINER_WORKDIR)
        .command(["--config", CONTAINER_CONFIG])
        .to_cli_args()
}

/// Chrome renders pages on several cores
const BROWSER_CPUS: f32 = 2.0;

async fn shutdown(instance: Instance) {
    let Instance {
        service, mut child, ..
    } = instance;
    let _ = service.cancel().await;
    let _ = child.kill().await;
}

/// Stops every agent's `pilot` browser (Library › Stop)
pub async fn stop_all(pilot: &str) {
    let agents: Vec<String> = POOL
        .lock()
        .await
        .keys()
        .filter(|(_, p)| p == pilot)
        .map(|(agent, _)| agent.clone())
        .collect();
    for agent in agents {
        stop(&agent, pilot).await;
    }
}

/// Stops `agent`'s `pilot` browser, if it runs
async fn stop(agent: &str, pilot: &str) {
    let key = (agent.to_string(), pilot.to_string());
    let instance = POOL.lock().await.remove(&key);
    if let Ok(mut pages) = PAGES.lock() {
        pages.remove(&key);
    }
    set_pages_open(agent, pilot, false);
    if let Some(instance) = instance {
        if let Ok(host) = host() {
            host.proxy_tokens.revoke(&identity(pilot, agent));
        }
        shutdown(instance).await;
    }
}

/// Whether `agent`'s `pilot` browser has a profile (sites may remember it)
pub fn has_profile(agent: &str, pilot: &str) -> bool {
    crate::utils::nandi_config::local_dir()
        .map(|local| profile_dir(&local, agent, pilot))
        .is_ok_and(|profile| profile.read_dir().is_ok_and(|mut d| d.next().is_some()))
}

/// "Sign out everywhere": stops `agent`'s `pilot` browser and deletes its
/// profile, so no site remembers it. `false`: there was none
pub async fn sign_out(agent: &str, pilot: &str) -> Result<bool, String> {
    stop(agent, pilot).await;
    clear_profile(&crate::utils::nandi_config::local_dir()?, agent, pilot).await
}

/// Deletes `agent`'s `pilot` profile under `local`, nothing else
async fn clear_profile(local: &Path, agent: &str, pilot: &str) -> Result<bool, String> {
    let profile = profile_dir(local, agent, pilot);
    if !profile.exists() {
        return Ok(false);
    }
    // The browser lets go of its files a moment after it stops
    let mut last = None;
    for _ in 0..10 {
        match std::fs::remove_dir_all(&profile) {
            Ok(()) => {
                log::info!("Signed '{agent}' out everywhere: cleared its {pilot} profile");
                return Ok(true);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(e) => last = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(format!(
        "Couldn't clear the browser profile: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// `content` with every secret value replaced by its name
fn mask_secrets(content: String, secrets: &HashMap<String, String>) -> String {
    secrets
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .fold(content, |content, (name, value)| {
            content.replace(value.as_str(), &format!("[{name}]"))
        })
}

/// The `aws s3 sync --exclude` pattern that keeps the browsers' profiles
/// out of a backup of `folder`, when `local` (`~/.nandi/local`) is in it
/// or it is inside a browser's folder
pub fn profile_backup_exclude(folder: &Path, local: &Path) -> Option<String> {
    let browsers = local.join(BROWSER_DIR);
    let joined = |path: &Path| -> String {
        path.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    };
    if let Ok(inside) = browsers.strip_prefix(folder) {
        let prefix = joined(inside);
        return Some(if prefix.is_empty() {
            format!("*/*/{PROFILE_DIR}/*")
        } else {
            format!("{prefix}/*/*/{PROFILE_DIR}/*")
        });
    }
    let inside: Vec<String> = folder
        .strip_prefix(&browsers)
        .ok()?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match inside.as_slice() {
        [_agent] => Some(format!("*/{PROFILE_DIR}/*")),
        [_agent, _pilot] => Some(format!("{PROFILE_DIR}/*")),
        [_agent, _pilot, profile, ..] if profile == PROFILE_DIR => Some("*".to_string()),
        _ => None,
    }
}

/// The tools `agent`'s `pilot` browser offers (starting it if needed)
pub async fn tools(agent: &Agent, pilot: &str) -> Result<Vec<PilotTool>, String> {
    let mut pool = POOL.lock().await;
    let instance = ensure(&mut pool, agent, pilot).await?;
    let mut tools: Vec<PilotTool> = instance
        .tools
        .iter()
        .map(|t| PilotTool {
            spec: with_reason(t.spec.clone()),
            read_only: t.read_only,
        })
        .collect();
    // Someone watches the visible browser, so the agent can hand over
    if pilot == BROWSER {
        tools.push(PilotTool {
            spec: handover::spec(),
            read_only: false,
        });
    }
    Ok(tools)
}

/// The page `tool` opens: `browser_navigate`, or a new tab with a URL
pub fn url_of(tool: &str, args: &Value) -> Option<String> {
    match tool {
        "browser_navigate" => args["url"].as_str().map(str::to_string),
        "browser_tabs" if args["action"].as_str() == Some("new") => {
            args["url"].as_str().map(str::to_string)
        }
        _ => None,
    }
}

/// `spec` with Nandi's optional `reason` argument, for a tool that opens a
/// page: a person asked to allow the site reads it
fn with_reason(mut spec: ToolSpec) -> ToolSpec {
    if !matches!(spec.name.as_str(), "browser_navigate" | "browser_tabs") {
        return spec;
    }
    if let Some(properties) = spec
        .input_schema
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            sites::REASON_ARG.to_string(),
            json!({
                "type": "string",
                "description": "Why the task needs this page, in a few words. If the site isn't one this agent may open yet, the person reads it when Nandi asks them to allow the site."
            }),
        );
    }
    spec
}

/// Whether `url` may be opened: http(s) to an allowed site (and the blank
/// page); the proxy enforces the same rules on every request the page makes
fn allowed_url(sites: &Capabilities, url: &str) -> Result<(), String> {
    if url.trim() == "about:blank" {
        return Ok(());
    }
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|_| format!("'{url}' isn't a web address (https://…)"))?;
    let host = parsed.host_str().unwrap_or_default();
    let allowed = match parsed.scheme() {
        "https" => sites.allows_tunnel(host, parsed.port().unwrap_or(443)),
        "http" => sites.allows_request(host, parsed.port(), "GET"),
        other => {
            return Err(format!(
                "{other}: pages can't be opened, only http and https"
            ))
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "{host} isn't one of this agent's allowed sites. Add it to the agent's Internet access to open it"
        ))
    }
}

/// Runs `tool` (the tool call `call_id`) in `agent`'s `pilot` browser:
/// `(content, is_error)`
pub async fn call(
    agent: &Agent,
    call_id: &str,
    pilot: &str,
    tool: &str,
    args: &Value,
) -> Result<(String, bool), String> {
    // The hand-over is the person's, never the browser's (`pilots::tools`)
    if HIDDEN_TOOLS.contains(&tool) || tool == handover::TOOL {
        return Ok((format!("'{tool}' isn't available"), true));
    }
    let host = host()?;
    let agent_id = agent.id().as_str();
    let skill = host.store.skill(agent_id)?;
    let mut args = args.clone();
    // Nandi's own argument (`sites::REASON_ARG`), not Playwright's
    if let Some(map) = args.as_object_mut() {
        map.remove(sites::REASON_ARG);
    }
    if let Some(url) = url_of(tool, &args) {
        // The agent's sites, plus any a person allowed once in this run
        let allowed = sites::with_run_sites(agent_id, skill.capabilities.clone());
        if let Err(refused) = allowed_url(&allowed, &url) {
            return Ok((refused, true));
        }
    }
    // Screenshots are files in the agent's work folder (the run's Files)
    let mut saved = None;
    if tool == "browser_take_screenshot" && args["filename"].as_str().is_none() {
        let name = format!(
            "browser/screenshot-{}.{}",
            crate::db::now_ms(),
            args["type"].as_str().unwrap_or("png")
        );
        args["filename"] = Value::from(name.clone());
        saved = Some(name);
    }
    let mut pool = POOL.lock().await;
    let instance = ensure(&mut pool, agent, pilot).await?;
    if !skill.browser_refusals.is_empty() && refusal::is_checked(tool) {
        let options = agent.capabilities().pilot_options(pilot);
        let secret_names: Vec<String> = options
            .map(|o| o.secrets.keys().cloned().collect())
            .unwrap_or_default();
        let kind = action(tool, &args, false, &secret_names);
        let predicted = refused_before(agent_id, pilot, &skill.browser_refusals, tool, &args, kind);
        let (page, element) = inspect(instance, &args).await;
        let refused = page.as_deref().and_then(|url| {
            refusal::refused(
                &skill.browser_refusals,
                url,
                tool,
                &args,
                kind,
                element.as_ref(),
            )
        });
        if let Some(message) = refused {
            drop(pool);
            crate::services::policies::record_refused(
                &host.store,
                agent_id,
                call_id,
                "browser controls",
            );
            return Ok((message, true));
        }
        // It wasn't asked about because it looked refused, and the page
        // has changed since: don't run it unasked
        if predicted && needs_approval(kind, options) {
            return Ok((
                "The page changed since this step was planned. Take a snapshot and try again."
                    .to_string(),
                true,
            ));
        }
    }
    let (content, is_error) = run_tool(instance, tool, &args).await?;
    // Playwright hides the values it typed; this is the second lock
    let mut content = mask_secrets(content, &instance.secrets);
    set_pages_open(agent_id, pilot, tool != "browser_close");
    if let Some(url) = refusal::page_url(&content) {
        remember_page(agent_id, pilot, &url);
    }
    if let (Some(name), false) = (saved, is_error) {
        content.push_str(&format!(
            "\nSaved the screenshot as {name} in the agent's files."
        ));
    }
    Ok((content, is_error))
}

/// One call to the browser's MCP server: `(content, is_error)`
async fn run_tool(
    instance: &mut Instance,
    tool: &str,
    args: &Value,
) -> Result<(String, bool), String> {
    let mut params = CallToolRequestParams::new(tool.to_string());
    params.arguments = args.as_object().cloned();
    let result = tokio::time::timeout(CALL_TIMEOUT, instance.service.call_tool(params))
        .await
        .map_err(|_| format!("'{tool}' timed out"))?
        .map_err(|e| format!("'{tool}' failed: {e}"))?;
    Ok(outcome(&serde_json::to_value(&result).unwrap_or_default()))
}

/// Where the page is and, for a call with a target, what the page calls
/// that control: a snapshot of just the target, else the tab list
async fn inspect(
    instance: &mut Instance,
    args: &Value,
) -> (Option<String>, Option<refusal::Element>) {
    let target = ["target", "startTarget"]
        .iter()
        .find_map(|key| args[*key].as_str())
        .filter(|t| !t.trim().is_empty());
    if let Some(target) = target {
        let look = json!({ "target": target, "depth": 1 });
        if let Ok((reply, false)) = run_tool(instance, "browser_snapshot", &look).await {
            if let Some(url) = refusal::page_url(&reply) {
                return (Some(url), refusal::element(&reply, target));
            }
        }
    }
    let tabs = run_tool(instance, "browser_tabs", &json!({ "action": "list" })).await;
    (
        tabs.ok().and_then(|(reply, _)| refusal::page_url(&reply)),
        None,
    )
}

/// Brings `agent`'s `pilot` browser window to the front, on its current tab
pub async fn bring_to_front(agent: &str, pilot: &str) -> Result<(), String> {
    let mut pool = POOL.lock().await;
    let instance = pool
        .get_mut(&(agent.to_string(), pilot.to_string()))
        .ok_or("its browser isn't open")?;
    let (tabs, _) = run_tool(instance, "browser_tabs", &json!({ "action": "list" })).await?;
    let index = refusal::current_tab(&tabs).unwrap_or(0);
    run_tool(
        instance,
        "browser_tabs",
        &json!({ "action": "select", "index": index }),
    )
    .await
    .map(|_| ())
}

/// The text blocks (images are files, not replies), cut to size
fn outcome(result: &Value) -> (String, bool) {
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let mut content = result["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if content.chars().count() > MAX_RESULT_CHARS {
        content = content.chars().take(MAX_RESULT_CHARS).collect();
        content.push_str("\n[… cut: take a snapshot of part of the page]");
    }
    (content, is_error)
}

/// Where each browser's install puts it ([`super::detect`])
pub fn probe(pilot_id: &str) -> Option<Probe> {
    match pilot_id {
        BROWSER => Some(Probe::NpmGlobal {
            package: PACKAGE_NAME,
            version: PACKAGE_VERSION,
        }),
        HEADLESS => Some(Probe::DockerImage(IMAGE)),
        _ => None,
    }
}

/// What the catalog lists: both browsers, any OS, one capability
pub fn definitions() -> Vec<PilotDefinition> {
    let definition = |id: &str, name: &str, install: String, description: &str| PilotDefinition {
        id: id.to_string(),
        name: name.to_string(),
        platform: "any".to_string(),
        install_command: install,
        // One browser per agent, started on its first use
        start_command: String::new(),
        mcp_port: 0,
        description: description.to_string(),
        capabilities: vec![super::catalog::BROWSER.to_string()],
    };
    vec![
        definition(
            BROWSER,
            "Browser",
            format!("npm install -g {PACKAGE}"),
            "A browser window agents drive while you watch (Playwright MCP, with your Chrome or Edge). Only the sites in an agent's Internet access open.",
        ),
        definition(
            HEADLESS,
            "Browser (headless)",
            format!("docker pull {IMAGE}"),
            "A browser agents drive in a sandbox, without a window (Playwright MCP). Only the sites in an agent's Internet access open.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::capability::PilotOptions;

    fn options(allow: &[&str]) -> PilotOptions {
        PilotOptions {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn identities_name_the_agent() {
        assert_eq!(identity(HEADLESS, "writer"), "browser-headless:writer");
        assert_eq!(agent_of_identity("browser-headless:writer"), Some("writer"));
        assert_eq!(agent_of_identity("browser:writer"), Some("writer"));
        for not in ["writer", "connector:x", "browser:", "browserx:writer"] {
            assert_eq!(agent_of_identity(not), None, "{not}");
        }
    }

    #[test]
    fn reading_and_moving_never_ask_and_the_rest_asks_unless_allowed() {
        let none: [String; 0] = [];
        let act = |tool: &str, args: Value, read_only: bool| action(tool, &args, read_only, &none);
        assert_eq!(act("browser_snapshot", json!({}), true), Action::Read);
        assert_eq!(
            act("browser_navigate", json!({"url": "https://a"}), false),
            Action::Navigate
        );
        assert_eq!(
            act("browser_click", json!({"element": "Next page link"}), false),
            Action::Interact
        );
        assert_eq!(
            act("browser_click", json!({"element": "Submit button"}), false),
            Action::Submit
        );
        assert_eq!(
            act("browser_click", json!({"element": "Place order"}), false),
            Action::Pay
        );
        assert_eq!(
            act(
                "browser_click",
                json!({"element": "Download report"}),
                false
            ),
            Action::Upload
        );
        // Whole words only
        assert_eq!(
            act(
                "browser_click",
                json!({"element": "Posted comments tab"}),
                false
            ),
            Action::Interact
        );
        assert_eq!(
            act(
                "browser_type",
                json!({"element": "Search", "text": "x", "submit": true}),
                false
            ),
            Action::Submit
        );
        assert_eq!(
            act("browser_press_key", json!({"key": "Enter"}), false),
            Action::Submit
        );
        assert_eq!(
            act("browser_file_upload", json!({"paths": []}), false),
            Action::Upload
        );
        assert_eq!(
            act(
                "browser_type",
                json!({"element": "Password field", "text": "x"}),
                false
            ),
            Action::Secret
        );
        let secret = ["SITE_PASSWORD".to_string()];
        assert_eq!(
            action(
                "browser_fill_form",
                &json!({"fields": [{"name": "Account", "value": "SITE_PASSWORD"}]}),
                false,
                &secret
            ),
            Action::Secret
        );

        assert!(!needs_approval(Action::Read, None));
        assert!(!needs_approval(Action::Navigate, None));
        for kind in [
            Action::Interact,
            Action::Submit,
            Action::Secret,
            Action::Upload,
            Action::Pay,
        ] {
            assert!(needs_approval(kind, None), "{kind:?}");
        }
        // Auto mode: skill.md allows them
        let auto = options(&["interact", "submit", "secrets"]);
        assert!(!needs_approval(Action::Interact, Some(&auto)));
        assert!(!needs_approval(Action::Submit, Some(&auto)));
        assert!(!needs_approval(Action::Secret, Some(&auto)));
        assert!(
            needs_approval(Action::Pay, Some(&auto)),
            "paying is its own allow"
        );
        assert!(needs_approval(Action::Upload, Some(&auto)));
    }

    fn sites(hosts: &[&str]) -> Capabilities {
        serde_yaml::from_str(&format!(
            "network:\n  allow:\n{}",
            hosts
                .iter()
                .map(|h| format!("    - host: \"{h}\"\n"))
                .collect::<String>()
        ))
        .unwrap()
    }

    #[test]
    fn only_allowed_sites_open() {
        let allowed = sites(&["example.com", "*.rust-lang.org"]);
        assert!(allowed_url(&allowed, "https://example.com/x").is_ok());
        assert!(allowed_url(&allowed, "https://doc.rust-lang.org/").is_ok());
        assert!(allowed_url(&allowed, "about:blank").is_ok());
        let refused = allowed_url(&allowed, "https://evil.example/").unwrap_err();
        assert!(
            refused.contains("isn't one of this agent's allowed sites"),
            "{refused}"
        );
        assert!(allowed_url(&allowed, "http://localhost:8080/").is_err());
        assert!(allowed_url(&allowed, "file:///etc/passwd").is_err());
        assert!(allowed_url(&allowed, "not a url").is_err());
        assert!(allowed_url(&sites(&[]), "https://example.com").is_err());
    }

    #[test]
    fn the_config_routes_everything_through_the_proxy() {
        let secrets = HashMap::from([("SITE_PASSWORD".to_string(), "hunter2".to_string())]);
        let settings = config(
            Mode::Container,
            "http://nandi-egress:11435",
            "tok",
            "/profile",
            "/agent/workdir/browser",
            &secrets,
        );
        let launch = &settings["browser"]["launchOptions"];
        assert_eq!(launch["headless"], true);
        assert_eq!(launch["proxy"]["server"], "http://nandi-egress:11435");
        assert_eq!(launch["proxy"]["password"], "tok");
        assert_eq!(launch["proxy"]["bypass"], "<-loopback>");
        let args: Vec<&str> = launch["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(args.contains(&"--force-webrtc-ip-handling-policy=disable_non_proxied_udp"));
        assert!(!args.contains(&"--test-type"), "no window, no warning bar");
        assert_eq!(settings["browser"]["userDataDir"], "/profile");
        assert_eq!(settings["secrets"]["SITE_PASSWORD"], "hunter2");
        assert_eq!(settings["imageResponses"], "omit");

        let window = config(
            Mode::Window("msedge"),
            "http://127.0.0.1:1",
            "t",
            "p",
            "o",
            &HashMap::new(),
        );
        assert_eq!(window["browser"]["launchOptions"]["headless"], false);
        assert_eq!(window["browser"]["launchOptions"]["channel"], "msedge");
        assert!(window["browser"]["launchOptions"]["args"]
            .as_array()
            .unwrap()
            .contains(&Value::from("--test-type")));
    }

    #[test]
    fn the_container_is_microsoft_s_image_on_the_agents_network() {
        let args = container_args(
            "n",
            HEADLESS,
            "writer",
            Path::new("/b"),
            Path::new("/b/profile"),
            Path::new("/w"),
        );
        let joined = args.join(" ");
        assert!(joined.contains(&format!(
            "--network={}",
            crate::services::egress::relay::AGENT_NETWORK
        )));
        assert!(joined.contains("--cap-drop=ALL --security-opt=no-new-privileges"));
        assert!(joined.contains("--pids-limit=512 --memory=2048m"));
        assert!(joined.contains("--shm-size=1024m"));
        assert!(joined.contains("--mount type=bind,source=/b,target=/config,readonly"));
        assert!(joined.ends_with(&format!("{IMAGE} --config {CONTAINER_CONFIG}")));
        assert!(!joined.contains("tok"), "no token on the command line");
    }

    #[test]
    fn the_probes_look_for_the_pinned_install() {
        assert_eq!(PACKAGE, format!("{PACKAGE_NAME}@{PACKAGE_VERSION}"));
        assert!(IMAGE.ends_with(&format!(":v{PACKAGE_VERSION}")));
        assert_eq!(
            probe(BROWSER),
            Some(Probe::NpmGlobal {
                package: "@playwright/mcp",
                version: "0.0.83"
            })
        );
        assert_eq!(probe(HEADLESS), Some(Probe::DockerImage(IMAGE)));
        assert_eq!(probe("opendesk"), None);
    }

    #[test]
    fn the_catalog_lists_both_browsers() {
        let ids: Vec<String> = definitions().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, [BROWSER, HEADLESS]);
        assert!(definitions()[1].install_command.contains(IMAGE));
        assert!(definitions()[0].install_command.contains(PACKAGE));
    }

    #[test]
    fn profiles_live_under_local_never_the_repo() {
        let home = Path::new("/home/u/.nandi");
        let profile = profile_dir(&home.join("local"), "cos", BROWSER);
        assert_eq!(
            profile,
            Path::new("/home/u/.nandi/local/browser/cos/browser/profile")
        );
        assert!(!profile.starts_with(home.join("repo")));
    }

    #[tokio::test]
    async fn signing_out_clears_only_that_profile() {
        let local = tempfile::tempdir().unwrap();
        let mine = profile_dir(local.path(), "cos", BROWSER);
        let other = profile_dir(local.path(), "shopper", BROWSER);
        for dir in [&mine, &other] {
            std::fs::create_dir_all(dir.join("Default")).unwrap();
            std::fs::write(dir.join("Default/Cookies"), "c").unwrap();
        }
        assert!(clear_profile(local.path(), "cos", BROWSER).await.unwrap());
        assert!(!mine.exists());
        assert!(browser_base(local.path(), "cos", BROWSER).exists());
        assert!(other.join("Default/Cookies").exists(), "others stay");
        assert!(
            !clear_profile(local.path(), "cos", BROWSER).await.unwrap(),
            "nothing left to clear"
        );
    }

    #[test]
    fn secret_values_never_reach_a_reply() {
        let secrets = HashMap::from([
            ("SITE_PASSWORD".to_string(), "hunter2-xyz".to_string()),
            ("EMPTY".to_string(), String::new()),
        ]);
        let reply = "- textbox \"Password\" [value=hunter2-xyz]
Typed hunter2-xyz"
            .to_string();
        let masked = mask_secrets(reply, &secrets);
        assert!(!masked.contains("hunter2-xyz"), "{masked}");
        assert!(masked.contains("[value=[SITE_PASSWORD]]"));
        assert_eq!(mask_secrets("plain".into(), &secrets), "plain");
    }

    #[test]
    fn backups_leave_out_every_browser_profile() {
        let local = Path::new("/h/.nandi/local");
        let exclude = |folder: &str| profile_backup_exclude(Path::new(folder), local);
        assert_eq!(
            exclude("/h/.nandi").as_deref(),
            Some("local/browser/*/*/profile/*")
        );
        assert_eq!(
            exclude("/h/.nandi/local").as_deref(),
            Some("browser/*/*/profile/*")
        );
        assert_eq!(
            exclude("/h/.nandi/local/browser").as_deref(),
            Some("*/*/profile/*")
        );
        assert_eq!(
            exclude("/h/.nandi/local/browser/cos").as_deref(),
            Some("*/profile/*")
        );
        assert_eq!(
            exclude("/h/.nandi/local/browser/cos/browser").as_deref(),
            Some("profile/*")
        );
        assert_eq!(
            exclude("/h/.nandi/local/browser/cos/browser/profile/Default").as_deref(),
            Some("*")
        );
        assert_eq!(exclude("/h/Documents"), None);
        assert_eq!(exclude("/h/.nandi/repo"), None);
    }
}
