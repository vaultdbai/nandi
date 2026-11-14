//! Delegation on one instance, with no relay: in-process delegation, the
//! tool's approval, the gateway, holds and assignments, timeouts and
//! rejections. Delegation between instances runs over the real relay
//! (`relay_world.rs`): in `relay_e2e_tests.rs` and, for the task
//! lifecycle, `lifecycle_relay_tests.rs`. This file keeps the helpers
//! they share.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::delegator::{A2aDeps, Delegator};
use super::history;
use super::*;
use crate::db;
use crate::engine::CANNED_SCRIPT_KEY;
use crate::model::{ApprovalDecision, RunStatus};
use crate::services::chat;
use crate::services::domain::{Agent, AgentId};
use crate::services::events::tests::RecordingSink;
use crate::services::relay::RelayClient;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn save_agent(state: &AppState, id: &str, frontmatter: &str) {
    let agent = Agent::from_skill_md(
        AgentId::parse(id).unwrap(),
        &format!("---\nname: {id}\n{frontmatter}---\nHelp.\n"),
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, id).unwrap();
}

/// Every agent replays `steps` (no model, no delay)
async fn canned(state: &AppState, steps: Value) {
    state
        .db()
        .call(move |conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(
                conn,
                CANNED_SCRIPT_KEY,
                &json!({ "agent": steps, "studio": [], "tokenDelayMs": 0 }),
            )
        })
        .await
        .unwrap();
}

/// Each agent answers with its input in brackets
pub(super) async fn echo_agents(state: &AppState) {
    canned(state, json!([{ "text": "[{{input}}]" }])).await;
}

/// The delegation row of `task_id`
async fn delegation(state: &AppState, task_id: &str) -> TaskResult {
    history::get(state.db(), task_id).await.unwrap().unwrap()
}

pub(super) fn instance(state: &AppState) -> String {
    crate::services::network::local_instance_id(state.db()).unwrap()
}

pub(super) fn deps(state: &AppState, relay: Option<Arc<dyn RelayClient>>) -> A2aDeps {
    A2aDeps {
        relay,
        ..A2aDeps::from_state(state).unwrap()
    }
}

/// A task service on `relay`, listening and registered for the state's
/// instance (no background loops: tests call the sweeps)
pub(crate) async fn lifecycle(
    state: &AppState,
    relay: Arc<dyn RelayClient>,
    config: task::TaskConfig,
) -> Arc<task::TaskService> {
    let service = task::TaskService::new(
        deps(state, Some(relay)),
        None,
        task::TaskConfig {
            status_timeout: Duration::from_millis(500),
            ..config
        },
        &tokio::runtime::Handle::current(),
    );
    service.listen().await;
    service.register();
    service
}

pub(crate) async fn wait_until<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    eventually(what, || {
        let checked = check();
        async move { checked.await.then_some(()) }
    })
    .await
}

