//! Network egress control for agent containers.
//!
//! One host server on `127.0.0.1:11435` is the only way out. Containers sit
//! on the internal `nandi-agents` network and reach it through the
//! `nandi-egress` relay at `nandi-egress:11435` ([`relay`]):
//!
//! - `POST /llm` — LLM completions with host-held provider keys
//! - `POST /aws/{service}/{action}` — AWS calls as the agent's AWS identity
//! - `POST /fetch` — HTTP requests to hosts in the agent's `network.allow`
//! - `CONNECT host:443` — HTTPS tunnels for `HTTPS_PROXY`, same allowlist
//!
//! Every route authenticates the agent by its per-container proxy token, and
//! checks the request against the `aws` / `network` capabilities its
//! skill.md asks for, narrowed by its policies (re-read per request, so
//! edits apply immediately); a stdio connector's container (`connector:<id>`)
//! may reach its package registries and `config.network`. AWS, fetch,
//! and tunnel requests are audited to `~/.nandi/local/logs/egress.log`.

mod aws_proxy;
pub mod capabilities;
pub mod credentials;
mod http_proxy;
mod llm_proxy;
pub mod relay;
mod tunnel;

use std::convert::Infallible;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::Router;
use serde::{Deserialize, Serialize};
use specta::Type;
use tower::ServiceExt;

use crate::audit::{self, AuditLog};
use crate::model::Timestamp;
use crate::runtime::ProxyTokens;
use crate::services::agents::SkillConfig;
use crate::services::secrets::Secrets;
use crate::services::AppState;

use capabilities::{AwsMode, Capabilities};
use credentials::{CredentialsManager, SessionUse};
use llm_proxy::error;

/// The port the proxy tries first. Another process (e.g. a second Nandi
/// with its own `NANDI_HOME`) may hold it; the proxy then takes a free one.
pub const PROXY_PORT: u16 = 11435;

/// The port the proxy actually listens on (`PROXY_PORT` until it binds
/// elsewhere). Containers are pointed at it, so it's set before any start.
static BOUND_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(PROXY_PORT);

/// Where this process's proxy listens
pub fn proxy_port() -> u16 {
    BOUND_PORT.load(std::sync::atomic::Ordering::Relaxed)
}
/// STS sessions injected into `sts`-mode containers
const CONTAINER_SESSION_SECS: u32 = 12 * 60 * 60;
const RECENT_LOG_ENTRIES: usize = 20;
/// Only the tail of egress.log is read for the status view
const LOG_TAIL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EgressKind {
    Llm,
    Aws,
    Fetch,
    Tunnel,
}

/// A route the proxy serves.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EgressRoute {
    pub kind: EgressKind,
    pub method: String,
    pub path: String,
}

/// Whether AWS is set up. Never includes the secret.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AwsCredentialStatus {
    pub configured: bool,
    /// e.g. `AKIA…WXYZ`
    pub access_key_hint: Option<String>,
    pub region: String,
    /// Cached STS sessions that haven't expired
    pub active_sessions: u32,
}

/// One line of `~/.nandi/local/logs/egress.log`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EgressLogEntry {
    pub ts: Timestamp,
    pub agent_id: String,
    pub kind: EgressKind,
    /// `GET https://host/path` (no query), `s3:GetObject arn:…`, or
    /// `host:port` — never credentials
    pub target: String,
    pub allowed: bool,
    pub status: Option<u16>,
    pub duration_ms: Option<f64>,
    pub error: Option<String>,
}

/// What the egress layer is currently allowing.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EgressStatus {
    /// Host loopback URL of the proxy
    pub proxy_url: String,
    pub routes: Vec<EgressRoute>,
    pub aws: AwsCredentialStatus,
    /// Newest first
    pub recent_entries: Vec<EgressLogEntry>,
}

/// Runtime state for egress control.
pub struct EgressState {
    credentials: CredentialsManager,
}

impl EgressState {
    pub fn new(secrets: Secrets) -> Self {
        Self {
            credentials: CredentialsManager::new(secrets),
        }
    }

    pub fn credentials(&self) -> &CredentialsManager {
        &self.credentials
    }

