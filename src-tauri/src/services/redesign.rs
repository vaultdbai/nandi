//! Wave 2B redesign wiring: run analysis, the companion, and the command
//! palette.
//!
//! FROZEN. Each track fills in its own module's `init`; `AppState` calls
//! `init` here once the core is up (after `network::init`). An `init`
//! returns `None` until its track lands, and must not block.

use super::{companion, palette, run_analysis, AppState};

/// What a stubbed command or route returns until its track lands
pub const NOT_IMPLEMENTED: &str = "not implemented";

pub(super) fn init(state: &AppState) {
    if let Some(analyzer) = run_analysis::init(state) {
        state.set_analysis(analyzer);
    }
    if let Some(companion) = companion::init(state) {
        state.set_companion(companion);
    }
    if let Some(palette) = palette::init(state) {
        state.set_palette(palette);
    }
}

#[cfg(test)]
mod tests {
    use crate::services::AppState;

    #[tokio::test]
    async fn init_runs_without_blocking() {
        // Each slot is `None` until its track lands; either way, starting
        // must not need the network or a window
        let state = AppState::for_tests(vec![]);
        let _ = (
            state.analysis().is_some(),
            state.companion().is_some(),
            state.palette().is_some(),
        );
        state.shutdown();
    }
}
