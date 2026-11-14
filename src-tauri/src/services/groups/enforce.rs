//! Group policies on incoming work: for a request from someone I share
//! groups with, the stricter of each group's `requests:` rules and my own
//! access level for that group, then the strictest across those groups.
//! No shared group: `None`, and everything stays as it was (the company
//! relay's trust rules and intake).
//!
//! Read from the copies in `.nandi/groups/` on every request. A copy whose
//! signature doesn't check out against its author's pinned key, or whose
//! text doesn't parse, refuses that group's work (fail closed) until a
//! good version arrives. A copy that doesn't read at all is skipped (its
//! members are unknown) and logged; the next refresh rewrites it.
//!
//! The rules that involve the other person are the relay's: it counts
//! `daily_limit` and holds `approval: first-time` at `SubmitTask`. A first
//! request it marks (`SealedTask.first_time_groups`) is kept here by task
//! (`group_first_requests`, from delivery on, so a restart keeps it) and
//! waits for me whatever my level says.

use chrono::{Datelike, Local, Timelike};
use nandi_protocol::PublicKeys;
use rusqlite::params;

use crate::db::Db;
use crate::services::domain::group::{
    verify, Decision, GroupAccess, GroupCopy, GroupPolicyText, RequestFacts,
};
use crate::services::network::peer_keys;
use crate::services::relay::protocol::ASK_ME;
use crate::services::repos::GroupRepository;

/// Why a first request the relay marked waits
const FIRST_REQUEST: &str =
    "the group's policy has you approve a member's first request (the relay marked this one)";

/// When it is (local time), as the rules read it
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    pub weekday: u8,
    pub minute: u16,
}

impl Moment {
    pub fn now() -> Self {
        let now = Local::now();
        Self {
            weekday: u8::try_from(now.weekday().num_days_from_monday()).unwrap_or(0),
            minute: u16::try_from(now.hour() * 60 + now.minute()).unwrap_or(0),
        }
    }
}

/// Everything [`decide`] reads
pub struct Enforcer<'a> {
    pub repo: &'a GroupRepository,
    pub db: &'a Db,
    /// This Nandi's instance ID
    pub me: &'a str,
    /// This device's public keys (for policies I signed)
    pub own_keys: Option<&'a PublicKeys>,
}

impl Enforcer<'_> {
    /// The groups' say on a request for `capability` from `requester`,
    /// which the relay marked as their first in `first_time` (group IDs),
    /// with its reason naming the group; `None` when I share no group with
    /// them and nothing is marked
    pub fn decide(
        &self,
        requester: &str,
        capability: &str,
        first_time: &[String],
        at: Moment,
    ) -> Option<Decision> {
        if requester == self.me {
            return None;
        }
        let (copies, broken) = match self.repo.load_all() {
            Ok(loaded) => loaded,
            Err(e) => {
                log::warn!("Groups: couldn't read the group copies: {e}");
                return None;
            }
        };
        for copy in &broken {
            log::warn!(
                "Groups: skipping {} (its copy doesn't read: {})",
                copy.group_id,
                copy.error
            );
        }
        let shared: Vec<&GroupCopy> = copies
            .iter()
            .filter(|c| c.has_member(requester) && c.has_member(self.me))
            .collect();
        // A mark for a group I have no copy of still waits for me
        let unknown_marks = first_time
            .iter()
            .filter(|id| !shared.iter().any(|c| &c.group_id == *id))
            .map(|id| Decision::Ask(format!("Group {id}: {FIRST_REQUEST}")));
        let unknown_marks: Vec<Decision> = unknown_marks.collect();
        if shared.is_empty() {
            return Decision::strictest(unknown_marks);
        }
        let access = self.repo.load_access().unwrap_or_else(|e| {
            log::warn!("Groups: my access levels don't read ({e}); asking first everywhere");
            GroupAccess::default()
        });
        let decisions = shared.iter().map(|copy| {
            let named = |decision: Decision| match decision {
                Decision::Auto => Decision::Auto,
                Decision::Ask(why) => Decision::Ask(format!("{}: {why}", label(copy))),
                Decision::Refuse(why) => Decision::Refuse(format!("{}: {why}", label(copy))),
            };
            if let Err(e) = self.verified(copy) {
                log::warn!("Groups: {}'s policy isn't verified: {e}", copy.group_id);
                return named(Decision::Refuse(
                    "this group's policy couldn't be verified".to_string(),
                ));
            }
            let rules = match GroupPolicyText::parse(&copy.group_id, &copy.text) {
                Ok(text) => text.requests,
                Err(e) => {
                    log::warn!("Groups: {}'s policy doesn't read: {e}", copy.group_id);
                    return named(Decision::Refuse(
                        "this group's policy couldn't be read".to_string(),
                    ));
                }
            };
            let facts = RequestFacts {
                capability,
                is_question: capability == ASK_ME,
                weekday: at.weekday,
                minute: at.minute,
            };
            let mine = access.level(&copy.group_id).decide(&facts);
            let first = if first_time.contains(&copy.group_id) {
                Decision::Ask(FIRST_REQUEST.to_string())
            } else {
                Decision::Auto
            };
            named(rules.decide(&facts).stricter(mine).stricter(first))
        });
        Decision::strictest(decisions.chain(unknown_marks))
    }

    /// Whether a copy's policy is signed by its author's key
    pub fn verified(&self, copy: &GroupCopy) -> Result<(), String> {
        let keys = if copy.author == self.me {
            self.own_keys.cloned()
        } else {
            peer_keys::pinned(self.db, &copy.author)?
        };
        verify(&copy.group_id, &copy.policy(), keys.as_ref())
    }
}

