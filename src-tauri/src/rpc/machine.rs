//! "Get your computer ready" (Tauri adapter for `services::machine`).

use crate::services::machine::{self, MachineReport};

super::commands! {
    /// Is this computer ready for agents to act: Docker, WSL, virtualization,
    /// disk space, Nandi's images. Read-only. `fresh` re-runs the slower
    /// checks (otherwise they come from the last minute).
    fn check_machine(fresh: bool) -> MachineReport => Ok(machine::check(fresh).await);

    /// Launches Docker Desktop (no admin rights). Linux Docker Engine needs
    /// `sudo systemctl start docker`, which the app shows instead.
    fn start_docker() -> () => machine::start_docker().await;
}
