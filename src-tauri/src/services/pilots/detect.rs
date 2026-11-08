//! Finding a pilot that is already on this computer.
//!
//! The `installed` flag ([`super::store`]) is set when Nandi runs a pilot's
//! install. A pilot installed some other way (by hand, or from another
//! Nandi home) is just as usable, so before saying "not installed", Nandi
//! looks for it the way the install would have placed it ([`Probe`], one
//! per catalog entry: [`super::catalog::probe`]). A find is recorded as
//! installed and logged.
//!
//! Looking is read-only: nothing is ever installed without the user's
//! request.

use std::collections::HashMap;
use std::future::Future;
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{catalog, install, store, PilotDefinition};
use crate::db::Db;

/// A slow npm or Docker mustn't hold up a run
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a "not there" answer stands before looking again: every step
/// of a run asks, and `npm ls -g` takes a second or two
const MISS_TTL: Duration = Duration::from_secs(60);

/// Where a pilot's install puts it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// `npm install -g <package>@<version>`
    NpmGlobal {
        package: &'static str,
        version: &'static str,
    },
    /// `docker pull <image>`
    DockerImage(&'static str),
}

impl Probe {
    /// The read-only command that answers, in JSON
    fn command_line(&self) -> String {
        match self {
            Probe::NpmGlobal { package, .. } => format!("npm ls -g --json --depth=0 {package}"),
            Probe::DockerImage(image) => format!("docker image inspect {image}"),
        }
    }

    /// Whether the command's output shows the pilot (exit codes say little:
    /// both commands exit 1 for "not there", and npm also for a warning)
    pub fn found_in(&self, output: &str) -> bool {
        match self {
            Probe::NpmGlobal { package, version } => npm_global_has(output, package, version),
            Probe::DockerImage(_) => docker_image_present(output),
        }
    }

    /// What was found, for the log
    pub fn describe(&self) -> String {
        match self {
            Probe::NpmGlobal { package, version } => format!("npm global {package}@{version}"),
            Probe::DockerImage(image) => format!("Docker image {image}"),
        }
    }
}

/// `npm ls -g --json` lists `package` at exactly `version`
pub fn npm_global_has(json: &str, package: &str, version: &str) -> bool {
    serde_json::from_str::<Value>(json)
        .is_ok_and(|tree| tree["dependencies"][package]["version"].as_str() == Some(version))
}

/// `docker image inspect` described an image (it prints `[]` when there's
/// none)
pub fn docker_image_present(json: &str) -> bool {
    serde_json::from_str::<Value>(json).is_ok_and(|images| {
        images
            .as_array()
            .is_some_and(|list| list.iter().any(|image| image["Id"].is_string()))
    })
}

/// Looks for a pilot on this computer. A trait so tests can stand in for
/// npm and Docker.
pub trait InstallProbe: Sync {
    /// What was found (for the log), or `None`
    fn find(&self, pilot: &PilotDefinition) -> impl Future<Output = Option<String>> + Send;
}

/// The real computer: runs the catalog's probe through the host shell, the
/// way the install command runs
pub struct SystemProbe;

/// Recent misses, by pilot ID ([`MISS_TTL`])
static MISSES: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Mutex::default);

fn missed_recently(id: &str) -> bool {
    let misses = MISSES.lock().unwrap_or_else(|e| e.into_inner());
    misses.get(id).is_some_and(|at| at.elapsed() < MISS_TTL)
}

fn remember_miss(id: &str, missed: bool) {
    let mut misses = MISSES.lock().unwrap_or_else(|e| e.into_inner());
    if missed {
        misses.insert(id.to_string(), Instant::now());
    } else {
        misses.remove(id);
    }
}

