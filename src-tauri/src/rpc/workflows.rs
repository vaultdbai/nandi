//! Workflow commands (`services::workflows`). Listing is
//! `snapshots::get_workflows`.

use crate::model::ApprovalDecision;
use crate::services::workflows::templates::WorkflowTemplate;
use crate::services::workflows::{self, CreateWorkflowInput, TemplateWorkflowInput, Workflow};

super::commands! {
    fn get_workflow(state, id: String) -> Workflow => workflows::get_workflow(&state, id).await;

    /// Saves a YAML workflow as `workflows/{id}.yaml` in the agents repo
    fn create_workflow(state, input: CreateWorkflowInput) -> Workflow => workflows::create_workflow(&state, input).await;

    /// The bundled templates for "New workflow › From a template"
    fn list_workflow_templates() -> Vec<WorkflowTemplate> => Ok(workflows::list_workflow_templates());

    /// Builds a template with the chosen agents and saves it
    fn create_workflow_from_template(state, input: TemplateWorkflowInput) -> Workflow => workflows::create_workflow_from_template(&state, input).await;

    /// Returns the new run's ID
    fn run_workflow(state, id: String) -> String => workflows::run_workflow(&state, id).await;

    /// Resolves a step paused by `requires_approval`
    fn approve_workflow_step(state, run_id: String, step_id: String, decision: ApprovalDecision) -> () => workflows::approve_workflow_step(&state, run_id, step_id, decision).await;
}
