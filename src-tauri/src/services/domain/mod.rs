//! The domain model of the gold copy: the definitional data in the user's
//! git repo (agents, workflows, triggers, portable settings, connector
//! declarations). See `docs/developer/domain-model.md`.
//!
//! This layer is pure. It parses, validates, and renders file contents and
//! enforces invariants, but never touches the filesystem, SQLite, git, the
//! network, or Tauri. Allowed dependencies are std, serde (+ yaml/json), and
//! pure-computation crates (`croner`, `glob`, `chrono`). `tests::domain_has_no_io_deps`
//! enforces this.
//!
//! - Value objects (`AgentId`, `ModelRef`, `CronSpec`, …) are immutable and
//!   validated at construction: holding one means it is valid.
//! - Entities (`Agent`, `Workflow`, `Trigger`) have identity and behavior.
//! - `AgentRepo` is the aggregate root: the whole gold copy, enforcing
//!   cross-entity invariants (unique IDs, valid references).
//!
//! I/O lives in `services::repos` (the only code that writes the files) and
//! `services::projection` (the SQLite view).

pub mod agent;
pub mod agent_card;
pub mod calendar;
pub mod capability;
pub mod connector;
pub mod error;
pub mod events;
pub mod ids;
pub mod installed;
pub mod policy;
pub mod repo;
pub mod settings;
pub mod trigger;
pub mod workflow;

pub use agent::{Agent, AgentRuntimeKind, ModelRef};
pub use agent_card::AgentCard;
pub use capability::CapabilitySet;
pub use connector::ConnectorManifest;
pub use error::{DomainError, DomainResult};
pub use events::{ChangeAction, ChangedBy, DomainEvent};
pub use ids::{
    AgentId, ConnectorId, PackId, PilotRef, PublisherHandle, StepId, TriggerId, WorkflowId,
};
pub use repo::{AgentRepo, Entity, Problem};
pub use settings::Settings;
pub use trigger::Trigger;
pub use workflow::Workflow;

/// The kinds of entity in the gold copy. Each maps to files (see
/// `services::repos::RepoLayout`) and to projection rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityKind {
    Agent,
    Workflow,
    Trigger,
    Settings,
    Connectors,
    Manifest,
}

impl EntityKind {
    pub const ALL: [EntityKind; 6] = [
        Self::Agent,
        Self::Workflow,
        Self::Trigger,
        Self::Settings,
        Self::Connectors,
        Self::Manifest,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Workflow => "workflow",
            Self::Trigger => "trigger",
            Self::Settings => "settings",
            Self::Connectors => "connectors",
            Self::Manifest => "manifest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == s)
    }

    /// Singletons have one instance per repo, keyed by this ID.
    pub fn singleton_id(self) -> Option<&'static str> {
        match self {
            Self::Settings => Some("settings"),
            Self::Connectors => Some("connectors"),
            Self::Manifest => Some("manifest"),
            _ => None,
        }
    }
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Rejects text fields over `max` characters.
pub(crate) fn check_len(
    entity: &str,
    field: &'static str,
    value: &str,
    max: usize,
) -> DomainResult<()> {
    if value.chars().count() > max {
        return Err(DomainError::invalid(
            entity,
            field,
            format!("is longer than {max} characters"),
        ));
    }
    Ok(())
}

/// Why a file with [`has_conflict_markers`] is refused
pub(crate) const CONFLICT_MARKERS: &str =
    "has unresolved merge conflict markers (a pull left a merge in progress): resolve them first";

/// A pull's merge left `<<<<<<<` … `>>>>>>>` in the file. A lone
/// `=======` doesn't count: it's also a Markdown heading underline.
pub(crate) fn has_conflict_markers(text: &str) -> bool {
    let mut lines = text.lines();
    lines.any(|line| line.starts_with("<<<<<<< ")) && lines.any(|line| line.starts_with(">>>>>>> "))
}

#[cfg(test)]
mod tests {
    /// The domain must stay free of I/O and infrastructure crates.
    #[test]
    fn domain_has_no_io_deps() {
        let sources = [
            ("mod.rs", include_str!("mod.rs")),
            ("agent.rs", include_str!("agent.rs")),
            ("agent_card.rs", include_str!("agent_card.rs")),
            ("calendar.rs", include_str!("calendar.rs")),
            ("capability.rs", include_str!("capability.rs")),
            ("connector.rs", include_str!("connector.rs")),
            ("error.rs", include_str!("error.rs")),
            ("events.rs", include_str!("events.rs")),
            ("ids.rs", include_str!("ids.rs")),
            ("installed.rs", include_str!("installed.rs")),
            ("repo.rs", include_str!("repo.rs")),
            ("settings.rs", include_str!("settings.rs")),
            ("trigger.rs", include_str!("trigger.rs")),
            ("workflow.rs", include_str!("workflow.rs")),
        ];
        let forbidden = [
            "rusqlite",
            "r2d2",
            "reqwest",
            "tauri",
            "bollard",
            "git2",
            "tokio",
            "notify",
            "keyring",
            "std::fs",
            "crate::db",
            "crate::services::",
            "crate::llm",
            "axum",
        ];
        for (file, source) in sources {
            // Only check code above the tests module (which may name them)
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for name in forbidden {
                    let used = line.contains(&format!("use {name}"))
                        || line.contains(&format!("{name}::"));
                    assert!(!used, "domain/{file} depends on {name}: {line}");
                }
            }
        }
    }
}