    /// Environment for an agent's container: proxy endpoints, and for
    /// `sts`-mode agents, credentials scoped to their AWS grants.
    pub async fn container_env(
        &self,
        skill: &SkillConfig,
        proxy_token: &str,
    ) -> Result<Vec<String>, String> {
        let mut env = proxy_env(proxy_token);
        let Some(aws) = &skill.capabilities.aws else {
            return Ok(env);
        };
        if aws.mode != AwsMode::Sts {
            return Ok(env);
        }
        let policy = skill.capabilities.session_policy().ok_or_else(|| {
            format!(
                "Agent '{}' uses AWS sts mode but grants no AWS actions",
                skill.id
            )
        })?;
        let session = self
            .credentials
            .session(
                &skill.id,
                aws.profile.as_deref(),
                &policy,
                CONTAINER_SESSION_SECS,
                SessionUse::Container,
            )
            .await
            .map_err(|e| format!("Agent '{}' needs AWS credentials: {e}", skill.id))?;
        let (session, region) = (session.credentials, session.region);
        env.extend([
            format!("AWS_ACCESS_KEY_ID={}", session.access_key),
            format!("AWS_SECRET_ACCESS_KEY={}", session.secret_key),
            format!("AWS_SESSION_TOKEN={}", session.session_token),
            format!("AWS_REGION={region}"),
            format!("AWS_DEFAULT_REGION={region}"),
        ]);
        Ok(env)
    }
}

/// Proxy endpoints for a container, through the relay. `HTTPS_PROXY`
/// carries the agent's proxy token as Basic credentials, so tunnels are
/// attributed like other routes. The relay and the LiteLLM gateway are on
/// the agents' network and reached directly.
pub fn proxy_env(proxy_token: &str) -> Vec<String> {
    let host = relay::RELAY_NAME;
    let port = relay::RELAY_PORT;
    let base = format!("http://{host}:{port}");
    let https_proxy = format!("http://nandi:{proxy_token}@{host}:{port}");
    let no_proxy = format!(
        "{host},{},localhost,127.0.0.1",
        crate::services::harness::gateway::CONTAINER_NAME
    );
    vec![
        format!("NANDI_LLM_PROXY={base}/llm"),
        // Read by the runtime image
        format!("NANDI_LLM_PROXY_URL={base}/llm"),
        format!("NANDI_LLM_PROXY_TOKEN={proxy_token}"),
        format!("NANDI_AWS_PROXY={base}/aws"),
        format!("NANDI_FETCH_PROXY={base}/fetch"),
        format!("HTTPS_PROXY={https_proxy}"),
        format!("https_proxy={https_proxy}"),
        // Plain HTTP too: nothing on the agents' network gets out otherwise
        format!("HTTP_PROXY={https_proxy}"),
        format!("http_proxy={https_proxy}"),
        format!("NO_PROXY={no_proxy}"),
        format!("no_proxy={no_proxy}"),
    ]
}

/// Hosts a running harness session's CLI needs for its own model calls
/// (`api.anthropic.com` for Claude Code, …), by agent. The tunnel allows
/// them on top of the agent's `network.allow` while the session runs.
static SESSION_HOSTS: std::sync::LazyLock<
    std::sync::RwLock<std::collections::HashMap<String, Vec<capabilities::HostRule>>>,
> = std::sync::LazyLock::new(Default::default);

/// Lets `agent_id`'s session reach `hosts` until [`clear_session_hosts`]
pub fn allow_session_hosts(agent_id: &str, hosts: &[String]) {
    let rules = hosts
        .iter()
        .map(|host| capabilities::HostRule {
            host: host.clone(),
            methods: Vec::new(),
        })
        .collect();
    SESSION_HOSTS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(agent_id.to_string(), rules);
}

pub fn clear_session_hosts(agent_id: &str) {
    SESSION_HOSTS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(agent_id);
}

fn session_hosts(agent_id: &str) -> Option<Vec<capabilities::HostRule>> {
    SESSION_HOSTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent_id)
        .cloned()
}

/// `capabilities` plus the hosts `agent_id`'s running session needs
fn with_session_hosts(agent_id: &str, mut capabilities: Capabilities) -> Capabilities {
    if let Some(hosts) = SESSION_HOSTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent_id)
    {
        capabilities.network.allow.extend(hosts.iter().cloned());
    }
    capabilities
}

