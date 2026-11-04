//! Which harness CLIs are on this machine, their versions, and how they're
//! configured. Each CLI keeps its own config (`~/.claude/settings.json`,
//! `~/.codex/config.toml`, `~/.gemini/settings.json`); Nandi only reads it
//! to show the provider and sign-in state, and never writes it.
//!
//! - `which` finds the executable on PATH; `--version` gives the version
//! - The provider comes from the CLI's own settings and the environment,
//!   the same switches the CLI itself reads (e.g. Claude Code's
//!   `CLAUDE_CODE_USE_BEDROCK`)
//! - "Authenticated" is a best guess from what's on disk or in the env; the
//!   Test connection check (`HarnessService::test`) is authoritative

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::adapter::{parse_version, HarnessAuth, HarnessAuthKind, HarnessHealth, HarnessRuntime};
use crate::db::{now_ms, ts, Db};

/// `--version` must answer within this
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// The environment detection reads; `current()` outside tests
#[derive(Debug, Clone, Default)]
pub struct DetectEnv {
    pub home: PathBuf,
    /// `None`: the process PATH
    pub path: Option<OsString>,
    pub vars: HashMap<String, String>,
}

impl DetectEnv {
    pub fn current() -> Self {
        Self {
            home: dirs::home_dir().unwrap_or_default(),
            path: None,
            vars: std::env::vars().collect(),
        }
    }

    fn var(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.trim().is_empty())
    }
}

/// One built-in harness: what to look for
#[derive(Debug, Clone, Copy)]
pub struct HarnessSpec {
    pub runtime_id: &'static str,
    pub display_name: &'static str,
    /// The CLI users install (and sign in with)
    pub executables: &'static [&'static str],
    /// The CLI's own config folder under `~`
    pub config_dir: &'static str,
}

pub const SPECS: [HarnessSpec; 3] = [
    HarnessSpec {
        runtime_id: "claude-code",
        display_name: "Claude Code",
        executables: &["claude", "claude-code"],
        config_dir: ".claude",
    },
    HarnessSpec {
        runtime_id: "codex",
        display_name: "Codex",
        executables: &["codex"],
        config_dir: ".codex",
    },
    HarnessSpec {
        runtime_id: "gemini-cli",
        display_name: "Gemini CLI",
        executables: &["gemini"],
        config_dir: ".gemini",
    },
];

pub fn spec(runtime_id: &str) -> Option<&'static HarnessSpec> {
    SPECS.iter().find(|s| s.runtime_id == runtime_id)
}

/// One harness, as last detected (a `detected_harnesses` row)
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub harness_id: String,
    pub executable: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub health: HarnessHealth,
}

pub struct HarnessDetector {
    db: Db,
    env: DetectEnv,
}

impl HarnessDetector {
    pub fn new(db: Db, env: DetectEnv) -> Self {
        Self { db, env }
    }

    pub fn env(&self) -> &DetectEnv {
        &self.env
    }

    /// Probes every built-in harness and stores the results
    pub async fn detect_all(&self) -> Result<Vec<Detection>, String> {
        let probes = SPECS.iter().map(|spec| detect(spec, &self.env));
        let detections = futures_util::future::join_all(probes).await;
        let rows = detections.clone();
        self.db.call(move |conn| save(conn, &rows)).await?;
        Ok(detections)
    }

    /// The stored detections (empty before the first run)
    pub async fn stored(&self) -> Result<Vec<Detection>, String> {
        let mut detections = self.db.call(load).await?;
        detections
            .iter_mut()
            .for_each(|d| self.recheck_container(d));
        Ok(detections)
    }

    pub async fn stored_one(&self, harness_id: &str) -> Result<Option<Detection>, String> {
        let id = harness_id.to_string();
        let mut detection = self.db.call(move |conn| load_one(conn, &id)).await?;
        if let Some(d) = detection.as_mut() {
            self.recheck_container(d);
        }
        Ok(detection)
    }