/// What `find` finds, once it finds something (or panics after ~5 s)
pub(crate) async fn eventually<T, F, Fut>(what: &str, mut find: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    for _ in 0..500 {
        if let Some(found) = find().await {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The user's requester card on `instance`
pub(crate) fn user_card(instance: &str) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(instance, delegator::USER_REQUESTER),
        name: "Nandi user".into(),
        description: String::new(),
        instance_id: instance.into(),
        endpoint: String::new(),
        capabilities: Vec::new(),
        accepts_delegation: false,
        max_concurrent: 0,
        current_load: 0,
        owner: Default::default(),
        signature: None,
        issued_at: 1.0,
    }
}

pub(super) fn from_writer(capability: &str, input: Value) -> DelegationRequest {
    DelegationRequest {
        requester_agent: Some("writer".into()),
        capability: capability.into(),
        input,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The owner dismisses the first request queued on `provider` (intake)
pub(super) async fn decline_when_queued(provider: &AppState) {
    let item = eventually("a request in the intake", || async {
        let queued = provider
            .db()
            .call(|conn| crate::services::intake::pending(conn))
            .await
            .unwrap();
        queued.into_iter().next()
    })
    .await;
    crate::services::intake::dismiss(provider, item.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn agents_that_dont_accept_delegation_reject_tasks() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let delegator = state.a2a_delegator().unwrap().clone();

    // Scoring skips it. Not "free right now": it isn't busy, it doesn't
    // take delegation
    let result = delegator.delegate("summarize", json!("hi")).await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    // In plain words: no YAML key in the advice (task 5)
    let error = result.error.unwrap();
    assert!(
        error.contains("don't take requests from other Nandis"),
        "{error}"
    );
    assert!(!error.contains("accepts_delegation"), "{error}");

    // Asked by name, it refuses
    let target = format!("{}/papers", instance(&state));
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap()
            .contains("doesn't take requests from other Nandis"),
        "{:?}",
        result.error
    );
    let rows = history::list(state.db(), 10).await.unwrap();
    assert_eq!(rows.len(), 2, "failures are recorded too");
    state.shutdown();
}

#[tokio::test]
async fn an_override_can_open_or_close_one_capability() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}, {name: translate}]\naccepts_delegation: true\n\
         delegation_overrides: {translate: deny}\n",
    );
    let delegator = state.a2a_delegator().unwrap().clone();
    let target = format!("{}/papers", instance(&state));
    let denied = delegator
        .delegate_to(&target, "translate", json!("hi"))
        .await
        .unwrap();
    assert!(denied.error.unwrap().contains("doesn't take 'translate'"));
    let done = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error);
    state.shutdown();
}

#[tokio::test]
async fn local_delegation_works_without_a_relay() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    // A relay client may exist; with no relay URL it never connects
    assert!(!state.relay_client().is_some_and(|c| c.status().connected));

    let providers = list_available_providers(&state, "summarize").await.unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(list_available_providers(&state, "").await.unwrap().len(), 1);
    assert!(list_available_providers(&state, "other")
        .await
        .unwrap()
        .is_empty());

    let task_id = delegate_task(&state, "summarize", json!({"text": "hello"}), None)
        .await
        .unwrap();
    assert_eq!(
        delegation(&state, &task_id).await.status,
        TaskStatus::Running
    );
    wait_until("the delegation", || {
        let state = state.clone();
        let task_id = task_id.clone();
        async move { delegation(&state, &task_id).await.status != TaskStatus::Running }
    })
    .await;
    let result = delegation(&state, &task_id).await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert!(result.output.as_str().unwrap().contains("hello"));
    assert!(result.requester_agent.ends_with("/_user"));

    let history = list_delegation_history(&state, 50).await.unwrap();
    assert_eq!(history.len(), 1, "one row for an in-process delegation");
    let names = recorder.names();
    assert!(names.contains(&"delegation://task_started"));
    assert!(names.contains(&"delegation://task_completed"));

    // A capability only others serve needs the relay's plan: not connected
    let delegator = state.a2a_delegator().unwrap().clone();
    let result = delegator.delegate("translate", json!("hi")).await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    let error = result.error.unwrap();
    assert!(
        crate::services::relay::unavailable::RelayUnavailable::is(&error),
        "{error}"
    );
    state.shutdown();
}

