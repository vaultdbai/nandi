//! The task lifecycle over the real relay (`relay_world.rs`): both sides'
//! rows and events, the outboxes, restarts, retries, status and cancels,
//! and what a row does with what its provider or the relay says.

use std::sync::Arc;

use serde_json::{json, Value};

use super::history;
use super::relay_world::{Nandi, RelayWorld, TestRelay};
use super::task::{TaskFilter, TaskRecord, TaskSourceKind, TaskState};
use super::tests::{eventually, user_card, wait_until};
use super::*;
use crate::db;
use crate::services::a2a::assignment::ASSIGN_ME;
use crate::services::a2a::returns::{self, ReturnOutcome};
use crate::services::relay::client::WsRelayClient;
use crate::services::relay::protocol::PeerMessage;
use crate::services::relay::RelayClient;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A `summarize` task from the user on `requester` to `target`
fn wire(id: &str, requester: &str, target: &str, input: Value) -> Task {
    Task {
        id: id.into(),
        requester: user_card(requester),
        target: target.into(),
        capability: "summarize".into(),
        input,
        created_at: db::now_ms() as f64,
        deadline: None,
    }
}

/// Card `id` (`instance/agent`), serving `summarize`
fn serving(id: &str) -> AgentCard {
    let (instance, agent) = id.split_once('/').unwrap();
    AgentCard {
        id: id.into(),
        name: agent.into(),
        capabilities: vec![crate::services::domain::capability::ServesCapability {
            name: "summarize".into(),
            description: String::new(),
            input_schema: None,
            output_schema: None,
        }],
        accepts_delegation: true,
        max_concurrent: 2,
        ..user_card(instance)
    }
}

/// The requester's row of `task`, as the user's, in `state`
fn asked(task: &Task, state: TaskState) -> TaskRecord {
    let mut record = TaskRecord::requester(
        &task.id,
        &task.requester.instance_id,
        delegator::USER_REQUESTER,
        &task.target,
        &task.capability,
        &task.input,
        None,
    );
    record.state = state;
    record
}

/// The provider's row of `task`, in `state`
fn served(task: &Task, state: TaskState) -> TaskRecord {
    let provider = task.target.split('/').next().unwrap();
    let mut record = TaskRecord::provider(task, provider);
    record.state = state;
    record
}

/// Keeps `record` (and the request it sends, if any) on `nandi`
async fn create(nandi: &Nandi, record: &TaskRecord, request: Option<&Task>) {
    let state = &nandi.state;
    task::create(state.db(), state.bus(), record, request)
        .await
        .unwrap();
}

fn states(events: &[task::TaskEvent]) -> Vec<TaskState> {
    events.iter().map(|e| e.to_state).collect()
}

async fn row(nandi: &Nandi, id: &str) -> TaskRecord {
    task::get_row(nandi.state.db(), id).await.unwrap().unwrap()
}

async fn state_of(nandi: &Nandi, id: &str) -> Option<TaskState> {
    let row = task::get_row(nandi.state.db(), id).await.unwrap();
    row.map(|r| r.state)
}

async fn wait_for_state(nandi: &Nandi, id: &str, state: TaskState) {
    wait_until(&format!("{id} {state:?}"), || async {
        state_of(nandi, id).await == Some(state)
    })
    .await;
}

/// The results waiting in `nandi`'s outbox
async fn kept_results(nandi: &Nandi) -> Vec<String> {
    let results = nandi.state.db().call(result_outbox::waiting).await.unwrap();
    results.into_iter().map(|result| result.task_id).collect()
}

/// A requester with `writer` on its own relay, and "them", a bare member
/// serving `summarize` as `them/papers`
async fn requester_alone() -> (TestRelay, Nandi, Arc<WsRelayClient>) {
    let relay = TestRelay::start().await;
    let log_dir = tempfile::tempdir().unwrap();
    let writer = [("writer", "delegation_outbound: true\n")];
    let requester = Nandi::join(&relay, &writer, log_dir.path()).await;
    let them = relay.member("them", vec![serving("them/papers")]).await;
    (relay, requester, them)
}