fn label(copy: &GroupCopy) -> String {
    if copy.name.trim().is_empty() {
        format!("Group {}", copy.group_id)
    } else {
        copy.name.clone()
    }
}

/// What the executor and the gateway hold to apply group policies
#[derive(Clone)]
pub struct GroupRules {
    repo: GroupRepository,
    db: Db,
    me: String,
    own_keys: super::PublicKeySource,
}

impl GroupRules {
    pub fn new(
        repo: GroupRepository,
        db: Db,
        me: String,
        own_keys: super::PublicKeySource,
    ) -> Self {
        Self {
            repo,
            db,
            me,
            own_keys,
        }
    }

    pub fn of(state: &crate::services::AppState) -> Result<Self, String> {
        Ok(Self::for_install(
            state.store(),
            state.db().clone(),
            state.secrets(),
            crate::services::network::local_instance_id(state.db())?,
        ))
    }

    /// The rules for this install, signed with its own device keys
    pub fn for_install(
        store: &crate::services::repo_store::RepoStore,
        db: Db,
        secrets: &crate::services::secrets::Secrets,
        me: String,
    ) -> Self {
        let keyring = crate::services::relay::device_keys::DeviceKeyring::new(secrets.clone());
        Self::new(
            store.repos().groups.clone(),
            db,
            me,
            std::sync::Arc::new(move || keyring.public_keys()),
        )
    }

    /// The groups' say on task `task_id`, a request from `requester` for
    /// `capability`, now
    pub fn decide(&self, task_id: &str, requester: &str, capability: &str) -> Option<Decision> {
        // Only read when a policy I signed needs checking
        let own = if self.repo.dir().exists() {
            (self.own_keys)().ok()
        } else {
            None
        };
        let first_time = first_requests(&self.db, task_id);
        Enforcer {
            repo: &self.repo,
            db: &self.db,
            me: &self.me,
            own_keys: own.as_ref(),
        }
        .decide(requester, capability, &first_time, Moment::now())
    }

