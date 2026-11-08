//! The group service over a fake relay that keeps groups the way the real
//! one does (members, admins, policy versions, invites)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use nandi_protocol::group::{GROUP_INVALID, GROUP_NOT_ADMIN, GROUP_NOT_MEMBER, GROUP_POLICY_STALE};
use nandi_protocol::{GroupMember, GroupSummary};

use super::*;
use crate::services::a2a::{Task, TaskResult};
use crate::services::domain::group::POLICY_TEMPLATE;
use crate::services::domain::AgentCard;
use crate::services::relay::client_api::{Handler, Submitted};
use crate::services::relay::protocol::{TaskAssigned, TaskCompleted, TaskFailed};
use crate::services::relay::RelayStatus;
use crate::services::repos::{RepoLayout, Repos};

#[derive(Default)]
struct Relay {
    groups: HashMap<String, GroupDetail>,
    /// Every op, as asked
    ops: Vec<GroupOp>,
    /// Refuse the next Create as taken
    taken_once: bool,
    /// Another admin's version lands before the next SetPolicy
    stale_once: bool,
    keys: HashMap<String, PublicKeys>,
}

struct FakeRelay {
    me: String,
    connected: bool,
    relay: Arc<Mutex<Relay>>,
}

fn refused(code: &str) -> GroupOutcome {
    GroupOutcome::Refused {
        code: code.into(),
        message: format!("refused: {code}"),
    }
}

impl FakeRelay {
    fn answer(&self, op: GroupOp) -> GroupOutcome {
        let mut relay = self.relay.lock().unwrap();
        relay.ops.push(op.clone());
        let me = self.me.clone();
        let role_of = |group: &GroupDetail| {
            group
                .members
                .iter()
                .find(|m| m.instance_id == me)
                .map(|m| m.role)
        };
        match op {
            GroupOp::List => GroupOutcome::List {
                groups: relay
                    .groups
                    .values()
                    .map(|g| GroupSummary {
                        role: role_of(g),
                        ..g.summary.clone()
                    })
                    .collect(),
                invites: Vec::new(),
            },
            GroupOp::Get { group_id } => match relay.groups.get(&group_id) {
                Some(g) if role_of(g).is_some() => GroupOutcome::Group { group: g.clone() },
                Some(_) => refused(GROUP_NOT_MEMBER),
                None => refused(GROUP_NOT_FOUND),
            },
            GroupOp::Create {
                group_id,
                name,
                description,
                policy,
            } => {
                if std::mem::take(&mut relay.taken_once) {
                    return refused(GROUP_INVALID);
                }
                assert!(is_group_id(&group_id), "{group_id}");
                assert_eq!(policy.version, 1);
                let group = GroupDetail {
                    summary: GroupSummary {
                        id: group_id.clone(),
                        name,
                        description,
                        member_count: 1,
                        created_at: 1.0,
                        policy_version: 1,
                        role: Some(GroupRole::Admin),
                    },
                    members: vec![GroupMember {
                        instance_id: me,
                        display_name: "Me".into(),
                        role: GroupRole::Admin,
                        joined_at: 1.0,
                    }],
                    policy: GroupPolicy {
                        updated_at: 2.0,
                        ..policy
                    },
                    invites: Vec::new(),
                };
                relay.groups.insert(group_id, group.clone());
                GroupOutcome::Group { group }
            }
            GroupOp::SetPolicy { group_id, policy } => {
                let stale = std::mem::take(&mut relay.stale_once);
                let Some(group) = relay.groups.get_mut(&group_id) else {
                    return refused(GROUP_NOT_FOUND);
                };
                if role_of(group) != Some(GroupRole::Admin) {
                    return refused(GROUP_NOT_ADMIN);
                }
                if stale {
                    group.summary.policy_version += 1;
                    group.policy.version += 1;
                    return refused(GROUP_POLICY_STALE);
                }
                if policy.version != group.summary.policy_version + 1 {
                    return refused(GROUP_POLICY_STALE);
                }
                group.summary.policy_version = policy.version;
                group.policy = policy;
                GroupOutcome::Done
            }
            GroupOp::Join { code } => {
                let Some(group) = relay.groups.get_mut(&code) else {
                    return refused(nandi_protocol::group::GROUP_INVITE_INVALID);
                };
                group.members.push(GroupMember {
                    instance_id: me,
                    display_name: "Me".into(),
                    role: GroupRole::Member,
                    joined_at: 3.0,
                });
                GroupOutcome::Group {
                    group: group.clone(),
                }
            }
            GroupOp::Leave { group_id } => {
                if let Some(group) = relay.groups.get_mut(&group_id) {
                    group.members.retain(|m| m.instance_id != me);
                }
                GroupOutcome::Done
            }
            other => panic!("not faked: {other:?}"),
        }
    }
}

