//! Pure parsing of what this computer's commands print: no I/O, so every
//! state has a fixture test. Labels are often translated (`wsl --version`
//! says "Version WSL" in French), so the parsers lean on what doesn't
//! change: version numbers, command names, `key=value` lines Nandi asked
//! for, exit codes.

/// Command output as text. `wsl.exe` writes UTF-16LE (with or without a
/// BOM) when its output isn't a console; everything else is UTF-8.
pub fn decode_output(bytes: &[u8]) -> String {
    let utf16 = bytes.starts_with(&[0xFF, 0xFE])
        || (bytes.len() >= 4
            && bytes.len().is_multiple_of(2)
            && bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 2 >= bytes.len() / 2);
    let text = if utf16 {
        let body = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.replace('\0', "").replace("\r\n", "\n")
}

/// `2.3.26.0` as numbers; anything after the first non-digit part is ignored
pub fn version_numbers(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map_while(|part| part.parse::<u32>().ok())
        .collect()
}

/// `a >= b`, comparing part by part (missing parts count as 0)
pub fn at_least(a: &[u32], b: &[u32]) -> bool {
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    true
}

/// The first dotted version (`2.3.26.0`, at least `x.y`) in `line`
fn first_version(line: &str) -> Option<String> {
    line.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map(|token| token.trim_matches('.'))
        .find(|token| token.split('.').filter(|p| !p.is_empty()).count() >= 2)
        .map(str::to_string)
}

/// The WSL version from `wsl --version`: the first line naming WSL (its
/// label is translated, "WSL" isn't), else the first version printed
pub fn wsl_version(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.contains("WSL") && !line.contains("WSLg"))
        .and_then(first_version)
        .or_else(|| output.lines().find_map(first_version))
}

/// What `wsl --status` / `wsl --version` say about WSL itself
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WslFinding {
    /// The store WSL, at this version
    Version,
    /// WSL isn't installed (the message offers `wsl --install`)
    NotInstalled,
    /// WSL 2's kernel needs an update
    KernelUpdate,
    /// The WSL built into Windows: works, but too old for Docker
    Inbox,
    Unknown,
}

/// The minimum WSL Docker Desktop asks for
pub const MIN_WSL: &[u32] = &[2, 1, 5];

/// Classifies WSL from `wsl --version` (exit code, text) and `wsl --status`
pub fn wsl_finding(version: Option<(i32, &str)>, status: Option<(i32, &str)>) -> WslFinding {
    if let Some((0, text)) = version {
        if wsl_version(text).is_some() {
            return WslFinding::Version;
        }
    }
    let texts: Vec<(i32, &str)> = [version, status].into_iter().flatten().collect();
    let lower = |t: &str| t.to_lowercase();
    if texts
        .iter()
        .any(|(_, t)| lower(t).contains("kernel") && lower(t).contains("update"))
    {
        return WslFinding::KernelUpdate;
    }
    if texts
        .iter()
        .any(|(code, t)| *code != 0 && t.contains("--install"))
    {
        return WslFinding::NotInstalled;
    }
    // `--status` works and `--version` doesn't: the inbox WSL
    if matches!(status, Some((0, _))) {
        return WslFinding::Inbox;
    }
    WslFinding::Unknown
}

/// `key=value` lines (the PowerShell probe prints its own, untranslated)
pub fn key_values(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

fn truthy(value: Option<&str>) -> Option<bool> {
    match value?.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// Virtualization on Windows, from the probe's `HypervisorPresent` and
/// `FirmwareEnabled` (the same facts as systeminfo's "Hyper-V
/// Requirements"). A running hypervisor hides the firmware flag, so it
/// counts as on. `None` when neither is known.
pub fn windows_virtualization(pairs: &[(String, String)]) -> Option<bool> {
    match (
        truthy(value(pairs, "HypervisorPresent")),
        truthy(value(pairs, "FirmwareEnabled")),
    ) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (_, Some(false)) => Some(false),
        _ => None,
    }
}

/// Free bytes from the probe's `FreeBytes=`
pub fn windows_free_bytes(pairs: &[(String, String)]) -> Option<u64> {
    value(pairs, "FreeBytes")?.parse().ok()
}

/// The English `systeminfo` fallback: "Virtualization Enabled In
/// Firmware: Yes/No", or "A hypervisor has been detected"
pub fn systeminfo_virtualization(output: &str) -> Option<bool> {
    if output.contains("A hypervisor has been detected") {
        return Some(true);
    }
    let line = output
        .lines()
        .find(|line| line.contains("Virtualization Enabled In Firmware"))?;
    let answer = line.rsplit(':').next()?.trim();
    truthy(Some(answer))
}

/// Free bytes from `df -Pk <path>` (POSIX format: the 4th column of the
/// last line, in KiB)
pub fn df_free_bytes(output: &str) -> Option<u64> {
    let line = output.lines().rev().find(|l| !l.trim().is_empty())?;
    let kib: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(kib * 1024)
}

/// `InstallLocation` from `reg query ... /v InstallLocation`
pub fn reg_install_location(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("InstallLocation")?;
        let path = rest.trim_start().strip_prefix("REG_SZ")?.trim();
        (!path.is_empty()).then(|| path.to_string())
    })
}

