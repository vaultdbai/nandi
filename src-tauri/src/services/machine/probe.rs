//! Runs the read-only probes. Every command has fixed arguments, runs
//! without a shell or a window, never elevated, and gives up after a short
//! timeout. Parsing is in `parse` and `assess`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use bollard::Docker;

use super::assess::{Cmd, ImagesFinding, Observations};
use super::parse;
use super::MachineOs;
use crate::services::images::{registry, SandboxImage};

const QUICK: Duration = Duration::from_secs(5);
const SLOW: Duration = Duration::from_secs(15);
/// The engine ping
const PING: Duration = Duration::from_secs(3);
/// The registry check
const REGISTRY: Duration = Duration::from_secs(5);

/// Windows: virtualization and free space in one PowerShell process,
/// printed as untranslated `key=value` lines. Fixed text, no user input.
const WINDOWS_PROBE: &str = "$ErrorActionPreference='SilentlyContinue'; \
$c = Get-CimInstance Win32_ComputerSystem; \
$p = Get-CimInstance Win32_Processor | Select-Object -First 1; \
\"HypervisorPresent=$($c.HypervisorPresent)\"; \
\"FirmwareEnabled=$($p.VirtualizationFirmwareEnabled)\"; \
\"FreeBytes=$(([System.IO.DriveInfo]::new($env:LOCALAPPDATA)).AvailableFreeSpace)\"";

/// Runs `program args` and decodes what it prints
pub(super) async fn run(program: &str, args: &[&str], timeout: Duration) -> Cmd {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Cmd::NotFound,
        Err(e) => return Cmd::done(-1, "", &e.to_string()),
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => Cmd::Done {
            code: output.status.code().unwrap_or(-1),
            stdout: parse::decode_output(&output.stdout),
            stderr: parse::decode_output(&output.stderr),
        },
        Ok(Err(e)) => Cmd::done(-1, "", &e.to_string()),
        Err(_) => Cmd::TimedOut,
    }
}

/// What's asked every time: is Docker there, and does it answer
#[derive(Debug, Clone, Default)]
pub(super) struct Fast {
    pub desktop_app: Option<String>,
    pub docker_cli: bool,
    pub cli_version: Option<Cmd>,
    pub dockerd: bool,
    pub system_socket: bool,
    pub rootless_socket: bool,
    pub engine_version: Option<String>,
    pub docker_version: Option<Cmd>,
    pub desktop_status: Option<Cmd>,
    pub desktop_process: bool,
    pub systemctl: Option<Cmd>,
    pub os_release: Option<String>,
}

/// What changes rarely and costs more: reused for a minute
#[derive(Debug, Clone, Default)]
pub(super) struct Slow {
    pub apple_silicon: bool,
    pub desktop_archs: Option<String>,
    pub wsl_version: Option<Cmd>,
    pub wsl_status: Option<Cmd>,
    pub windows_probe: Option<Cmd>,
    pub systeminfo: Option<Cmd>,
    pub hv_support: Option<Cmd>,
    pub df: Option<Cmd>,
    pub data_path: String,
    pub images: ImagesFinding,
}

pub(super) fn combine(os: MachineOs, fast: Fast, slow: Slow) -> Observations {
    Observations {
        os,
        arch: std::env::consts::ARCH.to_string(),
        apple_silicon: slow.apple_silicon,
        desktop_app: fast.desktop_app,
        desktop_archs: slow.desktop_archs,
        docker_cli: fast.docker_cli,
        cli_version: fast.cli_version,
        dockerd: fast.dockerd,
        system_socket: fast.system_socket,
        rootless_socket: fast.rootless_socket,
        engine_version: fast.engine_version,
        docker_version: fast.docker_version,
        desktop_status: fast.desktop_status,
        desktop_process: fast.desktop_process,
        systemctl: fast.systemctl,
        wsl_version: slow.wsl_version,
        wsl_status: slow.wsl_status,
        windows_probe: slow.windows_probe,
        systeminfo: slow.systeminfo,
        hv_support: slow.hv_support,
        data_path: slow.data_path,
        df: slow.df,
        os_release: fast.os_release,
        images: slow.images,
    }
}

