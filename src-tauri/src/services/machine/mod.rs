//! "Get your computer ready": is this computer ready for agents to act?
//!
//! Agents act only inside containers, so Docker is what lets them use
//! tools. Without it Nandi still works as a chat assistant; nothing here
//! blocks anything. This module checks, read-only, what's there:
//!
//! | Check            | Windows                        | macOS                          | Linux                                   |
//! | ---------------- | ------------------------------ | ------------------------------ | --------------------------------------- |
//! | `dockerInstalled`| Docker Desktop (registry/path) | `Docker.app`, the chip's build | Docker Engine or Desktop; Podman/rootless noted |
//! | `dockerRunning`  | API ping, then `docker version`, `docker desktop status`, the Desktop process | same | API ping, `systemctl is-active docker`, socket permission |
//! | `wsl`            | `wsl --version`, `wsl --status`| —                              | —                                       |
//! | `virtualization` | Hypervisor / firmware flag     | Apple silicon, or `kern.hv_support` | —                                  |
//! | `disk`           | free space where Docker keeps images (about 15 GB) | same       | same                                    |
//! | `images`         | Nandi's runtime image here, or its manifest reachable on ghcr.io | same | same                    |
//!
//! [`probe`] runs the commands (fixed arguments, no shell, short timeouts,
//! never elevated) and [`assess`] turns what they printed into
//! [`MachineCheck`]s: pure, with a fixture test for every state. The only
//! network call is the image check.
//!
//! Each check's actions are [`MachineAction::StartDocker`] (launches
//! Docker Desktop; no admin), links to official pages, and check again.
//! Phase 2 (Windows and macOS): [`ready::plan`] is the report's `plan`,
//! the steps that get the computer ready (`wsl --update` or `wsl
//! --install`, install Docker Desktop, start it). The user approves them
//! once; [`install`] runs them. Linux stays phase 1.

pub mod assess;
pub mod install;
pub mod parse;
mod probe;
pub mod ready;

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

pub use parse::LinuxDistro;
pub use ready::ReadyStep;

use crate::services::AppState;

/// About this much free space is needed for Nandi's images
pub const DISK_NEEDED_BYTES: u64 = 15 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MachineOs {
    Windows,
    Macos,
    Linux,
    #[default]
    Other,
}

impl MachineOs {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CheckId {
    DockerInstalled,
    DockerRunning,
    Wsl,
    Virtualization,
    Disk,
    Images,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    Ok,
    Missing,
    Stopped,
    NeedsUpdate,
    /// By policy, permissions, a proxy, the firmware or a failed engine
    Blocked,
    Unknown,
}

/// Why a check is in its state: the UI picks its sentence and steps by it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CheckReason {
    Ready,
    NotInstalled,
    /// macOS: the Intel build of Docker Desktop on Apple silicon
    WrongBuild,
    /// Linux: the `docker` command is Podman's
    Podman,
    /// Linux: only rootless Docker was found
    Rootless,
    /// Docker isn't installed, so it can't run
    NeedsDocker,
    NotRunning,
    Starting,
    /// "Docker Desktop is unable to start": the app is there, its engine failed
    DesktopFailed,
    /// Linux: permission denied on `/var/run/docker.sock`
    SocketPermission,
    /// The WSL built into Windows, or older than Docker needs
    OldWsl,
    KernelUpdate,
    VirtualizationOff,
    LowDisk,
    /// The runtime image is on this computer already
    Downloaded,
    Reachable,
    /// No answer from ghcr.io (offline, firewall, proxy)
    Unreachable,
    /// A certificate Nandi doesn't trust (often a company proxy)
    Certificate,
    /// The proxy wants a sign-in
    ProxyAuth,
    /// The registry refused (a company policy)
    Refused,
    /// Nandi couldn't tell
    NotChecked,
}

/// What the user can do about a check, in phase 1. Phase 2 adds the
/// approved installs and fixes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MachineAction {
    /// Launches Docker Desktop (no admin rights)
    StartDocker,
    /// An official download or docs page
    OpenLink {
        url: String,
    },
    CheckAgain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MachineCheck {
    pub id: CheckId,
    pub state: CheckState,
    pub reason: CheckReason,
    /// What Nandi saw: a path, a version, the error line
    pub detail: String,
    pub actions: Vec<MachineAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MachineReport {
    pub os: MachineOs,
    /// `x86_64`, `aarch64`
    pub arch: String,
    /// macOS on an M-series chip (even when Nandi runs under Rosetta)
    pub apple_silicon: bool,
    /// Linux: from `/etc/os-release`
    pub distro: Option<LinuxDistro>,
    pub checks: Vec<MachineCheck>,
    /// Docker answers: agents can act
    pub docker_ready: bool,
    /// Every check is `ok`
    pub ready: bool,
    /// RFC 3339
    pub checked_at: String,
    /// What "Get your computer ready" would run, in order (Windows, macOS)
    pub plan: Vec<ReadyStep>,
}

/// The slow, rarely-changing observations (WSL, virtualization, disk, the
/// registry) are reused for this long; Docker itself is asked every time
const SLOW_TTL: Duration = Duration::from_secs(60);

static SLOW: LazyLock<Mutex<Option<(Instant, probe::Slow)>>> = LazyLock::new(Mutex::default);

/// Checks this computer. `fresh` re-runs the slow checks too (the user
/// pressed Check again); otherwise they come from the last minute.
pub async fn check(fresh: bool) -> MachineReport {
    let os = MachineOs::current();
    let fast = probe::fast(os).await;
    let cached = SLOW
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .filter(|(at, _)| !fresh && at.elapsed() < SLOW_TTL)
        .map(|(_, slow)| slow);
    let slow = match cached {
        Some(slow) => slow,
        None => {
            let slow = probe::slow(os, fast.engine_version.is_some()).await;
            if let Ok(mut guard) = SLOW.lock() {
                *guard = Some((Instant::now(), slow.clone()));
            }
            slow
        }
    };
    let observations = probe::combine(os, fast, slow);
    assess::report(&observations, chrono::Utc::now().to_rfc3339())
}

/// Starts Docker Desktop: no admin rights, nothing installed. Linux Docker
/// Engine needs `sudo systemctl start docker`, which Nandi shows rather
/// than runs.
pub async fn start_docker() -> Result<(), String> {
    probe::start_docker(MachineOs::current()).await
}

/// The machine's portable settings (in `settings.yaml`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MachineSettings {
    /// Start Docker when Nandi opens; `None`: not asked yet
    pub start_docker_on_open: Option<bool>,
}

