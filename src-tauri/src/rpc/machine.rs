//! "Get your computer ready" (Tauri adapter for `services::machine`).

use crate::services::machine::ready::ReadyProgress;
use crate::services::machine::{self, install, MachineReport, MachineSettings, ReadyStep};

super::commands! {
    /// Is this computer ready for agents to act: Docker, WSL, virtualization,
    /// disk space, Nandi's images. Read-only. `fresh` re-runs the slower
    /// checks (otherwise they come from the last minute).
    fn check_machine(fresh: bool) -> MachineReport => Ok(machine::check(fresh).await);

    /// Launches Docker Desktop (no admin rights). Linux Docker Engine needs
    /// `sudo systemctl start docker`, which the app shows instead.
    fn start_docker() -> () => machine::start_docker().await;

    /// The one go-ahead: runs the report's `plan` (`steps`, as the user saw
    /// them) in the background; progress arrives as `machine://ready`. The
    /// installers ask the OS for admin rights themselves
    fn get_computer_ready(state, steps: Vec<ReadyStep>) -> () => install::start(&state, steps).await;

    /// How the last (or running) "Get your computer ready" is going
    fn computer_ready_status() -> ReadyProgress => Ok(install::status());

    /// The machine's portable settings (start Docker when Nandi opens)
    fn machine_settings(state) -> MachineSettings => Ok(machine::settings(&state));

    /// The answer to "Start Docker automatically when Nandi opens?"
    fn set_start_docker_on_open(state, start: bool) -> () => machine::set_start_docker_on_open(&state, start).await;
}
