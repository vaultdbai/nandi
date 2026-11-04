//! Workflows: `<workflows_dir>/<id>.yaml` (global) and
//! `<agents_dir>/<agent>/workflows/<id>.yaml` (agent-scoped). IDs are unique
//! across both.
//!
//! A workflow created from YAML text keeps that text (comments, ordering):
//! `save_source` writes it verbatim once it's validated. `save` renders the
//! entity.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::agent_locator;
use super::fs_atomic::{file_times, remove_if_exists, write_if_changed};
use super::git::GitIndex;
use super::layout::MARKETPLACE_DIR;
use super::{
    file_stem, read_text, yaml_files, Loaded, RepoError, RepoLayout, RepoResult, Repository,
};
use crate::services::domain::workflow::WorkflowScope;
use crate::services::domain::{AgentId, EntityKind, Workflow, WorkflowId};

#[derive(Clone)]
pub struct WorkflowRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

/// A workflow with the file it was read from.
#[derive(Debug, Clone)]
pub struct WorkflowSource {
    pub workflow: Workflow,
    pub source: String,
    pub path: PathBuf,
}

impl WorkflowRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    #[cfg(test)]
    pub fn global_dir(&self) -> &Path {
        &self.layout.workflows_dir
    }

    /// Every folder holding workflows, with its scope: the global folder,
    /// each agent's `workflows/` (local, then vendored), and each installed
    /// pack's `workflows/` (global scope, read-only).
    fn folders(&self) -> Vec<(PathBuf, WorkflowScope)> {
        let mut folders = vec![(self.layout.workflows_dir.clone(), WorkflowScope::Global)];
        let mut seen = std::collections::HashSet::new();
        // Local folders count even without a skill.md: the aggregate reports
        // the missing agent
        let local = fs::read_dir(&self.layout.agents_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != MARKETPLACE_DIR)
            .map(|name| (self.layout.agent_dir(&name), name));
        let mut local: Vec<(PathBuf, String)> = local.collect();
        local.sort();
        let vendored = agent_locator::vendored(&self.layout)
            .into_iter()
            .map(|place| (place.dir, place.id));
        for (agent_dir, name) in local.into_iter().chain(vendored) {
            let Ok(id) = AgentId::parse(&name) else {
                continue;
            };
            let dir = agent_dir.join("workflows");
            if dir.is_dir() && seen.insert(name) {
                folders.push((dir, WorkflowScope::Agent(id)));
            }
        }
        folders.extend(
            agent_locator::pack_workflow_dirs(&self.layout)
                .into_iter()
                .map(|(dir, _)| (dir, WorkflowScope::Global)),
        );
        folders
    }

    /// Fails if the workflow (or the agent it belongs to) is vendored.
    fn ensure_writable(&self, id: &WorkflowId, scope: &WorkflowScope) -> RepoResult<()> {
        let vendored_file = self
            .locate(id)
            .is_some_and(|(path, _)| self.layout.is_vendored(&path));
        let vendored_agent = match scope {
            WorkflowScope::Agent(agent) => agent_locator::locate(&self.layout, agent.as_str())
                .is_some_and(|place| place.origin.is_vendored()),
            WorkflowScope::Global => false,
        };
        if vendored_file || vendored_agent {
            return Err(RepoError::ReadOnly {
                kind: EntityKind::Workflow,
                id: id.to_string(),
            });
        }
        Ok(())
    }

    /// Where a workflow in `scope` is written.
    fn path_for(&self, id: &WorkflowId, scope: &WorkflowScope) -> PathBuf {
        match scope {
            WorkflowScope::Global => self.layout.workflow_path(id.as_str(), scope),
            WorkflowScope::Agent(agent) => agent_locator::agent_dir(&self.layout, agent.as_str())
                .join("workflows")
                .join(format!("{id}.yaml")),
        }
    }

    /// The IDs (file names) of every workflow file, valid or not.
    pub fn file_ids(&self) -> Vec<String> {
        self.folders()
            .into_iter()
            .flat_map(|(dir, _)| yaml_files(&dir))
            .map(|path| file_stem(&path))
            .collect()
    }

    /// Where workflow `id` lives, if anywhere.
    pub fn locate(&self, id: &WorkflowId) -> Option<(PathBuf, WorkflowScope)> {
        self.folders().into_iter().find_map(|(dir, scope)| {
            let path = dir.join(format!("{id}.yaml"));
            path.is_file().then_some((path, scope))
        })
    }

    fn read(&self, path: &Path, scope: WorkflowScope) -> RepoResult<WorkflowSource> {
        let stem = file_stem(path);
        let source = read_text(path, EntityKind::Workflow, &stem)?;
        let workflow = Workflow::from_yaml(&source)?;
        if workflow.id().as_str() != stem {
            return Err(RepoError::IdMismatch {
                path: path.to_path_buf(),
                declared: workflow.id().to_string(),
            });
        }
        let (created, updated) = file_times(path);
        Ok(WorkflowSource {
            workflow: workflow.in_scope(scope).with_timestamps(created, updated),
            source,
            path: path.to_path_buf(),
        })
    }

    pub fn load_source(&self, id: &WorkflowId) -> RepoResult<WorkflowSource> {
        let (path, scope) = self.locate(id).ok_or_else(|| RepoError::NotFound {
            kind: EntityKind::Workflow,
            id: id.to_string(),
        })?;
        self.read(&path, scope)
    }

    /// Writes `source` (already parsed into `workflow`) verbatim. Writing the
    /// identical text again is a no-op; a different workflow under an
    /// existing ID is refused.
    pub fn create_source(&self, workflow: &Workflow, source: &str) -> RepoResult<PathBuf> {
        if let Some((path, scope)) = self.locate(workflow.id()) {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            if existing == source && scope == *workflow.scope() {
                return Ok(path);
            }
            return Err(RepoError::AlreadyExists {
                kind: EntityKind::Workflow,
                id: workflow.id().to_string(),
            });
        }
        self.save_source(workflow, source)
    }

    /// Writes `source` as the workflow's file after checking it parses to
    /// `workflow`.
    pub fn save_source(&self, workflow: &Workflow, source: &str) -> RepoResult<PathBuf> {
        let parsed = Workflow::from_yaml(source)?;
        if !parsed.same_definition(workflow) {
            return Err(RepoError::Domain(
                crate::services::domain::DomainError::Rule(
                    "The YAML doesn't match the workflow being saved".to_string(),
                ),
            ));
        }
        self.write(workflow, source)
    }

    fn write(&self, workflow: &Workflow, contents: &str) -> RepoResult<PathBuf> {
        self.ensure_writable(workflow.id(), workflow.scope())?;
        let path = self.path_for(workflow.id(), workflow.scope());
        if write_if_changed(&path, contents.as_bytes()).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_add(&path);
            log::info!("Saved workflow {}", path.display());
        }
        Ok(path)
    }
}

