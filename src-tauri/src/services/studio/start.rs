//! `nandi_start_request`: Studio chat starts work on Ask's route, so "run
//! the weekly report" or "do this for me" become a Request, with a plan
//! the user approves first.
//!
//! Two calls. Without `plan`, the tool runs `ask::planner::plan` over a
//! fresh `GatewayContext` and returns the plan, changing nothing. With the
//! plan, it starts the Request (`requests::local::start_request`). Before
//! that second call runs, [`gate`] replaces the model's plan with a checked
//! one (steps naming anything this Nandi doesn't know are dropped;
//! durations, costs and warnings come from the context, never the model)
//! and asks for approval when a step acts (like Ask's `planNeedsApproval`).
//! The approval card shows that checked plan.
//!
//! Running an existing workflow is a one-step plan (`workflow` → its ID),
//! so there is no separate run-workflow tool.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::PlannedCall;
use crate::engine::ToolOutcome;
use crate::services::ask::context::{self, GatewayContext};
use crate::services::ask::{planner, ActionPlan, AskContext, ModelAskLlm, PlanStep, PlanStepKind};
use crate::services::AppState;

use super::prompt::REQUEST_LINK_PREFIX;

pub const START_REQUEST: &str = "nandi_start_request";

/// What Studio's Requests are planned against: all of this Nandi, as on
/// Studio's page
async fn gateway_context(state: &AppState) -> GatewayContext {
    context::load(
        state,
        &AskContext {
            current_view: Some("studio".to_string()),
            focused_artifact: None,
            recent_runs: Vec::new(),
        },
    )
    .await
}

/// Whether the user sees the plan before it runs: any step that acts
/// (an agent, a workflow, a delegation). People and manual steps don't.
pub fn needs_approval(plan: &ActionPlan) -> bool {
    plan.steps.iter().any(|step| {
        matches!(
            step.kind,
            PlanStepKind::LocalAgent | PlanStepKind::Workflow | PlanStepKind::Capability
        )
    })
}

/// The model's plan, checked against `context`: unknown targets dropped,
/// estimates recomputed
pub fn check(plan: ActionPlan, context: &GatewayContext) -> ActionPlan {
    let steps = plan
        .steps
        .into_iter()
        .filter(|step| planner::known(step.kind, &step.target, context))
        .collect();
    planner::estimate(steps, context)
}

/// The plan as the model passes it: only the steps count (estimates are
/// recomputed), and a step's description may be left out
#[derive(Deserialize)]
struct PlanArg {
    #[serde(default)]
    steps: Vec<StepArg>,
}

#[derive(Deserialize)]
struct StepArg {
    kind: PlanStepKind,
    target: String,
    #[serde(default)]
    description: String,
}

/// `None` without a plan; an unreadable plan is an error, never "no plan"
fn plan_arg(args: &Value) -> Option<Result<ActionPlan, String>> {
    let plan = args.get("plan").filter(|plan| !plan.is_null())?;
    Some(
        serde_json::from_value::<PlanArg>(plan.clone())
            .map(|plan| ActionPlan {
                steps: plan
                    .steps
                    .into_iter()
                    .map(|step| PlanStep {
                        kind: step.kind,
                        description: if step.description.trim().is_empty() {
                            step.target.clone()
                        } else {
                            step.description
                        },
                        target: step.target,
                    })
                    .collect(),
                ..ActionPlan::default()
            })
            .map_err(|e| format!("Unreadable plan: {e}. Pass the plan the first call returned.")),
    )
}

/// Studio's planned `nandi_start_request` calls that carry a plan: the plan
/// is checked in place, and the call waits for approval when it acts.
/// Without the app (orchestrator-only tests) every such call waits.
pub async fn gate(state: Option<&AppState>, calls: &mut [PlannedCall]) {
    for call in calls.iter_mut().filter(|c| c.tool == START_REQUEST) {
        let Some(plan) = plan_arg(&call.args) else {
            continue;
        };
        // Unreadable: the call fails without starting anything; waiting
        // keeps that true even if it didn't
        let (Some(state), Ok(plan)) = (state, plan) else {
            call.requires_approval = true;
            continue;
        };
        let checked = check(plan, &gateway_context(state).await);
        call.requires_approval |= needs_approval(&checked);
        if let Some(args) = call.args.as_object_mut() {
            args.insert(
                "plan".to_string(),
                serde_json::to_value(&checked).unwrap_or_default(),
            );
        }
    }
}

