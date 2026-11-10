//! Command-line flags.
//!
//! - (none): desktop app; web port on `127.0.0.1:<random>`
//! - `--server`: also expose the web port on all interfaces (`0.0.0.0`),
//!   port 17463 unless `--port` is given, and print the access link
//! - `--port <n>`: web port to use
//! - `--headless`: no window; run only the core and the web server
//! - `-h` / `--help`: print usage and exit
//! - `verify --agent <id>`: check an agent's end-to-end prerequisites and
//!   exit (see `verify.rs`)
//! - `rebuild [--force]`: rebuild the SQLite projection from the repo files
//!   and exit (see `services::repo_service`)
//! - `keygen` / `sign …`: marketplace publisher keys and release signatures
//!   (see `services::marketplace::signature`)
//!
//! Unknown flags are rejected. Positional arguments (e.g. file paths from
//! the OS) and macOS's `-psn_…` are ignored.

use std::net::{Ipv4Addr, SocketAddr};

use crate::http::DEFAULT_SERVER_PORT;

pub const USAGE: &str = "Usage: nandi [OPTIONS]
       nandi verify --agent <ID>
       nandi rebuild [--force]
       nandi keygen [--key <PATH>]
       nandi name
       nandi rename <NAME> | --reset
       nandi sign --kind <agent|pack|connector> --id <ID> --release <VERSION>
                 (--commit <SHA> | --repo <REPO>) [--key <PATH>]

Options:
  --server      Expose the web UI on all interfaces (port 17463 unless --port)
                and print the access link
  --headless    Run without a window: only the core and the web server
                (127.0.0.1:17463 unless --server or --port)
  --port <N>    Web server port (also --port=N)
  -h, --help    Print this help and exit

Commands:
  verify --agent <ID>  Check Docker, the runtime image, the state database,
                       the model's API key, and the agent's skill.md; exit
                       non-zero if any check fails
  rebuild [--force]    Rebuild the local database's view of the repo files
                       (agents, workflows, triggers, settings). Refuses while
                       a file is invalid unless --force (which skips it).
                       Run history is kept. Best run with the app closed
  keygen              Create a marketplace publisher key (default
                       ~/.nandi/keys/publisher.key) and print its public key
                       for publishers/<handle>.json
  name                 Print the bull's name (Nandi unless renamed)
  rename <NAME>        Rename the bull; --reset goes back to Nandi
  sign                 Sign a release for a registry entry and print its
                       \"commit\" and \"signature\" fields. --repo resolves the
                       v<VERSION> tag's commit (e.g. github.com/you/repo)

Environment:
  NANDI_RELAY_URL     Relay to connect to; wins over relay_url in
                       ~/.nandi/config.json. With neither, Nandi doesn't
                       connect (the Relay screen suggests
                       ws://127.0.0.1:8787/relay for LAN testing)
  NANDI_PUBLIC_RELAY_URL  The public relay \"Join the public relay\" signs
                       in to (default wss://relay.vaultdb.ai/relay)
  NANDI_HOME          Use this folder instead of ~/.nandi";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOptions {
    pub server: bool,
    pub headless: bool,
    pub port: Option<u16>,
    pub help: bool,
    /// `verify --agent <id>`: the agent to check
    pub verify: Option<String>,
    /// `rebuild [--force]`: whether to skip invalid files
    pub rebuild: Option<bool>,
    /// `keygen` / `sign`
    pub publisher: Option<PublisherCommand>,
    /// `name` / `rename` / the hidden phrase (`cli_mascot`)
    pub mascot: Option<crate::cli_mascot::MascotCommand>,
}

/// Marketplace publishing commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherCommand {
    Keygen {
        key: Option<String>,
    },
    Sign {
        kind: String,
        id: String,
        release: String,
        /// The release commit, or…
        commit: Option<String>,
        /// …the repo whose `v<release>` tag names it
        repo: Option<String>,
        key: Option<String>,
    },
}

