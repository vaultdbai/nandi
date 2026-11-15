//! Delegation between instances over the real relay (`relay_world.rs`):
//! the app's own WebSocket client on every side, sealing on, nothing
//! mocked. The task lifecycle over it is in `lifecycle_relay_tests.rs`.

use std::time::Duration;

use serde_json::json;

use super::history;
use super::relay_world::{RelayWorld, Setup, PAPERS};
use super::tests::{
    decline_when_queued, from_writer, save_agent, user_card, wait_for_history_end, wait_until,
};
use super::*;
use crate::db;
use crate::engine::CANNED_SCRIPT_KEY;
use crate::services::events::tests::RecordingSink;
use crate::services::network::member_trust::{
    set_blocked, update_relationship, TrustUpdate, ALL_AGENTS,
};
use crate::services::trust;

/// A live wait well inside a test's patience
const LIVE_WAIT: Duration = Duration::from_millis(150);

#[tokio::test]
async fn one_instance_delegates_to_another_through_the_relay() {
    let world = RelayWorld::allowing().await;
    let requester_instance = world.requester.instance();

    let providers = world.requester.delegator.providers("summarize").await;
    assert_eq!(providers.len(), 1, "the provider's card, through the relay");
    assert_eq!(providers[0].id, world.papers());

    let result = world
        .ask_for(json!({"text": "Attention is all you need"}))
        .await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.direction, DelegationDirection::Outbound);
    assert_eq!(result.provider_agent, world.papers());
    assert_eq!(
        result.requester_agent,
        format!("{requester_instance}/writer")
    );
    let output = result.output.as_str().unwrap();
    assert!(output.contains("Attention is all you need"), "{output}");

    // Both sides keep history
    let asked = history::get(world.requester.state.db(), &result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked.status, TaskStatus::Completed);
    let served = history::list(world.provider.state.db(), 10).await.unwrap();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].direction, DelegationDirection::Inbound);
    assert_eq!(served[0].status, TaskStatus::Completed);
    assert_eq!(served[0].requester_agent, result.requester_agent);

    // The provider ran it as a delegated turn
    let source: (String, Option<String>) = world
        .provider
        .state
        .db()
        .call(|conn| {
            conn.query_row(
                "SELECT source_type, source_id FROM runs WHERE agent_id = 'papers'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(source.0, "agent");
    assert_eq!(source.1.as_deref(), Some(result.requester_agent.as_str()));

    // Audit: started + finished on each side
    let log_name = format!("{}.log", world.provider.instance());
    let log = std::fs::read_to_string(world.log_dir.path().join(log_name)).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert!(!log.contains("Attention"), "no task content in the log");
    world.shutdown();
}

#[tokio::test]
async fn remote_requesters_need_an_allowing_trust_policy() {
    let world = RelayWorld::new(PAPERS).await;
    // No rule: `ask`, which waits for the owner (`services::intake`);
    // dismissed, it's declined
    let (result, ()) = tokio::join!(world.ask(), decline_when_queued(&world.provider.state));
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result.error.as_deref().unwrap().contains("declined"),
        "{:?}",
        result.error
    );

    trust::set_policy(
        world.provider.state.db(),
        &result.requester_agent,
        "papers",
        trust::TrustPolicy::Deny,
    )
    .unwrap();
    let denied = world.ask().await;
    assert!(denied.error.unwrap().contains("doesn't take requests from"));
    world.shutdown();
}

#[tokio::test]
async fn a_trusted_peer_passes_the_ask_policy() {
    let world = RelayWorld::new(PAPERS).await;
    // Default policy `ask`: a stranger waits for the owner (`services::intake`);
    // dismissed, it's declined
    let (refused, ()) = tokio::join!(world.ask(), decline_when_queued(&world.provider.state));
    assert_eq!(refused.status, TaskStatus::Failed);
    assert!(refused.error.unwrap_or_default().contains("declined"));

    // Marked trusted in Network: let in
    update_relationship(
        world.provider.state.db(),
        &world.requester.instance(),
        &TrustUpdate {
            added_as_trusted_peer: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let allowed = world.ask().await;
    assert_eq!(allowed.status, TaskStatus::Completed, "{:?}", allowed.error);

    // An explicit deny still wins
    trust::set_policy(
        world.provider.state.db(),
        trust::ANY_REQUESTER,
        "papers",
        trust::TrustPolicy::Deny,
    )
    .unwrap();
    assert_eq!(world.ask().await.status, TaskStatus::Failed);
    world.shutdown();
}

/// How the provider's owner shuts out the requester's instance
#[derive(Debug, Clone, Copy)]
enum ShutOut {
    Paused,
    Blocked,
}

impl ShutOut {
    fn set(self, world: &RelayWorld, on: bool) {
        let provider = world.provider.state.db();
        let requester = world.requester.instance();
        match self {
            Self::Paused => {
                let paused = TrustUpdate {
                    delegations_paused: Some(on),
                    ..Default::default()
                };
                update_relationship(provider, &requester, &paused).unwrap();
            }
            Self::Blocked => {
                set_blocked(provider, &requester, ALL_AGENTS, on).unwrap();
            }
        }
    }
}

#[tokio::test]
async fn a_shut_out_requester_instance_is_refused_on_arrival_until_let_back_in() {
    // (how, the word in the refusal)
    let cases = [(ShutOut::Paused, "paused"), (ShutOut::Blocked, "blocked")];
    for (how, said) in cases {
        let world = RelayWorld::allowing().await;
        how.set(&world, true);
        let refused = world.ask().await;
        assert_eq!(refused.status, TaskStatus::Failed, "{how:?}");
        let error = refused.error.unwrap_or_default();
        assert!(error.contains(said), "{how:?}: {error}");

        // Let back in: the same request goes through
        how.set(&world, false);
        let result = world.ask().await;
        assert_eq!(
            result.status,
            TaskStatus::Completed,
            "{how:?}: {:?}",
            result.error
        );
        world.shutdown();
    }
}

#[tokio::test]
async fn blocked_agents_and_paused_instances_are_never_picked() {
    let world = RelayWorld::new(PAPERS).await;
    let provider = world.provider.instance();
    let db = world.requester.state.db();
    let delegator = &world.requester.delegator;
    assert_eq!(delegator.providers("summarize").await.len(), 1);

    set_blocked(db, &provider, "papers", true).unwrap();
    assert!(delegator.providers("summarize").await.is_empty());
    let result = world.ask().await;
    assert_eq!(result.status, TaskStatus::Failed);
    // The relay knows a provider serves it, but none it may pick
    let error = result.error.unwrap_or_default();
    assert!(error.contains("Nobody who serves"), "{error}");
    // Named, it's refused before it's sent
    let named = delegator
        .run(DelegationRequest {
            target: Some(world.papers()),
            ..from_writer("summarize", json!("hi"))
        })
        .await
        .unwrap();
    assert!(named.error.unwrap().contains("blocked"));

    set_blocked(db, &provider, "papers", false).unwrap();
    update_relationship(
        db,
        &provider,
        &TrustUpdate {
            delegations_paused: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(delegator.providers("summarize").await.is_empty());
    // Our own instance is never filtered
    let me = world.requester.instance();
    let cards = vec![user_card(&me)];
    assert_eq!(failover::without_untrusted(db, &me, cards).len(), 1);
    world.shutdown();
}

#[tokio::test]
async fn my_own_agent_wins_unless_i_pinned_someone_elses() {
    let world = RelayWorld::allowing().await;
    save_agent(&world.requester.state, "local", PAPERS);
    let me = world.requester.instance();

    // Mine runs here, with no relay
    let result = world.ask().await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.provider_agent, format!("{me}/local"));
    assert!(world.relay.submitted(&me).is_empty(), "nothing went out");

    // Pinned and free at the relay: theirs first
    let remote = world.papers();
    crate::services::shared::set_provider_pin(&world.requester.state, "summarize", Some(&remote))
        .await
        .unwrap();
    let result = world.ask().await;
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.provider_agent, remote);
    world.shutdown();
}

#[tokio::test]
async fn a_request_with_no_answer_in_the_live_wait_stays_open_until_the_relay_ends_it() {
    let recorder = Arc::new(RecordingSink::default());
    let setup = Setup {
        live_wait: Some(LIVE_WAIT),
        recorder: Some(recorder.clone()),
    };
    let world = RelayWorld::with(PAPERS, setup).await;
    world.provider.allow_everyone("papers");
    // The provider never gets it: nothing answers within the live wait
    let faults = &world.relay.faults;
    faults.drop_next(&world.provider.instance(), "deliver.task");

    let started = db::now_ms();
    let result = world.ask().await;
    assert_eq!(result.status, TaskStatus::Running);
    assert_eq!(result.output["waiting"], json!(true));
    let db = world.requester.state.db();
    let row = task::get_row(db, &result.task_id).await.unwrap().unwrap();
    assert!(!row.state.is_terminal(), "{:?}", row.state);
    let due = row.deadline.unwrap() as i64;
    assert!(due >= started + assignment::DEFAULT_REQUEST_DUE_DAYS * 86_400_000);
    let open = history::get(db, &result.task_id).await.unwrap().unwrap();
    assert_eq!(open.status, TaskStatus::Running);

    // The relay ends it past its due date: so does its history row
    faults.expire(&result.task_id);
    let closed = wait_for_history_end(&world.requester.state, &result.task_id).await;
    assert_eq!(closed.status, TaskStatus::TimedOut);
    assert!(closed.error.unwrap().contains("due date"));
    assert!(recorder.names().contains(&"delegation://task_failed"));
    world.shutdown();
}

#[tokio::test]
async fn an_answer_that_comes_after_the_live_wait_is_kept() {
    let recorder = Arc::new(RecordingSink::default());
    let setup = Setup {
        live_wait: Some(LIVE_WAIT),
        recorder: Some(recorder.clone()),
    };
    let world = RelayWorld::with(PAPERS, setup).await;
    world.provider.allow_everyone("papers");
    // The provider gets it after the live wait has ended
    let late = LIVE_WAIT * 3;
    let faults = &world.relay.faults;
    faults.delay_next(&world.provider.instance(), "deliver.task", late);

    let result = world.ask_for(json!("done later")).await;
    assert_eq!(result.status, TaskStatus::Running);

    // The answer arrives when nobody waits on it live
    let ended = wait_for_history_end(&world.requester.state, &result.task_id).await;
    assert_eq!(ended.status, TaskStatus::Completed, "{:?}", ended.error);
    assert!(ended.output.to_string().contains("done later"));
    assert!(recorder.names().contains(&"delegation://task_completed"));
    // It's an Inbox item, and the companion hears about it
    let returns = returns::list(world.requester.state.db()).await.unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].outcome, returns::ReturnOutcome::Answered);
    assert!(returns[0].summary.contains("done later"), "{returns:?}");
    assert!(recorder.names().contains(&"request://returned"));
    world.shutdown();
}

#[tokio::test]
async fn a_provider_reassigns_a_task_and_the_requester_gets_the_result() {
    let world = RelayWorld::allowing().await;
    let alice = world.provider.instance();
    // Alice is slow: her agent streams a long reply
    let long = "word ".repeat(400);
    world
        .provider
        .state
        .db()
        .call(move |conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(
                conn,
                CANNED_SCRIPT_KEY,
                &json!({ "agent": [{ "text": long }], "studio": [], "tokenDelayMs": 50 }),
            )
        })
        .await
        .unwrap();
    let bob_nandi = world.join_provider().await;
    let bob = bob_nandi.instance();

    let requester = world.requester.delegator.clone();
    let target = world.papers();
    let run = tokio::spawn(async move {
        requester
            .run(DelegationRequest {
                target: Some(target),
                ..from_writer("summarize", json!("hand me on"))
            })
            .await
    });

    // Alice has it and is on it
    let alice_service = &world.provider.service;
    let provider_db = world.provider.state.db();
    let taken = || async {
        let rows = task::list_rows(provider_db, &alice, task::TaskFilter::default())
            .await
            .unwrap();
        rows.into_iter()
            .find(|r| {
                matches!(
                    r.state,
                    task::TaskState::Acknowledged | task::TaskState::Working
                )
            })
            .map(|r| r.id)
    };
    wait_until("Alice to pick the task up", || async {
        taken().await.is_some()
    })
    .await;
    let task_id = taken().await.unwrap();

    // Bob is the only teammate she can hand it to
    let candidates = reassign::reassign_candidates_for(alice_service, &task_id)
        .await
        .unwrap();
    let ids: Vec<&str> = candidates.iter().map(|c| c.instance_id.as_str()).collect();
    assert_eq!(ids, [bob.as_str()]);
    let not_back = reassign::reassign(
        alice_service,
        reassign::ReassignRequest {
            task_id: task_id.clone(),
            new_provider_instance: world.requester.instance(),
            reason: None,
        },
    )
    .await;
    assert!(not_back.unwrap_err().contains("back to"));

    let row = reassign::reassign(
        alice_service,
        reassign::ReassignRequest {
            task_id: task_id.clone(),
            new_provider_instance: bob.clone(),
            reason: Some("swamped".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(row.state, task::TaskState::Reassigned);

    // The requester never resubmitted, and Bob's echo is the result
    let result = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("the delegation followed the reassignment")
        .unwrap()
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert!(
        result.output.to_string().contains("hand me on"),
        "{}",
        result.output
    );
    assert!(
        result.provider_agent.starts_with(&format!("{bob}/")),
        "the answer is Bob's, not Alice's: {}",
        result.provider_agent
    );
    assert_eq!(world.relay.submitted(&world.requester.instance()).len(), 1);

    // One task, one row: it followed Bob, with the move in its thread
    let requester_db = world.requester.state.db();
    let asked = task::get_row(requester_db, &task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked.state, task::TaskState::Completed);
    assert_eq!(asked.provider_instance, bob);
    let notes = task::messages_of(requester_db, &task_id).await.unwrap();
    let note = notes
        .iter()
        .find(|m| m.kind == reassign::REASSIGNED_KIND)
        .expect("a reassigned note in the thread");
    assert_eq!(note.payload["toInstance"], json!(bob));
    assert_eq!(note.payload["reason"], json!("swamped"));
    // Bob saw an ordinary delivered task, under the same ID
    let served = task::get_row(bob_nandi.state.db(), &task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(served.state, task::TaskState::Completed);

    bob_nandi.state.shutdown();
    world.shutdown();
}
