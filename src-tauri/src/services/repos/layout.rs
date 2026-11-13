//! Where each entity's files live.
//!
//! The gold layout (`~/.nandi/repo/`, a git repo):
//!
//! ```text
//! .gitignore
//! .nandi/manifest.yaml
//! .nandi/settings.yaml
//! .nandi/connectors.yaml
//! .nandi/connectors/<id>/connector.md                     (connector packages)
//! .nandi/agents/<id>/skill.md
//! .nandi/agents/<id>/instructions.md
//! .nandi/agents/<id>/skills/<playbook>.md                (playbooks)
//! .nandi/agents/<id>/workflows/<workflow>.yaml
//! .nandi/workflows/<workflow>.yaml
//! .nandi/triggers/<trigger>.yaml
//! .nandi/capabilities/<id>.yaml                           (capability definitions)
//! .nandi/policies/<id>.md                                 (policies)
//! .nandi/groups/<group-id>.md                             (groups I'm in: signed policy copies)
//! .nandi/groups/access.yaml                               (my access level per group)
//! .nandi/decisions/<yyyy-mm>.md                           (every Inbox decision: who, what, why)
//! .nandi/installed.yaml                                   (marketplace installs)
//! .nandi/agents/marketplace/<publisher>/<id>/…            (installed agents)
//! .nandi/agents/marketplace/packs/<pack>/agents/<id>/…    (installed packs)
//! .nandi/agents/marketplace/packs/<pack>/workflows/<w>.yaml
//! ```
//!
//! Agent IDs stay flat and unique: `agent_locator` finds an agent's folder
//! wherever it lives.

use std::path::{Path, PathBuf};

use crate::services::domain::workflow::WorkflowScope;
use crate::services::domain::EntityKind;

pub const META_DIR: &str = ".nandi";
pub const SKILL_FILE: &str = "skill.md";
pub const INSTRUCTIONS_FILE: &str = "instructions.md";
/// Next to `skill.md`: the agent's playbooks, one `<name>.md` each
pub const PLAYBOOKS_DIR: &str = "skills";
/// Agent-maintained memory
#[allow(dead_code)]
pub const MEMORY_FILE: &str = "memory.md";
pub const SETTINGS_FILE: &str = "settings.yaml";
pub const CONNECTORS_FILE: &str = "connectors.yaml";
/// Under `.nandi/`: one folder per connector package, holding `connector.md`
pub const CONNECTOR_PACKAGES_DIR: &str = "connectors";
pub const CONNECTOR_FILE: &str = "connector.md";
pub const MANIFEST_FILE: &str = "manifest.yaml";
pub const INSTALLED_FILE: &str = "installed.yaml";
/// Under `.nandi/`: capability definitions
pub const CAPABILITIES_DIR: &str = "capabilities";
/// Under `.nandi/`: policies
pub const POLICIES_DIR: &str = "policies";
/// Under `.nandi/`: the groups this Nandi is in (protocol 7)
pub const GROUPS_DIR: &str = "groups";
/// Under `.nandi/`: the decisions made on the Inbox, one file per month
pub const DECISIONS_DIR: &str = "decisions";
/// Under the agents folder: vendored (read-only) marketplace installs. Not
/// usable as an agent ID.
pub const MARKETPLACE_DIR: &str = "marketplace";
/// Under `marketplace/`: whole packs. Not usable as a publisher handle.
pub const PACKS_DIR: &str = "packs";
/// `.nandi/overrides/<agent-id>/`: the user's own files for an installed
/// (read-only) agent, kept when the agent updates
pub const OVERRIDES_DIR: &str = "overrides";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLayout {
    /// The repo root (what the watcher watches and git stages within)
    pub root: PathBuf,
    /// `.nandi/`: manifest, settings, connectors
    pub meta_dir: PathBuf,
    pub agents_dir: PathBuf,
    pub workflows_dir: PathBuf,
    pub triggers_dir: PathBuf,
    /// Where deleted agent folders go (never inside the repo)
    pub trash_dir: PathBuf,
}

impl RepoLayout {
    pub fn new(root: impl Into<PathBuf>, trash_dir: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let meta_dir = root.join(META_DIR);
        Self {
            agents_dir: meta_dir.join("agents"),
            workflows_dir: meta_dir.join("workflows"),
            triggers_dir: meta_dir.join("triggers"),
            meta_dir,
            root,
            trash_dir: trash_dir.into(),
        }
    }

    pub fn agent_dir(&self, id: &str) -> PathBuf {
        self.agents_dir.join(id)
    }

    pub fn skill_path(&self, id: &str) -> PathBuf {
        self.agent_dir(id).join(SKILL_FILE)
    }

    pub fn instructions_path(&self, id: &str) -> PathBuf {
        self.agent_dir(id).join(INSTRUCTIONS_FILE)
    }

    /// The user's `instructions.md` for an installed agent: used in place
    /// of the shipped one, and never touched by an update
    pub fn instructions_override_path(&self, id: &str) -> PathBuf {
        self.meta_dir
            .join(OVERRIDES_DIR)
            .join(id)
            .join(INSTRUCTIONS_FILE)
    }