/// Where a package runner fetches the server from (`npx`, `uvx`)
const PACKAGE_REGISTRIES: &[&str] = &["registry.npmjs.org", "pypi.org", "files.pythonhosted.org"];

/// What a stdio connector's container may reach: its package registry and
/// the hosts in `config.network` (connectors.yaml)
pub fn connector_hosts(decl: &crate::services::domain::connector::Connector) -> Vec<String> {
    PACKAGE_REGISTRIES
        .iter()
        .map(|h| h.to_string())
        .chain(crate::services::connectors::catalog_config_list(
            decl, "network",
        ))
        .map(|host| host.trim().to_ascii_lowercase())
        .collect()
}

/// [`connector_hosts`] of an installed connector. One not installed yet
/// (Studio starting it once to list its tools) reaches only what that run
/// was allowed ([`allow_session_hosts`]).
fn connector_capabilities(
    store: &crate::services::repo_store::RepoStore,
    id: &str,
) -> Result<Capabilities, String> {
    let Some(decl) = store.read(|repo| repo.connectors().get(id).cloned()) else {
        let identity = crate::services::connectors::proxy_identity(id);
        return session_hosts(&identity)
            .map(|allow| Capabilities {
                aws: None,
                network: capabilities::NetworkCapabilities { allow },
            })
            .ok_or_else(|| format!("Connector '{id}' isn't installed"));
    };
    Ok(Capabilities {
        aws: None,
        network: capabilities::NetworkCapabilities {
            allow: connector_hosts(&decl)
                .into_iter()
                .map(|host| capabilities::HostRule {
                    host,
                    methods: Vec::new(),
                })
                .collect(),
        },
    })
}

fn routes() -> Vec<EgressRoute> {
    [
        (EgressKind::Llm, "POST", "/llm"),
        (EgressKind::Aws, "POST", "/aws/{service}/{action}"),
        (EgressKind::Fetch, "POST", "/fetch"),
        (EgressKind::Tunnel, "CONNECT", "host:443"),
    ]
    .into_iter()
    .map(|(kind, method, path)| EgressRoute {
        kind,
        method: method.to_string(),
        path: path.to_string(),
    })
    .collect()
}

pub async fn get_egress_status(state: &AppState) -> Result<EgressStatus, String> {
    let manager = state.egress().credentials();
    let secrets = manager.secrets();
    let creds = credentials::get_aws_credentials(secrets)?;
    let recent_entries = match AuditLog::Egress.path() {
        Ok(path) => tokio::task::spawn_blocking(move || read_recent(&path, RECENT_LOG_ENTRIES))
            .await
            .map_err(|e| format!("Failed to read the egress log: {e}"))?,
        Err(_) => Vec::new(),
    };
    Ok(EgressStatus {
        proxy_url: format!("http://127.0.0.1:{}", proxy_port()),
        routes: routes(),
        aws: AwsCredentialStatus {
            configured: creds.is_some(),
            access_key_hint: creds.map(|c| credentials::redact(&c.access_key)),
            region: credentials::get_aws_region(secrets)?,
            active_sessions: manager.active_sessions() as u32,
        },
        recent_entries,
    })
}

/// Stores the long-term AWS key; empty values remove it. Cached sessions
/// were issued from the old key, so they're dropped.
pub async fn set_aws_credentials(
    state: &AppState,
    access_key: String,
    secret: String,
) -> Result<(), String> {
    let manager = state.egress().credentials();
    credentials::set_aws_credentials(manager.secrets(), &access_key, &secret)?;
    manager.clear_sessions();
    log::info!("AWS credentials updated");
    Ok(())
}

pub async fn set_aws_region(state: &AppState, region: String) -> Result<(), String> {
    let manager = state.egress().credentials();
    credentials::set_aws_region(manager.secrets(), &region)?;
    manager.clear_sessions();
    Ok(())
}

/// The last `limit` parseable entries, newest first. A missing log is empty.
fn read_recent(path: &std::path::Path, limit: usize) -> Vec<EgressLogEntry> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or_default();
    let start = len.saturating_sub(LOG_TAIL_BYTES);
    let mut tail = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut tail).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&tail);
    let mut lines = text.lines();
    if start > 0 {
        // Partial first line
        lines.next();
    }
    let mut entries: Vec<EgressLogEntry> = lines
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