impl RelayClient for FakeRelay {
    fn connect<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn register(&self, _: Vec<AgentCard>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn heartbeat(&self, _: HashMap<String, u32>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn query_capability<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn delegate_task<'a>(
        &'a self,
        _: &'a Task,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<Submitted, String>> {
        Box::pin(async { Err("unused".to_string()) })
    }
    fn send_result(&self, _: TaskResult) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn on_task_assigned(&self, _: Handler<TaskAssigned>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn on_task_completed(&self, _: Handler<TaskCompleted>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn on_task_failed(&self, _: Handler<TaskFailed>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn status(&self) -> RelayStatus {
        RelayStatus {
            connected: self.connected,
            visibility: Some(RelayVisibility::Groups),
            ..RelayStatus::default()
        }
    }
    fn group_request(&self, op: GroupOp) -> BoxFuture<'_, Result<GroupOutcome, String>> {
        let outcome = self.answer(op);
        Box::pin(async move { Ok(outcome) })
    }
    fn public_keys_of<'a>(
        &'a self,
        instance: &'a str,
    ) -> BoxFuture<'a, Result<PublicKeys, String>> {
        let keys = self.relay.lock().unwrap().keys.get(instance).cloned();
        Box::pin(async move { keys.ok_or_else(|| "no keys".to_string()) })
    }
}

struct World {
    _tmp: tempfile::TempDir,
    repos: Repos,
    db: Db,
    relay: Arc<Mutex<Relay>>,
}

impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        Self {
            _tmp: tmp,
            repos,
            db: Db::in_memory(),
            relay: Arc::default(),
        }
    }

    /// `me`'s Nandi, with `keys`, on this relay (or none connected)
    fn nandi(&self, me: &str, keys: &DeviceKeys, connected: bool) -> Groups {
        let (k, p) = (keys.export_secret().to_string(), keys.public_keys().clone());
        self.relay
            .lock()
            .unwrap()
            .keys
            .insert(me.to_string(), p.clone());
        let relay: Arc<dyn RelayClient> = Arc::new(FakeRelay {
            me: me.to_string(),
            connected,
            relay: self.relay.clone(),
        });
        Groups::new(
            Some(relay),
            self.repos.groups.clone(),
            self.db.clone(),
            me.to_string(),
            Arc::new(move || DeviceKeys::import_secret(&k).map_err(|e| e.to_string())),
            Arc::new(move || Ok(p.clone())),
            None,
        )
    }

    fn ops(&self) -> Vec<GroupOp> {
        self.relay.lock().unwrap().ops.clone()
    }
}

#[tokio::test]
async fn without_a_relay_the_lists_are_empty_and_calls_say_so() {
    let w = World::new();
    let keys = DeviceKeys::generate().unwrap();
    let groups = w.nandi("me-1", &keys, false);
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
    assert!(w.ops().is_empty());
}

#[tokio::test]
async fn creating_signs_version_one_over_the_id_it_picks() {
    let w = World::new();
    let keys = DeviceKeys::generate().unwrap();
    let groups = w.nandi("me-1", &keys, true);
    w.relay.lock().unwrap().taken_once = true;
    let view = groups
        .create(" Finance ", "Month-end", POLICY_TEMPLATE)
        .await
        .unwrap();
    let id = view.group.summary.id.clone();
    assert!(is_group_id(&id));
    assert_eq!(view.group.summary.name, "Finance");
    assert!(view.policy.verified, "{:?}", view.policy.problem);
    assert_eq!(view.policy.version, 1);
    assert_eq!(view.policy.approval, Some(Approval::Always));
    assert_eq!(view.my_role, Some(GroupRole::Admin));
    assert_eq!(view.my_access, AccessLevel::AskFirst);
    // A taken ID was tried again with another
    let creates: Vec<String> = w
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            GroupOp::Create { group_id, .. } => Some(group_id),
            _ => None,
        })
        .collect();
    assert_eq!(creates.len(), 2);
    assert_ne!(creates[0], creates[1]);
    // The gold copy and my level are files
    let copy = w.repos.groups.load(&id).unwrap().unwrap();
    assert_eq!(copy.text, POLICY_TEMPLATE);
    assert_eq!(copy.author, "me-1");
    assert!(copy.has_member("me-1"));
    assert!(w.repos.groups.load_access().unwrap().has(&id));
    // A bad policy never reaches the relay
    let before = w.ops().len();
    let error = groups
        .create("X", "", "---\nrequests:\n  approval: maybe\n---\n")
        .await
        .unwrap_err();
    assert!(error.contains("requests"), "{error}");
    assert!(groups.create("  ", "", POLICY_TEMPLATE).await.is_err());
    assert_eq!(w.ops().len(), before);
}

