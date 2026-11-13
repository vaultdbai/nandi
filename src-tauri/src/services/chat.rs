//! Chat: sending messages, Studio conversations, approvals, cancellation.
//!
//! Each call validates and persists what it can synchronously (so a client
//! gets an immediate, durable answer), then hands work to the agent's
//! orchestrator task.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db;
use crate::model::{
    ApprovalDecision, ApprovalStatus, RunStatus, StudioConversationSummary, STUDIO_AGENT_ID,
};
use crate::services::harness::approvals as harness_approvals;
use crate::services::orchestrator::{OrchestratorCommand, Orchestrators, RunSource, TurnRequest};
use crate::services::pilots::handover::{self, HandOverAnswer, HandOverOutcome};
use crate::services::{a2a, agents, attachments, AppState};

const MAX_MESSAGE_CHARS: usize = 100_000;
const MAX_TITLE_CHARS: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResult {
    pub run_id: String,
    /// The user message's ID (the client ID, if one was given). A queued
    /// message is written to the conversation when its run starts.
    pub message_id: String,
    /// The conversation title, if this message set it (Studio)
    pub conversation_title: Option<String>,
    /// The agent was busy: the run waits in its queue
    pub queued: bool,
}

/// Validates a message and hands it to the agent's orchestrator, which
/// starts it or queues it. Returns once the run is persisted.
pub(crate) async fn submit(
    orchestrators: &Orchestrators,
    agent_id: &str,
    conversation_id: String,
    text: String,
    attachment_ids: Vec<String>,
    client_message_id: Option<String>,
    source: RunSource,
) -> Result<SendMessageResult, String> {
    let text = text.trim().to_string();
    if text.is_empty() && attachment_ids.is_empty() {
        return Err("Message is empty".to_string());
    }
    // Marketplace agents run only after the user approved what they request
    crate::services::marketplace::consent::check(orchestrators.store(), agent_id)?;
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!(
            "Message is longer than {MAX_MESSAGE_CHARS} characters"
        ));
    }
    let message_id = match client_message_id {
        Some(id) => uuid::Uuid::parse_str(&id)
            .map(|u| u.to_string())
            .map_err(|_| "Invalid message id".to_string())?,
        None => uuid::Uuid::new_v4().to_string(),
    };
    let staged = attachments::resolve_staged(&attachment_ids).await?;

    let run_id = uuid::Uuid::new_v4().to_string();
    let input = if text.is_empty() {
        staged
            .iter()
            .map(|s| s.info.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        text.clone()
    };
    let request = TurnRequest {
        run_id: run_id.clone(),
        conversation_id: conversation_id.clone(),
        message_id: message_id.clone(),
        text,
        input: input.clone(),
        attachments: staged.iter().map(|s| s.info.clone()).collect(),
        source,
    };
    let queued = orchestrators
        .request(agent_id, |reply| OrchestratorCommand::SendMessage {
            request: Box::new(request),
            reply,
        })
        .await?;
    if let Err(e) = attachments::commit_staged(staged, &run_id).await {
        log::error!("Failed to store attachments for run {run_id}: {e}");
    }

    // Studio: the first message names a new conversation
    let conversation_title = if agent_id == STUDIO_AGENT_ID {
        let title = crate::services::events::truncate(
            input.lines().next().unwrap_or_default(),
            MAX_TITLE_CHARS,
        );
        let id = conversation_id.clone();
        orchestrators
            .bus()
            .db
            .call(move |conn| Ok(db::set_title_if_untitled(conn, &id, &title)?.then_some(title)))
            .await?
    } else {
        None
    };

    Ok(SendMessageResult {
        run_id,
        message_id,
        conversation_title,
        queued,
    })
}

/// Starts a turn for an agent and returns as soon as the user message and
/// run are persisted. Progress arrives as `agent://*` events.
pub async fn send_message(
    state: &AppState,
    agent_id: String,
    text: String,
    attachments: Vec<String>,
    client_message_id: Option<String>,
) -> Result<SendMessageResult, String> {
    state.store().require_agent(&agent_id)?;
    let id = agent_id.clone();
    let conversation_id = state
        .db()
        .call(move |conn| db::ensure_agent_conversation(conn, &id))
        .await?;
    submit(
        state.orchestrators(),
        &agent_id,
        conversation_id,
        text,
        attachments,
        client_message_id,
        RunSource::user(),
    )
    .await
}

