//! Harness session commands: history, recorded ACP events, and kill.

use crate::services::harness;
use crate::services::harness::{HarnessEventRecord, HarnessSessionSummary};

super::commands! {
    /// Newest first. `limit`: `None` is 100
    fn list_harness_sessions(state, agent_id: Option<String>, limit: Option<u32>) -> Vec<HarnessSessionSummary> => harness::list_harness_sessions(&state, agent_id, limit).await;

    fn get_harness_session(state, session_id: String) -> HarnessSessionSummary => harness::get_harness_session(&state, &session_id).await;

    /// In order; `after_seq` pages past events already seen
    fn get_harness_events(state, session_id: String, after_seq: Option<u32>) -> Vec<HarnessEventRecord> => harness::get_harness_events(&state, &session_id, after_seq).await;

    /// Stops the CLI (and its container); the run fails
    fn kill_harness_session(state, session_id: String) -> () => harness::kill_harness_session(&state, &session_id).await;
}