impl Repository for WorkflowRepository {
    type Entity = Workflow;
    type Id = WorkflowId;

    fn load(&self, id: &WorkflowId) -> RepoResult<Workflow> {
        self.load_source(id).map(|s| s.workflow)
    }

    fn load_all(&self) -> Loaded<Workflow> {
        let mut loaded = Loaded::default();
        for (dir, scope) in self.folders() {
            for path in yaml_files(&dir) {
                match self.read(&path, scope.clone()) {
                    Ok(source) => loaded.entities.push(source.workflow),
                    Err(e) => {
                        log::warn!("Skipping workflow {}: {e}", path.display());
                        let id = Some(file_stem(&path));
                        loaded
                            .problems
                            .push(e.into_problem(EntityKind::Workflow, id, &path));
                    }
                }
            }
        }
        loaded
    }

    fn save(&self, workflow: &Workflow) -> RepoResult<PathBuf> {
        self.write(workflow, &workflow.to_yaml())
    }

    fn delete(&self, id: &WorkflowId) -> RepoResult<()> {
        let (path, _) = self.locate(id).ok_or_else(|| RepoError::NotFound {
            kind: EntityKind::Workflow,
            id: id.to_string(),
        })?;
        if self.layout.is_vendored(&path) {
            return Err(RepoError::ReadOnly {
                kind: EntityKind::Workflow,
                id: id.to_string(),
            });
        }
        remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))?;
        self.git.stage_remove(&path);
        Ok(())
    }

    fn exists(&self, id: &WorkflowId) -> bool {
        self.locate(id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::Repos;

    const YAML: &str =
        "# My workflow\nid: daily\nname: Daily\nsteps:\n  - {id: a, agent: writer, input: hi}\n";

    #[test]
    fn keeps_source_text_and_finds_scoped_workflows() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let workflow = Workflow::from_yaml(YAML).unwrap();
        repos.workflows.create_source(&workflow, YAML).unwrap();
        // Identical re-import is fine; a different one is refused
        repos.workflows.create_source(&workflow, YAML).unwrap();
        let other = YAML.replace("Daily", "Other");
        assert!(matches!(
            repos
                .workflows
                .create_source(&Workflow::from_yaml(&other).unwrap(), &other),
            Err(RepoError::AlreadyExists { .. })
        ));
        let loaded = repos.workflows.load_source(workflow.id()).unwrap();
        assert!(loaded.source.starts_with("# My workflow"));

        let scope = WorkflowScope::Agent(AgentId::parse("writer").unwrap());
        let scoped = Workflow::from_yaml(&YAML.replace("daily", "mine"))
            .unwrap()
            .in_scope(scope.clone());
        repos.workflows.save(&scoped).unwrap();
        // Scoped workflows are found by folder (the aggregate checks the agent)
        let all = repos.workflows.load_all();
        let ids: Vec<_> = all.entities.iter().map(|w| w.id().to_string()).collect();
        assert_eq!(ids, ["daily", "mine"]);
        assert_eq!(all.entities[1].scope(), &scope);

        repos.workflows.delete(scoped.id()).unwrap();
        assert!(!repos.workflows.exists(scoped.id()));
    }

    #[test]
    fn pack_and_vendored_agent_workflows_load_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let pack_wf = repos.layout().pack_dir("research").join("workflows");
        fs::create_dir_all(&pack_wf).unwrap();
        fs::write(pack_wf.join("daily.yaml"), YAML).unwrap();
        let agent = repos.layout().vendored_agent_dir("alice", "writer");
        fs::create_dir_all(agent.join("workflows")).unwrap();
        fs::write(agent.join("skill.md"), "---\nname: W\n---\n").unwrap();
        fs::write(
            agent.join("workflows/mine.yaml"),
            YAML.replace("daily", "mine"),
        )
        .unwrap();

        let all = repos.workflows.load_all();
        let ids: Vec<_> = all.entities.iter().map(|w| w.id().to_string()).collect();
        assert_eq!(ids, ["mine", "daily"]);
        assert_eq!(all.entities[1].scope(), &WorkflowScope::Global);
        for workflow in &all.entities {
            assert!(matches!(
                repos.workflows.save(workflow),
                Err(RepoError::ReadOnly { .. })
            ));
            assert!(matches!(
                repos.workflows.delete(workflow.id()),
                Err(RepoError::ReadOnly { .. })
            ));
        }
    }

    #[test]
    fn reports_broken_and_misnamed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = Repos::new(RepoLayout::new(tmp.path(), tmp.path().join("trash")));
        let dir = &repos.layout().workflows_dir;
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("daily.yaml"), YAML).unwrap();
        fs::write(dir.join("broken.yaml"), "not: [valid").unwrap();
        fs::write(dir.join("wrong-name.yaml"), YAML).unwrap();
        fs::write(dir.join(".daily.yaml.nandi-tmp-1"), YAML).unwrap();
        let loaded = repos.workflows.load_all();
        assert_eq!(loaded.entities.len(), 1);
        assert_eq!(loaded.problems.len(), 2);
        assert!(loaded
            .problems
            .iter()
            .any(|p| p.error.to_string().contains("must match the file name")));
    }
}