/// Where Docker Desktop is installed, if it is
async fn desktop_app(os: MachineOs) -> Option<PathBuf> {
    match os {
        MachineOs::Windows => {
            let mut candidates = Vec::new();
            if let Cmd::Done { stdout, .. } = run(
                "reg",
                &[
                    "query",
                    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Docker Desktop",
                    "/v",
                    "InstallLocation",
                ],
                QUICK,
            )
            .await
            {
                if let Some(dir) = parse::reg_install_location(&stdout) {
                    candidates.push(PathBuf::from(dir));
                }
            }
            for var in ["ProgramFiles", "LOCALAPPDATA"] {
                if let Ok(root) = std::env::var(var) {
                    let base = PathBuf::from(root);
                    candidates.push(base.join("Docker").join("Docker"));
                    candidates.push(base.join("Programs").join("Docker").join("Docker"));
                }
            }
            candidates
                .into_iter()
                .map(|dir| dir.join("Docker Desktop.exe"))
                .find(|exe| exe.is_file())
        }
        MachineOs::Macos => {
            let mut candidates = vec![PathBuf::from("/Applications/Docker.app")];
            if let Some(home) = dirs::home_dir() {
                candidates.push(home.join("Applications").join("Docker.app"));
            }
            candidates.into_iter().find(|app| app.is_dir())
        }
        MachineOs::Linux => Some(PathBuf::from("/opt/docker-desktop")).filter(|dir| dir.is_dir()),
        MachineOs::Other => None,
    }
}

/// The engine's version, when it answers in time
async fn engine_version() -> Option<String> {
    let docker = Docker::connect_with_local_defaults().ok()?;
    let version = tokio::time::timeout(PING, docker.version())
        .await
        .ok()?
        .ok()?;
    Some(
        version
            .version
            .unwrap_or_else(|| "(version unknown)".to_string()),
    )
}

pub(super) async fn fast(os: MachineOs) -> Fast {
    let (desktop, engine) = tokio::join!(desktop_app(os), engine_version());
    let mut fast = Fast {
        desktop_app: desktop.map(|p| p.display().to_string()),
        docker_cli: which::which("docker").is_ok(),
        engine_version: engine,
        ..Default::default()
    };
    if os == MachineOs::Linux {
        fast.cli_version = Some(run("docker", &["--version"], QUICK).await);
        fast.dockerd = which::which("dockerd").is_ok();
        fast.system_socket = Path::new("/var/run/docker.sock").exists();
        fast.rootless_socket = std::env::var("XDG_RUNTIME_DIR")
            .is_ok_and(|dir| Path::new(&dir).join("docker.sock").exists());
        fast.os_release = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .ok();
    }
    if fast.engine_version.is_some() {
        return fast;
    }
    // Docker doesn't answer: ask why
    if fast.docker_cli {
        fast.docker_version = Some(
            run(
                "docker",
                &["version", "--format", "{{.Server.Version}}"],
                QUICK,
            )
            .await,
        );
    }
    match os {
        MachineOs::Windows | MachineOs::Macos if fast.desktop_app.is_some() => {
            if fast.docker_cli {
                fast.desktop_status = Some(run("docker", &["desktop", "status"], QUICK).await);
            }
            fast.desktop_process = desktop_process(os).await;
        }
        MachineOs::Linux => {
            fast.systemctl = Some(run("systemctl", &["is-active", "docker"], QUICK).await);
        }
        _ => {}
    }
    fast
}

