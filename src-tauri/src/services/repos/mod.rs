//! Repositories: the only code that reads and writes the gold copy.
//!
//! Each entity has a repository with explicit load/save semantics over a
//! `RepoLayout`. Every write is atomic (`fs_atomic`) and staged in git
//! (`git`). Repositories don't touch SQLite or the event bus; the write path
//! (`services::repo_service`) sequences repo save → aggregate → projection
//! → event.
//!
//! See `docs/developer/domain-model.md`.

pub mod agent_locator;
pub mod agent_repo;
pub mod capability_repo;
pub mod connector_repo;
pub mod decision_repo;
pub mod fs_atomic;
pub mod git;
pub mod group_repo;
pub mod layout;
// Wave CD: Studio policies
pub mod policy_repo;
pub mod settings_repo;
pub mod trigger_repo;
pub mod vendor_repo;
pub mod workflow_repo;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use agent_repo::AgentRepository;
pub use connector_repo::ConnectorRepository;
pub use decision_repo::DecisionRepository;
pub use group_repo::GroupRepository;
pub use layout::RepoLayout;
pub use policy_repo::PolicyRepository;
pub use settings_repo::SettingsRepository;
pub use trigger_repo::TriggerRepository;
pub use vendor_repo::VendorRepository;
pub use workflow_repo::WorkflowRepository;

use crate::services::domain::repo::{Problem, RepoManifest, RepoSnapshot, Severity};
use crate::services::domain::{
    AgentId, AgentRepo, DomainError, Entity, EntityKind, TriggerId, WorkflowId,
};
use crate::services::projection::Fingerprint;

#[derive(Debug, Clone, PartialEq)]
pub enum RepoError {
    Domain(DomainError),
    Io {
        path: PathBuf,
        message: String,
    },
    NotFound {
        kind: EntityKind,
        id: String,
    },
    AlreadyExists {
        kind: EntityKind,
        id: String,
    },
    /// A marketplace agent or workflow: fork it to change it
    ReadOnly {
        kind: EntityKind,
        id: String,
    },
    /// A file whose declared `id` isn't its file name
    IdMismatch {
        path: PathBuf,
        declared: String,
    },
}

pub type RepoResult<T> = Result<T, RepoError>;

impl RepoError {
    pub fn io(path: &Path, error: impl fmt::Display) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }

    /// As a problem report for `kind`/`id` at `path`.
    pub fn into_problem(self, kind: EntityKind, id: Option<String>, path: &Path) -> Problem {
        let error = match self {
            Self::Domain(error) => error,
            other => DomainError::Rule(other.to_string()),
        };
        Problem {
            kind,
            id,
            source_path: Some(path.to_path_buf()),
            error,
            severity: Severity::Error,
        }
    }
}

impl fmt::Display for RepoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(error) => error.fmt(f),
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::NotFound { kind, id } => DomainError::NotFound {
                kind: *kind,
                id: id.clone(),
            }
            .fmt(f),
            Self::AlreadyExists { kind, id } => {
                let kind = kind.to_string();
                let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
                    "An"
                } else {
                    "A"
                };
                write!(f, "{article} {kind} with id '{id}' already exists")
            }
            Self::ReadOnly { kind, id } => write!(
                f,
                "The {kind} '{id}' is shared from the marketplace and is read-only. Fork it to customize."
            ),
            Self::IdMismatch { path, declared } => write!(
                f,
                "{} declares id '{declared}'; it must match the file name",
                path.display()
            ),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<DomainError> for RepoError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl From<RepoError> for String {
    fn from(error: RepoError) -> Self {
        error.to_string()
    }
}

/// Everything a `load_all` read: valid entities, and why the rest failed.
#[derive(Debug)]
pub struct Loaded<T> {
    pub entities: Vec<T>,
    pub problems: Vec<Problem>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self {
            entities: Vec::new(),
            problems: Vec::new(),
        }
    }
}

/// A collection of entities in the gold copy (agents, workflows, triggers).
pub trait Repository {
    type Entity;
    type Id: ?Sized;

    fn load(&self, id: &Self::Id) -> RepoResult<Self::Entity>;
    /// Every entity; broken files are reported, never fatal.
    fn load_all(&self) -> Loaded<Self::Entity>;
    /// Writes the entity's files (atomically) and stages them. Returns the
    /// main file's path.
    fn save(&self, entity: &Self::Entity) -> RepoResult<PathBuf>;
    /// Removes the entity's files and stages the removal.
    fn delete(&self, id: &Self::Id) -> RepoResult<()>;
    fn exists(&self, id: &Self::Id) -> bool;
}

