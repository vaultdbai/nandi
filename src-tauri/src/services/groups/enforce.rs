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
//! Requests that went ahead are counted per group, member and day
//! (`group_requests`), for `daily_limit` and `approval: first-time`.

use chrono::{Datelike, Local, Timelike};
use nandi_protocol::PublicKeys;
use rusqlite::{params, OptionalExtension};

use crate::db::Db;
use crate::services::domain::group::{
    verify, Decision, GroupAccess, GroupCopy, GroupPolicyText, RequestFacts,
};
use crate::services::network::peer_keys;
use crate::services::relay::protocol::ASK_ME;
use crate::services::repos::GroupRepository;

/// What the groups I share with a requester say about one request
#[derive(Debug, Clone, PartialEq)]
pub struct GroupGate {
    /// The strictest decision, its reason naming the group
    pub decision: Decision,
    /// The groups it came through (counted when it goes ahead)
    pub groups: Vec<String>,
}

/// When it is (local time), as the rules read it
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    pub day: chrono::NaiveDate,
    pub weekday: u8,
    pub minute: u16,
}

impl Moment {
    pub fn now() -> Self {
        let now = Local::now();
        Self {
            day: now.date_naive(),
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
    /// The groups' say on a request for `capability` from `requester`;
    /// `None` when I share no group with them
    pub fn decide(&self, requester: &str, capability: &str, at: Moment) -> Option<GroupGate> {
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
        if shared.is_empty() {
            return None;
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
            let (today, ever) = counts(self.db, &copy.group_id, requester, at.day);
            let facts = RequestFacts {
                capability,
                is_question: capability == ASK_ME,
                first_time: ever == 0,
                today,
                weekday: at.weekday,
                minute: at.minute,
            };
            let mine = access.level(&copy.group_id).decide(&facts);
            named(rules.decide(&facts).stricter(mine))
        });
        let decision = Decision::strictest(decisions)?;
        Some(GroupGate {
            decision,
            groups: shared.iter().map(|c| c.group_id.clone()).collect(),
        })
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
        let keyring =
            crate::services::relay::device_keys::DeviceKeyring::new(state.secrets().clone());
        Ok(Self::new(
            state.store().repos().groups.clone(),
            state.db().clone(),
            crate::services::network::local_instance_id(state.db())?,
            std::sync::Arc::new(move || keyring.public_keys()),
        ))
    }

    /// The groups' say on a request from `requester` for `capability`, now
    pub fn decide(&self, requester: &str, capability: &str) -> Option<GroupGate> {
        // Only read when a policy I signed needs checking
        let own = if self.repo.dir().exists() {
            (self.own_keys)().ok()
        } else {
            None
        };
        Enforcer {
            repo: &self.repo,
            db: &self.db,
            me: &self.me,
            own_keys: own.as_ref(),
        }
        .decide(requester, capability, Moment::now())
    }

    /// The request `gate` decided went ahead
    pub fn went_ahead(&self, gate: &GroupGate, requester: &str) {
        went_ahead(&self.db, &gate.groups, requester, Moment::now().day);
    }
}

/// `(today, ever)`: requests from `instance` through `group` that went ahead
fn counts(db: &Db, group: &str, instance: &str, day: chrono::NaiveDate) -> (u32, u32) {
    let day = day.to_string();
    let read = || -> Result<(u32, u32), String> {
        let conn = db.conn()?;
        let today: Option<u32> = conn
            .query_row(
                "SELECT count FROM group_requests WHERE group_id = ?1 AND instance_id = ?2 AND day = ?3",
                params![group, instance, day],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let ever: u32 = conn
            .query_row(
                "SELECT COALESCE(SUM(count), 0) FROM group_requests WHERE group_id = ?1 AND instance_id = ?2",
                params![group, instance],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok((today.unwrap_or(0), ever))
    };
    read().unwrap_or_else(|e| {
        // Unknown counts: treated as the limit reached is too harsh;
        // first time (asks) is the safe side
        log::warn!("Groups: couldn't read request counts: {e}");
        (0, 0)
    })
}

/// A request from `instance` through `groups` went ahead: counted
pub fn went_ahead(db: &Db, groups: &[String], instance: &str, day: chrono::NaiveDate) {
    let day = day.to_string();
    let write = || -> Result<(), String> {
        let conn = db.conn()?;
        for group in groups {
            conn.execute(
                "INSERT INTO group_requests (group_id, instance_id, day, count) VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(group_id, instance_id, day) DO UPDATE SET count = count + 1",
                params![group, instance, day],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    if let Err(e) = write() {
        log::warn!("Groups: couldn't count a request: {e}");
    }
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

        fn decide(&self, from: &str, capability: &str) -> Option<GroupGate> {
            Enforcer {
                repo: &self.repos.groups,
                db: &self.db,
                me: "me-1",
                own_keys: Some(self.me.public_keys()),
            }
            .decide(from, capability, wednesday())
        }
    }

    fn wednesday() -> Moment {
        Moment {
            day: chrono::NaiveDate::from_ymd_opt(2026, 11, 11).unwrap(),
            weekday: 2,
            minute: 600,
        }
    }

    fn is_ask(gate: &Option<GroupGate>) -> bool {
        matches!(
            gate,
            Some(GroupGate {
                decision: Decision::Ask(_),
                ..
            })
        )
    }

    fn refusal(gate: &Option<GroupGate>) -> String {
        match gate {
            Some(GroupGate {
                decision: Decision::Refuse(why),
                ..
            }) => why.clone(),
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
        let gate = w.decide("bob-1", "review").unwrap();
        assert_eq!(gate.decision, Decision::Auto);
        assert_eq!(gate.groups, [G1]);
        w.level(G1, AccessLevel::QuestionsOnly);
        assert_eq!(
            w.decide("bob-1", "ask-me").unwrap().decision,
            Decision::Auto
        );
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
        let gate = w.decide("bob-1", "summarize").unwrap();
        assert_eq!(gate.decision, Decision::Auto);
        assert_eq!(gate.groups, [G1, G2]);
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
    fn daily_limits_and_first_time_count_what_went_ahead() {
        let w = world();
        w.group(
            G1,
            "Finance",
            "  approval: first-time\n  daily_limit: 2\n",
            &["me-1", "bob-1"],
        );
        w.level(G1, AccessLevel::FullyAutomated);
        assert!(is_ask(&w.decide("bob-1", "review")), "first time");
        let day = wednesday().day;
        went_ahead(&w.db, &[G1.to_string()], "bob-1", day);
        assert_eq!(
            w.decide("bob-1", "review").unwrap().decision,
            Decision::Auto
        );
        went_ahead(&w.db, &[G1.to_string()], "bob-1", day);
        assert!(refusal(&w.decide("bob-1", "review")).contains("limit of 2"));
        // Another day
        went_ahead(&w.db, &[G1.to_string()], "bob-1", day.pred_opt().unwrap());
        let tomorrow = Moment {
            day: day.succ_opt().unwrap(),
            ..wednesday()
        };
        let gate = Enforcer {
            repo: &w.repos.groups,
            db: &w.db,
            me: "me-1",
            own_keys: None,
        }
        .decide("bob-1", "review", tomorrow);
        assert_eq!(gate.unwrap().decision, Decision::Auto);
    }
}
