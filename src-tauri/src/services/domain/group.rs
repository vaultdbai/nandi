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
//! - **The rule** ([`RequestRules::decide`], [`AccessLevel::decide`]): each
//!   gives a [`Decision`] for one request; the stricter one applies
//!   ([`Decision::stricter`]), so I can tighten a group's policy, never
//!   loosen it. Across several groups I share with the requester, the
//!   strictest applies.
//!
//! A copy is trusted only when its signature checks out against the
//! author's key ([`verify`]); anything else fails closed.

use std::collections::BTreeMap;

use nandi_protocol::{GroupPolicy, GroupRole, PublicKeys};
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
    /// the group's policy allows it
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
    /// What this level says about one request
    pub fn decide(self, facts: &RequestFacts<'_>) -> Decision {
        match self {
            Self::FullyAutomated => Decision::Auto,
            Self::AskFirst => {
                Decision::Ask("the owner approves requests from this group first".into())
            }
            Self::QuestionsOnly if facts.is_question => Decision::Auto,
            Self::QuestionsOnly => {
                Decision::Refuse("the owner takes only questions through this group".into())
            }
            // No intake for information on its own yet: a question lands in
            // the owner's queue and waits for them
            Self::SendMeInfo if facts.is_question => Decision::Ask(
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

/// When a group's members may have work done without asking
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum Approval {
    /// Every request waits for the owner
    #[default]
    Always,
    /// A member's first request waits; later ones follow the owner's level
    FirstTime,
    /// Each owner's own access level decides
    PerMember,
}

/// Capabilities a group lets its members ask for
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRules {
    /// Only these (globs); empty: any
    #[serde(default)]
    pub allow: Vec<String>,
    /// Never these (globs); wins over `allow`
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Days of the week and a time of day (local), e.g. `Mon-Fri 08:00-18:00`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hours {
    text: String,
    /// Monday first
    days: [bool; 7],
    /// Minutes since midnight, `start < end`
    start: u16,
    end: u16,
}

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Hours {
    /// `Mon-Fri 08:00-18:00`, `Mon,Wed 09:00-12:00`, `08:00-18:00` (every
    /// day) or `Sat-Sun` (all day)
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let mut days = [true; 7];
        let (mut start, mut end) = (0u16, 24 * 60);
        let mut seen_days = false;
        let mut seen_time = false;
        for part in text.split_whitespace() {
            if part.contains(':') && !seen_time {
                let (from, to) = part
                    .split_once('-')
                    .ok_or_else(|| format!("'{part}' isn't a time range like 08:00-18:00"))?;
                start = minutes(from)?;
                end = minutes(to)?;
                if start >= end {
                    return Err(format!(
                        "'{part}' ends before it starts (overnight hours aren't supported)"
                    ));
                }
                seen_time = true;
            } else if !seen_days && !seen_time {
                days = [false; 7];
                for item in part.split(',') {
                    let (from, to) = item.split_once('-').unwrap_or((item, item));
                    let (from, to) = (day(from)?, day(to)?);
                    if from > to {
                        return Err(format!("'{item}' runs backwards; write it Monday first"));
                    }
                    days[from..=to].iter_mut().for_each(|d| *d = true);
                }
                seen_days = true;
            } else {
                return Err(format!(
                    "'{text}' isn't like 'Mon-Fri 08:00-18:00': unexpected '{part}'"
                ));
            }
        }
        if !seen_days && !seen_time {
            return Err("is empty; write it like 'Mon-Fri 08:00-18:00'".to_string());
        }
        Ok(Self {
            text: text.to_string(),
            days,
            start,
            end,
        })
    }

    /// Whether `weekday` (0 = Monday) at `minute` (since midnight) is inside
    pub fn contains(&self, weekday: u8, minute: u16) -> bool {
        self.days
            .get(usize::from(weekday))
            .copied()
            .unwrap_or(false)
            && (self.start..self.end).contains(&minute)
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

fn day(text: &str) -> Result<usize, String> {
    let lower = text.trim().to_ascii_lowercase();
    DAYS.iter()
        .position(|d| lower.starts_with(d))
        .ok_or_else(|| format!("'{text}' isn't a day (Mon, Tue, Wed, Thu, Fri, Sat, Sun)"))
}

fn minutes(text: &str) -> Result<u16, String> {
    let bad = || format!("'{text}' isn't a time like 08:00");
    let (h, m) = text.trim().split_once(':').ok_or_else(bad)?;
    let (h, m): (u16, u16) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if m >= 60 || h > 24 || (h == 24 && m > 0) {
        return Err(bad());
    }
    Ok(h * 60 + m)
}

/// The `requests:` block of a group's policy: what members may have done
/// for each other, and how
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestRules {
    pub approval: Approval,
    pub capabilities: CapabilityRules,
    /// Per member per day
    pub daily_limit: Option<u32>,
    pub hours: Option<Hours>,
    /// Capabilities (or words in them) that always wait for the owner
    pub always_ask: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestsYaml {
    #[serde(default)]
    approval: Option<Approval>,
    #[serde(default)]
    capabilities: Option<CapabilityRules>,
    #[serde(default)]
    daily_limit: Option<u32>,
    #[serde(default)]
    hours: Option<String>,
    #[serde(default)]
    always_ask: Vec<String>,
}

/// One request, as the rules see it
#[derive(Debug, Clone, Copy)]
pub struct RequestFacts<'a> {
    /// What it asks for (`ask-me`, `assign-me`, an agent's capability)
    pub capability: &'a str,
    /// A question to the owner (`ask-me`)
    pub is_question: bool,
    /// No earlier request from this member through this group went ahead
    pub first_time: bool,
    /// Requests from this member through this group that went ahead today
    pub today: u32,
    /// 0 = Monday, local time
    pub weekday: u8,
    /// Minutes since midnight, local time
    pub minute: u16,
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let options = glob::MatchOptions {
        case_sensitive: false,
        ..glob::MatchOptions::default()
    };
    glob::Pattern::new(pattern.trim()).is_ok_and(|p| p.matches_with(value, options))
}

/// `term` names `capability`: the same, a glob over it, or one of its
/// words (`send` in `send-email`)
fn names(term: &str, capability: &str) -> bool {
    let term = term.trim();
    glob_matches(term, capability)
        || capability
            .split(['-', '_', ':', '.', '/'])
            .any(|word| word.eq_ignore_ascii_case(term))
}

impl RequestRules {
    /// What the group's policy says about one request
    pub fn decide(&self, facts: &RequestFacts<'_>) -> Decision {
        let capability = facts.capability;
        if self
            .capabilities
            .deny
            .iter()
            .any(|p| glob_matches(p, capability))
        {
            return Decision::Refuse(format!("the group's policy doesn't allow '{capability}'"));
        }
        if !self.capabilities.allow.is_empty()
            && !self
                .capabilities
                .allow
                .iter()
                .any(|p| glob_matches(p, capability))
        {
            return Decision::Refuse(format!("the group's policy doesn't allow '{capability}'"));
        }
        if let Some(limit) = self.daily_limit {
            if facts.today >= limit {
                return Decision::Refuse(format!(
                    "the group's limit of {limit} requests a day per member is reached"
                ));
            }
        }
        if let Some(hours) = &self.hours {
            if !hours.contains(facts.weekday, facts.minute) {
                return Decision::Ask(format!(
                    "it came outside the group's hours ({})",
                    hours.as_str()
                ));
            }
        }
        if let Some(term) = self.always_ask.iter().find(|t| names(t, capability)) {
            return Decision::Ask(format!("the group's policy always asks first for '{term}'"));
        }
        match self.approval {
            Approval::Always => {
                Decision::Ask("the group's policy has the owner approve every request".into())
            }
            Approval::FirstTime if facts.first_time => Decision::Ask(
                "the group's policy has the owner approve a member's first request".into(),
            ),
            Approval::FirstTime | Approval::PerMember => Decision::Auto,
        }
    }
}

/// A group's policy text, parsed: its `requests:` block, and the other
/// blocks checked like any policy (`domain::policy`)
#[derive(Debug, Clone, PartialEq)]
pub struct GroupPolicyText {
    pub requests: RequestRules,
}

impl GroupPolicyText {
    /// Parses a group's policy file. No frontmatter, or no `requests:`
    /// block: every request waits for approval (`approval: always`).
    pub fn parse(group_id: &str, text: &str) -> DomainResult<Self> {
        let what = format!("group policy '{group_id}'");
        let (yaml, _body) = split_frontmatter(text).map_err(|e| DomainError::parse(&what, e))?;
        let requests = match yaml.map(str::trim).filter(|y| !y.is_empty()) {
            Some(yaml) => {
                let front: serde_yaml::Value =
                    serde_yaml::from_str(yaml).map_err(|e| DomainError::parse(&what, e))?;
                match front.get("requests") {
                    Some(block) if !block.is_null() => {
                        let raw: RequestsYaml = serde_yaml::from_value(block.clone())
                            .map_err(|e| DomainError::invalid(&what, "requests", e.to_string()))?;
                        rules_from(&what, raw)?
                    }
                    _ => RequestRules::default(),
                }
            }
            None => RequestRules::default(),
        };
        // The other blocks (tools, connectors, network, …) follow the
        // policy format; `requests` is a note to it
        Policy::parse(&format!("group-{group_id}"), text)?;
        Ok(Self { requests })
    }
}

fn rules_from(what: &str, raw: RequestsYaml) -> DomainResult<RequestRules> {
    let check_globs = |field: &'static str, list: &[String]| -> DomainResult<()> {
        for pattern in list {
            glob::Pattern::new(pattern.trim()).map_err(|e| {
                DomainError::invalid(what, field, format!("has a bad pattern '{pattern}': {e}"))
            })?;
        }
        Ok(())
    };
    let capabilities = raw.capabilities.unwrap_or_default();
    check_globs("requests.capabilities.allow", &capabilities.allow)?;
    check_globs("requests.capabilities.deny", &capabilities.deny)?;
    let hours = raw
        .hours
        .map(|h| Hours::parse(&h).map_err(|e| DomainError::invalid(what, "requests.hours", e)))
        .transpose()?;
    if raw.daily_limit == Some(0) {
        return Err(DomainError::invalid(
            what,
            "requests.daily_limit",
            "is 0; leave it out for no limit, or deny the capabilities instead",
        ));
    }
    let always_ask: Vec<String> = raw
        .always_ask
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    check_globs("requests.always_ask", &always_ask)?;
    Ok(RequestRules {
        approval: raw.approval.unwrap_or_default(),
        capabilities,
        daily_limit: raw.daily_limit,
        hours,
        always_ask,
    })
}

/// The policy a new group starts from, with what each `requests:` field
/// does
pub const POLICY_TEMPLATE: &str = r#"---
# What members of this group may have done for each other, and how.
# Each person can be stricter for themselves (My access level), never looser.
requests:
  # always: every request waits for the person's approval
  # first-time: a member's first request waits; later ones follow each person's level
  # per-member: each person's own access level decides
  approval: always
  # Capabilities members may ask for (globs; empty allows any) and never ask for
  capabilities: { allow: [], deny: [] }
  # Most requests one member may send each person per day (leave out for no limit)
  # daily_limit: 20
  # When requests may run without waiting (local time); outside them they wait
  # hours: "Mon-Fri 08:00-18:00"
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
    use super::*;

    fn facts(capability: &str) -> RequestFacts<'_> {
        RequestFacts {
            capability,
            is_question: capability == "ask-me",
            first_time: false,
            today: 0,
            // A Wednesday at 10:00
            weekday: 2,
            minute: 600,
        }
    }

    fn rules(yaml: &str) -> RequestRules {
        GroupPolicyText::parse("g1", &format!("---\n{yaml}---\nWhy.\n"))
            .unwrap()
            .requests
    }

    fn ask(d: &Decision) -> bool {
        matches!(d, Decision::Ask(_))
    }

    fn refused(d: &Decision) -> bool {
        matches!(d, Decision::Refuse(_))
    }

    #[test]
    fn the_template_parses_and_asks_every_time() {
        let parsed = GroupPolicyText::parse("g1", POLICY_TEMPLATE).unwrap();
        assert_eq!(parsed.requests.approval, Approval::Always);
        assert_eq!(parsed.requests.always_ask, ["send", "payment"]);
        assert!(ask(&parsed.requests.decide(&facts("summarize"))));
    }

    #[test]
    fn no_requests_block_means_approval_always() {
        let parsed = GroupPolicyText::parse("g1", "Just prose.\n").unwrap();
        assert_eq!(parsed.requests, RequestRules::default());
        let parsed = GroupPolicyText::parse("g1", "---\nname: x\n---\n").unwrap();
        assert_eq!(parsed.requests.approval, Approval::Always);
    }

    #[test]
    fn the_requests_block_parses_every_field() {
        let r = rules(
            "requests:\n  approval: per-member\n  capabilities: { allow: [review-*, ask-me], deny: [review-secret] }\n  daily_limit: 20\n  hours: \"Mon-Fri 08:00-18:00\"\n  always_ask: [send]\n",
        );
        assert_eq!(r.approval, Approval::PerMember);
        assert_eq!(r.capabilities.allow, ["review-*", "ask-me"]);
        assert_eq!(r.daily_limit, Some(20));
        assert_eq!(r.hours.as_ref().unwrap().as_str(), "Mon-Fri 08:00-18:00");
        assert_eq!(
            rules("requests:\n  approval: first-time\n").approval,
            Approval::FirstTime
        );
    }

    #[test]
    fn bad_requests_blocks_say_what_to_fix() {
        let error = |yaml: &str| {
            GroupPolicyText::parse("g1", &format!("---\n{yaml}---\n"))
                .unwrap_err()
                .to_string()
        };
        assert!(error("requests:\n  approval: sometimes\n").contains("requests"));
        assert!(error("requests:\n  nope: 1\n").contains("nope"));
        assert!(error("requests:\n  hours: \"Mon-Fri 18:00-08:00\"\n").contains("overnight"));
        assert!(error("requests:\n  hours: \"Funday 08:00-10:00\"\n").contains("isn't a day"));
        assert!(error("requests:\n  hours: \"Mon-Fri 8am-6pm\"\n").contains("requests.hours"));
        assert!(error("requests:\n  daily_limit: 0\n").contains("daily_limit"));
        assert!(error("requests:\n  capabilities: { allow: ['[x'] }\n").contains("bad pattern"));
        // The other blocks are checked like any policy's
        assert!(error("tools: { deny: [shell] }\n").contains("applies_to"));
        assert!(error("requests: [\n").contains("Invalid group policy"));
    }

    #[test]
    fn hours_cover_days_and_times() {
        let h = Hours::parse("Mon-Fri 08:00-18:00").unwrap();
        assert!(h.contains(0, 8 * 60));
        assert!(h.contains(4, 18 * 60 - 1));
        assert!(!h.contains(4, 18 * 60));
        assert!(!h.contains(5, 600));
        let h = Hours::parse("mon,wed,Friday 09:00-12:00").unwrap();
        assert!(h.contains(2, 600) && !h.contains(1, 600) && h.contains(4, 600));
        let h = Hours::parse("08:00-18:00").unwrap();
        assert!(h.contains(6, 600));
        let h = Hours::parse("Sat-Sun").unwrap();
        assert!(h.contains(5, 0) && !h.contains(0, 600));
        assert!(Hours::parse("").is_err());
        assert!(Hours::parse("Fri-Mon").is_err());
        assert!(Hours::parse("Mon 08:00-25:00").is_err());
    }

    #[test]
    fn the_group_policy_refuses_then_asks_then_allows() {
        let r = rules(
            "requests:\n  approval: per-member\n  capabilities: { allow: [review-*, ask-me], deny: [review-secret] }\n  daily_limit: 2\n  hours: \"Mon-Fri 08:00-18:00\"\n  always_ask: [send]\n",
        );
        assert_eq!(r.decide(&facts("review-pr")), Decision::Auto);
        assert!(refused(&r.decide(&facts("review-secret"))), "deny wins");
        assert!(refused(&r.decide(&facts("summarize"))), "not allowed");
        assert!(refused(&r.decide(&RequestFacts {
            today: 2,
            ..facts("review-pr")
        })));
        assert!(ask(&r.decide(&RequestFacts {
            weekday: 6,
            ..facts("review-pr")
        })));
        assert!(ask(&rules(
            "requests:\n  approval: per-member\n  always_ask: [send]\n"
        )
        .decide(&facts("send-email"))));
        assert!(ask(&rules(
            "requests:\n  approval: per-member\n  always_ask: [send]\n"
        )
        .decide(&facts("SEND"))));
        assert_eq!(
            rules("requests:\n  approval: per-member\n  always_ask: [send]\n")
                .decide(&facts("sender-check")),
            Decision::Auto
        );
        let first = rules("requests:\n  approval: first-time\n");
        assert!(ask(&first.decide(&RequestFacts {
            first_time: true,
            ..facts("x")
        })));
        assert_eq!(first.decide(&facts("x")), Decision::Auto);
    }

    #[test]
    fn each_access_level_decides() {
        let q = facts("ask-me");
        let work = facts("review-pr");
        assert_eq!(AccessLevel::FullyAutomated.decide(&work), Decision::Auto);
        assert!(ask(&AccessLevel::AskFirst.decide(&work)));
        assert!(ask(&AccessLevel::AskFirst.decide(&q)));
        assert_eq!(AccessLevel::QuestionsOnly.decide(&q), Decision::Auto);
        assert!(refused(&AccessLevel::QuestionsOnly.decide(&work)));
        assert!(ask(&AccessLevel::SendMeInfo.decide(&q)));
        assert!(refused(&AccessLevel::SendMeInfo.decide(&work)));
        assert!(refused(&AccessLevel::None.decide(&q)));
        assert_eq!(AccessLevel::default(), AccessLevel::AskFirst);
    }

    #[test]
    fn the_stricter_wins_and_across_groups_the_strictest() {
        let per_member = rules("requests:\n  approval: per-member\n");
        let always = rules("requests:\n  approval: always\n");
        let work = facts("review-pr");
        // I can't loosen the group: fully automated still asks
        let d = always
            .decide(&work)
            .stricter(AccessLevel::FullyAutomated.decide(&work));
        assert!(ask(&d));
        // I can tighten it
        let d = per_member
            .decide(&work)
            .stricter(AccessLevel::None.decide(&work));
        assert!(refused(&d));
        let d = per_member
            .decide(&work)
            .stricter(AccessLevel::FullyAutomated.decide(&work));
        assert_eq!(d, Decision::Auto);
        let all = Decision::strictest([Decision::Auto, Decision::Ask("a".into()), Decision::Auto]);
        assert_eq!(all, Some(Decision::Ask("a".into())));
        let all = Decision::strictest([Decision::Ask("a".into()), Decision::Refuse("r".into())]);
        assert_eq!(all, Some(Decision::Refuse("r".into())));
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