async fn record(entry: EgressLogEntry) {
    if entry.allowed {
        log::debug!("Egress: agent '{}' → {}", entry.agent_id, entry.target);
    } else {
        log::warn!(
            "Egress denied: agent '{}' → {} ({})",
            entry.agent_id,
            entry.target,
            entry.error.as_deref().unwrap_or_default()
        );
    }
    audit::record(AuditLog::Egress, &entry).await;
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Loads an agent's capabilities (from skill.md in production).
pub(crate) type CapabilitiesFn = Arc<dyn Fn(&str) -> Result<Capabilities, String> + Send + Sync>;

/// What the AWS, fetch, and tunnel routes share: who's calling, and what
/// they may do.
#[derive(Clone)]
pub(crate) struct ProxyContext {
    tokens: ProxyTokens,
    capabilities: CapabilitiesFn,
}

impl ProxyContext {
    /// The agent behind a bearer token. A body that names a different
    /// agent is refused: identity comes from the token only.
    #[allow(clippy::result_large_err)]
    fn authenticate(&self, headers: &HeaderMap, claimed: Option<&str>) -> Result<String, Response> {
        let agent_id = llm_proxy::bearer_token(headers)
            .and_then(|token| self.tokens.agent_for(token))
            .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "missing or invalid proxy token"))?;
        match claimed {
            Some(claimed) if !claimed.is_empty() && claimed != agent_id => Err(error(
                StatusCode::FORBIDDEN,
                "agent_id does not match the proxy token",
            )),
            _ => Ok(agent_id),
        }
    }

    #[allow(clippy::result_large_err)]
    fn capabilities(&self, agent_id: &str) -> Result<Capabilities, Response> {
        (self.capabilities)(agent_id).map_err(|e| error(StatusCode::FORBIDDEN, &e))
    }
}

/// Binds `127.0.0.1:PROXY_PORT`, or any free port when that one is taken
pub fn bind_proxy_listener() -> std::io::Result<std::net::TcpListener> {
    std::net::TcpListener::bind(("127.0.0.1", PROXY_PORT)).or_else(|e| {
        log::warn!("Egress proxy port {PROXY_PORT} is taken ({e}); using a free port");
        std::net::TcpListener::bind(("127.0.0.1", 0))
    })
}

/// Starts the proxy in the background. Binds first, on the calling thread,
/// so `proxy_env` names the right port before any container starts. A bind
/// failure is logged, not fatal: agents just can't reach the outside.
pub fn start_server(state: &AppState) {
    let context = ProxyContext {
        tokens: state.proxy_tokens().clone(),
        capabilities: {
            let store = state.store().clone();
            Arc::new(move |agent_id| {
                // A stdio connector's container (`connectors::mcp`)
                if let Some(id) = crate::services::connectors::connector_of_identity(agent_id) {
                    return connector_capabilities(&store, id);
                }
                // A browser pilot (`browser:<agent>`): that agent's own sites,
                // plus any a person allowed once in its current run
                if let Some(agent) = crate::services::pilots::browser::agent_of_identity(agent_id) {
                    return store.skill(agent).map(|s| {
                        crate::services::pilots::sites::with_run_sites(agent, s.capabilities)
                    });
                }
                store
                    .skill(agent_id)
                    .map(|s| with_session_hosts(agent_id, s.capabilities))
            })
        },
    };
    let router = llm_proxy::router(llm_proxy::ProxyState::new(state))
        .merge(aws_proxy::router(context.clone(), state.egress().clone()))
        .merge(http_proxy::router(context.clone()));

    let listener = match bind_proxy_listener().and_then(|listener| {
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok((listener, port))
    }) {
        Ok((listener, port)) => {
            BOUND_PORT.store(port, std::sync::atomic::Ordering::Relaxed);
            listener
        }
        Err(e) => {
            log::error!("Egress proxy could not bind 127.0.0.1: {e}");
            return;
        }
    };
    state.runtime().spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(e) => {
                log::error!("Egress proxy could not start: {e}");
                return;
            }
        };
        log::info!("Egress proxy listening on 127.0.0.1:{}", proxy_port());
        if let Err(e) = serve(listener, router, context).await {
            log::error!("Egress proxy stopped: {e}");
        }
    });
}

