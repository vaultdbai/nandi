//! Group policies on incoming work: for a request from someone I share
//! groups with, my own access level for each group, and the relay's marks,
//! then the strictest of those. No shared group and no mark: `None`, and
//! everything stays as it was (the company relay's trust rules and
//! intake).
//!
//! The rules in a group's policy are the relay's (owner, 2026-11-14): it
//! refuses what a group doesn't allow at `SubmitTask`, and marks what a
//! group has me approve first (`SealedTask.first_time_groups` for a first
//! request, `SealedTask.group_holds` for hours, `always_ask` and
//! `approval: always`). A marked task waits for me whatever my level says.
//! The marks are kept here by task (`group_holds`, from delivery on, so a
//! restart keeps them).
//!
//! Who is in a group comes from the copies in `.nandi/groups/`, read on
//! every request. A copy that doesn't read is skipped (its members are
//! unknown) and logged; the next refresh rewrites it.

use nandi_protocol::{GroupHold, HOLD_ALWAYS_ASK, HOLD_APPROVAL_ALWAYS, HOLD_OUTSIDE_HOURS};
use rusqlite::params;

use crate::db::Db;
use crate::services::domain::group::{Decision, GroupAccess, GroupCopy};
use crate::services::relay::protocol::ASK_ME;
use crate::services::repos::GroupRepository;

/// The code a first request the relay marked is kept under: the relay
/// marks it apart from the other holds (`SealedTask.first_time_groups`)
const FIRST_REQUEST: &str = "first_request";

/// Why the relay held a request, as the owner reads it, by its code
fn why_held(code: &str) -> &'static str {
    match code {
        FIRST_REQUEST => "the group's policy has you approve a member's first request",
        HOLD_OUTSIDE_HOURS => "it came outside the group's hours",
        HOLD_ALWAYS_ASK => "the group's policy always asks you first for this",
        HOLD_APPROVAL_ALWAYS => "the group's policy has you approve every request",
        // A code this Nandi doesn't know still holds
        _ => "the group's policy has you approve it",
    }
}

/// Everything [`decide`](Enforcer::decide) reads
pub struct Enforcer<'a> {
    pub repo: &'a GroupRepository,
    /// This Nandi's instance ID
    pub me: &'a str,
}

