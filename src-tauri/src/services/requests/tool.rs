//! The Chief of Staff's work tools: it gets things done from what arrives
//! (new mail, calendar changes, a chat) without the user asking. Run on
//! the host, offered only to the Chief of Staff, in chats and in the email
//! and calendar checks' runs.
//!
//! | Tool          | Arguments                                  | Does                                             |
//! | ------------- | ------------------------------------------ | ------------------------------------------------ |
//! | `work_open`   | `{}`                                       | work in progress, and what awaits the user's OK  |
//! | `work_start`  | `{what, source?, message_id?}`             | plans it (agents, workflows, people) and runs it |
//! | `work_update` | `{request_id, news, source?, message_id?}` | adds news to work in progress                    |
//!
//! From mail, the calendar or any other outside source ([`from_outside`]),
//! in an intake thread, or from a run doing a Chief of Staff step of a plan
//! (`self_step`), `work_start` and `work_update` don't act: the item is
//! screened and queued for the user's approval (`services::intake`). In the
//! user's own chat they act now. A plan may give the Chief of Staff one
//! step of its own (`self_step`). Work is a local Request (`requests::local`): it shows
//! on Requests and in "Since you were away" (`requests::digest`). Each
//! step's agent keeps its own approvals.

use serde_json::{json, Value};

use super::local::{self, RequestSource};
use super::self_step;
use super::StepStatus;
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::model::SourceType;
use crate::services::ask::{self, ActionPlan, AskContext, PlanStep, PlanStepKind};
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::intake;
use crate::services::AppState;

pub const OPEN: &str = "work_open";
pub const START: &str = "work_start";
pub const UPDATE: &str = "work_update";
const TOOLS: [&str; 3] = [OPEN, START, UPDATE];

/// Longest `what` or `news`, in characters
const MAX_TEXT_CHARS: usize = 2000;

pub fn is_work_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// The work tools, for the Chief of Staff only
pub fn offered(agent_id: &str) -> Vec<ToolSpec> {
    if agent_id != CHIEF_OF_STAFF {
        return Vec::new();
    }
    TOOLS.iter().filter_map(|name| spec(name)).collect()
}

/// Who started the work, from the run the call came from
pub fn source_of(run: Option<SourceType>) -> RequestSource {
    match run {
        Some(SourceType::Email) => RequestSource::Email,
        Some(SourceType::CalendarCheck) => RequestSource::Calendar,
        _ => RequestSource::ChiefOfStaff,
    }
}

fn spec(name: &str) -> Option<ToolSpec> {
    let (description, schema) = match name {
        OPEN => (
            "List work in progress (Requests: id, title, who started it, status, steps) and what \
already awaits the user's approval. Call it before work_start, so news about work already going \
goes to work_update, and nothing is queued twice.",
            json!({"type": "object", "properties": {}}),
        ),
        START => (
            "Start a piece of work for the user: it is planned onto their agents, workflows or \
teammates, or one step you do yourself (it runs in the work's own thread, not this chat). From mail or the calendar it is screened and waits for the user's approval; in the \
user's chat it runs now. Write `what` in your own words as a request that stands on its own \
(what, for whom, by when, links). Put where you learned it in `source` \
(e.g. \"Lena Park, 2026-11-03, 'Q3 evidence'\") and the email's id in `message_id`, so the email \
is screened too. Each agent still asks the user before it sends, pays or posts anything.",
            json!({"type": "object", "required": ["what"], "properties": {
                "what": {"type": "string", "maxLength": MAX_TEXT_CHARS},
                "source": {"type": "string"},
                "message_id": {"type": "string"},
            }}),
        ),
        UPDATE => (
            "Add news to work in progress (an id from work_open; from mail or the calendar it \
waits for the user's approval first): the agent working on it now gets it in its chat, and the steps still to come get it with their input. Write `news` in your \
own words.",
            json!({"type": "object", "required": ["request_id", "news"], "properties": {
                "request_id": {"type": "string"},
                "news": {"type": "string", "maxLength": MAX_TEXT_CHARS},
                "source": {"type": "string"},
                "message_id": {"type": "string"},
            }}),
        ),
        _ => return None,
    };
    Some(ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schema,
    })
}

fn error(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: message.into(),
        is_error: true,
    }
}

fn ok(value: Value) -> ToolOutcome {
    ToolOutcome {
        result: value.to_string(),
        is_error: false,
    }
}

