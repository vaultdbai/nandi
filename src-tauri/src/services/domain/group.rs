//! Groups (protocol 7) as this Nandi keeps them: the copy of each group's
//! signed policy, my access level per group, and the rule that decides what
//! happens to a request from someone I share a group with.
//!
//! - **The copy** (`.nandi/groups/<group-id>.md`, [`GroupCopy`]): Nandi's
//!   own frontmatter (`group`, `name`, `version`, `author`, `signature`,
//!   `updated_at`, `members`), then the policy text exactly as the admin
//!   signed it. The text is itself Markdown with YAML frontmatter (the
//!   format of `.nandi/policies/`, plus a `requests:` block).
//! - **My access levels** (`.nandi/groups/access.yaml`, [`GroupAccess`]):
//!   one [`AccessLevel`] per group, "Ask first" until I pick another.
//! - **The rule** ([`AccessLevel::decide`]): my level gives a [`Decision`]
//!   for one request; across several groups I share with the requester,
//!   the strictest applies ([`Decision::strictest`]).
//! - **The relay's part**: every rule in a policy's `requests:` block
//!   involves the other person, so the relay enforces them all at
//!   `SubmitTask` ([`GroupRequestRules`]). It refuses what a group doesn't
//!   allow, and marks what a group has me approve first; a marked request
//!   waits for me whatever my level says (`services::groups::enforce`).
//!   Here the block is only checked to read the way the relay reads it.
//!
//! A copy is trusted only when its signature checks out against the
//! author's key ([`verify`]); the groups screen says when one doesn't.

use std::collections::BTreeMap;

use nandi_protocol::{GroupPolicy, GroupRequestRules, GroupRole, PublicKeys};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::agent::split_frontmatter;
use super::policy::Policy;
use super::{DomainError, DomainResult};

/// What I let people do for me through one group
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "kebab-case")]
pub enum AccessLevel {
    /// Their requests run without asking me, when screening is clean and
    /// the relay didn't hold them for the group's policy
    FullyAutomated,
    /// Every request waits for my approval
    #[default]
    AskFirst,
    /// Only questions (`ask-me`), answered from my notes; nothing else
    QuestionsOnly,
    /// Only information for my queue; nothing runs
    SendMeInfo,
    /// Nothing
    None,
}

impl AccessLevel {
    /// What this level says about one request: a question to the owner
    /// (`ask-me`) when `is_question`, other work when not
    pub fn decide(self, is_question: bool) -> Decision {
        match self {
            Self::FullyAutomated => Decision::Auto,
            Self::AskFirst => {
                Decision::Ask("the owner approves requests from this group first".into())
            }
            Self::QuestionsOnly if is_question => Decision::Auto,
            Self::QuestionsOnly => {
                Decision::Refuse("the owner takes only questions through this group".into())
            }
            // No intake for information on its own yet: a question lands in
            // the owner's queue and waits for them
            Self::SendMeInfo if is_question => Decision::Ask(
                "the owner reads what this group sends before anything is answered".into(),
            ),
            Self::SendMeInfo => {
                Decision::Refuse("the owner takes only information through this group".into())
            }
            Self::None => Decision::Refuse("the owner takes no requests through this group".into()),
        }
    }
}

/// What happens to one request
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Runs without asking (screening still applies)
    Auto,
    /// Waits for the owner's approval, with why
    Ask(String),
    /// Refused, with why
    Refuse(String),
}

impl Decision {
    fn rank(&self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::Ask(_) => 1,
            Self::Refuse(_) => 2,
        }
    }

    /// The stricter of the two (the first when they're as strict)
    pub fn stricter(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    /// The strictest of several; `None` for none
    pub fn strictest(decisions: impl IntoIterator<Item = Self>) -> Option<Self> {
        decisions.into_iter().reduce(Self::stricter)
    }
}

/// A group's policy text, parsed: its `requests:` block as the relay reads
/// it, and the other blocks checked like any policy (`domain::policy`)
#[derive(Debug, Clone, PartialEq)]
pub struct GroupPolicyText {
    pub requests: GroupRequestRules,
}

impl GroupPolicyText {
    /// Parses a group's policy file. No frontmatter, or no `requests:`
    /// block: every request waits for approval (`approval: always`).
    pub fn parse(group_id: &str, text: &str) -> DomainResult<Self> {
        let what = format!("group policy '{group_id}'");
        split_frontmatter(text).map_err(|e| DomainError::parse(&what, e))?;
        // The rules the relay enforces read here as they read there
        let requests = GroupRequestRules::read(text)
            .map_err(|e| DomainError::invalid(&what, "requests", e))?;
        // The other blocks (tools, connectors, network, …) follow the
        // policy format; `requests` is a note to it
        Policy::parse(&format!("group-{group_id}"), text)?;
        Ok(Self { requests })
    }
}

