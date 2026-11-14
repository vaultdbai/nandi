//! Agent runtime containers: sandboxed MCP tool servers, one per agent.
//!
//! Each container gets:
//! - the agent folder at `/agent/skill` (read-only) and a persistent workdir
//!   from `~/.nandi/local/work/<id>` at `/agent/workdir` (read-write), as
//!   [`AgentUser`] (the host user's uid on a Linux engine);
//! - only the internal `nandi-agents` network, whose one way out is the
//!   egress relay ([`crate::services::egress::relay`]), and no published
//!   ports: the host runs the tool server over `docker exec -i` stdio
//!   ([`crate::mcp_client`]);
//! - a proxy token for the host egress proxy (LLM, AWS, HTTP) — never a
//!   provider API key or long-term AWS key. `sts`-mode agents also get
//!   short-lived AWS credentials scoped to their skill.md grants.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{CreateContainerOptions, RemoveContainerOptions};
use bollard::Docker;

use crate::sandbox::{self, SandboxSpec, Tmpfs, DEFAULT_CPU, DEFAULT_MEMORY_MB};
use crate::services::agents::{SkillConfig, AGENT_ID_LABEL, INSTANCE_LABEL};
use crate::services::egress::relay::{self, RelayImage};
use crate::services::egress::EgressState;
use crate::utils::nandi_config;

/// Nandi's own sandbox images, as published to GitHub's container registry
/// by `.github/workflows/images.yml` and pinned to this app's
/// [`SANDBOX_VERSION`] (`sandbox-<version>`, never `latest`). The app pulls
/// them when missing (`services::images::ensure`); the advanced local build
/// tags the same name. The one place their names live.
/// The tool server of Nandi's own agents (`runtimes/nandi-runtime`)
pub const DEFAULT_RUNTIME_IMAGE: &str = "ghcr.io/vaultdbai/nandi-runtime:sandbox-6";
/// The harness container (`runtimes/nandi-harness`): Node and the harness
/// CLIs, nothing of Nandi's own. Claude Code is fetched on first use into a
/// volume (`services::images::tools`)
pub const HARNESS_IMAGE: &str = "ghcr.io/vaultdbai/nandi-harness:sandbox-6";
/// Where published MCP connectors run (`runtimes/nandi-connectors`): Node
/// LTS, Python and uv, nothing else
pub const CONNECTORS_IMAGE: &str = "ghcr.io/vaultdbai/nandi-connectors:sandbox-6";

/// Who an agent container runs as: the containers that write the host's
/// folders (its workdir, shared folders it may write).
///
/// On Docker Desktop (Windows, macOS) the image's uid 1000: its file
/// sharing makes bind mounts writable to it. On a Linux engine a bind
/// mount keeps the host's ownership, so the agent runs as the host user's
/// own uid and gid: what the user owns is writable to their agent as to
/// them, and nothing is opened up to anyone else (making the folders
/// writable for uid 1000 instead would hand them to whoever is uid 1000
/// on that machine). Nandi running as root keeps 1000: never root inside.
/// The container has no passwd entry for that uid, so `HOME` and `USER`
/// are set ([`AgentUser::env`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentUser {
    pub uid: u32,
    pub gid: u32,
}

impl AgentUser {
    /// The images' own user (`agent`, `node`): uid 1000. Containers that
    /// write nothing of the host's (the relay, connectors, tool installs)
    /// run as it, set so the uid never depends on the image alone
    pub const IMAGE: Self = Self {
        uid: 1000,
        gid: 1000,
    };

    /// This host's agent user (see [`AgentUser`])
    pub fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            // `/proc/self` belongs to this process's user and group
            if let Ok(own) = std::fs::metadata("/proc/self") {
                if own.uid() != 0 {
                    return Self {
                        uid: own.uid(),
                        gid: own.gid(),
                    };
                }
            }
        }
        Self::IMAGE
    }

    /// Docker's `--user` value
    pub fn user(self) -> String {
        format!("{}:{}", self.uid, self.gid)
    }

    /// A home tmpfs's mount options, owned by this user
    pub fn home_tmpfs(self, options: &str) -> String {
        format!("{options},uid={},gid={}", self.uid, self.gid)
    }

    /// `HOME` (the home tmpfs) and `USER`: a uid with no passwd entry
    /// would get `HOME=/`, on the read-only root
    pub fn env(home: &str, user: &str) -> [String; 2] {
        [format!("HOME={home}"), format!("USER={user}")]
    }
}