/// The `Status` value from `docker desktop status` (`running`, `stopped`,
/// `starting`, ...), lower-cased
pub fn desktop_status(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        (parts.next()? == "Status").then(|| parts.next().map(str::to_ascii_lowercase))?
    })
}

/// Whether `tasklist /FO CSV /NH` listed `image`
pub fn tasklist_has(output: &str, image: &str) -> bool {
    let quoted = format!("\"{}\"", image.to_ascii_lowercase());
    output
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with(&quoted))
}

/// Why the Docker CLI couldn't reach the engine, from `docker version`'s
/// error output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerError {
    /// Linux: the socket is there, the user may not use it
    SocketPermission,
    /// Docker Desktop answers, its engine failed ("unable to start", 500)
    DesktopFailed,
    /// Nothing listens: not started (or still starting)
    NotRunning,
    /// The CLI says it's Podman
    Podman,
    Other,
}

pub fn docker_error(stderr: &str) -> DockerError {
    let lower = stderr.to_lowercase();
    if lower.contains("permission denied") && lower.contains("docker.sock") {
        DockerError::SocketPermission
    } else if lower.contains("unable to start") || lower.contains("500 internal server error") {
        DockerError::DesktopFailed
    } else if lower.contains("podman") {
        DockerError::Podman
    } else if lower.contains("cannot connect to the docker daemon")
        || lower.contains("the system cannot find the file specified")
        || lower.contains("error during connect")
        || lower.contains("is the docker daemon running")
        || lower.contains("no such file or directory")
    {
        DockerError::NotRunning
    } else {
        DockerError::Other
    }
}

/// `systemctl is-active docker`: `active`, `inactive`, `failed`,
/// `activating`, ...
pub fn systemctl_state(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Linux distributions, grouped by how Docker installs on them
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DistroFamily {
    /// Debian, Ubuntu, Mint, Pop!_OS, ...
    Debian,
    /// Fedora, RHEL, CentOS, Rocky, Alma
    Fedora,
    /// Arch, Manjaro, EndeavourOS
    Arch,
    Other,
}

/// `/etc/os-release`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LinuxDistro {
    /// `ID`, e.g. `ubuntu`
    pub id: String,
    /// `ID_LIKE`, e.g. `["debian"]`
    pub id_like: Vec<String>,
    /// `VERSION_ID`, e.g. `24.04`
    pub version_id: Option<String>,
    /// `PRETTY_NAME`, e.g. `Ubuntu 24.04.1 LTS`
    pub name: Option<String>,
    pub family: DistroFamily,
}

