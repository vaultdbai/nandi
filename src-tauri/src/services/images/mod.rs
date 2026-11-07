//! Gets the sandbox images, so nobody builds anything: `nandi-runtime`
//! (the tool server for Nandi's own agents' tools) and `nandi-harness`
//! (Codex, Gemini CLI and the Claude Code tools volume, for agents with
//! `runtime: claude-code | codex | gemini-cli`). Ask and agents that only
//! chat need neither.
//!
//! **Pull only.** [`ensure`] is the one way the app gets one of its images:
//! is it here and built for this sandbox (`org.nandi.sandbox`), else pull
//! the public image pinned to this app's sandbox version from ghcr.io, with
//! its size and progress on `images://pull`, then check the label again.
//! A new [`SandboxImage`] variant (its tag and build context) is all another
//! image needs.
//!
//! | Way                  | What it does                                                                                |
//! | -------------------- | ------------------------------------------------------------------------------------------- |
//! | Download (pull)      | The default, and what runs do by themselves: the pinned image from ghcr.io                  |
//! | Build here (advanced) | Optional, never required: the contexts compiled in from `runtimes/`, built with bollard   |
//!
//! Both end with the same name ([`SandboxImage::tag`], from
//! `runtime::DEFAULT_RUNTIME_IMAGE` and `runtime::HARNESS_IMAGE`), so the
//! rest of the app doesn't care which. A failed download says so plainly;
//! it never falls back to a build.
//!
//! A job runs in the background: its output streams as `images://pull`
//! events (a local build's output too), and the last lines are kept so a
//! page opened mid-job (or after) shows them. A local build is labelled
//! with the app version that built it; the published images carry the
//! version of the release that built them.
//!
//! `tools`: tool bundles fetched on first use into a Docker volume (Claude
//! Code, which the harness image doesn't redistribute). `registry`: an
//! image's download size, read from its registry before the pull.

pub mod registry;
pub mod tools;

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bollard::query_parameters::{BuildImageOptionsBuilder, CreateImageOptionsBuilder};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;

/// Lines of output kept per image
const LOG_LINES: usize = 300;
/// Progress events go out at most this often
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// The label naming the app version that built an image
pub const VERSION_LABEL: &str = "org.nandi.built-by";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SandboxImage {
    /// Codex, Gemini CLI, and where Claude Code is installed on first use
    Harness,
    /// Nandi's own agents' tools
    Runtime,
    /// Published MCP connectors (Node, Python, uv)
    Connectors,
}