/// The native container's home, a tmpfs like `/tmp`: the root filesystem
/// is read-only, so these are its writable scratch places besides the
/// workdir and shared folders
const HOME: &str = "/home/agent";

/// A running agent container; its tool server is reached with
/// `docker exec` ([`crate::mcp_client::McpSession::connect`])
#[derive(Debug, Clone)]
pub struct AgentContainer {
    pub container_id: String,
}

/// Per-agent bearer tokens accepted by the host LLM proxy, so a request's
/// agent is known from its credential rather than from what it claims.
#[derive(Debug, Clone, Default)]
pub struct ProxyTokens(Arc<RwLock<HashMap<String, String>>>);

impl ProxyTokens {
    /// Issues a fresh token for `agent_id`, revoking any previous one.
    pub fn issue(&self, agent_id: &str) -> String {
        let token = new_token();
        let mut tokens = self.0.write().unwrap_or_else(|e| e.into_inner());
        tokens.retain(|_, agent| agent != agent_id);
        tokens.insert(token.clone(), agent_id.to_string());
        token
    }

    pub fn revoke(&self, agent_id: &str) {
        let mut tokens = self.0.write().unwrap_or_else(|e| e.into_inner());
        tokens.retain(|_, agent| agent != agent_id);
    }

    /// The agent a token was issued to, if it's still valid.
    pub fn agent_for(&self, token: &str) -> Option<String> {
        let tokens = self.0.read().unwrap_or_else(|e| e.into_inner());
        tokens.get(token).cloned()
    }
}

/// 244 random bits from two v4 UUIDs.
fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// `nandi-agent-<instance tag>-<agent id>`: per agent and per Nandi
pub fn container_name(agent_id: &str) -> String {
    format!("nandi-agent-{}-{agent_id}", nandi_config::instance_tag())
}

fn connect_docker() -> Result<Docker, String> {
    Docker::connect_with_local_defaults().map_err(|e| {
        log::error!("Failed to connect to Docker: {e}");
        format!("Docker is not available: {e}")
    })
}

/// Whether `image` exists locally and was built for this sandbox
/// ([`image_is_current`]). `Err` if Docker isn't reachable.
pub async fn image_present(image: &str) -> Result<bool, String> {
    let docker = connect_docker()?;
    docker
        .ping()
        .await
        .map_err(|e| format!("Docker is not responding: {e}"))?;
    Ok(match docker.inspect_image(image).await {
        Ok(info) => image_is_current(info.config.and_then(|c| c.labels).as_ref()),
        Err(_) => false,
    })
}

/// The `SANDBOX_LABEL` our Dockerfiles set. Raised when the containers
/// need something new from the image (read-only root, stdio tools), so an
/// image built for an older Nandi is rebuilt rather than half-working.
pub const SANDBOX_VERSION: u32 = 6;
pub const SANDBOX_LABEL: &str = "org.nandi.sandbox";

/// Whether an image's labels say it was built for [`SANDBOX_VERSION`] or later
pub fn image_is_current(labels: Option<&HashMap<String, String>>) -> bool {
    labels
        .and_then(|labels| labels.get(SANDBOX_LABEL))
        .and_then(|version| version.parse::<u32>().ok())
        .is_some_and(|version| version >= SANDBOX_VERSION)
}

