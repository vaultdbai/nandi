//! Workflow builder commands (`services::workflow_graph`).

use crate::services::workflow_graph;
use crate::services::workflow_graph::WorkflowGraph;
use crate::services::workflows::Workflow;

super::commands! {
    /// The workflow's steps as nodes and edges, with the latest run's statuses
    fn get_workflow_graph(state, workflow_id: String) -> WorkflowGraph => workflow_graph::get_workflow_graph(&state, &workflow_id).await;

    /// Saves the workflow's YAML; returns the reloaded workflow
    fn save_workflow_yaml(state, workflow_id: String, yaml: String) -> Workflow => workflow_graph::save_workflow_yaml(&state, &workflow_id, &yaml).await;
}
