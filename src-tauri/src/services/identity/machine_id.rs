//! This computer, as the public relay and the trial gateway see it: a
//! **salted hash** of the OS machine ID, never the ID itself.
//!
//! | OS | Machine ID |
//! | --- | --- |
//! | Windows | `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` (`reg query`) |
//! | macOS | `IOPlatformUUID` (`ioreg -rd1 -c IOPlatformExpertDevice`) |
//! | Linux | `/etc/machine-id`, else `/var/lib/dbus/machine-id` |
//!
//! The hash is `hex(SHA-256("nandi-machine-v1\n<salt>\n<id>"))` with the ID
//! trimmed and lowercased. Each service has its own salt ([`Service`]), so
//! the relay's and the gateway's hashes of one computer can't be matched
//! with each other. The services use it to share the free trial per
//! computer and to cap how many Nandis one computer joins to the public
//! relay. Reading the ID never fails the app: without it, those calls say
//! so.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

/// Who the hash is for (each has its own salt)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// The public relay (`relay::public`)
    Relay,
    /// The free trial's gateway (`llm_trial::gateway`)
    Trial,
}

impl Service {
    pub fn salt(self) -> &'static str {
        match self {
            Self::Relay => "nandi-relay",
            Self::Trial => "nandi-trial",
        }
    }
}

/// `hex(SHA-256("nandi-machine-v1\n<salt>\n<id>"))`: 64 lowercase hex digits
pub fn salted_hash(service: Service, machine_id: &str) -> String {
    let id = machine_id.trim().to_ascii_lowercase();
    let salt = service.salt();
    Sha256::digest(format!("nandi-machine-v1\n{salt}\n{id}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// This computer's hash for `service`. The ID is read once per run.
pub fn this_machine(service: Service) -> Result<String, String> {
    static ID: OnceLock<Result<String, String>> = OnceLock::new();
    ID.get_or_init(read_machine_id)
        .as_ref()
        .map(|id| salted_hash(service, id))
        .map_err(Clone::clone)
}

/// The `MachineGuid` value in `reg query`'s output
#[cfg(any(target_os = "windows", test))]
pub fn parse_reg_query(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == "MachineGuid" && words.next()? == "REG_SZ")
            .then(|| words.next().map(str::to_string))
            .flatten()
            .filter(|id| !id.is_empty())
    })
}

/// The `IOPlatformUUID` in `ioreg`'s output
#[cfg(any(target_os = "macos", test))]
pub fn parse_ioreg(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim().trim_matches('"') == "IOPlatformUUID")
            .then(|| value.trim().trim_matches('"').to_string())
            .filter(|id| !id.is_empty())
    })
}

/// A `machine-id` file's ID (32 hex digits)
#[cfg(any(not(any(target_os = "windows", target_os = "macos")), test))]
pub fn parse_machine_id_file(text: &str) -> Option<String> {
    let id = text.trim();
    (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| id.to_string())
}

#[cfg(target_os = "windows")]
fn read_machine_id() -> Result<String, String> {
    let output = command_output(
        "reg",
        &[
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ],
    )?;
    parse_reg_query(&output).ok_or_else(|| "Windows has no MachineGuid".to_string())
}

#[cfg(target_os = "macos")]
fn read_machine_id() -> Result<String, String> {
    let output = command_output("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])?;
    parse_ioreg(&output).ok_or_else(|| "macOS gave no IOPlatformUUID".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn read_machine_id() -> Result<String, String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|path| {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|text| parse_machine_id_file(&text))
        })
        .ok_or_else(|| "This system has no /etc/machine-id".to_string())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|e| format!("Couldn't run {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!("{program} failed ({})", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_salted_per_service_and_never_the_id() {
        let id = "4C4C4544-0042-3510-8051-B4C04F383732";
        let relay = salted_hash(Service::Relay, id);
        let trial = salted_hash(Service::Trial, id);
        assert_eq!(relay.len(), 64);
        assert!(relay
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_ne!(relay, trial);
        assert!(!relay.contains("4c4c4544"));
        // Case and spaces around the ID don't matter
        assert_eq!(
            relay,
            salted_hash(Service::Relay, &format!(" {} ", id.to_lowercase()))
        );
        // The fixed vector both services can check
        assert_eq!(
            salted_hash(Service::Trial, "0123456789abcdef0123456789abcdef"),
            Sha256::digest(b"nandi-machine-v1\nnandi-trial\n0123456789abcdef0123456789abcdef")
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
    }

    #[test]
    fn windows_reg_query_output() {
        let output = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    5c8f3a2e-1b2c-4d5e-9f00-112233445566\r\n\r\n";
        assert_eq!(
            parse_reg_query(output).as_deref(),
            Some("5c8f3a2e-1b2c-4d5e-9f00-112233445566")
        );
        assert_eq!(parse_reg_query("ERROR: not found"), None);
    }

    #[test]
    fn macos_ioreg_output() {
        let output = "+-o J314sAP  <class IOPlatformExpertDevice>\n    {\n      \"IOPlatformSerialNumber\" = \"C02XXXX\"\n      \"IOPlatformUUID\" = \"8D6E2E4C-58C1-5B3A-9F1E-0A1B2C3D4E5F\"\n    }\n";
        assert_eq!(
            parse_ioreg(output).as_deref(),
            Some("8D6E2E4C-58C1-5B3A-9F1E-0A1B2C3D4E5F")
        );
        assert_eq!(parse_ioreg("\"IOPlatformSerialNumber\" = \"x\""), None);
    }

    #[test]
    fn linux_machine_id_file() {
        assert_eq!(
            parse_machine_id_file("b08dfa6083e7567a1921a715000001fb\n").as_deref(),
            Some("b08dfa6083e7567a1921a715000001fb")
        );
        assert_eq!(parse_machine_id_file("uninitialized\n"), None);
        assert_eq!(parse_machine_id_file(""), None);
    }

    #[test]
    fn this_computer_has_an_id() {
        // Every OS CI runs on has one
        let hash = this_machine(Service::Relay).unwrap();
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, this_machine(Service::Relay).unwrap());
    }
}