#[tokio::test]
async fn the_delegation_tool_waits_for_approval() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    canned(
        &state,
        json!([
            {
                "text": "Let me ask a colleague.",
                // A delegation needs a `delegate` step being done; its
                // approval still applies on top (run plans)
                "toolCalls": [{
                    "tool": "plan_set",
                    "args": {"steps": [{"title": "Ask for a summary", "kind": "delegate"}]},
                    "result": "unused"
                }, {
                    "tool": "plan_update",
                    "args": {"step": 1, "status": "doing"},
                    "result": "unused"
                }, {
                    "tool": "delegate_to_agent",
                    "args": {"capability": "summarize", "input": {"text": "hello"}},
                    "result": "unused"
                }]
            },
            { "text": "[{{input}}]" }
        ]),
    )
    .await;
    save_agent(&state, "writer", "delegation_outbound: true\n");
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );

    let sent = chat::send_message(&state, "writer".into(), "Summarize".into(), vec![], None)
        .await
        .unwrap();
    let pending = || {
        let db = state.db().clone();
        async move {
            db.call(|conn| db::pending_approvals(conn, Some("writer")))
                .await
                .unwrap()
        }
    };
    wait_until("the approval request", || async move {
        !pending().await.is_empty()
    })
    .await;
    let approval = pending().await.remove(0);
    assert_eq!(approval.tool, "delegate_to_agent");
    assert!(
        history::list(state.db(), 10).await.unwrap().is_empty(),
        "nothing is delegated before approval"
    );

    chat::approve_action(&state, None, approval.id, ApprovalDecision::Approved)
        .await
        .unwrap();
    let run_id = sent.run_id.clone();
    wait_until("the writer's run", || {
        let db = state.db().clone();
        let id = run_id.clone();
        async move {
            db.call(move |conn| db::get_run(conn, &id))
                .await
                .unwrap()
                .is_some_and(|run| run.status.is_finished())
        }
    })
    .await;
    let id = sent.run_id.clone();
    let run = state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let rows = history::list(state.db(), 10).await.unwrap();
    assert_eq!(rows.len(), 1, "papers (no outbound) couldn't delegate");
    assert_eq!(rows[0].status, TaskStatus::Completed, "{:?}", rows[0].error);
    assert!(rows[0].provider_agent.ends_with("/papers"));
    assert!(rows[0].requester_agent.ends_with("/writer"));

    // The tool result carries the provider's answer back to the writer
    let id = sent.run_id.clone();
    let messages = state
        .db()
        .call(move |conn| db::list_run_messages(conn, &id))
        .await
        .unwrap();
    let tool_result = messages
        .iter()
        .find(|m| m.role == db::role::TOOL_CALL && m.metadata["tool"] == "delegate_to_agent")
        .expect("a tool call message");
    assert!(
        tool_result.metadata.to_string().contains("hello") || tool_result.content.contains("hello"),
        "{tool_result:?}"
    );
    state.shutdown();
}

#[tokio::test]
async fn declaring_approvals_without_the_tool_skips_approval() {
    let state = AppState::for_tests(vec![]);
    save_agent(
        &state,
        "auto",
        "delegation_outbound: true\nrequires_approval: [shell]\n",
    );
    save_agent(&state, "careful", "delegation_outbound: true\n");
    save_agent(&state, "plain", "");
    let gate = |id: &str| tool::policy(state.store(), id);
    assert!(gate("auto").offered && !gate("auto").requires_approval_for(tool::NAME));
    assert!(gate("careful").requires_approval_for(tool::NAME));
    assert!(!gate("careful").requires_approval_for("shell"));
    assert!(!gate("plain").offered);
    state.shutdown();
}

#[tokio::test]
async fn settings_are_written_to_skill_md() {
    let state = AppState::for_tests(vec![]);
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let settings = AgentDelegationSettings {
        allow_outbound: true,
        accept_inbound: true,
        max_concurrent: Some(2),
        overrides: vec![DelegationOverride {
            capability: "summarize".into(),
            policy: TrustPolicy::Allow,
        }],
    };
    set_agent_delegation(&state, "papers", settings.clone())
        .await
        .unwrap();
    assert_eq!(
        get_agent_delegation(&state, "papers").await.unwrap(),
        settings
    );
    let md = std::fs::read_to_string(state.store().layout().skill_path("papers")).unwrap();
    assert!(md.contains("accepts_delegation: true"), "{md}");
    assert!(md.contains("delegation_outbound: true"), "{md}");
    assert!(state
        .store()
        .agent("papers")
        .unwrap()
        .capabilities()
        .accepts_delegation());
    assert!(get_agent_delegation(&state, "nope").await.is_err());
    state.shutdown();
}