impl InstallProbe for SystemProbe {
    async fn find(&self, pilot: &PilotDefinition) -> Option<String> {
        let probe = catalog::probe(&pilot.id)?;
        if missed_recently(&pilot.id) {
            return None;
        }
        let output = tokio::time::timeout(
            PROBE_TIMEOUT,
            install::command(&probe.command_line())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let found = match output {
            Ok(Ok(output)) => probe.found_in(&String::from_utf8_lossy(&output.stdout)),
            // No npm or Docker, or it hung: not found
            _ => false,
        };
        remember_miss(&pilot.id, !found);
        found.then(|| probe.describe())
    }
}

/// Whether `pilot` is installed: the flag, else a look on this computer. A
/// find is recorded, so the next check is the flag alone. `Ok(true)` with
/// `newly` set tells the caller to announce the change.
pub async fn installed(
    db: &Db,
    probe: &impl InstallProbe,
    pilot: &PilotDefinition,
) -> Result<Found, String> {
    let id = pilot.id.clone();
    let flagged = db
        .call(move |conn| Ok(store::load(conn, &id)?.installed))
        .await?;
    if flagged {
        return Ok(Found::Recorded);
    }
    let Some(what) = probe.find(pilot).await else {
        return Ok(Found::No);
    };
    let id = pilot.id.clone();
    db.call(move |conn| store::save_installed(conn, &id, true))
        .await?;
    log::info!(
        "Found pilot '{}' already installed ({what}); recorded it as installed",
        pilot.id
    );
    Ok(Found::Detected)
}

/// What [`installed`] found
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// Not installed
    No,
    /// Installed through Nandi, or found before
    Recorded,
    /// Found on this computer just now, and recorded
    Detected,
}

impl Found {
    pub fn yes(self) -> bool {
        self != Found::No
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Finds what it's told to, without npm or Docker
    pub(crate) struct FakeProbe(pub Option<&'static str>);

    impl InstallProbe for FakeProbe {
        async fn find(&self, _pilot: &PilotDefinition) -> Option<String> {
            self.0.map(str::to_string)
        }
    }

    const NPM_FOUND: &str = r#"{
  "name": "npm",
  "dependencies": {
    "@playwright/mcp": {
      "version": "0.0.83",
      "overridden": false
    }
  }
}"#;

    #[test]
    fn reads_npm_global_listings() {
        let probe = Probe::NpmGlobal {
            package: "@playwright/mcp",
            version: "0.0.83",
        };
        assert!(probe.found_in(NPM_FOUND));
        // Not installed: npm prints only the root
        assert!(!probe.found_in("{\n  \"name\": \"npm\"\n}"));
        // Another version isn't the pinned one
        assert!(!probe.found_in(&NPM_FOUND.replace("0.0.83", "0.0.40")));
        // npm missing or printing an error
        assert!(!probe.found_in(""));
        assert!(!probe.found_in("'npm' is not recognized"));
        assert_eq!(
            probe.command_line(),
            "npm ls -g --json --depth=0 @playwright/mcp"
        );
    }

    #[test]
    fn reads_docker_image_inspect() {
        let probe = Probe::DockerImage("mcr.microsoft.com/playwright/mcp:v0.0.83");
        let found = r#"[
    {
        "Id": "sha256:3f1c",
        "RepoTags": ["mcr.microsoft.com/playwright/mcp:v0.0.83"],
        "Architecture": "amd64"
    }
]"#;
        assert!(probe.found_in(found));
        // No such image: `[]` on stdout, the error on stderr
        assert!(!probe.found_in("[]\n"));
        assert!(!probe.found_in(""));
        assert_eq!(
            probe.command_line(),
            "docker image inspect mcr.microsoft.com/playwright/mcp:v0.0.83"
        );
    }

    #[test]
    fn misses_are_remembered_for_a_while() {
        let id = "detect-test-miss";
        assert!(!missed_recently(id));
        remember_miss(id, true);
        assert!(missed_recently(id));
        remember_miss(id, false);
        assert!(!missed_recently(id));
    }

    #[tokio::test]
    async fn a_find_is_recorded_and_a_miss_is_not() {
        let db = Db::in_memory();
        let pilot = catalog::find("browser").unwrap();

        let found = installed(&db, &FakeProbe(None), &pilot).await.unwrap();
        assert_eq!(found, Found::No);

        let found = installed(&db, &FakeProbe(Some("npm global x")), &pilot)
            .await
            .unwrap();
        assert_eq!(found, Found::Detected);
        let id = pilot.id.clone();
        let flagged = db
            .call(move |conn| Ok(store::load(conn, &id)?.installed))
            .await
            .unwrap();
        assert!(flagged);

        // Now the flag answers, without looking
        let found = installed(&db, &FakeProbe(None), &pilot).await.unwrap();
        assert_eq!(found, Found::Recorded);
    }
}
