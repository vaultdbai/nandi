//! Capability version bumps (Track B for edit previews, Track C for
//! saves). What changed decides the smallest legal bump:
//!
//! | Change                                                     | Bump  |
//! | ---------------------------------------------------------- | ----- |
//! | input/output schemas, required tools, visibility           | major |
//! | other policy (SLA, cost, approval, limits), routing, optional tools | minor |
//! | name, description, tags, owner, body                       | patch |
//!
//! A version may go up by more than needed, never by less, and never down.
//! The ID never changes: that's a new capability.

use std::cmp::Ordering;
use std::fmt;

use crate::services::capability::definition::CapabilityDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    None,
    Patch,
    Minor,
    Major,
}

impl Bump {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

impl fmt::Display for Bump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The smallest bump the change from `before` to `after` needs
pub fn required_bump(before: &CapabilityDefinition, after: &CapabilityDefinition) -> Bump {
    let (b, a) = (before, after);
    if b.schemas != a.schemas
        || b.required_tools != a.required_tools
        || b.policy.visibility != a.policy.visibility
    {
        return Bump::Major;
    }
    if b.policy != a.policy || b.routing != a.routing || b.optional_tools != a.optional_tools {
        return Bump::Minor;
    }
    if b.name != a.name
        || b.description != a.description
        || b.tags != a.tags
        || b.owner != a.owner
        || b.body.trim() != a.body.trim()
    {
        return Bump::Patch;
    }
    Bump::None
}

/// How `after` moved from `before` (both semver)
pub fn actual_bump(before: &str, after: &str) -> Result<Bump, String> {
    let parse = |v: &str| {
        semver::Version::parse(v.trim()).map_err(|_| format!("'{v}' isn't a semver version"))
    };
    let (b, a) = (parse(before)?, parse(after)?);
    match a.cmp(&b) {
        Ordering::Less => Err(format!("The version can't go down ({before} → {after})")),
        Ordering::Equal => Ok(Bump::None),
        Ordering::Greater if a.major > b.major => Ok(Bump::Major),
        Ordering::Greater if a.minor > b.minor => Ok(Bump::Minor),
        Ordering::Greater => Ok(Bump::Patch),
    }
}

/// What a check found
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BumpCheck {
    pub required: Bump,
    pub actual: Bump,
}

impl BumpCheck {
    /// The version is unchanged although the change needs a bump
    pub fn missing(&self) -> bool {
        self.actual == Bump::None && self.required != Bump::None
    }
}

/// Refuses an ID change, a version going down, and a bump smaller than
/// the change needs. An unchanged version is allowed here (`missing()`):
/// the caller bumps it or refuses.
pub fn check(
    before: &CapabilityDefinition,
    after: &CapabilityDefinition,
) -> Result<BumpCheck, String> {
    if before.id != after.id {
        return Err(format!(
            "A capability's ID can't change ('{}' → '{}'); create a new capability instead",
            before.id, after.id
        ));
    }
    let required = required_bump(before, after);
    let actual = actual_bump(&before.version, &after.version)?;
    if actual != Bump::None && actual < required {
        return Err(format!(
            "This change needs a {required} version bump, but {} → {} is a {actual} bump",
            before.version, after.version
        ));
    }
    Ok(BumpCheck { required, actual })
}

/// `version` bumped by `bump`
pub fn bumped(version: &str, bump: Bump) -> Result<String, String> {
    let mut v = semver::Version::parse(version.trim())
        .map_err(|_| format!("'{version}' isn't a semver version"))?;
    match bump {
        Bump::None => {}
        Bump::Patch => v.patch += 1,
        Bump::Minor => {
            v.minor += 1;
            v.patch = 0;
        }
        Bump::Major => {
            v.major += 1;
            v.minor = 0;
            v.patch = 0;
        }
    }
    v.pre = semver::Prerelease::EMPTY;
    Ok(v.to_string())
}

/// `text` with its frontmatter's `version:` line set to `version`, or
/// `None` when there's no such line
pub fn set_version_line(text: &str, version: &str) -> Option<String> {
    let mut found = false;
    let replaced: Vec<String> = text
        .split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if !found && trimmed.starts_with("version:") {
                found = true;
                let ending = &line[trimmed.len()..];
                format!("version: {version}{ending}")
            } else {
                line.to_string()
            }
        })
        .collect();
    found.then(|| replaced.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::capability::definition::{self, Visibility};

    const CAP: &str = "---\nid: code-review.python\nname: Python review\ndescription: Reviews Python\nversion: 1.2.3\nowner:\n  kind: user\n  id: ada\n  display_name: Ada\npolicy:\n  sla_seconds: 300\n---\n\nReview it.\n";

    fn cap() -> CapabilityDefinition {
        definition::parse(CAP).unwrap()
    }

    #[test]
    fn what_changed_decides_the_bump() {
        let before = cap();
        let mut after = cap();
        assert_eq!(required_bump(&before, &after), Bump::None);
        after.description = "Reviews Python code".into();
        assert_eq!(required_bump(&before, &after), Bump::Patch);
        after.policy.sla_seconds = 60;
        assert_eq!(required_bump(&before, &after), Bump::Minor);
        after.policy.visibility = Visibility::Network;
        assert_eq!(required_bump(&before, &after), Bump::Major);
        let mut schema = cap();
        schema.schemas.input = Some(serde_json::json!({"type": "object"}));
        assert_eq!(required_bump(&before, &schema), Bump::Major);
        let mut tools = cap();
        tools.required_tools = vec!["git".into()];
        assert_eq!(required_bump(&before, &tools), Bump::Major);
    }

    #[test]
    fn illegal_changes_are_refused() {
        let before = cap();
        // Too small: an SLA change is minor
        let mut after = cap();
        after.policy.sla_seconds = 60;
        after.version = "1.2.4".into();
        let error = check(&before, &after).unwrap_err();
        assert!(error.contains("needs a minor version bump"), "{error}");
        // Down
        after.version = "1.0.0".into();
        assert!(check(&before, &after)
            .unwrap_err()
            .contains("can't go down"));
        // Not semver
        after.version = "two".into();
        assert!(check(&before, &after)
            .unwrap_err()
            .contains("isn't a semver"));
        // A new ID
        let mut renamed = cap();
        renamed.id = "code-review.rust".into();
        assert!(check(&before, &renamed)
            .unwrap_err()
            .contains("ID can't change"));
    }

    #[test]
    fn legal_changes_pass_and_a_missing_bump_is_reported() {
        let before = cap();
        let mut after = cap();
        after.policy.sla_seconds = 60;
        let missing = check(&before, &after).unwrap();
        assert!(missing.missing());
        assert_eq!(missing.required, Bump::Minor);
        // More than needed is fine
        after.version = "2.0.0".into();
        assert_eq!(
            check(&before, &after).unwrap(),
            BumpCheck {
                required: Bump::Minor,
                actual: Bump::Major
            }
        );
        after.version = "1.3.0".into();
        assert!(!check(&before, &after).unwrap().missing());
    }

    #[test]
    fn versions_bump_and_the_line_is_rewritten_in_place() {
        assert_eq!(bumped("1.2.3", Bump::Patch).unwrap(), "1.2.4");
        assert_eq!(bumped("1.2.3", Bump::Minor).unwrap(), "1.3.0");
        assert_eq!(bumped("1.2.3", Bump::Major).unwrap(), "2.0.0");
        let text = set_version_line(CAP, "1.3.0").unwrap();
        assert!(text.contains("\nversion: 1.3.0\n"));
        assert_eq!(text.len(), CAP.len());
        assert_eq!(set_version_line("name: x\n", "1.0.0"), None);
        let crlf = set_version_line("version: 1.0.0\r\nname: x\r\n", "1.0.1").unwrap();
        assert_eq!(crlf, "version: 1.0.1\r\nname: x\r\n");
    }
}