fn ok(value: Value) -> ToolOutcome {
    ToolOutcome {
        result: value.to_string(),
        is_error: false,
    }
}

fn error(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: message.into(),
        is_error: true,
    }
}

/// Runs `nandi_start_request` (after [`gate`], and after approval when it
/// asked for one)
pub async fn call(state: &AppState, args: &Value) -> ToolOutcome {
    let Some(text) = args
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return error("text is required: the user's ask, self-contained");
    };
    let context = gateway_context(state).await;

    let Some(plan) = plan_arg(args) else {
        let plan = match state.ask() {
            Some(ask) => planner::plan(ask.llm().as_ref(), text, &context).await,
            None => {
                let llm = ModelAskLlm::new(state.secrets().clone(), state.store().clone());
                planner::plan(&llm, text, &context).await
            }
        };
        if let Some(rejection) = &plan.rejection {
            return ok(json!({ "rejection": rejection }));
        }
        return ok(json!({
            "plan": plan,
            "needsApproval": needs_approval(&plan),
            "next": format!(
                "Tell the user the plan in a sentence, then call {START_REQUEST} again with the same text and this plan to start it."
            ),
        }));
    };

    let plan = match plan {
        Ok(plan) => plan,
        Err(e) => return error(e),
    };
    // Checked again: a plan that skipped `gate` still can't name unknowns
    let plan = check(plan, &context);
    if plan.steps.is_empty() {
        return error(format!(
            "None of the plan's steps name an agent, workflow, capability or person this Nandi knows. Call {START_REQUEST} without a plan to get one."
        ));
    }
    match crate::services::requests::local::start_request(state, plan, text.to_string()).await {
        Ok(id) => ok(json!({
            "requestId": id,
            "link": format!("{REQUEST_LINK_PREFIX}{id}"),
        })),
        Err(e) => error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::ask::context::{AgentBrief, ContextSources, ViewScope, WorkflowBrief};

    fn step(kind: PlanStepKind, target: &str) -> PlanStep {
        PlanStep {
            kind,
            target: target.to_string(),
            description: format!("do {target}"),
        }
    }

    #[test]
    fn unknown_targets_are_dropped_and_estimates_recomputed() {
        let context = context::build(
            ContextSources {
                agents: vec![AgentBrief {
                    id: "writer".into(),
                    name: "Writer".into(),
                    description: String::new(),
                }],
                workflows: vec![WorkflowBrief {
                    id: "weekly".into(),
                    name: "Weekly".into(),
                    steps: vec!["writer".into()],
                }],
                ..Default::default()
            },
            ViewScope::parse(None),
            None,
            &[],
        );
        let plan = ActionPlan {
            steps: vec![
                step(PlanStepKind::Workflow, "weekly"),
                step(PlanStepKind::LocalAgent, "made-up"),
                step(PlanStepKind::LocalAgent, "writer"),
            ],
            // The model's numbers are never kept
            estimated_cost_usd: Some(999.0),
            ..Default::default()
        };
        let checked = check(plan, &context);
        let targets: Vec<&str> = checked.steps.iter().map(|s| s.target.as_str()).collect();
        assert_eq!(targets, ["weekly", "writer"]);
        assert_ne!(checked.estimated_cost_usd, Some(999.0));
        assert!(needs_approval(&checked));
    }

    #[test]
    fn plans_for_people_or_manual_steps_need_no_approval() {
        let plan = ActionPlan {
            steps: vec![step(PlanStepKind::Manual, "sign the form")],
            ..Default::default()
        };
        assert!(!needs_approval(&plan));
    }

    #[tokio::test]
    async fn without_the_app_a_call_with_a_plan_always_waits() {
        let mut calls = vec![
            PlannedCall {
                id: "1".into(),
                tool: START_REQUEST.into(),
                args: json!({ "text": "go", "plan": { "steps": [] } }),
                requires_approval: false,
            },
            PlannedCall {
                id: "2".into(),
                tool: START_REQUEST.into(),
                args: json!({ "text": "go" }),
                requires_approval: false,
            },
        ];
        gate(None, &mut calls).await;
        assert!(calls[0].requires_approval);
        assert!(!calls[1].requires_approval, "planning changes nothing");
    }
}
