//! Gets the sandbox images, so nobody needs a checkout and a terminal:
//! `nandi-harness` (Claude Code, Codex and Gemini CLI, for agents with
//! `runtime: claude-code | codex | gemini-cli`) and `nandi-runtime` (the
//! tool server for Nandi's own agents' tools). Ask and agents that only
//! chat need neither.
//!
//! | Way             | What it does                                                                        |
//! | --------------- | ----------------------------------------------------------------------------------- |
//! | Download (pull) | The default: the public image pinned to this app's sandbox version from ghcr.io     |
//! | Build here      | The guided local build: the contexts compiled in from `runtimes/`, built with bollard |
//!
//! Both end with the same name ([`SandboxImage::tag`], from
//! `runtime::DEFAULT_RUNTIME_IMAGE` and `runtime::HARNESS_IMAGE`), so the
//! rest of the app doesn't care which. A download starts by itself when a
//! run needs a missing image; offline, or without the registry, it fails
//! with a plain message and Build here remains.
//!
//! A job runs in the background: its output streams as `images://build`
//! events, and the last lines are kept so a page opened mid-job (or after)
//! shows them. A local build is labelled with the app version that built
//! it; the published images carry the version of the release that built
//! them.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex, OnceLock};

use bollard::query_parameters::{BuildImageOptionsBuilder, CreateImageOptionsBuilder};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;

/// Lines of output kept per image
const LOG_LINES: usize = 300;
/// The label naming the app version that built an image
pub const VERSION_LABEL: &str = "org.nandi.built-by";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SandboxImage {
    /// Claude Code, Codex and Gemini CLI
    Harness,
    /// Nandi's own agents' tools
    Runtime,
}

impl SandboxImage {
    /// The full name the app runs: `ghcr.io/vaultdbai/<name>:sandbox-<n>`
    pub fn tag(self) -> &'static str {
        match self {
            Self::Harness => crate::runtime::HARNESS_IMAGE,
            Self::Runtime => crate::runtime::DEFAULT_RUNTIME_IMAGE,
        }
    }

    /// [`Self::tag`] as the repository and the tag to pull
    pub fn pull_ref(self) -> (&'static str, &'static str) {
        let name = self.tag();
        name.rsplit_once(':').unwrap_or((name, "latest"))
    }

    /// The build context: each file's path in the context and its bytes
    fn context(self) -> &'static [(&'static str, &'static [u8])] {
        match self {
            Self::Harness => &[
                (
                    "Dockerfile",
                    include_bytes!("../../../runtimes/nandi-harness/Dockerfile"),
                ),
                (
                    "codex-config.toml",
                    include_bytes!("../../../runtimes/nandi-harness/codex-config.toml"),
                ),
                (
                    "entrypoint.sh",
                    include_bytes!("../../../runtimes/nandi-harness/entrypoint.sh"),
                ),
                (
                    "relay.mjs",
                    include_bytes!("../../../runtimes/nandi-harness/relay.mjs"),
                ),
            ],
            Self::Runtime => &[
                (
                    "Dockerfile",
                    include_bytes!("../../../runtimes/nandi-runtime/Dockerfile"),
                ),
                (
                    "requirements.txt",
                    include_bytes!("../../../runtimes/nandi-runtime/requirements.txt"),
                ),
                (
                    "app.py",
                    include_bytes!("../../../runtimes/nandi-runtime/app.py"),
                ),
                (
                    "relay.py",
                    include_bytes!("../../../runtimes/nandi-runtime/relay.py"),
                ),
            ],
        }
    }
}

/// The context as a tar archive, as Docker's build API takes it
pub fn context_tar(image: SandboxImage) -> Result<Vec<u8>, String> {
    let mut archive = tar::Builder::new(Vec::new());
    for (path, bytes) in image.context() {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        archive
            .append_data(&mut header, path, *bytes)
            .map_err(|e| format!("Couldn't pack {path}: {e}"))?;
    }
    archive
        .into_inner()
        .map_err(|e| format!("Couldn't pack the build context: {e}"))
}