#[tokio::test]
async fn a_rejected_pick_falls_back_to_the_next_provider() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "closed",
        "serves: [{name: summarize}]\naccepts_delegation: true\n\
         delegation_overrides: {summarize: deny}\n",
    );
    save_agent(
        &state,
        "open",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let me = instance(&state);
    // Pinned, so it's tried first; it rejects the task without running it
    crate::services::shared::set_provider_pin(&state, "summarize", Some(&format!("{me}/closed")))
        .await
        .unwrap();

    let overview = crate::services::shared::list_capability_providers(&state, Some("summarize"))
        .await
        .unwrap();
    assert_eq!(overview.len(), 1);
    assert_eq!(
        overview[0].pinned.as_deref(),
        Some(format!("{me}/closed").as_str())
    );
    assert_eq!(overview[0].auto_choice, overview[0].pinned);
    assert_eq!(overview[0].providers.len(), 2);

    let delegator = state.a2a_delegator().unwrap().clone();
    let result = delegator.delegate("summarize", json!("hi")).await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.provider_agent, format!("{me}/open"));

    let history = list_delegation_history(&state, 50).await.unwrap();
    assert_eq!(history.len(), 2, "the final row and the rejected attempt");
    let attempt = history
        .iter()
        .find(|row| row.task_id.ends_with("-attempt1"))
        .expect("the rejected attempt keeps a row");
    assert_eq!(attempt.provider_agent, format!("{me}/closed"));
    assert!(attempt
        .error
        .as_deref()
        .unwrap()
        .contains("passed to the next provider"));
    state.shutdown();
}

