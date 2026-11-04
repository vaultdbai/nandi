//! Workflow commands (Tauri adapter for `services::workflows`). Listing is
//! `snapshots::get_workflows`.

use tauri::State;

use crate::model::ApprovalDecision;
use crate::services::workflows::{self, CreateWorkflowInput, Workflow};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_workflow(state: State<'_, AppState>, id: String) -> Result<Workflow, String> {
    workflows::get_workflow(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_workflow(
    state: State<'_, AppState>,
    input: CreateWorkflowInput,
) -> Result<Workflow, String> {
    workflows::create_workflow(&state, input).await
}

/// The bundled templates for "New workflow › From a template"
#[tauri::command]
#[specta::specta]
pub async fn list_workflow_templates() -> Result<Vec<workflows::templates::WorkflowTemplate>, String>
{
    Ok(workflows::list_workflow_templates())
}

#[tauri::command]
#[specta::specta]
pub async fn create_workflow_from_template(
    state: State<'_, AppState>,
    input: workflows::TemplateWorkflowInput,
) -> Result<Workflow, String> {
    workflows::create_workflow_from_template(&state, input).await
}

/// Returns the new run's ID.
#[tauri::command]
#[specta::specta]
pub async fn run_workflow(state: State<'_, AppState>, id: String) -> Result<String, String> {
    workflows::run_workflow(&state, id).await
}

#[tauri::command]
#[specta::specta]
pub async fn approve_workflow_step(
    state: State<'_, AppState>,
    run_id: String,
    step_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    workflows::approve_workflow_step(&state, run_id, step_id, decision).await
}
