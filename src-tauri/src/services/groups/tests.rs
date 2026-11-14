//! The group service on the real relay: `nandi_relay::testing::
//! serve_with_faults` as a groups relay, and one app relay client per
//! person with its own keys and files, as the delegation tests do
//! (`a2a::relay_world`)

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nandi_protocol::{
    GroupHold, GroupPolicy, OwnerRef, ServesCapability, Visibility, GROUP_CAPABILITY_NOT_ALLOWED,
    HOLD_ALWAYS_ASK, HOLD_APPROVAL_ALWAYS, NEEDS_FIRST_TIME_APPROVAL,
};
use nandi_relay::RelayServerConfig;
use serde_json::json;

use super::*;
use crate::services::a2a::relay_world::{wait_until, TestRelay};
use crate::services::a2a::Task;
use crate::services::domain::group::{AccessLevel, Decision, POLICY_TEMPLATE};
use crate::services::domain::AgentCard;
use crate::services::relay::client::{ClientOptions, WsRelayClient};
use crate::services::relay::client_api::{stored_at_once, REFUSED_BY_RELAY};
use crate::services::relay::protocol::TaskAssigned;
use crate::services::repos::{RepoLayout, Repos};

/// A group request, as the relay's frames name it
const REQUEST: &str = "group_request";
/// The relay's answer to one
const REPLY: &str = "group_reply";
/// Longer than a few calls on a local relay take
const LATE: Duration = Duration::from_millis(400);

const LOOSER: &str = "---\nrequests:\n  approval: per-member\n---\nLooser.\n";

/// A groups relay of one test's own
struct World {
    relay: TestRelay,
}

/// One person's Nandi on the relay: its files, its database, its keys and
/// its own relay client
struct Member {
    id: String,
    _tmp: tempfile::TempDir,
    repos: Repos,
    db: Db,
    client: Arc<WsRelayClient>,
    groups: Groups,
}

impl World {
    async fn start() -> Self {
        let config = RelayServerConfig::open().with_visibility(Visibility::Groups);
        Self {
            relay: TestRelay::start_with(config).await,
        }
    }

    /// `me`'s Nandi, connected and registered with its keys
    async fn member(&self, me: &str) -> Member {
        Member::on(&self.relay, me, Some(Vec::new())).await
    }

    /// `me`'s Nandi with an agent `papers` serving `find-papers`
    async fn provider(&self, me: &str) -> Member {
        Member::on(&self.relay, me, Some(vec![papers(me)])).await
    }

    /// `me`'s Nandi with a relay client that never connected
    async fn offline_member(&self, me: &str) -> Member {
        Member::on(&self.relay, me, None).await
    }

    /// Alice's group "Finance", and Bob joined through her invite
    async fn pair(&self) -> (Member, Member, String) {
        let alice = self.member("alice-1").await;
        let bob = self.member("bob-1").await;
        let id = alice.create_finance().await;
        bob.join_through(&alice, &id).await;
        (alice, bob, id)
    }

    /// How many group requests `member` has sent the relay
    fn requests(&self, member: &Member) -> usize {
        self.relay.frames_sent(&member.id, REQUEST)
    }

    /// The group IDs `member` asked the relay to create, in order
    fn creates(&self, member: &Member) -> Vec<String> {
        self.relay.groups_created(&member.id)
    }
}

impl Member {
    /// Registered with `cards`; `None`: never connected
    async fn on(relay: &TestRelay, me: &str, cards: Option<Vec<AgentCard>>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let db = Db::in_memory();
        let keys = DeviceKeys::generate().unwrap();
        let (secret, public) = (keys.export_secret().to_string(), keys.public_keys().clone());
        let own_keys: KeySource =
            Arc::new(move || DeviceKeys::import_secret(&secret).map_err(|e| e.to_string()));
        let options = ClientOptions {
            device_keys: Some(own_keys.clone()),
            peer_keys: Some(Arc::new(peer_keys::PinnedKeys::new(db.clone()))),
            ..ClientOptions::default()
        };
        let client = match cards {
            Some(cards) => {
                let client = relay.client_with(me, options).await;
                client.register(cards).await.unwrap();
                client
            }
            None => TestRelay::unconnected_client(me, options),
        };
        let groups = Groups::new(
            Some(client.clone() as Arc<dyn RelayClient>),
            repos.groups.clone(),
            db.clone(),
            me.to_string(),
            own_keys,
            Arc::new(move || Ok(public.clone())),
            None,
        );
        Self {
            id: me.to_string(),
            _tmp: tmp,
            repos,
            db,
            client,
            groups,
        }
    }

