//! `plan_set` and `plan_update`: every agent's plan tools, answered on the
//! host. Each change is saved with the run and sent as
//! `run://plan_updated`, so the checklist card updates live.

use serde_json::{json, Value};

use super::{apply_update, link_to_current, parse_steps, store, RunPlan, RunPlanUpdated};
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::services::events::{self, EventBus, RunContext};

pub const SET: &str = "plan_set";
pub const UPDATE: &str = "plan_update";
const TOOLS: [&str; 2] = [SET, UPDATE];

pub fn is_plan_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// The plan tools, offered to every agent outside the email and calendar
/// checks
pub fn specs() -> Vec<ToolSpec> {
    let kinds = json!([
        "navigate", "read", "fill", "submit", "handover", "approval", "delegate", "verify",
        "cleanup", "other"
    ]);
    vec![
        ToolSpec {
            name: SET.to_string(),
            description: "Plan this run before you change anything (browse and click, type, \
submit, start or update work, delegate, write files outside your notes): the steps in order, at a \
high level (open the site, sign in, go to billing, fill in the amount, pay, read the confirmation, \
close the browser). Kinds: `submit` for sending a form, `handover` for what the person does \
themselves (sign-in, payment, posting, anything irreversible), `approval` for what the user \
decides, `delegate` for starting work or asking an agent, `cleanup` for closing the browser. Mark \
`evidence: screenshot` on steps whose result belongs in the log (never on a handover). Calling it \
again replaces the plan (the old one is kept as a revision)."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["steps"],
                "properties": {
                    "steps": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": super::MAX_STEPS,
                        "items": {
                            "type": "object",
                            "required": ["title", "kind"],
                            "properties": {
                                "title": {"type": "string", "maxLength": super::MAX_TITLE_CHARS},
                                "kind": {"type": "string", "enum": kinds},
                                "evidence": {"type": "string", "enum": ["screenshot", "none"]},
                                "note": {"type": "string", "maxLength": super::MAX_TEXT_CHARS}
                            }
                        }
                    }
                }
            }),
        },
        ToolSpec {
            name: UPDATE.to_string(),
            description: "Keep the run's plan current: mark the step you start `doing` (one at a \
time), then `done`, `skipped` or `failed` with a short `result`. Screenshots taken during a step \
are linked to it; add other evidence files with `files`. A risky action (submit, pay, type a \
secret, hand over, start work, delegate) runs only while a step of a kind that covers it is \
`doing`. Before you finish, every step is done or skipped and the browser is closed."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["step"],
                "properties": {
                    "step": {"type": "integer", "minimum": 1, "description": "The step's number, from 1"},
                    "status": {"type": "string", "enum": ["pending", "doing", "done", "skipped", "failed"]},
                    "result": {"type": "string", "maxLength": super::MAX_TEXT_CHARS},
                    "files": {"type": "array", "items": {"type": "string"}, "maxItems": super::MAX_FILES}
                }
            }),
        },
    ]
}

fn emit(bus: &EventBus, ctx: &RunContext, plan: &RunPlan) {
    events::emit(
        bus,
        &RunPlanUpdated {
            agent_id: ctx.agent_id.clone(),
            conversation_id: ctx.conversation_id.clone(),
            run_id: ctx.run_id.clone(),
            plan: plan.clone(),
        },
    );
}

fn error(result: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: result.into(),
        is_error: true,
    }
}

/// Answers a plan tool call for `ctx`'s run
pub async fn call(bus: &EventBus, ctx: &RunContext, tool: &str, args: &Value) -> ToolOutcome {
    let (run_id, tool_name, args) = (ctx.run_id.clone(), tool.to_string(), args.clone());
    let saved = bus
        .db
        .call(move |conn| {
            Ok(match tool_name.as_str() {
                SET => match parse_steps(&args) {
                    Ok(steps) => Ok(store::set(conn, &run_id, steps)?),
                    Err(e) => Err(e),
                },
                _ => match store::current(conn, &run_id)? {
                    None => Err("This run has no plan yet: call plan_set first".to_string()),
                    Some(mut plan) => match apply_update(&mut plan, &args) {
                        Ok(()) => Ok(store::update(conn, &run_id, &plan)?),
                        Err(e) => Err(e),
                    },
                },
            })
        })
        .await;
    match saved {
        Ok(Ok(plan)) => {
            emit(bus, ctx, &plan);
            ToolOutcome {
                result: plan.checklist(),
                is_error: false,
            }
        }
        Ok(Err(refused)) => error(refused),
        Err(e) => error(format!("Couldn't save the plan: {e}")),
    }
}

/// The plan `run_id` works to, if it has one (a broken read counts as none)
pub async fn current(bus: &EventBus, run_id: &str) -> Option<RunPlan> {
    let id = run_id.to_string();
    bus.db
        .call(move |conn| store::current(conn, &id))
        .await
        .unwrap_or_else(|e| {
            log::warn!("Couldn't read the run plan of {run_id}: {e}");
            None
        })
}

/// Links a screenshot the run just took to its current step
pub async fn link_screenshot(bus: &EventBus, ctx: &RunContext, file: &str) {
    let (run_id, file) = (ctx.run_id.clone(), file.to_string());
    let saved = bus
        .db
        .call(move |conn| {
            let Some(mut plan) = store::current(conn, &run_id)? else {
                return Ok(None);
            };
            if !link_to_current(&mut plan, &file) {
                return Ok(None);
            }
            store::update(conn, &run_id, &plan).map(Some)
        })
        .await;
    match saved {
        Ok(Some(plan)) => emit(bus, ctx, &plan),
        Ok(None) => {}
        Err(e) => log::warn!("Couldn't link the screenshot to the run plan: {e}"),
    }
}
