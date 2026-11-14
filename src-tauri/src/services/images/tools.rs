//! Tool bundles fetched on first use, on the user's computer, into a Docker
//! volume: what the public harness image may not redistribute. Today that's
//! Claude Code (Anthropic's ACP adapter and the Claude Agent SDK, whose
//! bundled CLI is Claude Code).
//!
//! | Step      | What happens                                                                                       |
//! | --------- | -------------------------------------------------------------------------------------------------- |
//! | First use | A one-off harness container runs `install-tools.sh`: `npm ci` from the lockfile in the image, into the volume `nandi-tools-<bundle>-<lock hash>` |
//! | Check     | The volume's `.nandi-ready` holds the lockfile's sha256 (checked once per app session)              |
//! | Runs      | Mount the volume read-only at `/opt/nandi-tools/<bundle>`; its bin is on the image's `PATH`        |
//! | Update    | A new lockfile is a new volume; older volumes of the bundle are removed after an install            |
//!
//! `npm ci` checks every package against the lockfile's integrity hash. The
//! user accepts Anthropic's terms when they sign in to Claude Code, as they
//! do today. Progress streams as `images://tools`.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use bollard::models::VolumeCreateRequest;
use bollard::query_parameters::{ListVolumesOptionsBuilder, RemoveVolumeOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::SandboxImage;
use crate::sandbox::{Access, SandboxSpec, Tmpfs, BRIDGE_NETWORK};
use crate::services::events::{self, NandiEvent};

/// What the harness image ships and the first use installs (keep the ACP
/// version in step with `harness::claude_code::ACP_VERSION`)
pub const CLAUDE_CODE_PACKAGE: &[u8] =
    include_bytes!("../../../../runtimes/nandi-harness/claude-code/package.json");
pub const CLAUDE_CODE_LOCK: &[u8] =
    include_bytes!("../../../../runtimes/nandi-harness/claude-code/package-lock.json");

/// The label naming a tools volume's bundle (for removing old ones)
pub const TOOLS_LABEL: &str = "org.nandi.tools";
/// npm's download for Claude Code (its cache after `npm ci`, both libc
/// builds), measured for the pinned lockfile
const CLAUDE_CODE_DOWNLOAD_BYTES: u64 = 237 * 1024 * 1024;
/// The install gives up after this
const INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ToolBundle {
    ClaudeCode,
}

/// A tools volume as a run mounts it (always read-only)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeMount {
    pub volume: String,
    pub target: String,
}

impl ToolBundle {
    /// The bundle a harness runtime needs, if any (Codex and Gemini CLI are
    /// in the image)
    pub fn for_harness(harness_id: &str) -> Option<Self> {
        (harness_id == "claude-code").then_some(Self::ClaudeCode)
    }

    /// Its folder name in the image and the volume
    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
        }
    }

    /// The image whose `install-tools.sh` installs it and whose runs use it
    pub fn image(self) -> SandboxImage {
        SandboxImage::Harness
    }

    fn lock(self) -> &'static [u8] {
        match self {
            Self::ClaudeCode => CLAUDE_CODE_LOCK,
        }
    }

    /// The lockfile's sha256: what a ready volume's marker says
    pub fn lock_hash(self) -> String {
        let digest = Sha256::digest(self.lock());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// `nandi-tools-claude-code-<12 hex>`: a new lockfile is a new volume
    pub fn volume(self) -> String {
        let hash = self.lock_hash();
        format!("nandi-tools-{}-{}", self.id(), &hash[..12])
    }

    /// Where runs mount the volume
    pub fn mount_path(self) -> &'static str {
        match self {
            Self::ClaudeCode => "/opt/nandi-tools/claude-code",
        }
    }

    /// On the image's `PATH` (the Dockerfile's `ENV PATH`)
    #[cfg(test)]
    pub fn bin_dir(self) -> &'static str {
        match self {
            Self::ClaudeCode => "/opt/nandi-tools/claude-code/node_modules/.bin",
        }
    }

    /// About how much npm downloads, for "one time, about N MB"
    pub fn download_bytes(self) -> u64 {
        match self {
            Self::ClaudeCode => CLAUDE_CODE_DOWNLOAD_BYTES,
        }
    }

    pub fn mount(self) -> VolumeMount {
        VolumeMount {
            volume: self.volume(),
            target: self.mount_path().to_string(),
        }
    }
}

/// A bundle as Settings shows it
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub volume: String,
    /// Installed and checked: runs use it with no wait
    pub ready: bool,
    pub installing: bool,
    pub docker_running: bool,
    /// About how much the first use downloads
    #[specta(type = f64)]
    pub download_bytes: u64,
    pub log: Vec<String>,
    /// Why the last install failed
    pub error: Option<String>,
}

