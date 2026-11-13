//! The built-in `delegate_to_agent` tool: `{capability, input, target?}`.
//!
//! - Offered only to agents that may delegate (`settings::outbound_allowed`).
//! - Needs approval through the orchestrator's usual approval path (the
//!   call is persisted, the user approves or rejects it, the run resumes),
//!   unless skill.md declares `requires_approval` without it.
//! - Runs on the delegator whatever the engine, so canned runs exercise it.
//!
//! Also `ask_requester`: `{questions: [{question,
//! topic, requires_human?}]}`, offered only during a delegated turn
//! (`SourceType::Agent` with a running delegation). It sends the questions
//! to the requester and blocks the turn until they're answered or the wait
//! ends (see `clarification`). Never needs approval: the requester's side
//! decides who answers.

use std::sync::{Arc, RwLock, Weak};

use serde_json::{json, Value};

use super::clarification::{AskedQuestion, ClarificationService, ASK_REQUESTER_TOOL};
use super::{settings, A2ADelegator, DelegationRequest, TaskStatus};
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::model::STUDIO_AGENT_ID;
use crate::services::domain::capability::DELEGATION_TOOL;
use crate::services::domain::AgentCard;
use crate::services::gateway::GATEWAY_AGENT_ID;
use crate::services::network::NetworkMember;
use crate::services::repo_store::RepoStore;

pub const NAME: &str = DELEGATION_TOOL;

/// Whether an agent gets the tool, and whether its calls need approval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolGate {
    pub offered: bool,
    pub requires_approval: bool,
}

impl ToolGate {
    pub fn requires_approval_for(&self, tool: &str) -> bool {
        tool == NAME && self.offered && self.requires_approval
    }
}

/// Studio's gate: it may send the user's requests to people on their
/// network (and to local agents), each call approved by the user first
pub const STUDIO_GATE: ToolGate = ToolGate {
    offered: true,
    requires_approval: true,
};

/// What Studio's prompt says about who it can ask: online members whose
/// gateway takes requests, with the target to use
pub fn studio_network_section(members: &[NetworkMember]) -> String {
    let lines: Vec<String> = members
        .iter()
        .filter(|m| m.online && m.gateway.accepts_delegation)
        .map(|m| {
            let mut line = format!(
                "- {} — target `{}`",
                m.display_name,
                AgentCard::card_id(&m.instance_id, GATEWAY_AGENT_ID)
            );
            let description = m.gateway.description.trim();
            if !description.is_empty() {
                line.push_str(&format!(": {description}"));
            }
            if !m.gateway.capabilities.is_empty() {
                line.push_str(&format!(
                    " (can help with: {})",
                    m.gateway.capabilities.join(", ")
                ));
            }
            line
        })
        .collect();
    let people = if lines.is_empty() {
        "Nobody on the user's network is online and taking requests right now.".to_string()
    } else {
        lines.join(
            "
",
        )
    };
    format!(
        "## People on the user's Nandi network

When the user asks you to get something done by someone on their network (\"ask Bob to…\"), call `{NAME}` with that person's target, capability `{GATEWAY_AGENT_ID}`, and input `{{\"text\": \"<the request, self-contained>\"}}`. Their Nandi routes it to the right agent and you get the answer back. The user approves each request before it's sent. Don't invent people or targets.

{people}"
    )
}

/// The gate for `agent_id` (closed for unknown agents; Studio's is
/// [`STUDIO_GATE`]).
pub fn policy(store: &RepoStore, agent_id: &str) -> ToolGate {
    if agent_id == STUDIO_AGENT_ID {
        return STUDIO_GATE;
    }
    let Ok(agent) = store.agent(agent_id) else {
        return ToolGate::default();
    };
    let requires_approval = agent
        .capabilities()
        .tools()
        .declared_approvals()
        .is_none_or(|tools| tools.iter().any(|t| t == NAME));
    ToolGate {
        offered: settings::outbound_allowed(&agent),
        requires_approval,
    }
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: NAME.to_string(),
        description: "Ask another agent (yours or a teammate's) to do a task for you. \
Name the capability you need, e.g. `summarize-paper`; Nandi picks an agent that serves it \
and returns that agent's answer. The user may have to approve the call first. `assign-me` assigns work to a person, you included (input `{text, due?: YYYY-MM-DD, links?: [{url, name?}]}`, due in 7 days by default; `links` are http(s) links shown on their card, so share a file as a link to where it lives): it returns `status: assigned` at once and stays on their list until they mark it done or decline it; `my_requests` shows how it stands."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "capability": {
                    "type": "string",
                    "description": "The capability you need, e.g. summarize-paper"
                },
                "input": {
                    "type": "object",
                    "description": "What the other agent needs to do the task"
                },
                "target": {
                    "type": "string",
                    "description": "Optional: a specific agent card ID (<instance>/<agent>)"
                }
            },
            "required": ["capability", "input"]
        }),
    }
}

