//! A workflow as a graph for the builder: one node per step (with its live
//! status from the latest run) and an edge per dependency.
//! Derived from the workflow definition plus run state.
//!
//! Steps run in file order, so each step depends on the one before it
//! (`sequence` edges). A step whose input uses `{{other.output}}` from an
//! earlier, non-adjacent step also gets a `data` edge from that step.
//!
//! Statuses come from the most recent run (`Workflow::runs[0]`), matched by
//! step ID: a step the run hasn't reached is `pending` while the run is live
//! and `skipped` once it has ended; a step added to the file after that run
//! is `pending`.
//!
//! Saving YAML follows the gold-copy rule: the domain parses and validates,
//! the workflow repository writes the file, and `commit` projects it and
//! publishes `workflow://changed`.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::domain::{self, EntityKind, WorkflowId};
use super::workflows::{Workflow, WorkflowRun, WorkflowRunStatus, WorkflowStepStatus};
use super::AppState;
use crate::model::Timestamp;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowGraph {
    pub workflow_id: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// The run the statuses come from, if any
    pub run_id: Option<String>,
    /// That run's status
    pub run_status: Option<WorkflowRunStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    /// The step ID
    pub id: String,
    pub name: String,
    pub agent_id: Option<String>,
    /// `pending` | `running` | `awaitingApproval` | `done` | `failed` | `skipped`
    pub status: String,
    pub requires_approval: bool,
    pub started_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    /// The agent run that executed this step, for "View run"
    pub agent_run_id: Option<String>,
    /// The agent's display name (its ID if unknown)
    pub agent_name: Option<String>,
    /// The step's input template from the file
    pub input_template: String,
    /// The message the run sent (or will send, awaiting approval)
    pub input: Option<String>,
    /// The agent's final reply
    pub output: Option<String>,
    pub error: Option<String>,
    pub critical: bool,
    pub continue_on_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// `sequence` (runs after) | `data` (uses its output)
    pub kind: String,
}

/// `{{ step.output }}`
static OUTPUT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z0-9_-]+)\.output\s*\}\}").unwrap());

fn status_name(status: WorkflowStepStatus, run_ended: bool) -> &'static str {
    match status {
        WorkflowStepStatus::Pending if run_ended => "skipped",
        WorkflowStepStatus::Pending => "pending",
        WorkflowStepStatus::AwaitingApproval => "awaitingApproval",
        WorkflowStepStatus::AwaitingInput => "awaitingInput",
        WorkflowStepStatus::Running => "running",
        WorkflowStepStatus::Done => "done",
        WorkflowStepStatus::Failed | WorkflowStepStatus::Rejected => "failed",
    }
}

/// The graph of `workflow`'s current definition, with statuses from `run`.
pub fn build_graph(
    workflow: &Workflow,
    run: Option<&WorkflowRun>,
    agent_name: impl Fn(&str) -> String,
) -> WorkflowGraph {
    let run_ended = run.is_some_and(|run| {
        matches!(
            run.status,
            WorkflowRunStatus::Completed | WorkflowRunStatus::Failed
        )
    });
    let nodes = workflow
        .steps
        .iter()
        .map(|step| {
            let state = run.and_then(|run| run.steps.iter().find(|s| s.step_id == step.id));
            GraphNode {
                id: step.id.clone(),
                name: step.id.clone(),
                // Manual steps are done by a person and ask steps by a
                // colleague, not a local agent
                agent_id: (!step.manual && step.ask.is_none()).then(|| step.agent.clone()),
                agent_name: match &step.ask {
                    Some(ask) => Some(format!(
                        "Asks {}",
                        ask.find
                            .as_deref()
                            .or(ask.person.as_deref())
                            .or(ask.capability.as_deref())
                            .unwrap_or_default()
                    )),
                    None => (!step.manual).then(|| agent_name(&step.agent)),
                },
                status: state
                    .map_or("pending", |s| status_name(s.status, run_ended))
                    .to_string(),
                requires_approval: step.requires_approval,
                started_at: state.and_then(|s| s.started_at),
                ended_at: state.and_then(|s| s.ended_at),
                agent_run_id: state.and_then(|s| s.agent_run_id.clone()),
                input_template: match (&step.instructions, step.manual, &step.ask) {
                    (Some(instructions), true, _) => instructions.clone(),
                    (_, _, Some(ask)) => ask.input.clone(),
                    _ => step.input.clone(),
                },
                input: state.and_then(|s| s.input.clone()),
                output: state.and_then(|s| s.output.clone()),
                error: state.and_then(|s| s.error.clone()),
                critical: step.critical,
                continue_on_failure: step.continue_on_failure,
            }
        })
        .collect();

    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    for (index, step) in workflow.steps.iter().enumerate() {
        if let Some(previous) = index.checked_sub(1).map(|i| &workflow.steps[i]) {
            seen.insert((previous.id.clone(), step.id.clone()));
            edges.push(GraphEdge {
                from: previous.id.clone(),
                to: step.id.clone(),
                kind: "sequence".to_string(),
            });
        }
        let earlier: HashSet<&str> = workflow.steps[..index]
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        for caps in OUTPUT_REF.captures_iter(&step.input) {
            let from = &caps[1];
            if earlier.contains(from) && seen.insert((from.to_string(), step.id.clone())) {
                edges.push(GraphEdge {
                    from: from.to_string(),
                    to: step.id.clone(),
                    kind: "data".to_string(),
                });
            }
        }
    }

    WorkflowGraph {
        workflow_id: workflow.id.clone(),
        nodes,
        edges,
        run_id: run.map(|run| run.id.clone()),
        run_status: run.map(|run| run.status),
    }
}