#[tokio::test]
async fn a_named_provider_is_never_swapped() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "closed",
        "serves: [{name: summarize}]\naccepts_delegation: true\n\
         delegation_overrides: {summarize: deny}\n",
    );
    save_agent(
        &state,
        "open",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let target = format!("{}/closed", instance(&state));
    let delegator = state.a2a_delegator().unwrap().clone();
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert_eq!(result.provider_agent, target);
    assert_eq!(list_delegation_history(&state, 50).await.unwrap().len(), 1);
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Task lifecycle (over the real relay in `lifecycle_relay_tests.rs`)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn eta_estimates_come_from_history() {
    let state = AppState::for_tests(vec![]);
    let none = task::get_eta_estimate(&state, "summarize", None)
        .await
        .unwrap();
    assert_eq!(none.sample_count, 0);
    assert_eq!(
        none.median_seconds,
        crate::services::a2a::eta::DEFAULT_MEDIAN_SECONDS
    );
    assert!(task::get_eta_estimate(&state, " ", None).await.is_err());
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Wiring: gateway tasks, trust, clarification pauses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gateway_tasks_go_to_the_gateway_handler() {
    let state = AppState::for_tests(vec![]);
    let me = instance(&state);
    let target = AgentCard::card_id(&me, crate::services::gateway::GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();

    // The hook `gateway::start` registered: the gateway answers (whatever it
    // says without NANDI.md or a model), never "no such agent"
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    let error = result.error.unwrap_or_default();
    assert!(!error.contains("no agent"), "{error}");
    assert!(!error.contains("isn't an agent"), "{error}");

    // A handler that routes, then one that refuses
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let handled = seen.clone();
    executor::set_gateway_handler(
        &me,
        Arc::new(move |task: Task| {
            handled.lock().unwrap().push(task.target.clone());
            Box::pin(async move {
                let mut result = history::placeholder(
                    &task.id,
                    &task.capability,
                    &task.input,
                    Some(&task.target),
                );
                result.output = json!("routed");
                history::finish(&mut result, TaskStatus::Completed, None);
                Ok(result)
            })
        }),
    );
    let result = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.output, json!("routed"));
    assert_eq!(*seen.lock().unwrap(), std::slice::from_ref(&target));

    executor::set_gateway_handler(
        &me,
        Arc::new(|_task: Task| {
            Box::pin(async {
                Err(task::Refusal::new(
                    task::ReasonKind::NotAccepting,
                    task::TaskStage::Policy,
                    "Not for you",
                ))
            })
        }),
    );
    let refused = delegator
        .delegate_to(&target, "summarize", json!("hi"))
        .await
        .unwrap();
    assert_eq!(refused.status, TaskStatus::Failed);
    assert_eq!(refused.error.as_deref(), Some("Not for you"));
    state.shutdown();
}

#[tokio::test]
async fn the_gateway_reaches_agents_that_dont_take_direct_delegation() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    // Behind the gateway only: no `accepts_delegation`
    save_agent(
        &state,
        "quiet",
        "serves: [{name: summarize}]
",
    );
    let me = instance(&state);
    let delegator = state.a2a_delegator().unwrap().clone();
    let request = |requester_agent: Option<&str>| DelegationRequest {
        task_id: None,
        requester_agent: requester_agent.map(str::to_string),
        // The gateway passes what the caller asked for, maybe "gateway"
        capability: crate::services::gateway::GATEWAY_AGENT_ID.to_string(),
        input: json!("What is 2+2?"),
        target: Some(AgentCard::card_id(&me, "quiet")),
    };

    let routed = delegator
        .delegate_request(request(Some(crate::services::gateway::GATEWAY_AGENT_ID)))
        .await
        .unwrap();
    assert_eq!(routed.status, TaskStatus::Completed, "{:?}", routed.error);
    assert!(routed.output.to_string().contains("What is 2+2?"));

    // Anyone else still needs the agent to accept delegation
    let direct = delegator.delegate_request(request(None)).await.unwrap();
    assert_eq!(direct.status, TaskStatus::Failed);
    state.shutdown();
}

#[tokio::test]
async fn each_delegated_task_runs_in_its_own_conversation() {
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]
accepts_delegation: true
",
    );
    let me = instance(&state);
    let delegator = state.a2a_delegator().unwrap().clone();
    let mut tasks = Vec::new();
    for text in ["Alice's private request", "Carol's request"] {
        let result = delegator
            .delegate_request(DelegationRequest {
                capability: "summarize".into(),
                input: json!(text),
                target: Some(AgentCard::card_id(&me, "papers")),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
        tasks.push(result.task_id);
    }
    let conversations: Vec<String> = state
        .db()
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT conversation_id FROM runs WHERE agent_id = 'papers' ORDER BY started_at",
            )?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .await
        .unwrap();
    // Neither sees the other, nor the owner's chat with the agent
    let expected: Vec<String> = tasks.iter().map(|t| db::task_conversation_id(t)).collect();
    assert_eq!(conversations, expected);
    assert!(!conversations.contains(&db::agent_conversation_id("papers")));
    state.shutdown();
}

/// The pending gateway hold, once one appears
async fn wait_for_hold(state: &AppState) -> crate::services::gateway::holds::GatewayHold {
    eventually("a gateway hold", || async {
        let holds = crate::services::gateway::list_gateway_holds(state)
            .await
            .unwrap();
        holds.into_iter().next()
    })
    .await
}

/// Task 5: "Needs my approval" holds the request instead of refusing it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_capability_needing_approval_waits_for_the_owner() {
    use crate::services::gateway::{self, holds::HoldStatus, GATEWAY_AGENT_ID};
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec!["summarize".into()];
    gateway::set_gateway_config(&state, config).await.unwrap();
    let target = AgentCard::card_id(&instance(&state), GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();
    let ask = |text: &str| {
        let (delegator, target) = (delegator.clone(), target.clone());
        let input = json!({ "text": text });
        tokio::spawn(async move {
            delegator
                .delegate_request(DelegationRequest {
                    capability: "summarize".into(),
                    input,
                    target: Some(target),
                    ..Default::default()
                })
                .await
                .unwrap()
        })
    };

    // Held, listed for the Inbox, and it goes on once approved
    let approved = ask("Summarize Q3 risk");
    let hold = wait_for_hold(&state).await;
    assert_eq!(hold.status, HoldStatus::Pending);
    assert_eq!(hold.capability.as_deref(), Some("summarize"));
    assert_eq!(hold.summary, "Summarize Q3 risk");
    assert!(!approved.is_finished(), "it waits");
    gateway::decide_gateway_hold(&state, hold.task_id.clone(), true, None)
        .await
        .unwrap();
    let result = approved.await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    // Decided once only
    assert!(
        gateway::decide_gateway_hold(&state, hold.task_id, false, None)
            .await
            .is_err()
    );

    // Declined: refused with the owner's no, not a policy reason
    let declined = ask("Summarize Q4 risk");
    let hold = wait_for_hold(&state).await;
    gateway::decide_gateway_hold(&state, hold.task_id, false, None)
        .await
        .unwrap();
    let result = declined.await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("declined"),
        "{:?}",
        result.error
    );
    assert!(gateway::list_gateway_holds(&state)
        .await
        .unwrap()
        .is_empty());
    state.shutdown();
}

