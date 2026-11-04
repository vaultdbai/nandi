//! Pilot processes: spawning `start_command`, killing it, and the health
//! monitor that decides whether the orchestrator may route to it.
//!
//! Every check is a background task with a hard timeout, so a hung pilot
//! never blocks a command, let alone the UI thread.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Child;

use super::{announce, install, store, PilotDefinition, PilotRegistry};
use crate::services::events::EventBus;

/// Per health check
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
/// Between checks while running: a crash is noticed within this long
pub const HEALTH_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(30)
};
/// Right after start, poll faster until the server first answers
const STARTUP_POLL: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_millis(500)
};
const STARTUP_POLLS: u32 = 40;

/// `GET http://127.0.0.1:<port>/health` answers 2xx within [`HEALTH_TIMEOUT`].
pub async fn health_check(port: u16) -> bool {
    check_url(&format!("http://127.0.0.1:{port}/health"), HEALTH_TIMEOUT).await
}

/// Whether `url` answers 2xx within `timeout`. Any error is `false`.
pub async fn check_url(url: &str, timeout: Duration) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .no_proxy()
        .build()
    else {
        return false;
    };
    // The outer timeout also bounds DNS and anything reqwest doesn't
    tokio::time::timeout(timeout, client.get(url).send())
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|response| response.status().is_success())
}

/// Spawns the pilot's server. It dies with the app (`kill_on_drop`).
pub fn spawn(pilot: &PilotDefinition) -> Result<Child, String> {
    // `exec` so the pid is the server's, not the shell's
    let line = if cfg!(windows) {
        pilot.start_command.clone()
    } else {
        format!("exec {}", pilot.start_command)
    };
    install::command(&line)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start {}: {e}", pilot.name))
}

/// Kills the pilot and waits for it to exit. On Windows the shell's whole
/// process tree goes, since `cmd /C` doesn't exec.
pub async fn kill(mut child: Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let killed = install::command(&format!("taskkill /PID {pid} /T /F"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if let Err(e) = killed {
            log::warn!("taskkill {pid} failed: {e}");
        }
    }
    if let Err(e) = child.kill().await {
        log::debug!("Pilot process already gone: {e}");
    }
}

/// Watches one start of a pilot until it's stopped or restarted
/// (`generation` changes): the process exiting marks it stopped, a failed
/// health check marks it unhealthy. Both emit `pilot://changed`.
pub async fn monitor(
    bus: EventBus,
    registry: Arc<PilotRegistry>,
    pilot: PilotDefinition,
    generation: u64,
    mut healthy: bool,
) {
    let id = pilot.id.as_str();
    let mut startup_polls = if healthy { 0 } else { STARTUP_POLLS };
    loop {
        let wait = if startup_polls > 0 {
            startup_polls -= 1;
            STARTUP_POLL
        } else {
            HEALTH_INTERVAL
        };
        tokio::time::sleep(wait).await;
        if registry.generation_of(id) != Some(generation) {
            return;
        }

        if registry.has_exited(id) {
            log::warn!("Pilot '{id}' exited");
            if let Some(Some(child)) = registry.remove_if(id, generation) {
                kill(child).await;
            }
            let pilot_id = id.to_string();
            if let Err(e) = bus
                .db
                .call(move |conn| store::save_stopped(conn, &pilot_id))
                .await
            {
                log::warn!("Failed to record that pilot '{id}' stopped: {e}");
            }
            announce(&bus, id).await;
            return;
        }

        let now = health_check(pilot.mcp_port).await;
        if now {
            startup_polls = 0;
        }
        // Compare with what's stored: a failed tool call may have marked it
        // unhealthy since the last check
        let pilot_id = id.to_string();
        let stored = bus
            .db
            .call(move |conn| store::load(conn, &pilot_id))
            .await
            .map(|p| p.healthy)
            .unwrap_or(healthy);
        if now == stored {
            healthy = now;
            continue;
        }
        if healthy && !now {
            log::warn!("Pilot '{id}' failed its health check; not routing to it");
        }
        healthy = now;
        set_healthy(&bus, id, healthy).await;
    }
}

/// Records a health change and announces it.
pub async fn set_healthy(bus: &EventBus, id: &str, healthy: bool) {
    let pilot_id = id.to_string();
    match bus
        .db
        .call(move |conn| store::save_healthy(conn, &pilot_id, healthy))
        .await
    {
        Ok(()) => announce(bus, id).await,
        Err(e) => log::warn!("Failed to record pilot '{id}' health: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_check_times_out_as_unhealthy() {
        // Accepts the connection, never answers
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hold = tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    held.push(socket);
                }
            }
        });
        let started = std::time::Instant::now();
        let url = format!("http://127.0.0.1:{port}/health");
        assert!(!check_url(&url, Duration::from_millis(200)).await);
        assert!(started.elapsed() < Duration::from_secs(2));
        hold.abort();
    }

    #[tokio::test]
    async fn nothing_listening_is_unhealthy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!health_check(port).await);
    }
}