async fn desktop_process(os: MachineOs) -> bool {
    match os {
        MachineOs::Windows => match run(
            "tasklist",
            &[
                "/FI",
                "IMAGENAME eq Docker Desktop.exe",
                "/FO",
                "CSV",
                "/NH",
            ],
            QUICK,
        )
        .await
        {
            Cmd::Done { stdout, .. } => parse::tasklist_has(&stdout, "Docker Desktop.exe"),
            _ => false,
        },
        MachineOs::Macos => matches!(
            run("pgrep", &["-f", "Docker.app/Contents/MacOS"], QUICK).await,
            Cmd::Done { code: 0, .. }
        ),
        _ => false,
    }
}

fn data_path(os: MachineOs) -> PathBuf {
    match os {
        MachineOs::Windows => std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(r"C:\")),
        MachineOs::Linux => ["/var/lib/docker", "/var/lib", "/"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("/")),
        _ => dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
    }
}

pub(super) async fn slow(os: MachineOs, engine_answers: bool) -> Slow {
    let path = data_path(os);
    let mut slow = Slow {
        data_path: path.display().to_string(),
        ..Default::default()
    };
    let images = images(engine_answers);
    match os {
        MachineOs::Windows => {
            let (version, status, probe, images) = tokio::join!(
                run("wsl", &["--version"], SLOW),
                run("wsl", &["--status"], SLOW),
                run(
                    "powershell",
                    &["-NoProfile", "-NonInteractive", "-Command", WINDOWS_PROBE],
                    SLOW
                ),
                images
            );
            let known = matches!(&probe, Cmd::Done { stdout, .. }
                if parse::windows_virtualization(&parse::key_values(stdout)).is_some());
            if !known {
                slow.systeminfo = Some(run("systeminfo", &[], SLOW).await);
            }
            slow.wsl_version = Some(version);
            slow.wsl_status = Some(status);
            slow.windows_probe = Some(probe);
            slow.images = images;
        }
        _ => {
            let path_arg = slow.data_path.clone();
            let df_args = ["-Pk", path_arg.as_str()];
            let (df, images) = tokio::join!(run("df", &df_args, QUICK), images);
            slow.df = Some(df);
            slow.images = images;
            if os == MachineOs::Macos {
                slow.apple_silicon = std::env::consts::ARCH == "aarch64"
                    || matches!(
                        run("sysctl", &["-n", "hw.optional.arm64"], QUICK).await,
                        Cmd::Done { ref stdout, .. } if stdout.trim() == "1"
                    );
                if !slow.apple_silicon {
                    slow.hv_support = Some(run("sysctl", &["-n", "kern.hv_support"], QUICK).await);
                }
                slow.desktop_archs = mac_desktop_archs().await;
            }
        }
    }
    slow
}

/// `lipo -archs` of Docker Desktop's binary: `arm64`, `x86_64`, or both
async fn mac_desktop_archs() -> Option<String> {
    let app = Path::new("/Applications/Docker.app/Contents/MacOS");
    for binary in ["com.docker.backend", "Docker Desktop", "Docker"] {
        let path = app.join(binary);
        if !path.is_file() {
            continue;
        }
        if let Cmd::Done {
            code: 0, stdout, ..
        } = run("lipo", &["-archs", &path.display().to_string()], QUICK).await
        {
            return Some(stdout.trim().to_string());
        }
    }
    None
}

static REGISTRY_OVERRIDE: LazyLock<Mutex<Option<String>>> = LazyLock::new(Mutex::default);

/// Tests: the image check asks this base URL instead of the registry (the
/// offline suite points it where nothing listens)
#[cfg(test)]
pub(crate) fn use_registry_for_tests(base: &str) {
    if let Ok(mut guard) = REGISTRY_OVERRIDE.lock() {
        *guard = Some(base.to_string());
    }
}

/// Nandi's runtime image: here already, else is its manifest reachable
async fn images(engine_answers: bool) -> ImagesFinding {
    let image = SandboxImage::Runtime.tag();
    if engine_answers {
        if let Ok(docker) = Docker::connect_with_local_defaults() {
            if let Ok(Ok(_)) = tokio::time::timeout(PING, docker.inspect_image(image)).await {
                return ImagesFinding::Downloaded;
            }
        }
    }
    match tokio::time::timeout(REGISTRY, reach(image)).await {
        Ok(finding) => finding,
        Err(_) => ImagesFinding::Unreachable("No answer from the registry in time".to_string()),
    }
}

/// A manifest HEAD, anonymous as any public pull (the registry's
/// challenge names the token to get). Honours `HTTPS_PROXY`/`HTTP_PROXY`.
async fn reach(image: &str) -> ImagesFinding {
    let Some(reference) = registry::Reference::parse(image) else {
        return ImagesFinding::NotChecked;
    };
    let base = REGISTRY_OVERRIDE
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| format!("https://{}", reference.registry));
    let url = format!(
        "{base}/v2/{}/manifests/{}",
        reference.repository, reference.tag
    );
    let client = match reqwest::Client::builder().timeout(REGISTRY).build() {
        Ok(client) => client,
        Err(e) => return ImagesFinding::Unreachable(e.to_string()),
    };
    let head = |token: Option<String>| {
        let mut request = client.head(&url).header("Accept", registry::ACCEPT);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send()
    };
    let mut response = match head(None).await {
        Ok(response) => response,
        Err(e) => return request_error(&e),
    };
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let token = match registry::pull_token(&client, &challenge, &reference.repository).await {
            Ok(token) => token,
            Err(e) => return ImagesFinding::Unreachable(format!("No pull token: {e}")),
        };
        response = match head(Some(token)).await {
            Ok(response) => response,
            Err(e) => return request_error(&e),
        };
    }
    let status = response.status();
    if status.is_success() {
        ImagesFinding::Reachable
    } else if status == reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED {
        ImagesFinding::ProxyAuth
    } else {
        ImagesFinding::Refused(status.as_u16())
    }
}