/// Like `send_message`, for a Studio conversation.
pub async fn send_studio_message(
    state: &AppState,
    conversation_id: String,
    text: String,
    attachments: Vec<String>,
    client_message_id: Option<String>,
) -> Result<SendMessageResult, String> {
    let id = conversation_id.clone();
    let owner = state
        .db()
        .call(move |conn| db::conversation_agent(conn, &id))
        .await?;
    if owner.as_deref() != Some(STUDIO_AGENT_ID) {
        return Err("That conversation was not found".to_string());
    }
    submit(
        state.orchestrators(),
        STUDIO_AGENT_ID,
        conversation_id,
        text,
        attachments,
        client_message_id,
        RunSource::user(),
    )
    .await
}

/// Creates an untitled conversation; its first message names it.
pub async fn create_studio_conversation(
    state: &AppState,
) -> Result<StudioConversationSummary, String> {
    let title = String::new();
    let id = uuid::Uuid::new_v4().to_string();
    state
        .db()
        .call(move |conn| db::insert_studio_conversation(conn, &id, &title))
        .await
}

/// Resolves a pending approval, whichever view it was clicked in. With
/// `agent_id`, the approval must belong to that agent (HTTP routes are
/// scoped by agent).
pub async fn approve_action(
    state: &AppState,
    agent_id: Option<&str>,
    approval_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let id = approval_id.clone();
    let approval = state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await?
        .filter(|a| agent_id.is_none_or(|agent| a.agent_id == agent))
        .ok_or("Approval not found")?;
    if approval.status != ApprovalStatus::Pending {
        return Err("This approval request is no longer pending".to_string());
    }
    // Who decided, kept on the approval (the decisions file names them too)
    let who = decider(state);
    let id = approval_id.clone();
    state
        .db()
        .call(move |conn| db::set_approval_decider(conn, &id, &who))
        .await?;
    let decided = resolve(state, &approval, approval_id, decision).await;
    if decided.is_ok() {
        record_decision(state, &approval, decision);
    }
    decided
}

/// Sends a decided approval where its call waits
async fn resolve(
    state: &AppState,
    approval: &db::ApprovalRow,
    approval_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    // An escalated clarification question names no
    // run: the clarification service resolves it, not the orchestrator
    if approval.tool == a2a::escalation::ESCALATION_TOOL {
        return a2a::escalation::resolve_approval(state, &approval_id, decision).await;
    }
    // A harness run waits on the call itself; the orchestrator would
    // restart the run natively
    if harness_approvals::is_harness(approval) {
        if harness_approvals::decide(&approval_id, decision) {
            return Ok(());
        }
        harness_approvals::close_stale(state, approval).await?;
        return Err("This run stopped while it waited, so it was cancelled".to_string());
    }
    state
        .orchestrators()
        .send(
            &approval.agent_id,
            OrchestratorCommand::Approve {
                approval_id,
                decision,
            },
        )
        .await
}

/// The decisions file (`inbox::audit`). An Inbox chat's own decision call
/// isn't one: the decision it makes is recorded where it's made.
fn record_decision(state: &AppState, approval: &db::ApprovalRow, decision: ApprovalDecision) {
    if crate::services::inbox::tool::is_decision_tool(&approval.tool) {
        return;
    }
    let approved = decision == ApprovalDecision::Approved;
    let (kind, word) = if approval.tool == a2a::escalation::ESCALATION_TOOL {
        (
            "clarification",
            if approved { "allowed" } else { "refused" },
        )
    } else {
        ("approval", if approved { "approved" } else { "refused" })
    };
    // An access request names the choice (once, on this computer, saved)
    let choice = approval
        .args
        .get("choice")
        .and_then(serde_json::Value::as_str)
        .map(|choice| format!("{word} ({choice})"));
    crate::services::inbox::audit::record(
        state,
        crate::services::inbox::audit::Decided::new(
            kind,
            &approval.id,
            choice.unwrap_or_else(|| word.to_string()),
        )
        .subject(format!(
            "{}: {}",
            approval.agent_id,
            approval_subject(approval)
        )),
    );
}

