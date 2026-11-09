//! Which Inbox item a thread is about ([`InboxItemRef`], in its
//! conversation ID), and what the Chief of Staff is told about it
//! ([`ItemFacts`]): what it is, what is being decided, and which decision
//! tools fit it right now.
//!
//! | Kind               | ID                     | Tools                      |
//! | ------------------ | ---------------------- | -------------------------- |
//! | `approval`         | approval ID            | approve, refuse            |
//! | `workflowApproval` | `<run>/<step>`         | approve, refuse            |
//! | `gatewayHold`      | gateway task ID        | approve, refuse, reassign  |
//! | `assignment`       | gateway task ID        | done, refuse, reassign     |
//! | `clarification`    | `<task>/<question>`    | answer, refuse             |
//! | `manualStep`       | `<run>/<step>`         | done                       |
//! | `trustRequest`     | their instance ID      | approve, refuse            |
//! | `requestReturn`    | task ID                | done                       |
//! | `failed`           | work item ID           | approve (retry), done      |
//! | intake             | `intake:<id>` (its own)| approve, refuse            |
//!
//! A browser hand-over can only be skipped from a chat (the person
//! finishes it in the browser), and a failed workflow run is run again from
//! its button (it may send things again). An item that no longer waits gets
//! no tools.

use serde_json::Value;

use crate::model::ApprovalStatus;
use crate::services::AppState;

/// The prefix of an Inbox item's thread
pub const PREFIX: &str = "inbox:";

/// Longest text from an item put in the prompt, in characters
const MAX_FACT_CHARS: usize = 3000;

/// An Inbox item, as its thread's conversation ID names it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxItemRef {
    Approval {
        approval_id: String,
    },
    WorkflowApproval {
        run_id: String,
        step_id: String,
    },
    GatewayHold {
        task_id: String,
    },
    Assignment {
        task_id: String,
    },
    Clarification {
        task_id: String,
        question_id: String,
    },
    ManualStep {
        run_id: String,
        step_id: String,
    },
    TrustRequest {
        instance_id: String,
    },
    RequestReturn {
        task_id: String,
    },
    Failed {
        work_id: String,
    },
    /// An intake item's thread (`intake:<id>`), kept as it is
    Intake {
        conversation_id: String,
    },
}

/// `/` and `%` escaped, so an ID never splits a two-part one
fn encode(part: &str) -> String {
    part.replace('%', "%25").replace('/', "%2F")
}

fn decode(part: &str) -> String {
    part.replace("%2F", "/").replace("%25", "%")
}

fn two(rest: &str) -> Option<(String, String)> {
    let (a, b) = rest.split_once('/')?;
    (!a.is_empty() && !b.is_empty()).then(|| (decode(a), decode(b)))
}

impl InboxItemRef {
    /// The item a conversation is the thread of, if it is one
    pub fn parse(conversation_id: &str) -> Option<Self> {
        if crate::services::intake::thread::is_thread(conversation_id) {
            return Some(Self::Intake {
                conversation_id: conversation_id.to_string(),
            });
        }
        let rest = conversation_id.strip_prefix(PREFIX)?;
        let (kind, id) = rest.split_once(':')?;
        if id.is_empty() {
            return None;
        }
        let one = || decode(id);
        Some(match kind {
            "approval" => Self::Approval { approval_id: one() },
            "workflowApproval" => {
                let (run_id, step_id) = two(id)?;
                Self::WorkflowApproval { run_id, step_id }
            }
            "gatewayHold" => Self::GatewayHold { task_id: one() },
            "assignment" => Self::Assignment { task_id: one() },
            "clarification" => {
                let (task_id, question_id) = two(id)?;
                Self::Clarification {
                    task_id,
                    question_id,
                }
            }
            "manualStep" => {
                let (run_id, step_id) = two(id)?;
                Self::ManualStep { run_id, step_id }
            }
            "trustRequest" => Self::TrustRequest { instance_id: one() },
            "requestReturn" => Self::RequestReturn { task_id: one() },
            "failed" => Self::Failed { work_id: one() },
            _ => return None,
        })
    }