/// A string argument, trimmed and capped
fn text_arg(args: &Value, key: &str) -> String {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    text.chars().take(MAX_TEXT_CHARS).collect()
}

/// `what` with where it came from, as the Request's input. Pure, for tests.
pub fn work_text(what: &str, source: &str) -> String {
    if source.is_empty() {
        what.to_string()
    } else {
        format!("{what}\n\n(From {source})")
    }
}

/// The plan with at most one Chief of Staff step (`self_step::keep_one`);
/// nothing fitting becomes one step for the user, so it still shows up as
/// theirs to do. Pure, for tests.
pub fn for_the_user(plan: ActionPlan, title: &str) -> ActionPlan {
    let mut plan = self_step::keep_one(plan);
    if plan.steps.is_empty() {
        let why = plan.rejection.take();
        plan = ActionPlan {
            steps: vec![PlanStep {
                kind: PlanStepKind::Manual,
                target: String::new(),
                description: format!("For you: {title}"),
            }],
            warnings: why.into_iter().collect(),
            ..ActionPlan::default()
        };
    }
    plan
}

/// Runs a work tool. `thread`: the call came from an intake thread;
/// `in_self_step`: from a run doing a Chief of Staff step of a plan.
/// Either way, whatever it starts or adds is queued for approval too.
pub async fn call(
    state: &AppState,
    source: RequestSource,
    thread: Option<String>,
    in_self_step: bool,
    tool: &str,
    args: &Value,
) -> ToolOutcome {
    let queued = self_step::must_queue(source, thread.is_some(), in_self_step);
    match tool {
        OPEN => open(state).await,
        START => {
            let what = text_arg(args, "what");
            if what.is_empty() {
                return error("'what' is required");
            }
            let text = work_text(&what, &text_arg(args, "source"));
            if queued {
                return queue(state, source, thread, None, text, args).await;
            }
            match start_now(state, source, &text).await {
                Ok((id, plan)) => ok(started(&id, &plan)),
                Err(e) => error(e),
            }
        }
        UPDATE => {
            let id = text_arg(args, "request_id");
            let news = text_arg(args, "news");
            if id.is_empty() || news.is_empty() {
                return error("'request_id' and 'news' are required");
            }
            let news = work_text(&news, &text_arg(args, "source"));
            if queued {
                return queue(state, source, thread, Some(id), news, args).await;
            }
            match local::add_update(state, id.clone(), news, source).await {
                Ok(told) => ok(json!({ "request_id": id, "told_now": told })),
                Err(e) => error(e),
            }
        }
        _ => error(format!("Unknown work tool '{tool}'")),
    }
}

/// Mail, calendar and any source other than the user's own chat: screened
/// and queued for the user's approval (`services::intake`). Pure.
pub fn from_outside(source: RequestSource) -> bool {
    !matches!(source, RequestSource::Me | RequestSource::ChiefOfStaff)
}

async fn queue(
    state: &AppState,
    source: RequestSource,
    thread_id: Option<String>,
    request_id: Option<String>,
    text: String,
    args: &Value,
) -> ToolOutcome {
    let message_id = Some(text_arg(args, "message_id")).filter(|id| !id.is_empty());
    let proposal = intake::Proposal {
        source,
        request_id,
        text,
        message_id,
        thread_id,
    };
    match intake::propose(state, proposal).await {
        Ok(item) => ok(json!({
            "queued": item.id,
            "screening": item.screening.verdict,
            "reasons": item.screening.reasons,
            "note": "Waiting for the user's approval on their Home. Don't queue it again.",
        })),
        Err(e) => error(e),
    }
}

