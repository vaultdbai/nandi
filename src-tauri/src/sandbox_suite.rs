//! The sandbox suite (`pnpm run test:sandbox`, the CI job `sandbox`): the
//! container's isolation claims as tests against real Docker and the
//! published sandbox images (`docs/testing/runtimes-models-sandbox.md`
//! SB-01, SB-13, SB-14, SB-15, RT-05, RT-06). Each claim fails CI when it
//! breaks.
//!
//! Every test is `#[ignore]`, so `cargo test` needs no Docker; the job runs
//! them with `--ignored --test-threads=1` because they share the one
//! `nandi-egress` relay. They pull the images on first use.
//!
//! An in-process egress proxy (the real routes and CONNECT tunnels) stands
//! in for the app's, set up the way the app's is: on the host's loopback,
//! and, once a relay is made ready on a Linux engine, on Docker's bridge
//! gateway too (`egress::listen_on_bridge_gateway`). Agents may reach one
//! host through it, an echo server on the host's loopback. The containers
//! run as the app runs them (`runtime::AgentUser`), with no folders opened
//! up. A relay that a running Nandi left is pointed back at that Nandi's
//! port when each test ends.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::runtime::{self, ProxyTokens};
use crate::services::agents::SkillConfig;
use crate::services::connectors::folders::{Access, SharedFolder};
use crate::services::egress::relay::{self, RelayImage, AGENT_NETWORK, RELAY_NAME};
use crate::services::egress::tests::{allow_hosts, serve_on, set_proxy_port};
use crate::services::egress::EgressState;
use crate::services::secrets::Secrets;

/// What the planted credentials carry. Searched for as `LEAK[M]ARK`, so the
/// search's own command line never matches it
const MARK: &str = "LEAKMARK";
const MARK_PATTERN: &str = "LEAK[M]ARK";
/// Outside every agent's `network.allow`
const REFUSED_SITE: &str = "example.org:443";

/// The test proxy, and the one host agents may reach through it
struct Proxy {
    port: u16,
    /// `127.0.0.1:<port>`: an echo server on the host's loopback, the only
    /// entry in every agent's `network.allow`
    allowed: String,
    tokens: ProxyTokens,
}

/// One proxy for the whole process, on its own runtime so it outlives each
/// test's
fn proxy() -> &'static Proxy {
    static PROXY: OnceLock<Proxy> = OnceLock::new();
    PROXY.get_or_init(|| {
        let tokens = ProxyTokens::default();
        let (ready, started) = std::sync::mpsc::channel();
        let served = tokens.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let echo = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                    .await
                    .unwrap();
                let allowed = format!("127.0.0.1:{}", echo.local_addr().unwrap().port());
                tokio::spawn(async move {
                    while let Ok((mut socket, _)) = echo.accept().await {
                        tokio::spawn(async move {
                            let (mut read, mut write) = socket.split();
                            let _ = tokio::io::copy(&mut read, &mut write).await;
                        });
                    }
                });
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                    .await
                    .unwrap();
                ready
                    .send((listener.local_addr().unwrap().port(), allowed.clone()))
                    .unwrap();
                serve_on(listener, served, allow_hosts(&[&allowed]))
                    .await
                    .unwrap();
            });
        });
        let (port, allowed) = started.recv().unwrap();
        Proxy {
            port,
            allowed,
            tokens,
        }
    })
}

