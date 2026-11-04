//! Runs a pilot's install command through the platform shell, streaming
//! each output line to the UI as `pilot://install_output`.
//!
//! Commands come only from the built-in catalog, never from user input.

use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

use super::catalog::current_platform;
use crate::services::events::{self, EventBus, NandiEvent};

/// Package managers can be slow, but an install must not hang forever
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// One line of install output.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PilotInstallOutput {
    pub pilot_id: String,
    /// `stdout` | `stderr`
    pub stream: &'static str,
    pub line: String,
}

impl NandiEvent for PilotInstallOutput {
    const NAME: &'static str = "pilot://install_output";
}

/// The shell invocation for `line` on `platform`: `cmd /C` on Windows,
/// `sh -c` elsewhere.
pub fn shell_command(platform: &str, line: &str) -> (&'static str, Vec<String>) {
    if platform == "windows" {
        ("cmd", vec!["/C".to_string(), line.to_string()])
    } else {
        ("sh", vec!["-c".to_string(), line.to_string()])
    }
}

/// `line` as a child process of the host shell, without a console window.
pub fn command(line: &str) -> Command {
    let (program, args) = shell_command(current_platform(), line);
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Runs `line` to completion. `Err` if it can't start, times out, or exits
/// non-zero.
pub async fn run(bus: &EventBus, pilot_id: &str, line: &str) -> Result<(), String> {
    log::info!("Installing pilot '{pilot_id}': {line}");
    let mut child = command(line)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to run the install command: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let finished = tokio::time::timeout(INSTALL_TIMEOUT, async {
        tokio::join!(
            pump(bus, pilot_id, "stdout", stdout),
            pump(bus, pilot_id, "stderr", stderr),
        );
        child.wait().await
    })
    .await;
    match finished {
        Err(_) => Err(format!(
            "The install command didn't finish within {} minutes",
            INSTALL_TIMEOUT.as_secs() / 60
        )),
        Ok(Err(e)) => Err(format!("The install command failed: {e}")),
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(match status.code() {
            Some(code) => format!("The install command exited with code {code}"),
            None => "The install command was terminated".to_string(),
        }),
    }
}

async fn pump(
    bus: &EventBus,
    pilot_id: &str,
    stream: &'static str,
    reader: Option<impl AsyncRead + Unpin>,
) {
    let Some(reader) = reader else { return };
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        events::emit(
            bus,
            &PilotInstallOutput {
                pilot_id: pilot_id.to_string(),
                stream,
                line,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::bus;

    #[test]
    fn uses_the_platform_shell() {
        assert_eq!(
            shell_command("windows", "pipx install x"),
            ("cmd", vec!["/C".to_string(), "pipx install x".to_string()])
        );
        for unix in ["macos", "linux"] {
            assert_eq!(
                shell_command(unix, "brew install x"),
                ("sh", vec!["-c".to_string(), "brew install x".to_string()])
            );
        }
    }

    #[tokio::test]
    async fn streams_output_and_reports_failure() {
        let (bus, sink) = bus();
        run(&bus, "p", "echo installed-ok").await.unwrap();
        let events = sink.take();
        let line = events
            .iter()
            .find(|(name, _)| *name == "pilot://install_output")
            .map(|(_, payload)| payload.clone())
            .expect("an output line");
        assert_eq!(line["pilotId"], "p");
        assert_eq!(line["stream"], "stdout");
        assert_eq!(line["line"].as_str().unwrap().trim(), "installed-ok");

        let error = run(&bus, "p", "exit 3").await.unwrap_err();
        assert!(error.contains("code 3"), "{error}");
    }
}