fn request_error(error: &reqwest::Error) -> ImagesFinding {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        text = format!("{text}: {inner}");
        source = inner.source();
    }
    if text.to_lowercase().contains("certificate") {
        ImagesFinding::Certificate(text)
    } else {
        ImagesFinding::Unreachable(text)
    }
}

pub(super) async fn start_docker(os: MachineOs) -> Result<(), String> {
    let app = desktop_app(os).await;
    match (os, app) {
        (MachineOs::Windows, Some(exe)) => {
            // A GUI app: started and left running on its own
            std::process::Command::new(&exe)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Docker Desktop didn't start: {e}"))
        }
        (MachineOs::Macos, Some(_)) => match run("open", &["-a", "Docker"], QUICK).await {
            Cmd::Done { code: 0, .. } => Ok(()),
            other => Err(format!("Docker Desktop didn't start: {other:?}")),
        },
        (MachineOs::Linux, Some(_)) => {
            match run("systemctl", &["--user", "start", "docker-desktop"], SLOW).await {
                Cmd::Done { code: 0, .. } => Ok(()),
                Cmd::Done { stderr, .. } => {
                    Err(format!("Docker Desktop didn't start: {}", stderr.trim()))
                }
                other => Err(format!("Docker Desktop didn't start: {other:?}")),
            }
        }
        (MachineOs::Linux, None) => {
            Err("Docker Engine starts from a terminal: sudo systemctl start docker".to_string())
        }
        _ => Err("Docker Desktop isn't installed".to_string()),
    }
}

#[cfg(test)]
mod tests {
    /// `cargo test --lib live_registry_reach -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_registry_reach() {
        let finding = super::reach(super::SandboxImage::Runtime.tag()).await;
        println!("{finding:?}");
        assert_eq!(finding, super::ImagesFinding::Reachable);
    }

    #[tokio::test]
    async fn an_unreachable_registry_says_so_quickly() {
        super::use_registry_for_tests("http://127.0.0.1:9");
        let started = std::time::Instant::now();
        let finding = super::images(false).await;
        assert!(
            matches!(finding, super::ImagesFinding::Unreachable(_)),
            "{finding:?}"
        );
        assert!(started.elapsed() < super::REGISTRY + std::time::Duration::from_secs(1));
    }
}