fn error(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: message.into(),
        is_error: true,
    }
}

/// Runs one call for `agent_id`, once approved.
pub async fn call(
    delegator: Option<Arc<dyn A2ADelegator>>,
    store: &RepoStore,
    agent_id: &str,
    args: &Value,
) -> ToolOutcome {
    // Studio asks on the user's behalf (as the user, `<instance>/_user`)
    let studio = agent_id == STUDIO_AGENT_ID;
    if !studio {
        match store.agent(agent_id) {
            Ok(agent) if settings::outbound_allowed(&agent) => {}
            _ => return error(
                "This agent may not delegate: turn on outbound delegation in its Delegation                  settings",
            ),
        }
    }
    let Some(capability) = args
        .get("capability")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
    else {
        return error("'capability' is required");
    };
    let Some(delegator) = delegator else {
        return error("Delegation is unavailable right now");
    };
    let request = DelegationRequest {
        task_id: None,
        requester_agent: (!studio).then(|| agent_id.to_string()),
        capability: capability.to_string(),
        input: args.get("input").cloned().unwrap_or_else(|| json!({})),
        target: args
            .get("target")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
    };
    match delegator.delegate_request(request).await {
        Ok(result) if result.status == TaskStatus::Completed => ToolOutcome {
            result: json!({
                "provider": result.provider_agent,
                "output": result.output,
            })
            .to_string(),
            is_error: false,
        },
        // No answer yet: it stays open until it's due
        Ok(result)
            if result.status == TaskStatus::Running
                && result.output.get("waiting").and_then(Value::as_bool) == Some(true) =>
        {
            ToolOutcome {
                result: json!({
                    "status": "waiting",
                    "provider": result.provider_agent,
                    "task_id": result.task_id,
                    "state": result.output.get("state"),
                    "due": result.output.get("due"),
                    "note": "No answer yet. The request stays open until it's due: if they're away, it reaches them when they're back. You don't wait for it; my_requests shows how it stands, and the answer arrives on its own.",
                })
                .to_string(),
                is_error: false,
            }
        }
        // An assignment: with its person now, open until they end it
        Ok(result) if result.status == TaskStatus::Running => ToolOutcome {
            result: json!({
                "status": "assigned",
                "provider": result.provider_agent,
                "task_id": result.task_id,
                "state": result.output.get("state"),
                "due": result.output.get("due"),
                "note": "It's on their list (their Inbox) until they mark it done or decline it;                          you don't wait for it. my_requests shows how it stands, and the                          result arrives on its own.",
            })
            .to_string(),
            is_error: false,
        },
        Ok(result) => error(format!(
            "The delegated task {} ({}): {}",
            result.status.as_str(),
            result.provider_agent,
            result.error.unwrap_or_default()
        )),
        Err(e) => error(format!("Delegation failed: {e}")),
    }
}

// ---------------------------------------------------------------------------
// `ask_requester`
// ---------------------------------------------------------------------------

pub const ASK_REQUESTER: &str = ASK_REQUESTER_TOOL;

/// The clarification service `ask_requester` calls, like `DelegationSlot`.
/// Weak: `AppState` owns the service.
#[derive(Clone, Default)]
pub struct ClarificationSlot(Arc<RwLock<Option<Weak<ClarificationService>>>>);

impl ClarificationSlot {
    pub fn set(&self, service: &Arc<ClarificationService>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(service));
    }

    pub fn get(&self) -> Option<Arc<ClarificationService>> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }
}

