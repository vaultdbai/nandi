//! Re-detects the harness CLIs every five minutes (installs, upgrades,
//! sign-ins happen outside Nandi); `HarnessService::detect` emits
//! `harness://health_changed` for whatever changed.

use std::sync::Weak;
use std::time::Duration;

use super::HarnessService;

pub const INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Runs until the service is dropped (the app shuts down)
pub async fn run(service: Weak<HarnessService>) {
    let mut ticker = tokio::time::interval(INTERVAL);
    // The first tick is immediate; startup already detected
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let Some(service) = service.upgrade() else {
            return;
        };
        if let Err(e) = service.detect().await {
            log::warn!("Harness health check failed: {e}");
        }
    }
}