/// Inbox Ask chats: a refusal confirmed in the hold's chat goes back to the
/// requester with the owner's reason, and is recorded with the chat
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refusal_from_the_holds_chat_tells_the_requester_why() {
    use crate::services::gateway::{self, GATEWAY_AGENT_ID};
    use crate::services::inbox;
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]
",
    );
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec!["summarize".into()];
    gateway::set_gateway_config(&state, config).await.unwrap();
    let target = AgentCard::card_id(&instance(&state), GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();
    let declined = tokio::spawn(async move {
        delegator
            .delegate_request(DelegationRequest {
                capability: "summarize".into(),
                input: json!({ "text": "Summarize Q4 risk" }),
                target: Some(target),
                ..Default::default()
            })
            .await
            .unwrap()
    });
    let hold = wait_for_hold(&state).await;
    let thread = format!("inbox:gatewayHold:{}", hold.task_id);
    let opened = inbox::thread::open(&state, thread.clone()).await.unwrap();
    assert!(opened.tools.contains(&"inbox_refuse".to_string()));
    let outcome = inbox::tool::call(
        &state,
        &thread,
        inbox::tool::REFUSE,
        &json!({ "reason": "Not before the board meeting" }),
    )
    .await;
    assert!(!outcome.is_error, "{}", outcome.result);
    let result = declined.await.unwrap();
    assert_eq!(result.status, TaskStatus::Failed);
    let error = result.error.unwrap_or_default();
    assert!(
        error.contains("declined this request: Not before the board meeting"),
        "{error}"
    );
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    let decisions = state
        .store()
        .repos()
        .decisions
        .read(&month)
        .unwrap()
        .unwrap();
    assert!(
        decisions.contains("- **Decision:** declined"),
        "{decisions}"
    );
    assert!(
        decisions.contains("- **Reason:** Not before the board meeting"),
        "{decisions}"
    );
    assert!(
        decisions.contains(&format!("- **Chat:** `{thread}`")),
        "{decisions}"
    );
    // Decided: the chat has nothing left to decide
    let again = inbox::tool::call(&state, &thread, inbox::tool::APPROVE, &json!({})).await;
    assert!(again.is_error);
    state.shutdown();
}

/// The pending assignment, once one appears
async fn wait_for_assignment(
    state: &AppState,
) -> crate::services::gateway::assignments::GatewayAssignment {
    eventually("a gateway assignment", || async {
        let open = crate::services::gateway::list_gateway_assignments(state)
            .await
            .unwrap();
        open.into_iter().next()
    })
    .await
}

/// The delegation's history row once it has ended
pub(super) async fn wait_for_history_end(state: &AppState, task_id: &str) -> TaskResult {
    eventually(&format!("the delegation {task_id} to end"), || async {
        let row = history::get(state.db(), task_id).await.unwrap();
        row.filter(|r| r.status != TaskStatus::Running)
    })
    .await
}

