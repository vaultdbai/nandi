//! The decision tools of an Inbox item's thread. Offered only to the Chief
//! of Staff, only in that thread, and only those that fit the item now
//! (`item::load`). Every call waits for the user to confirm it on its
//! approval card ([`gate`]): a sentence in the chat never decides anything
//! by itself. On confirm, [`call`] calls the same service function as the
//! item's button, so the same checks, group policies, typed reasons and
//! audit apply. Conditions travel as the note or reason; they never change
//! a policy or a grant.
//!
//! | Tool             | Arguments             | Calls (by kind)                                                  |
//! | ---------------- | --------------------- | ---------------------------------------------------------------- |
//! | `inbox_approve`  | `{note?, item?}`      | `approve_action` / `access::answer` (once), `approve_workflow_step`, `decide_gateway_hold`, `respond_trust_request`, `retry_task`, `intake::approve` |
//! | `inbox_refuse`   | `{reason?, item?}`    | the same, refusing; `close_gateway_assignment` (declined), `respond_to_clarification` (no), hand-over skipped, `intake::dismiss` |
//! | `inbox_answer`   | `{text}`              | `respond_to_clarification`                                       |
//! | `inbox_done`     | `{note?}`             | `close_gateway_assignment`, `complete_manual_step`, `dismiss_request_return`, `dismiss_work` |
//! | `inbox_reassign` | `{to, reason?}`       | `reassign_task`                                                  |
//! | `knowme_propose` | a `KnowMeProposal`    | `knowme::apply` (NANDI.md, My notes), know-me chat only          |
//!
//! A know-me proposal may be edited on its card before the person confirms
//! it (`knowme::confirm`): the confirmed arguments are the approval's
//! ([`confirmed_args`]), not the model's.
//!
//! `item` picks one of an intake thread's queued items when it has several.

use serde_json::{json, Value};

use super::audit;
use super::item::{self, DecisionTool, InboxItemRef};
use crate::db::PlannedCall;
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::model::ApprovalDecision;
use crate::services::AppState;

pub const APPROVE: &str = "inbox_approve";
pub const REFUSE: &str = "inbox_refuse";
pub const ANSWER: &str = "inbox_answer";
pub const DONE: &str = "inbox_done";
pub const REASSIGN: &str = "inbox_reassign";
pub const PROPOSE: &str = "knowme_propose";
const TOOLS: [&str; 6] = [APPROVE, REFUSE, ANSWER, DONE, REASSIGN, PROPOSE];

/// Longest note, reason or answer, in characters
const MAX_TEXT_CHARS: usize = 2000;

pub fn is_decision_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

pub fn name_of(tool: DecisionTool) -> &'static str {
    match tool {
        DecisionTool::Approve => APPROVE,
        DecisionTool::Refuse => REFUSE,
        DecisionTool::Answer => ANSWER,
        DecisionTool::Done => DONE,
        DecisionTool::Reassign => REASSIGN,
        DecisionTool::Propose => PROPOSE,
    }
}

fn tool_of(name: &str) -> Option<DecisionTool> {
    Some(match name {
        APPROVE => DecisionTool::Approve,
        REFUSE => DecisionTool::Refuse,
        ANSWER => DecisionTool::Answer,
        DONE => DecisionTool::Done,
        REASSIGN => DecisionTool::Reassign,
        PROPOSE => DecisionTool::Propose,
        _ => return None,
    })
}

/// Every decision call waits for the user, whatever else said it needn't
pub fn gate(calls: &mut [PlannedCall]) {
    for call in calls.iter_mut().filter(|c| is_decision_tool(&c.tool)) {
        call.requires_approval = true;
    }
}

/// The arguments a confirmed call runs with: the approval's, which a
/// know-me card may have edited before the person confirmed it; else the
/// model's
pub async fn confirmed_args(state: &AppState, call_id: &str, args: &Value) -> Value {
    let id = call_id.to_string();
    state
        .db()
        .call(move |conn| crate::db::approval_for_call(conn, &id))
        .await
        .ok()
        .flatten()
        .filter(|a| a.status == crate::model::ApprovalStatus::Approved)
        .map_or_else(|| args.clone(), |a| a.args)
}