pub fn settings(state: &AppState) -> MachineSettings {
    MachineSettings {
        start_docker_on_open: state
            .store()
            .read(|repo| repo.settings().start_docker_on_open()),
    }
}

/// The answer to "Start Docker automatically when Nandi opens?"
pub async fn set_start_docker_on_open(state: &AppState, start: bool) -> Result<(), String> {
    use crate::services::domain::EntityKind;
    use crate::services::repos::SingletonRepository;

    let mut settings = state.store().read(|repo| repo.settings().clone());
    if settings.start_docker_on_open() == Some(start) {
        return Ok(());
    }
    settings.set_start_docker_on_open(start);
    let store = state.store().clone();
    tokio::task::spawn_blocking(move || {
        store.repos().settings.save(&settings)?;
        store.commit(EntityKind::Settings, "settings")?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Settings write failed: {e}"))?
}

/// Whether Nandi starts Docker now, as it opens
fn starts_on_open(setting: Option<bool>, report: &MachineReport) -> bool {
    setting == Some(true)
        && report
            .checks
            .iter()
            .any(|c| c.id == CheckId::DockerRunning && c.reason == CheckReason::NotRunning)
}

/// At launch, when the user said yes: starts Docker Desktop in the
/// background if it's stopped. Nothing waits for it.
pub fn start_docker_on_open(state: &AppState) {
    let setting = settings(state).start_docker_on_open;
    if setting != Some(true) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let report = check(false).await;
        if !starts_on_open(setting, &report) {
            return;
        }
        match start_docker().await {
            Ok(()) => log::info!("Started Docker, as Nandi opened"),
            Err(e) => log::warn!("Docker didn't start as Nandi opened: {e}"),
        }
    });
}

#[cfg(test)]
pub(crate) use probe::use_registry_for_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn running(reason: CheckReason) -> MachineReport {
        MachineReport {
            os: MachineOs::Windows,
            arch: "x86_64".into(),
            apple_silicon: false,
            distro: None,
            checks: vec![MachineCheck {
                id: CheckId::DockerRunning,
                state: CheckState::Stopped,
                reason,
                detail: String::new(),
                actions: vec![],
            }],
            docker_ready: false,
            ready: false,
            checked_at: String::new(),
            plan: vec![],
        }
    }

    #[test]
    fn docker_starts_on_open_only_when_asked_and_stopped() {
        let stopped = running(CheckReason::NotRunning);
        assert!(starts_on_open(Some(true), &stopped));
        assert!(!starts_on_open(Some(false), &stopped));
        assert!(!starts_on_open(None, &stopped));
        // Starting, failed or not installed: nothing to start
        for reason in [
            CheckReason::Starting,
            CheckReason::DesktopFailed,
            CheckReason::NeedsDocker,
            CheckReason::Ready,
        ] {
            assert!(!starts_on_open(Some(true), &running(reason)), "{reason:?}");
        }
    }

    #[tokio::test]
    async fn the_start_docker_answer_is_a_portable_setting() {
        let state = AppState::for_tests(vec![]);
        assert_eq!(settings(&state).start_docker_on_open, None);
        set_start_docker_on_open(&state, true).await.unwrap();
        assert_eq!(settings(&state).start_docker_on_open, Some(true));
        let yaml = std::fs::read_to_string(state.store().layout().settings_path()).unwrap();
        assert!(yaml.contains("start_docker_on_open: true"), "{yaml}");
        set_start_docker_on_open(&state, false).await.unwrap();
        assert_eq!(settings(&state).start_docker_on_open, Some(false));
        state.shutdown();
    }

    /// `cargo test --lib live_machine_report -- --ignored --nocapture`:
    /// what this computer reports, for manual checks
    #[tokio::test]
    #[ignore]
    async fn live_machine_report() {
        let started = std::time::Instant::now();
        let report = super::check(true).await;
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        println!("took {:?}", started.elapsed());
        let started = std::time::Instant::now();
        let _ = super::check(false).await;
        println!("cached took {:?}", started.elapsed());
    }
}
