//! What the probes saw → the checklist. Pure: every state is a fixture
//! test below, for Windows, macOS and Linux.

use super::parse::{self, DistroFamily, DockerError, LinuxDistro, WslFinding};
use super::{
    CheckId, CheckReason, CheckState, MachineAction, MachineCheck, MachineOs, MachineReport,
    DISK_NEEDED_BYTES,
};

/// One command's result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// The program isn't on this computer
    NotFound,
    TimedOut,
    Done {
        code: i32,
        stdout: String,
        stderr: String,
    },
}

impl Cmd {
    pub fn done(code: i32, stdout: &str, stderr: &str) -> Self {
        Self::Done {
            code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    /// Exit code and stdout + stderr, when it ran
    fn output(&self) -> Option<(i32, String)> {
        match self {
            Self::Done {
                code,
                stdout,
                stderr,
            } => Some((*code, format!("{stdout}\n{stderr}").trim().to_string())),
            _ => None,
        }
    }

    fn stdout(&self) -> Option<&str> {
        match self {
            Self::Done { stdout, .. } => Some(stdout),
            _ => None,
        }
    }
}

/// The image check's finding
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ImagesFinding {
    /// The runtime image is here already
    Downloaded,
    Reachable,
    Unreachable(String),
    Certificate(String),
    ProxyAuth,
    Refused(u16),
    #[default]
    NotChecked,
}

/// Everything the probes saw, raw
#[derive(Debug, Clone, Default)]
pub struct Observations {
    pub os: MachineOs,
    pub arch: String,
    pub apple_silicon: bool,
    /// Where Docker Desktop is (Windows `.exe`, macOS `.app`, Linux `/opt/docker-desktop`)
    pub desktop_app: Option<String>,
    /// macOS: `lipo -archs` of Docker Desktop's binary
    pub desktop_archs: Option<String>,
    /// `docker` is on PATH
    pub docker_cli: bool,
    /// `docker --version`
    pub cli_version: Option<Cmd>,
    /// Linux: `dockerd` is on PATH
    pub dockerd: bool,
    /// Linux: `/var/run/docker.sock` exists
    pub system_socket: bool,
    /// Linux: `$XDG_RUNTIME_DIR/docker.sock` exists
    pub rootless_socket: bool,
    /// The engine's version, when the API ping answered
    pub engine_version: Option<String>,
    /// `docker version`, run only when the ping didn't answer
    pub docker_version: Option<Cmd>,
    /// `docker desktop status`
    pub desktop_status: Option<Cmd>,
    /// Docker Desktop's process is running
    pub desktop_process: bool,
    /// Linux: `systemctl is-active docker`
    pub systemctl: Option<Cmd>,
    pub wsl_version: Option<Cmd>,
    pub wsl_status: Option<Cmd>,
    /// Windows: the PowerShell probe's `key=value` lines
    pub windows_probe: Option<Cmd>,
    /// Windows: `systeminfo`, only when the probe couldn't tell (English
    /// output only; slower)
    pub systeminfo: Option<Cmd>,
    /// macOS Intel: `sysctl -n kern.hv_support`
    pub hv_support: Option<Cmd>,
    /// Where Docker keeps its images, and `df -Pk` of it (macOS, Linux)
    pub data_path: String,
    pub df: Option<Cmd>,
    pub os_release: Option<String>,
    pub images: ImagesFinding,
}

pub mod links {
    pub const WINDOWS_INSTALL: &str =
        "https://docs.docker.com/desktop/setup/install/windows-install/";
    pub const MAC_INSTALL: &str = "https://docs.docker.com/desktop/setup/install/mac-install/";
    pub const MAC_ARM_DMG: &str = "https://desktop.docker.com/mac/main/arm64/Docker.dmg";
    pub const MAC_INTEL_DMG: &str = "https://desktop.docker.com/mac/main/amd64/Docker.dmg";
    pub const LINUX_INSTALL: &str = "https://docs.docker.com/engine/install/";
    pub const ARCH_WIKI: &str = "https://wiki.archlinux.org/title/Docker";
    pub const LINUX_POSTINSTALL: &str = "https://docs.docker.com/engine/install/linux-postinstall/";
    pub const TROUBLESHOOT: &str =
        "https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/";
    pub const VIRTUALIZATION: &str =
        "https://docs.docker.com/desktop/troubleshoot-and-support/troubleshoot/topics/#virtualization";
    pub const WSL_INSTALL: &str = "https://learn.microsoft.com/windows/wsl/install";
    pub const PROXY: &str =
        "https://docs.docker.com/desktop/settings-and-maintenance/settings/#proxies";
}

/// Docker's official install page for this distribution
pub fn linux_install_link(distro: Option<&LinuxDistro>) -> String {
    let Some(distro) = distro else {
        return links::LINUX_INSTALL.to_string();
    };
    let page = match distro.id.as_str() {
        id @ ("ubuntu" | "debian" | "fedora" | "rhel" | "centos" | "raspbian" | "sles") => Some(id),
        _ => match distro.family {
            DistroFamily::Debian if distro.id_like.iter().any(|l| l == "ubuntu") => Some("ubuntu"),
            DistroFamily::Debian => Some("debian"),
            DistroFamily::Fedora if distro.id_like.iter().any(|l| l == "rhel") => Some("rhel"),
            DistroFamily::Fedora => Some("fedora"),
            DistroFamily::Arch => return links::ARCH_WIKI.to_string(),
            DistroFamily::Other => None,
        },
    };
    match page {
        Some(page) => format!("{}{page}/", links::LINUX_INSTALL),
        None => links::LINUX_INSTALL.to_string(),
    }
}

fn link(url: impl Into<String>) -> MachineAction {
    MachineAction::OpenLink { url: url.into() }
}

fn check(
    id: CheckId,
    state: CheckState,
    reason: CheckReason,
    detail: impl Into<String>,
    mut actions: Vec<MachineAction>,
) -> MachineCheck {
    if state != CheckState::Ok {
        actions.push(MachineAction::CheckAgain);
    }
    MachineCheck {
        id,
        state,
        reason,
        detail: detail.into(),
        actions,
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

pub fn report(o: &Observations, checked_at: String) -> MachineReport {
    let distro = (o.os == MachineOs::Linux)
        .then(|| o.os_release.as_deref().map(parse::os_release))
        .flatten();
    let installed = docker_installed(o, distro.as_ref());
    let running = docker_running(o, &installed);
    let mut checks = vec![installed, running];
    if o.os == MachineOs::Windows {
        checks.push(wsl(o));
    }
    if matches!(o.os, MachineOs::Windows | MachineOs::Macos) {
        checks.push(virtualization(o));
    }
    checks.push(disk(o));
    checks.push(images(o));
    let docker_ready = checks
        .iter()
        .any(|c| c.id == CheckId::DockerRunning && c.state == CheckState::Ok);
    MachineReport {
        os: o.os,
        arch: o.arch.clone(),
        apple_silicon: o.apple_silicon,
        ready: checks.iter().all(|c| c.state == CheckState::Ok),
        distro,
        checks,
        docker_ready,
        checked_at,
    }
}

fn download_actions(o: &Observations, distro: Option<&LinuxDistro>) -> Vec<MachineAction> {
    match o.os {
        MachineOs::Windows => vec![link(links::WINDOWS_INSTALL)],
        MachineOs::Macos => vec![
            link(if o.apple_silicon {
                links::MAC_ARM_DMG
            } else {
                links::MAC_INTEL_DMG
            }),
            link(links::MAC_INSTALL),
        ],
        _ => vec![link(linux_install_link(distro))],
    }
}

fn docker_installed(o: &Observations, distro: Option<&LinuxDistro>) -> MachineCheck {
    use CheckState::*;
    let id = CheckId::DockerInstalled;
    let podman = o.cli_is_podman();
    if o.os == MachineOs::Linux && podman {
        return check(
            id,
            Unknown,
            CheckReason::Podman,
            "The docker command is Podman's",
            vec![link(linux_install_link(distro))],
        );
    }
    if let Some(app) = &o.desktop_app {
        if o.os == MachineOs::Macos && o.apple_silicon {
            if let Some(archs) = &o.desktop_archs {
                if !archs.contains("arm64") {
                    return check(
                        id,
                        NeedsUpdate,
                        CheckReason::WrongBuild,
                        format!("{app} is the Intel build ({})", archs.trim()),
                        vec![link(links::MAC_ARM_DMG)],
                    );
                }
            }
        }
        return check(
            id,
            Ok,
            CheckReason::Ready,
            format!("Docker Desktop at {app}"),
            vec![],
        );
    }
    if o.os == MachineOs::Linux
        && o.rootless_socket
        && !o.system_socket
        && o.engine_version.is_none()
    {
        return check(
            id,
            Unknown,
            CheckReason::Rootless,
            "Only rootless Docker was found",
            vec![link(linux_install_link(distro))],
        );
    }
    if o.docker_cli || o.dockerd || o.engine_version.is_some() {
        let version = o
            .cli_version
            .as_ref()
            .and_then(Cmd::stdout)
            .map(first_line)
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "The docker command".to_string());
        return check(id, Ok, CheckReason::Ready, version, vec![]);
    }
    check(
        id,
        Missing,
        CheckReason::NotInstalled,
        "No Docker found",
        download_actions(o, distro),
    )
}

impl Observations {
    fn cli_is_podman(&self) -> bool {
        self.cli_version
            .as_ref()
            .and_then(Cmd::output)
            .is_some_and(|(_, text)| text.to_lowercase().contains("podman"))
    }
}

fn docker_running(o: &Observations, installed: &MachineCheck) -> MachineCheck {
    use CheckState::*;
    let id = CheckId::DockerRunning;
    if let Some(version) = &o.engine_version {
        return check(
            id,
            Ok,
            CheckReason::Ready,
            format!("Docker Engine {version}"),
            vec![],
        );
    }
    if installed.state == Missing {
        return check(
            id,
            Missing,
            CheckReason::NeedsDocker,
            "Docker isn't installed",
            vec![],
        );
    }
    let error = o
        .docker_version
        .as_ref()
        .and_then(Cmd::output)
        .map(|(_, text)| text)
        .unwrap_or_default();
    let line = first_line(&error);
    let desktop = o.desktop_app.is_some();
    let start = || {
        if desktop {
            vec![MachineAction::StartDocker]
        } else {
            vec![]
        }
    };
    match parse::docker_error(&error) {
        DockerError::SocketPermission => {
            return check(
                id,
                Blocked,
                CheckReason::SocketPermission,
                line,
                vec![link(links::LINUX_POSTINSTALL)],
            )
        }
        DockerError::DesktopFailed => return desktop_failed(line),
        _ => {}
    }
    if o.os == MachineOs::Linux && !desktop {
        let service = o
            .systemctl
            .as_ref()
            .and_then(Cmd::stdout)
            .and_then(parse::systemctl_state);
        return match service.as_deref() {
            Some("activating") => check(
                id,
                Stopped,
                CheckReason::Starting,
                "docker.service is starting",
                vec![],
            ),
            Some(state @ ("inactive" | "failed" | "unknown")) => check(
                id,
                Stopped,
                CheckReason::NotRunning,
                format!("docker.service is {state}"),
                vec![],
            ),
            _ => check(id, Stopped, CheckReason::NotRunning, line, vec![]),
        };
    }
    let status = o
        .desktop_status
        .as_ref()
        .and_then(Cmd::stdout)
        .and_then(parse::desktop_status);
    match status.as_deref() {
        Some("starting") => check(
            id,
            Stopped,
            CheckReason::Starting,
            "Docker Desktop is starting",
            vec![],
        ),
        Some("stopped") | Some("paused") | None if !o.desktop_process => check(
            id,
            Stopped,
            CheckReason::NotRunning,
            if line.is_empty() {
                "Docker isn't running".to_string()
            } else {
                line
            },
            start(),
        ),
        Some("stopped") | Some("paused") => check(
            id,
            Stopped,
            CheckReason::NotRunning,
            "Docker Desktop is open; its engine is stopped",
            start(),
        ),
        // The app says it runs, or says something else, and the engine
        // doesn't answer: it failed to start
        Some(other) => desktop_failed(format!(
            "Docker Desktop reports '{other}'; the engine doesn't answer"
        )),
        // Open, no status command (older Docker Desktop), no engine yet
        None => check(
            id,
            Stopped,
            CheckReason::Starting,
            "Docker Desktop is open; its engine isn't answering yet",
            vec![link(links::TROUBLESHOOT)],
        ),
    }
}

fn desktop_failed(detail: impl Into<String>) -> MachineCheck {
    check(
        CheckId::DockerRunning,
        CheckState::Blocked,
        CheckReason::DesktopFailed,
        detail,
        vec![MachineAction::StartDocker, link(links::TROUBLESHOOT)],
    )
}

fn wsl(o: &Observations) -> MachineCheck {
    use CheckState::*;
    let id = CheckId::Wsl;
    let not_found = matches!(o.wsl_status, Some(Cmd::NotFound) | None)
        && matches!(o.wsl_version, Some(Cmd::NotFound) | None);
    if not_found {
        return check(
            id,
            Missing,
            CheckReason::NotInstalled,
            "wsl.exe wasn't found",
            vec![link(links::WSL_INSTALL)],
        );
    }
    let version = o.wsl_version.as_ref().and_then(Cmd::output);
    let status = o.wsl_status.as_ref().and_then(Cmd::output);
    let finding = parse::wsl_finding(
        version.as_ref().map(|(c, t)| (*c, t.as_str())),
        status.as_ref().map(|(c, t)| (*c, t.as_str())),
    );
    match finding {
        WslFinding::Version => {
            let text = version.map(|(_, t)| t).unwrap_or_default();
            let found = parse::wsl_version(&text).unwrap_or_default();
            if parse::at_least(&parse::version_numbers(&found), parse::MIN_WSL) {
                check(id, Ok, CheckReason::Ready, format!("WSL {found}"), vec![])
            } else {
                check(
                    id,
                    NeedsUpdate,
                    CheckReason::OldWsl,
                    format!("WSL {found}; Docker needs 2.1.5 or later"),
                    vec![link(links::WSL_INSTALL)],
                )
            }
        }
        WslFinding::NotInstalled => check(
            id,
            Missing,
            CheckReason::NotInstalled,
            first_line(&status.or(version).map(|(_, t)| t).unwrap_or_default()),
            vec![link(links::WSL_INSTALL)],
        ),
        WslFinding::KernelUpdate => check(
            id,
            NeedsUpdate,
            CheckReason::KernelUpdate,
            "WSL 2 needs its kernel updated",
            vec![link(links::WSL_INSTALL)],
        ),
        WslFinding::Inbox => check(
            id,
            NeedsUpdate,
            CheckReason::OldWsl,
            "The WSL built into Windows; Docker needs the current WSL",
            vec![link(links::WSL_INSTALL)],
        ),
        WslFinding::Unknown => check(
            id,
            Unknown,
            CheckReason::NotChecked,
            match (&o.wsl_version, &o.wsl_status) {
                (Some(Cmd::TimedOut), _) | (_, Some(Cmd::TimedOut)) => {
                    "wsl.exe didn't answer in time".to_string()
                }
                _ => "Nandi couldn't read WSL's status".to_string(),
            },
            vec![link(links::WSL_INSTALL)],
        ),
    }
}

fn virtualization(o: &Observations) -> MachineCheck {
    use CheckState::*;
    let id = CheckId::Virtualization;
    let on = match o.os {
        MachineOs::Macos if o.apple_silicon => Some(true),
        MachineOs::Macos => o
            .hv_support
            .as_ref()
            .and_then(Cmd::stdout)
            .map(str::trim)
            .and_then(|v| match v {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            }),
        _ => o
            .windows_probe
            .as_ref()
            .and_then(Cmd::stdout)
            .and_then(|out| parse::windows_virtualization(&parse::key_values(out)))
            .or_else(|| {
                o.systeminfo
                    .as_ref()
                    .and_then(Cmd::stdout)
                    .and_then(parse::systeminfo_virtualization)
            }),
    };
    match on {
        Some(true) => check(
            id,
            Ok,
            CheckReason::Ready,
            if o.apple_silicon {
                "Apple silicon"
            } else {
                "Turned on"
            },
            vec![],
        ),
        Some(false) => check(
            id,
            Blocked,
            CheckReason::VirtualizationOff,
            "Virtualization is turned off in the firmware (BIOS/UEFI)",
            vec![link(links::VIRTUALIZATION)],
        ),
        None => check(
            id,
            Unknown,
            CheckReason::NotChecked,
            "Nandi couldn't tell whether virtualization is on",
            vec![link(links::VIRTUALIZATION)],
        ),
    }
}

/// `40989736960` → `38.2 GB`
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

fn disk(o: &Observations) -> MachineCheck {
    use CheckState::*;
    let id = CheckId::Disk;
    let free = match o.os {
        MachineOs::Windows => o
            .windows_probe
            .as_ref()
            .and_then(Cmd::stdout)
            .and_then(|out| parse::windows_free_bytes(&parse::key_values(out))),
        _ => {
            o.df.as_ref()
                .and_then(Cmd::stdout)
                .and_then(parse::df_free_bytes)
        }
    };
    match free {
        Some(bytes) if bytes >= DISK_NEEDED_BYTES => check(
            id,
            Ok,
            CheckReason::Ready,
            format!("{} free at {}", gigabytes(bytes), o.data_path),
            vec![],
        ),
        Some(bytes) => check(
            id,
            Blocked,
            CheckReason::LowDisk,
            format!(
                "{} free at {}; about 15 GB is needed",
                gigabytes(bytes),
                o.data_path
            ),
            vec![],
        ),
        None => check(
            id,
            Unknown,
            CheckReason::NotChecked,
            format!("Nandi couldn't read the free space at {}", o.data_path),
            vec![],
        ),
    }
}

fn images(o: &Observations) -> MachineCheck {
    use CheckState::*;
    let id = CheckId::Images;
    match &o.images {
        ImagesFinding::Downloaded => check(id, Ok, CheckReason::Downloaded, "Downloaded", vec![]),
        ImagesFinding::Reachable => {
            check(id, Ok, CheckReason::Reachable, "ghcr.io answers", vec![])
        }
        ImagesFinding::Unreachable(e) => check(
            id,
            Blocked,
            CheckReason::Unreachable,
            e.clone(),
            vec![link(links::PROXY)],
        ),
        ImagesFinding::Certificate(e) => check(
            id,
            Blocked,
            CheckReason::Certificate,
            e.clone(),
            vec![link(links::PROXY)],
        ),
        ImagesFinding::ProxyAuth => check(
            id,
            Blocked,
            CheckReason::ProxyAuth,
            "The proxy asked for a sign-in (407)",
            vec![link(links::PROXY)],
        ),
        ImagesFinding::Refused(status) => check(
            id,
            Blocked,
            CheckReason::Refused,
            format!("ghcr.io answered {status}"),
            vec![],
        ),
        ImagesFinding::NotChecked => {
            check(id, Unknown, CheckReason::NotChecked, "Not checked", vec![])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN_PROBE_OK: &str =
        "HypervisorPresent=True\nFirmwareEnabled=False\nFreeBytes=40989736960\n";
    const WSL_OK: &str = "WSL version: 3.0.5.0\nKernel version: 6.18.58.1-1\n";
    const WSL_OLD: &str = "WSL version: 2.0.9.0\nKernel version: 5.15.133.1-1\n";
    const WSL_NOT_INSTALLED: &str = "The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'.";
    const WIN_PIPE: &str = "error during connect: Get \"http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine/v1.47/version\": open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified.";

    fn windows() -> Observations {
        Observations {
            os: MachineOs::Windows,
            arch: "x86_64".into(),
            desktop_app: Some(r"C:\Program Files\Docker\Docker\Docker Desktop.exe".into()),
            docker_cli: true,
            cli_version: Some(Cmd::done(0, "Docker version 27.2.1, build 9e34c9b", "")),
            engine_version: Some("29.9.2".into()),
            wsl_version: Some(Cmd::done(0, WSL_OK, "")),
            wsl_status: Some(Cmd::done(0, "Default Version: 2", "")),
            windows_probe: Some(Cmd::done(0, WIN_PROBE_OK, "")),
            data_path: r"C:\Users\ada\AppData\Local".into(),
            images: ImagesFinding::Downloaded,
            ..Default::default()
        }
    }

    fn mac(apple_silicon: bool) -> Observations {
        Observations {
            os: MachineOs::Macos,
            arch: if apple_silicon { "aarch64" } else { "x86_64" }.into(),
            apple_silicon,
            desktop_app: Some("/Applications/Docker.app".into()),
            desktop_archs: Some(if apple_silicon { "arm64" } else { "x86_64" }.into()),
            docker_cli: true,
            engine_version: Some("29.9.2".into()),
            hv_support: Some(Cmd::done(0, "1\n", "")),
            data_path: "/Users/ada".into(),
            df: Some(Cmd::done(0, "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s5 971350180 500000000 400000000 56% /System/Volumes/Data\n", "")),
            images: ImagesFinding::Reachable,
            ..Default::default()
        }
    }

    fn linux() -> Observations {
        Observations {
            os: MachineOs::Linux,
            arch: "x86_64".into(),
            docker_cli: true,
            dockerd: true,
            system_socket: true,
            cli_version: Some(Cmd::done(0, "Docker version 27.3.1, build ce12230", "")),
            engine_version: Some("27.3.1".into()),
            data_path: "/var/lib/docker".into(),
            df: Some(Cmd::done(0, "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 490617784 301234567 164383217 65% /\n", "")),
            os_release: Some("ID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n".into()),
            images: ImagesFinding::Reachable,
            ..Default::default()
        }
    }

    fn get(report: &MachineReport, id: CheckId) -> MachineCheck {
        report
            .checks
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id:?}"))
    }

    fn state(o: &Observations, id: CheckId) -> (CheckState, CheckReason) {
        let c = get(&report(o, String::new()), id);
        (c.state, c.reason)
    }

    fn stopped_engine(o: &mut Observations, error: &str) {
        o.engine_version = None;
        o.docker_version = Some(Cmd::done(1, "", error));
    }

    #[test]
    fn windows_ready_lists_every_row_and_is_ready() {
        let report = report(&windows(), "t".into());
        let ids: Vec<CheckId> = report.checks.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                CheckId::DockerInstalled,
                CheckId::DockerRunning,
                CheckId::Wsl,
                CheckId::Virtualization,
                CheckId::Disk,
                CheckId::Images
            ]
        );
        assert!(report.ready && report.docker_ready, "{report:#?}");
        assert!(report.checks.iter().all(|c| c.actions.is_empty()));
        assert_eq!(
            get(&report, CheckId::Disk).detail,
            r"38.2 GB free at C:\Users\ada\AppData\Local"
        );
        assert_eq!(get(&report, CheckId::Wsl).detail, "WSL 3.0.5.0");
    }

    #[test]
    fn windows_docker_missing_links_to_the_download() {
        let mut o = windows();
        o.desktop_app = None;
        o.docker_cli = false;
        o.cli_version = Some(Cmd::NotFound);
        stopped_engine(&mut o, "");
        let report = report(&o, String::new());
        let installed = get(&report, CheckId::DockerInstalled);
        assert_eq!(
            (installed.state, installed.reason),
            (CheckState::Missing, CheckReason::NotInstalled)
        );
        assert_eq!(
            installed.actions,
            [
                MachineAction::OpenLink {
                    url: links::WINDOWS_INSTALL.into()
                },
                MachineAction::CheckAgain
            ]
        );
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Missing, CheckReason::NeedsDocker)
        );
        assert!(!report.docker_ready);
    }