pub async fn get_workflow_graph(
    state: &AppState,
    workflow_id: &str,
) -> Result<WorkflowGraph, String> {
    let workflow = state.workflows().get_workflow(workflow_id).await?;
    let store = state.store();
    Ok(build_graph(&workflow, workflow.runs.first(), |agent| {
        store.display_name(agent)
    }))
}

/// Validates and saves the workflow's YAML (through the repo layer); returns
/// the reloaded workflow
pub async fn save_workflow_yaml(
    state: &AppState,
    workflow_id: &str,
    yaml: &str,
) -> Result<Workflow, String> {
    let id = WorkflowId::parse(workflow_id).map_err(|e| e.to_string())?;
    let parsed = domain::Workflow::from_yaml(yaml).map_err(|e| e.to_string())?;
    if parsed.id() != &id {
        return Err(format!(
            "The YAML's id '{}' doesn't match workflow '{id}'; a workflow's id can't be changed here",
            parsed.id()
        ));
    }
    let (store, source) = (state.store().clone(), yaml.to_string());
    tokio::task::spawn_blocking(move || {
        let (_, scope) = store
            .repos()
            .workflows
            .locate(&id)
            .ok_or_else(|| format!("Workflow '{id}' not found"))?;
        let workflow = parsed.in_scope(scope);
        let entity = domain::Entity::Workflow(workflow.clone());
        // Steps may name agents that don't exist yet (as on import): runs
        // check them
        match store.read(|repo| repo.check_references(&entity)) {
            Err(domain::DomainError::DanglingReference { from, to }) => {
                log::warn!("{from} refers to {to}, which doesn't exist yet");
            }
            Err(e) => return Err(e.to_string()),
            Ok(()) => {}
        }
        store.repos().workflows.save_source(&workflow, &source)?;
        store.commit(EntityKind::Workflow, id.as_str())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Workflow write failed: {e}"))??;
    state.workflows().get_workflow(workflow_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::workflows::{WorkflowStep, WorkflowStepRun};

    fn step(id: &str, agent: &str, input: &str) -> WorkflowStep {
        WorkflowStep {
            id: id.to_string(),
            agent: agent.to_string(),
            input: input.to_string(),
            requires_approval: false,
            critical: false,
            continue_on_failure: false,
            manual: false,
            instructions: None,
            deadline_hours: None,
            ask: None,
        }
    }

    fn workflow(steps: Vec<WorkflowStep>) -> Workflow {
        Workflow {
            id: "flow".to_string(),
            name: "Flow".to_string(),
            description: None,
            steps,
            source: String::new(),
            path: String::new(),
            runs: vec![],
        }
    }

    fn step_run(id: &str, status: WorkflowStepStatus) -> WorkflowStepRun {
        WorkflowStepRun {
            step_id: id.to_string(),
            agent_id: "a".to_string(),
            template: String::new(),
            requires_approval: false,
            critical: false,
            continue_on_failure: false,
            approved: false,
            manual: false,
            instructions: None,
            deadline_hours: None,
            deadline: None,
            ask: None,
            ask_target: None,
            ask_capability: None,
            asked: None,
            task_id: None,
            status,
            input: Some(format!("{id} input")),
            output: (status == WorkflowStepStatus::Done).then(|| format!("{id} output")),
            agent_run_id: Some(format!("run-{id}")),
            error: None,
            started_at: Some(1.0),
            ended_at: None,
        }
    }

    fn three_steps() -> Vec<WorkflowStep> {
        let mut publish = step("publish", "publisher", "Send {{research.output}}");
        publish.requires_approval = true;
        vec![
            step("research", "researcher", "Find news"),
            step("summarize", "writer", "Summarize {{research.output}}"),
            publish,
        ]
    }

    fn run(status: WorkflowRunStatus, steps: Vec<WorkflowStepRun>) -> WorkflowRun {
        WorkflowRun {
            id: "r1".to_string(),
            workflow_id: "flow".to_string(),
            status,
            started_at: 0.0,
            ended_at: None,
            steps,
            error: None,
            source: None,
        }
    }

    fn edge(from: &str, to: &str, kind: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn builds_nodes_and_edges_from_the_definition() {
        let graph = build_graph(&workflow(three_steps()), None, |a| format!("Agent {a}"));
        assert_eq!(graph.workflow_id, "flow");
        assert_eq!(graph.run_id, None);
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            vec!["research", "summarize", "publish"]
        );
        assert!(graph.nodes.iter().all(|n| n.status == "pending"));
        let publish = &graph.nodes[2];
        assert!(publish.requires_approval);
        assert_eq!(publish.agent_id.as_deref(), Some("publisher"));
        assert_eq!(publish.agent_name.as_deref(), Some("Agent publisher"));
        assert_eq!(publish.input_template, "Send {{research.output}}");
        // Order edges, plus a data edge that skips a step; the adjacent
        // reference doesn't duplicate its order edge
        assert_eq!(
            graph.edges,
            vec![
                edge("research", "summarize", "sequence"),
                edge("summarize", "publish", "sequence"),
                edge("research", "publish", "data"),
            ]
        );
    }

    #[test]
    fn takes_statuses_from_the_latest_run() {
        let latest = run(
            WorkflowRunStatus::AwaitingApproval,
            vec![
                step_run("research", WorkflowStepStatus::Done),
                step_run("summarize", WorkflowStepStatus::Running),
                step_run("publish", WorkflowStepStatus::AwaitingApproval),
            ],
        );
        let graph = build_graph(&workflow(three_steps()), Some(&latest), str::to_string);
        assert_eq!(graph.run_id.as_deref(), Some("r1"));
        assert_eq!(graph.run_status, Some(WorkflowRunStatus::AwaitingApproval));
        let statuses: Vec<_> = graph.nodes.iter().map(|n| n.status.as_str()).collect();
        assert_eq!(statuses, vec!["done", "running", "awaitingApproval"]);
        assert_eq!(graph.nodes[0].output.as_deref(), Some("research output"));
        assert_eq!(graph.nodes[0].agent_run_id.as_deref(), Some("run-research"));
        assert_eq!(graph.nodes[1].started_at, Some(1.0));
    }

    #[test]
    fn unreached_steps_of_an_ended_run_are_skipped_and_new_steps_pending() {
        let latest = run(
            WorkflowRunStatus::Failed,
            vec![
                step_run("research", WorkflowStepStatus::Rejected),
                step_run("summarize", WorkflowStepStatus::Pending),
            ],
        );
        let graph = build_graph(&workflow(three_steps()), Some(&latest), str::to_string);
        let statuses: Vec<_> = graph.nodes.iter().map(|n| n.status.as_str()).collect();
        assert_eq!(statuses, vec!["failed", "skipped", "pending"]);
    }

    const TWO_STEPS: &str = r#"id: two-step
name: Two Step
steps:
  - id: research
    agent: research-agent
    input: "Find today's AI news"
  - id: summarize
    agent: writer-agent
    input: "Summarize: {{research.output}}"
"#;

    #[tokio::test]
    async fn graph_and_yaml_round_trip_through_the_repo() {
        let state = AppState::for_tests(vec![]);
        state.workflows().create_workflow(TWO_STEPS).await.unwrap();

        let graph = get_workflow_graph(&state, "two-step").await.unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges, vec![edge("research", "summarize", "sequence")]);

        let edited = TWO_STEPS.replace("Find today's AI news", "Find this week's news");
        let saved = save_workflow_yaml(&state, "two-step", &edited)
            .await
            .unwrap();
        assert_eq!(saved.source, edited);
        assert_eq!(saved.steps[0].input, "Find this week's news");
        // The file is the gold copy
        assert_eq!(std::fs::read_to_string(&saved.path).unwrap(), edited);

        // Invalid YAML, a changed id, a loop, and an unknown workflow are
        // refused and leave the file alone
        assert!(save_workflow_yaml(&state, "two-step", "steps: [")
            .await
            .is_err());
        let renamed = save_workflow_yaml(
            &state,
            "two-step",
            &edited.replace("id: two-step", "id: other"),
        )
        .await
        .unwrap_err();
        assert!(renamed.contains("doesn't match"), "{renamed}");
        let looping = edited.replace("Find this week's news", "{{summarize.output}}");
        assert!(save_workflow_yaml(&state, "two-step", &looping)
            .await
            .is_err());
        let missing =
            save_workflow_yaml(&state, "nope", &edited.replace("id: two-step", "id: nope"))
                .await
                .unwrap_err();
        assert!(missing.contains("not found"), "{missing}");
        assert_eq!(std::fs::read_to_string(&saved.path).unwrap(), edited);
        state.shutdown();
    }
}
