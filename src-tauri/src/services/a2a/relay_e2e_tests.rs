//! Delegation between two instances over the real relay
//! (`nandi_relay::serve_open` on 127.0.0.1:0, the dev-dependency): the
//! app's own WebSocket client on both sides, sealing on, nothing mocked.
//!
//! Each test starts its own relay: the relay plans by capability across
//! every instance it knows, so a relay shared between tests would hand one
//! test's request to another test's provider. Starting one takes a few
//! milliseconds (in-memory store).
//!
//! The cases that need a fault the real relay can't be told to make (an
//! answer lost, an instance dropped at a chosen moment) stay on the mock
//! relay in `tests.rs`.

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::json;

use super::delegator::{A2aDeps, Delegator};
use super::history::{self, AuditLog};
use super::tests::{
    decline_when_queued, deps, echo_agents, from_writer, instance, lifecycle, save_agent, user_card,
};
use super::*;
use crate::services::relay::client::WsRelayClient;
use crate::services::relay::RelayClient;
use crate::services::team::trust::{set_blocked, update_relationship, TrustUpdate, ALL_AGENTS};
use crate::services::trust;

/// The provider's agent: serves `summarize` and takes delegation
const PAPERS: &str = "serves: [{name: summarize}]\naccepts_delegation: true\n";

/// A relay of its own for one test
async fn relay() -> SocketAddr {
    nandi_relay::serve_open(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap()
}

/// `state`'s client on the relay at `addr`, connected
async fn client(addr: SocketAddr, state: &AppState) -> Arc<WsRelayClient> {
    let me = instance(state);
    let client = Arc::new(WsRelayClient::new(None, me.clone()));
    client
        .connect(&format!("ws://{addr}/relay"), &me)
        .await
        .unwrap();
    client
}

/// Two app instances on a real relay: a requester with `writer`, and a
/// provider with `papers` serving `summarize`
struct RelayWorld {
    requester_state: AppState,
    provider_state: AppState,
    requester: Arc<Delegator>,
    log_dir: tempfile::TempDir,
    /// Kept alive: the relay clients, the provider's delegator and both
    /// task services (the registry holds services weakly)
    _relays: [Arc<WsRelayClient>; 2],
    _provider: Arc<Delegator>,
    _services: [Arc<task::TaskService>; 2],
}

impl RelayWorld {
    async fn new(provider_frontmatter: &str) -> Self {
        let addr = relay().await;
        let requester_state = AppState::for_tests(vec![]);
        let provider_state = AppState::for_tests(vec![]);
        echo_agents(&provider_state).await;
        save_agent(&requester_state, "writer", "delegation_outbound: true\n");
        save_agent(&provider_state, "papers", provider_frontmatter);

        let requester_relay = client(addr, &requester_state).await;
        let provider_relay = client(addr, &provider_state).await;
        let requester_service = lifecycle(
            &requester_state,
            requester_relay.clone(),
            Default::default(),
        )
        .await;
        let provider_service =
            lifecycle(&provider_state, provider_relay.clone(), Default::default()).await;
        let log_dir = tempfile::tempdir().unwrap();
        let delegator = |state: &AppState, relay: &Arc<WsRelayClient>, log: &str| {
            Delegator::new(A2aDeps {
                audit: AuditLog::at(log_dir.path().join(log)),
                ..deps(state, Some(relay.clone()))
            })
        };
        let requester = delegator(&requester_state, &requester_relay, "requester.log");
        let provider = delegator(&provider_state, &provider_relay, "provider.log");
        requester.clone().register_relay_handlers().await;
        provider.clone().register_relay_handlers().await;
        // Both sides register, as the app does once connected: the relay
        // answers only registered instances
        for (relay, delegator) in [(&requester_relay, &requester), (&provider_relay, &provider)] {
            relay.register(delegator.local_cards("")).await.unwrap();
        }
        Self {
            requester_state,
            provider_state,
            requester,
            log_dir,
            _relays: [requester_relay, provider_relay],
            _provider: provider,
            _services: [requester_service, provider_service],
        }
    }

    /// The provider lets anyone ask `papers`
    fn allow_everyone(&self) {
        trust::set_policy(
            self.provider_state.db(),
            trust::ANY_REQUESTER,
            "papers",
            trust::TrustPolicy::Allow,
        )
        .unwrap();
    }

    async fn ask(&self) -> TaskResult {
        self.requester
            .run(from_writer("summarize", json!("hi")))
            .await
            .unwrap()
    }

    fn shutdown(self) {
        self.requester_state.shutdown();
        self.provider_state.shutdown();
    }
}

#[tokio::test]
async fn one_instance_delegates_to_another_through_the_relay() {
    let world = RelayWorld::new(PAPERS).await;
    world.allow_everyone();
    let requester_instance = instance(&world.requester_state);
    let provider_instance = instance(&world.provider_state);

    let providers = world.requester.providers("summarize").await;
    assert_eq!(providers.len(), 1, "the provider's card, through the relay");
    assert_eq!(providers[0].id, format!("{provider_instance}/papers"));

    let result = world
        .requester
        .run(from_writer(
            "summarize",
            json!({"text": "Attention is all you need"}),
        ))
        .await
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert_eq!(result.direction, DelegationDirection::Outbound);
    assert_eq!(result.provider_agent, format!("{provider_instance}/papers"));
    assert_eq!(
        result.requester_agent,
        format!("{requester_instance}/writer")
    );
    let output = result.output.as_str().unwrap();
    assert!(output.contains("Attention is all you need"), "{output}");

    // Both sides keep history
    let asked = history::get(world.requester_state.db(), &result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asked.status, TaskStatus::Completed);
    let served = history::list(world.provider_state.db(), 10).await.unwrap();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].direction, DelegationDirection::Inbound);
    assert_eq!(served[0].status, TaskStatus::Completed);
    assert_eq!(served[0].requester_agent, result.requester_agent);

    // The provider ran it as a delegated turn
    let source: (String, Option<String>) = world
        .provider_state
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
    let log = std::fs::read_to_string(world.log_dir.path().join("provider.log")).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert!(!log.contains("Attention"), "no task content in the log");
    world.shutdown();
}

