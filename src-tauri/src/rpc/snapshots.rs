//! Snapshot commands (`services::snapshots`). Opening a run's file is
//! desktop-only: `desktop::run_files`.

use crate::model::{StudioConversationSummary, WorkflowSummary};
use crate::services::run_plan::RunPlan;
use crate::services::snapshots::{
    self, AgentSnapshot, ConversationSnapshot, DashboardSnapshot, RunDetail,
};

super::commands! {
    fn get_agent_snapshot(state, agent_id: String) -> AgentSnapshot => snapshots::get_agent_snapshot(&state, agent_id).await;

    fn get_conversation(state, conversation_id: String) -> ConversationSnapshot => snapshots::get_conversation(&state, conversation_id).await;

    fn get_studio_conversations(state) -> Vec<StudioConversationSummary> => snapshots::get_studio_conversations(&state).await;

    fn get_workflows(state) -> Vec<WorkflowSummary> => snapshots::get_workflows(&state).await;

    fn get_run_detail(state, run_id: String) -> RunDetail => snapshots::get_run_detail(&state, run_id).await;

    /// The run's plan (its checklist card), if it has one
    fn get_run_plan(state, run_id: String) -> Option<RunPlan> => snapshots::get_run_plan(&state, run_id).await;

    /// One of the run's screenshots as a `data:` URL (a plan card thumbnail)
    fn get_run_screenshot(state, run_id: String, path: String) -> Option<String> => snapshots::run_screenshot(&state, run_id, path).await;

    /// The host path of a file a run touched, if it exists
    fn resolve_run_file(state, run_id: String, path: String) -> Option<String> => snapshots::run_file_path(&state, run_id, path).await;

    /// `day_start`: local midnight (ms), for "Runs Today"
    fn get_dashboard_snapshot(state, day_start: f64) -> DashboardSnapshot => snapshots::get_dashboard_snapshot(&state, day_start).await;
}
