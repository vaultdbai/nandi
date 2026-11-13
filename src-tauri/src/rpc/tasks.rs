//! Task lifecycle commands (Tauri adapter for `services::a2a::task`).

use crate::services::a2a::task;
use crate::services::a2a::task::{
    EtaEstimate, TaskEvent, TaskFilter, TaskMessage, TaskRecord, TaskStatusReport,
};

super::commands! {
    fn get_task(state, task_id: String) -> TaskRecord => task::get_task(&state, &task_id).await;

    fn list_tasks(state, filter: Option<TaskFilter>) -> Vec<TaskRecord> => task::list_tasks(&state, filter.unwrap_or_default()).await;

    fn cancel_task(state, task_id: String, reason: Option<String>) -> TaskRecord => task::cancel_task(&state, &task_id, reason).await;

    fn query_task_status(state, task_id: String) -> TaskStatusReport => task::query_task_status(&state, &task_id).await;

    fn get_task_events(state, task_id: String) -> Vec<TaskEvent> => task::get_task_events(&state, &task_id).await;

    fn get_task_messages(state, task_id: String) -> Vec<TaskMessage> => task::get_task_messages(&state, &task_id).await;

    fn get_eta_estimate(state, capability: String, provider_agent: Option<String>) -> EtaEstimate => task::get_eta_estimate(&state, &capability, provider_agent).await;
}