fn parse_publisher(
    command: &str,
    args: impl Iterator<Item = String>,
) -> Result<PublisherCommand, String> {
    let mut values: std::collections::HashMap<String, String> = Default::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (arg.clone(), None),
        };
        let allowed: &[&str] = if command == "keygen" {
            &["--key"]
        } else {
            &["--kind", "--id", "--release", "--commit", "--repo", "--key"]
        };
        if !allowed.contains(&name.as_str()) {
            return Err(format!(
                "Unknown option '{arg}' for {command}. Run with --help for usage."
            ));
        }
        let value = match inline {
            Some(value) => value,
            None => args.next().ok_or(format!("{name} needs a value"))?,
        };
        values.insert(name, value);
    }
    let mut take = |name: &str| values.remove(name);
    if command == "keygen" {
        return Ok(PublisherCommand::Keygen { key: take("--key") });
    }
    let kind = take("--kind").ok_or("sign needs --kind agent|pack|connector")?;
    if !matches!(kind.as_str(), "agent" | "pack" | "connector") {
        return Err("--kind must be agent, pack or connector".to_string());
    }
    let id = take("--id").ok_or("sign needs --id <ID>")?;
    let release = take("--release").ok_or("sign needs --release <VERSION>")?;
    let (commit, repo) = (take("--commit"), take("--repo"));
    if commit.is_none() == repo.is_none() {
        return Err("sign needs exactly one of --commit <SHA> or --repo <REPO>".to_string());
    }
    Ok(PublisherCommand::Sign {
        kind,
        id,
        release,
        commit,
        repo,
        key: take("--key"),
    })
}

impl CliOptions {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::args().skip(1))
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let args: Vec<String> = args.into_iter().collect();
        if let Some(command) = crate::cli_mascot::parse(&args)? {
            options.mascot = Some(command);
            return Ok(options);
        }
        let mut args = args.into_iter().peekable();
        if let Some(command) = args.next_if(|arg| arg == "keygen" || arg == "sign") {
            options.publisher = Some(parse_publisher(&command, args)?);
            return Ok(options);
        }
        let verify = args.next_if(|arg| arg == "verify").is_some();
        let rebuild = !verify && args.next_if(|arg| arg == "rebuild").is_some();
        let mut force = false;
        let mut agent = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--agent" => agent = Some(args.next().ok_or("--agent needs a value")?),
                "--force" => force = true,
                "--server" => options.server = true,
                "--headless" => options.headless = true,
                "-h" | "--help" => options.help = true,
                "--port" => {
                    let value = args.next().ok_or("--port needs a value")?;
                    options.port = Some(parse_port(&value)?);
                }
                other => {
                    if let Some(value) = other.strip_prefix("--port=") {
                        options.port = Some(parse_port(value)?);
                    } else if let Some(value) = other.strip_prefix("--agent=") {
                        agent = Some(value.to_string());
                    } else if other.starts_with('-') && !other.starts_with("-psn_") {
                        return Err(format!(
                            "Unknown option '{other}'. Run with --help for usage."
                        ));
                    }
                }
            }
        }
        match (verify, agent) {
            (true, Some(agent)) if !agent.is_empty() => options.verify = Some(agent),
            (true, _) => return Err("verify needs --agent <id>".to_string()),
            (false, Some(_)) => return Err("--agent is only valid with verify".to_string()),
            (false, None) => {}
        }
        match (rebuild, force) {
            (true, force) => options.rebuild = Some(force),
            (false, true) => return Err("--force is only valid with rebuild".to_string()),
            (false, false) => {}
        }
        Ok(options)
    }

    /// Where the web server binds. Only `--server` exposes it beyond this
    /// machine.
    pub fn bind(&self) -> SocketAddr {
        let ip = if self.server {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let port = self.port.unwrap_or(if self.server || self.headless {
            DEFAULT_SERVER_PORT
        } else {
            0 // random free port for the desktop app
        });
        SocketAddr::from((ip, port))
    }

    /// Whether to print the access link (with token) at startup.
    pub fn announce(&self) -> bool {
        self.server || self.headless
    }
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse()
        .map_err(|_| format!("Invalid --port value '{value}'"))
}

/// Release builds on Windows are GUI-subsystem apps with no console, so
/// output from `--help`, errors, and the access link would be lost. When
/// started with arguments from a terminal, attach to that terminal's
/// console. Elsewhere this does nothing.
pub fn attach_parent_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn AttachConsole(process_id: u32) -> i32;
        }
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        if std::env::args_os().len() > 1 {
            // Fails harmlessly when there's no parent console (e.g. Explorer)
            unsafe {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
}

/// Also writes the access link to `~/.nandi/server.log` on Windows release
/// builds, where there may be no console to print it to.
pub fn record_access_url(url: &str) {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        let written = crate::utils::nandi_config::nandi_dir().and_then(|dir| {
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(dir.join("server.log"), format!("{url}\n")))
                .map_err(|e| e.to_string())
        });
        if let Err(e) = written {
            log::warn!("Failed to write server.log: {e}");
        }
    }
    #[cfg(not(all(windows, not(debug_assertions))))]
    let _ = url;
}