#[tokio::test]
async fn a_policy_change_is_the_next_version_signed_by_me() {
    let w = World::new();
    let keys = DeviceKeys::generate().unwrap();
    let groups = w.nandi("me-1", &keys, true);
    let id = groups
        .create("Finance", "", POLICY_TEMPLATE)
        .await
        .unwrap()
        .group
        .summary
        .id;
    let text = "---\nrequests:\n  approval: per-member\n---\nLooser.\n";
    let view = groups.set_policy(&id, text).await.unwrap();
    assert_eq!(view.policy.version, 2);
    assert!(view.policy.verified);
    assert_eq!(view.policy.approval, Some(Approval::PerMember));
    assert_eq!(w.repos.groups.load(&id).unwrap().unwrap().text, text);
    // Another admin got there first: signed again over the version after theirs
    w.relay.lock().unwrap().stale_once = true;
    let view = groups.set_policy(&id, POLICY_TEMPLATE).await.unwrap();
    assert_eq!(view.policy.version, 4);
    assert!(view.policy.verified);
    assert!(groups
        .set_policy(&id, "---\nrequests: 3\n---\n")
        .await
        .is_err());
}

#[tokio::test]
async fn joining_keeps_a_verified_copy_and_leaving_drops_it() {
    let w = World::new();
    let alice_keys = DeviceKeys::generate().unwrap();
    let alice = w.nandi("alice-1", &alice_keys, true);
    let id = alice
        .create("Finance", "", POLICY_TEMPLATE)
        .await
        .unwrap()
        .group
        .summary
        .id;
    // Bob's Nandi shares the repo dir in this test: use another world for Bob
    let bob_world = World {
        relay: w.relay.clone(),
        ..World::new()
    };
    let bob_keys = DeviceKeys::generate().unwrap();
    let bob = bob_world.nandi("bob-1", &bob_keys, true);
    let view = bob.join(&id).await.unwrap();
    assert!(view.policy.verified, "{:?}", view.policy.problem);
    assert_eq!(view.my_role, Some(GroupRole::Member));
    assert_eq!(view.my_access, AccessLevel::AskFirst);
    // Alice's key was learned (pinned) on the way
    assert!(peer_keys::pinned(&bob_world.db, "alice-1")
        .unwrap()
        .is_some());
    assert!(bob_world.repos.groups.load(&id).unwrap().is_some());
    bob.leave(&id).await.unwrap();
    assert!(bob_world.repos.groups.load(&id).unwrap().is_none());
    assert!(!bob_world.repos.groups.load_access().unwrap().has(&id));
}

#[tokio::test]
async fn a_forged_policy_shows_unverified() {
    let w = World::new();
    let keys = DeviceKeys::generate().unwrap();
    let groups = w.nandi("me-1", &keys, true);
    let id = groups
        .create("Finance", "", POLICY_TEMPLATE)
        .await
        .unwrap()
        .group
        .summary
        .id;
    // The relay serves text the signature doesn't cover
    w.relay
        .lock()
        .unwrap()
        .groups
        .get_mut(&id)
        .unwrap()
        .policy
        .text = "---\nrequests:\n  approval: per-member\n---\n".into();
    let view = groups.get(&id).await.unwrap();
    assert!(!view.policy.verified);
    assert!(view.policy.problem.unwrap().contains("signature"));
}

#[tokio::test]
async fn refreshing_drops_copies_of_groups_im_not_in() {
    let w = World::new();
    let keys = DeviceKeys::generate().unwrap();
    let groups = w.nandi("me-1", &keys, true);
    let id = groups
        .create("Finance", "", POLICY_TEMPLATE)
        .await
        .unwrap()
        .group
        .summary
        .id;
    // A copy left from a group I'm not in any more
    let mut old = w.repos.groups.load(&id).unwrap().unwrap();
    old.group_id = "g_00000000000000ff".into();
    w.repos.groups.save(&old).unwrap();
    // Removed from Finance at the relay
    w.relay
        .lock()
        .unwrap()
        .groups
        .get_mut(&id)
        .unwrap()
        .members
        .clear();
    groups.on_signal(GroupSignal::Registered).await;
    let (copies, broken) = w.repos.groups.load_all().unwrap();
    assert!(copies.is_empty() && broken.is_empty(), "{copies:?}");
}

#[test]
fn group_ids_are_g_and_sixteen_hex() {
    for _ in 0..20 {
        assert!(is_group_id(&new_group_id()));
    }
    assert_ne!(new_group_id(), new_group_id());
}
