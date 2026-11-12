//! Work commands (Tauri adapter for `services::work`).

use crate::services::work;
use crate::services::work::{WorkItem, WorkState};

super::commands! {
    /// Workflow runs and tasks, newest first; `state_filter` `None` for all
    fn list_work(state, state_filter: Option<WorkState>) -> Vec<WorkItem> => work::list_work(&state, state_filter).await;

    fn get_work_item(state, id: String) -> WorkItem => work::get_work_item(&state, id).await;

    fn pause_work(state, id: String) -> () => work::pause_work(&state, id).await;

    fn resume_work(state, id: String) -> () => work::resume_work(&state, id).await;

    fn cancel_work(state, id: String) -> () => work::cancel_work(&state, id).await;

    /// Rates a finished delegated task 1–5
    fn rate_work(state, id: String, rating: u32) -> () => work::rate_work(&state, id, rating).await;

    /// Hides a failed item from the Inbox (it stays in history)
    fn dismiss_work(state, id: String) -> () => work::dismiss_work(&state, id).await;

    /// IDs of failures dismissed from the Inbox
    fn list_dismissed_work(state) -> Vec<String> => work::list_dismissed_work(&state).await;
}