    /// Container sign-in and the auth method aren't stored (a token can
    /// expire, a login change, between checks)
    fn recheck_container(&self, detection: &mut Detection) {
        detection.health.container_authenticated = container_auth(&detection.harness_id, &self.env);
        detection.health.auth = auth_of(&detection.harness_id, &self.env);
    }
}

/// Probes one harness: PATH, `--version`, its config folder
pub async fn detect(spec: &HarnessSpec, env: &DetectEnv) -> Detection {
    let executable = find_executable(spec.executables, env);
    let config_dir = Some(env.home.join(spec.config_dir)).filter(|d| d.is_dir());
    let (provider, host_authenticated) = provider_and_auth(spec.runtime_id, env);
    let mut health = HarnessHealth {
        installed: executable.is_some(),
        version: None,
        host_authenticated,
        container_authenticated: container_auth(spec.runtime_id, env),
        configured_provider: provider,
        auth: auth_of(spec.runtime_id, env),
        last_check: ts(now_ms()),
        error: None,
    };
    if let Some(path) = &executable {
        match version_of(path).await {
            Ok(version) => health.version = version,
            Err(error) => health.error = Some(error),
        }
    }
    Detection {
        harness_id: spec.runtime_id.to_string(),
        executable,
        config_dir,
        health,
    }
}

pub fn find_executable(names: &[&str], env: &DetectEnv) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    names.iter().find_map(|name| match &env.path {
        Some(path) => which::which_in(name, Some(path), &cwd).ok(),
        None => which::which(name).ok(),
    })
}

/// `<path> --version`, parsed to `x.y.z`
pub async fn version_of(path: &Path) -> Result<Option<String>, String> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("{} --version did not answer", path.display()))?
        .map_err(|e| format!("Could not run {}: {e}", path.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(parse_version(&text).map(|v| v.to_string()))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn truthy(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "True" | "yes"))
}