    /// The Inbox kind, as the UI names it
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Approval { .. } => "approval",
            Self::WorkflowApproval { .. } => "workflowApproval",
            Self::GatewayHold { .. } => "gatewayHold",
            Self::Assignment { .. } => "assignment",
            Self::Clarification { .. } => "clarification",
            Self::ManualStep { .. } => "manualStep",
            Self::TrustRequest { .. } => "trustRequest",
            Self::RequestReturn { .. } => "requestReturn",
            Self::Failed { .. } => "failed",
            Self::Intake { .. } => "intake",
        }
    }

    /// The item's ID inside its kind (`<run>/<step>` for two-part ones)
    pub fn id(&self) -> String {
        match self {
            Self::Approval { approval_id: id }
            | Self::GatewayHold { task_id: id }
            | Self::Assignment { task_id: id }
            | Self::TrustRequest { instance_id: id }
            | Self::RequestReturn { task_id: id }
            | Self::Failed { work_id: id }
            | Self::Intake {
                conversation_id: id,
            } => encode(id),
            Self::WorkflowApproval { run_id, step_id } | Self::ManualStep { run_id, step_id } => {
                format!("{}/{}", encode(run_id), encode(step_id))
            }
            Self::Clarification {
                task_id,
                question_id,
            } => format!("{}/{}", encode(task_id), encode(question_id)),
        }
    }

    /// Its thread's conversation ID
    pub fn conversation_id(&self) -> String {
        match self {
            Self::Intake { conversation_id } => conversation_id.clone(),
            _ => format!("{PREFIX}{}:{}", self.kind(), self.id()),
        }
    }
}

/// A decision the Chief of Staff may propose in a thread
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionTool {
    Approve,
    Refuse,
    Answer,
    Done,
    Reassign,
}

/// What a thread's turns are told about its item
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemFacts {
    /// A short title (the conversation's)
    pub title: String,
    /// What the person decides, in our words (trusted)
    pub deciding: String,
    /// What the item says, line by line (untrusted)
    pub facts: Vec<String>,
    /// Empty once it no longer waits
    pub tools: Vec<DecisionTool>,
    /// Who it goes back to, when a person asked (for reassigning)
    pub requester: Option<String>,
}

