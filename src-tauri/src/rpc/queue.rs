//! Queue commands (Tauri adapter for `services::queue`).

use crate::model::{QueueEntry, RunSummary};
use crate::services::queue;

super::commands! {
    fn get_agent_queue(state, agent_id: String) -> Vec<QueueEntry> => queue::get_agent_queue(&state, agent_id).await;

    fn get_current_turn(state, agent_id: String) -> Option<RunSummary> => queue::get_current_turn(&state, agent_id).await;

    fn cancel_queued_run(state, run_id: String) -> () => queue::cancel_queued_run(&state, run_id).await;

    fn reprioritize_queued_run(state, run_id: String, new_priority: i32) -> () => queue::reprioritize_queued_run(&state, run_id, new_priority).await;
}