/// A one-per-repo file (settings, connectors). A missing file loads as the
/// defaults.
pub trait SingletonRepository {
    type Entity;

    fn load(&self) -> RepoResult<Self::Entity>;
    fn save(&self, entity: &Self::Entity) -> RepoResult<PathBuf>;
    fn path(&self) -> PathBuf;
}

/// Every repository over one layout. Cheap to clone.
#[derive(Clone)]
pub struct Repos {
    layout: Arc<RepoLayout>,
    pub agents: AgentRepository,
    pub workflows: WorkflowRepository,
    pub triggers: TriggerRepository,
    pub settings: SettingsRepository,
    pub connectors: ConnectorRepository,
    pub vendor: VendorRepository,
    pub policies: PolicyRepository,
    pub groups: GroupRepository,
    pub decisions: DecisionRepository,
}

impl Repos {
    pub fn new(layout: RepoLayout) -> Self {
        let layout = Arc::new(layout);
        let git = git::GitIndex::new(&layout.root);
        Self {
            agents: AgentRepository::new(layout.clone(), git.clone()),
            workflows: WorkflowRepository::new(layout.clone(), git.clone()),
            triggers: TriggerRepository::new(layout.clone(), git.clone()),
            settings: SettingsRepository::new(layout.clone(), git.clone()),
            connectors: ConnectorRepository::new(layout.clone(), git.clone()),
            vendor: VendorRepository::new(layout.clone(), git.clone()),
            policies: PolicyRepository::new(layout.clone(), git.clone()),
            groups: GroupRepository::new(layout.clone(), git.clone()),
            decisions: DecisionRepository::new(layout.clone(), git),
            layout,
        }
    }

    pub fn layout(&self) -> &RepoLayout {
        &self.layout
    }

    /// Loads one entity from its file(s).
    pub fn load_entity(&self, kind: EntityKind, id: &str) -> RepoResult<Entity> {
        Ok(match kind {
            EntityKind::Agent => Entity::from(self.agents.load(&AgentId::parse(id)?)?),
            EntityKind::Workflow => Entity::Workflow(self.workflows.load(&WorkflowId::parse(id)?)?),
            EntityKind::Trigger => Entity::Trigger(self.triggers.load(&TriggerId::parse(id)?)?),
            EntityKind::Settings => Entity::Settings(self.settings.load()?),
            EntityKind::Connectors => Entity::Connectors(self.connectors.load()?),
            EntityKind::Manifest => Entity::Manifest(self.load_manifest()?),
        })
    }

