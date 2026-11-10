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
//! Phase 1 actions are [`MachineAction::StartDocker`] (launches Docker
//! Desktop; no admin), links to official pages, and check again. Phase 2's
//! approved installs and fixes (winget, brew, `wsl --update`, pkexec) add
//! variants to [`MachineAction`]; the UI already leaves their place.

pub mod assess;
pub mod parse;
mod probe;

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

pub use parse::LinuxDistro;

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

#[cfg(test)]
pub(crate) use probe::use_registry_for_tests;

#[cfg(test)]
mod tests {
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