/// Starts a fresh runtime container for `skill`, replacing any previous one
/// (its tokens died with the previous app session).
pub async fn start_container(
    skill: &SkillConfig,
    proxy_token: &str,
    egress: &EgressState,
) -> Result<AgentContainer, String> {
    let docker = connect_docker()?;
    let image = skill.image.as_deref().unwrap_or(DEFAULT_RUNTIME_IMAGE);
    if image == DEFAULT_RUNTIME_IMAGE {
        // Pulled when missing or older (progress on `images://pull`), and
        // its sandbox label checked
        crate::services::images::ensure(crate::services::images::SandboxImage::Runtime).await?;
    } else {
        match docker.inspect_image(image).await {
            Err(_) => return Err(format!("Container image {image} not found locally")),
            Ok(info) if !image_is_current(info.config.as_ref().and_then(|c| c.labels.as_ref())) => {
                // Custom images build FROM nandi-runtime and inherit the label
                return Err(format!(
                    "Image {image} was built for an older Nandi and lacks its sandbox. Rebuild                      it FROM {DEFAULT_RUNTIME_IMAGE}"
                ));
            }
            Ok(_) => {}
        }
    }

    let name = container_name(&skill.id);
    remove(&docker, &name).await;

    let workdir = nandi_config::work_dir()?.join(&skill.id);
    tokio::fs::create_dir_all(&workdir)
        .await
        .map_err(|e| format!("Failed to create workdir {}: {e}", workdir.display()))?;

    // Before creating anything: an sts-mode agent without AWS access fails here
    let egress_env = egress.container_env(skill, proxy_token).await?;
    // Its network and the relay out of it, or it doesn't start at all
    relay::ensure_ready(RelayImage::Runtime).await?;
    let config = container_config(skill, image, &workdir.display().to_string(), egress_env);
    let options = CreateContainerOptions {
        name: Some(name.clone()),
        ..Default::default()
    };
    let created = docker
        .create_container(Some(options), config)
        .await
        .map_err(|e| format!("Failed to create the agent container: {e}"))?;

    if let Err(e) = docker.start_container(&created.id, None).await {
        // Don't leak a half-started container
        remove(&docker, &created.id).await;
        return Err(format!("Failed to start the agent container: {e}"));
    }
    log::info!("Started runtime container {name} for agent '{}'", skill.id);
    Ok(AgentContainer {
        container_id: created.id,
    })
}

/// Stops and removes the container. Errors are logged; the container may
/// already be gone.
pub async fn stop_container(container_id: &str) {
    if let Ok(docker) = connect_docker() {
        remove(&docker, container_id).await;
    }
}

/// Whether Docker reports the container as running. A missing container is
/// `Ok(false)`; `Err` means Docker couldn't be asked.
pub async fn container_running(container_id: &str) -> Result<bool, String> {
    let docker = connect_docker()?;
    match docker.inspect_container(container_id, None).await {
        Ok(info) => Ok(info.state.and_then(|s| s.running).unwrap_or(false)),
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => Ok(false),
        Err(e) => Err(format!("Failed to inspect container {container_id}: {e}")),
    }
}

async fn remove(docker: &Docker, name_or_id: &str) {
    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    match docker.remove_container(name_or_id, Some(options)).await {
        Ok(()) => log::debug!("Removed container {name_or_id}"),
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {}
        Err(e) => log::warn!("Failed to remove container {name_or_id}: {e}"),
    }
}

fn container_config(
    skill: &SkillConfig,
    image: &str,
    workdir: &str,
    egress_env: Vec<String>,
) -> ContainerCreateBody {
    let mut spec = SandboxSpec::new(image)
        .label(AGENT_ID_LABEL, &skill.id)
        .label(INSTANCE_LABEL, nandi_config::instance_tag())
        .user(AgentUser::current())
        // Internal: no route out but the relay, no published ports
        .network(relay::AGENT_NETWORK)
        // Reaps what the shell tool leaves behind
        .init()
        .bind(
            &skill.dir.display().to_string(),
            "/agent/skill",
            sandbox::Access::ReadOnly,
        )
        .bind(workdir, "/agent/workdir", sandbox::Access::ReadWrite)
        .tmpfs(Tmpfs::scratch("/tmp", 256))
        .tmpfs(Tmpfs::home(HOME, 64))
        .memory_mb(skill.memory_mb.unwrap_or(DEFAULT_MEMORY_MB))
        .cpus(skill.cpu.unwrap_or(DEFAULT_CPU))
        .env(
            std::iter::once(format!("NANDI_AGENT_ID={}", skill.id))
                .chain(AgentUser::env(HOME, "agent"))
                .chain(
                    skill
                        .shell_timeout_s
                        .map(|seconds| format!("NANDI_SHELL_TIMEOUT_SECONDS={seconds}")),
                )
                .chain(egress_env),
        );
    // Its shared folders, read-only unless it may write (`agent_folders`)
    for folder in folder_mounts(&skill.folders) {
        spec = spec.bind(
            &folder.source,
            &folder.target,
            sandbox::Access::read_only_if(folder.read_only),
        );
    }
    spec.to_bollard()
}