    /// The relay delivered `task_id` as a member's first request through
    /// `groups`: kept until it's decided, over a restart
    pub fn first_request(&self, task_id: &str, groups: &[String]) -> Result<(), String> {
        let conn = self.db.conn()?;
        for group in groups {
            conn.execute(
                "INSERT OR IGNORE INTO group_first_requests (task_id, group_id) VALUES (?1, ?2)",
                params![task_id, group],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// The groups the relay marked `task_id` as a member's first request in
fn first_requests(db: &Db, task_id: &str) -> Vec<String> {
    let read = || -> Result<Vec<String>, String> {
        let conn = db.conn()?;
        let mut statement = conn
            .prepare(
                "SELECT group_id FROM group_first_requests WHERE task_id = ?1 ORDER BY group_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![task_id], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| e.to_string())
    };
    read().unwrap_or_else(|e| {
        // Unknown: the groups' own rules still apply; the relay holds the
        // requester's next one until this is decided
        log::warn!("Groups: couldn't read task {task_id}'s first-request marks: {e}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::group::{AccessLevel, CopyMember};
    use crate::services::repos::{RepoLayout, Repos};
    use nandi_protocol::{GroupPolicy, GroupRole};
    use nandi_seal::DeviceKeys;

    const G1: &str = "g_0000000000000001";
    const G2: &str = "g_0000000000000002";

    struct World {
        _tmp: tempfile::TempDir,
        repos: Repos,
        db: Db,
        alice: DeviceKeys,
        me: DeviceKeys,
    }

    fn world() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let db = Db::in_memory();
        let alice = DeviceKeys::generate().unwrap();
        peer_keys::learn(&db, "alice-1", alice.public_keys()).unwrap();
        World {
            _tmp: tmp,
            repos,
            db,
            alice,
            me: DeviceKeys::generate().unwrap(),
        }
    }

    impl World {
        fn group(&self, id: &str, name: &str, requests: &str, members: &[&str]) {
            let text = format!("---\nrequests:\n{requests}---\nWhy.\n");
            let mut copy = GroupCopy {
                group_id: id.into(),
                name: name.into(),
                version: 3,
                author: "alice-1".into(),
                signature: String::new(),
                updated_at: 0.0,
                members: members
                    .iter()
                    .map(|m| CopyMember {
                        instance_id: (*m).into(),
                        role: GroupRole::Member,
                    })
                    .collect(),
                text,
            };
            copy.signature = self
                .alice
                .sign(&GroupPolicy::signed_bytes(id, 3, &copy.text));
            self.repos.groups.save(&copy).unwrap();
        }

        fn level(&self, id: &str, level: AccessLevel) {
            let mut access = self.repos.groups.load_access().unwrap();
            access.set(id, level).unwrap();
            self.repos.groups.save_access(&access).unwrap();
        }

        fn decide(&self, from: &str, capability: &str) -> Option<Decision> {
            self.decide_marked(from, capability, &[])
        }

        /// As the relay delivered it, marked first in `first_time`
        fn decide_marked(
            &self,
            from: &str,
            capability: &str,
            first_time: &[&str],
        ) -> Option<Decision> {
            let first_time: Vec<String> = first_time.iter().map(|g| g.to_string()).collect();
            Enforcer {
                repo: &self.repos.groups,
                db: &self.db,
                me: "me-1",
                own_keys: Some(self.me.public_keys()),
            }
            .decide(from, capability, &first_time, wednesday())
        }
    }

    fn wednesday() -> Moment {
        Moment {
            weekday: 2,
            minute: 600,
        }
    }

    fn is_ask(decision: &Option<Decision>) -> bool {
        matches!(decision, Some(Decision::Ask(_)))
    }

    fn refusal(decision: &Option<Decision>) -> String {
        match decision {
            Some(Decision::Refuse(why)) => why.clone(),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn no_shared_group_changes_nothing() {
        let w = world();
        assert_eq!(w.decide("bob-1", "review"), None);
        w.group(G1, "Finance", "  approval: always\n", &["me-1", "alice-1"]);
        assert_eq!(w.decide("bob-1", "review"), None, "bob isn't in it");
        assert_eq!(w.decide("me-1", "review"), None, "my own work");
    }

    #[test]
    fn my_level_and_the_policy_the_stricter_wins() {
        let w = world();
        w.group(
            G1,
            "Finance",
            "  approval: per-member\n",
            &["me-1", "bob-1"],
        );
        // Joined: ask first
        assert!(is_ask(&w.decide("bob-1", "review")));
        w.level(G1, AccessLevel::FullyAutomated);
        assert_eq!(w.decide("bob-1", "review"), Some(Decision::Auto));
        w.level(G1, AccessLevel::QuestionsOnly);
        assert_eq!(w.decide("bob-1", "ask-me"), Some(Decision::Auto));
        assert!(refusal(&w.decide("bob-1", "review")).starts_with("Finance: "));
        w.level(G1, AccessLevel::SendMeInfo);
        assert!(is_ask(&w.decide("bob-1", "ask-me")));
        w.level(G1, AccessLevel::None);
        refusal(&w.decide("bob-1", "ask-me"));
        // A policy that always asks can't be loosened
        w.group(G1, "Finance", "  approval: always\n", &["me-1", "bob-1"]);
        w.level(G1, AccessLevel::FullyAutomated);
        assert!(is_ask(&w.decide("bob-1", "review")));
    }

    #[test]
    fn across_groups_the_strictest_applies() {
        let w = world();
        w.group(
            G1,
            "Finance",
            "  approval: per-member\n",
            &["me-1", "bob-1"],
        );
        w.group(
            G2,
            "Ops",
            "  approval: per-member\n  capabilities: { deny: [review] }\n",
            &["me-1", "bob-1"],
        );
        w.level(G1, AccessLevel::FullyAutomated);
        w.level(G2, AccessLevel::FullyAutomated);
        assert!(refusal(&w.decide("bob-1", "review")).starts_with("Ops: "));
        assert_eq!(w.decide("bob-1", "summarize"), Some(Decision::Auto));
    }

    #[test]
    fn an_unverifiable_policy_fails_closed() {
        let w = world();
        w.group(
            G1,
            "Finance",
            "  approval: per-member\n",
            &["me-1", "bob-1"],
        );
        w.level(G1, AccessLevel::FullyAutomated);
        // Edited on disk: the signature no longer covers it
        let path = w.repos.groups.path(G1);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("Why.", "Why not.")).unwrap();
        assert!(refusal(&w.decide("bob-1", "review")).contains("couldn't be verified"));
        // An author whose key isn't known
        w.group(
            G1,
            "Finance",
            "  approval: per-member\n",
            &["me-1", "bob-1"],
        );
        let mut copy = w.repos.groups.load(G1).unwrap().unwrap();
        copy.author = "stranger-1".into();
        w.repos.groups.save(&copy).unwrap();
        assert!(refusal(&w.decide("bob-1", "review")).contains("couldn't be verified"));
    }

    #[test]
    fn a_first_request_the_relay_marked_waits_whatever_my_level() {
        let w = world();
        w.group(
            G1,
            "Finance",
            "  approval: first-time\n  daily_limit: 2\n",
            &["me-1", "bob-1"],
        );
        w.group(G2, "Ops", "  approval: per-member\n", &["me-1", "bob-1"]);
        w.level(G1, AccessLevel::FullyAutomated);
        w.level(G2, AccessLevel::FullyAutomated);
        let ask = |decision: Option<Decision>| match decision {
            Some(Decision::Ask(why)) => why,
            other => panic!("expected to ask, got {other:?}"),
        };
        // (marked in, from, what it decides)
        let cases: [(&[&str], &str, Option<&str>); 5] = [
            (&[], "bob-1", None),
            (&[G1], "bob-1", Some("Finance: ")),
            (&[G2], "bob-1", Some("Ops: ")),
            // A group I have no copy of (or don't share) still waits
            (
                &["g_00000000000000ff"],
                "bob-1",
                Some("Group g_00000000000000ff: "),
            ),
            (
                &["g_00000000000000ff"],
                "carol-1",
                Some("Group g_00000000000000ff: "),
            ),
        ];
        for (marked, from, asks) in cases {
            let decision = w.decide_marked(from, "review", marked);
            match asks {
                None => assert_eq!(decision, Some(Decision::Auto), "{marked:?}"),
                Some(prefix) => {
                    let why = ask(decision);
                    assert!(why.starts_with(prefix), "{marked:?}: {why}");
                    assert!(why.contains("first request"), "{marked:?}: {why}");
                }
            }
        }
        // A refusal stays a refusal
        w.level(G1, AccessLevel::None);
        refusal(&w.decide_marked("bob-1", "review", &[G1]));
    }

    #[test]
    fn marks_are_kept_by_task() {
        let w = world();
        let rules = GroupRules::new(
            w.repos.groups.clone(),
            w.db.clone(),
            "me-1".into(),
            std::sync::Arc::new(|| Err("no keys".to_string())),
        );
        rules.first_request("t1", &[G2.into(), G1.into()]).unwrap();
        rules.first_request("t1", &[G1.into()]).unwrap();
        assert_eq!(first_requests(&w.db, "t1"), [G1, G2]);
        assert!(first_requests(&w.db, "t2").is_empty());
    }
}
