//! "Get your computer ready" (Tauri adapter for `services::machine`).
//! Starting the installs is desktop-only (`desktop::machine`).

use crate::services::machine::ready::ReadyProgress;
use crate::services::machine::{self, install, MachineReport, MachineSettings};

super::commands! {
    /// Is this computer ready for agents to act: Docker, WSL, virtualization,
    /// disk space, Nandi's images. Read-only. `fresh` re-runs the slower
    /// checks (otherwise they come from the last minute).
    fn check_machine(fresh: bool) -> MachineReport => Ok(machine::check(fresh).await);

    /// Launches Docker Desktop (no admin rights). Linux Docker Engine needs
    /// `sudo systemctl start docker`, which the app shows instead.
    fn start_docker() -> () => machine::start_docker().await;

    /// How the last (or running) "Get your computer ready" is going
    fn computer_ready_status() -> ReadyProgress => Ok(install::status());

    /// This computer's settings (start Docker when Nandi opens)
    fn machine_settings(state) -> MachineSettings => machine::settings(&state).await;

    /// The answer to "Start Docker automatically when Nandi opens?",
    /// remembered on this computer
    fn set_start_docker_on_open(state, start: bool) -> () => machine::set_start_docker_on_open(&state, start).await;
}