/// The provider's gateway: puts a task on its owner's list due in
/// `eta_seconds`, and answers `output` once `release` says (as a person
/// does, however late). Its row goes `Acknowledged`, then `Working`, as
/// `gateway::router` does it; the requester hears the acknowledgement only
/// (the harness's state has no connected client: `relay_world::Nandi`).
fn gateway_answering_when_released(
    provider: &Nandi,
    release: Arc<tokio::sync::Notify>,
    output: Value,
    eta_seconds: Option<u32>,
) {
    let (state, relay) = (provider.state.clone(), provider.relay.clone());
    executor::set_gateway_handler(
        &provider.instance(),
        Arc::new(move |task: Task| {
            let (state, relay) = (state.clone(), relay.clone());
            let (release, output) = (release.clone(), output.clone());
            Box::pin(async move {
                let note = "On Lena's list";
                let acknowledged = PeerMessage::AcknowledgeTask {
                    task_id: task.id.clone(),
                    eta_seconds,
                    note: Some(note.into()),
                };
                let requester = task.requester.instance_id.clone();
                relay.send_peer(requester, acknowledged).await.unwrap();
                executor::gateway_dispatched(&state, &task, note.into(), eta_seconds).await;
                release.notified().await;
                let mut result = history::placeholder(
                    &task.id,
                    &task.capability,
                    &task.input,
                    Some(&task.target),
                );
                // The requester, as `gateway::router` names it
                result.requester_agent = task.requester.id.clone();
                result.output = output;
                history::finish(&mut result, TaskStatus::Completed, None);
                Ok(result)
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// A delegation end to end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_cross_instance_delegation_walks_the_lifecycle_on_both_sides() {
    let world = RelayWorld::allowing().await;
    let provider = world.provider.instance();
    let result = world.ask_for(json!({"text": "hi"})).await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);

    let asked = row(&world.requester, &result.task_id).await;
    assert_eq!(asked.state, TaskState::Completed);
    assert_eq!(asked.requester_agent, "writer");
    assert_eq!(asked.provider_instance, provider);
    assert_eq!(asked.provider_agent, "papers");
    assert_eq!(asked.attempt_number, 1);
    assert_eq!(asked.source_kind, TaskSourceKind::Direct);
    assert!(asked.result.is_some());
    let requester_db = world.requester.state.db();
    let path = states(
        &task::events_of(requester_db, &result.task_id)
            .await
            .unwrap(),
    );
    assert_eq!(
        &path[..3],
        [
            TaskState::Submitted,
            TaskState::Delivered,
            TaskState::Acknowledged
        ]
    );
    assert_eq!(path.last(), Some(&TaskState::Completed));

    let served = row(&world.provider, &result.task_id).await;
    assert_eq!(served.state, TaskState::Completed);
    assert_eq!(served.provider_instance, provider);
    assert!(served.eta_seconds.is_some(), "acknowledged with an ETA");
    let provider_db = world.provider.state.db();
    let path = states(&task::events_of(provider_db, &result.task_id).await.unwrap());
    assert_eq!(
        path,
        [
            TaskState::Submitted,
            TaskState::Delivered,
            TaskState::Acknowledged,
            TaskState::Working,
            TaskState::Completed
        ]
    );
    let samples: u32 = provider_db
        .call(|conn| conn.query_row("SELECT COUNT(*) FROM eta_history", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(samples, 1, "the completion feeds the ETA");
    let relay = &world.relay;
    assert_eq!(relay.peer_count(&provider, "task_delivered"), 1);
    assert_eq!(relay.peer_count(&provider, "acknowledge_task"), 1);
    // One "started" update always: it's how the requester sees `Working`
    assert_eq!(relay.peer_count(&provider, "progress_update"), 1);
    // The delegation history is unchanged
    let history = history::get(requester_db, &result.task_id).await.unwrap();
    assert_eq!(history.unwrap().status, TaskStatus::Completed);
    world.shutdown();
}

#[tokio::test]
async fn a_gateway_task_gets_a_provider_row_with_the_routing_reason() {
    let world = RelayWorld::new(super::relay_world::PAPERS).await;
    let provider = world.provider.instance();
    let gateway = world.provider.open_gateway().await;
    // A gateway that routes, as `handle_gateway_task` does
    let state = world.provider.state.clone();
    executor::set_gateway_handler(
        &provider,
        Arc::new(move |task: Task| {
            let state = state.clone();
            Box::pin(async move {
                executor::gateway_dispatched(&state, &task, "Routed to papers: math".into(), None)
                    .await;
                let mut result = history::placeholder(
                    &task.id,
                    &task.capability,
                    &task.input,
                    Some(&task.target),
                );
                // The requester, as `gateway::router` names it
                result.requester_agent = task.requester.id.clone();
                result.output = json!("4");
                history::finish(&mut result, TaskStatus::Completed, None);
                Ok(result)
            })
        }),
    );
    let result = world
        .requester
        .delegator
        .run(DelegationRequest {
            capability: "gateway".into(),
            input: json!({"text": "What is 2+2?"}),
            target: Some(gateway),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.output, json!("4"));

    let served = row(&world.provider, &result.task_id).await;
    assert_eq!(served.state, TaskState::Completed);
    let events = task::events_of(world.provider.state.db(), &result.task_id)
        .await
        .unwrap();
    assert_eq!(
        states(&events),
        [
            TaskState::Submitted,
            TaskState::Delivered,
            TaskState::Acknowledged,
            TaskState::Working,
            TaskState::Completed
        ]
    );
    assert_eq!(events[2].reason.as_deref(), Some("Routed to papers: math"));
    world.shutdown();
}

#[tokio::test]
async fn progress_is_sent_only_when_the_task_wants_it() {
    let world = RelayWorld::allowing().await;
    let provider = world.provider.instance();
    let result = world
        .ask_for(json!({"text": "hi", "wants_progress": true}))
        .await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert!(world.relay.peer_count(&provider, "progress_update") >= 1);
    let db = world.requester.state.db();
    let messages = eventually("the progress message", || async {
        let messages = task::messages_of(db, &result.task_id).await.unwrap();
        let has = messages.iter().any(|m| m.kind == task::PROGRESS_KIND);
        has.then_some(messages)
    })
    .await;
    // The relay's reasons come first: why it went there
    assert_eq!(messages[0].kind, route::ROUTE_KIND);
    assert!(
        messages[0].payload["reasons"][0]["code"].is_string(),
        "{}",
        messages[0].payload
    );

    // Heartbeats: only working tasks that want progress
    let requester = world.requester.instance();
    let target = world.papers();
    for (id, input) in [
        ("hb-wants", json!({"wants_progress": true})),
        ("hb-quiet", json!({"text": "x"})),
    ] {
        let task = wire(id, &requester, &target, input);
        world.carry(&task).await;
        create(&world.requester, &asked(&task, TaskState::Delivered), None).await;
        create(
            &world.provider,
            &served(&task, TaskState::Working),
            Some(&task),
        )
        .await;
    }
    let before = world.relay.peer_sent(&provider).len();
    assert_eq!(world.provider.service.heartbeat().await.unwrap(), 1);
    let note = eventually("the heartbeat", || async {
        let messages = task::messages_of(db, "hb-wants").await.unwrap();
        let progress = messages.iter().find(|m| m.kind == task::PROGRESS_KIND)?;
        progress.payload["note"].as_str().map(str::to_string)
    })
    .await;
    assert_eq!(note, "still working");
    let sent = world.relay.peer_sent(&provider)[before..].to_vec();
    assert_eq!(sent, [("progress_update".to_string(), "hb-wants".into())]);
    assert!(!world
        .provider
        .service
        .send_progress("hb-quiet", Some(50), None)
        .await
        .unwrap());
    world.shutdown();
}

// ---------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------

/// An assignment lasts: the requester's call returns once the person has
/// it, and the result comes back however late, kept at the relay while
/// the requester is offline
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_assignment_stays_open_until_its_person_ends_it() {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    let world = RelayWorld::new(super::relay_world::PAPERS).await;
    let gateway = world.provider.open_gateway().await;
    let release = Arc::new(tokio::sync::Notify::new());
    let done = json!({"status": "done", "note": "Uploaded"});
    gateway_answering_when_released(
        &world.provider,
        release.clone(),
        done.clone(),
        Some(DAY as u32 / 1000),
    );

    let started = db::now_ms();
    let result = world
        .requester
        .delegator
        .run(DelegationRequest {
            capability: ASSIGN_ME.into(),
            input: json!({"text": "Upload the SOC 1 evidence", "due": "3 days"}),
            target: Some(gateway),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);
    assert!(
        db::now_ms() - started < 5_000,
        "it didn't wait for the work"
    );
    let open = row(&world.requester, &result.task_id).await;
    assert!(!open.state.is_terminal(), "{:?}", open.state);
    let due = open.deadline.unwrap() as i64;
    assert!((due - started - 3 * DAY).abs() < 60_000, "due in 3 days");
    let db = world.requester.state.db();
    let history = history::get(db, &result.task_id).await.unwrap().unwrap();
    assert_eq!(history.status, TaskStatus::Running);

    // Done while we're offline: the result waits at the relay
    let me = world.requester.instance();
    let provider = world.provider.instance();
    world.relay.faults.set_online(&me, false);
    release.notify_one();
    wait_until("the result at the relay", || async {
        world.relay.results_sent(&provider) == 1
    })
    .await;
    assert!(!row(&world.requester, &result.task_id)
        .await
        .state
        .is_terminal());
    world.relay.bring_back(&world.requester.relay, &me).await;
    wait_for_state(&world.requester, &result.task_id, TaskState::Completed).await;
    let ended = row(&world.requester, &result.task_id).await;
    assert_eq!(ended.result, Some(done.clone()));
    // The history row ends just after the task row
    let history = super::tests::wait_for_history_end(&world.requester.state, &result.task_id).await;
    assert_eq!(history.output, done);
    world.shutdown();
}

/// Declined while it waited for the person's approval: the requester's
/// open history row ends and the Inbox gets "{name} declined it"
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_assignment_declined_while_held_comes_back_as_declined() {
    let world = RelayWorld::new(super::relay_world::PAPERS).await;
    let gateway = world.provider.open_gateway().await;
    let decline = Arc::new(tokio::sync::Notify::new());
    let (relay, declined) = (world.provider.relay.clone(), decline.clone());
    executor::set_gateway_handler(
        &world.provider.instance(),
        Arc::new(move |task: Task| {
            let (declined, relay) = (declined.clone(), relay.clone());
            Box::pin(async move {
                let requester = task.requester.instance_id.clone();
                let waiting = PeerMessage::ProgressUpdate {
                    task_id: task.id.clone(),
                    percent: None,
                    note: Some("Waiting for the owner to approve".into()),
                    reason_kind: Some(task::ReasonKind::NeedsTheirApproval),
                    queue_position: None,
                };
                relay.send_peer(requester.clone(), waiting).await.unwrap();
                declined.notified().await;
                let no = PeerMessage::RejectTask {
                    task_id: task.id.clone(),
                    reason: "The owner declined this request".into(),
                    reason_kind: Some(task::ReasonKind::Declined),
                };
                relay.send_peer(requester, no).await.unwrap();
                std::future::pending::<()>().await;
                unreachable!()
            })
        }),
    );
    let result = world
        .requester
        .delegator
        .run(DelegationRequest {
            capability: ASSIGN_ME.into(),
            input: json!({"text": "Collect the screenshots", "due": "3 days"}),
            target: Some(gateway),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);

    decline.notify_one();
    let ended = super::tests::wait_for_history_end(&world.requester.state, &result.task_id).await;
    assert_eq!(ended.status, TaskStatus::Failed);
    assert_eq!(
        row(&world.requester, &result.task_id).await.state,
        TaskState::Rejected
    );
    let returned = returns::list(world.requester.state.db()).await.unwrap();
    let item = returned
        .iter()
        .find(|r| r.task_id == result.task_id)
        .expect("a return for the declined request");
    assert_eq!(item.outcome, ReturnOutcome::Declined);
    assert_eq!(item.request, "Collect the screenshots");
    world.shutdown();
}

// ---------------------------------------------------------------------------
// Outboxes: what waits for the relay
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_outbox_flushes_in_priority_order_once_the_relay_is_back() {
    let (relay, requester, _them) = requester_alone().await;
    let me = requester.instance();
    relay.faults.set_online(&me, false);
    wait_until("the requester offline", || async {
        !requester.relay.status().connected
    })
    .await;
    for (id, priority, created_at) in [("old", 0, 1.0), ("urgent", 5, 3.0), ("new", 0, 2.0)] {
        let mut record = TaskRecord::requester(
            id,
            &me,
            "writer",
            "them/papers",
            "summarize",
            &json!({ "priority": priority }),
            None,
        );
        record.created_at = created_at;
        assert_eq!(record.priority, priority);
        let task = wire(id, &me, "them/papers", record.input.clone());
        create(&requester, &record, Some(&task)).await;
    }
    // No relay: everything waits
    let service = &requester.service;
    assert!(service.flush().await.is_empty());
    assert!(relay.submitted(&me).is_empty());
    assert_eq!(
        state_of(&requester, "old").await,
        Some(TaskState::Submitted)
    );

    relay.bring_back(&requester.relay, &me).await;
    // Two flushes at once (the relay connecting + the sweep) send each
    // request once
    let (first, second) = tokio::join!(service.flush(), service.flush());
    assert_eq!([first, second].concat(), ["urgent", "old", "new"]);
    assert_eq!(relay.submitted(&me), ["urgent", "old", "new"]);
    for id in ["urgent", "old", "new"] {
        assert_eq!(state_of(&requester, id).await, Some(TaskState::Delivered));
    }
    assert!(service.flush().await.is_empty(), "nothing left");
    requester.state.shutdown();
}

/// A result finished while the provider was offline waits in its outbox,
/// goes on the next connect, and stays until the relay says it has it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_result_the_relay_missed_goes_on_connect_and_stays_until_the_relay_has_it() {
    let world = RelayWorld::new(super::relay_world::PAPERS).await;
    let gateway = world.provider.open_gateway().await;
    let release = Arc::new(tokio::sync::Notify::new());
    gateway_answering_when_released(&world.provider, release.clone(), json!("done"), None);
    let requester = world.requester.delegator.clone();
    let running = tokio::spawn(async move {
        requester
            .run(DelegationRequest {
                capability: "gateway".into(),
                input: json!({"text": "hi"}),
                target: Some(gateway),
                ..Default::default()
            })
            .await
    });
    // On the provider's list; it finishes once the relay is away
    let provider = world.provider.instance();
    let on_list = eventually("the provider's row", || async {
        let rows = task::list_rows(world.provider.state.db(), &provider, TaskFilter::default())
            .await
            .unwrap();
        rows.into_iter().next().map(|row| row.id)
    })
    .await;
    wait_for_state(&world.provider, &on_list, TaskState::Working).await;
    world.relay.faults.set_online(&provider, false);
    release.notify_one();
    wait_until("the kept result", || async {
        kept_results(&world.provider).await == [on_list.clone()]
    })
    .await;
    assert_eq!(world.relay.results_sent(&provider), 0);

    world
        .relay
        .bring_back(&world.provider.relay, &provider)
        .await;
    let service = &world.provider.service;
    assert_eq!(service.flush_results().await, 1);
    let result = running.await.unwrap().unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(
        kept_results(&world.provider).await,
        std::slice::from_ref(&result.task_id),
        "kept until the relay says it has it"
    );

    // Sent again on the next connect, the relay answers `task_ended`
    assert_eq!(service.flush_results().await, 1);
    wait_until("the dropped result", || async {
        kept_results(&world.provider).await.is_empty()
    })
    .await;
    world.shutdown();
}

#[tokio::test]
async fn a_submission_whose_answer_was_lost_is_sent_again_and_its_result_kept() {
    let world = RelayWorld::allowing().await;
    let (me, provider) = (world.requester.instance(), world.provider.instance());
    world.relay.faults.set_online(&provider, false);
    world.relay.faults.drop_next(&me, "deliver.event.accepted");
    let requester = world.requester.delegator.clone();
    let target = world.papers();
    let running = tokio::spawn(async move {
        requester
            .delegate_to(&target, "summarize", json!("lost answer"))
            .await
    });
    // Not abandoned: it waits `Submitted`, and the outbox sends it again
    // until the relay answers
    let service = &world.requester.service;
    let resent = eventually("the resend", || async {
        Some(service.flush().await).filter(|resent| !resent.is_empty())
    })
    .await;

    world
        .relay
        .bring_back(&world.provider.relay, &provider)
        .await;
    let result = running.await.unwrap().unwrap();
    assert_eq!(resent, std::slice::from_ref(&result.task_id));
    assert_eq!(
        world.relay.submitted(&me),
        [result.task_id.clone(), result.task_id.clone()],
        "sent twice"
    );
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(
        state_of(&world.requester, &result.task_id).await,
        Some(TaskState::Completed)
    );
    world.shutdown();
}

#[tokio::test]
async fn a_delegation_to_an_offline_provider_waits_at_the_relay() {
    let world = RelayWorld::allowing().await;
    let (me, provider) = (world.requester.instance(), world.provider.instance());
    world.relay.faults.set_online(&provider, false);
    let requester = world.requester.delegator.clone();
    let target = world.papers();
    let running = tokio::spawn(async move {
        requester
            .delegate_to(&target, "summarize", json!("queued work"))
            .await
    });
    // Held: the relay took it, and the row, still `Submitted`, says it
    // waits there until they're online
    let db = world.requester.state.db();
    let held = eventually("the held task", || async {
        let filter = TaskFilter {
            state: Some("submitted".into()),
            ..TaskFilter::default()
        };
        let waiting = task::list_tasks(&world.requester.state, filter).await;
        let id = waiting.unwrap().first()?.id.clone();
        let events = task::events_of(db, &id).await.unwrap();
        let note = events.last()?;
        (note.reason_kind == Some(task::ReasonKind::Offline)).then(|| note.clone())
    })
    .await;
    assert_eq!(held.to_state, TaskState::Submitted);
    assert_eq!(held.stage, Some(task::TaskStage::Dispatch));
    assert_eq!(world.relay.submitted(&me).len(), 1);
    // The relay has it: nothing for the outbox
    assert!(world.requester.service.flush().await.is_empty());

    world
        .relay
        .bring_back(&world.provider.relay, &provider)
        .await;
    let result = running.await.unwrap().unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(
        state_of(&world.requester, &result.task_id).await,
        Some(TaskState::Completed)
    );
    assert_eq!(world.relay.submitted(&me).len(), 1, "never sent again");
    world.shutdown();
}

// ---------------------------------------------------------------------------
// Restarts, retries, status and cancels
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_restart_resumes_in_flight_tasks() {
    let world = RelayWorld::allowing().await;
    let requester = world.requester.instance();
    let target = world.papers();

    // Provider: a task that was working when the app stopped, and the
    // requester's row of it waiting for the result
    let task = wire("resumed", &requester, &target, json!("again"));
    world.carry(&task).await;
    create(
        &world.provider,
        &served(&task, TaskState::Working),
        Some(&task),
    )
    .await;
    create(
        &world.requester,
        &asked(&task, TaskState::Acknowledged),
        None,
    )
    .await;
    // Requester: a queued task
    let queued = wire("queued", &requester, &target, json!("later"));
    create(
        &world.requester,
        &asked(&queued, TaskState::Submitted),
        Some(&queued),
    )
    .await;

    let resumed = world.provider.service.resume().await.unwrap();
    assert_eq!(resumed, ["resumed"]);
    let events = task::events_of(world.provider.state.db(), "resumed")
        .await
        .unwrap();
    let resume = events.last().unwrap();
    assert_eq!(resume.from_state, Some(TaskState::Working));
    assert_eq!(resume.to_state, TaskState::Working);
    assert_eq!(resume.reason.as_deref(), Some(task_state::RESUMED_REASON));

    let mut ids = world.requester.service.resume().await.unwrap();
    ids.sort();
    assert_eq!(ids, ["queued", "resumed"]);

    for (nandi, id) in [
        (&world.provider, "resumed"),
        (&world.requester, "resumed"),
        (&world.requester, "queued"),
    ] {
        wait_for_state(nandi, id, TaskState::Completed).await;
    }
    let done = row(&world.requester, "resumed").await;
    assert!(done.result.unwrap().as_str().unwrap().contains("again"));
    world.shutdown();
}

#[tokio::test]
async fn a_retry_creates_the_next_attempt_elsewhere() {
    let (relay, requester, _them) = requester_alone().await;
    let me = requester.instance();
    let task = wire("first", &me, "them/papers", json!("x"));
    let mut failed = TaskRecord::requester(
        "first",
        &me,
        "writer",
        "them/papers",
        "summarize",
        &json!("x"),
        None,
    );
    failed.state = TaskState::Failed;
    create(&requester, &failed, Some(&task)).await;

    // Nobody else serves it at the relay: nowhere to go
    let service = &requester.service;
    assert!(service.retry(&failed).await.unwrap().is_none());
    let _other = relay.member("other", vec![serving("other/papers")]).await;

    let next = service.retry(&failed).await.unwrap().unwrap();
    assert_eq!(next.attempt_number, 2);
    assert_eq!(next.previous_attempt_id.as_deref(), Some("first"));
    assert_eq!(next.source_kind, TaskSourceKind::Failover);
    assert_eq!(next.provider_instance, "other", "never the failed one");
    assert_eq!(next.state, TaskState::Delivered);
    assert_eq!(relay.submitted(&me), std::slice::from_ref(&next.id));
    let sent = task::request_of(requester.state.db(), &next.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent.target, "other/papers");
    assert_eq!(sent.input, json!("x"));
    requester.state.shutdown();
}

#[tokio::test]
async fn status_queries_and_cancels_reach_the_provider() {
    let world = RelayWorld::new(super::relay_world::PAPERS).await;
    let provider = world.provider.instance();
    let gateway = world.provider.open_gateway().await;
    // On the owner's list for 42 s, never done
    let never = Arc::new(tokio::sync::Notify::new());
    gateway_answering_when_released(&world.provider, never, json!("unused"), Some(42));
    let result = world
        .requester
        .delegator
        .run(DelegationRequest {
            capability: ASSIGN_ME.into(),
            input: json!({"text": "Check the logs", "due": "3 days"}),
            target: Some(gateway),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Running, "{:?}", result.error);
    let c = result.task_id.as_str();
    // The provider works on it; the requester heard it acknowledged
    wait_for_state(&world.provider, c, TaskState::Working).await;
    wait_for_state(&world.requester, c, TaskState::Acknowledged).await;

    // A live query: what the provider's row says
    let service = &world.requester.service;
    let live = service.query_status(c).await.unwrap();
    assert!(live.live);
    assert_eq!(live.state, TaskState::Working);
    assert_eq!(live.eta_seconds, Some(42));
    assert_eq!(row(&world.requester, c).await.eta_seconds, Some(42));

    // The query lost on the way: what we know here
    let faults = &world.relay.faults;
    faults.drop_next(&provider, "deliver.peer.status_query");
    let local = service.query_status(c).await.unwrap();
    assert!(!local.live);
    assert_eq!(local.state, TaskState::Acknowledged);

    let cancelled = service
        .cancel(c, Some("no longer needed".into()))
        .await
        .unwrap();
    assert_eq!(cancelled.state, TaskState::Cancelled);
    wait_for_state(&world.provider, c, TaskState::Cancelled).await;
    assert!(service.cancel(c, None).await.is_err());

    let mine = task::list_tasks(
        &world.requester.state,
        TaskFilter {
            role: Some("requester".into()),
            ..TaskFilter::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(mine.len(), 1);
    let events = task::get_task_events(&world.requester.state, c)
        .await
        .unwrap();
    assert_eq!(
        events.last().unwrap().reason.as_deref(),
        Some("no longer needed")
    );
    assert!(task::get_task(&world.requester.state, "nope")
        .await
        .is_err());
    world.shutdown();
}

// ---------------------------------------------------------------------------
// What a requester's row does with what it hears
// ---------------------------------------------------------------------------

/// Something a delivered task's requester hears about it
#[derive(Debug, Clone, Copy)]
enum Heard {
    /// Its provider has it, done in about this many seconds
    Acknowledged(u32),
    /// Its provider is on it
    Progress,
    /// Its provider turns it down, with no one next
    Rejected,
    /// Someone who isn't its provider turns it down: the relay refuses it
    RejectedByAnIntruder,
    /// The relay ends it past its due date
    Expired,
}

impl Heard {
    /// Makes it happen to task "t" from `me` at "them"
    async fn happen(self, relay: &TestRelay, me: &str, them: &WsRelayClient) {
        let task_id = "t".to_string();
        let said = match self {
            Self::Acknowledged(eta) => PeerMessage::AcknowledgeTask {
                task_id,
                eta_seconds: Some(eta),
                note: None,
            },
            Self::Progress => PeerMessage::ProgressUpdate {
                task_id,
                percent: Some(10),
                note: None,
                reason_kind: None,
                queue_position: None,
            },
            Self::Rejected | Self::RejectedByAnIntruder => PeerMessage::RejectTask {
                task_id,
                reason: "no".into(),
                reason_kind: None,
            },
            Self::Expired => return relay.faults.expire("t"),
        };
        if let Self::RejectedByAnIntruder = self {
            let intruder = relay.member("intruder", Vec::new()).await;
            intruder.send_peer(me.to_string(), said).await.unwrap();
            wait_until("the relay's refusal", || async {
                let told = relay
                    .faults
                    .kinds("intruder", nandi_relay::testing::Direction::Sent);
                told.contains(&"error".to_string())
            })
            .await;
        } else {
            them.send_peer(me.to_string(), said).await.unwrap();
        }
    }
}

#[tokio::test]
async fn a_delivered_tasks_row_moves_only_on_what_its_provider_or_the_relay_says() {
    // (what's heard, the row's state after it, its ETA)
    let cases = [
        (Heard::Acknowledged(7), TaskState::Acknowledged, Some(7)),
        (Heard::Progress, TaskState::Working, None),
        (Heard::Rejected, TaskState::Rejected, None),
        (Heard::RejectedByAnIntruder, TaskState::Delivered, None),
        (Heard::Expired, TaskState::TimedOut, None),
    ];
    for (heard, expected, eta) in cases {
        let (relay, requester, them) = requester_alone().await;
        let me = requester.instance();
        let task = wire("t", &me, "them/papers", json!(1));
        create(&requester, &asked(&task, TaskState::Delivered), None).await;
        requester.relay.delegate_task(&task, &[]).await.unwrap();

        heard.happen(&relay, &me, &them).await;
        let reached = || async { state_of(&requester, "t").await == Some(expected) };
        wait_until(&format!("{heard:?}: {expected:?}"), reached).await;
        assert_eq!(row(&requester, "t").await.eta_seconds, eta, "{heard:?}");
        requester.state.shutdown();
    }
}