/// One image: is it there, is Docker running, is a build going, and how
/// the last build went
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageStatus {
    pub tag: String,
    pub present: bool,
    /// Built for this app's sandbox (`runtime::SANDBOX_VERSION`); an image
    /// that's present but not current must be rebuilt before agents use it
    pub current: bool,
    pub docker_running: bool,
    /// Building it here
    pub building: bool,
    /// Downloading it from the registry
    pub pulling: bool,
    /// The app version that built the image here, when it says
    pub built_by: Option<String>,
    /// The last lines of the current or last build
    pub log: Vec<String>,
    /// Why the last build failed
    pub error: Option<String>,
}

/// What is getting an image right now
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Build,
    Pull,
}

#[derive(Debug, Default)]
struct BuildState {
    job: Option<Job>,
    log: VecDeque<String>,
    error: Option<String>,
}

static BUILDS: LazyLock<Mutex<HashMap<SandboxImage, BuildState>>> = LazyLock::new(Mutex::default);

fn builds() -> std::sync::MutexGuard<'static, HashMap<SandboxImage, BuildState>> {
    BUILDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The bus jobs started without an `AppState` (a run that needs an image)
/// report on; set once at startup
static BUS: OnceLock<EventBus> = OnceLock::new();

/// Lets downloads started by runs report their progress
pub fn init(bus: &EventBus) {
    let _ = BUS.set(bus.clone());
}

/// `images://build`: a line of output, or the end of a build or download
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageBuildEvent {
    pub image: SandboxImage,
    pub line: Option<String>,
    /// Set when the build ended: whether it worked
    pub finished: Option<bool>,
    pub error: Option<String>,
}

impl NandiEvent for ImageBuildEvent {
    const NAME: &'static str = "images://build";
}

/// Keeps the last [`LOG_LINES`] lines
fn push_line(log: &mut VecDeque<String>, line: &str) {
    for line in line.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(line.to_string());
    }
}

pub async fn image_status(image: SandboxImage) -> ImageStatus {
    let (job, log, error) = {
        let builds = builds();
        let state = builds.get(&image);
        (
            state.and_then(|s| s.job),
            state
                .map(|s| s.log.iter().cloned().collect())
                .unwrap_or_default(),
            state.and_then(|s| s.error.clone()),
        )
    };
    let mut status = ImageStatus {
        tag: image.tag().to_string(),
        building: job == Some(Job::Build),
        pulling: job == Some(Job::Pull),
        log,
        error,
        ..ImageStatus::default()
    };
    let Ok(docker) = Docker::connect_with_local_defaults() else {
        return status;
    };
    status.docker_running = docker.ping().await.is_ok();
    if status.docker_running {
        if let Ok(inspect) = docker.inspect_image(image.tag()).await {
            let labels = inspect.config.and_then(|c| c.labels);
            status.present = true;
            status.current = crate::runtime::image_is_current(labels.as_ref());
            status.built_by = labels.and_then(|labels| labels.get(VERSION_LABEL).cloned());
        }
    }
    status
}

/// Starts building `image` here in the background (refused while a build
/// or download of it runs)
pub fn start_build(state: &AppState, image: SandboxImage) -> Result<(), String> {
    let bus = state.bus().clone();
    start_job(image, Job::Build, Some(bus))
}

/// "Download": starts downloading `image` in the background (refused while
/// a build or download of it runs)
pub fn start_download(state: &AppState, image: SandboxImage) -> Result<(), String> {
    start_job(image, Job::Pull, Some(state.bus().clone()))
}

/// Starts downloading `image` in the background for a run that needs it,
/// unless a build or download of it already runs
pub fn start_pull(image: SandboxImage) {
    // Already busy: that job gets the image
    let _ = start_job(image, Job::Pull, BUS.get().cloned());
}