/// The policy a new group starts from, with what each `requests:` field
/// does
pub const POLICY_TEMPLATE: &str = r#"---
# What members of this group may have done for each other, and how.
# The relay applies these rules; each person can be stricter for themselves (My access level).
requests:
  # always: every request waits for the person's approval
  # first-time: a member's first request to each person waits; later ones follow each person's level
  # per-member: each person's own access level decides
  approval: always
  # Capabilities members may ask for (globs; empty allows any) and never ask for
  capabilities: { allow: [], deny: [] }
  # Most requests one member may send each person per day (leave out for no limit)
  # daily_limit: 20
  # When requests may run without waiting; outside them they wait
  # hours: "Mon-Fri 08:00-18:00"
  # The time zone of the hours (an IANA name; UTC when left out)
  # time_zone: Europe/London
  # Capabilities (or words in them) that always wait for approval
  always_ask: [send, payment]
---
What this group is for, and anything members should know.
"#;

/// A group ID as the relay assigns it, safe as a file name
pub fn check_group_id(id: &str) -> DomainResult<()> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(DomainError::invalid(
            format!("group '{id}'"),
            "id",
            "must be 1-128 letters, digits, '-', '_' or '.', not starting with '.'",
        ))
    }
}

/// One member, as the copy keeps them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyMember {
    pub instance_id: String,
    pub role: GroupRole,
}

/// This Nandi's copy of a group it's in: who's in it and its signed policy
#[derive(Debug, Clone, PartialEq)]
pub struct GroupCopy {
    pub group_id: String,
    pub name: String,
    pub version: u32,
    pub author: String,
    pub signature: String,
    pub updated_at: f64,
    pub members: Vec<CopyMember>,
    /// The policy text, exactly as signed
    pub text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyFront {
    group: String,
    #[serde(default)]
    name: String,
    version: u32,
    author: String,
    signature: String,
    #[serde(default)]
    updated_at: f64,
    #[serde(default)]
    members: Vec<CopyMember>,
}

const COPY_HEADER: &str =
    "# This Nandi's copy of the group's signed policy, refreshed from the relay.\n\
# Read-only: admins change it in Network > Groups. Editing it here breaks the signature.\n";

impl GroupCopy {
    /// Reads `.nandi/groups/<id>.md`
    pub fn parse(id: &str, content: &str) -> DomainResult<Self> {
        let what = format!("group copy '{id}'");
        let (yaml, text) = split_frontmatter(content).map_err(|e| DomainError::parse(&what, e))?;
        let yaml = yaml.ok_or_else(|| DomainError::parse(&what, "has no frontmatter"))?;
        let front: CopyFront =
            serde_yaml::from_str(yaml).map_err(|e| DomainError::parse(&what, e))?;
        if front.group != id {
            return Err(DomainError::invalid(
                &what,
                "group",
                format!("is '{}', not the file's name", front.group),
            ));
        }
        check_group_id(&front.group)?;
        Ok(Self {
            group_id: front.group,
            name: front.name,
            version: front.version,
            author: front.author,
            signature: front.signature,
            updated_at: front.updated_at,
            members: front.members,
            text: text.to_string(),
        })
    }

    /// The file: Nandi's frontmatter, then the policy text unchanged
    pub fn render(&self) -> String {
        let front = CopyFront {
            group: self.group_id.clone(),
            name: self.name.clone(),
            version: self.version,
            author: self.author.clone(),
            signature: self.signature.clone(),
            updated_at: self.updated_at,
            members: self.members.clone(),
        };
        let yaml = serde_yaml::to_string(&front).unwrap_or_default();
        format!("---\n{COPY_HEADER}{yaml}---\n{}", self.text)
    }

    /// Whether `instance` is a member
    pub fn has_member(&self, instance: &str) -> bool {
        self.members.iter().any(|m| m.instance_id == instance)
    }

    #[cfg(test)]
    pub fn role_of(&self, instance: &str) -> Option<GroupRole> {
        self.members
            .iter()
            .find(|m| m.instance_id == instance)
            .map(|m| m.role)
    }