/// Runs `test` with the relay pointed at the test proxy, then gives the
/// relay back to the Nandi that had it (if one did), even when `test` fails
async fn with_relay<F, Fut>(test: F)
where
    F: FnOnce(&'static Proxy) -> Fut,
    Fut: Future<Output = ()>,
{
    let proxy = proxy();
    let previous = relay_target().await.filter(|port| *port != proxy.port);
    set_proxy_port(proxy.port);
    let outcome =
        futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(test(proxy))).await;
    if let Some(port) = previous {
        set_proxy_port(port);
        let _ = relay::ensure_ready(RelayImage::Runtime).await;
        set_proxy_port(proxy.port);
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// The host port the running relay forwards to
async fn relay_target() -> Option<u16> {
    let info = inspect(RELAY_NAME).await?;
    info["Config"]["Labels"]["org.nandi.egress.target"]
        .as_str()?
        .parse()
        .ok()
}

/// Removes containers when a test ends, however it ends
struct Remove(Vec<String>);

impl Drop for Remove {
    fn drop(&mut self) {
        for name in &self.0 {
            let _ = std::process::Command::new("docker")
                .args(["rm", "-f", name])
                .output();
        }
    }
}

async fn docker(args: &[&str]) -> std::process::Output {
    tokio::process::Command::new("docker")
        .args(args)
        .output()
        .await
        .expect("Docker is installed")
}

/// `docker inspect` of a container, image or network; `None` if missing
async fn inspect(name: &str) -> Option<Value> {
    let output = docker(&["inspect", name]).await;
    let list: Value = serde_json::from_slice(&output.stdout).ok()?;
    list.get(0).cloned()
}

/// Runs `command` in a running container: (exit code, stdout, stderr)
async fn exec(container: &str, command: &[&str]) -> (i32, String, String) {
    let mut args = vec!["exec", container];
    args.extend_from_slice(command);
    let output = docker(&args).await;
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The host credentials Nandi must never let into a container, under
/// `home`: Claude Code's sign-in (its refresh token; the short-lived access
/// token is what a container may get, `claude_code::env`), the AWS CLI's
/// keys and SSO cache, and registry tokens in pip and npm settings
fn plant_credentials(home: &Path) {
    let expires = crate::db::now_ms() + 60 * 60 * 1000;
    let files = [
        (
            ".claude/.credentials.json",
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat-short-lived","refreshToken":"{MARK}-refresh","expiresAt":{expires}}}}}"#
            ),
        ),
        (
            ".aws/credentials",
            format!(
                "[default]\naws_access_key_id = AKIA{MARK}\naws_secret_access_key = {MARK}-aws\n"
            ),
        ),
        (
            ".aws/sso/cache/0123.json",
            format!(r#"{{"accessToken":"{MARK}-sso","expiresAt":"2099-01-01T00:00:00Z"}}"#),
        ),
        (
            ".config/pip/pip.conf",
            format!("[global]\nindex-url = https://user:{MARK}-pip@pypi.example.com/simple\n"),
        ),
        (
            ".npmrc",
            format!("//registry.npmjs.org/:_authToken={MARK}-npm\n"),
        ),
    ];
    for (path, content) in files {
        let path = home.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// Files under `/` (but `/proc`, `/sys`, `/dev`) that hold a planted value
async fn files_with_the_mark(container: &str) -> String {
    let search = format!(
        "grep -rIl --exclude-dir=proc --exclude-dir=sys --exclude-dir=dev '{MARK_PATTERN}' / 2>/dev/null; true"
    );
    let (_, found, _) = exec(container, &["sh", "-c", &search]).await;
    found
}

/// Asks, from inside a container, whether it can reach the internet or the
/// host directly, and what the proxy says to an allowed and a refused site
/// through the container's own `HTTPS_PROXY`
const PROBE: &str = r#"
import base64, json, os, socket, sys, urllib.parse
gateway, proxy_port, allowed, refused = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
def reach(host, port):
    try:
        socket.create_connection((host, port), timeout=5).close()
        return "connected"
    except Exception as e:
        return type(e).__name__
def through_proxy(target):
    proxy = urllib.parse.urlparse(os.environ["HTTPS_PROXY"])
    s = socket.create_connection((proxy.hostname, proxy.port), timeout=15)
    auth = base64.b64encode(f"{proxy.username}:{proxy.password}".encode()).decode()
    s.sendall(f"CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: Basic {auth}\r\n\r\n".encode())
    status = s.recv(256).decode(errors="replace").split("\r\n")[0]
    s.close()
    return status
print(json.dumps({
    "internet": reach("1.1.1.1", 443),
    "host": reach(gateway, proxy_port),
    "host_docker_internal": reach("host.docker.internal", proxy_port),
    "allowed": through_proxy(allowed),
    "refused": through_proxy(refused),
}))
"#;

/// Docker's bridge gateway: on a Linux engine the host, where the proxy
/// listens for the relay; agents must not reach it directly
async fn bridge_gateway() -> String {
    let output = docker(&[
        "network",
        "inspect",
        "bridge",
        "--format",
        "{{(index .IPAM.Config 0).Gateway}}",
    ])
    .await;
    let gateway = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!gateway.is_empty(), "no bridge gateway: {output:?}");
    gateway
}

/// SB-13, SB-14: no way out but the relay, and the proxy decides
async fn only_the_proxy_gets_out(container: &str, proxy: &Proxy) {
    let port = proxy.port.to_string();
    let gateway = bridge_gateway().await;
    let (code, stdout, stderr) = exec(
        container,
        &[
            "python3",
            "-c",
            PROBE,
            &gateway,
            &port,
            &proxy.allowed,
            REFUSED_SITE,
        ],
    )
    .await;
    assert_eq!(code, 0, "probe failed: {stdout} {stderr}");
    let seen: Value = serde_json::from_str(stdout.trim()).unwrap();
    for direct in ["internet", "host", "host_docker_internal"] {
        assert_ne!(
            seen[direct], "connected",
            "{direct} reached directly: {seen}"
        );
    }
    assert!(
        seen["allowed"]
            .as_str()
            .is_some_and(|s| s.starts_with("HTTP/1.1 200")),
        "the relay carries allowed traffic to the proxy: {seen}"
    );
    assert!(
        seen["refused"]
            .as_str()
            .is_some_and(|s| s.starts_with("HTTP/1.1 403")),
        "the proxy refuses a site outside network.allow: {seen}"
    );
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// SB-01: the hardening both container kinds share
fn assert_hardened(info: &Value, pids_limit: i64, home_tmpfs: &str) {
    let host = &info["HostConfig"];
    assert_eq!(host["ReadonlyRootfs"], true, "read-only root");
    assert_eq!(strings(&host["CapDrop"]), ["ALL"], "{}", host["CapDrop"]);
    assert!(
        strings(&host["SecurityOpt"]).contains(&"no-new-privileges".to_string()),
        "{}",
        host["SecurityOpt"]
    );
    assert_eq!(host["PidsLimit"], pids_limit);
    assert_eq!(host["Init"], true, "init reaps leftovers");
    assert_eq!(host["NetworkMode"], AGENT_NETWORK);
    assert!(host["Privileged"] != true);
    let tmpfs = host["Tmpfs"].as_object().cloned().unwrap_or_default();
    assert!(
        tmpfs.contains_key("/tmp") && tmpfs.contains_key(home_tmpfs),
        "{tmpfs:?}"
    );
    assert_eq!(info["Config"]["User"], runtime::AgentUser::current().user());
    // No published ports, no way to the host by name
    let bindings = host["PortBindings"].as_object().map_or(0, |b| b.len());
    assert_eq!(bindings, 0, "{}", host["PortBindings"]);
    assert!(
        strings(&host["ExtraHosts"]).is_empty(),
        "{}",
        host["ExtraHosts"]
    );
    let networks = info["NetworkSettings"]["Networks"]
        .as_object()
        .map(|n| n.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    assert_eq!(networks, [AGENT_NETWORK], "only the internal network");
}

/// Each bind mount's container path → whether it's writable
fn mounts(info: &Value) -> HashMap<String, bool> {
    info["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["Type"] == "bind" || m["Type"] == "volume")
        .map(|m| {
            (
                m["Destination"].as_str().unwrap_or_default().to_string(),
                m["RW"] == true,
            )
        })
        .collect()
}

/// The agent's workdir, as `runtime::start_container` makes it
fn workdir_of(id: &str) {
    let dir = crate::utils::nandi_config::work_dir().unwrap().join(id);
    std::fs::create_dir_all(&dir).unwrap();
}

fn skill(id: &str, dir: &Path, folders: Vec<SharedFolder>) -> SkillConfig {
    SkillConfig {
        id: id.into(),
        dir: dir.to_path_buf(),
        model: "ollama:unused".into(),
        image: None,
        memory_mb: None,
        cpu: None,
        shell_timeout_s: None,
        instructions: String::new(),
        allowed_tools: None,
        requires_approval: vec![],
        capabilities: Default::default(),
        folders,
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

/// SB-01, SB-13, SB-14, RT-05: Nandi's own tool container, started the way
/// a run starts it, with credentials planted around its folders and
/// provider and AWS keys in the keychain
#[tokio::test]
#[ignore = "needs Docker and the sandbox images (pnpm run test:sandbox)"]
async fn the_native_container_is_locked_down() {
    with_relay(|proxy| async move {
        let home = tempfile::tempdir().unwrap();
        plant_credentials(home.path());
        let agent_dir = home.path().join(".nandi/agents/sandbox-e2e-native");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("skill.md"), "# sandbox e2e").unwrap();
        let notes = home.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("plan.md"), "shared, read-only").unwrap();
        let skill = skill(
            "sandbox-e2e-native",
            &agent_dir,
            vec![SharedFolder {
                id: "notes".into(),
                path: notes.display().to_string(),
                access: Access::Read,
            }],
        );
        let secrets = Secrets::in_memory();
        secrets
            .set(
                crate::services::llm_keys::KeyedProvider::Anthropic.keyring_user(),
                &format!("sk-ant-{MARK}-provider"),
            )
            .unwrap();
        crate::services::egress::credentials::set_aws_credentials(
            &secrets,
            &format!("AKIA{MARK}"),
            &format!("{MARK}-keychain-aws"),
        )
        .unwrap();
        let egress = EgressState::new(secrets);
        workdir_of(&skill.id);
        // The relay this container gets is the Python one
        docker(&["rm", "-f", RELAY_NAME]).await;
        let token = proxy.tokens.issue(&skill.id);
        let container = runtime::start_container(&skill, &token, &egress)
            .await
            .unwrap();
        let _remove = Remove(vec![container.container_id.clone()]);
        let id = container.container_id.as_str();

        let info = inspect(id).await.unwrap();
        assert_hardened(&info, 256, "/home/agent");
        let host = &info["HostConfig"];
        assert_eq!(host["Memory"], 1024 * 1024 * 1024_i64);
        assert_eq!(host["NanoCpus"], 1_000_000_000_i64);
        assert!(info["Config"]["ExposedPorts"].is_null());
        // RT-05: the agent folder and shared folder read-only, the workdir
        // writable, and nothing else of the host's
        assert_eq!(
            mounts(&info),
            HashMap::from([
                ("/agent/skill".to_string(), false),
                ("/agent/workdir".to_string(), true),
                ("/agent/folders/notes".to_string(), false),
            ])
        );
        let env = strings(&info["Config"]["Env"]);
        assert!(env.contains(&"NANDI_AGENT_ID=sandbox-e2e-native".to_string()));
        assert!(
            env.iter().any(|v| v.starts_with("HTTPS_PROXY=")
                && v.contains(&format!("@{RELAY_NAME}:{}", relay::RELAY_PORT))),
            "{env:?}"
        );
        for var in &env {
            assert!(!var.contains(MARK), "a key in the env: {var}");
            let name = var.split('=').next().unwrap_or_default();
            assert!(
                !name.contains("API_KEY") && !name.starts_with("AWS_"),
                "{var}"
            );
        }

        // Inside: the agent user (the host user's uid on a Linux engine),
        // nothing writable but the workdir and tmpfs
        let (_, uid, _) = exec(id, &["id", "-u"]).await;
        assert_eq!(uid.trim(), runtime::AgentUser::current().uid.to_string());
        for path in ["/usr/x", "/agent/skill/x", "/agent/folders/notes/x"] {
            let (code, _, _) = exec(id, &["touch", path]).await;
            assert_ne!(code, 0, "{path} is writable");
        }
        let (code, _, stderr) = exec(id, &["touch", "/agent/workdir/x", "/tmp/x"]).await;
        assert_eq!(code, 0, "{stderr}");
        // SB-01: none of the planted credentials is anywhere inside
        let found = files_with_the_mark(id).await;
        assert!(found.trim().is_empty(), "host credentials inside: {found}");

        only_the_proxy_gets_out(id, proxy).await;
    })
    .await;
}

/// SB-01, SB-13, SB-15: a coding agent's container, planned the way a run
/// plans it (`runtime_router::plan`) with the CLI's environment
/// (`claude_code::env`) from a home full of credentials, and a real CLI
/// (`git`) going through the relay to a site outside `network.allow`
#[tokio::test]
#[ignore = "needs Docker and the sandbox images (pnpm run test:sandbox)"]
async fn the_coding_agent_container_is_locked_down() {
    use crate::services::domain::{Agent, AgentId};
    use crate::services::harness::adapter::{spawn_with_env, CliProfile, Invocation, Placement};
    use crate::services::harness::claude_code::ClaudeCodeAdapter;
    use crate::services::harness::detection::DetectEnv;
    use crate::services::orchestrator::runtime_router::{
        self, RouterInputs, RuntimeDecision, Sandbox,
    };

    with_relay(|proxy| async move {
        crate::services::images::ensure(crate::services::images::SandboxImage::Harness)
            .await
            .unwrap();
        let home = tempfile::tempdir().unwrap();
        plant_credentials(home.path());
        let workdir = home.path().join("work");
        std::fs::create_dir_all(&workdir).unwrap();
        let agent = Agent::from_skill_md(
            AgentId::parse("sandbox-e2e-coder").unwrap(),
            "---\nname: Coder\nruntime: claude-code\n---\n\nYou code.\n",
        )
        .unwrap();
        let token = proxy.tokens.issue("sandbox-e2e-coder");
        let RuntimeDecision::Harness(plan) = runtime_router::plan(
            &agent,
            RouterInputs {
                sandbox: Sandbox::Ready(Vec::new()),
                workdir: workdir.clone(),
                proxy_token: token,
                ..RouterInputs::default()
            },
        ) else {
            panic!("a harness plan");
        };
        let Placement::Container { name, .. } = &plan.config.placement else {
            panic!("runs in the container");
        };
        let name = name.clone();
        let adapter = ClaudeCodeAdapter::with_env(DetectEnv {
            home: home.path().to_path_buf(),
            path: None,
            vars: HashMap::from([
                ("GITHUB_TOKEN".to_string(), format!("ghp_{MARK}-env")),
                ("NPM_TOKEN".to_string(), format!("{MARK}-npm-env")),
            ]),
        });
        let env = adapter.profile().env(&plan.config);
        // The relay a coding agent gets is the Node one
        docker(&["rm", "-f", RELAY_NAME]).await;
        relay::ensure_ready(RelayImage::Harness).await.unwrap();
        let _remove = Remove(vec![name.clone()]);
        let mut sleeper = spawn_with_env(
            &Invocation::new("sleep", &["300"]),
            &env,
            &plan.config.placement,
            &workdir,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let info = loop {
            if let Some(info) = inspect(&name)
                .await
                .filter(|i| i["State"]["Running"] == true)
            {
                break info;
            }
            assert!(Instant::now() < deadline, "the harness container never ran");
            tokio::time::sleep(Duration::from_millis(200)).await;
        };

        assert_hardened(&info, 512, "/home/node");
        assert_eq!(info["HostConfig"]["AutoRemove"], true);
        assert_eq!(
            mounts(&info),
            HashMap::from([("/agent/workdir".to_string(), true)]),
            "only the workdir of the host's"
        );
        let container_env = strings(&info["Config"]["Env"]);
        for var in &container_env {
            assert!(!var.contains(MARK), "a host credential in the env: {var}");
        }
        assert!(
            container_env
                .iter()
                .any(|v| v.starts_with("HTTPS_PROXY=") && v.contains(RELAY_NAME)),
            "{container_env:?}"
        );
        let found = files_with_the_mark(&name).await;
        assert!(found.trim().is_empty(), "host credentials inside: {found}");

        only_the_proxy_gets_out(&name, proxy).await;
        // SB-15: a CLI honouring HTTPS_PROXY is refused by the proxy
        let (code, _, stderr) = exec(
            &name,
            &[
                "git",
                "ls-remote",
                &format!(
                    "https://{}/nandi.git",
                    REFUSED_SITE.trim_end_matches(":443")
                ),
            ],
        )
        .await;
        assert_ne!(code, 0);
        assert!(stderr.contains("403"), "refused by the proxy: {stderr}");

        let _ = sleeper.child.start_kill();
    })
    .await;
}

/// SB-13: the agents' network is internal, and a same-named network that
/// isn't is refused rather than used
#[tokio::test]
#[ignore = "needs Docker and the sandbox images (pnpm run test:sandbox)"]
async fn the_agents_network_is_internal_or_refused() {
    relay::ensure_agent_network().await.unwrap();
    let real = inspect(AGENT_NETWORK).await.unwrap();
    assert_eq!(real["Internal"], true, "{real}");

    let name = format!(
        "nandi-agents-e2e-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let created = docker(&["network", "create", &name]).await;
    assert!(created.status.success(), "{created:?}");
    let refused = relay::ensure_network_named(&name).await;
    docker(&["network", "rm", &name]).await;
    let error = refused.unwrap_err();
    assert!(error.contains("isn't internal"), "{error}");

    relay::ensure_network_named(&name).await.unwrap();
    let made = inspect(&name).await.unwrap();
    docker(&["network", "rm", &name]).await;
    assert_eq!(made["Internal"], true, "{made}");
    assert_eq!(made["Labels"]["nandi.agents"], "1", "{made}");
}

/// SB-14: the relay holds nothing, and a relay that's stopped, off the
/// agents' network or forwarding to another port is replaced
#[tokio::test]
#[ignore = "needs Docker and the sandbox images (pnpm run test:sandbox)"]
async fn the_egress_relay_holds_nothing_and_is_replaced_when_wrong() {
    with_relay(|proxy| async move {
        crate::services::images::ensure(crate::services::images::SandboxImage::Runtime)
            .await
            .unwrap();
        relay::ensure_ready(RelayImage::Runtime).await.unwrap();
        let info = inspect(RELAY_NAME).await.unwrap();
        let host = &info["HostConfig"];
        assert_eq!(host["ReadonlyRootfs"], true);
        assert_eq!(strings(&host["CapDrop"]), ["ALL"]);
        assert!(strings(&host["SecurityOpt"]).contains(&"no-new-privileges".to_string()));
        assert_eq!(host["Memory"], 64 * 1024 * 1024_i64);
        assert_eq!(host["PidsLimit"], 64);
        assert_eq!(host["Init"], true);
        assert_eq!(info["Config"]["User"], runtime::AgentUser::IMAGE.user());
        assert_eq!(
            info["Config"]["Labels"]["org.nandi.egress.target"],
            proxy.port.to_string()
        );
        let mut networks: Vec<_> = info["NetworkSettings"]["Networks"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        networks.sort();
        assert_eq!(networks, ["bridge", AGENT_NETWORK]);
        // No env beyond its image's: no tokens, no keys
        let image = inspect(info["Image"].as_str().unwrap()).await.unwrap();
        let image_env = strings(&image["Config"]["Env"]);
        for var in strings(&info["Config"]["Env"]) {
            assert!(image_env.contains(&var), "the relay holds {var}");
        }

        let relay_id = |info: &Value| info["Id"].as_str().unwrap_or_default().to_string();
        let first = relay_id(&info);
        // Stopped → replaced
        docker(&["stop", RELAY_NAME]).await;
        relay::ensure_ready(RelayImage::Runtime).await.unwrap();
        let restarted = inspect(RELAY_NAME).await.unwrap();
        assert_eq!(restarted["State"]["Running"], true);
        assert_ne!(relay_id(&restarted), first);
        // Off the agents' network → replaced
        docker(&["network", "disconnect", AGENT_NETWORK, RELAY_NAME]).await;
        relay::ensure_ready(RelayImage::Runtime).await.unwrap();
        let reattached = inspect(RELAY_NAME).await.unwrap();
        assert!(reattached["NetworkSettings"]["Networks"][AGENT_NETWORK].is_object());
        // Labelled for another proxy port → replaced for this one
        set_proxy_port(proxy.port.wrapping_add(1));
        relay::ensure_ready(RelayImage::Runtime).await.unwrap();
        set_proxy_port(proxy.port);
        relay::ensure_ready(RelayImage::Runtime).await.unwrap();
        assert_eq!(relay_target().await, Some(proxy.port));
        // Untouched when it's right
        let settled = relay_id(&inspect(RELAY_NAME).await.unwrap());
        relay::ensure_ready(RelayImage::Runtime).await.unwrap();
        assert_eq!(relay_id(&inspect(RELAY_NAME).await.unwrap()), settled);
    })
    .await;
}

/// RT-06: the tool server over `docker exec` stdio, confined to the
/// workdir, in a read-only sandbox with no keys and no direct way out
#[tokio::test]
#[ignore = "needs Docker and the sandbox images (pnpm run test:sandbox)"]
async fn the_native_tool_server_is_confined() {
    use crate::mcp_client::{call_timeout, McpSession, ToolOutcome};
    use serde_json::json;

    with_relay(|proxy| async move {
        let agent_dir = tempfile::tempdir().unwrap();
        std::fs::write(agent_dir.path().join("skill.md"), "# e2e").unwrap();
        let skill = skill("sandbox-e2e-tools", agent_dir.path(), Vec::new());
        workdir_of(&skill.id);
        let egress = EgressState::new(Secrets::in_memory());
        let token = proxy.tokens.issue(&skill.id);
        let container = runtime::start_container(&skill, &token, &egress)
            .await
            .unwrap();
        let _remove = Remove(vec![container.container_id.clone()]);
        let mcp = McpSession::connect(&container.container_id, call_timeout(None))
            .await
            .unwrap();
        let mut names: Vec<_> = mcp
            .list_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        names.sort();
        assert_eq!(names, ["list_dir", "read_file", "shell", "write_file"]);

        let wrote = mcp
            .call_tool(
                "write_file",
                &json!({"path": "notes/a.txt", "content": "hi"}),
            )
            .await
            .unwrap();
        assert!(!wrote.is_error, "{wrote:?}");
        let read = mcp
            .call_tool("read_file", &json!({"path": "notes/a.txt"}))
            .await
            .unwrap();
        assert!(read.content.contains("hi"), "{read:?}");
        let listed = mcp.call_tool("list_dir", &json!({})).await.unwrap();
        assert!(listed.content.contains("notes/"), "{listed:?}");

        let exit_code = |outcome: &ToolOutcome| {
            serde_json::from_str::<Value>(&outcome.content).unwrap()["exit_code"].as_i64()
        };
        let shell = mcp
            .call_tool("shell", &json!({"cmd": "cat notes/a.txt; ls /agent/skill"}))
            .await
            .unwrap();
        assert_eq!(exit_code(&shell), Some(0), "{shell:?}");
        assert!(shell.content.contains("skill.md"), "{shell:?}");
        let escaped = mcp
            .call_tool("read_file", &json!({"path": "../skill/skill.md"}))
            .await
            .unwrap();
        assert!(escaped.is_error, "path escape must fail: {escaped:?}");
        let readonly = mcp
            .call_tool("shell", &json!({"cmd": "touch /agent/skill/x"}))
            .await
            .unwrap();
        assert_ne!(exit_code(&readonly), Some(0), "{readonly:?}");
        let direct = mcp
            .call_tool(
                "shell",
                &json!({"cmd": "env -u HTTPS_PROXY -u https_proxy -u HTTP_PROXY -u http_proxy \
                    python -c \"import urllib.request; urllib.request.urlopen('http://example.com', timeout=5)\""}),
            )
            .await
            .unwrap();
        assert_ne!(exit_code(&direct), Some(0), "no direct internet: {direct:?}");
        let root = mcp
            .call_tool("shell", &json!({"cmd": "touch /usr/x"}))
            .await
            .unwrap();
        assert_ne!(exit_code(&root), Some(0), "read-only root: {root:?}");
        let env = mcp
            .call_tool("shell", &json!({"cmd": "env"}))
            .await
            .unwrap();
        assert!(!env.content.contains("API_KEY"), "no keys: {env:?}");
        assert!(env.content.contains("nandi-egress:11435"), "{env:?}");
        mcp.close().await;
    })
    .await;
}