impl SandboxImage {
    /// The full name the app runs: `ghcr.io/vaultdbai/<name>:sandbox-<n>`
    pub fn tag(self) -> &'static str {
        match self {
            Self::Harness => crate::runtime::HARNESS_IMAGE,
            Self::Runtime => crate::runtime::DEFAULT_RUNTIME_IMAGE,
            Self::Connectors => crate::runtime::CONNECTORS_IMAGE,
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
                    include_bytes!("../../../../runtimes/nandi-harness/Dockerfile"),
                ),
                (
                    "codex-config.toml",
                    include_bytes!("../../../../runtimes/nandi-harness/codex-config.toml"),
                ),
                (
                    "entrypoint.sh",
                    include_bytes!("../../../../runtimes/nandi-harness/entrypoint.sh"),
                ),
                (
                    "install-tools.sh",
                    include_bytes!("../../../../runtimes/nandi-harness/install-tools.sh"),
                ),
                (
                    "relay.mjs",
                    include_bytes!("../../../../runtimes/nandi-harness/relay.mjs"),
                ),
                ("claude-code/package.json", tools::CLAUDE_CODE_PACKAGE),
                ("claude-code/package-lock.json", tools::CLAUDE_CODE_LOCK),
            ],
            Self::Runtime => &[
                (
                    "Dockerfile",
                    include_bytes!("../../../../runtimes/nandi-runtime/Dockerfile"),
                ),
                (
                    "requirements.txt",
                    include_bytes!("../../../../runtimes/nandi-runtime/requirements.txt"),
                ),
                (
                    "app.py",
                    include_bytes!("../../../../runtimes/nandi-runtime/app.py"),
                ),
                (
                    "relay.py",
                    include_bytes!("../../../../runtimes/nandi-runtime/relay.py"),
                ),
            ],
            Self::Connectors => &[
                (
                    "Dockerfile",
                    include_bytes!("../../../../runtimes/nandi-connectors/Dockerfile"),
                ),
                (
                    "relay.mjs",
                    include_bytes!("../../../../runtimes/nandi-connectors/relay.mjs"),
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
        // Scripts keep the execute bit the Dockerfile's chmod sets anyway
        header.set_mode(if path.ends_with(".sh") { 0o755 } else { 0o644 });
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

/// How far a download is: bytes received of the image's compressed size
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    #[specta(type = f64)]
    pub done_bytes: u64,
    /// From the registry before the pull, else the layers seen so far
    #[specta(type = Option<f64>)]
    pub total_bytes: Option<u64>,
}

/// One image: is it there, is Docker running, is a job going, and how the
/// last one went
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageStatus {
    pub tag: String,
    pub present: bool,
    /// Built for this app's sandbox (`runtime::SANDBOX_VERSION`); an image
    /// that's present but not current is downloaded again before use
    pub current: bool,
    pub docker_running: bool,
    /// Building it here (the advanced option)
    pub building: bool,
    /// Downloading it from the registry
    pub pulling: bool,
    /// How much to download (compressed), when the registry says
    #[specta(type = Option<f64>)]
    pub download_bytes: Option<u64>,
    /// The running download's progress
    pub progress: Option<PullProgress>,
    /// The app version that built the image here, when it says
    pub built_by: Option<String>,
    /// The last lines of the current or last job
    pub log: Vec<String>,
    /// Why the last job failed
    pub error: Option<String>,
}

/// What is getting an image right now
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Build,
    Pull,
}

#[derive(Debug, Default)]
struct JobState {
    job: Option<Job>,
    log: VecDeque<String>,
    progress: Option<PullProgress>,
    error: Option<String>,
}

static JOBS: LazyLock<Mutex<HashMap<SandboxImage, JobState>>> = LazyLock::new(Mutex::default);

fn jobs() -> std::sync::MutexGuard<'static, HashMap<SandboxImage, JobState>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The bus jobs started without an `AppState` (a run that needs an image)
/// report on; set once at startup
static BUS: OnceLock<EventBus> = OnceLock::new();

/// Lets downloads started by runs report their progress
pub fn init(bus: &EventBus) {
    let _ = BUS.set(bus.clone());
}

/// The bus set at startup, for jobs a run starts
pub(crate) fn bus() -> Option<EventBus> {
    BUS.get().cloned()
}

/// `images://pull`: a line of output, the progress, or the end of a
/// download (or of the advanced local build)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePullEvent {
    pub image: SandboxImage,
    /// The advanced local build's output, not a download's
    pub build: bool,
    pub line: Option<String>,
    pub progress: Option<PullProgress>,
    /// Set when the job ended: whether it worked
    pub finished: Option<bool>,
    pub error: Option<String>,
}

impl NandiEvent for ImagePullEvent {
    const NAME: &'static str = "images://pull";
}

/// Keeps the last [`LOG_LINES`] lines
pub(crate) fn push_line(log: &mut VecDeque<String>, line: &str) {
    for line in line.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(line.to_string());
    }
}

