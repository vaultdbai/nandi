//! Domain events: what changed in the gold copy, and who changed it. The
//! services layer publishes them on the event bus as `repo://…` topics.

use std::path::PathBuf;

use super::EntityKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeAction {
    Created,
    Updated,
    Deleted,
}

impl ChangeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Deleted => "deleted",
        }
    }
}

/// Where a change came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangedBy {
    /// A command from the UI or API
    User,
    /// The file watcher (an editor, a script)
    Watcher,
    /// A git pull
    Git,
}

impl ChangedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Watcher => "watcher",
            Self::Git => "git",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainEvent {
    pub kind: EntityKind,
    pub id: String,
    pub action: ChangeAction,
    pub source_path: PathBuf,
    pub changed_by: ChangedBy,
}

impl DomainEvent {
    /// The bus topic, e.g. `repo://agent-created`. Singletons only update.
    pub fn topic(&self) -> &'static str {
        use ChangeAction::{Created, Deleted, Updated};
        match (self.kind, self.action) {
            (EntityKind::Agent, Created) => "repo://agent-created",
            (EntityKind::Agent, Updated) => "repo://agent-updated",
            (EntityKind::Agent, Deleted) => "repo://agent-deleted",
            (EntityKind::Workflow, Created) => "repo://workflow-created",
            (EntityKind::Workflow, Updated) => "repo://workflow-updated",
            (EntityKind::Workflow, Deleted) => "repo://workflow-deleted",
            (EntityKind::Trigger, Created) => "repo://trigger-created",
            (EntityKind::Trigger, Updated) => "repo://trigger-updated",
            (EntityKind::Trigger, Deleted) => "repo://trigger-deleted",
            (EntityKind::Settings, _) => "repo://settings-updated",
            (EntityKind::Connectors, _) => "repo://connector-updated",
            (EntityKind::Manifest, _) => "repo://manifest-updated",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_follow_the_bus_naming() {
        let event = DomainEvent {
            kind: EntityKind::Agent,
            id: "writer".into(),
            action: ChangeAction::Created,
            source_path: PathBuf::from(".nandi/agents/writer/skill.md"),
            changed_by: ChangedBy::User,
        };
        assert_eq!(event.topic(), "repo://agent-created");
        let settings = DomainEvent {
            kind: EntityKind::Settings,
            action: ChangeAction::Created,
            ..event
        };
        assert_eq!(settings.topic(), "repo://settings-updated");
    }

    #[test]
    fn every_topic_is_a_valid_tauri_event_name() {
        use ChangeAction::{Created, Deleted, Updated};
        let kinds = [
            EntityKind::Agent,
            EntityKind::Workflow,
            EntityKind::Trigger,
            EntityKind::Settings,
            EntityKind::Connectors,
            EntityKind::Manifest,
        ];
        for kind in kinds {
            for action in [Created, Updated, Deleted] {
                let event = DomainEvent {
                    kind,
                    id: "x".into(),
                    action,
                    source_path: PathBuf::new(),
                    changed_by: ChangedBy::User,
                };
                let topic = event.topic();
                assert!(
                    crate::services::events::is_valid_event_name(topic),
                    "{topic}"
                );
            }
        }
    }
}