pub fn os_release(text: &str) -> LinuxDistro {
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
        })
    };
    let id = field("ID").unwrap_or_default().to_ascii_lowercase();
    let id_like: Vec<String> = field("ID_LIKE")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let is = |names: &[&str]| {
        names.contains(&id.as_str()) || id_like.iter().any(|like| names.contains(&like.as_str()))
    };
    let family = if is(&["debian", "ubuntu"]) {
        DistroFamily::Debian
    } else if is(&["fedora", "rhel", "centos"]) {
        DistroFamily::Fedora
    } else if is(&["arch"]) {
        DistroFamily::Arch
    } else {
        DistroFamily::Other
    };
    LinuxDistro {
        id,
        id_like,
        version_id: field("VERSION_ID"),
        name: field("PRETTY_NAME"),
        family,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, bom: bool) -> Vec<u8> {
        let mut bytes = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn wsl_output_is_decoded_from_utf16_with_or_without_a_bom() {
        let text = "Default Distribution: Ubuntu\r\nDefault Version: 2\r\n";
        assert_eq!(
            decode_output(&utf16(text, false)),
            "Default Distribution: Ubuntu\nDefault Version: 2\n"
        );
        assert_eq!(
            decode_output(&utf16(text, true)),
            decode_output(&utf16(text, false))
        );
        assert_eq!(decode_output(b"plain\r\ntext"), "plain\ntext");
        assert_eq!(
            decode_output(&utf16("WSL バージョン: 2.0.9.0", false)),
            "WSL バージョン: 2.0.9.0"
        );
    }

    const WSL_VERSION_EN: &str = "WSL version: 3.0.5.0\nKernel version: 6.18.58.1-1\nWSLg version: 1.0.79\nWindows version: 10.0.26300.9818\n";
    const WSL_VERSION_FR: &str =
        "Version WSL : 2.0.9.0\nVersion du noyau : 5.15.133.1-1\nVersion WSLg : 1.0.59\n";
    const WSL_VERSION_JA: &str = "WSL バージョン: 1.2.5.0\nカーネル バージョン: 5.15.90.1\n";
    const WSL_NOT_INSTALLED: &str = "The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'.\nFor more information please visit https://aka.ms/wslinstall\n";
    const WSL_INBOX_HELP: &str = "Invalid command line option: --version\nUsage: wsl.exe [Argument] [Options...] [CommandLine]\n";
    const WSL_KERNEL: &str = "WSL 2 requires an update to its kernel component. For information please visit https://aka.ms/wsl2kernel\n";

    #[test]
    fn wsl_version_is_read_whatever_the_language() {
        assert_eq!(wsl_version(WSL_VERSION_EN).as_deref(), Some("3.0.5.0"));
        assert_eq!(wsl_version(WSL_VERSION_FR).as_deref(), Some("2.0.9.0"));
        assert_eq!(wsl_version(WSL_VERSION_JA).as_deref(), Some("1.2.5.0"));
        assert!(at_least(&version_numbers("3.0.5.0"), MIN_WSL));
        assert!(at_least(&version_numbers("2.1.5.0"), MIN_WSL));
        assert!(!at_least(&version_numbers("2.0.9.0"), MIN_WSL));
        assert!(!at_least(&version_numbers("1.2.5.0"), MIN_WSL));
    }

    #[test]
    fn wsl_states_from_fixtures() {
        assert_eq!(
            wsl_finding(Some((0, WSL_VERSION_EN)), Some((0, "Default Version: 2"))),
            WslFinding::Version
        );
        assert_eq!(
            wsl_finding(Some((1, WSL_NOT_INSTALLED)), Some((1, WSL_NOT_INSTALLED))),
            WslFinding::NotInstalled
        );
        assert_eq!(
            wsl_finding(Some((-1, WSL_INBOX_HELP)), Some((0, "Default Version: 2"))),
            WslFinding::Inbox
        );
        assert_eq!(
            wsl_finding(Some((-1, WSL_INBOX_HELP)), Some((1, WSL_KERNEL))),
            WslFinding::KernelUpdate
        );
        assert_eq!(wsl_finding(None, None), WslFinding::Unknown);
    }

    #[test]
    fn virtualization_and_disk_from_the_powershell_probe() {
        let on =
            key_values("HypervisorPresent=True\nFirmwareEnabled=False\nFreeBytes=40989736960\n");
        assert_eq!(windows_virtualization(&on), Some(true));
        assert_eq!(windows_free_bytes(&on), Some(40_989_736_960));
        let firmware = key_values("HypervisorPresent=False\nFirmwareEnabled=True\n");
        assert_eq!(windows_virtualization(&firmware), Some(true));
        let off = key_values("HypervisorPresent=False\nFirmwareEnabled=False\nFreeBytes=\n");
        assert_eq!(windows_virtualization(&off), Some(false));
        assert_eq!(windows_free_bytes(&off), None);
        assert_eq!(windows_virtualization(&key_values("garbage")), None);
    }

    #[test]
    fn systeminfo_fallback_reads_the_hyper_v_lines() {
        let hypervisor = "Hyper-V Requirements:      A hypervisor has been detected. Features required for Hyper-V will not be displayed.";
        assert_eq!(systeminfo_virtualization(hypervisor), Some(true));
        let off = "Hyper-V Requirements:      VM Monitor Mode Extensions: Yes\n                           Virtualization Enabled In Firmware: No\n";
        assert_eq!(systeminfo_virtualization(off), Some(false));
        assert_eq!(
            systeminfo_virtualization("Configuration requise pour Hyper-V"),
            None
        );
    }

    #[test]
    fn df_and_registry_and_tasklist() {
        let df = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/nvme0n1p2   490617784 301234567 164383217      65% /\n";
        assert_eq!(df_free_bytes(df), Some(164_383_217 * 1024));
        assert_eq!(df_free_bytes(""), None);
        let reg = "\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Docker Desktop\n    InstallLocation    REG_SZ    C:\\Program Files\\Docker\\Docker\n";
        assert_eq!(
            reg_install_location(reg).as_deref(),
            Some("C:\\Program Files\\Docker\\Docker")
        );
        assert_eq!(
            reg_install_location(
                "ERROR: The system was unable to find the specified registry key or value."
            ),
            None
        );
        let tasks = "\"Docker Desktop.exe\",\"30196\",\"Console\",\"1\",\"14,816 K\"\n";
        assert!(tasklist_has(tasks, "Docker Desktop.exe"));
        assert!(!tasklist_has(
            "INFO: No tasks are running which match the specified criteria.",
            "Docker Desktop.exe"
        ));
    }

    #[test]
    fn docker_desktop_status_lines() {
        let running = "Name                Value\nStatus              running\nSessionID           a04a3260\n";
        assert_eq!(desktop_status(running).as_deref(), Some("running"));
        assert_eq!(
            desktop_status("Status   starting").as_deref(),
            Some("starting")
        );
        assert_eq!(
            desktop_status("docker: 'desktop' is not a docker command."),
            None
        );
    }

    pub const WIN_NOT_RUNNING: &str = "error during connect: Get \"http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine/v1.47/version\": open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified.";
    pub const WIN_UNABLE_TO_START: &str = "Docker Desktop is unable to start";
    pub const WIN_500: &str = "request returned 500 Internal Server Error for API route and version http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine/v1.47/version, check if the server supports the requested API version";
    pub const MAC_NOT_RUNNING: &str = "Cannot connect to the Docker daemon at unix:///Users/ada/.docker/run/docker.sock. Is the docker daemon running?";
    pub const LINUX_PERMISSION: &str = "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/version\": dial unix /var/run/docker.sock: connect: permission denied";
    pub const LINUX_NOT_RUNNING: &str = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?";

    #[test]
    fn docker_errors_are_classified() {
        assert_eq!(docker_error(WIN_NOT_RUNNING), DockerError::NotRunning);
        assert_eq!(
            docker_error(WIN_UNABLE_TO_START),
            DockerError::DesktopFailed
        );
        assert_eq!(docker_error(WIN_500), DockerError::DesktopFailed);
        assert_eq!(docker_error(MAC_NOT_RUNNING), DockerError::NotRunning);
        assert_eq!(
            docker_error(LINUX_PERMISSION),
            DockerError::SocketPermission
        );
        assert_eq!(docker_error(LINUX_NOT_RUNNING), DockerError::NotRunning);
        assert_eq!(docker_error("Error: something odd"), DockerError::Other);
    }

    #[test]
    fn os_release_families() {
        let ubuntu = os_release("NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n");
        assert_eq!(ubuntu.family, DistroFamily::Debian);
        assert_eq!(ubuntu.id, "ubuntu");
        assert_eq!(ubuntu.version_id.as_deref(), Some("24.04"));
        assert_eq!(ubuntu.name.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        let rocky =
            os_release("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\nVERSION_ID=\"9.4\"\n");
        assert_eq!(rocky.family, DistroFamily::Fedora);
        assert_eq!(rocky.id_like, ["rhel", "centos", "fedora"]);
        assert_eq!(os_release("ID=fedora\n").family, DistroFamily::Fedora);
        assert_eq!(
            os_release("ID=manjaro\nID_LIKE=arch\n").family,
            DistroFamily::Arch
        );
        assert_eq!(os_release("ID=nixos\n").family, DistroFamily::Other);
        assert_eq!(os_release("").family, DistroFamily::Other);
    }
}