/// Claude Code's environment: its `settings.json` `env` block under the
/// process environment (which wins, as in Claude Code). Only the
/// non-secret switches are ever forwarded from here (see `claude_code`).
pub fn claude_code_env(env: &DetectEnv) -> HashMap<String, String> {
    let mut merged: HashMap<String, String> = read_json(&env.home.join(".claude/settings.json"))
        .and_then(|s| s.get("env").cloned())
        .and_then(|e| e.as_object().cloned())
        .map(|object| {
            object
                .into_iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    merged.extend(env.vars.iter().map(|(k, v)| (k.clone(), v.clone())));
    merged
}

/// A token a container can sign in to Claude Code with:
/// `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`) in the environment
/// or Claude Code's settings, else the access token of the host's own
/// sign-in (`~/.claude/.credentials.json`, or on macOS the keychain item
/// Claude Code keeps it in) while it has at least `OAUTH_MIN_REMAINING`
/// left. The refresh token is never passed on: the host keeps refreshing.
pub fn claude_code_oauth_token(env: &DetectEnv) -> Option<String> {
    if let Some(token) = setup_token(env) {
        return Some(token);
    }
    let credentials =
        read_json(&env.home.join(".claude/.credentials.json")).or_else(keychain::read_login)?;
    fresh_access_token(&credentials)
}

/// `CLAUDE_CODE_OAUTH_TOKEN` in the environment or Claude Code's settings
fn setup_token(env: &DetectEnv) -> Option<String> {
    claude_code_env(env)
        .remove("CLAUDE_CODE_OAUTH_TOKEN")
        .filter(|t| !t.trim().is_empty())
}

fn fresh_access_token(credentials: &Value) -> Option<String> {
    let oauth = credentials.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?.trim();
    // `expiresAt` is in ms; without one the token is taken as long-lived
    let expires_at = oauth.get("expiresAt").and_then(Value::as_i64);
    let fresh = expires_at.is_none_or(|at| at - now_ms() >= OAUTH_MIN_REMAINING_MS);
    (!token.is_empty() && fresh).then(|| token.to_string())
}

/// Whether `claude_code_oauth_token` would find a token, without reading
/// the macOS keychain item (reading it can prompt; health checks run often)
fn has_oauth_token(env: &DetectEnv) -> bool {
    if setup_token(env).is_some() {
        return true;
    }
    match read_json(&env.home.join(".claude/.credentials.json")) {
        Some(credentials) => fresh_access_token(&credentials).is_some(),
        None => keychain::has_login(),
    }
}

/// Claude Code's login in the macOS login keychain (`Claude Code-credentials`,
/// the same JSON as `~/.claude/.credentials.json`). Read with `security`, so
/// macOS asks the user before the first read. Nothing elsewhere, or in tests.
mod keychain {
    use serde_json::Value;

    #[cfg(all(target_os = "macos", not(test)))]
    const SERVICE: &str = "Claude Code-credentials";

    #[cfg(all(target_os = "macos", not(test)))]
    fn security(args: &[&str]) -> Option<std::process::Output> {
        std::process::Command::new("/usr/bin/security")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
    }

    /// The item's attributes only: no secret, no prompt
    pub fn has_login() -> bool {
        #[cfg(all(target_os = "macos", not(test)))]
        {
            security(&["find-generic-password", "-s", SERVICE]).is_some()
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            false
        }
    }

    pub fn read_login() -> Option<Value> {
        #[cfg(all(target_os = "macos", not(test)))]
        {
            let output = security(&["find-generic-password", "-s", SERVICE, "-w"])?;
            serde_json::from_slice(&output.stdout).ok()
        }
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            None
        }
    }
}

/// A run must be able to finish before the host's access token expires
const OAUTH_MIN_REMAINING_MS: i64 = 15 * 60 * 1000;

/// Whether a container session can sign in from what's on this machine:
/// keys or provider switches in the environment (and, for Claude Code, its
/// settings or a subscription token). The CLIs' own sign-in files stay on
/// the host. Nandi's keyring keys are added by the caller.
pub fn container_auth(runtime_id: &str, env: &DetectEnv) -> bool {
    match runtime_id {
        "claude-code" => super::claude_code::forwards_credentials(env) || has_oauth_token(env),
        "codex" => ["OPENAI_API_KEY", "CODEX_API_KEY", "AZURE_OPENAI_API_KEY"]
            .iter()
            .any(|name| env.var(name).is_some()),
        "gemini-cli" => ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
            .iter()
            .any(|name| env.var(name).is_some()),
        _ => false,
    }
}

fn aws_configured(vars: &HashMap<String, String>, home: &Path) -> bool {
    let set = |name: &str| vars.get(name).is_some_and(|v| !v.trim().is_empty());
    set("AWS_BEARER_TOKEN_BEDROCK")
        || set("AWS_PROFILE")
        || set("AWS_ACCESS_KEY_ID")
        || home.join(".aws/config").is_file()
        || home.join(".aws/credentials").is_file()
}

/// The provider the CLI is set up for, and whether it looks signed in
pub fn provider_and_auth(runtime_id: &str, env: &DetectEnv) -> (Option<String>, bool) {
    match runtime_id {
        "claude-code" => {
            let vars = claude_code_env(env);
            let flag = |name: &str| truthy(vars.get(name).map(String::as_str));
            if flag("CLAUDE_CODE_USE_BEDROCK") {
                (Some("bedrock".into()), aws_configured(&vars, &env.home))
            } else if flag("CLAUDE_CODE_USE_VERTEX") {
                (Some("vertex".into()), true)
            } else if flag("CLAUDE_CODE_USE_FOUNDRY") {
                (Some("azure".into()), true)
            } else {
                let signed_in = vars.contains_key("ANTHROPIC_API_KEY")
                    || vars.contains_key("ANTHROPIC_AUTH_TOKEN")
                    || env.home.join(".claude/.credentials.json").is_file()
                    || read_json(&env.home.join(".claude.json"))
                        .is_some_and(|c| c.get("oauthAccount").is_some());
                (Some("anthropic".into()), signed_in)
            }
        }
        "codex" => {
            let provider = std::fs::read_to_string(env.home.join(".codex/config.toml"))
                .ok()
                .and_then(|text| text.parse::<toml::Table>().ok())
                .and_then(|t| t.get("model_provider")?.as_str().map(str::to_string))
                .unwrap_or_else(|| "openai".to_string());
            let signed_in = env.var("OPENAI_API_KEY").is_some()
                || env.var("AZURE_OPENAI_API_KEY").is_some()
                || env.home.join(".codex/auth.json").is_file();
            (Some(provider), signed_in)
        }
        "gemini-cli" => {
            let settings = read_json(&env.home.join(".gemini/settings.json"));
            let auth_type = settings.as_ref().and_then(|s| {
                s.pointer("/security/auth/selectedType")
                    .or_else(|| s.get("selectedAuthType"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            let vertex = auth_type.as_deref() == Some("vertex-ai")
                || truthy(env.var("GOOGLE_GENAI_USE_VERTEXAI"));
            let signed_in = auth_type.is_some()
                || env.var("GEMINI_API_KEY").is_some()
                || env.var("GOOGLE_API_KEY").is_some()
                || env.home.join(".gemini/oauth_creds.json").is_file();
            (
                Some(if vertex { "vertex" } else { "google" }.to_string()),
                signed_in,
            )
        }
        _ => (None, false),
    }
}

/// How the CLI signs in, from the same switches `provider_and_auth` reads,
/// in the order the CLI itself picks them. `None`: nothing to sign in with.
pub fn auth_of(runtime_id: &str, env: &DetectEnv) -> Option<HarnessAuth> {
    let (provider, signed_in) = provider_and_auth(runtime_id, env);
    if !signed_in {
        return None;
    }
    let of = |kind| HarnessAuth {
        kind,
        ..HarnessAuth::default()
    };
    match runtime_id {
        "claude-code" => {
            let vars = claude_code_env(env);
            let flag = |name: &str| truthy(vars.get(name).map(String::as_str));
            let set = |name: &str| vars.get(name).is_some_and(|v| !v.trim().is_empty());
            Some(if flag("CLAUDE_CODE_USE_BEDROCK") {
                HarnessAuth {
                    kind: HarnessAuthKind::Bedrock,
                    aws_profile: vars.get("AWS_PROFILE").cloned(),
                    aws_region: vars
                        .get("AWS_REGION")
                        .or_else(|| vars.get("AWS_DEFAULT_REGION"))
                        .cloned(),
                    ..HarnessAuth::default()
                }
            } else if flag("CLAUDE_CODE_USE_VERTEX") {
                of(HarnessAuthKind::Vertex)
            } else if flag("CLAUDE_CODE_USE_FOUNDRY") {
                of(HarnessAuthKind::Azure)
            } else if set("ANTHROPIC_BASE_URL") {
                of(HarnessAuthKind::Gateway)
            } else if set("ANTHROPIC_API_KEY") || set("ANTHROPIC_AUTH_TOKEN") {
                of(HarnessAuthKind::ApiKey)
            } else {
                claude_subscription(env)
            })
        }
        "codex" => Some(
            if env.var("OPENAI_API_KEY").is_some() || env.var("AZURE_OPENAI_API_KEY").is_some() {
                of(HarnessAuthKind::ApiKey)
            } else {
                // `auth.json` holds a ChatGPT login (`tokens`) or a key
                let login = read_json(&env.home.join(".codex/auth.json"));
                let chatgpt = login
                    .as_ref()
                    .and_then(|l| l.get("tokens"))
                    .is_some_and(|t| !t.is_null());
                of(if chatgpt {
                    HarnessAuthKind::Subscription
                } else {
                    HarnessAuthKind::ApiKey
                })
            },
        ),
        "gemini-cli" => Some(
            if env.var("GEMINI_API_KEY").is_some() || env.var("GOOGLE_API_KEY").is_some() {
                of(HarnessAuthKind::ApiKey)
            } else if provider.as_deref() == Some("vertex") {
                of(HarnessAuthKind::Vertex)
            } else {
                of(HarnessAuthKind::Subscription)
            },
        ),
        _ => None,
    }
}

/// Claude Code's own account login: the plan from its credentials, the
/// organization from `~/.claude.json` (never a token)
fn claude_subscription(env: &DetectEnv) -> HarnessAuth {
    let credentials = read_json(&env.home.join(".claude/.credentials.json"));
    let account =
        read_json(&env.home.join(".claude.json")).and_then(|c| c.get("oauthAccount").cloned());
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let plan = text(
        credentials
            .as_ref()
            .and_then(|c| c.pointer("/claudeAiOauth/subscriptionType")),
    )
    .or_else(|| {
        text(account.as_ref().and_then(|a| a.get("organizationType")))
            .map(|t| t.trim_start_matches("claude_").to_string())
    });
    HarnessAuth {
        kind: HarnessAuthKind::Subscription,
        plan,
        organization: text(account.as_ref().and_then(|a| a.get("organizationName"))),
        ..HarnessAuth::default()
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn save(conn: &mut Connection, detections: &[Detection]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    for d in detections {
        tx.execute(
            "INSERT INTO detected_harnesses (harness_id, installed, executable, version,
                 config_dir, configured_provider, authenticated, error, detected_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(harness_id) DO UPDATE SET installed = ?2, executable = ?3,
                 version = ?4, config_dir = ?5, configured_provider = ?6,
                 authenticated = ?7, error = ?8, detected_at = ?9",
            params![
                d.harness_id,
                d.health.installed,
                d.executable.as_ref().map(|p| p.display().to_string()),
                d.health.version,
                d.config_dir.as_ref().map(|p| p.display().to_string()),
                d.health.configured_provider,
                d.health.host_authenticated,
                d.health.error,
                d.health.last_check as i64,
            ],
        )?;
    }
    tx.commit()
}

const COLUMNS: &str = "harness_id, installed, executable, version, config_dir,
                       configured_provider, authenticated, error, detected_at";

fn row(row: &rusqlite::Row) -> rusqlite::Result<Detection> {
    let detected_at: i64 = row.get(8)?;
    Ok(Detection {
        harness_id: row.get(0)?,
        executable: row.get::<_, Option<String>>(2)?.map(PathBuf::from),
        config_dir: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
        health: HarnessHealth {
            installed: row.get(1)?,
            version: row.get(3)?,
            host_authenticated: row.get(6)?,
            // Not stored: re-read from the environment on load
            container_authenticated: false,
            configured_provider: row.get(5)?,
            auth: None,
            last_check: ts(detected_at),
            error: row.get(7)?,
        },
    })
}

fn load(conn: &mut Connection) -> rusqlite::Result<Vec<Detection>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM detected_harnesses ORDER BY harness_id"
    ))?;
    let rows = stmt.query_map([], row)?.collect();
    rows
}

fn load_one(conn: &mut Connection, id: &str) -> rusqlite::Result<Option<Detection>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM detected_harnesses WHERE harness_id = ?1"),
        [id],
        row,
    )
    .optional()
}

/// The runtime for a detection's ID
pub fn runtime(harness_id: &str) -> Option<HarnessRuntime> {
    HarnessRuntime::parse(harness_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_env(vars: &[(&str, &str)]) -> (DetectEnv, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        let env = DetectEnv {
            home: home.path().to_path_buf(),
            path: Some(home.path().as_os_str().to_owned()),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        (env, home)
    }

    #[test]
    fn the_auth_method_comes_from_the_clis_own_setup() {
        let (none, _a) = home_env(&[]);
        assert_eq!(auth_of("claude-code", &none), None);

        let (login, home) = home_env(&[]);
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/.credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"secret","subscriptionType":"max"}}"#,
        )
        .unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"organizationName":"Acme","emailAddress":"a@b"}}"#,
        )
        .unwrap();
        let auth = auth_of("claude-code", &login).unwrap();
        assert_eq!(auth.kind, HarnessAuthKind::Subscription);
        assert_eq!(auth.plan.as_deref(), Some("max"));
        assert_eq!(auth.organization.as_deref(), Some("Acme"));
        assert!(!format!("{auth:?}").contains("secret"));

        let (bedrock, _b) = home_env(&[
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("AWS_PROFILE", "acme-dev"),
            ("AWS_REGION", "us-east-1"),
        ]);
        let auth = auth_of("claude-code", &bedrock).unwrap();
        assert_eq!(auth.kind, HarnessAuthKind::Bedrock);
        assert_eq!(auth.aws_profile.as_deref(), Some("acme-dev"));
        assert_eq!(auth.aws_region.as_deref(), Some("us-east-1"));

        let (key, _c) = home_env(&[("ANTHROPIC_API_KEY", "sk-ant")]);
        assert_eq!(
            auth_of("claude-code", &key).unwrap().kind,
            HarnessAuthKind::ApiKey
        );
    }

    /// A fake CLI in `dir` that prints `output` for any arguments
    fn fake_cli(dir: &Path, name: &str, output: &str) {
        #[cfg(windows)]
        std::fs::write(
            dir.join(format!("{name}.cmd")),
            format!("@echo off\r\necho {output}\r\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho '{output}'\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn env_with(bin: &Path, home: &Path, vars: &[(&str, &str)]) -> DetectEnv {
        DetectEnv {
            home: home.to_path_buf(),
            path: Some(bin.as_os_str().to_owned()),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[tokio::test]
    async fn finds_clis_on_a_mocked_path_and_reads_their_versions() {
        let bin = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        fake_cli(bin.path(), "claude", "2.1.301 (Claude Code)");
        fake_cli(bin.path(), "gemini", "0.64.0");
        let env = env_with(bin.path(), home.path(), &[]);

        let claude = detect(spec("claude-code").unwrap(), &env).await;
        assert!(claude.health.installed);
        assert_eq!(claude.health.version.as_deref(), Some("2.1.301"));
        let gemini = detect(spec("gemini-cli").unwrap(), &env).await;
        assert_eq!(gemini.health.version.as_deref(), Some("0.64.0"));
        let codex = detect(spec("codex").unwrap(), &env).await;
        assert!(!codex.health.installed);
        assert_eq!(codex.health.version, None);
        assert_eq!(codex.executable, None);
    }

    #[test]
    fn claude_code_bedrock_comes_from_its_own_settings() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1","AWS_REGION":"us-west-2","AWS_PROFILE":"work"}}"#,
        )
        .unwrap();
        let env = env_with(home.path(), home.path(), &[]);
        assert_eq!(
            provider_and_auth("claude-code", &env),
            (Some("bedrock".to_string()), true)
        );
        assert_eq!(
            claude_code_env(&env).get("AWS_REGION").map(String::as_str),
            Some("us-west-2")
        );
    }

    #[test]
    fn the_environment_overrides_claude_code_settings() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#,
        )
        .unwrap();
        let env = env_with(
            home.path(),
            home.path(),
            &[("CLAUDE_CODE_USE_BEDROCK", "0"), ("ANTHROPIC_API_KEY", "k")],
        );
        assert_eq!(
            provider_and_auth("claude-code", &env),
            (Some("anthropic".to_string()), true)
        );
    }

    #[test]
    fn codex_and_gemini_providers() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        std::fs::write(
            home.path().join(".codex/config.toml"),
            "model_provider = \"azure\"\n",
        )
        .unwrap();
        std::fs::write(
            home.path().join(".gemini/settings.json"),
            r#"{"security":{"auth":{"selectedType":"vertex-ai"}}}"#,
        )
        .unwrap();
        let env = env_with(home.path(), home.path(), &[]);
        assert_eq!(
            provider_and_auth("codex", &env),
            (Some("azure".to_string()), false)
        );
        assert_eq!(
            provider_and_auth("gemini-cli", &env),
            (Some("vertex".to_string()), true)
        );
    }

    #[tokio::test]
    async fn detections_round_trip_through_the_table() {
        let db = Db::in_memory();
        let bin = tempfile::tempdir().unwrap();
        let detector = HarnessDetector::new(db, env_with(bin.path(), bin.path(), &[]));
        assert!(detector.stored().await.unwrap().is_empty());
        let detected = detector.detect_all().await.unwrap();
        assert_eq!(detected.len(), 3);
        let stored = detector.stored().await.unwrap();
        assert_eq!(stored.len(), 3);
        let codex = detector.stored_one("codex").await.unwrap().unwrap();
        assert!(!codex.health.installed);
    }
}