pub fn ask_requester_spec() -> ToolSpec {
    ToolSpec {
        name: ASK_REQUESTER_TOOL.to_string(),
        description: "Ask the agent (or person) that delegated this task to you a question \
before you go on, when the task is ambiguous or missing something you can't safely guess. Ask \
everything you need at once: you get a few rounds at most, and each waits for the answer. Set \
`requires_human` when only a person can answer (a decision or a preference). Never ask for \
passwords or keys."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": super::clarification::MAX_QUESTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string" },
                            "topic": {
                                "type": "string",
                                "description": "A short label, e.g. format, scope, budget, deadline"
                            },
                            "requires_human": { "type": "boolean" }
                        },
                        "required": ["question", "topic"]
                    }
                }
            },
            "required": ["questions"]
        }),
    }
}

/// Whether the turn of `run_id` is delegated, so it gets `ask_requester`
pub async fn ask_requester_offered(
    clarifier: Option<&Arc<ClarificationService>>,
    run_id: &str,
    agent_id: &str,
) -> bool {
    match clarifier {
        Some(service) => matches!(service.delegated_turn(run_id, agent_id).await, Ok(Some(_))),
        None => false,
    }
}

/// Runs one `ask_requester` call of `agent_id`'s run `run_id`.
pub async fn ask_requester(
    clarifier: Option<Arc<ClarificationService>>,
    run_id: &str,
    agent_id: &str,
    args: &Value,
) -> ToolOutcome {
    let Some(service) = clarifier else {
        return error("Clarification is unavailable right now");
    };
    let turn = match service.delegated_turn(run_id, agent_id).await {
        Ok(Some(turn)) => turn,
        Ok(None) => return error("ask_requester only works while doing a delegated task"),
        Err(e) => return error(format!("Couldn't find the delegated task: {e}")),
    };
    let questions: Vec<AskedQuestion> =
        match args.get("questions").cloned().map(serde_json::from_value) {
            Some(Ok(questions)) => questions,
            Some(Err(e)) => return error(format!("Invalid 'questions': {e}")),
            None => return error("'questions' is required"),
        };
    match service.ask(&turn, questions).await {
        Ok((round, answers)) => ToolOutcome {
            result: json!({
                "round": round,
                "answers": answers
                    .into_iter()
                    .map(|(q, answer)| json!({ "question": q.question, "answer": answer }))
                    .collect::<Vec<_>>(),
            })
            .to_string(),
            is_error: false,
        },
        Err(e) => error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent_card::OwnerRef;
    use crate::services::gateway::GatewayCard;

    fn member(name: &str, online: bool, accepting: bool) -> NetworkMember {
        NetworkMember {
            instance_id: format!("{name}-inst"),
            display_name: name.to_string(),
            avatar_url: None,
            online,
            last_seen: None,
            gateway: GatewayCard {
                instance_id: format!("{name}-inst"),
                endpoint: String::new(),
                display_name: name.to_string(),
                description: format!("{name}'s agents"),
                capabilities: vec!["general-help".into()],
                accepts_delegation: accepting,
                max_concurrent: 3,
                current_load: 0,
                owner: OwnerRef::default(),
                federation_enabled: false,
                gateway_version: 1,
                agent_count: 1,
                body: None,
                direct_delegation: None,
                responsibilities: Vec::new(),
                availability: Default::default(),
            },
            handle: None,
        }
    }

    #[test]
    fn studio_lists_who_it_can_ask_with_their_targets() {
        let section = studio_network_section(&[
            member("Bob", true, true),
            member("Carol", false, true),
            member("Dan", true, false),
        ]);
        assert!(section.contains("- Bob — target `Bob-inst/gateway`: Bob's agents"));
        assert!(section.contains("can help with: general-help"));
        assert!(!section.contains("Carol") && !section.contains("Dan"));
        assert!(studio_network_section(&[]).contains("Nobody on the user's network"));
    }

    #[test]
    fn studio_may_delegate_with_approval() {
        let gate = STUDIO_GATE;
        assert!(gate.offered);
        assert!(gate.requires_approval_for(NAME));
    }
}
