//! `AgentRepo`, the aggregate root: the whole gold copy in memory.
//!
//! It enforces the invariants that span entities: IDs are unique per kind
//! (workflow IDs across global and agent-scoped folders too), and
//! references between entities resolve.
//!
//! References are enforced in two strengths:
//! - Commands (`check_references`, `check_removable`) are strict: you can't
//!   create a workflow for a missing agent or delete an agent a trigger
//!   targets.
//! - Files are lenient: a `git pull` may deliver a workflow before its
//!   agent, so a dangling reference in a loaded file is a warning
//!   (`problems()`), and the entity stays. Running it fails until the
//!   reference resolves.
//!
//! The aggregate is pure. `services::repos` loads it from disk
//! (`RepoStore::load`), reloads single entities, and rebuilds the projection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::trigger::Target;
use super::workflow::WorkflowScope;
use super::{
    Agent, AgentId, ChangeAction, ConnectorManifest, DomainError, DomainResult, EntityKind,
    Settings, Trigger, TriggerId, Workflow, WorkflowId,
};

/// The repo format this version reads and writes (`.nandi/manifest.yaml`).
pub const REPO_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoManifest {
    pub schema_version: u32,
    /// GitHub login of the repo's owner, if known
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

impl Default for RepoManifest {
    fn default() -> Self {
        Self {
            schema_version: REPO_SCHEMA_VERSION,
            owner: None,
        }
    }
}

impl RepoManifest {
    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        let manifest: Self =
            serde_yaml::from_str(content).map_err(|e| DomainError::parse("manifest.yaml", e))?;
        if manifest.schema_version == 0 || manifest.schema_version > REPO_SCHEMA_VERSION {
            return Err(DomainError::invalid(
                "manifest.yaml",
                "schema_version",
                format!(
                    "is {}; this version of Nandi reads up to {REPO_SCHEMA_VERSION}. Update Nandi.",
                    manifest.schema_version
                ),
            ));
        }
        Ok(manifest)
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_default()
    }
}

/// Any entity of the gold copy.
#[derive(Debug, Clone, PartialEq)]
pub enum Entity {
    /// Boxed: an agent is far larger than the other entities
    Agent(Box<Agent>),
    Workflow(Workflow),
    Trigger(Trigger),
    Settings(Settings),
    Connectors(ConnectorManifest),
    Manifest(RepoManifest),
}

impl From<Agent> for Entity {
    fn from(agent: Agent) -> Self {
        Self::Agent(Box::new(agent))
    }
}

impl Entity {
    pub fn kind(&self) -> EntityKind {
        match self {
            Self::Agent(_) => EntityKind::Agent,
            Self::Workflow(_) => EntityKind::Workflow,
            Self::Trigger(_) => EntityKind::Trigger,
            Self::Settings(_) => EntityKind::Settings,
            Self::Connectors(_) => EntityKind::Connectors,
            Self::Manifest(_) => EntityKind::Manifest,
        }
    }