impl Enforcer<'_> {
    /// The groups' say on a request for `capability` from `requester`,
    /// which the relay held for me in `holds`; each reason names its
    /// group. `None` when I share no group with them and nothing is held.
    pub fn decide(
        &self,
        requester: &str,
        capability: &str,
        holds: &[GroupHold],
    ) -> Option<Decision> {
        if requester == self.me {
            return None;
        }
        let (copies, broken) = match self.repo.load_all() {
            Ok(loaded) => loaded,
            Err(e) => {
                log::warn!("Groups: couldn't read the group copies: {e}");
                (Vec::new(), Vec::new())
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
        // A hold for a group I have no copy of still waits for me
        let held_label = |group_id: &str| match shared.iter().find(|c| c.group_id == group_id) {
            Some(copy) => label(copy),
            None => format!("Group {group_id}"),
        };
        let held: Vec<Decision> = holds
            .iter()
            .map(|hold| {
                Decision::Ask(format!(
                    "{}: {}",
                    held_label(&hold.group_id),
                    why_held(&hold.code)
                ))
            })
            .collect();
        if shared.is_empty() {
            return Decision::strictest(held);
        }
        let access = self.repo.load_access().unwrap_or_else(|e| {
            log::warn!("Groups: my access levels don't read ({e}); asking first everywhere");
            GroupAccess::default()
        });
        let is_question = capability == ASK_ME;
        let mine =
            shared.iter().map(
                |copy| match access.level(&copy.group_id).decide(is_question) {
                    Decision::Auto => Decision::Auto,
                    Decision::Ask(why) => Decision::Ask(format!("{}: {why}", label(copy))),
                    Decision::Refuse(why) => Decision::Refuse(format!("{}: {why}", label(copy))),
                },
            );
        Decision::strictest(mine.chain(held))
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
}

impl GroupRules {
    pub fn new(repo: GroupRepository, db: Db, me: String) -> Self {
        Self { repo, db, me }
    }

    pub fn of(state: &crate::services::AppState) -> Result<Self, String> {
        Ok(Self::for_install(
            state.store(),
            state.db().clone(),
            crate::services::network::local_instance_id(state.db())?,
        ))
    }

    /// The rules for this install
    pub fn for_install(store: &crate::services::repo_store::RepoStore, db: Db, me: String) -> Self {
        Self::new(store.repos().groups.clone(), db, me)
    }

    /// The groups' say on task `task_id`, a request from `requester` for
    /// `capability`
    pub fn decide(&self, task_id: &str, requester: &str, capability: &str) -> Option<Decision> {
        let holds = held(&self.db, task_id);
        Enforcer {
            repo: &self.repo,
            me: &self.me,
        }
        .decide(requester, capability, &holds)
    }

    /// The relay delivered `task_id` marked as a member's first request in
    /// `first_time_groups` and held in `holds`: kept until it's decided,
    /// over a restart
    pub fn keep_holds(
        &self,
        task_id: &str,
        first_time_groups: &[String],
        holds: &[GroupHold],
    ) -> Result<(), String> {
        let first = first_time_groups
            .iter()
            .map(|group| (group.as_str(), FIRST_REQUEST));
        let others = holds
            .iter()
            .map(|hold| (hold.group_id.as_str(), hold.code.as_str()));
        let conn = self.db.conn()?;
        for (group, code) in first.chain(others) {
            conn.execute(
                "INSERT OR IGNORE INTO group_holds (task_id, group_id, code) VALUES (?1, ?2, ?3)",
                params![task_id, group, code],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// The holds the relay put on `task_id`, by group and code
fn held(db: &Db, task_id: &str) -> Vec<GroupHold> {
    let read = || -> Result<Vec<GroupHold>, String> {
        let conn = db.conn()?;
        let mut statement = conn
            .prepare(
                "SELECT group_id, code FROM group_holds WHERE task_id = ?1 ORDER BY group_id, code",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![task_id], |row| {
                Ok(GroupHold {
                    group_id: row.get(0)?,
                    code: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<GroupHold>>>()
            .map_err(|e| e.to_string())
    };
    read().unwrap_or_else(|e| {
        // Unknown: it waits for me, the safe side
        log::warn!("Groups: couldn't read task {task_id}'s holds: {e}");
        vec![GroupHold {
            group_id: "unknown".to_string(),
            code: String::new(),
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::group::{AccessLevel, CopyMember};
    use crate::services::repos::{RepoLayout, Repos};
    use nandi_protocol::GroupRole;

    const G1: &str = "g_0000000000000001";
    const G2: &str = "g_0000000000000002";
    const UNKNOWN: &str = "g_00000000000000ff";

    struct World {
        _tmp: tempfile::TempDir,
        repos: Repos,
        db: Db,
    }

    fn world() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        World {
            _tmp: tmp,
            repos,
            db: Db::in_memory(),
        }
    }

    impl World {
        fn group(&self, id: &str, name: &str, members: &[&str]) {
            let copy = GroupCopy {
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
                text: "---\nrequests:\n  approval: always\n---\nWhy.\n".into(),
            };
            self.repos.groups.save(&copy).unwrap();
        }

        fn level(&self, id: &str, level: AccessLevel) {
            let mut access = self.repos.groups.load_access().unwrap();
            access.set(id, level).unwrap();
            self.repos.groups.save_access(&access).unwrap();
        }

        /// As the relay delivered it, held in `holds` (group, code)
        fn decide(&self, from: &str, capability: &str, holds: &[(&str, &str)]) -> Option<Decision> {
            let holds: Vec<GroupHold> = holds
                .iter()
                .map(|(group_id, code)| GroupHold {
                    group_id: (*group_id).into(),
                    code: (*code).into(),
                })
                .collect();
            Enforcer {
                repo: &self.repos.groups,
                me: "me-1",
            }
            .decide(from, capability, &holds)
        }

        fn rules(&self) -> GroupRules {
            GroupRules::new(self.repos.groups.clone(), self.db.clone(), "me-1".into())
        }
    }

    /// What a decision is, and the start of its reason
    #[derive(Debug, PartialEq)]
    enum Says {
        Nothing,
        Auto,
        Ask(&'static str),
        Refuse(&'static str),
    }

    fn says(decision: Option<Decision>, expected: &Says) -> bool {
        match (decision, expected) {
            (None, Says::Nothing) => true,
            (Some(Decision::Auto), Says::Auto) => true,
            (Some(Decision::Ask(why)), Says::Ask(start)) => why.starts_with(start),
            (Some(Decision::Refuse(why)), Says::Refuse(start)) => why.starts_with(start),
            _ => false,
        }
    }

    #[test]
    fn my_level_and_the_relays_holds_the_strictest_wins() {
        struct Case {
            name: &'static str,
            /// My level in Finance (G1); Ops (G2) stays fully automated
            level: AccessLevel,
            from: &'static str,
            capability: &'static str,
            holds: &'static [(&'static str, &'static str)],
            says: Says,
        }
        use AccessLevel::{AskFirst, FullyAutomated, None as NoAccess, QuestionsOnly, SendMeInfo};
        let cases = [
            Case {
                name: "someone I share no group with",
                level: FullyAutomated,
                from: "carol-1",
                capability: "review",
                holds: &[],
                says: Says::Nothing,
            },
            Case {
                name: "my own work",
                level: NoAccess,
                from: "me-1",
                capability: "review",
                holds: &[],
                says: Says::Nothing,
            },
            Case {
                name: "fully automated, nothing held",
                level: FullyAutomated,
                from: "bob-1",
                capability: "review",
                holds: &[],
                says: Says::Auto,
            },
            Case {
                name: "ask first",
                level: AskFirst,
                from: "bob-1",
                capability: "review",
                holds: &[],
                says: Says::Ask("Finance: the owner approves"),
            },
            Case {
                name: "questions only, a question",
                level: QuestionsOnly,
                from: "bob-1",
                capability: ASK_ME,
                holds: &[],
                says: Says::Auto,
            },
            Case {
                name: "questions only, work",
                level: QuestionsOnly,
                from: "bob-1",
                capability: "review",
                holds: &[],
                says: Says::Refuse("Finance: "),
            },
            Case {
                name: "send me info, a question",
                level: SendMeInfo,
                from: "bob-1",
                capability: ASK_ME,
                holds: &[],
                says: Says::Ask("Finance: "),
            },
            Case {
                name: "held for approval: always, whatever my level",
                level: FullyAutomated,
                from: "bob-1",
                capability: "review",
                holds: &[(G1, HOLD_APPROVAL_ALWAYS)],
                says: Says::Ask("Finance: the group's policy has you approve every request"),
            },
            Case {
                name: "held outside the hours, named by the other group",
                level: FullyAutomated,
                from: "bob-1",
                capability: "review",
                holds: &[(G2, HOLD_OUTSIDE_HOURS)],
                says: Says::Ask("Ops: it came outside the group's hours"),
            },
            Case {
                name: "held for always_ask",
                level: FullyAutomated,
                from: "bob-1",
                capability: "send-email",
                holds: &[(G1, HOLD_ALWAYS_ASK)],
                says: Says::Ask("Finance: the group's policy always asks"),
            },
            Case {
                name: "a first request",
                level: FullyAutomated,
                from: "bob-1",
                capability: "review",
                holds: &[(G1, FIRST_REQUEST)],
                says: Says::Ask("Finance: the group's policy has you approve a member's first"),
            },
            Case {
                name: "a code this Nandi doesn't know still holds",
                level: FullyAutomated,
                from: "bob-1",
                capability: "review",
                holds: &[(G1, "something_new")],
                says: Says::Ask("Finance: the group's policy has you approve it"),
            },
            Case {
                name: "a hold for a group I have no copy of",
                level: FullyAutomated,
                from: "carol-1",
                capability: "review",
                holds: &[(UNKNOWN, HOLD_APPROVAL_ALWAYS)],
                says: Says::Ask("Group g_00000000000000ff: "),
            },
            Case {
                name: "a refusal stays a refusal when held",
                level: NoAccess,
                from: "bob-1",
                capability: "review",
                holds: &[(G1, HOLD_APPROVAL_ALWAYS)],
                says: Says::Refuse("Finance: the owner takes no requests"),
            },
        ];
        for case in cases {
            let w = world();
            w.group(G1, "Finance", &["me-1", "bob-1"]);
            w.group(G2, "Ops", &["me-1", "bob-1"]);
            w.level(G1, case.level);
            w.level(G2, FullyAutomated);
            let decision = w.decide(case.from, case.capability, case.holds);
            assert!(
                says(decision.clone(), &case.says),
                "{}: got {decision:?}, want {:?}",
                case.name,
                case.says
            );
        }
    }

    #[test]
    fn a_copy_without_a_name_is_named_by_its_id() {
        let w = world();
        w.group(G1, " ", &["me-1", "bob-1"]);
        let decision = w.decide("bob-1", "review", &[]);
        assert!(says(decision, &Says::Ask("Group g_0000000000000001: ")));
    }

    #[test]
    fn holds_are_kept_by_task() {
        let w = world();
        let rules = w.rules();
        let held_for = |task: &str| -> Vec<(String, String)> {
            held(&w.db, task)
                .into_iter()
                .map(|hold| (hold.group_id, hold.code))
                .collect()
        };
        let always = GroupHold {
            group_id: G2.into(),
            code: HOLD_APPROVAL_ALWAYS.into(),
        };
        rules
            .keep_holds("t1", &[G1.into()], std::slice::from_ref(&always))
            .unwrap();
        // Delivered again: kept once
        rules.keep_holds("t1", &[G1.into()], &[always]).unwrap();
        assert_eq!(
            held_for("t1"),
            [
                (G1.to_string(), FIRST_REQUEST.to_string()),
                (G2.to_string(), HOLD_APPROVAL_ALWAYS.to_string()),
            ]
        );
        assert!(held_for("t2").is_empty());
        // Read back as the task's say
        w.group(G1, "Finance", &["me-1", "bob-1"]);
        w.level(G1, AccessLevel::FullyAutomated);
        let decision = rules.decide("t1", "bob-1", "review");
        assert!(says(decision, &Says::Ask("Finance: ")));
        assert!(says(rules.decide("t2", "bob-1", "review"), &Says::Auto));
    }
}