pub fn spec(tool: DecisionTool, intake: bool) -> ToolSpec {
    if tool == DecisionTool::Propose {
        return crate::services::knowme::proposal::spec();
    }
    let item = json!({"type": "string", "description": "The queued item's id, when this thread has several"});
    let text = json!({"type": "string", "maxLength": MAX_TEXT_CHARS});
    let (description, schema) = match tool {
        DecisionTool::Approve => (
            "Propose approving this Inbox item, as the user decided in this chat. It waits until \
the user confirms it on its card. Put their condition (\"up to $200\", \"skip the email\") in \
`note`: it goes with the approval to the requester or the step.",
            json!({"type": "object", "properties": {"note": text, "item": item}}),
        ),
        DecisionTool::Refuse => (
            "Propose refusing (declining, dismissing) this Inbox item, as the user decided in \
this chat. It waits until the user confirms it on its card. Put why in `reason`, in their words: \
the requester gets it.",
            json!({"type": "object", "properties": {"reason": text, "item": item}}),
        ),
        DecisionTool::Answer => (
            "Propose sending the user's answer to this question. It waits until the user confirms \
it on its card. Write `text` as the user means it, ready to send.",
            json!({"type": "object", "required": ["text"], "properties": {"text": text}}),
        ),
        DecisionTool::Done => (
            "Propose marking this Inbox item done, as the user said. It waits until the user \
confirms it on its card. `note` goes to whoever asked (an assignment's result, a step's output).",
            json!({"type": "object", "properties": {"note": text}}),
        ),
        DecisionTool::Reassign => (
            "Propose handing this request to a teammate who serves the same thing, as the user \
asked. It waits until the user confirms it on its card. `to` is the teammate's name (or instance \
ID); `reason` goes with it.",
            json!({"type": "object", "required": ["to"], "properties": {
                "to": {"type": "string"},
                "reason": text,
            }}),
        ),
        DecisionTool::Propose => unreachable!("specified by knowme::proposal"),
    };
    let mut schema = schema;
    if !intake {
        if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            props.remove("item");
        }
    }
    ToolSpec {
        name: name_of(tool).to_string(),
        description: description.to_string(),
        input_schema: schema,
    }
}