async fn open(state: &AppState) -> ToolOutcome {
    let found = state
        .db()
        .call(|conn| {
            let mut out = Vec::new();
            for id in local::open_ids(conn)? {
                if let Some(request) = local::load(conn, &id)? {
                    out.push(request);
                }
            }
            Ok((out, intake::pending(conn)?))
        })
        .await;
    match found {
        Ok((requests, queued)) => ok(json!({
            "awaiting_approval": queued.iter().map(|q| json!({
                "queued": q.id,
                "action": q.action,
                "request_id": q.request_id,
                "text": q.text,
            })).collect::<Vec<_>>(),
            "open": requests.iter().map(|r| json!({
                "id": r.id,
                "title": r.title,
                "started_by": r.source,
                "status": r.status,
                "steps": r.steps.iter().map(|s| json!({
                    "label": s.label,
                    "kind": s.kind,
                    "target": s.target,
                    "status": s.status,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })),
        Err(e) => error(e),
    }
}

/// Plans `text` and starts it now. Returns the Request's ID and its plan.
pub async fn start_now(
    state: &AppState,
    source: RequestSource,
    text: &str,
) -> Result<(String, ActionPlan), String> {
    let plan = plan_for(state, text).await?;
    let id = local::start_request_from(state, plan.clone(), text.to_string(), source).await?;
    Ok((id, plan))
}

/// Who would do `text`: the user's agents, workflows or teammates, the
/// Chief of Staff itself for one step at most, else a step for the user
pub async fn plan_for(state: &AppState, text: &str) -> Result<ActionPlan, String> {
    let service = state.ask();
    let context = ask::context::load(
        state,
        &AskContext {
            current_view: Some("work".to_string()),
            ..AskContext::default()
        },
    )
    .await;
    let plan = ask::planner::plan(service.llm().as_ref(), text, &context).await;
    Ok(for_the_user(plan, &local::title_for(text)))
}

fn started(id: &str, plan: &ActionPlan) -> Value {
    let waits_on_user = plan
        .steps
        .iter()
        .all(|s| matches!(s.kind, PlanStepKind::Manual | PlanStepKind::Human));
    let steps: Vec<Value> = plan
        .steps
        .iter()
        .map(|s| json!({ "kind": s.kind, "target": s.target, "label": s.description }))
        .collect();
    json!({
        "request_id": id,
        "steps": steps,
        "status": if waits_on_user { StepStatus::Waiting } else { StepStatus::Running },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: PlanStepKind, target: &str) -> PlanStep {
        PlanStep {
            kind,
            target: target.to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn only_the_chief_of_staff_gets_the_tools() {
        assert_eq!(offered(CHIEF_OF_STAFF).len(), 3);
        assert!(offered("writer").is_empty());
        assert!(TOOLS.iter().all(|t| is_work_tool(t)));
        assert!(!is_work_tool("mail_list_new"));
    }

    #[test]
    fn work_from_outside_waits_for_the_user() {
        assert!(from_outside(RequestSource::Email));
        assert!(from_outside(RequestSource::Calendar));
        assert!(!from_outside(RequestSource::ChiefOfStaff));
        assert!(!from_outside(RequestSource::Me));
    }

    #[test]
    fn the_run_says_who_started_the_work() {
        assert_eq!(source_of(Some(SourceType::Email)), RequestSource::Email);
        assert_eq!(
            source_of(Some(SourceType::CalendarCheck)),
            RequestSource::Calendar
        );
        assert_eq!(source_of(None), RequestSource::ChiefOfStaff);
    }

    #[test]
    fn the_input_says_where_it_came_from() {
        assert_eq!(work_text("Pay it", ""), "Pay it");
        assert_eq!(
            work_text("Pay the power bill", "Acme Energy, 2026-11-05"),
            "Pay the power bill\n\n(From Acme Energy, 2026-11-05)"
        );
    }

    #[test]
    fn a_plan_keeps_one_chief_of_staff_step() {
        let plan = ActionPlan {
            steps: vec![
                step(PlanStepKind::LocalAgent, CHIEF_OF_STAFF),
                step(PlanStepKind::LocalAgent, "social-manager"),
                step(PlanStepKind::LocalAgent, CHIEF_OF_STAFF),
            ],
            ..ActionPlan::default()
        };
        let plan = for_the_user(plan, "Post for Sam's birthday");
        let targets: Vec<&str> = plan.steps.iter().map(|s| s.target.as_str()).collect();
        assert_eq!(targets, vec![CHIEF_OF_STAFF, "social-manager"]);
    }

    #[test]
    fn nothing_fitting_becomes_a_step_for_the_user() {
        let plan = ActionPlan {
            rejection: Some("Nothing offers this".to_string()),
            ..ActionPlan::default()
        };
        let plan = for_the_user(plan, "Pay the power bill");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].kind, PlanStepKind::Manual);
        assert_eq!(plan.steps[0].description, "For you: Pay the power bill");
        assert_eq!(plan.rejection, None);
        assert_eq!(plan.warnings, vec!["Nothing offers this".to_string()]);
    }
}