/// Downloads `image` and waits for it (or for the job already getting it)
pub async fn pull(image: SandboxImage) -> Result<(), String> {
    start_pull(image);
    loop {
        {
            let builds = builds();
            if let Some(state) = builds.get(&image) {
                if state.job.is_none() {
                    return match &state.error {
                        Some(e) => Err(e.clone()),
                        None => Ok(()),
                    };
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// Claims `image` for `job` (refused while another runs), then runs it in
/// the background, reporting on `bus`
fn start_job(image: SandboxImage, job: Job, bus: Option<EventBus>) -> Result<(), String> {
    {
        let mut builds = builds();
        let entry = builds.entry(image).or_default();
        if entry.job.is_some() {
            return Err(format!(
                "{} is already being built or downloaded",
                image.tag()
            ));
        }
        *entry = BuildState {
            job: Some(job),
            ..BuildState::default()
        };
    }
    tauri::async_runtime::spawn(async move {
        let emit = |event: ImageBuildEvent| {
            if let Some(bus) = &bus {
                events::emit(bus, &event);
            }
        };
        let output = |line: &str| {
            push_line(&mut builds().entry(image).or_default().log, line);
            emit(ImageBuildEvent {
                image,
                line: Some(line.to_string()),
                finished: None,
                error: None,
            });
        };
        let result = match job {
            Job::Build => build(image, output).await,
            Job::Pull => download(image, output).await,
        };
        let error = result.err();
        {
            let mut builds = builds();
            let entry = builds.entry(image).or_default();
            entry.job = None;
            entry.error = error.clone();
        }
        match (&error, job) {
            (Some(e), _) => log::warn!("Getting {} failed: {e}", image.tag()),
            (None, Job::Build) => log::info!("Built {}", image.tag()),
            (None, Job::Pull) => log::info!("Downloaded {}", image.tag()),
        }
        emit(ImageBuildEvent {
            image,
            line: None,
            finished: Some(error.is_none()),
            error,
        });
    });
    Ok(())
}

/// Docker, when it's running
async fn docker() -> Result<Docker, String> {
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| format!("Docker is not available: {e}"))?;
    docker
        .ping()
        .await
        .map_err(|_| "Docker isn't running. Start Docker Desktop, then try again.".to_string())?;
    Ok(docker)
}

/// What a failed download says: offline or no registry, and the way out
fn download_failed(image: SandboxImage, error: &str) -> String {
    format!(
        "Couldn't download {} ({error}). If this computer is offline or can't reach \
         ghcr.io, use Build here instead.",
        image.tag()
    )
}

/// `docker pull` of the pinned image, one line per layer change
async fn download(image: SandboxImage, mut output: impl FnMut(&str)) -> Result<(), String> {
    let docker = docker().await?;
    let (repository, tag) = image.pull_ref();
    output(&format!("Downloading {}…", image.tag()));
    let options = CreateImageOptionsBuilder::new()
        .from_image(repository)
        .tag(tag)
        .build();
    let mut stream = docker.create_image(Some(options), None, None);
    while let Some(item) = stream.next().await {
        let info = item.map_err(|e| download_failed(image, &e.to_string()))?;
        if let Some(line) = pull_line(info.id.as_deref(), info.status.as_deref()) {
            output(&line);
        }
    }
    output(&format!("Downloaded {}.", image.tag()));
    Ok(())
}

/// A pull's progress as a line: `<layer>: <status>`. The byte counters of
/// "Downloading" and "Extracting" are left out (one line each per layer).
fn pull_line(id: Option<&str>, status: Option<&str>) -> Option<String> {
    let status = status?.trim();
    if status.is_empty() {
        return None;
    }
    Some(match id.filter(|id| !id.is_empty()) {
        Some(id) => format!("{id}: {status}"),
        None => status.to_string(),
    })
}

/// Proxy settings the build's downloads (apt, npm, pip) need behind a
/// company proxy, from this process's environment
fn proxy_args() -> HashMap<String, String> {
    [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
    ]
    .into_iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| (name.to_string(), value))
    })
    .filter(|(_, value)| !value.trim().is_empty())
    .collect()
}

async fn build(image: SandboxImage, mut output: impl FnMut(&str)) -> Result<(), String> {
    let docker = docker().await?;
    let context = context_tar(image)?;
    let labels = HashMap::from([(VERSION_LABEL, env!("CARGO_PKG_VERSION"))]);
    let options = BuildImageOptionsBuilder::default()
        .dockerfile("Dockerfile")
        .t(image.tag())
        .rm(true)
        .forcerm(true)
        .labels(&labels)
        .buildargs(&proxy_args())
        .build();
    output(&format!("Building {}…", image.tag()));
    let mut stream = docker.build_image(options, None, Some(bollard::body_full(context.into())));
    while let Some(item) = stream.next().await {
        let info = item.map_err(|e| format!("The build failed: {e}"))?;
        if let Some(detail) = info.error_detail {
            return Err(detail
                .message
                .unwrap_or_else(|| "The build failed".to_string()));
        }
        if let Some(text) = info.stream.as_deref().or(info.status.as_deref()) {
            output(text);
        }
    }
    output(&format!("Built {}.", image.tag()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tar_bytes: &[u8]) -> Vec<String> {
        let mut archive = tar::Archive::new(tar_bytes);
        archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn each_context_has_its_dockerfile_and_what_it_copies() {
        assert_eq!(
            names(&context_tar(SandboxImage::Harness).unwrap()),
            [
                "Dockerfile",
                "codex-config.toml",
                "entrypoint.sh",
                "relay.mjs"
            ]
        );
        assert_eq!(
            names(&context_tar(SandboxImage::Runtime).unwrap()),
            ["Dockerfile", "requirements.txt", "app.py", "relay.py"]
        );
    }

    /// Every file a Dockerfile COPYs is in its context
    #[test]
    fn the_contexts_cover_every_copy() {
        for image in [SandboxImage::Harness, SandboxImage::Runtime] {
            let files: Vec<&str> = image.context().iter().map(|(p, _)| *p).collect();
            let dockerfile = std::str::from_utf8(image.context()[0].1).unwrap();
            for line in dockerfile
                .lines()
                .filter(|l| l.trim_start().starts_with("COPY"))
            {
                let source = line
                    .split_whitespace()
                    .filter(|w| !w.starts_with("--"))
                    .nth(1)
                    .unwrap();
                assert!(files.contains(&source), "{image:?} misses {source}");
            }
        }
    }

    #[test]
    fn the_log_keeps_the_last_lines_without_blanks() {
        let mut log = VecDeque::new();
        push_line(&mut log, "Step 1/9\n\n");
        assert_eq!(log, ["Step 1/9"]);
        for n in 0..LOG_LINES + 5 {
            push_line(&mut log, &format!("line {n}"));
        }
        assert_eq!(log.len(), LOG_LINES);
        assert_eq!(log.back().map(String::as_str), Some("line 304"));
    }

    #[test]
    fn tags_match_what_runs_them() {
        assert_eq!(
            SandboxImage::Harness.tag(),
            "ghcr.io/vaultdbai/nandi-harness:sandbox-5"
        );
        assert_eq!(
            SandboxImage::Runtime.tag(),
            "ghcr.io/vaultdbai/nandi-runtime:sandbox-5"
        );
    }

    /// The pinned tag follows the sandbox version, the Dockerfiles' label
    /// says the same version, and nothing floats on `latest`
    #[test]
    fn the_images_are_pinned_to_this_app_s_sandbox_version() {
        let pinned = format!("sandbox-{}", crate::runtime::SANDBOX_VERSION);
        for image in [SandboxImage::Harness, SandboxImage::Runtime] {
            let (repository, tag) = image.pull_ref();
            assert_eq!(tag, pinned, "{image:?}");
            assert!(
                repository.starts_with("ghcr.io/vaultdbai/nandi-"),
                "{repository}"
            );
            let dockerfile = std::str::from_utf8(image.context()[0].1).unwrap();
            let label = format!(
                "LABEL {}=\"{}\"",
                crate::runtime::SANDBOX_LABEL,
                crate::runtime::SANDBOX_VERSION
            );
            assert!(dockerfile.contains(&label), "{image:?} lacks {label}");
        }
    }

    #[test]
    fn pull_progress_reads_one_line_per_layer_change() {
        assert_eq!(
            pull_line(Some("a1b2"), Some("Pull complete")).as_deref(),
            Some("a1b2: Pull complete")
        );
        assert_eq!(
            pull_line(None, Some("Status: Downloaded newer image")).as_deref(),
            Some("Status: Downloaded newer image")
        );
        assert_eq!(pull_line(Some("a1b2"), Some("  ")), None);
        assert_eq!(pull_line(Some("a1b2"), None), None);
    }

    #[test]
    fn a_failed_download_points_to_the_local_build() {
        let message = download_failed(SandboxImage::Runtime, "no such host");
        assert!(message.contains("no such host"), "{message}");
        assert!(message.contains("Build here"), "{message}");
    }
}