    pub fn id(&self) -> String {
        match self {
            Self::Agent(agent) => agent.id().to_string(),
            Self::Workflow(workflow) => workflow.id().to_string(),
            Self::Trigger(trigger) => trigger.id().to_string(),
            other => other.kind().singleton_id().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The file couldn't be loaded; the entity is missing (or stale)
    Error,
    /// Loaded, but something about it won't work (e.g. a dangling reference)
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub kind: EntityKind,
    /// `None` if the file was too broken to tell
    pub id: Option<String>,
    pub source_path: Option<PathBuf>,
    pub error: DomainError,
    pub severity: Severity,
}

/// What the loader read from disk, for `AgentRepo::assemble`.
#[derive(Debug, Default)]
pub struct RepoSnapshot {
    pub manifest: Option<RepoManifest>,
    pub agents: Vec<Agent>,
    pub workflows: Vec<Workflow>,
    pub triggers: Vec<Trigger>,
    pub settings: Option<Settings>,
    pub connectors: Option<ConnectorManifest>,
    /// Files that failed to parse or validate
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone)]
pub struct AgentRepo {
    root: PathBuf,
    manifest: RepoManifest,
    agents: BTreeMap<AgentId, Agent>,
    workflows: BTreeMap<WorkflowId, Workflow>,
    triggers: BTreeMap<TriggerId, Trigger>,
    settings: Settings,
    connectors: ConnectorManifest,
    /// Files that failed to load, by (kind, id or path)
    load_problems: Vec<Problem>,
}

impl AgentRepo {
    pub fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            manifest: RepoManifest::default(),
            agents: BTreeMap::new(),
            workflows: BTreeMap::new(),
            triggers: BTreeMap::new(),
            settings: Settings::default(),
            connectors: ConnectorManifest::default(),
            load_problems: Vec::new(),
        }
    }

    /// Builds the aggregate from loaded files. Duplicates become problems
    /// (the first one wins), never a failure: a bad file must not hide the
    /// rest of the repo.
    pub fn assemble(root: impl Into<PathBuf>, snapshot: RepoSnapshot) -> Self {
        let mut repo = Self::empty(root);
        repo.manifest = snapshot.manifest.unwrap_or_default();
        repo.settings = snapshot.settings.unwrap_or_default();
        repo.connectors = snapshot.connectors.unwrap_or_default();
        repo.load_problems = snapshot.problems;
        let entities = snapshot
            .agents
            .into_iter()
            .map(Entity::from)
            .chain(snapshot.workflows.into_iter().map(Entity::Workflow))
            .chain(snapshot.triggers.into_iter().map(Entity::Trigger));
        for entity in entities {
            let (kind, id) = (entity.kind(), entity.id());
            let duplicate = match &entity {
                Entity::Agent(a) => repo.agents.contains_key(a.id()),
                Entity::Workflow(w) => repo.workflows.contains_key(w.id()),
                Entity::Trigger(t) => repo.triggers.contains_key(t.id()),
                _ => false,
            };
            if duplicate {
                repo.load_problems.push(Problem {
                    kind,
                    id: Some(id.clone()),
                    source_path: None,
                    error: DomainError::Duplicate { kind, id },
                    severity: Severity::Error,
                });
                continue;
            }
            repo.insert(entity);
        }
        repo.link_agent_workflows();
        repo
    }

    fn insert(&mut self, entity: Entity) {
        match entity {
            Entity::Agent(agent) => {
                self.agents.insert(agent.id().clone(), *agent);
            }
            Entity::Workflow(workflow) => {
                self.workflows.insert(workflow.id().clone(), workflow);
            }
            Entity::Trigger(trigger) => {
                self.triggers.insert(trigger.id().clone(), trigger);
            }
            Entity::Settings(settings) => self.settings = settings,
            Entity::Connectors(connectors) => self.connectors = connectors,
            Entity::Manifest(manifest) => self.manifest = manifest,
        }
    }

    /// Each agent lists its agent-scoped workflows.
    fn link_agent_workflows(&mut self) {
        let mut by_agent: BTreeMap<AgentId, Vec<WorkflowId>> = BTreeMap::new();
        for workflow in self.workflows.values() {
            if let WorkflowScope::Agent(agent) = workflow.scope() {
                by_agent
                    .entry(agent.clone())
                    .or_default()
                    .push(workflow.id().clone());
            }
        }
        for (id, agent) in &mut self.agents {
            agent.set_workflows(by_agent.remove(id).unwrap_or_default());
        }
    }

    /// Replaces (or adds) an entity loaded from its file. Returns what
    /// happened, or `None` if nothing changed. Clears earlier load problems
    /// for it.
    pub fn upsert(&mut self, entity: Entity) -> DomainResult<Option<ChangeAction>> {
        let (kind, id) = (entity.kind(), entity.id());
        let existing = self.get(kind, &id);
        if let (Entity::Workflow(new), Some(Entity::Workflow(old))) = (&entity, &existing) {
            if new.scope() != old.scope() {
                return Err(DomainError::Duplicate { kind, id });
            }
        }
        self.clear_problems(kind, &id);
        let action = match existing {
            Some(old) if old == entity => None,
            Some(_) => Some(ChangeAction::Updated),
            None if kind.singleton_id().is_some() => Some(ChangeAction::Updated),
            None => Some(ChangeAction::Created),
        };
        if action.is_some() {
            self.insert(entity);
            self.link_agent_workflows();
        }
        Ok(action)
    }

    /// Removes an entity whose file is gone. Singletons reset to defaults.
    pub fn remove(&mut self, kind: EntityKind, id: &str) -> Option<Entity> {
        self.clear_problems(kind, id);
        let removed = match kind {
            EntityKind::Agent => self.agents.remove(id).map(Entity::from),
            EntityKind::Workflow => self.workflows.remove(id).map(Entity::Workflow),
            EntityKind::Trigger => self.triggers.remove(id).map(Entity::Trigger),
            EntityKind::Settings => Some(Entity::Settings(std::mem::take(&mut self.settings))),
            EntityKind::Connectors => {
                Some(Entity::Connectors(std::mem::take(&mut self.connectors)))
            }
            EntityKind::Manifest => Some(Entity::Manifest(std::mem::take(&mut self.manifest))),
        };
        self.link_agent_workflows();
        removed
    }

    /// Records that an entity's file is invalid. The last good version (if
    /// any) stays in the aggregate.
    pub fn record_problem(&mut self, problem: Problem) {
        if let Some(id) = &problem.id {
            self.clear_problems(problem.kind, id);
        }
        self.load_problems.push(problem);
    }

    /// Whether the entity's file failed to load last time.
    pub fn has_load_problem(&self, kind: EntityKind, id: &str) -> bool {
        self.load_problems
            .iter()
            .any(|p| p.kind == kind && p.id.as_deref() == Some(id))
    }

    fn clear_problems(&mut self, kind: EntityKind, id: &str) {
        self.load_problems
            .retain(|p| !(p.kind == kind && p.id.as_deref() == Some(id)));
    }

    pub fn get(&self, kind: EntityKind, id: &str) -> Option<Entity> {
        match kind {
            EntityKind::Agent => self.agents.get(id).cloned().map(Entity::from),
            EntityKind::Workflow => self.workflows.get(id).cloned().map(Entity::Workflow),
            EntityKind::Trigger => self.triggers.get(id).cloned().map(Entity::Trigger),
            EntityKind::Settings => Some(Entity::Settings(self.settings.clone())),
            EntityKind::Connectors => Some(Entity::Connectors(self.connectors.clone())),
            EntityKind::Manifest => Some(Entity::Manifest(self.manifest.clone())),
        }
    }

    // -- Invariants for commands --------------------------------------------

    /// Strict reference check for an entity about to be saved by a command.
    pub fn check_references(&self, entity: &Entity) -> DomainResult<()> {
        match entity {
            Entity::Workflow(workflow) => {
                if let WorkflowScope::Agent(agent) = workflow.scope() {
                    self.require_agent(agent, || format!("Workflow '{}'", workflow.id()))?;
                }
                if let Some(Entity::Workflow(existing)) =
                    self.get(EntityKind::Workflow, workflow.id().as_str())
                {
                    if existing.scope() != workflow.scope() {
                        return Err(DomainError::Duplicate {
                            kind: EntityKind::Workflow,
                            id: workflow.id().to_string(),
                        });
                    }
                }
                workflow.validate_step_graph(|agent| self.agents.contains_key(agent))
            }
            Entity::Trigger(trigger) => match trigger.target() {
                Target::Agent(agent) => {
                    self.require_agent(agent, || format!("Trigger '{}'", trigger.id()))
                }
                Target::Workflow(workflow) if !self.workflows.contains_key(workflow) => {
                    Err(DomainError::DanglingReference {
                        from: format!("Trigger '{}'", trigger.id()),
                        to: format!("workflow '{workflow}'"),
                    })
                }
                Target::Workflow(_) => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn require_agent(&self, agent: &AgentId, from: impl Fn() -> String) -> DomainResult<()> {
        if self.agents.contains_key(agent) {
            Ok(())
        } else {
            Err(DomainError::DanglingReference {
                from: from(),
                to: format!("agent '{agent}'"),
            })
        }
    }

    /// What still refers to an entity, e.g. `["workflow 'daily'"]`.
    pub fn references_to(&self, kind: EntityKind, id: &str) -> Vec<String> {
        let mut by = Vec::new();
        if kind == EntityKind::Agent {
            for workflow in self.workflows.values() {
                let scoped =
                    matches!(workflow.scope(), WorkflowScope::Agent(a) if a.as_str() == id);
                if !scoped && workflow.agents().any(|a| a.as_str() == id) {
                    by.push(format!("workflow '{}'", workflow.id()));
                }
            }
        }
        for trigger in self.triggers.values() {
            let hit = match (kind, trigger.target()) {
                (EntityKind::Agent, Target::Agent(a)) => a.as_str() == id,
                (EntityKind::Workflow, Target::Workflow(w)) => w.as_str() == id,
                _ => false,
            };
            if hit {
                by.push(format!("trigger '{}'", trigger.id()));
            }
        }
        by
    }

    /// Refuses to remove an entity others still reference.
    pub fn check_removable(&self, kind: EntityKind, id: &str) -> DomainResult<()> {
        if self.get(kind, id).is_none() {
            return Err(DomainError::NotFound {
                kind,
                id: id.to_string(),
            });
        }
        let by = self.references_to(kind, id);
        if by.is_empty() {
            Ok(())
        } else {
            Err(DomainError::InUse {
                kind,
                id: id.to_string(),
                by,
            })
        }
    }

    /// Load problems plus dangling references (warnings).
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = self.load_problems.clone();
        for workflow in self.workflows.values() {
            if let Err(error) = workflow.validate_step_graph(|a| self.agents.contains_key(a)) {
                problems.push(Problem {
                    kind: EntityKind::Workflow,
                    id: Some(workflow.id().to_string()),
                    source_path: None,
                    error,
                    severity: Severity::Warning,
                });
            }
        }
        for trigger in self.triggers.values() {
            let missing = match trigger.target() {
                Target::Agent(a) => (!self.agents.contains_key(a)).then(|| format!("agent '{a}'")),
                Target::Workflow(w) => {
                    (!self.workflows.contains_key(w)).then(|| format!("workflow '{w}'"))
                }
            };
            if let Some(to) = missing {
                problems.push(Problem {
                    kind: EntityKind::Trigger,
                    id: Some(trigger.id().to_string()),
                    source_path: None,
                    error: DomainError::DanglingReference {
                        from: format!("Trigger '{}'", trigger.id()),
                        to,
                    },
                    severity: Severity::Warning,
                });
            }
        }
        problems
    }

    // -- Accessors ----------------------------------------------------------

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &RepoManifest {
        &self.manifest
    }

    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.get(id)
    }

    pub fn agents(&self) -> impl Iterator<Item = &Agent> {
        self.agents.values()
    }

    pub fn workflow(&self, id: &str) -> Option<&Workflow> {
        self.workflows.get(id)
    }

    pub fn workflows(&self) -> impl Iterator<Item = &Workflow> {
        self.workflows.values()
    }

    pub fn trigger(&self, id: &str) -> Option<&Trigger> {
        self.triggers.get(id)
    }

    pub fn triggers(&self) -> impl Iterator<Item = &Trigger> {
        self.triggers.values()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn connectors(&self) -> &ConnectorManifest {
        &self.connectors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> Agent {
        Agent::from_skill_md(AgentId::parse(id).unwrap(), "---\nname: A\n---\n").unwrap()
    }

    fn workflow(id: &str, agent: &str) -> Workflow {
        Workflow::from_yaml(&format!(
            "id: {id}\nname: W\nsteps:\n  - {{id: s, agent: {agent}, input: hi}}\n"
        ))
        .unwrap()
    }

    fn trigger(id: &str, agent: &str) -> Trigger {
        Trigger::from_yaml(&format!(
            "id: {id}\nname: T\nsource: {{type: webhook}}\ntarget: {{type: agent, id: {agent}}}\n"
        ))
        .unwrap()
    }

    fn repo() -> AgentRepo {
        AgentRepo::assemble(
            "/repo",
            RepoSnapshot {
                agents: vec![agent("writer"), agent("writer"), agent("research")],
                workflows: vec![
                    workflow("daily", "writer"),
                    workflow("orphan", "ghost"),
                    workflow("mine", "research")
                        .in_scope(WorkflowScope::Agent(AgentId::parse("research").unwrap())),
                ],
                triggers: vec![trigger("hook", "writer")],
                ..Default::default()
            },
        )
    }

    #[test]
    fn assembles_with_duplicates_and_dangling_refs_as_problems() {
        let repo = repo();
        assert_eq!(repo.agents().count(), 2);
        let problems = repo.problems();
        assert!(problems
            .iter()
            .any(|p| matches!(p.error, DomainError::Duplicate { .. })
                && p.severity == Severity::Error));
        assert!(problems
            .iter()
            .any(|p| p.id.as_deref() == Some("orphan") && p.severity == Severity::Warning));
        // The dangling workflow is still there
        assert!(repo.workflow("orphan").is_some());
        assert_eq!(
            repo.agent("research").unwrap().workflows()[0].as_str(),
            "mine"
        );
    }

    #[test]
    fn commands_are_checked_strictly() {
        let repo = repo();
        let ghost = Entity::Workflow(workflow("new", "ghost"));
        assert!(matches!(
            repo.check_references(&ghost),
            Err(DomainError::DanglingReference { .. })
        ));
        assert!(repo
            .check_references(&Entity::Workflow(workflow("new", "writer")))
            .is_ok());
        assert!(repo
            .check_references(&Entity::Trigger(trigger("t", "ghost")))
            .is_err());

        let in_use = repo
            .check_removable(EntityKind::Agent, "writer")
            .unwrap_err();
        let message = in_use.to_string();
        assert!(
            message.contains("workflow 'daily'") && message.contains("trigger 'hook'"),
            "{message}"
        );
        // An agent's own scoped workflows go with it
        assert!(repo.check_removable(EntityKind::Agent, "research").is_ok());
        assert!(matches!(
            repo.check_removable(EntityKind::Agent, "nobody"),
            Err(DomainError::NotFound { .. })
        ));
    }

    #[test]
    fn upsert_reports_what_changed() {
        let mut repo = repo();
        assert_eq!(
            repo.upsert(Entity::from(agent("new"))).unwrap(),
            Some(ChangeAction::Created)
        );
        assert_eq!(repo.upsert(Entity::from(agent("new"))).unwrap(), None);
        let renamed =
            Agent::from_skill_md(AgentId::parse("new").unwrap(), "---\nname: B\n---\n").unwrap();
        assert_eq!(
            repo.upsert(Entity::from(renamed)).unwrap(),
            Some(ChangeAction::Updated)
        );
        // A workflow ID can't exist in two scopes
        assert!(repo
            .upsert(Entity::Workflow(workflow("mine", "writer")))
            .is_err());
        assert!(repo.remove(EntityKind::Agent, "new").is_some());
        assert!(repo.agent("new").is_none());
    }

    #[test]
    fn invalid_reloads_keep_the_last_good_version() {
        let mut repo = repo();
        repo.record_problem(Problem {
            kind: EntityKind::Agent,
            id: Some("writer".into()),
            source_path: Some("/repo/.nandi/agents/writer/skill.md".into()),
            error: DomainError::parse("skill.md", "bad"),
            severity: Severity::Error,
        });
        assert!(repo.agent("writer").is_some());
        assert!(repo
            .problems()
            .iter()
            .any(|p| p.id.as_deref() == Some("writer")));
        repo.upsert(Entity::from(agent("writer"))).unwrap();
        assert!(!repo
            .problems()
            .iter()
            .any(|p| p.kind == EntityKind::Agent && p.id.as_deref() == Some("writer")));
    }

    #[test]
    fn manifests_from_newer_versions_are_rejected() {
        assert!(RepoManifest::from_yaml("schema_version: 1\nowner: octocat").is_ok());
        let error = RepoManifest::from_yaml("schema_version: 99").unwrap_err();
        assert!(error.to_string().contains("Update Nandi"));
    }
}