/// What an approval asked for, in a few words: the tool, or for an access
/// request what it wanted to reach (`read with Issue tracker`, `the site
/// shop.example`)
fn approval_subject(approval: &db::ApprovalRow) -> String {
    if approval.tool != crate::services::access::ACCESS_TOOL {
        return approval.tool.clone();
    }
    let args = &approval.args;
    if let Some(site) = args["site"]["display"].as_str() {
        return format!("the site {site}");
    }
    let names: Vec<&str> = args["connectors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["name"].as_str())
        .collect();
    format!(
        "{} with {}",
        args["access"].as_str().unwrap_or("use"),
        names.join(", ")
    )
}

/// Answers a hand-over (`pilots::handover`): Done and Changed it approve
/// the call, with the final text kept for its result; Skip rejects it.
pub async fn answer_hand_over(
    state: &AppState,
    approval_id: String,
    answer: HandOverAnswer,
) -> Result<(), String> {
    let answer = handover::validate(answer)?;
    let id = approval_id.clone();
    let approval = state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await?
        .ok_or("Approval not found")?;
    if !handover::is_hand_over(&approval.tool) {
        return Err("This approval isn't a hand-over".to_string());
    }
    let decision = match answer.outcome {
        HandOverOutcome::Skipped => ApprovalDecision::Rejected,
        HandOverOutcome::Done | HandOverOutcome::Changed => ApprovalDecision::Approved,
    };
    handover::record(&approval.tool_call_id, answer);
    let decided = approve_action(state, Some(&approval.agent_id), approval_id, decision).await;
    if decided.is_err() {
        handover::take(&approval.tool_call_id);
    }
    decided
}

/// The local user, by the name in NANDI.md (else their slug, else "you")
pub(crate) fn decider(state: &AppState) -> String {
    let root = &state.store().layout().root;
    crate::services::auth::display_name_at(root)
        .or_else(|| crate::services::auth::user_slug_at(root))
        .unwrap_or_else(|| "you".to_string())
}

/// Every pending tool approval, oldest first (the Inbox's approvals). An
/// Inbox chat's decision waiting for its confirm shows in that chat, not as
/// an item of its own.
pub async fn list_pending_approvals(
    state: &AppState,
) -> Result<Vec<crate::model::PendingApproval>, String> {
    state
        .db()
        .call(|conn| db::pending_approvals(conn, None))
        .await
        .map(|approvals| {
            approvals
                .into_iter()
                .filter(|a| {
                    a.tool != a2a::escalation::ESCALATION_TOOL
                        && !crate::services::inbox::tool::is_decision_tool(&a.tool)
                })
                .collect()
        })
}

/// Closes, as expired, the pending approvals nobody decided in time; their
/// runs go on without those calls. Returns how many were sent to expire.
pub async fn expire_overdue_approvals(state: &AppState) -> Result<usize, String> {
    let now = db::now_ms();
    let overdue = state
        .db()
        .call(move |conn| db::overdue_approvals(conn, now))
        .await?;
    let mut expired = 0;
    for approval in overdue {
        // Escalated questions follow their clarification's deadline
        if approval.tool == a2a::escalation::ESCALATION_TOOL {
            continue;
        }
        // A waiting harness call expires itself; one nobody waits for
        // ends its run
        if harness_approvals::is_harness(&approval) {
            if !harness_approvals::is_waiting(&approval.id) {
                match harness_approvals::close_stale(state, &approval).await {
                    Ok(()) => expired += 1,
                    Err(e) => log::warn!("Couldn't expire approval {}: {e}", approval.id),
                }
            }
            continue;
        }
        match state
            .orchestrators()
            .send(
                &approval.agent_id,
                OrchestratorCommand::ExpireApproval {
                    approval_id: approval.id.clone(),
                },
            )
            .await
        {
            Ok(()) => expired += 1,
            Err(e) => log::warn!("Couldn't expire approval {}: {e}", approval.id),
        }
    }
    Ok(expired)
}

/// How often overdue approvals are looked for
pub const APPROVAL_SWEEP: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn cancel_turn(
    state: &AppState,
    agent_id: String,
    turn_id: String,
) -> Result<(), String> {
    if agent_id != STUDIO_AGENT_ID {
        agents::validate_agent_id(&agent_id)?;
    }
    let id = turn_id.clone();
    let run = state.db().call(move |conn| db::get_run(conn, &id)).await?;
    match run {
        // Cancel only ever stops the current turn
        Some(run) if run.agent_id == agent_id && run.status == RunStatus::Queued => {
            Err("This run is still queued; remove it from the queue instead".to_string())
        }
        Some(run) if run.agent_id == agent_id => {
            state
                .orchestrators()
                .send(&agent_id, OrchestratorCommand::Cancel { run_id: turn_id })
                .await
        }
        _ => Err("Run not found for this agent".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::{bus, RecordingSink};
    use crate::services::orchestrator::{Orchestrators, RunSource};
    use serde_json::json;
    use std::time::Duration;

    async fn submit_text(o: &Orchestrators, text: &str) -> SendMessageResult {
        let conversation = o
            .bus()
            .db
            .call(|conn| db::ensure_agent_conversation(conn, "writer"))
            .await
            .unwrap();
        submit(
            o,
            "writer",
            conversation,
            text.into(),
            vec![],
            None,
            RunSource::user(),
        )
        .await
        .unwrap()
    }

    async fn wait_for(sink: &RecordingSink, name: &str, count: usize) {
        for _ in 0..400 {
            if sink.names().iter().filter(|n| **n == name).count() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {count}× {name}: {:?}", sink.names());
    }

    fn orchestrators(bus: crate::services::events::EventBus) -> Orchestrators {
        Orchestrators::new(
            crate::services::orchestrator::RunDeps {
                store: crate::services::repo_store::RepoStore::for_tests(bus.db.clone()),
                bus,
                connectors: None,
                harness: Default::default(),
                secrets: crate::services::secrets::Secrets::in_memory(),
                runtimes: crate::services::runtimes::AgentRuntimes::new(
                    Default::default(),
                    std::sync::Arc::new(crate::services::egress::EgressState::new(
                        crate::services::secrets::Secrets::in_memory(),
                    )),
                ),
            },
            tokio::runtime::Handle::current(),
        )
    }

    fn fast(o: &Orchestrators) -> Orchestrators {
        let conn = o.bus().db.conn().unwrap();
        db::set_setting(&conn, db::ENGINE_KEY, &json!("canned")).unwrap();
        let mut script = crate::engine::default_script();
        script["tokenDelayMs"] = json!(0);
        db::set_setting(&conn, crate::engine::CANNED_SCRIPT_KEY, &script).unwrap();
        o.clone()
    }

    async fn run_status(o: &Orchestrators, run_id: &str) -> RunStatus {
        let id = run_id.to_string();
        o.bus()
            .db
            .call(move |c| db::get_run(c, &id))
            .await
            .unwrap()
            .unwrap()
            .status
    }

    #[tokio::test]
    async fn runs_suspend_for_approval_and_resume() {
        let (bus, sink) = bus();
        let o = fast(&orchestrators(bus));
        let sent = submit_text(&o, "check the report").await;

        wait_for(&sink, "agent://approval_request", 1).await;
        assert_eq!(
            run_status(&o, &sent.run_id).await,
            RunStatus::AwaitingApproval
        );
        // The automatic tool ran; the approval-gated one didn't
        assert_eq!(
            sink.names()
                .iter()
                .filter(|n| **n == "agent://tool_result")
                .count(),
            1
        );

        // A second message while one is active waits in the queue; take it
        // out again so it doesn't run after this one
        let conversation = db::agent_conversation_id("writer");
        let again = submit(
            &o,
            "writer",
            conversation,
            "again".into(),
            vec![],
            None,
            RunSource::user(),
        )
        .await
        .unwrap();
        assert!(again.queued);
        assert_eq!(run_status(&o, &again.run_id).await, RunStatus::Queued);
        o.request("writer", |reply| OrchestratorCommand::CancelQueued {
            run_id: again.run_id.clone(),
            reply,
        })
        .await
        .unwrap();

        let approval = o
            .bus()
            .db
            .call(|c| db::pending_approvals(c, Some("writer")))
            .await
            .unwrap()
            .remove(0);
        o.send(
            "writer",
            OrchestratorCommand::Approve {
                approval_id: approval.id,
                decision: ApprovalDecision::Approved,
            },
        )
        .await
        .unwrap();

        // The first turn_done was the removed queued run's
        wait_for(&sink, "agent://turn_done", 2).await;
        assert_eq!(run_status(&o, &sent.run_id).await, RunStatus::Completed);
        assert_eq!(
            sink.names()
                .iter()
                .filter(|n| **n == "agent://tool_result")
                .count(),
            2
        );

        // The transcript is fully reconstructible from the DB
        let id = sent.run_id.clone();
        let roles: Vec<String> = o
            .bus()
            .db
            .call(move |c| db::list_run_messages(c, &id))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.role)
            .collect();
        assert_eq!(
            roles,
            [
                "user",
                "assistant",
                "tool_call",
                "approval",
                "tool_call",
                "assistant"
            ]
        );
        let run = o
            .bus()
            .db
            .call(move |c| db::get_run(c, &sent.run_id))
            .await
            .unwrap()
            .unwrap();
        assert!(run.summary.unwrap().contains("check the report"));
    }

    #[tokio::test]
    async fn rejection_skips_the_tool_and_completes() {
        let (bus, sink) = bus();
        let o = fast(&orchestrators(bus));
        let sent = submit_text(&o, "hi").await;
        wait_for(&sink, "agent://approval_request", 1).await;

        let approval = o
            .bus()
            .db
            .call(|c| db::pending_approvals(c, None))
            .await
            .unwrap()
            .remove(0);
        o.send(
            "writer",
            OrchestratorCommand::Approve {
                approval_id: approval.id,
                decision: ApprovalDecision::Rejected,
            },
        )
        .await
        .unwrap();

        wait_for(&sink, "agent://turn_done", 1).await;
        assert_eq!(run_status(&o, &sent.run_id).await, RunStatus::Completed);
        assert_eq!(
            sink.names()
                .iter()
                .filter(|n| **n == "agent://tool_call")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn cancel_ends_a_suspended_run_and_expires_its_approval() {
        let (bus, sink) = bus();
        let o = fast(&orchestrators(bus));
        let sent = submit_text(&o, "hi").await;
        wait_for(&sink, "agent://approval_request", 1).await;

        o.send(
            "writer",
            OrchestratorCommand::Cancel {
                run_id: sent.run_id.clone(),
            },
        )
        .await
        .unwrap();

        wait_for(&sink, "agent://turn_done", 1).await;
        assert_eq!(run_status(&o, &sent.run_id).await, RunStatus::Cancelled);
        assert!(o
            .bus()
            .db
            .call(|c| db::pending_approvals(c, None))
            .await
            .unwrap()
            .is_empty());
        // Now free for the next message
        submit_text(&o, "next").await;
    }

    #[tokio::test]
    async fn approvals_survive_an_orchestrator_restart() {
        let (bus, sink) = bus();
        let o = fast(&orchestrators(bus.clone()));
        let sent = submit_text(&o, "hi").await;
        wait_for(&sink, "agent://approval_request", 1).await;
        o.shutdown_all();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // A fresh registry over the same DB, as after an app restart
        let restarted = orchestrators(bus);
        restarted.ensure("writer");
        wait_for(&sink, "dashboard://attention", 2).await;

        let approval = restarted
            .bus()
            .db
            .call(|c| db::pending_approvals(c, None))
            .await
            .unwrap()
            .remove(0);
        restarted
            .send(
                "writer",
                OrchestratorCommand::Approve {
                    approval_id: approval.id,
                    decision: ApprovalDecision::Approved,
                },
            )
            .await
            .unwrap();
        wait_for(&sink, "agent://turn_done", 1).await;
        assert_eq!(
            run_status(&restarted, &sent.run_id).await,
            RunStatus::Completed
        );
    }

    /// The DeepSeek URL override is global: one mock at a time.
    static DEEPSEEK_MOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    type Requests = std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

    /// Serves `responses` (SSE bodies) in order and records request bodies.
    /// Hold the guard for the whole test.
    async fn mock_deepseek(
        responses: Vec<&'static str>,
    ) -> (tokio::sync::MutexGuard<'static, ()>, Requests) {
        use axum::routing::post;
        let guard = DEEPSEEK_MOCK.lock().await;
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (recorded, queue) = (
            requests.clone(),
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(
                responses,
            ))),
        );
        let app = axum::Router::new().route(
            "/chat/completions",
            post(
                move |axum::extract::Json(body): axum::extract::Json<serde_json::Value>| {
                    let (recorded, queue) = (recorded.clone(), queue.clone());
                    async move {
                        recorded.lock().unwrap().push(body);
                        let sse = queue
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or("data: [DONE]\n\n");
                        ([("content-type", "text/event-stream")], sse)
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        *crate::llm::DEEPSEEK_BASE_URL_OVERRIDE.lock().unwrap() =
            Some(format!("http://{}", listener.local_addr().unwrap()));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (guard, requests)
    }

    #[tokio::test]
    async fn llm_engine_streams_loops_on_tool_calls_and_completes() {
        const TOOL_CALL: &str = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"Try a tool.\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        const ANSWER: &str = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Here is \"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"your plan.\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":4}}\n\n",
            "data: [DONE]\n\n",
        );
        let (_mock, requests) = mock_deepseek(vec![TOOL_CALL, ANSWER]).await;

        let (bus, sink) = bus();
        let secrets = crate::services::secrets::Secrets::in_memory();
        secrets.set("nandi.llm.deepseek", "sk-test").unwrap();
        let o = Orchestrators::new(
            crate::services::orchestrator::RunDeps {
                store: crate::services::repo_store::RepoStore::for_tests(bus.db.clone()),
                bus,
                connectors: None,
                harness: Default::default(),
                secrets,
                runtimes: crate::services::runtimes::AgentRuntimes::new(
                    Default::default(),
                    std::sync::Arc::new(crate::services::egress::EgressState::new(
                        crate::services::secrets::Secrets::in_memory(),
                    )),
                ),
            },
            tokio::runtime::Handle::current(),
        );
        o.bus()
            .db
            .call(|c| db::insert_studio_conversation(c, "s1", ""))
            .await
            .unwrap();

        let sent = submit(
            &o,
            STUDIO_AGENT_ID,
            "s1".into(),
            "Plan my week".into(),
            vec![],
            None,
            RunSource::user(),
        )
        .await
        .unwrap();
        wait_for(&sink, "agent://turn_done", 1).await;

        let id = sent.run_id.clone();
        let run = o
            .bus()
            .db
            .call(move |c| db::get_run(c, &id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, RunStatus::Completed, "{:?}", run.summary);
        assert_eq!(run.summary.as_deref(), Some("Here is your plan."));

        // Studio has no tools: the call got an error result the model saw
        let results: Vec<_> = sink
            .take()
            .into_iter()
            .filter(|(name, _)| *name == "agent://tool_result")
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1["isError"], true);

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["model"], "deepseek-flash");
        assert_eq!(requests[0]["stream"], true);
        assert_eq!(requests[0]["messages"][0]["role"], "system");
        // Second call replays the assistant step (with its reasoning) and the tool result
        let second = requests[1]["messages"].as_array().unwrap();
        let assistant = second.iter().find(|m| m["role"] == "assistant").unwrap();
        assert_eq!(assistant["reasoning_content"], "Try a tool.");
        assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
        let tool = second.iter().find(|m| m["role"] == "tool").unwrap();
        assert_eq!(tool["tool_call_id"], "call_1");
        assert!(tool["content"].as_str().unwrap().contains("not available"));
    }

    #[tokio::test]
    async fn agent_answers_text_only_when_its_container_cannot_start() {
        const ANSWER: &str = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello without tools.\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":4}}

",
            "data: [DONE]

",
        );
        let (_mock, requests) = mock_deepseek(vec![ANSWER]).await;

        let (bus, sink) = bus();
        let store = crate::services::repo_store::RepoStore::for_tests(bus.db.clone());
        let skill = store.layout().skill_path("writer");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            &skill,
            "---
name: Writer
---
You write.
",
        )
        .unwrap();

        let secrets = crate::services::secrets::Secrets::in_memory();
        secrets.set("nandi.llm.deepseek", "sk-test").unwrap();
        let o = Orchestrators::new(
            crate::services::orchestrator::RunDeps {
                store: store.clone(),
                bus,
                connectors: None,
                harness: Default::default(),
                secrets,
                runtimes: crate::services::runtimes::AgentRuntimes::failing(
                    "Docker is not running",
                ),
            },
            tokio::runtime::Handle::current(),
        );

        let sent = submit_text(&o, "Hello").await;
        wait_for(&sink, "agent://turn_done", 1).await;
        let _ = std::fs::remove_dir_all(&store.layout().root);

        let id = sent.run_id.clone();
        let run = o
            .bus()
            .db
            .call(move |c| db::get_run(c, &id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, RunStatus::Completed, "{:?}", run.summary);
        assert_eq!(run.summary.as_deref(), Some("Hello without tools."));
        assert_eq!((run.input_tokens, run.output_tokens), (30, 4));
        assert_eq!(run.model.as_deref(), Some("deepseek:deepseek-flash"));

        let warnings: Vec<_> = sink
            .take()
            .into_iter()
            .filter(|(name, _)| *name == "agent://warning")
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].1["agentId"], "writer");
        assert_eq!(
            warnings[0].1["message"],
            "Tool execution unavailable: Docker is not running"
        );

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let tools = requests[0].get("tools").and_then(|t| t.as_array());
        assert!(tools.is_none_or(|t| t.is_empty()), "{tools:?}");
        let system = requests[0]["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("tools are unavailable"), "{system}");
    }

    #[tokio::test]
    async fn llm_engine_without_a_key_fails_the_turn_clearly() {
        let (bus, sink) = bus();
        let o = orchestrators(bus);
        o.bus()
            .db
            .call(|c| db::insert_studio_conversation(c, "s2", ""))
            .await
            .unwrap();

        let sent = submit(
            &o,
            STUDIO_AGENT_ID,
            "s2".into(),
            "hi".into(),
            vec![],
            None,
            RunSource::user(),
        )
        .await
        .unwrap();
        wait_for(&sink, "agent://turn_done", 1).await;

        let id = sent.run_id.clone();
        let run = o
            .bus()
            .db
            .call(move |c| db::get_run(c, &id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.summary.unwrap().contains("No LLM API key is set"));
    }

    #[tokio::test]
    async fn studio_replies_without_tools() {
        let (bus, sink) = bus();
        let o = fast(&orchestrators(bus));
        o.bus()
            .db
            .call(|c| db::insert_studio_conversation(c, "s1", ""))
            .await
            .unwrap();

        let sent = submit(
            &o,
            STUDIO_AGENT_ID,
            "s1".into(),
            "Plan my week".into(),
            vec![],
            None,
            RunSource::user(),
        )
        .await
        .unwrap();
        assert_eq!(sent.conversation_title.as_deref(), Some("Plan my week"));

        wait_for(&sink, "agent://turn_done", 1).await;
        assert!(!sink.names().contains(&"agent://tool_call"));
        assert!(
            !sink.names().contains(&"dashboard://activity"),
            "Studio isn't dashboard activity"
        );
    }

    #[tokio::test]
    async fn client_message_ids_are_used_and_validated() {
        let (bus, _sink) = bus();
        let o = fast(&orchestrators(bus));
        let conversation = o
            .bus()
            .db
            .call(|c| db::ensure_agent_conversation(c, "writer"))
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();

        assert!(submit(
            &o,
            "writer",
            conversation.clone(),
            "x".into(),
            vec![],
            Some("not-a-uuid".into()),
            RunSource::user()
        )
        .await
        .is_err());
        let sent = submit(
            &o,
            "writer",
            conversation,
            "x".into(),
            vec![],
            Some(id.clone()),
            RunSource::user(),
        )
        .await
        .unwrap();
        assert_eq!(sent.message_id, id);
    }
}
