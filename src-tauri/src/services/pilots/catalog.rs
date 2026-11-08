//! The built-in pilot catalog. Each entry is a host-side MCP server on a
//! fixed loopback port; the UI only ever shows the current OS's entries.
//! Only packages that exist are listed (checked 2026-10-29: Win Pilot,
//! UIA-X, MacPilot, Desktop Pilot and OSWright were never published).

use super::PilotDefinition;

/// Capabilities a pilot can expose (what the consent dialog asks about)
pub const APP_CONTROL: &str = "app_control";
pub const FILE_SYSTEM: &str = "file_system";
pub const BROWSER: &str = "browser";
pub const SYSTEM_SETTINGS: &str = "system_settings";

/// `windows` | `macos` | `linux`, as `PilotDefinition::platform` spells it.
pub fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

#[allow(clippy::too_many_arguments)] // One row of the catalog table
fn pilot(
    id: &str,
    name: &str,
    platform: &str,
    install_command: &str,
    start_command: &str,
    mcp_port: u16,
    description: &str,
    capabilities: &[&str],
) -> PilotDefinition {
    PilotDefinition {
        id: id.to_string(),
        name: name.to_string(),
        platform: platform.to_string(),
        install_command: install_command.to_string(),
        start_command: start_command.to_string(),
        mcp_port,
        description: description.to_string(),
        capabilities: capabilities.iter().map(ToString::to_string).collect(),
    }
}

/// Every pilot Nandi knows, for every OS. Only published packages: an
/// entry the user can't install would show a broken card.
pub fn all() -> Vec<PilotDefinition> {
    let mut all = super::browser::definitions();
    all.push(pilot(
        "opendesk",
        "OpenDesk",
        "any",
        "pipx install opendesk",
        "opendesk serve --port 17899",
        17899,
        "Open-source desktop control: windows, files, and browser tabs.",
        &[APP_CONTROL, FILE_SYSTEM, BROWSER],
    ));
    all
}

/// The pilots that run on `platform` (its own plus the cross-platform ones).
pub fn for_platform(platform: &str) -> Vec<PilotDefinition> {
    all()
        .into_iter()
        .filter(|p| p.platform == platform || p.platform == "any")
        .collect()
}

/// How to find `id` already installed on this computer, when the catalog
/// knows ([`super::detect`])
pub fn probe(id: &str) -> Option<super::detect::Probe> {
    super::browser::probe(id)
}

/// A pilot available on this OS.
pub fn find(id: &str) -> Result<PilotDefinition, String> {
    find_in(all(), id, current_platform())
}

/// `find` over `pilots` for `platform`
fn find_in(
    pilots: Vec<PilotDefinition>,
    id: &str,
    platform: &str,
) -> Result<PilotDefinition, String> {
    match pilots.into_iter().find(|p| p.id == id) {
        Some(p) if p.platform == platform || p.platform == "any" => Ok(p),
        Some(p) => Err(format!("{} isn't available on {platform}", p.name)),
        None => Err(format!("Unknown pilot '{id}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_platform() {
        let ids = |platform| -> Vec<String> {
            for_platform(platform).into_iter().map(|p| p.id).collect()
        };
        for platform in ["windows", "macos", "linux"] {
            assert_eq!(
                ids(platform),
                ["browser", "browser-headless", "opendesk"],
                "{platform}"
            );
        }
    }

    #[test]
    fn ports_are_unique_and_match_the_start_command() {
        // The browsers run one server per agent over stdio: no port
        let pilots: Vec<_> = all()
            .into_iter()
            .filter(|p| !super::super::browser::is_browser(&p.id))
            .collect();
        let mut ports: Vec<u16> = pilots.iter().map(|p| p.mcp_port).collect();
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(ports.len(), pilots.len());
        for p in &pilots {
            assert!(
                p.start_command.contains(&p.mcp_port.to_string()),
                "{}",
                p.id
            );
            assert!(!p.capabilities.is_empty(), "{}", p.id);
        }
    }

    #[test]
    fn find_rejects_other_platforms() {
        assert!(find("opendesk").is_ok());
        assert!(find("nope").unwrap_err().contains("Unknown"));
        let mac_only = vec![pilot(
            "mac-only",
            "Mac only",
            "macos",
            "",
            "",
            1,
            "",
            &[APP_CONTROL],
        )];
        assert!(find_in(mac_only.clone(), "mac-only", "macos").is_ok());
        assert!(find_in(mac_only, "mac-only", "windows")
            .unwrap_err()
            .contains("isn't available on windows"));
    }
}