    /// `.nandi/manifest.yaml`; a missing file means the current version.
    pub fn load_manifest(&self) -> RepoResult<RepoManifest> {
        let path = self.layout.manifest_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(RepoManifest::from_yaml(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RepoManifest::default()),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    /// Reads every file into a fresh aggregate. Broken files become
    /// problems; this never fails.
    pub fn load_aggregate(&self) -> AgentRepo {
        let agents = self.agents.load_all();
        let workflows = self.workflows.load_all();
        let triggers = self.triggers.load_all();
        let mut problems = Vec::new();
        let mut singleton = |kind: EntityKind| match self.load_entity(kind, "") {
            Ok(entity) => Some(entity),
            Err(e) => {
                let path = self.layout.source_path(kind, "", None);
                problems.push(e.into_problem(kind, kind.singleton_id().map(String::from), &path));
                None
            }
        };
        let settings = match singleton(EntityKind::Settings) {
            Some(Entity::Settings(settings)) => Some(settings),
            _ => None,
        };
        let connectors = match singleton(EntityKind::Connectors) {
            Some(Entity::Connectors(connectors)) => Some(connectors),
            _ => None,
        };
        let manifest = match singleton(EntityKind::Manifest) {
            Some(Entity::Manifest(manifest)) => Some(manifest),
            _ => None,
        };
        problems.extend(agents.problems);
        problems.extend(workflows.problems);
        problems.extend(triggers.problems);
        AgentRepo::assemble(RepoSnapshot {
            manifest,
            agents: agents.entities,
            workflows: workflows.entities,
            triggers: triggers.entities,
            settings,
            connectors,
            problems,
        })
    }

    /// The main file and content hash of an entity's file(s), or `None` if
    /// they don't exist. Singletons always have one (a missing file hashes
    /// as empty: the defaults).
    pub fn fingerprint(&self, kind: EntityKind, id: &str) -> Option<Fingerprint> {
        if kind.singleton_id().is_none() && !is_plain_name(id) {
            return None; // never build a path from `..` or separators
        }
        let read = |path: &Path| std::fs::read(path).ok();
        let (path, bytes) = match kind {
            EntityKind::Agent => {
                let dir = self.agents.locate(id)?.dir;
                let skill = dir.join(layout::SKILL_FILE);
                let mut bytes = read(&skill)?;
                if let Some(extra) = read(&dir.join(layout::INSTRUCTIONS_FILE)) {
                    bytes.push(0);
                    bytes.extend(extra);
                }
                // An installed agent's own rules (`agent_repo::save_instructions`)
                if let Some(extra) = read(&self.layout.instructions_override_path(id)) {
                    bytes.extend(b"\0override\0");
                    bytes.extend(extra);
                }
                // Its playbooks: editing one updates the agent
                for (name, path) in agent_repo::playbook_files(&dir.join(layout::PLAYBOOKS_DIR)) {
                    if let Some(extra) = read(&path) {
                        bytes.extend(format!("\0skills/{name}\0").as_bytes());
                        bytes.extend(extra);
                    }
                }
                (skill, bytes)
            }
            EntityKind::Workflow => {
                let (path, _) = self.workflows.locate(&WorkflowId::parse(id).ok()?)?;
                let bytes = read(&path)?;
                (path, bytes)
            }
            EntityKind::Trigger => {
                let path = self.layout.trigger_path(id);
                let bytes = read(&path)?;
                (path, bytes)
            }
            EntityKind::Settings | EntityKind::Connectors | EntityKind::Manifest => {
                let path = self.layout.source_path(kind, id, None);
                let bytes = read(&path).unwrap_or_default();
                (path, bytes)
            }
        };
        Some(Fingerprint {
            kind,
            id: kind.singleton_id().unwrap_or(id).to_string(),
            source_path: path.display().to_string(),
            hash: fs_atomic::content_hash(&bytes),
        })
    }

    /// Fingerprints of everything in the aggregate, singletons included.
    pub fn fingerprints(&self, repo: &AgentRepo) -> Vec<Fingerprint> {
        let ids = repo
            .agents()
            .map(|a| (EntityKind::Agent, a.id().to_string()))
            .chain(
                repo.workflows()
                    .map(|w| (EntityKind::Workflow, w.id().to_string())),
            )
            .chain(
                repo.triggers()
                    .map(|t| (EntityKind::Trigger, t.id().to_string())),
            )
            .chain(
                [
                    EntityKind::Settings,
                    EntityKind::Connectors,
                    EntityKind::Manifest,
                ]
                .into_iter()
                .map(|k| (k, k.singleton_id().unwrap_or_default().to_string())),
            );
        ids.filter_map(|(kind, id)| self.fingerprint(kind, &id))
            .collect()
    }

    /// The IDs of `kind` with files on disk right now (valid or not).
    pub fn ids_on_disk(&self, kind: EntityKind) -> Vec<String> {
        match kind {
            EntityKind::Agent => self.agents.folder_names(),
            EntityKind::Workflow => self.workflows.file_ids(),
            EntityKind::Trigger => yaml_files(&self.layout.triggers_dir)
                .iter()
                .map(|p| file_stem(p))
                .collect(),
            other => other.singleton_id().map(String::from).into_iter().collect(),
        }
    }
}

/// Reads a file, mapping "not found" to `RepoError::NotFound`.
pub(crate) fn read_text(path: &Path, kind: EntityKind, id: &str) -> RepoResult<String> {
    std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RepoError::NotFound {
            kind,
            id: id.to_string(),
        },
        _ => RepoError::io(path, e),
    })
}

/// `*.yaml` / `*.yml` files in a folder, sorted; empty if it doesn't exist.
pub(crate) fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    files.sort();
    files
}

/// A single, non-hidden path component: safe to join onto a folder.
pub(crate) fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && Path::new(name).components().count() == 1
}

pub(crate) fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