/// `images://tools`: a line of an install's output, or its end
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsEvent {
    pub bundle: ToolBundle,
    /// About how much it downloads (for "one time, about N MB")
    pub download_bytes: u64,
    pub line: Option<String>,
    /// Set when the install ended: whether it worked
    pub finished: Option<bool>,
    pub error: Option<String>,
}

impl NandiEvent for ToolsEvent {
    const NAME: &'static str = "images://tools";
}

#[derive(Debug, Default)]
struct BundleState {
    installing: bool,
    /// Checked ready this app session
    verified: bool,
    log: VecDeque<String>,
    error: Option<String>,
}

static BUNDLES: LazyLock<Mutex<HashMap<ToolBundle, BundleState>>> = LazyLock::new(Mutex::default);

fn bundles() -> std::sync::MutexGuard<'static, HashMap<ToolBundle, BundleState>> {
    BUNDLES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forgets this session's check, as a fresh app start would (live tests)
#[cfg(test)]
pub fn forget(bundle: ToolBundle) {
    bundles().remove(&bundle);
}

fn verified(bundle: ToolBundle) -> bool {
    bundles().get(&bundle).is_some_and(|s| s.verified)
}

pub async fn status(bundle: ToolBundle) -> ToolsStatus {
    let mut status = {
        let bundles = bundles();
        let state = bundles.get(&bundle);
        ToolsStatus {
            volume: bundle.volume(),
            ready: state.is_some_and(|s| s.verified),
            installing: state.is_some_and(|s| s.installing),
            download_bytes: bundle.download_bytes(),
            log: state
                .map(|s| s.log.iter().cloned().collect())
                .unwrap_or_default(),
            error: state.and_then(|s| s.error.clone()),
            ..ToolsStatus::default()
        }
    };
    let Ok(docker) = super::docker().await else {
        return status;
    };
    status.docker_running = true;
    if !status.ready && !status.installing && check_ready(&docker, bundle).await {
        bundles().entry(bundle).or_default().verified = true;
        status.ready = true;
    }
    status
}