/// Where an agent's shared folders appear in its container
pub const FOLDERS_MOUNT: &str = "/agent/folders";

/// A shared folder bound into an agent's container
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMount {
    /// On the host
    pub source: String,
    /// `/agent/folders/<id>`
    pub target: String,
    pub read_only: bool,
}

/// A mount for each shared folder that exists on the host (`~` expanded)
pub fn folder_mounts(
    folders: &[crate::services::connectors::folders::SharedFolder],
) -> Vec<FolderMount> {
    use crate::services::connectors::folders::Access;
    folders
        .iter()
        .filter_map(|folder| {
            let path = crate::services::connectors::expand_home(&folder.path);
            let path = std::fs::canonicalize(path).ok().filter(|p| p.is_dir())?;
            // Docker refuses Windows' verbatim `\\?\C:\…` form
            let path = path.display().to_string();
            let path = path
                .strip_prefix(r"\\?\")
                .map_or(path.clone(), str::to_string);
            Some(FolderMount {
                source: path,
                target: format!("{FOLDERS_MOUNT}/{}", folder.id),
                read_only: folder.access != Access::Write,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skill() -> SkillConfig {
        SkillConfig {
            id: "writer".into(),
            dir: PathBuf::from("/home/u/.nandi/agents/writer"),
            model: "claude:claude-sonnet-5".into(),
            image: None,
            memory_mb: None,
            cpu: None,
            shell_timeout_s: None,
            instructions: String::new(),
            allowed_tools: None,
            requires_approval: vec![],
            capabilities: Default::default(),
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
    fn proxy_tokens_map_to_agents_and_rotate() {
        let tokens = ProxyTokens::default();
        let first = tokens.issue("a");
        let other = tokens.issue("b");

        assert_eq!(tokens.agent_for(&first).as_deref(), Some("a"));
        let second = tokens.issue("a");
        assert!(tokens.agent_for(&first).is_none(), "old token revoked");
        assert_eq!(tokens.agent_for(&second).as_deref(), Some("a"));

        tokens.revoke("a");
        assert!(tokens.agent_for(&second).is_none());
        assert_eq!(tokens.agent_for(&other).as_deref(), Some("b"));
        assert!(tokens.agent_for("guess").is_none());
    }

    #[test]
    fn shared_folders_mount_read_only_unless_writable() {
        use crate::services::connectors::folders::{Access, SharedFolder};
        let (docs, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let folder = |id: &str, dir: &tempfile::TempDir, access| SharedFolder {
            id: id.into(),
            path: dir.path().display().to_string(),
            access,
        };
        let skill = SkillConfig {
            folders: vec![
                folder("docs", &docs, Access::Read),
                folder("out", &out, Access::Write),
                SharedFolder {
                    id: "gone".into(),
                    path: docs.path().join("missing").display().to_string(),
                    access: Access::Write,
                },
            ],
            ..skill()
        };
        let config = container_config(&skill, DEFAULT_RUNTIME_IMAGE, "/w", vec![]);
        let mounts = config.host_config.unwrap().mounts.unwrap();
        let folders: Vec<(Option<&str>, Option<bool>)> = mounts[2..]
            .iter()
            .map(|m| (m.target.as_deref(), m.read_only))
            .collect();
        assert_eq!(
            folders,
            [
                (Some("/agent/folders/docs"), Some(true)),
                (Some("/agent/folders/out"), Some(false)),
            ],
            "missing folders aren't mounted"
        );
        assert!(!mounts[2].source.as_deref().unwrap().starts_with(r"\\?\"));
    }

    #[test]
    fn container_is_sandboxed_and_gets_no_api_keys() {
        let config = container_config(
            &skill(),
            DEFAULT_RUNTIME_IMAGE,
            "/w",
            crate::services::egress::proxy_env("proxy-t"),
        );
        let host = config.host_config.as_ref().unwrap();

        let mounts = host.mounts.as_ref().unwrap();
        assert_eq!(mounts[0].target.as_deref(), Some("/agent/skill"));
        assert_eq!(mounts[0].read_only, Some(true));
        assert_eq!(mounts[1].target.as_deref(), Some("/agent/workdir"));
        assert_eq!(mounts[1].read_only, Some(false));

        // Only the internal network, nothing published, no host mapping
        assert_eq!(host.network_mode.as_deref(), Some("nandi-agents"));
        assert!(host.port_bindings.is_none() && config.exposed_ports.is_none());
        assert!(host.extra_hosts.is_none());
        assert_eq!(host.cap_drop.as_deref(), Some(&["ALL".to_string()][..]));
        assert_eq!(host.readonly_rootfs, Some(true));
        let tmpfs = host.tmpfs.as_ref().unwrap();
        assert!(tmpfs.contains_key("/tmp") && tmpfs.contains_key("/home/agent"));
        let user = AgentUser::current();
        assert_eq!(config.user, Some(user.user()));
        assert!(tmpfs["/home/agent"].ends_with(&format!(",uid={},gid={}", user.uid, user.gid)));

        let env = config.env.unwrap();
        assert!(env.contains(&"HOME=/home/agent".to_string()), "{env:?}");
        assert!(env.iter().all(|var| !var.contains("MCP_TOKEN")));
        assert!(env.contains(&"NANDI_LLM_PROXY_TOKEN=proxy-t".to_string()));
        assert!(
            env.iter().all(|var| !var.contains("API_KEY")),
            "no provider keys in the container: {env:?}"
        );
        let labels = config.labels.unwrap();
        assert_eq!(labels[AGENT_ID_LABEL], "writer");
        assert_eq!(labels[INSTANCE_LABEL], nandi_config::instance_tag());
    }

    #[test]
    fn agents_run_as_the_host_user_on_linux_and_never_as_root() {
        let user = AgentUser::current();
        assert_ne!(user.uid, 0, "never root inside");
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let own = std::fs::metadata("/proc/self").unwrap();
            if own.uid() != 0 {
                assert_eq!((user.uid, user.gid), (own.uid(), own.gid()));
            }
        }
        #[cfg(not(target_os = "linux"))]
        assert_eq!(user, AgentUser::IMAGE, "Docker Desktop: the image's user");
        assert_eq!(
            AgentUser {
                uid: 1001,
                gid: 118
            }
            .home_tmpfs("rw,size=64m"),
            "rw,size=64m,uid=1001,gid=118"
        );
    }

    #[test]
    fn only_images_built_for_this_sandbox_are_current() {
        let labels = |version: &str| HashMap::from([(SANDBOX_LABEL.to_string(), version.into())]);
        assert!(image_is_current(Some(&labels(
            &SANDBOX_VERSION.to_string()
        ))));
        assert!(image_is_current(Some(&labels("99"))));
        assert!(!image_is_current(Some(&labels("1"))));
        assert!(!image_is_current(Some(&labels("new"))));
        assert!(!image_is_current(Some(&HashMap::new())));
        assert!(!image_is_current(None));
    }

    #[test]
    fn container_limits_come_from_skill_md() {
        let limits = |skill: &SkillConfig| {
            let config = container_config(skill, DEFAULT_RUNTIME_IMAGE, "/w", vec![]);
            let host = config.host_config.unwrap();
            (host.memory, host.nano_cpus)
        };

        // Defaults: 1 GiB, one core
        assert_eq!(
            limits(&skill()),
            (Some(1024 * 1024 * 1024), Some(1_000_000_000))
        );
        let sized = SkillConfig {
            memory_mb: Some(2048),
            cpu: Some(1.5),
            ..skill()
        };
        assert_eq!(
            limits(&sized),
            (Some(2048 * 1024 * 1024), Some(1_500_000_000))
        );
    }

    #[test]
    fn shell_timeout_reaches_the_tool_server_only_when_set() {
        let env = |skill: &SkillConfig| {
            container_config(skill, DEFAULT_RUNTIME_IMAGE, "/w", vec![])
                .env
                .unwrap()
        };
        assert!(env(&skill())
            .iter()
            .all(|var| !var.starts_with("NANDI_SHELL_TIMEOUT_SECONDS=")));
        let slow = SkillConfig {
            shell_timeout_s: Some(600),
            ..skill()
        };
        assert!(env(&slow).contains(&"NANDI_SHELL_TIMEOUT_SECONDS=600".to_string()));
    }
}
