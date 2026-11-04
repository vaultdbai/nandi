//! Identifier value objects. Holding one means it passed validation, so
//! IDs that become folder and file names can't escape the repo.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{DomainError, DomainResult};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $kind:literal, $rule:path) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> DomainResult<Self> {
                let value = value.into();
                match $rule(&value) {
                    Ok(()) => Ok(Self(value)),
                    Err(reason) => Err(DomainError::InvalidId {
                        kind: $kind,
                        value,
                        reason,
                    }),
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = DomainError;
            fn try_from(value: String) -> DomainResult<Self> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

pub const MAX_AGENT_ID_LEN: usize = 64;
pub const MAX_NAME_ID_LEN: usize = 128;

/// Agent IDs: lowercase ASCII letters, digits, and hyphens, 1–64 long,
/// starting with a letter or digit.
fn agent_rule(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > MAX_AGENT_ID_LEN {
        return Err("must be 1-64 characters");
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("use only lowercase letters, digits, and hyphens");
    }
    if value.starts_with('-') {
        return Err("must start with a letter or digit");
    }
    Ok(())
}

/// Workflow, step, trigger, connector, and pilot IDs: plain file names
/// (letters, digits, `-`, `_`, `.`; no leading dot), up to 128 long.
pub(super) fn name_rule(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > MAX_NAME_ID_LEN {
        return Err("must be 1-128 characters");
    }
    if value.starts_with('.') {
        return Err("must not start with '.'");
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("use only letters, digits, '-', '_', and '.'");
    }
    Ok(())
}

/// Publisher handles: GitHub logins, lowercased (letters, digits, single
/// hyphens, 1-39 long). `packs` is reserved: it's the packs folder.
fn publisher_rule(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 39 {
        return Err("must be 1-39 characters");
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("use only lowercase letters, digits, and hyphens");
    }
    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err("hyphens must be single and between letters or digits");
    }
    if value == "packs" {
        return Err("'packs' is reserved");
    }
    Ok(())
}

id_type!(
    /// An agent's ID: its folder name under `.nandi/agents/`.
    AgentId,
    "agent",
    agent_rule
);
id_type!(
    /// A marketplace pack: its folder under `.nandi/agents/marketplace/packs/`.
    PackId,
    "pack",
    agent_rule
);
id_type!(
    /// A marketplace publisher (a GitHub login, lowercased): its folder
    /// under `.nandi/agents/marketplace/`.
    PublisherHandle,
    "publisher",
    publisher_rule
);
id_type!(
    /// A workflow's ID: its file name under a `workflows/` folder.
    WorkflowId,
    "workflow",
    name_rule
);
id_type!(
    /// A step's ID within its workflow, used in `{{step.output}}`.
    StepId,
    "step",
    name_rule
);
id_type!(
    /// A trigger's ID: its file name under `.nandi/triggers/`.
    TriggerId,
    "trigger",
    name_rule
);
id_type!(
    /// A connector declared in `.nandi/connectors.yaml`.
    ConnectorId,
    "connector",
    name_rule
);
id_type!(
    /// A pilot an agent may use. Declared only; no runtime consumer yet.
    PilotRef,
    "pilot",
    name_rule
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_ids_are_strict() {
        for ok in ["writer", "code-reviewer-2", "a", &"x".repeat(64)] {
            assert!(AgentId::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-rf",
            "Writer",
            "code_reviewer",
            "v1.2",
            "a/b",
            "a\\b",
            "..",
            "a b",
            "C:",
            &"x".repeat(65),
        ] {
            assert!(AgentId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn name_ids_reject_paths() {
        assert!(WorkflowId::parse("daily_report.v2").is_ok());
        for bad in ["", ".hidden", "a/b", "..", "a b"] {
            assert!(WorkflowId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn deserializes_through_validation() {
        let id: AgentId = serde_yaml::from_str("writer").unwrap();
        assert_eq!(id.as_str(), "writer");
        assert!(serde_yaml::from_str::<AgentId>("Bad_Id").is_err());
        let err = AgentId::parse("Bad").unwrap_err().to_string();
        assert!(err.starts_with("Invalid agent id 'Bad'"), "{err}");
    }
}