/// Work assigned to the owner (your own gateway): the call returns once
/// it's held or on the list, as it does for someone else. Approved, it
/// isn't routed to an agent but waits in the Inbox until the owner marks it
/// Done (the note is the result) or Declines it (refused with the note)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn assigned_work_waits_for_the_owners_done_or_decline() {
    use crate::services::a2a::assignment::ASSIGN_ME;
    use crate::services::gateway::{self, assignments::AssignmentStatus, GATEWAY_AGENT_ID};
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(
        &state,
        "papers",
        "serves: [{name: assign-me}]
",
    );
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec![ASSIGN_ME.into()];
    gateway::set_gateway_config(&state, config).await.unwrap();
    let target = AgentCard::card_id(&instance(&state), GATEWAY_AGENT_ID);
    let delegator = state.a2a_delegator().unwrap().clone();
    let assign = |text: &str, links: Value| {
        let (delegator, target) = (delegator.clone(), target.clone());
        let input = json!({ "text": text, "due": "2027-03-01", "links": links });
        async move {
            let started = db::now_ms();
            let result = delegator
                .delegate_request(DelegationRequest {
                    capability: ASSIGN_ME.into(),
                    input,
                    target: Some(target),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);
            assert!(
                db::now_ms() - started < 5_000,
                "it didn't wait for the owner"
            );
            result
        }
    };

    // Held for approval: the call has returned already
    let asked = assign("Upload the SOC 1 evidence", json!([])).await;
    assert_eq!(asked.output["state"], "delivered");
    let hold = wait_for_hold(&state).await;
    assert_eq!(hold.task_id, asked.task_id);
    gateway::decide_gateway_hold(&state, hold.task_id.clone(), true, None)
        .await
        .unwrap();
    let assignment = wait_for_assignment(&state).await;
    assert_eq!(assignment.task_id, asked.task_id);
    assert_eq!(assignment.request, "Upload the SOC 1 evidence");
    assert_eq!(assignment.status, AssignmentStatus::Open);
    assert!(assignment.due_at.is_some());
    // The owner holds it: no run of the agent started for it
    let runs: i64 = state
        .db()
        .call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE agent_id = 'papers'",
                [],
                |row| row.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(runs, 0, "no agent did it for the owner");
    assert_eq!(
        history::get(state.db(), &asked.task_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Running,
        "no agent did it for the owner"
    );
    let closed = gateway::close_gateway_assignment(
        &state,
        assignment.task_id.clone(),
        true,
        Some("Uploaded to the audit folder".into()),
    )
    .await
    .unwrap();
    assert_eq!(closed.status, AssignmentStatus::Done);
    let result = wait_for_history_end(&state, &asked.task_id).await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.output["status"], "done");
    assert_eq!(result.output["note"], "Uploaded to the audit folder");
    // Closed once only
    assert!(
        gateway::close_gateway_assignment(&state, assignment.task_id, false, None)
            .await
            .is_err()
    );

    // Declined: refused with the owner's note
    let declined = assign("Rotate the keys", json!([])).await;
    let hold = wait_for_hold(&state).await;
    gateway::decide_gateway_hold(&state, hold.task_id, true, None)
        .await
        .unwrap();
    let assignment = wait_for_assignment(&state).await;
    gateway::close_gateway_assignment(
        &state,
        assignment.task_id,
        false,
        Some("Not my system".into()),
    )
    .await
    .unwrap();
    let result = wait_for_history_end(&state, &declined.task_id).await;
    assert_eq!(result.status, TaskStatus::Failed);
    let error = result.error.unwrap_or_default();
    assert!(error.contains("declined it: Not my system"), "{error}");
    assert!(gateway::list_gateway_assignments(&state)
        .await
        .unwrap()
        .is_empty());

    // Without approval it goes straight onto the list
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.require_approval = vec![];
    gateway::set_gateway_config(&state, config).await.unwrap();
    // with the links the requester attached
    let listed = assign(
        "Book the auditor",
        json!([{ "url": "https://cal.example.com/audit", "name": "Auditor's calendar" }]),
    )
    .await;
    assert_eq!(listed.output["state"], "acknowledged");
    let assignment = wait_for_assignment(&state).await;
    assert_eq!(assignment.task_id, listed.task_id);
    assert_eq!(assignment.links.len(), 1);
    assert_eq!(assignment.links[0].name, "Auditor's calendar");
    assert_eq!(assignment.links[0].url, "https://cal.example.com/audit");
    state.shutdown();
}