    pub fn workflows_dir_for(&self, scope: &WorkflowScope) -> PathBuf {
        match scope {
            WorkflowScope::Global => self.workflows_dir.clone(),
            WorkflowScope::Agent(agent) => self.agent_dir(agent.as_str()).join("workflows"),
        }
    }

    pub fn workflow_path(&self, id: &str, scope: &WorkflowScope) -> PathBuf {
        self.workflows_dir_for(scope).join(format!("{id}.yaml"))
    }

    pub fn trigger_path(&self, id: &str) -> PathBuf {
        self.triggers_dir.join(format!("{id}.yaml"))
    }

    pub fn settings_path(&self) -> PathBuf {
        self.meta_dir.join(SETTINGS_FILE)
    }

    pub fn connectors_path(&self) -> PathBuf {
        self.meta_dir.join(CONNECTORS_FILE)
    }

    /// `.nandi/connectors/<id>/`: a connector package's folder
    pub fn connector_package_dir(&self, id: &str) -> PathBuf {
        self.meta_dir.join(CONNECTOR_PACKAGES_DIR).join(id)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.meta_dir.join(MANIFEST_FILE)
    }

    /// `.nandi/installed.yaml`: every vendored marketplace source
    pub fn installed_path(&self) -> PathBuf {
        self.meta_dir.join(INSTALLED_FILE)
    }

    /// `.nandi/capabilities/`: one `<id>.yaml` per capability
    pub fn capabilities_dir(&self) -> PathBuf {
        self.meta_dir.join(CAPABILITIES_DIR)
    }

    pub fn capability_path(&self, id: &str) -> PathBuf {
        self.capabilities_dir().join(format!("{id}.yaml"))
    }

    /// `.nandi/policies/`: one `<id>.md` per policy
    pub fn policies_dir(&self) -> PathBuf {
        self.meta_dir.join(POLICIES_DIR)
    }

    pub fn policy_path(&self, id: &str) -> PathBuf {
        self.policies_dir().join(format!("{id}.md"))
    }

    /// `.nandi/groups/`: one `<group-id>.md` per group this Nandi is in,
    /// and `access.yaml`
    pub fn groups_dir(&self) -> PathBuf {
        self.meta_dir.join(GROUPS_DIR)
    }

    /// `.nandi/decisions/`: one `<yyyy-mm>.md` per month, appended to
    pub fn decisions_dir(&self) -> PathBuf {
        self.meta_dir.join(DECISIONS_DIR)
    }

    /// `.nandi/agents/marketplace/`
    pub fn marketplace_dir(&self) -> PathBuf {
        self.agents_dir.join(MARKETPLACE_DIR)
    }

    /// `.nandi/agents/marketplace/packs/<pack>/`
    pub fn pack_dir(&self, pack: &str) -> PathBuf {
        self.marketplace_dir().join(PACKS_DIR).join(pack)
    }

    /// `.nandi/agents/marketplace/<publisher>/<id>/`
    pub fn vendored_agent_dir(&self, publisher: &str, id: &str) -> PathBuf {
        self.marketplace_dir().join(publisher).join(id)
    }

    /// Whether `path` is inside the vendored (read-only) marketplace folder.
    pub fn is_vendored(&self, path: &Path) -> bool {
        path.starts_with(self.marketplace_dir())
    }

    /// The main file of an entity (what `sync_state.source_path` records).
    pub fn source_path(
        &self,
        kind: EntityKind,
        id: &str,
        scope: Option<&WorkflowScope>,
    ) -> PathBuf {
        match kind {
            EntityKind::Agent => self.skill_path(id),
            EntityKind::Workflow => self.workflow_path(id, scope.unwrap_or(&WorkflowScope::Global)),
            EntityKind::Trigger => self.trigger_path(id),
            EntityKind::Settings => self.settings_path(),
            EntityKind::Connectors => self.connectors_path(),
            EntityKind::Manifest => self.manifest_path(),
        }
    }

    /// `path` relative to the repo root, with `/` separators, if inside it.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        Some(
            relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::AgentId;

    #[test]
    fn paths_live_under_dot_nandi() {
        let layout = RepoLayout::new("/r", "/trash");
        assert_eq!(
            layout.skill_path("writer"),
            Path::new("/r/.nandi/agents/writer/skill.md")
        );
        let scoped = WorkflowScope::Agent(AgentId::parse("writer").unwrap());
        assert_eq!(
            layout.workflow_path("daily", &scoped),
            Path::new("/r/.nandi/agents/writer/workflows/daily.yaml")
        );
        assert_eq!(
            layout.trigger_path("t"),
            Path::new("/r/.nandi/triggers/t.yaml")
        );
        assert_eq!(layout.settings_path(), Path::new("/r/.nandi/settings.yaml"));
        assert_eq!(layout.decisions_dir(), Path::new("/r/.nandi/decisions"));
        assert_eq!(
            layout.relative(&layout.trigger_path("t")).as_deref(),
            Some(".nandi/triggers/t.yaml")
        );
        assert_eq!(layout.relative(Path::new("/elsewhere/x")), None);
    }
}