#[tokio::test]
async fn remote_requesters_need_an_allowing_trust_policy() {
    let world = RelayWorld::new(PAPERS).await;
    // No rule: `ask`, which waits for the owner (`services::intake`);
    // dismissed, it's declined
    let (result, ()) = tokio::join!(world.ask(), decline_when_queued(&world.provider_state));
    assert_eq!(result.status, TaskStatus::Failed);
    assert!(
        result.error.as_deref().unwrap().contains("declined"),
        "{:?}",
        result.error
    );

    trust::set_policy(
        world.provider_state.db(),
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
    let (refused, ()) = tokio::join!(world.ask(), decline_when_queued(&world.provider_state));
    assert_eq!(refused.status, TaskStatus::Failed);
    assert!(refused.error.unwrap_or_default().contains("declined"));

    // Marked trusted in Network: let in
    update_relationship(
        world.provider_state.db(),
        &instance(&world.requester_state),
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
        world.provider_state.db(),
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
        let provider = world.provider_state.db();
        let requester = instance(&world.requester_state);
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
        let world = RelayWorld::new(PAPERS).await;
        world.allow_everyone();
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
    let provider = instance(&world.provider_state);
    let db = world.requester_state.db();
    assert_eq!(world.requester.providers("summarize").await.len(), 1);

    set_blocked(db, &provider, "papers", true).unwrap();
    assert!(world.requester.providers("summarize").await.is_empty());
    let result = world.ask().await;
    assert_eq!(result.status, TaskStatus::Failed);
    // The relay knows a provider serves it, but none it may pick
    let error = result.error.unwrap_or_default();
    assert!(error.contains("Nobody who serves"), "{error}");
    // Named, it's refused before it's sent
    let named = world
        .requester
        .run(DelegationRequest {
            target: Some(format!("{provider}/papers")),
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
    assert!(world.requester.providers("summarize").await.is_empty());
    // Our own instance is never filtered
    let me = instance(&world.requester_state);
    let cards = vec![user_card(&me)];
    assert_eq!(failover::without_untrusted(db, &me, cards).len(), 1);
    world.shutdown();
}