    /// A new group "Finance" with the template policy; its ID
    async fn create_finance(&self) -> String {
        let view = self
            .groups
            .create("Finance", "", POLICY_TEMPLATE)
            .await
            .unwrap();
        view.group.summary.id
    }

    /// Joins `group` with an invite from `host`, who is in it
    async fn join_through(&self, host: &Member, group: &str) -> GroupView {
        let request = InviteRequest {
            instance_id: Some(self.id.clone()),
            max_uses: 1,
            expires_at: None,
        };
        let invite = host.groups.invite(group, request).await.unwrap();
        self.groups.join(&invite.code).await.unwrap()
    }

    /// The group as the relay holds it
    async fn detail_at_relay(&self, group: &str) -> GroupDetail {
        let get = GroupOp::Get {
            group_id: group.to_string(),
        };
        match self.client.group_request(get).await.unwrap() {
            GroupOutcome::Group { group } => group,
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn without_a_relay_the_lists_are_empty_and_calls_say_so() {
    let world = World::start().await;
    let groups = world.offline_member("me-1").await.groups;
    let overview = groups.overview().await.unwrap();
    assert!(!overview.connected && overview.groups.is_empty());
    let error = groups
        .create("Finance", "", POLICY_TEMPLATE)
        .await
        .unwrap_err();
    assert!(RelayUnavailable::is(&error), "{error}");
    // My own level is mine: it works offline
    let g = "g_0000000000000001";
    groups.set_access(g, AccessLevel::QuestionsOnly).unwrap();
    assert_eq!(groups.access(g).unwrap(), AccessLevel::QuestionsOnly);
    assert!(world.relay.faults.frames().is_empty());
}

#[tokio::test]
async fn creating_signs_version_one_over_the_id_it_picks() {
    let world = World::start().await;
    let me = world.member("me-1").await;
    world.relay.faults.take_next_group_id(&me.id);
    let view = me
        .groups
        .create(" Finance ", "Month-end", POLICY_TEMPLATE)
        .await
        .unwrap();
    let id = view.group.summary.id.clone();
    assert!(is_group_id(&id));
    assert_eq!(view.group.summary.name, "Finance");
    assert!(view.policy.verified, "{:?}", view.policy.problem);
    assert_eq!(view.policy.version, 1);
    assert_eq!(view.policy.approval, Some(GroupApproval::Always));
    assert_eq!(view.my_role, Some(GroupRole::Admin));
    assert_eq!(view.my_access, AccessLevel::AskFirst);
    // The relay refused the first ID as taken: another was tried
    let creates = world.creates(&me);
    assert_eq!(creates.len(), 2);
    assert_ne!(creates[0], creates[1]);
    // The gold copy and my level are files
    let copy = me.repos.groups.load(&id).unwrap().unwrap();
    assert_eq!(copy.text, POLICY_TEMPLATE);
    assert_eq!(copy.author, "me-1");
    assert!(copy.has_member("me-1"));
    assert!(me.repos.groups.load_access().unwrap().has(&id));
}

#[tokio::test]
async fn a_bad_policy_or_name_never_reaches_the_relay() {
    let world = World::start().await;
    let me = world.member("me-1").await;
    let before = world.requests(&me);
    let error = me
        .groups
        .create("X", "", "---\nrequests:\n  approval: maybe\n---\n")
        .await
        .unwrap_err();
    assert!(error.contains("requests"), "{error}");
    assert!(me.groups.create("  ", "", POLICY_TEMPLATE).await.is_err());
    assert_eq!(world.requests(&me), before);
}

#[tokio::test]
async fn a_policy_change_is_the_next_version_signed_by_me() {
    let world = World::start().await;
    let me = world.member("me-1").await;
    let id = me.create_finance().await;
    let view = me.groups.set_policy(&id, LOOSER).await.unwrap();
    assert_eq!(view.policy.version, 2);
    assert!(view.policy.verified);
    assert_eq!(view.policy.approval, Some(GroupApproval::PerMember));
    assert_eq!(me.repos.groups.load(&id).unwrap().unwrap().text, LOOSER);
    let refused = me.groups.set_policy(&id, "---\nrequests: 3\n---\n").await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn a_policy_changed_meanwhile_is_signed_again_over_the_version_after_it() {
    let world = World::start().await;
    let (alice, bob, id) = world.pair().await;
    alice
        .groups
        .set_role(&id, "bob-1", GroupRole::Admin)
        .await
        .unwrap();
    let view = alice.groups.set_policy(&id, LOOSER).await.unwrap();
    assert_eq!(view.policy.version, 2);
    // The relay answers Alice's read late: Bob's version 3 lands in between
    world.relay.faults.delay_next("alice-1", REPLY, LATE);
    let reads = world.requests(&alice);
    let alice_sets = alice.groups.set_policy(&id, POLICY_TEMPLATE);
    let bob_sets = async {
        wait_until("Alice's read at the relay", || async {
            world.requests(&alice) > reads
        })
        .await;
        bob.groups.set_policy(&id, LOOSER).await.unwrap()
    };
    let (alice_view, bob_view) = tokio::join!(alice_sets, bob_sets);
    assert_eq!(bob_view.policy.version, 3);
    let view = alice_view.unwrap();
    assert_eq!(view.policy.version, 4);
    assert!(view.policy.verified);
    assert_eq!(
        alice.repos.groups.load(&id).unwrap().unwrap().text,
        POLICY_TEMPLATE
    );
}

#[tokio::test]
async fn joining_keeps_a_verified_copy_and_leaving_drops_it() {
    let world = World::start().await;
    let alice = world.member("alice-1").await;
    let bob = world.member("bob-1").await;
    let id = alice.create_finance().await;
    let view = bob.join_through(&alice, &id).await;
    assert!(view.policy.verified, "{:?}", view.policy.problem);
    assert_eq!(view.my_role, Some(GroupRole::Member));
    assert_eq!(view.my_access, AccessLevel::AskFirst);
    // Alice's key was learned (pinned) on the way
    assert!(peer_keys::pinned(&bob.db, "alice-1").unwrap().is_some());
    assert!(bob.repos.groups.load(&id).unwrap().is_some());
    bob.groups.leave(&id).await.unwrap();
    assert!(bob.repos.groups.load(&id).unwrap().is_none());
    assert!(!bob.repos.groups.load_access().unwrap().has(&id));
}

#[tokio::test]
async fn a_forged_policy_shows_unverified() {
    let world = World::start().await;
    let alice = world.member("alice-1").await;
    let id = alice.create_finance().await;
    // The relay is handed text that Alice's signature doesn't cover
    let forged = GroupPolicy {
        version: 2,
        text: "---\nrequests:\n  approval: per-member\n---\n".into(),
        ..alice.detail_at_relay(&id).await.policy
    };
    let set = GroupOp::SetPolicy {
        group_id: id.clone(),
        policy: forged,
    };
    let outcome = alice.client.group_request(set).await.unwrap();
    assert_eq!(outcome, GroupOutcome::Done);
    let view = alice.groups.get(&id).await.unwrap();
    assert!(!view.policy.verified);
    assert!(view.policy.problem.unwrap().contains("signature"));
}

#[tokio::test]
async fn refreshing_drops_copies_of_groups_im_not_in() {
    let world = World::start().await;
    let (alice, bob, id) = world.pair().await;
    let admin = GroupRole::Admin;
    alice.groups.set_role(&id, "bob-1", admin).await.unwrap();
    // A copy left from a group I'm not in any more
    let mut old = alice.repos.groups.load(&id).unwrap().unwrap();
    old.group_id = "g_00000000000000ff".into();
    alice.repos.groups.save(&old).unwrap();
    // Removed from Finance at the relay, by Bob
    bob.groups.remove_member(&id, "alice-1").await.unwrap();
    alice.groups.on_signal(GroupSignal::Registered).await;
    let (copies, broken) = alice.repos.groups.load_all().unwrap();
    assert!(copies.is_empty() && broken.is_empty(), "{copies:?}");
}

#[test]
fn group_ids_are_g_and_sixteen_hex() {
    for _ in 0..20 {
        assert!(is_group_id(&new_group_id()));
    }
    assert_ne!(new_group_id(), new_group_id());
}

/// `instance`'s agent `papers`, serving `find-papers`
fn papers(instance: &str) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(instance, "papers"),
        name: "papers".into(),
        description: String::new(),
        instance_id: instance.into(),
        endpoint: String::new(),
        capabilities: vec![ServesCapability {
            name: "find-papers".into(),
            description: String::new(),
            input_schema: None,
            output_schema: None,
        }],
        accepts_delegation: true,
        max_concurrent: 1,
        current_load: 0,
        owner: OwnerRef::default(),
        signature: None,
        issued_at: 0.0,
    }
}

/// Bob's request `id` for Alice's `papers`
fn ask_alice(id: &str) -> Task {
    Task {
        id: id.into(),
        requester: papers("bob-1"),
        target: "alice-1/papers".into(),
        capability: "find-papers".into(),
        input: json!({"q": "relays"}),
        created_at: 1.0,
        deadline: None,
    }
}

#[tokio::test]
async fn the_relay_marks_a_first_request_and_refuses_the_next_while_it_waits() {
    let world = World::start().await;
    let alice = world.provider("alice-1").await;
    let bob = world.member("bob-1").await;
    let first_time =
        "---\nrequests:\n  approval: first-time\n  daily_limit: 5\n---\nNew people wait.\n";
    let view = alice
        .groups
        .create("Finance", "", first_time)
        .await
        .unwrap();
    let group = view.group.summary.id;
    bob.join_through(&alice, &group).await;
    let delivered: Arc<Mutex<Vec<TaskAssigned>>> = Arc::default();
    let sink = delivered.clone();
    alice
        .client
        .on_task_assigned(stored_at_once(move |assigned| {
            sink.lock().unwrap().push(assigned)
        }))
        .await;

    bob.client
        .delegate_task(&ask_alice("t1"), &[])
        .await
        .unwrap();
    wait_until("Alice has Bob's first request", || async {
        !delivered.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(delivered.lock().unwrap()[0].first_time_groups, [group]);

    // The next waits for her answer to the first, told by its code
    let refused = bob
        .client
        .delegate_task(&ask_alice("t2"), &[])
        .await
        .unwrap_err();
    let coded = format!("{REFUSED_BY_RELAY}{NEEDS_FIRST_TIME_APPROVAL}: ");
    assert!(refused.starts_with(&coded), "{refused}");
    assert!(refused.contains("Finance"), "{refused}");
}

#[tokio::test]
async fn the_relay_holds_or_refuses_by_the_groups_rules_and_my_nandi_honours_it() {
    enum Outcome {
        /// Delivered with this hold, and it waits for Alice's approval
        Held(&'static str),
        /// Refused at the relay, told by this code
        Refused(&'static str),
    }
    use Outcome::{Held, Refused};
    let cases = [
        ("  approval: always\n", Held(HOLD_APPROVAL_ALWAYS)),
        (
            "  approval: per-member\n  always_ask: [papers]\n",
            Held(HOLD_ALWAYS_ASK),
        ),
        (
            "  approval: per-member\n  capabilities: { deny: [\"find-*\"] }\n",
            Refused(GROUP_CAPABILITY_NOT_ALLOWED),
        ),
    ];
    for (requests, outcome) in cases {
        let world = World::start().await;
        let alice = world.provider("alice-1").await;
        let bob = world.member("bob-1").await;
        let policy = format!("---\nrequests:\n{requests}---\nFinance.\n");
        let view = alice.groups.create("Finance", "", &policy).await.unwrap();
        let group = view.group.summary.id;
        bob.join_through(&alice, &group).await;
        // Alice's copy names Bob, and she lets the group's work run, so
        // only a hold makes it wait
        alice.groups.refresh_group(&group).await.unwrap();
        alice
            .groups
            .set_access(&group, AccessLevel::FullyAutomated)
            .unwrap();
        let delivered: Arc<Mutex<Vec<TaskAssigned>>> = Arc::default();
        let sink = delivered.clone();
        alice
            .client
            .on_task_assigned(stored_at_once(move |assigned| {
                sink.lock().unwrap().push(assigned)
            }))
            .await;

        let sent = bob.client.delegate_task(&ask_alice("t1"), &[]).await;
        match outcome {
            Refused(code) => {
                let refused = sent.unwrap_err();
                let coded = format!("{REFUSED_BY_RELAY}{code}: ");
                assert!(refused.starts_with(&coded), "{requests}: {refused}");
                assert!(refused.contains("Finance"), "{refused}");
            }
            Held(code) => {
                sent.unwrap();
                wait_until("Alice has Bob's request", || async {
                    !delivered.lock().unwrap().is_empty()
                })
                .await;
                let assigned = delivered.lock().unwrap()[0].clone();
                let hold = GroupHold {
                    group_id: group.clone(),
                    code: code.to_string(),
                };
                assert_eq!(assigned.group_holds, [hold], "{requests}");
                let rules = enforce::GroupRules::new(
                    alice.repos.groups.clone(),
                    alice.db.clone(),
                    alice.id.clone(),
                );
                let task = &assigned.task;
                rules
                    .keep_holds(&task.id, &assigned.first_time_groups, &assigned.group_holds)
                    .unwrap();
                let decision = rules.decide(&task.id, "bob-1", &task.capability);
                assert!(
                    matches!(&decision, Some(Decision::Ask(why)) if why.starts_with("Finance: ")),
                    "{requests}: {decision:?}"
                );
            }
        }
    }
}