/// Makes `bundle` ready for a run: its image first, then the volume,
/// installing it the first time (progress on `images://tools`). The mount
/// to add to the run's container.
pub async fn ensure(bundle: ToolBundle) -> Result<VolumeMount, String> {
    if verified(bundle) {
        return Ok(bundle.mount());
    }
    super::ensure(bundle.image()).await?;
    let docker = super::docker().await?;
    if check_ready(&docker, bundle).await {
        bundles().entry(bundle).or_default().verified = true;
        return Ok(bundle.mount());
    }
    // Already installing: wait for that one
    let _ = start_install(bundle);
    loop {
        {
            let bundles = bundles();
            if let Some(state) = bundles.get(&bundle).filter(|s| !s.installing) {
                return match &state.error {
                    Some(e) => Err(e.clone()),
                    None => Ok(bundle.mount()),
                };
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// "Get it now" / Retry: starts the install in the background (refused
/// while one runs)
pub fn start_install(bundle: ToolBundle) -> Result<(), String> {
    {
        let mut bundles = bundles();
        let state = bundles.entry(bundle).or_default();
        if state.installing {
            return Err(format!(
                "{} is already being installed",
                bundle.display_name()
            ));
        }
        *state = BundleState {
            installing: true,
            ..BundleState::default()
        };
    }
    let bus = super::bus();
    tauri::async_runtime::spawn(async move {
        let emit = |event: ToolsEvent| {
            if let Some(bus) = &bus {
                events::emit::<ToolsEvent>(bus, &event);
            }
        };
        let output = |line: &str| {
            super::push_line(&mut bundles().entry(bundle).or_default().log, line);
            emit(ToolsEvent {
                bundle,
                download_bytes: bundle.download_bytes(),
                line: Some(line.to_string()),
                finished: None,
                error: None,
            });
        };
        let result = install(bundle, &output).await;
        let error = result.err();
        {
            let mut bundles = bundles();
            let state = bundles.entry(bundle).or_default();
            state.installing = false;
            state.verified = error.is_none();
            state.error = error.clone();
        }
        match &error {
            Some(e) => log::warn!("Installing {} failed: {e}", bundle.display_name()),
            None => log::info!(
                "Installed {} into {}",
                bundle.display_name(),
                bundle.volume()
            ),
        }
        emit(ToolsEvent {
            bundle,
            download_bytes: bundle.download_bytes(),
            line: None,
            finished: Some(error.is_none()),
            error,
        });
    });
    Ok(())
}

/// Whether the volume exists and its marker matches this app's lockfile
async fn check_ready(docker: &bollard::Docker, bundle: ToolBundle) -> bool {
    if docker.inspect_volume(&bundle.volume()).await.is_err() {
        return false;
    }
    if super::is_current(docker, bundle.image()).await {
        let output = docker_command()
            .args(marker_args(bundle))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
        if let Ok(output) = output {
            return output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == bundle.lock_hash();
        }
    }
    false
}

/// `docker run` that prints the volume's marker (no network, read-only)
fn marker_args(bundle: ToolBundle) -> Vec<String> {
    SandboxSpec::new(bundle.image().tag())
        .remove_on_exit()
        .volume(&bundle.volume(), "/tools", Access::ReadOnly)
        .entrypoint("cat")
        .command(["/tools/.nandi-ready"])
        .to_cli_args()
}

/// The one-off install container: Nandi's own script (not agent code), as
/// the image's `node` user with a read-only root, on Docker's default
/// network to reach the npm registry. A company proxy passes by name
/// (`-e NAME`), as for image builds.
fn install_args(bundle: ToolBundle, proxy_names: &[String]) -> Vec<String> {
    SandboxSpec::new(bundle.image().tag())
        .remove_on_exit()
        .name(&format!("nandi-tools-install-{}", bundle.id()))
        .label("nandi.tools-install", "1")
        .pids(512)
        // npm's cache lives in the /tmp tmpfs, which counts as memory
        .memory_mb(INSTALL_MEMORY_MB)
        .cpus(INSTALL_CPUS)
        .tmpfs(Tmpfs::scratch("/tmp", 2048))
        .tmpfs(Tmpfs::home("/home/node", 64))
        .network(BRIDGE_NETWORK)
        .volume(&bundle.volume(), bundle.mount_path(), Access::ReadWrite)
        .env_names(proxy_names)
        .command(["/opt/nandi/install-tools.sh", bundle.id()])
        .to_cli_args()
}

/// The install's limits: room for its 2 GB `/tmp` and npm itself
const INSTALL_MEMORY_MB: u32 = 4096;
const INSTALL_CPUS: f32 = 2.0;

fn docker_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("docker");
    #[cfg(windows)]
    {
        // No console window flashing up
        command.creation_flags(0x0800_0000);
    }
    command.kill_on_drop(true);
    command
}

/// What a failed install says: what went wrong, and to try again
fn install_failed(bundle: ToolBundle, detail: &str) -> String {
    format!(
        "Couldn't get {} ({detail}). Check that this computer is online and can reach \
         registry.npmjs.org, then try again.",
        bundle.display_name()
    )
}

async fn install(bundle: ToolBundle, output: &impl Fn(&str)) -> Result<(), String> {
    super::ensure(bundle.image()).await?;
    let docker = super::docker().await?;
    output(&format!(
        "Getting {} (one time, about {})…",
        bundle.display_name(),
        super::registry::megabytes(bundle.download_bytes())
    ));
    let labels = HashMap::from([
        (TOOLS_LABEL.to_string(), bundle.id().to_string()),
        (format!("{TOOLS_LABEL}.lock"), bundle.lock_hash()),
    ]);
    docker
        .create_volume(VolumeCreateRequest {
            name: Some(bundle.volume()),
            labels: Some(labels),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("Couldn't create the volume {}: {e}", bundle.volume()))?;
    // One left from an install the app didn't see end
    let _ = docker_command()
        .args(["rm", "-f", &format!("nandi-tools-install-{}", bundle.id())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    let proxies: Vec<String> = super::proxy_env().into_keys().collect();
    let mut child = docker_command()
        .args(install_args(bundle, &proxies))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| install_failed(bundle, &format!("docker: {e}")))?;
    let last = std::sync::Arc::new(Mutex::new(String::new()));
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let read = |stream: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| {
        let last = last.clone();
        async move {
            let Some(stream) = stream else { return };
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                output(&line);
                if !line.trim().is_empty() {
                    *last.lock().unwrap_or_else(|e| e.into_inner()) = line;
                }
            }
        }
    };
    let status = tokio::time::timeout(INSTALL_TIMEOUT, async {
        let (_, _, status) = tokio::join!(
            read(stdout.map(|s| Box::new(s) as _)),
            read(stderr.map(|s| Box::new(s) as _)),
            child.wait()
        );
        status
    })
    .await
    .map_err(|_| install_failed(bundle, "it took too long"))?
    .map_err(|e| install_failed(bundle, &e.to_string()))?;
    if !status.success() {
        let detail = last.lock().unwrap_or_else(|e| e.into_inner()).clone();
        return Err(install_failed(bundle, detail.trim()));
    }
    if !check_ready(&docker, bundle).await {
        return Err(install_failed(bundle, "the install didn't finish"));
    }
    remove_old_volumes(&docker, bundle).await;
    output(&format!("{} is ready.", bundle.display_name()));
    Ok(())
}

/// Removes the bundle's volumes from earlier lockfiles (one in use stays)
async fn remove_old_volumes(docker: &bollard::Docker, bundle: ToolBundle) {
    let filters = HashMap::from([(
        "label".to_string(),
        vec![format!("{TOOLS_LABEL}={}", bundle.id())],
    )]);
    let options = ListVolumesOptionsBuilder::default()
        .filters(&filters)
        .build();
    let Ok(list) = docker.list_volumes(Some(options)).await else {
        return;
    };
    let current = bundle.volume();
    for volume in list.volumes.unwrap_or_default() {
        if volume.name != current {
            match docker
                .remove_volume(&volume.name, None::<RemoveVolumeOptions>)
                .await
            {
                Ok(()) => log::info!("Removed the old tools volume {}", volume.name),
                Err(e) => log::debug!("Kept the old tools volume {}: {e}", volume.name),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_claude_code_is_fetched_on_first_use() {
        assert_eq!(
            ToolBundle::for_harness("claude-code"),
            Some(ToolBundle::ClaudeCode)
        );
        assert_eq!(ToolBundle::for_harness("codex"), None);
        assert_eq!(ToolBundle::for_harness("gemini-cli"), None);
    }

    /// A new lockfile is a new volume, named by its hash
    #[test]
    fn the_volume_is_named_by_the_lockfile() {
        let bundle = ToolBundle::ClaudeCode;
        let hash = bundle.lock_hash();
        assert_eq!(hash.len(), 64);
        assert_eq!(
            bundle.volume(),
            format!("nandi-tools-claude-code-{}", &hash[..12])
        );
        assert_eq!(
            bundle.mount(),
            VolumeMount {
                volume: bundle.volume(),
                target: "/opt/nandi-tools/claude-code".into(),
            }
        );
        assert!(bundle.bin_dir().starts_with(bundle.mount_path()));
    }

    /// The lockfile pins the ACP adapter the app is tested with, with an
    /// integrity hash for every package
    #[test]
    fn the_lockfile_pins_the_tested_adapter_with_integrity() {
        let lock: serde_json::Value = serde_json::from_slice(CLAUDE_CODE_LOCK).unwrap();
        let package: serde_json::Value = serde_json::from_slice(CLAUDE_CODE_PACKAGE).unwrap();
        let acp = crate::services::harness::claude_code::ACP_PACKAGE;
        let version = crate::services::harness::claude_code::ACP_VERSION;
        assert_eq!(package["dependencies"][acp], version);
        assert_eq!(lock["packages"][""]["dependencies"][acp], version);
        assert_eq!(
            lock["packages"][format!("node_modules/{acp}")]["version"],
            version
        );
        for (path, entry) in lock["packages"].as_object().unwrap() {
            if path.is_empty() {
                continue;
            }
            let integrity = entry["integrity"].as_str().unwrap_or_default();
            assert!(integrity.starts_with("sha512-"), "{path} has no integrity");
        }
    }

    #[test]
    fn the_install_runs_hardened_with_the_volume_writable() {
        let args = install_args(ToolBundle::ClaudeCode, &["HTTPS_PROXY".to_string()]);
        let joined = args.join(" ");
        assert!(joined.contains("--user=1000:1000 --read-only"), "{joined}");
        assert!(joined.contains("--cap-drop=ALL"));
        assert!(joined.contains("--security-opt=no-new-privileges --pids-limit=512"));
        assert!(joined.contains(&format!(
            "--mount type=volume,source={},target=/opt/nandi-tools/claude-code -e HTTPS_PROXY",
            ToolBundle::ClaudeCode.volume()
        )));
        assert!(joined.ends_with(&format!(
            "{} /opt/nandi/install-tools.sh claude-code",
            crate::runtime::HARNESS_IMAGE
        )));
        // The proxy's value never shows in the command line
        assert!(!joined.contains("HTTPS_PROXY="));
    }

    #[test]
    fn the_check_reads_the_marker_read_only_without_network() {
        let joined = marker_args(ToolBundle::ClaudeCode).join(" ");
        assert!(joined.contains("--network=none"));
        assert!(joined.contains("--read-only"));
        assert!(joined.contains("--cap-drop=ALL"));
        assert!(joined.contains(&format!(
            "--mount type=volume,source={},target=/tools,readonly",
            ToolBundle::ClaudeCode.volume()
        )));
        assert!(joined.ends_with("/tools/.nandi-ready"));
    }

    #[test]
    fn a_failed_install_says_to_try_again() {
        let message = install_failed(ToolBundle::ClaudeCode, "ENOTFOUND");
        assert!(message.starts_with("Couldn't get Claude Code (ENOTFOUND)"));
        assert!(message.contains("try again"));
    }
}
