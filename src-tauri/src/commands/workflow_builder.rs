//! Workflow builder commands (Tauri adapter for `services::workflow_graph`).

use tauri::State;

pub use crate::services::workflow_graph::WorkflowGraph;

use crate::services::workflow_graph;
use crate::services::workflows::Workflow;
use crate::services::AppState;

/// The workflow's steps as nodes and edges, with the latest run's statuses
#[tauri::command]
#[specta::specta]
pub async fn get_workflow_graph(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<WorkflowGraph, String> {
    workflow_graph::get_workflow_graph(&state, &workflow_id).await
}

/// Saves the workflow's YAML; returns the reloaded workflow
#[tauri::command]
#[specta::specta]
pub async fn save_workflow_yaml(
    state: State<'_, AppState>,
    workflow_id: String,
    yaml: String,
) -> Result<Workflow, String> {
    workflow_graph::save_workflow_yaml(&state, &workflow_id, &yaml).await
}