pub async fn image_status(image: SandboxImage) -> ImageStatus {
    let (job, log, progress, error) = {
        let jobs = jobs();
        let state = jobs.get(&image);
        (
            state.and_then(|s| s.job),
            state
                .map(|s| s.log.iter().cloned().collect())
                .unwrap_or_default(),
            state.and_then(|s| s.progress),
            state.and_then(|s| s.error.clone()),
        )
    };
    let mut status = ImageStatus {
        tag: image.tag().to_string(),
        building: job == Some(Job::Build),
        pulling: job == Some(Job::Pull),
        progress,
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
    if !status.current {
        status.download_bytes = registry::download_size(image.tag()).await;
    }
    status
}

/// Whether `image` is here and built for this sandbox
async fn is_current(docker: &Docker, image: SandboxImage) -> bool {
    docker.inspect_image(image.tag()).await.is_ok_and(|info| {
        crate::runtime::image_is_current(info.config.and_then(|c| c.labels).as_ref())
    })
}

/// Makes `image` ready to run: when it isn't here or is from an older
/// Nandi, downloads it (progress on `images://pull`) and checks its sandbox
/// label. What every container start calls first.
pub async fn ensure(image: SandboxImage) -> Result<(), String> {
    let docker = docker().await?;
    if is_current(&docker, image).await {
        return Ok(());
    }
    pull(image).await?;
    if is_current(&docker, image).await {
        Ok(())
    } else {
        Err(format!(
            "Downloaded {}, but it isn't built for this Nandi (sandbox {}). Update Nandi, or \
             try the download again later.",
            image.tag(),
            crate::runtime::SANDBOX_VERSION
        ))
    }
}

/// Starts building `image` here in the background (refused while a build
/// or download of it runs). The advanced option: nothing requires it.
pub fn start_build(state: &AppState, image: SandboxImage) -> Result<(), String> {
    start_job(image, Job::Build, Some(state.bus().clone()))
}

/// "Download": starts downloading `image` in the background (refused while
/// a build or download of it runs)
pub fn start_download(state: &AppState, image: SandboxImage) -> Result<(), String> {
    start_job(image, Job::Pull, Some(state.bus().clone()))
}

/// Downloads `image` and waits for it (or for the job already getting it)
async fn pull(image: SandboxImage) -> Result<(), String> {
    // Already busy: that job gets the image
    let _ = start_job(image, Job::Pull, bus());
    loop {
        {
            let jobs = jobs();
            if let Some(state) = jobs.get(&image) {
                if state.job.is_none() {
                    return match &state.error {
                        Some(e) => Err(e.clone()),
                        None => Ok(()),
                    };
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// What a job reports as it goes
struct Reporter {
    image: SandboxImage,
    job: Job,
    bus: Option<EventBus>,
    last_progress: Option<Instant>,
}

impl Reporter {
    fn emit(&self, event: ImagePullEvent) {
        if let Some(bus) = &self.bus {
            events::emit(bus, &event);
        }
    }

    fn line(&mut self, line: &str) {
        push_line(&mut jobs().entry(self.image).or_default().log, line);
        self.emit(ImagePullEvent {
            image: self.image,
            build: self.job == Job::Build,
            line: Some(line.to_string()),
            progress: None,
            finished: None,
            error: None,
        });
    }

    /// Sends the progress at most every [`PROGRESS_EVERY`] (and always the
    /// last, `force`)
    fn progress(&mut self, progress: PullProgress, force: bool) {
        jobs().entry(self.image).or_default().progress = Some(progress);
        if !force
            && self
                .last_progress
                .is_some_and(|at| at.elapsed() < PROGRESS_EVERY)
        {
            return;
        }
        self.last_progress = Some(Instant::now());
        self.emit(ImagePullEvent {
            image: self.image,
            build: false,
            line: None,
            progress: Some(progress),
            finished: None,
            error: None,
        });
    }
}

/// Claims `image` for `job` (refused while another runs), then runs it in
/// the background, reporting on `bus`
fn start_job(image: SandboxImage, job: Job, bus: Option<EventBus>) -> Result<(), String> {
    {
        let mut jobs = jobs();
        let entry = jobs.entry(image).or_default();
        if entry.job.is_some() {
            return Err(format!(
                "{} is already being downloaded or built",
                image.tag()
            ));
        }
        *entry = JobState {
            job: Some(job),
            ..JobState::default()
        };
    }
    tauri::async_runtime::spawn(async move {
        let mut reporter = Reporter {
            image,
            job,
            bus,
            last_progress: None,
        };
        let result = match job {
            Job::Build => build(image, &mut reporter).await,
            Job::Pull => download(image, &mut reporter).await,
        };
        let error = result.err();
        {
            let mut jobs = jobs();
            let entry = jobs.entry(image).or_default();
            entry.job = None;
            entry.error = error.clone();
        }
        match (&error, job) {
            (Some(e), _) => log::warn!("Getting {} failed: {e}", image.tag()),
            (None, Job::Build) => log::info!("Built {}", image.tag()),
            (None, Job::Pull) => log::info!("Downloaded {}", image.tag()),
        }
        reporter.emit(ImagePullEvent {
            image,
            build: job == Job::Build,
            line: None,
            progress: None,
            finished: Some(error.is_none()),
            error,
        });
    });
    Ok(())
}

/// Docker, when it's running
pub(crate) async fn docker() -> Result<Docker, String> {
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| format!("Docker is not available: {e}"))?;
    docker
        .ping()
        .await
        .map_err(|_| "Docker isn't running. Start Docker Desktop, then try again.".to_string())?;
    Ok(docker)
}

/// What a failed download says: what went wrong, and to try again
fn download_failed(image: SandboxImage, error: &str) -> String {
    format!(
        "Couldn't download {} ({error}). Check that this computer is online and can reach \
         ghcr.io, then try again.",
        image.tag()
    )
}

/// Bytes per layer while a pull runs, summed into [`PullProgress`]
#[derive(Debug, Default)]
struct LayerBytes {
    /// layer id → (downloaded, size)
    layers: HashMap<String, (u64, Option<u64>)>,
}

impl LayerBytes {
    /// Takes one pull message; whether the downloaded bytes moved
    fn update(
        &mut self,
        id: Option<&str>,
        status: Option<&str>,
        current: Option<i64>,
        total: Option<i64>,
    ) -> bool {
        let (Some(id), Some(status)) = (id.filter(|id| !id.is_empty()), status) else {
            return false;
        };
        let size = total.filter(|t| *t > 0).map(|t| t as u64);
        let entry = self.layers.entry(id.to_string()).or_insert((0, None));
        if size.is_some() && status == "Downloading" {
            entry.1 = size;
        }
        let before = entry.0;
        match status {
            "Downloading" => entry.0 = current.unwrap_or(0).max(0) as u64,
            // Done (or not needed): all of it
            "Download complete" | "Pull complete" | "Already exists" => {
                if let Some(size) = entry.1 {
                    entry.0 = size;
                }
            }
            _ => {}
        }
        entry.0 != before
    }

    /// `total`: the registry's size, else the sizes seen so far
    fn progress(&self, total: Option<u64>) -> PullProgress {
        let done = self.layers.values().map(|(done, _)| done).sum();
        let seen: u64 = self.layers.values().filter_map(|(_, size)| *size).sum();
        PullProgress {
            done_bytes: done,
            total_bytes: total.or((seen > 0).then_some(seen)).map(|t| t.max(done)),
        }
    }
}

/// `docker pull` of the pinned image: one line per layer change, and the
/// bytes received
async fn download(image: SandboxImage, reporter: &mut Reporter) -> Result<(), String> {
    let docker = docker().await?;
    let (repository, tag) = image.pull_ref();
    let total = registry::download_size(image.tag()).await;
    reporter.line(&match total {
        Some(bytes) => format!(
            "Downloading {} ({})…",
            image.tag(),
            registry::megabytes(bytes)
        ),
        None => format!("Downloading {}…", image.tag()),
    });
    let options = CreateImageOptionsBuilder::new()
        .from_image(repository)
        .tag(tag)
        .build();
    let mut layers = LayerBytes::default();
    let mut stream = docker.create_image(Some(options), None, None);
    while let Some(item) = stream.next().await {
        let info = item.map_err(|e| download_failed(image, &e.to_string()))?;
        let (current, size) = info
            .progress_detail
            .as_ref()
            .map(|p| (p.current, p.total))
            .unwrap_or_default();
        if layers.update(info.id.as_deref(), info.status.as_deref(), current, size) {
            reporter.progress(layers.progress(total), false);
        }
        if let Some(line) = pull_line(info.id.as_deref(), info.status.as_deref()) {
            reporter.line(&line);
        }
    }
    reporter.progress(layers.progress(total), true);
    reporter.line(&format!("Downloaded {}.", image.tag()));
    Ok(())
}

/// A pull's progress as a line: `<layer>: <status>`. The byte counters of
/// "Downloading" and "Extracting" are left out (they go in the progress).
fn pull_line(id: Option<&str>, status: Option<&str>) -> Option<String> {
    let status = status?.trim();
    if status.is_empty() || status == "Downloading" || status == "Extracting" {
        return None;
    }
    Some(match id.filter(|id| !id.is_empty()) {
        Some(id) => format!("{id}: {status}"),
        None => status.to_string(),
    })
}

/// Proxy settings downloads (apt, npm, pip) need behind a company proxy,
/// from this process's environment
pub(crate) fn proxy_env() -> HashMap<String, String> {
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

async fn build(image: SandboxImage, reporter: &mut Reporter) -> Result<(), String> {
    let docker = docker().await?;
    let context = context_tar(image)?;
    let labels = HashMap::from([(VERSION_LABEL, env!("CARGO_PKG_VERSION"))]);
    let options = BuildImageOptionsBuilder::default()
        .dockerfile("Dockerfile")
        .t(image.tag())
        .rm(true)
        .forcerm(true)
        .labels(&labels)
        .buildargs(&proxy_env())
        .build();
    reporter.line(&format!("Building {}…", image.tag()));
    let mut stream = docker.build_image(options, None, Some(bollard::body_full(context.into())));
    while let Some(item) = stream.next().await {
        let info = item.map_err(|e| format!("The build failed: {e}"))?;
        if let Some(detail) = info.error_detail {
            return Err(detail
                .message
                .unwrap_or_else(|| "The build failed".to_string()));
        }
        if let Some(text) = info.stream.as_deref().or(info.status.as_deref()) {
            reporter.line(text);
        }
    }
    reporter.line(&format!("Built {}.", image.tag()));
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
                "install-tools.sh",
                "relay.mjs",
                "claude-code/package.json",
                "claude-code/package-lock.json"
            ]
        );
        assert_eq!(
            names(&context_tar(SandboxImage::Runtime).unwrap()),
            ["Dockerfile", "requirements.txt", "app.py", "relay.py"]
        );
        assert_eq!(
            names(&context_tar(SandboxImage::Connectors).unwrap()),
            ["Dockerfile", "relay.mjs"]
        );
    }

    /// Every file a Dockerfile COPYs is in its context
    #[test]
    fn the_contexts_cover_every_copy() {
        for image in [
            SandboxImage::Harness,
            SandboxImage::Runtime,
            SandboxImage::Connectors,
        ] {
            let files: Vec<&str> = image.context().iter().map(|(p, _)| *p).collect();
            let dockerfile = std::str::from_utf8(image.context()[0].1).unwrap();
            for line in dockerfile
                .lines()
                .filter(|l| l.trim_start().starts_with("COPY"))
            {
                let words: Vec<&str> = line
                    .split_whitespace()
                    .skip(1)
                    .filter(|w| !w.starts_with("--"))
                    .collect();
                // The last word is the destination
                for source in &words[..words.len() - 1] {
                    assert!(files.contains(source), "{image:?} misses {source}");
                }
            }
        }
    }

    /// Claude Code isn't redistributed: the harness image installs no
    /// Anthropic package, only ships the lockfile the first use installs
    #[test]
    fn the_harness_image_does_not_bundle_claude_code() {
        let dockerfile = std::str::from_utf8(SandboxImage::Harness.context()[0].1).unwrap();
        let installs: Vec<&str> = dockerfile
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect();
        for line in installs {
            assert!(!line.contains("@anthropic-ai/"), "{line}");
            assert!(!line.contains("claude-agent-acp"), "{line}");
        }
        assert!(dockerfile.contains(tools::ToolBundle::ClaudeCode.bin_dir()));
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
            "ghcr.io/vaultdbai/nandi-harness:sandbox-6"
        );
        assert_eq!(
            SandboxImage::Runtime.tag(),
            "ghcr.io/vaultdbai/nandi-runtime:sandbox-6"
        );
        assert_eq!(
            SandboxImage::Connectors.tag(),
            "ghcr.io/vaultdbai/nandi-connectors:sandbox-6"
        );
    }

    /// The pinned tag follows the sandbox version, the Dockerfiles' label
    /// says the same version, and nothing floats on `latest`
    #[test]
    fn the_images_are_pinned_to_this_app_s_sandbox_version() {
        let pinned = format!("sandbox-{}", crate::runtime::SANDBOX_VERSION);
        for image in [
            SandboxImage::Harness,
            SandboxImage::Runtime,
            SandboxImage::Connectors,
        ] {
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
    fn pull_lines_leave_the_byte_counters_to_the_progress() {
        assert_eq!(
            pull_line(Some("a1b2"), Some("Pull complete")).as_deref(),
            Some("a1b2: Pull complete")
        );
        assert_eq!(
            pull_line(None, Some("Status: Downloaded newer image")).as_deref(),
            Some("Status: Downloaded newer image")
        );
        assert_eq!(pull_line(Some("a1b2"), Some("Downloading")), None);
        assert_eq!(pull_line(Some("a1b2"), Some("Extracting")), None);
        assert_eq!(pull_line(Some("a1b2"), Some("  ")), None);
        assert_eq!(pull_line(Some("a1b2"), None), None);
    }

    #[test]
    fn progress_sums_the_layers_against_the_registry_size() {
        let mut layers = LayerBytes::default();
        assert!(layers.update(Some("a"), Some("Downloading"), Some(10), Some(100)));
        assert!(layers.update(Some("b"), Some("Downloading"), Some(5), Some(50)));
        assert!(!layers.update(Some("c"), Some("Waiting"), None, None));
        // The registry's size wins; without it, the layers seen so far
        assert_eq!(
            layers.progress(Some(400)),
            PullProgress {
                done_bytes: 15,
                total_bytes: Some(400)
            }
        );
        assert_eq!(layers.progress(None).total_bytes, Some(150));
        // A finished layer counts in full; one already here too
        assert!(layers.update(Some("a"), Some("Download complete"), None, None));
        assert!(!layers.update(Some("d"), Some("Already exists"), None, None));
        assert_eq!(layers.progress(Some(400)).done_bytes, 105);
        // No id: a status line, not a layer
        assert!(!layers.update(None, Some("Downloading"), Some(1), Some(2)));
    }

    #[test]
    fn a_failed_download_says_to_try_again_never_to_build() {
        let message = download_failed(SandboxImage::Runtime, "no such host");
        assert!(message.contains("no such host"), "{message}");
        assert!(message.contains("try again"), "{message}");
        assert!(!message.contains("Build"), "{message}");
    }

    /// Tauri accepts only these characters in an event name
    #[test]
    fn the_event_names_are_valid_tauri_event_names() {
        for name in [ImagePullEvent::NAME, tools::ToolsEvent::NAME] {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-/:_".contains(c)),
                "{name}"
            );
        }
    }
}