    /// The policy as the relay carries it
    #[cfg(test)]
    pub fn policy(&self) -> GroupPolicy {
        GroupPolicy {
            version: self.version,
            text: self.text.clone(),
            author: self.author.clone(),
            signature: self.signature.clone(),
            updated_at: self.updated_at,
        }
    }
}

/// Whether a policy is the group's own: signed by `author_keys` over this
/// group, version and text. `None` keys: unknown, so not verified.
pub fn verify(
    group_id: &str,
    policy: &GroupPolicy,
    author_keys: Option<&PublicKeys>,
) -> Result<(), String> {
    let keys =
        author_keys.ok_or_else(|| format!("no known key for its author {}", policy.author))?;
    let bytes = GroupPolicy::signed_bytes(group_id, policy.version, &policy.text);
    nandi_seal::verify(keys, &bytes, &policy.signature).map_err(|e| {
        format!(
            "its signature doesn't check out against {}'s key: {e}",
            policy.author
        )
    })
}

/// My access level for each group I'm in (`.nandi/groups/access.yaml`)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupAccess {
    levels: BTreeMap<String, AccessLevel>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AccessFile {
    #[serde(default)]
    groups: BTreeMap<String, AccessLevel>,
}

const ACCESS_HEADER: &str =
    "# My access level for each group I'm in: fully-automated, ask-first,\n\
# questions-only, send-me-info or none. The stricter of this and the group's\n\
# policy applies. A group not listed is ask-first.\n";

impl GroupAccess {
    pub fn parse(content: &str) -> DomainResult<Self> {
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        let file: AccessFile = serde_yaml::from_str(content)
            .map_err(|e| DomainError::parse("group access levels", e))?;
        for id in file.groups.keys() {
            check_group_id(id)?;
        }
        Ok(Self {
            levels: file.groups,
        })
    }

    pub fn render(&self) -> String {
        let yaml = serde_yaml::to_string(&AccessFile {
            groups: self.levels.clone(),
        })
        .unwrap_or_default();
        format!("{ACCESS_HEADER}{yaml}")
    }

    /// My level for `group` ("Ask first" when I never picked one)
    pub fn level(&self, group: &str) -> AccessLevel {
        self.levels.get(group).copied().unwrap_or_default()
    }

    /// Whether I picked a level for `group`
    pub fn has(&self, group: &str) -> bool {
        self.levels.contains_key(group)
    }

    pub fn set(&mut self, group: &str, level: AccessLevel) -> DomainResult<()> {
        check_group_id(group)?;
        self.levels.insert(group.to_string(), level);
        Ok(())
    }

    pub fn remove(&mut self, group: &str) -> bool {
        self.levels.remove(group).is_some()
    }

    #[cfg(test)]
    pub fn groups(&self) -> impl Iterator<Item = &str> {
        self.levels.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use nandi_protocol::GroupApproval;

    use super::*;

    fn parse(yaml: &str) -> DomainResult<GroupPolicyText> {
        GroupPolicyText::parse("g1", &format!("---\n{yaml}---\nWhy.\n"))
    }

    #[test]
    fn the_template_reads_as_the_relay_reads_it() {
        let parsed = GroupPolicyText::parse("g1", POLICY_TEMPLATE).unwrap();
        assert_eq!(
            parsed.requests,
            GroupRequestRules::read(POLICY_TEMPLATE).unwrap()
        );
        assert_eq!(parsed.requests.approval, GroupApproval::Always);
        assert_eq!(parsed.requests.always_ask, ["send", "payment"]);
    }

    #[test]
    fn no_requests_block_means_approval_always() {
        let parsed = GroupPolicyText::parse("g1", "Just prose.\n").unwrap();
        assert_eq!(parsed.requests, GroupRequestRules::default());
        let parsed = GroupPolicyText::parse("g1", "---\nname: x\n---\n").unwrap();
        assert_eq!(parsed.requests.approval, GroupApproval::Always);
    }

    #[test]
    fn bad_policies_say_what_to_fix() {
        // (the frontmatter, what the error names)
        let cases = [
            ("requests:\n  approval: sometimes\n", "requests"),
            ("requests:\n  nope: 1\n", "nope"),
            ("requests:\n  hours: \"Mon-Fri 18:00-08:00\"\n", "overnight"),
            (
                "requests:\n  hours: \"Funday 08:00-10:00\"\n",
                "isn't a day",
            ),
            (
                "requests:\n  hours: \"Mon-Fri 8am-6pm\"\n",
                "requests.hours",
            ),
            (
                "requests:\n  hours: \"Mon 08:00-10:00\"\n  time_zone: Nowhere\n",
                "time zone",
            ),
            ("requests:\n  daily_limit: 0\n", "daily_limit"),
            (
                "requests:\n  capabilities: { allow: ['[x'] }\n",
                "bad pattern",
            ),
            // The other blocks are checked like any policy's
            ("tools: { deny: [shell] }\n", "applies_to"),
            ("requests: [\n", "frontmatter doesn't read"),
        ];
        for (yaml, names) in cases {
            let error = parse(yaml).unwrap_err().to_string();
            assert!(error.contains(names), "{yaml}: {error}");
        }
    }

    /// A decision without its reason
    fn kind(d: &Decision) -> &'static str {
        match d {
            Decision::Auto => "auto",
            Decision::Ask(_) => "ask",
            Decision::Refuse(_) => "refuse",
        }
    }