pub(crate) fn cut(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > MAX_FACT_CHARS {
        let cut: String = text.chars().take(MAX_FACT_CHARS).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

fn short_title(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 80 {
        let cut: String = line.chars().take(80).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

fn when(ms: f64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn gone(title: String, what: &str) -> ItemFacts {
    ItemFacts {
        title,
        deciding: format!("{what}; it no longer waits on the user (decided, or it ended)"),
        ..ItemFacts::default()
    }
}

/// The tools for an approval of `tool`: a hand-over is finished in the
/// browser, so a chat can only skip it
pub fn approval_tools(tool: &str) -> Vec<DecisionTool> {
    if crate::services::pilots::handover::is_hand_over(tool) {
        vec![DecisionTool::Refuse]
    } else {
        vec![DecisionTool::Approve, DecisionTool::Refuse]
    }
}

fn args_text(args: &Value) -> String {
    cut(&serde_json::to_string_pretty(args).unwrap_or_default())
}

/// What a thread about `item` is told, read now
pub async fn load(state: &AppState, item: &InboxItemRef) -> Result<ItemFacts, String> {
    use DecisionTool::*;
    match item {
        InboxItemRef::Approval { approval_id } => {
            let id = approval_id.clone();
            let approval = state
                .db()
                .call(move |conn| crate::db::get_approval(conn, &id))
                .await?
                .ok_or("That approval wasn't found")?;
            let title = approval.tool.clone();
            if approval.status != ApprovalStatus::Pending {
                return Ok(gone(title, "A tool call an agent asked to make"));
            }
            Ok(ItemFacts {
                title,
                deciding: format!(
                    "whether the agent `{}` may make this tool call (Approve lets this one call \
run; Refuse skips it)",
                    approval.agent_id
                ),
                facts: vec![
                    format!("Agent: {}", approval.agent_id),
                    format!("Tool: {}", approval.tool),
                    format!("Arguments:\n{}", args_text(&approval.args)),
                ],
                tools: approval_tools(&approval.tool),
                requester: None,
            })
        }
        InboxItemRef::WorkflowApproval { run_id, step_id } => {
            let work = crate::services::work::get_work_item(state, run_id.clone())
                .await
                .ok();
            let title = work
                .as_ref()
                .map(|w| w.title.clone())
                .unwrap_or_else(|| step_id.clone());
            let waiting = work.as_ref().is_none_or(|w| {
                w.state == crate::services::work::WorkState::Waiting
                    && w.next_action
                        .as_ref()
                        .is_some_and(|a| a.kind == crate::services::work::NextActionKind::Approval)
            });
            if !waiting {
                return Ok(gone(title, "A workflow step that waited for approval"));
            }
            let mut facts = vec![
                format!("Workflow run: {run_id}"),
                format!("Step: {step_id}"),
            ];
            if let Some(action) = work.as_ref().and_then(|w| w.next_action.as_ref()) {
                facts.push(format!("Waiting for: {}", action.description));
            }
            Ok(ItemFacts {
                title,
                deciding: "whether this workflow step may go ahead (Approve runs it; Refuse \
stops it)"
                    .to_string(),
                facts,
                tools: vec![Approve, Refuse],
                requester: None,
            })
        }
        InboxItemRef::GatewayHold { task_id } => {
            let id = task_id.clone();
            let hold = state
                .db()
                .call(move |conn| crate::services::gateway::holds::get(conn, &id))
                .await?
                .ok_or("That request wasn't found")?;
            let title = format!("{}: {}", hold.caller_name, short_title(&hold.summary));
            if hold.status != crate::services::gateway::holds::HoldStatus::Pending {
                return Ok(gone(
                    title,
                    "Someone's request held for the user's approval",
                ));
            }
            Ok(ItemFacts {
                title,
                deciding: format!(
                    "whether {}'s request goes ahead (Approve lets this Nandi work on it; Refuse \
declines it, and they get the reason)",
                    hold.caller_name
                ),
                facts: vec![
                    format!("From: {}", hold.caller_name),
                    format!(
                        "What they use: {}",
                        hold.capability
                            .clone()
                            .unwrap_or_else(|| "a request".to_string())
                    ),
                    format!("Expires: {}", when(hold.expires_at)),
                    format!("What they asked:\n{}", cut(&hold.summary)),
                ],
                tools: vec![Approve, Refuse, Reassign],
                requester: Some(hold.caller_name),
            })
        }
        InboxItemRef::Assignment { task_id } => {
            let id = task_id.clone();
            let assignment = state
                .db()
                .call(move |conn| crate::services::gateway::assignments::get(conn, &id))
                .await?
                .ok_or("That assignment wasn't found")?;
            let title = format!(
                "{}: {}",
                assignment.caller_name,
                short_title(&assignment.request)
            );
            if assignment.status != crate::services::gateway::assignments::AssignmentStatus::Open {
                return Ok(gone(title, "Work someone assigned to the user"));
            }
            let mut facts = vec![
                format!("From: {}", assignment.caller_name),
                format!("What they asked:\n{}", cut(&assignment.request)),
            ];
            if let Some(due) = assignment.due_at {
                facts.push(format!("Due: {}", when(due)));
            }
            for link in &assignment.links {
                facts.push(format!("Link: {} ({})", link.name, link.url));
            }
            Ok(ItemFacts {
                title,
                deciding: format!(
                    "how to close work {} assigned to the user (Done sends them the note as the \
result; Refuse declines it with the reason)",
                    assignment.caller_name
                ),
                facts,
                tools: vec![Done, Refuse, Reassign],
                requester: Some(assignment.caller_name),
            })
        }
        InboxItemRef::Clarification {
            task_id,
            question_id,
        } => {
            let pending = crate::services::a2a::clarification::get_pending_clarifications(state)
                .await
                .unwrap_or_default();
            let Some(question) = pending
                .into_iter()
                .find(|c| &c.task_id == task_id && &c.question.id == question_id)
            else {
                return Ok(gone(question_id.clone(), "A question from an agent"));
            };
            Ok(ItemFacts {
                title: short_title(&question.question.question),
                deciding: "the answer to a question an agent asked during delegated work \
(Answer sends the user's words; Refuse says no)"
                    .to_string(),
                facts: vec![
                    format!("For the agent: {}", question.agent_id),
                    format!("Topic: {}", question.question.topic),
                    format!("Question:\n{}", cut(&question.question.question)),
                ],
                tools: vec![Answer, Refuse],
                requester: None,
            })
        }
        InboxItemRef::ManualStep { run_id, step_id } => {
            let pending =
                crate::services::workflows::manual_steps::list_pending_manual_steps(state)
                    .await
                    .unwrap_or_default();
            let Some(step) = pending
                .into_iter()
                .find(|s| &s.run_id == run_id && &s.step_id == step_id)
            else {
                return Ok(gone(
                    step_id.clone(),
                    "A workflow step the user does by hand",
                ));
            };
            Ok(ItemFacts {
                title: format!("{}: {}", step.workflow_name, step.step_id),
                deciding: "when this step the user does by hand is done (Done passes the note on \
as the step's output, and the workflow goes on)"
                    .to_string(),
                facts: vec![
                    format!("Workflow: {}", step.workflow_name),
                    format!("Step: {}", step.step_id),
                    format!("Instructions:\n{}", cut(&step.instructions)),
                ],
                tools: vec![Done],
                requester: None,
            })
        }
        InboxItemRef::TrustRequest { instance_id } => {
            let requests = crate::services::team::trust_requests::list_trust_requests(state)
                .await
                .unwrap_or_default();
            let Some(request) = requests.into_iter().find(|r| &r.instance_id == instance_id) else {
                return Ok(gone(instance_id.clone(), "Someone asking to be trusted"));
            };
            let name = request
                .member_name
                .clone()
                .or_else(|| request.handle.clone())
                .unwrap_or_else(|| request.instance_id.clone());
            Ok(ItemFacts {
                title: name.clone(),
                deciding: format!(
                    "whether to trust {name} (Approve trusts them from now on; Refuse is Not now)"
                ),
                facts: vec![
                    format!("Who: {name} ({})", request.instance_id),
                    format!("Asked: {} on {}", request.capability, request.agent_name),
                    format!("Refused so far: {}", request.attempts),
                ],
                tools: vec![Approve, Refuse],
                requester: None,
            })
        }
        InboxItemRef::RequestReturn { task_id } => {
            let returns = crate::services::a2a::returns::list(state.db())
                .await
                .unwrap_or_default();
            let Some(ret) = returns.into_iter().find(|r| &r.task_id == task_id) else {
                return Ok(gone(task_id.clone(), "A request that came back"));
            };
            Ok(ItemFacts {
                title: format!("{}: {}", ret.from_name, short_title(&ret.request)),
                deciding: "nothing but whether the user is done with it (Done takes it out of the \
Inbox); they may ask about it"
                    .to_string(),
                facts: vec![
                    format!("From: {}", ret.from_name),
                    format!("Outcome: {:?}", ret.outcome),
                    format!("What the user asked:\n{}", cut(&ret.request)),
                    format!("What came back:\n{}", cut(&ret.summary)),
                ],
                tools: vec![Done],
                requester: None,
            })
        }
        InboxItemRef::Failed { work_id } => {
            let work = crate::services::work::get_work_item(state, work_id.clone()).await?;
            if work.state != crate::services::work::WorkState::Failed {
                return Ok(gone(work.title, "Work that failed"));
            }
            let mut facts = vec![
                format!("What: {}", work.title),
                format!("Kind: {:?}", work.kind),
            ];
            if let Some(owner) = &work.owner {
                facts.push(format!("Who worked on it: {owner}"));
            }
            if let Some(action) = &work.next_action {
                facts.push(format!("Details: {}", action.description));
            }
            // A workflow run again may send things again: its button asks first
            let tools = if work.kind == crate::services::work::WorkKind::Workflow {
                vec![Done]
            } else {
                vec![Approve, Done]
            };
            Ok(ItemFacts {
                title: work.title,
                deciding: "what to do about work that failed (Approve tries it again; Done \
dismisses it)"
                    .to_string(),
                facts,
                tools,
                requester: None,
            })
        }
        InboxItemRef::Intake { conversation_id } => {
            let id = conversation_id.clone();
            let items = state
                .db()
                .call(move |conn| crate::services::intake::for_thread(conn, &id))
                .await?;
            let title = items
                .first()
                .map(|i| crate::services::requests::local::title_for(&i.text))
                .unwrap_or_default();
            let pending = items
                .iter()
                .any(|i| i.status == crate::services::intake::IntakeStatus::Pending);
            Ok(ItemFacts {
                title,
                deciding: "whether the work queued in this thread goes ahead (Approve starts it; \
Refuse dismisses it)"
                    .to_string(),
                facts: Vec::new(),
                tools: if pending {
                    vec![Approve, Refuse]
                } else {
                    Vec::new()
                },
                requester: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_conversation_id() {
        let items = [
            InboxItemRef::Approval {
                approval_id: "a1".into(),
            },
            InboxItemRef::WorkflowApproval {
                run_id: "r1".into(),
                step_id: "review".into(),
            },
            InboxItemRef::GatewayHold {
                task_id: "t/1".into(),
            },
            InboxItemRef::Assignment {
                task_id: "t2".into(),
            },
            InboxItemRef::Clarification {
                task_id: "t3".into(),
                question_id: "q%1".into(),
            },
            InboxItemRef::ManualStep {
                run_id: "r2".into(),
                step_id: "sign".into(),
            },
            InboxItemRef::TrustRequest {
                instance_id: "desktop-sam".into(),
            },
            InboxItemRef::RequestReturn {
                task_id: "t4".into(),
            },
            InboxItemRef::Failed {
                work_id: "w1".into(),
            },
            InboxItemRef::Intake {
                conversation_id: "intake:q1".into(),
            },
        ];
        for item in items {
            let id = item.conversation_id();
            assert_eq!(InboxItemRef::parse(&id), Some(item.clone()), "{id}");
        }
        assert_eq!(
            InboxItemRef::WorkflowApproval {
                run_id: "r1".into(),
                step_id: "review".into()
            }
            .conversation_id(),
            "inbox:workflowApproval:r1/review"
        );
        assert_eq!(
            InboxItemRef::GatewayHold {
                task_id: "t/1".into()
            }
            .conversation_id(),
            "inbox:gatewayHold:t%2F1"
        );
    }

    #[test]
    fn other_conversations_are_not_threads() {
        for id in [
            "c1",
            "inbox:",
            "inbox:approval:",
            "inbox:nope:x",
            "inbox:manualStep:r1",
            "inbox:manualStep:/s",
        ] {
            assert_eq!(InboxItemRef::parse(id), None, "{id}");
        }
    }

    #[test]
    fn a_hand_over_can_only_be_skipped_from_a_chat() {
        assert_eq!(
            approval_tools("pilot_browser__browser_hand_over"),
            vec![DecisionTool::Refuse]
        );
        assert_eq!(
            approval_tools("shell"),
            vec![DecisionTool::Approve, DecisionTool::Refuse]
        );
    }
}