/// The decision tools for a turn in `conversation_id`: none outside an
/// Inbox item's thread, and none once the item no longer waits
pub async fn offered(state: &AppState, conversation_id: &str) -> Vec<ToolSpec> {
    let Some(item) = InboxItemRef::parse(conversation_id) else {
        return Vec::new();
    };
    let intake = matches!(item, InboxItemRef::Intake { .. });
    match item::load(state, &item).await {
        Ok(facts) => facts.tools.into_iter().map(|t| spec(t, intake)).collect(),
        Err(_) => Vec::new(),
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

/// A text argument, trimmed and capped; blank is none
fn text(args: &Value, key: &str) -> Option<String> {
    let text = args.get(key)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.chars().take(MAX_TEXT_CHARS).collect())
}

/// A confirmed decision in `conversation_id`'s thread: the same service
/// call as the item's button, recorded as made in that chat
pub async fn call(
    state: &AppState,
    conversation_id: &str,
    name: &str,
    args: &Value,
) -> ToolOutcome {
    let Some(tool) = tool_of(name) else {
        return error(format!("Unknown tool '{name}'"));
    };
    let Some(item) = InboxItemRef::parse(conversation_id) else {
        return error(format!(
            "'{name}' only works in an Inbox item's chat: open the item first"
        ));
    };
    let facts = match item::load(state, &item).await {
        Ok(facts) => facts,
        Err(e) => return error(e),
    };
    if facts.tools.is_empty() {
        return error("This item no longer waits on the user: nothing was changed");
    }
    if !facts.tools.contains(&tool) {
        return error(format!(
            "'{name}' doesn't fit this item; it takes {}",
            facts
                .tools
                .iter()
                .map(|t| name_of(*t))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let note = text(args, "note").or_else(|| text(args, "reason"));
    let decided = audit::in_chat(
        conversation_id.to_string(),
        note.clone(),
        decide(state, &item, tool, args, note),
    )
    .await;
    match decided {
        Ok(what) => ok(json!({ "decided": what })),
        Err(e) => error(e),
    }
}

/// What a confirmed decision does, by item. Returns what happened, in words.
async fn decide(
    state: &AppState,
    item: &InboxItemRef,
    tool: DecisionTool,
    args: &Value,
    note: Option<String>,
) -> Result<String, String> {
    use DecisionTool::*;
    let approve = tool == Approve;
    match (item, tool) {
        (InboxItemRef::Approval { approval_id }, Approve | Refuse) => {
            decide_approval(state, approval_id, approve).await
        }
        (InboxItemRef::WorkflowApproval { run_id, step_id }, Approve | Refuse) => {
            crate::services::workflows::approve_workflow_step(
                state,
                run_id.clone(),
                step_id.clone(),
                decision(approve),
            )
            .await?;
            Ok(if approve { "approved" } else { "refused" }.to_string())
        }
        (InboxItemRef::GatewayHold { task_id }, Approve | Refuse) => {
            crate::services::gateway::decide_gateway_hold(state, task_id.clone(), approve, note)
                .await?;
            Ok(if approve { "approved" } else { "declined" }.to_string())
        }
        (InboxItemRef::Assignment { task_id }, Done | Refuse) => {
            crate::services::gateway::close_gateway_assignment(
                state,
                task_id.clone(),
                tool == Done,
                note,
            )
            .await?;
            Ok(if tool == Done { "done" } else { "declined" }.to_string())
        }
        (
            InboxItemRef::GatewayHold { task_id } | InboxItemRef::Assignment { task_id },
            Reassign,
        ) => {
            let to = text(args, "to").ok_or("Say who to hand it to")?;
            reassign(state, task_id, &to, text(args, "reason")).await
        }
        (
            InboxItemRef::Clarification {
                task_id,
                question_id,
            },
            Answer | Refuse,
        ) => {
            let answer = if tool == Answer {
                text(args, "text").ok_or("Write the answer to send")?
            } else {
                crate::services::a2a::clarification::DENY_ANSWER.to_string()
            };
            crate::services::a2a::clarification::respond_to_clarification(
                state,
                crate::services::a2a::ClarificationResponse {
                    task_id: task_id.clone(),
                    round: 0,
                    answers: std::collections::HashMap::from([(question_id.clone(), answer)]),
                },
            )
            .await?;
            Ok(if tool == Answer {
                "answered"
            } else {
                "refused"
            }
            .to_string())
        }
        (InboxItemRef::ManualStep { run_id, step_id }, Done) => {
            crate::services::workflows::manual_steps::complete_manual_step(
                state,
                run_id,
                step_id,
                crate::services::workflows::manual_steps::ManualStepCompletion {
                    output: note,
                    attachment_ids: Vec::new(),
                },
            )
            .await?;
            Ok("done".to_string())
        }
        (InboxItemRef::TrustRequest { instance_id }, Approve | Refuse) => {
            use crate::services::team::trust_requests::{respond_trust_request, TrustAnswer};
            let answer = if approve {
                TrustAnswer::Trust
            } else {
                TrustAnswer::Deny
            };
            respond_trust_request(state, instance_id, answer, false).await?;
            Ok(if approve { "trusted" } else { "not now" }.to_string())
        }
        (InboxItemRef::RequestReturn { task_id }, Done) => {
            crate::services::a2a::returns::dismiss_request_return(state, task_id.clone()).await?;
            Ok("dismissed".to_string())
        }
        (InboxItemRef::Failed { work_id }, Approve) => {
            crate::services::requests::retry_task(state, work_id.clone()).await?;
            Ok("retried".to_string())
        }
        (InboxItemRef::KnowMe, Propose) => crate::services::knowme::apply(state, args).await,
        (InboxItemRef::Failed { work_id }, Done) => {
            crate::services::work::dismiss_work(state, work_id.clone()).await?;
            Ok("dismissed".to_string())
        }
        (InboxItemRef::Intake { conversation_id }, Approve | Refuse) => {
            let id = queued_item(state, conversation_id, text(args, "item")).await?;
            if approve {
                crate::services::intake::approve(state, id).await?;
                Ok("approved".to_string())
            } else {
                crate::services::intake::dismiss(state, id).await?;
                Ok("dismissed".to_string())
            }
        }
        _ => Err(format!("'{}' doesn't fit this item", name_of(tool))),
    }
}

fn decision(approve: bool) -> ApprovalDecision {
    if approve {
        ApprovalDecision::Approved
    } else {
        ApprovalDecision::Rejected
    }
}

/// A tool approval, as its card decides it: an access request is allowed
/// once (a chat never grants lasting access); a hand-over is skipped
async fn decide_approval(
    state: &AppState,
    approval_id: &str,
    approve: bool,
) -> Result<String, String> {
    use crate::services::access::{self, AccessChoice};
    use crate::services::pilots::handover::{HandOverAnswer, HandOverOutcome};
    let id = approval_id.to_string();
    let approval = state
        .db()
        .call(move |conn| crate::db::get_approval(conn, &id))
        .await?
        .ok_or("That approval wasn't found")?;
    if approval.tool == access::ACCESS_TOOL {
        let choice = if approve {
            AccessChoice::AllowOnce
        } else {
            AccessChoice::Deny
        };
        let who = crate::services::chat::decider(state);
        access::answer(state, approval_id, choice, false, who).await?;
    } else if crate::services::pilots::handover::is_hand_over(&approval.tool) {
        if approve {
            return Err("A hand-over is finished in the browser, then marked done there".into());
        }
        crate::services::chat::answer_hand_over(
            state,
            approval_id.to_string(),
            HandOverAnswer {
                outcome: HandOverOutcome::Skipped,
                final_text: None,
            },
        )
        .await?;
    } else {
        crate::services::chat::approve_action(
            state,
            None,
            approval_id.to_string(),
            decision(approve),
        )
        .await?;
    }
    Ok(if approve { "approved" } else { "refused" }.to_string())
}

/// The teammate `to` names among those who can take the task: their
/// instance ID, or their name (whole, or the start of it, when only one
/// matches)
fn pick<'a>(
    candidates: &'a [crate::services::a2a::reassign::ReassignCandidate],
    to: &str,
) -> Result<&'a crate::services::a2a::reassign::ReassignCandidate, String> {
    let wanted = to.trim().to_lowercase();
    if let Some(exact) = candidates
        .iter()
        .find(|c| c.instance_id.to_lowercase() == wanted || c.display_name.to_lowercase() == wanted)
    {
        return Ok(exact);
    }
    let starts: Vec<_> = candidates
        .iter()
        .filter(|c| c.display_name.to_lowercase().starts_with(&wanted))
        .collect();
    match starts.as_slice() {
        [one] => Ok(one),
        _ if candidates.is_empty() => {
            Err("Nobody online serves this right now, so it can't be handed on".to_string())
        }
        _ => Err(format!(
            "Who is '{to}'? It can go to: {}",
            candidates
                .iter()
                .map(|c| c.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

async fn reassign(
    state: &AppState,
    task_id: &str,
    to: &str,
    reason: Option<String>,
) -> Result<String, String> {
    use crate::services::a2a::reassign::{reassign_candidates, reassign_task, ReassignRequest};
    let candidates = reassign_candidates(state, task_id).await?;
    let candidate = pick(&candidates, to)?;
    reassign_task(
        state,
        ReassignRequest {
            task_id: task_id.to_string(),
            new_provider_instance: candidate.instance_id.clone(),
            reason,
        },
    )
    .await?;
    Ok(format!("handed to {}", candidate.display_name))
}

/// The queued item an intake decision acts on: the one named, else the
/// only one waiting
async fn queued_item(
    state: &AppState,
    conversation_id: &str,
    named: Option<String>,
) -> Result<String, String> {
    let id = conversation_id.to_string();
    let items = state
        .db()
        .call(move |conn| crate::services::intake::for_thread(conn, &id))
        .await?;
    let pending: Vec<_> = items
        .iter()
        .filter(|i| i.status == crate::services::intake::IntakeStatus::Pending)
        .collect();
    match (named, pending.as_slice()) {
        (Some(named), _) => pending
            .iter()
            .find(|i| i.id == named)
            .map(|i| i.id.clone())
            .ok_or_else(|| format!("'{named}' isn't waiting in this thread")),
        (None, [one]) => Ok(one.id.clone()),
        (None, []) => Err("Nothing waits for approval in this thread".to_string()),
        (None, _) => Err(format!(
            "Several items wait here; name one in `item`: {}",
            pending
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests;