    #[test]
    fn each_access_level_decides() {
        // (level, on a question, on other work)
        let cases = [
            (AccessLevel::FullyAutomated, "auto", "auto"),
            (AccessLevel::AskFirst, "ask", "ask"),
            (AccessLevel::QuestionsOnly, "auto", "refuse"),
            (AccessLevel::SendMeInfo, "ask", "refuse"),
            (AccessLevel::None, "refuse", "refuse"),
        ];
        for (level, question, work) in cases {
            assert_eq!(
                kind(&level.decide(true)),
                question,
                "{level:?} on a question"
            );
            assert_eq!(kind(&level.decide(false)), work, "{level:?} on work");
        }
        assert_eq!(AccessLevel::default(), AccessLevel::AskFirst);
    }

    #[test]
    fn the_strictest_decision_wins() {
        let all = Decision::strictest([Decision::Auto, Decision::Ask("a".into()), Decision::Auto]);
        assert_eq!(all, Some(Decision::Ask("a".into())));
        let all = Decision::strictest([Decision::Ask("a".into()), Decision::Refuse("r".into())]);
        assert_eq!(all, Some(Decision::Refuse("r".into())));
        let first = Decision::Ask("first".into()).stricter(Decision::Ask("second".into()));
        assert_eq!(first, Decision::Ask("first".into()));
        assert_eq!(Decision::strictest(Vec::new()), None);
    }

    fn copy() -> GroupCopy {
        GroupCopy {
            group_id: "g1".into(),
            name: "Finance".into(),
            version: 2,
            author: "alice-1".into(),
            signature: "sig".into(),
            updated_at: 5.0,
            members: vec![
                CopyMember {
                    instance_id: "alice-1".into(),
                    role: GroupRole::Admin,
                },
                CopyMember {
                    instance_id: "bob-1".into(),
                    role: GroupRole::Member,
                },
            ],
            text: POLICY_TEMPLATE.to_string(),
        }
    }

    #[test]
    fn a_copy_round_trips_with_its_text_unchanged() {
        let copy = copy();
        let file = copy.render();
        assert!(file.starts_with("---\n# This Nandi's copy"));
        let back = GroupCopy::parse("g1", &file).unwrap();
        assert_eq!(back, copy);
        assert_eq!(back.text, POLICY_TEMPLATE);
        assert!(back.has_member("bob-1") && !back.has_member("eve"));
        assert_eq!(back.role_of("alice-1"), Some(GroupRole::Admin));
        // Named after another group: refused
        assert!(GroupCopy::parse("g2", &file).is_err());
        assert!(GroupCopy::parse("g1", "no frontmatter").is_err());
    }

    #[test]
    fn group_ids_are_safe_file_names() {
        assert!(check_group_id("g-1_a.b").is_ok());
        for bad in ["", ".hidden", "../x", "a/b", "a b"] {
            assert!(check_group_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_signature_checks_out_only_for_its_group_version_and_text() {
        let keys = nandi_seal::DeviceKeys::generate().unwrap();
        let other = nandi_seal::DeviceKeys::generate().unwrap();
        let mut policy = copy().policy();
        policy.signature = keys.sign(&GroupPolicy::signed_bytes("g1", 2, &policy.text));
        assert!(verify("g1", &policy, Some(keys.public_keys())).is_ok());
        assert!(
            verify("g2", &policy, Some(keys.public_keys())).is_err(),
            "another group"
        );
        assert!(
            verify("g1", &policy, Some(other.public_keys())).is_err(),
            "another key"
        );
        assert!(verify("g1", &policy, None)
            .unwrap_err()
            .contains("no known key"));
        let mut edited = policy.clone();
        edited.text.push('!');
        assert!(
            verify("g1", &edited, Some(keys.public_keys())).is_err(),
            "edited text"
        );
        let mut replayed = policy.clone();
        replayed.version = 3;
        assert!(
            verify("g1", &replayed, Some(keys.public_keys())).is_err(),
            "another version"
        );
    }

    #[test]
    fn access_levels_default_to_ask_first_and_round_trip() {
        let mut access = GroupAccess::parse("").unwrap();
        assert_eq!(access.level("g1"), AccessLevel::AskFirst);
        assert!(!access.has("g1"));
        access.set("g1", AccessLevel::QuestionsOnly).unwrap();
        access.set("g2", AccessLevel::None).unwrap();
        assert!(access.set("../x", AccessLevel::None).is_err());
        let file = access.render();
        assert!(file.contains("g1: questions-only"), "{file}");
        let back = GroupAccess::parse(&file).unwrap();
        assert_eq!(back, access);
        assert!(access.remove("g2"));
        assert_eq!(access.groups().collect::<Vec<_>>(), ["g1"]);
        assert!(GroupAccess::parse("groups:\n  g1: sometimes\n").is_err());
        assert!(GroupAccess::parse("other: 1\n").is_err());
    }
}