/// Task 5: a busy gateway queues the request instead of refusing it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_gateway_queues_the_request_until_a_slot_frees() {
    use crate::services::gateway::{self, GATEWAY_AGENT_ID, GATEWAY_LINE};
    let state = AppState::for_tests(vec![]);
    echo_agents(&state).await;
    save_agent(&state, "papers", "serves: [{name: summarize}]\n");
    let mut config = gateway::get_gateway_config(&state).await.unwrap();
    config.policy.max_concurrent = 1;
    gateway::set_gateway_config(&state, config).await.unwrap();
    let me = instance(&state);
    let service = state.gateway().unwrap().clone();
    let manifest = service.manifest().unwrap();
    // Another request holds the only slot
    let caller = service.caller_policy(&me, "").await.unwrap();
    let busy = service.acquire(&caller, &manifest).unwrap();

    let delegator = state.a2a_delegator().unwrap().clone();
    let target = AgentCard::card_id(&me, GATEWAY_AGENT_ID);
    let queued = tokio::spawn(async move {
        delegator
            .delegate_request(DelegationRequest {
                capability: "summarize".into(),
                input: json!({ "text": "Summarize this" }),
                target: Some(target),
                ..Default::default()
            })
            .await
            .unwrap()
    });
    // It waits first in line, not refused
    wait_until("the request in line", || async {
        service.slots().waiting(GATEWAY_LINE) > 0
    })
    .await;
    assert_eq!(
        service.slots().waiting(GATEWAY_LINE),
        1,
        "queued, alone in line (position 1)"
    );
    assert!(!queued.is_finished());

    drop(busy);
    let result = queued.await.unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(service.slots().waiting(GATEWAY_LINE), 0);
    state.shutdown();
}

#[tokio::test]
async fn a_task_waiting_for_an_answer_doesnt_time_out() {
    let state = AppState::for_tests(vec![]);
    // A turn of ~12 × 40ms, well past the 150ms timeout
    state
        .db()
        .call(|conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(
                conn,
                CANNED_SCRIPT_KEY,
                &json!({
                    "agent": [{ "text": "one two three four five six seven eight nine ten eleven twelve" }],
                    "studio": [],
                    "tokenDelayMs": 40
                }),
            )
        })
        .await
        .unwrap();
    save_agent(
        &state,
        "papers",
        "serves: [{name: summarize}]\naccepts_delegation: true\n",
    );
    let me = instance(&state);
    let delegator = Delegator::new(A2aDeps {
        timeout: Duration::from_millis(150),
        ..deps(&state, None)
    });
    let task = |id: &str| Task {
        id: id.into(),
        requester: user_card(&me),
        target: format!("{me}/papers"),
        capability: "summarize".into(),
        input: json!("hi"),
        created_at: db::now_ms() as f64,
        deadline: None,
    };

    // Not waiting: the timeout fires
    let timed_out = delegator.executor().execute(&task("plain")).await;
    assert_eq!(timed_out.status, TaskStatus::TimedOut);

    // Its row is `InputRequired` (the requester's human is answering)
    let waiting = task("waiting");
    let record = task::TaskRecord::requester(
        &waiting.id,
        &me,
        "_user",
        &waiting.target,
        &waiting.capability,
        &waiting.input,
        None,
    );
    task::create(state.db(), state.bus(), &record, None)
        .await
        .unwrap();
    task_state::advance(
        state.db(),
        state.bus(),
        &waiting.id,
        task::TaskState::InputRequired,
        task_state::Change::reason("asked"),
    )
    .await
    .unwrap()
    .unwrap();
    let done = delegator.executor().execute(&waiting).await;
    assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error);
    state.shutdown();
}