/// Serves `router`, plus CONNECT tunnels (which can't be routed by path).
async fn serve(
    listener: tokio::net::TcpListener,
    router: Router,
    context: ProxyContext,
) -> std::io::Result<()> {
    let service = tower::service_fn(move |request: Request<Body>| {
        let router = router.clone();
        let context = context.clone();
        async move {
            if request.method() == Method::CONNECT {
                Ok::<_, Infallible>(tunnel::handle_connect(context, request).await)
            } else {
                router.oneshot(request).await
            }
        }
    });
    axum::serve(
        listener,
        axum::ServiceExt::<Request<Body>>::into_make_service(service),
    )
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::services::egress::capabilities::{HostRule, NetworkCapabilities};

    /// A context whose agents all have `capabilities`.
    pub fn context(tokens: ProxyTokens, capabilities: Capabilities) -> ProxyContext {
        ProxyContext {
            tokens,
            capabilities: Arc::new(move |_| Ok(capabilities.clone())),
        }
    }

    pub fn allow_hosts(hosts: &[&str]) -> Capabilities {
        Capabilities {
            network: NetworkCapabilities {
                allow: hosts
                    .iter()
                    .map(|host| HostRule {
                        host: host.to_string(),
                        methods: vec![],
                    })
                    .collect(),
            },
            ..Default::default()
        }
        .validate()
        .unwrap()
    }

    fn skill(capabilities: Capabilities) -> SkillConfig {
        SkillConfig {
            id: "writer".into(),
            dir: "/a/writer".into(),
            model: "ollama:llama3.2".into(),
            image: None,
            memory_mb: None,
            cpu: None,
            shell_timeout_s: None,
            instructions: String::new(),
            allowed_tools: None,
            requires_approval: vec![],
            capabilities,
            folders: Vec::new(),
            denied_tools: vec![],
            connectors: vec![],
            connectors_on_host: vec![],
            pilots: vec![],
            browser_refusals: vec![],
            policies: vec![],
            model_rules: vec![],
            connector_forbid: vec![],
            connector_ask: vec![],
        }
    }

    #[test]
    fn falls_back_to_a_free_port_when_the_default_is_taken() {
        // Whoever holds PROXY_PORT (this test, or another process)
        let _held = std::net::TcpListener::bind(("127.0.0.1", PROXY_PORT));
        let listener = bind_proxy_listener().unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_ne!(port, PROXY_PORT);
    }

    #[test]
    fn a_session_s_hosts_are_allowed_only_while_it_runs() {
        let caps = || with_session_hosts("session-agent", Capabilities::default());
        assert!(!caps().allows_tunnel("api.anthropic.com", 443));
        allow_session_hosts("session-agent", &["api.anthropic.com".to_string()]);
        assert!(caps().allows_tunnel("api.anthropic.com", 443));
        assert!(!caps().allows_tunnel("evil.example", 443));
        assert!(!with_session_hosts("other", Capabilities::default())
            .allows_tunnel("api.anthropic.com", 443));
        clear_session_hosts("session-agent");
        assert!(!caps().allows_tunnel("api.anthropic.com", 443));
    }

    #[test]
    fn a_connector_container_reaches_its_registries_and_its_own_hosts() {
        let store = crate::services::repo_store::RepoStore::for_tests(crate::db::Db::in_memory());
        let path = store.layout().connectors_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "connectors:
  - id: notes
    kind: mcp
    config:
      command: npx
      network: [' API.Notes.dev ']
",
        )
        .unwrap();
        store.load().unwrap();

        let caps = connector_capabilities(&store, "notes").unwrap();
        let hosts: Vec<&str> = caps.network.allow.iter().map(|r| r.host.as_str()).collect();
        assert_eq!(
            hosts,
            [
                "registry.npmjs.org",
                "pypi.org",
                "files.pythonhosted.org",
                "api.notes.dev"
            ]
        );
        assert!(caps.network.allow.iter().all(|r| r.methods.is_empty()));
        assert!(caps.aws.is_none());
        assert!(caps.allows_tunnel("api.notes.dev", 443));
        assert!(!caps.allows_tunnel("evil.example", 443));

        let missing = connector_capabilities(&store, "ghost").unwrap_err();
        assert!(missing.contains("isn't installed"), "{missing}");

        // Studio's one start of a connector not installed yet
        allow_session_hosts("connector:ghost", &["pypi.org".to_string()]);
        let probe = connector_capabilities(&store, "ghost").unwrap();
        clear_session_hosts("connector:ghost");
        assert!(probe.allows_tunnel("pypi.org", 443));
        assert!(!probe.allows_tunnel("api.notes.dev", 443));
        assert!(connector_capabilities(&store, "ghost").is_err());
    }

    #[test]
    fn proxy_env_points_at_the_relay() {
        let env = proxy_env("tok");
        for expected in [
            "NANDI_LLM_PROXY=http://nandi-egress:11435/llm",
            "NANDI_AWS_PROXY=http://nandi-egress:11435/aws",
            "NANDI_LLM_PROXY_TOKEN=tok",
            "HTTPS_PROXY=http://nandi:tok@nandi-egress:11435",
            "HTTP_PROXY=http://nandi:tok@nandi-egress:11435",
        ] {
            assert!(env.contains(&expected.to_string()), "{expected}: {env:?}");
        }
        assert!(env
            .iter()
            .any(|v| v == "NO_PROXY=nandi-egress,nandi-litellm,localhost,127.0.0.1"));
    }

    #[tokio::test]
    async fn sts_agents_get_scoped_credentials() {
        let secrets = Secrets::in_memory();
        credentials::set_aws_credentials(&secrets, "AKIAEXAMPLE12345", "s3cr3t").unwrap();
        let policies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = policies.clone();
        let state = EgressState {
            credentials: CredentialsManager::with_issuer(
                secrets,
                Arc::new(move |_creds, _name, policy, _duration| {
                    seen.lock().unwrap().push(policy);
                    Box::pin(async {
                        Ok(credentials::StsCredentials {
                            access_key: "ASIATEMP".into(),
                            secret_key: "temp-secret".into(),
                            session_token: "temp-token".into(),
                            expires_at: std::time::SystemTime::now()
                                + std::time::Duration::from_secs(3600),
                        })
                    })
                }),
            ),
        };

        let sts: Capabilities = serde_yaml::from_str(
            "aws:\n  mode: sts\n  services:\n    - service: s3\n      actions: [GetObject]\n",
        )
        .unwrap();
        let env = state.container_env(&skill(sts), "tok").await.unwrap();
        assert!(env.contains(&"AWS_SESSION_TOKEN=temp-token".to_string()));
        assert!(env.contains(&"AWS_REGION=us-east-1".to_string()));
        assert!(
            env.iter().all(|v| !v.contains("s3cr3t")),
            "the long-term secret never enters the container"
        );
        assert!(policies.lock().unwrap()[0].contains("s3:GetObject"));

        // Proxy-mode and network-only agents get no AWS credentials
        let proxy: Capabilities = serde_yaml::from_str("aws:\n  mode: proxy\n").unwrap();
        for caps in [proxy, allow_hosts(&["example.com"])] {
            let env = state.container_env(&skill(caps), "tok").await.unwrap();
            assert!(env.iter().all(|v| !v.starts_with("AWS_")), "{env:?}");
        }
        assert_eq!(policies.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sts_agents_fail_to_start_without_credentials() {
        let state = EgressState::new(Secrets::in_memory());
        let sts: Capabilities = serde_yaml::from_str(
            "aws:\n  mode: sts\n  services:\n    - service: s3\n      actions: [GetObject]\n",
        )
        .unwrap();
        let err = state.container_env(&skill(sts), "tok").await.unwrap_err();
        assert!(err.contains("needs AWS credentials"), "{err}");
    }

    #[test]
    fn reads_recent_entries_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("egress.log");
        assert!(read_recent(&path, 20).is_empty());

        let mut log = String::from("not json\n");
        for i in 0..25 {
            let entry = EgressLogEntry {
                ts: i as f64,
                agent_id: "a".into(),
                kind: EgressKind::Fetch,
                target: format!("GET https://example.com/{i}"),
                allowed: true,
                status: Some(200),
                duration_ms: Some(1.0),
                error: None,
            };
            log.push_str(&serde_json::to_string(&entry).unwrap());
            log.push('\n');
        }
        std::fs::write(&path, log).unwrap();

        let entries = read_recent(&path, 20);
        assert_eq!(entries.len(), 20);
        assert_eq!(entries[0].ts, 24.0);
        assert_eq!(entries[19].ts, 5.0);
    }
}
