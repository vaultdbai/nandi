//! Typed domain errors. Transports turn them into strings at the edge.

use std::fmt;

use super::EntityKind;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// An identifier that breaks its value object's rules
    InvalidId {
        kind: &'static str,
        value: String,
        reason: &'static str,
    },
    /// A field that breaks an entity invariant
    Invalid {
        entity: String,
        field: &'static str,
        reason: String,
    },
    /// A file that isn't valid YAML / frontmatter, or has the wrong shape
    Parse {
        what: String,
        reason: String,
    },
    /// Two entities of a kind with the same ID
    Duplicate {
        kind: EntityKind,
        id: String,
    },
    /// A reference to an entity that doesn't exist
    DanglingReference {
        from: String,
        to: String,
    },
    /// A workflow step that depends on itself or a later step
    Cycle {
        workflow: String,
        step: String,
        reference: String,
    },
    NotFound {
        kind: EntityKind,
        id: String,
    },
    /// A rule spanning fields, as a full sentence
    Rule(String),
    /// Removing an entity others still reference
    InUse {
        kind: EntityKind,
        id: String,
        by: Vec<String>,
    },
}

impl DomainError {
    pub fn invalid(
        entity: impl Into<String>,
        field: &'static str,
        reason: impl Into<String>,
    ) -> Self {
        Self::Invalid {
            entity: entity.into(),
            field,
            reason: reason.into(),
        }
    }

    pub fn parse(what: impl Into<String>, reason: impl fmt::Display) -> Self {
        Self::Parse {
            what: what.into(),
            reason: reason.to_string(),
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId {
                kind,
                value,
                reason,
            } => write!(f, "Invalid {kind} id '{value}': {reason}"),
            Self::Invalid {
                entity,
                field,
                reason,
            } => write!(f, "{entity}: {field} {reason}"),
            Self::Parse { what, reason } => write!(f, "Invalid {what}: {reason}"),
            Self::Rule(message) => f.write_str(message),
            Self::Duplicate { kind, id } => write!(f, "Duplicate {kind} id '{id}'"),
            Self::DanglingReference { from, to } => write!(f, "{from} refers to missing {to}"),
            Self::Cycle {
                workflow,
                step,
                reference,
            } => write!(
                f,
                "Workflow '{workflow}': step '{step}' uses {{{{{reference}.output}}}}, but no earlier step is named '{reference}'"
            ),
            Self::NotFound { kind, id } => {
                let kind = kind.as_str();
                let mut chars = kind.chars();
                let kind: String = chars
                    .next()
                    .map(|c| c.to_ascii_uppercase())
                    .into_iter()
                    .chain(chars)
                    .collect();
                write!(f, "{kind} '{id}' not found")
            }
            Self::InUse { kind, id, by } => {
                write!(f, "The {kind} '{id}' is used by {}", by.join(", "))
            }
        }
    }
}

impl std::error::Error for DomainError {}

impl From<DomainError> for String {
    fn from(error: DomainError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_read_naturally() {
        let not_found = DomainError::NotFound {
            kind: EntityKind::Agent,
            id: "writer".into(),
        };
        assert_eq!(not_found.to_string(), "Agent 'writer' not found");
        let cycle = DomainError::Cycle {
            workflow: "w".into(),
            step: "a".into(),
            reference: "b".into(),
        };
        assert!(cycle.to_string().contains("{{b.output}}"));
    }
}