    #[test]
    fn windows_docker_stopped_offers_start_docker() {
        let mut o = windows();
        stopped_engine(&mut o, WIN_PIPE);
        o.desktop_status = Some(Cmd::done(0, "Name   Value\nStatus stopped\n", ""));
        let c = get(&report(&o, String::new()), CheckId::DockerRunning);
        assert_eq!(
            (c.state, c.reason),
            (CheckState::Stopped, CheckReason::NotRunning)
        );
        assert_eq!(
            c.actions,
            [MachineAction::StartDocker, MachineAction::CheckAgain]
        );

        // No status command (older Docker Desktop) and the app closed
        o.desktop_status = Some(Cmd::done(
            1,
            "",
            "docker: 'desktop' is not a docker command.",
        ));
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Stopped, CheckReason::NotRunning)
        );
    }

    #[test]
    fn windows_unable_to_start_is_a_failed_engine() {
        for error in ["Docker Desktop is unable to start", "request returned 500 Internal Server Error for API route and version http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine/v1.47/version, check if the server supports the requested API version"] {
            let mut o = windows();
            stopped_engine(&mut o, error);
            o.desktop_process = true;
            let c = get(&report(&o, String::new()), CheckId::DockerRunning);
            assert_eq!((c.state, c.reason), (CheckState::Blocked, CheckReason::DesktopFailed), "{error}");
            assert_eq!(c.actions[0], MachineAction::StartDocker);
        }
        // The app says it's running and the engine doesn't answer
        let mut o = windows();
        stopped_engine(&mut o, WIN_PIPE);
        o.desktop_process = true;
        o.desktop_status = Some(Cmd::done(0, "Status   running\n", ""));
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Blocked, CheckReason::DesktopFailed)
        );
    }

    #[test]
    fn windows_docker_starting() {
        let mut o = windows();
        stopped_engine(&mut o, WIN_PIPE);
        o.desktop_process = true;
        o.desktop_status = Some(Cmd::done(0, "Status   starting\n", ""));
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Stopped, CheckReason::Starting)
        );
        o.desktop_status = None;
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Stopped, CheckReason::Starting)
        );
    }

    #[test]
    fn windows_wsl_states() {
        let mut o = windows();
        o.wsl_version = Some(Cmd::done(1, WSL_NOT_INSTALLED, ""));
        o.wsl_status = Some(Cmd::done(1, WSL_NOT_INSTALLED, ""));
        assert_eq!(
            state(&o, CheckId::Wsl),
            (CheckState::Missing, CheckReason::NotInstalled)
        );
        o.wsl_version = Some(Cmd::done(0, WSL_OLD, ""));
        assert_eq!(
            state(&o, CheckId::Wsl),
            (CheckState::NeedsUpdate, CheckReason::OldWsl)
        );
        o.wsl_version = Some(Cmd::done(-1, "Invalid command line option: --version", ""));
        o.wsl_status = Some(Cmd::done(0, "Default Version: 2", ""));
        assert_eq!(
            state(&o, CheckId::Wsl),
            (CheckState::NeedsUpdate, CheckReason::OldWsl)
        );
        o.wsl_status = Some(Cmd::done(
            1,
            "WSL 2 requires an update to its kernel component.",
            "",
        ));
        assert_eq!(
            state(&o, CheckId::Wsl),
            (CheckState::NeedsUpdate, CheckReason::KernelUpdate)
        );
        o.wsl_version = Some(Cmd::NotFound);
        o.wsl_status = Some(Cmd::NotFound);
        assert_eq!(
            state(&o, CheckId::Wsl),
            (CheckState::Missing, CheckReason::NotInstalled)
        );
        o.wsl_version = Some(Cmd::TimedOut);
        o.wsl_status = Some(Cmd::TimedOut);
        assert_eq!(
            state(&o, CheckId::Wsl),
            (CheckState::Unknown, CheckReason::NotChecked)
        );
    }

    #[test]
    fn windows_virtualization_and_disk() {
        let mut o = windows();
        o.windows_probe = Some(Cmd::done(
            0,
            "HypervisorPresent=False\nFirmwareEnabled=False\nFreeBytes=5368709120\n",
            "",
        ));
        assert_eq!(
            state(&o, CheckId::Virtualization),
            (CheckState::Blocked, CheckReason::VirtualizationOff)
        );
        let disk = get(&report(&o, String::new()), CheckId::Disk);
        assert_eq!(
            (disk.state, disk.reason),
            (CheckState::Blocked, CheckReason::LowDisk)
        );
        assert!(disk.detail.starts_with("5.0 GB free"), "{}", disk.detail);
        o.windows_probe = Some(Cmd::TimedOut);
        assert_eq!(
            state(&o, CheckId::Virtualization),
            (CheckState::Unknown, CheckReason::NotChecked)
        );
        o.systeminfo = Some(Cmd::done(0, "Hyper-V Requirements:      A hypervisor has been detected. Features required for Hyper-V will not be displayed.", ""));
        assert_eq!(
            state(&o, CheckId::Virtualization),
            (CheckState::Ok, CheckReason::Ready)
        );
        assert_eq!(
            state(&o, CheckId::Disk),
            (CheckState::Unknown, CheckReason::NotChecked)
        );
    }

    #[test]
    fn images_states() {
        let mut o = windows();
        for (finding, expected) in [
            (
                ImagesFinding::Reachable,
                (CheckState::Ok, CheckReason::Reachable),
            ),
            (
                ImagesFinding::Unreachable("timed out".into()),
                (CheckState::Blocked, CheckReason::Unreachable),
            ),
            (
                ImagesFinding::Certificate("invalid peer certificate".into()),
                (CheckState::Blocked, CheckReason::Certificate),
            ),
            (
                ImagesFinding::ProxyAuth,
                (CheckState::Blocked, CheckReason::ProxyAuth),
            ),
            (
                ImagesFinding::Refused(403),
                (CheckState::Blocked, CheckReason::Refused),
            ),
            (
                ImagesFinding::NotChecked,
                (CheckState::Unknown, CheckReason::NotChecked),
            ),
        ] {
            o.images = finding;
            assert_eq!(state(&o, CheckId::Images), expected);
        }
    }

    #[test]
    fn mac_rows_and_chip_builds() {
        let report = report(&mac(true), String::new());
        assert!(report.ready, "{report:#?}");
        assert!(report.checks.iter().all(|c| c.id != CheckId::Wsl));
        assert_eq!(
            get(&report, CheckId::Virtualization).detail,
            "Apple silicon"
        );

        let mut intel_build = mac(true);
        intel_build.desktop_archs = Some("x86_64".into());
        let c = get(
            &super::report(&intel_build, String::new()),
            CheckId::DockerInstalled,
        );
        assert_eq!(
            (c.state, c.reason),
            (CheckState::NeedsUpdate, CheckReason::WrongBuild)
        );
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: links::MAC_ARM_DMG.into()
            }
        );

        let mut missing = mac(false);
        missing.desktop_app = None;
        missing.docker_cli = false;
        missing.engine_version = None;
        let c = get(
            &super::report(&missing, String::new()),
            CheckId::DockerInstalled,
        );
        assert_eq!(c.state, CheckState::Missing);
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: links::MAC_INTEL_DMG.into()
            }
        );

        let mut no_vtx = mac(false);
        no_vtx.hv_support = Some(Cmd::done(0, "0\n", ""));
        assert_eq!(
            state(&no_vtx, CheckId::Virtualization),
            (CheckState::Blocked, CheckReason::VirtualizationOff)
        );
    }

    #[test]
    fn mac_stopped_and_unable_to_start() {
        let mut o = mac(true);
        stopped_engine(&mut o, "Cannot connect to the Docker daemon at unix:///Users/ada/.docker/run/docker.sock. Is the docker daemon running?");
        let c = get(&report(&o, String::new()), CheckId::DockerRunning);
        assert_eq!(
            (c.state, c.reason),
            (CheckState::Stopped, CheckReason::NotRunning)
        );
        assert_eq!(c.actions[0], MachineAction::StartDocker);
        stopped_engine(&mut o, "Docker Desktop is unable to start");
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Blocked, CheckReason::DesktopFailed)
        );
    }

    #[test]
    fn linux_running_has_no_wsl_or_virtualization_rows() {
        let report = report(&linux(), String::new());
        assert!(report.ready, "{report:#?}");
        let ids: Vec<CheckId> = report.checks.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                CheckId::DockerInstalled,
                CheckId::DockerRunning,
                CheckId::Disk,
                CheckId::Images
            ]
        );
        let distro = report.distro.unwrap();
        assert_eq!(
            (distro.id.as_str(), distro.family),
            ("ubuntu", DistroFamily::Debian)
        );
    }

    #[test]
    fn linux_not_installed_links_to_the_distro_page() {
        let mut o = linux();
        o.docker_cli = false;
        o.dockerd = false;
        o.system_socket = false;
        o.cli_version = Some(Cmd::NotFound);
        o.engine_version = None;
        let c = get(&report(&o, String::new()), CheckId::DockerInstalled);
        assert_eq!(
            (c.state, c.reason),
            (CheckState::Missing, CheckReason::NotInstalled)
        );
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: "https://docs.docker.com/engine/install/ubuntu/".into()
            }
        );
        o.os_release = Some("ID=fedora\n".into());
        let c = get(&report(&o, String::new()), CheckId::DockerInstalled);
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: "https://docs.docker.com/engine/install/fedora/".into()
            }
        );
        o.os_release = Some("ID=endeavouros\nID_LIKE=arch\n".into());
        let c = get(&report(&o, String::new()), CheckId::DockerInstalled);
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: links::ARCH_WIKI.into()
            }
        );
        o.os_release = Some("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n".into());
        let c = get(&report(&o, String::new()), CheckId::DockerInstalled);
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: "https://docs.docker.com/engine/install/ubuntu/".into()
            }
        );
    }

    #[test]
    fn linux_stopped_service() {
        let mut o = linux();
        stopped_engine(&mut o, "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?");
        o.systemctl = Some(Cmd::done(3, "inactive\n", ""));
        let c = get(&report(&o, String::new()), CheckId::DockerRunning);
        assert_eq!(
            (c.state, c.reason),
            (CheckState::Stopped, CheckReason::NotRunning)
        );
        assert_eq!(c.detail, "docker.service is inactive");
        // Docker Engine is started with sudo, shown as a command, never run
        assert_eq!(c.actions, [MachineAction::CheckAgain]);
        o.systemctl = Some(Cmd::done(3, "failed\n", ""));
        assert_eq!(
            get(&report(&o, String::new()), CheckId::DockerRunning).detail,
            "docker.service is failed"
        );
        o.systemctl = Some(Cmd::done(0, "activating\n", ""));
        assert_eq!(
            state(&o, CheckId::DockerRunning),
            (CheckState::Stopped, CheckReason::Starting)
        );
    }

    #[test]
    fn linux_socket_permission_denied_is_blocked() {
        let mut o = linux();
        stopped_engine(&mut o, "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/version\": dial unix /var/run/docker.sock: connect: permission denied");
        o.systemctl = Some(Cmd::done(0, "active\n", ""));
        let c = get(&report(&o, String::new()), CheckId::DockerRunning);
        assert_eq!(
            (c.state, c.reason),
            (CheckState::Blocked, CheckReason::SocketPermission)
        );
        assert_eq!(
            c.actions[0],
            MachineAction::OpenLink {
                url: links::LINUX_POSTINSTALL.into()
            }
        );
    }

    #[test]
    fn linux_podman_and_rootless_are_noted_not_claimed() {
        let mut o = linux();
        o.cli_version = Some(Cmd::done(0, "podman version 5.2.2", ""));
        assert_eq!(
            state(&o, CheckId::DockerInstalled),
            (CheckState::Unknown, CheckReason::Podman)
        );
        let mut o = linux();
        o.system_socket = false;
        o.rootless_socket = true;
        o.engine_version = None;
        o.docker_version = Some(Cmd::done(1, "", "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?"));
        assert_eq!(
            state(&o, CheckId::DockerInstalled),
            (CheckState::Unknown, CheckReason::Rootless)
        );
    }

    #[test]
    fn linux_desktop_offers_start_docker() {
        let mut o = linux();
        o.desktop_app = Some("/opt/docker-desktop".into());
        stopped_engine(&mut o, "Cannot connect to the Docker daemon at unix:///home/ada/.docker/desktop/docker.sock. Is the docker daemon running?");
        let c = get(&report(&o, String::new()), CheckId::DockerRunning);
        assert_eq!(
            (c.state, c.reason),
            (CheckState::Stopped, CheckReason::NotRunning)
        );
        assert_eq!(c.actions[0], MachineAction::StartDocker);
    }

    #[test]
    fn report_serializes_with_camel_case_and_tagged_actions() {
        let mut o = windows();
        stopped_engine(&mut o, "Docker Desktop is unable to start");
        let json = serde_json::to_value(report(&o, "now".into())).unwrap();
        assert_eq!(json["dockerReady"], false);
        assert_eq!(json["checks"][1]["reason"], "desktopFailed");
        assert_eq!(json["checks"][1]["actions"][0]["kind"], "startDocker");
        assert_eq!(json["checks"][1]["actions"][1]["kind"], "openLink");
        assert_eq!(json["checks"][1]["actions"][1]["url"], links::TROUBLESHOOT);
    }
}
