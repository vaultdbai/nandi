//! Harness commands (Tauri adapter for `services::harness`): detection,
//! health, the end-to-end test, and the default harness.

use crate::services::harness;
use crate::services::harness::{HarnessDefault, HarnessHealth, HarnessInfo, HarnessTestResult};

super::commands! {
    /// Every known harness with its last detected health
    fn list_harnesses(state) -> Vec<HarnessInfo> => harness::list_harnesses(&state).await;

    /// Probes the machine again (PATH, `--version`, config folders)
    fn detect_harnesses(state) -> Vec<HarnessInfo> => harness::detect_harnesses(&state).await;

    fn get_harness_health(state, harness_id: String) -> HarnessHealth => harness::get_harness_health(&state, &harness_id).await;

    /// Starts the CLI over ACP, opens a session, and closes it
    fn test_harness(state, harness_id: String) -> HarnessTestResult => harness::test_harness(&state, &harness_id).await;

    fn set_harness_default(state, harness_id: String, model: Option<String>) -> HarnessDefault => harness::set_harness_default(&state, &harness_id, model).await;

    fn clear_harness_default(state) -> () => harness::clear_harness_default(&state).await;

    fn get_harness_default(state) -> Option<HarnessDefault> => harness::get_harness_default(&state).await;
}