/// The browser link for a bound address, including the token.
pub fn access_url(addr: SocketAddr, token: &str) -> String {
    let host = if addr.ip().is_unspecified() {
        "localhost".to_string()
    } else {
        addr.ip().to_string()
    };
    format!("http://{host}:{}/?token={token}", addr.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> CliOptions {
        CliOptions::parse(args.iter().map(|s| s.to_string())).unwrap()
    }

    #[test]
    fn defaults_to_a_random_loopback_port() {
        let options = parse(&[]);
        assert_eq!(options.bind(), "127.0.0.1:0".parse().unwrap());
        assert!(!options.announce());
    }

    #[test]
    fn server_binds_all_interfaces() {
        assert_eq!(
            parse(&["--server"]).bind(),
            "0.0.0.0:17463".parse().unwrap()
        );
        assert_eq!(
            parse(&["--server", "--port", "8080"]).bind(),
            "0.0.0.0:8080".parse().unwrap()
        );
        assert_eq!(
            parse(&["--port=9000"]).bind(),
            "127.0.0.1:9000".parse().unwrap()
        );
    }

    #[test]
    fn headless_stays_on_loopback_unless_server() {
        let options = parse(&["--headless"]);
        assert!(options.headless && options.announce());
        assert_eq!(options.bind(), "127.0.0.1:17463".parse().unwrap());
    }

    fn parse_err(args: &[&str]) -> String {
        CliOptions::parse(args.iter().map(|s| s.to_string())).unwrap_err()
    }

    #[test]
    fn parses_publisher_commands() {
        assert_eq!(
            parse(&["keygen"]).publisher,
            Some(PublisherCommand::Keygen { key: None })
        );
        assert_eq!(
            parse(&[
                "sign",
                "--kind",
                "pack",
                "--id",
                "research-pack",
                "--release=1.0.0",
                "--repo",
                "github.com/a/b"
            ])
            .publisher,
            Some(PublisherCommand::Sign {
                kind: "pack".into(),
                id: "research-pack".into(),
                release: "1.0.0".into(),
                commit: None,
                repo: Some("github.com/a/b".into()),
                key: None,
            })
        );
        assert!(
            parse_err(&["sign", "--kind", "pack", "--id", "x", "--release", "1.0.0"])
                .contains("exactly one")
        );
        assert!(parse_err(&["sign", "--kind", "tool"]).contains("agent, pack or connector"));
        assert!(parse_err(&["keygen", "--server"]).contains("Unknown option"));
    }

    #[test]
    fn rejects_unknown_flags_and_bad_ports() {
        assert!(parse_err(&["--foo"]).contains("Unknown option '--foo'"));
        assert!(parse_err(&["--server", "-x"]).contains("--help"));
        assert!(parse_err(&["--port", "x"]).contains("Invalid --port"));
        assert!(parse_err(&["--port"]).contains("needs a value"));
    }

    #[test]
    fn ignores_positional_args_and_macos_psn() {
        assert_eq!(parse(&["-psn_0_123", "file.txt"]), CliOptions::default());
    }

    #[test]
    fn parses_verify() {
        assert_eq!(
            parse(&["verify", "--agent", "echo-agent"])
                .verify
                .as_deref(),
            Some("echo-agent")
        );
        assert_eq!(
            parse(&["verify", "--agent=echo-agent"]).verify.as_deref(),
            Some("echo-agent")
        );
        assert!(parse_err(&["verify"]).contains("needs --agent"));
        assert!(parse_err(&["--agent", "a"]).contains("only valid with verify"));
        // Only as the first argument
        assert_eq!(parse(&["--server", "verify"]).verify, None);
    }

    #[test]
    fn parses_rebuild() {
        assert_eq!(parse(&["rebuild"]).rebuild, Some(false));
        assert_eq!(parse(&["rebuild", "--force"]).rebuild, Some(true));
        assert_eq!(parse(&[]).rebuild, None);
        let error = CliOptions::parse(["--force".to_string()]).unwrap_err();
        assert!(error.contains("only valid with rebuild"));
        assert!(USAGE.contains("rebuild [--force]"));
    }

    #[test]
    fn parses_help() {
        assert!(parse(&["-h"]).help);
        assert!(parse(&["--server", "--help"]).help);
        assert!(USAGE.contains("--headless") && USAGE.contains("--port"));
    }

    #[test]
    fn usage_documents_the_relay_url_override() {
        use crate::services::relay::{DEFAULT_RELAY_URL, RELAY_URL_ENV};
        assert!(USAGE.contains(RELAY_URL_ENV));
        assert!(USAGE.contains(DEFAULT_RELAY_URL));
    }

    #[test]
    fn builds_access_links() {
        assert_eq!(
            access_url("0.0.0.0:17463".parse().unwrap(), "t"),
            "http://localhost:17463/?token=t"
        );
    }
}
