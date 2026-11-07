//! The live gold copy: the `AgentRepo` aggregate in memory, kept in step
//! with the files (canonical) and the SQLite projection (derived).
//!
//! - `load` reads every file at startup, then brings the projection up to
//!   date: a full rebuild if it's empty or from another projection version,
//!   otherwise only entities whose file hash drifted.
//! - `reload(kind, id, by)` re-reads one entity after its file changed and
//!   publishes what changed (`EventPublisher`). A broken file records a
//!   problem, publishes `repo://validation_failed`, and keeps the last good
//!   version (in memory and in SQLite). Unchanged files (same hash) are a
//!   no-op.
//! - `commit(kind, id)` is `reload` after a command wrote the files: the
//!   last step of the write path (domain → repo → projection → event).
//!
//! Reads (`agent`, `skill`, `list_agents`) are read-through until the file
//! watcher keeps the aggregate fresh: they re-check the files' hashes,
//! which is what lets an edited skill.md apply to the next model call.
//!
//! All methods block (filesystem + SQLite); async callers use
//! `spawn_blocking` for anything that may touch many files.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::db::Db;
use crate::services::domain::repo::Problem;
use crate::services::domain::{
    Agent, AgentRepo, ChangeAction, ChangedBy, DomainEvent, Entity, EntityKind,
};
use crate::services::projection::{Fingerprint, ProjectionSink, ProjectionStats, SqliteProjection};
use crate::services::repos::{RepoError, RepoLayout, Repos};

/// What a reload did.
#[derive(Debug, Clone, PartialEq)]
pub enum Reload {
    /// Same file contents as last time
    Unchanged,
    Changed(ChangeAction),
    /// The file is invalid; the last good version stays
    Invalid(Problem),
}

/// The port changes are published through: the event bus, and the
/// runtime reactions (orchestrators, trigger sources).
pub trait EventPublisher: Send + Sync {
    fn changed(&self, event: &DomainEvent);
    fn invalid(&self, problem: &Problem, changed_by: ChangedBy);
}

/// What a reload found, published once the writer lock is released.
enum Notice {
    Changed(DomainEvent),
    Invalid(Problem),
}

#[derive(Clone)]
pub struct RepoStore(Arc<Inner>);

/// A [`RepoStore`] that doesn't keep it alive, for its own subscribers
#[derive(Clone)]
pub struct WeakRepoStore(std::sync::Weak<Inner>);

impl WeakRepoStore {
    pub fn upgrade(&self) -> Option<RepoStore> {
        self.0.upgrade().map(RepoStore)
    }
}

struct Inner {
    subscribers: RwLock<Vec<Arc<dyn EventPublisher>>>,
    /// The file watcher is keeping the aggregate fresh
    watching: AtomicBool,
    repos: Repos,
    aggregate: RwLock<AgentRepo>,
    projection: Arc<dyn ProjectionSink>,
    /// The hash each entity was last loaded from (mirrors `sync_state`)
    hashes: Mutex<HashMap<(EntityKind, String), String>>,
    /// One reload or rebuild at a time, so a file's state and its
    /// projection never interleave with another writer's
    writer: Mutex<()>,
}

impl RepoStore {
    pub fn new(repos: Repos, projection: Arc<dyn ProjectionSink>) -> Self {
        let root = repos.layout().root.clone();
        Self(Arc::new(Inner {
            subscribers: RwLock::default(),
            watching: AtomicBool::new(false),
            repos,
            aggregate: RwLock::new(AgentRepo::empty(root)),
            projection,
            hashes: Mutex::default(),
            writer: Mutex::new(()),
        }))
    }

    pub fn sqlite(repos: Repos, db: Db) -> Self {
        Self::new(repos, Arc::new(SqliteProjection::new(db)))
    }

    /// A store over a fresh temporary folder.
    #[cfg(test)]
    pub fn for_tests(db: Db) -> Self {
        let root = std::env::temp_dir().join(format!("nandi-store-{}", uuid::Uuid::new_v4()));
        Self::sqlite(Repos::new(RepoLayout::new(&root, root.join("trash"))), db)
    }

    pub fn repos(&self) -> &Repos {
        &self.0.repos
    }

    pub fn downgrade(&self) -> WeakRepoStore {
        WeakRepoStore(Arc::downgrade(&self.0))
    }

    /// Whether the file watcher keeps the aggregate fresh (so listing can
    /// skip re-scanning the files).
    pub fn is_watching(&self) -> bool {
        self.0.watching.load(Ordering::Relaxed)
    }

    pub fn set_watching(&self, watching: bool) {
        self.0.watching.store(watching, Ordering::Relaxed);
    }

    /// Re-scans every entity of every kind. Returns whether anything changed.
    pub fn rescan_all(&self, changed_by: ChangedBy) -> bool {
        let mut changed = false;
        for kind in [EntityKind::Agent, EntityKind::Workflow, EntityKind::Trigger] {
            changed |= self.rescan(kind, changed_by);
        }
        for kind in [
            EntityKind::Settings,
            EntityKind::Connectors,
            EntityKind::Manifest,
        ] {
            changed |= matches!(self.reload(kind, "", changed_by), Reload::Changed(_));
        }
        changed
    }

    /// Publishes every later change to `subscriber`.
    pub fn subscribe(&self, subscriber: Arc<dyn EventPublisher>) {
        self.0
            .subscribers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(subscriber);
    }

    fn publish(&self, notices: Vec<Notice>, changed_by: ChangedBy) {
        if notices.is_empty() {
            return;
        }
        let subscribers = self
            .0
            .subscribers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for notice in &notices {
            for subscriber in &subscribers {
                match notice {
                    Notice::Changed(event) => subscriber.changed(&DomainEvent {
                        changed_by,
                        ..event.clone()
                    }),
                    Notice::Invalid(problem) => subscriber.invalid(problem, changed_by),
                }
            }
        }
    }

    pub fn layout(&self) -> &RepoLayout {
        self.0.repos.layout()
    }

    /// Reads the aggregate.
    pub fn read<R>(&self, f: impl FnOnce(&AgentRepo) -> R) -> R {
        f(&self.0.aggregate.read().unwrap_or_else(|e| e.into_inner()))
    }

    fn write_aggregate<R>(&self, f: impl FnOnce(&mut AgentRepo) -> R) -> R {
        f(&mut self.0.aggregate.write().unwrap_or_else(|e| e.into_inner()))
    }

    fn hashes(&self) -> std::sync::MutexGuard<'_, HashMap<(EntityKind, String), String>> {
        self.0.hashes.lock().unwrap_or_else(|e| e.into_inner())
    }

    // -- Loading and rebuilding ---------------------------------------------

    /// Loads every file and brings the projection up to date (rebuilding
    /// drifted entities, or everything if needed). Returns what it did.
    pub fn load(&self) -> Result<ProjectionStats, String> {
        let _writer = self.0.writer.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = std::fs::create_dir_all(&self.layout().agents_dir) {
            log::warn!(
                "Failed to create {}: {e}",
                self.layout().agents_dir.display()
            );
        }
        let aggregate = self.0.repos.load_aggregate();
        let fingerprints = self.0.repos.fingerprints(&aggregate);
        let stats = if self.0.projection.needs_rebuild()? {
            log::info!("Rebuilding the projection from the repo files");
            self.0.projection.rebuild_all(&aggregate, &fingerprints)?
        } else {
            self.project_drift(&aggregate, &fingerprints)?
        };
        self.install(aggregate, &fingerprints);
        Ok(stats)
    }

    /// Wipes the projection and rebuilds it from the files (invalid files
    /// are skipped; see `rebuild_checked`).
    pub fn rebuild(&self) -> Result<ProjectionStats, String> {
        let _writer = self.0.writer.lock().unwrap_or_else(|e| e.into_inner());
        let aggregate = self.0.repos.load_aggregate();
        let fingerprints = self.0.repos.fingerprints(&aggregate);
        let stats = self.0.projection.rebuild_all(&aggregate, &fingerprints)?;
        self.install(aggregate, &fingerprints);
        Ok(stats)
    }

    /// Re-projects entities whose hash differs from `sync_state`, and drops
    /// rows whose files are gone.
    fn project_drift(
        &self,
        aggregate: &AgentRepo,
        fingerprints: &[Fingerprint],
    ) -> Result<ProjectionStats, String> {
        let stored = self.0.projection.hashes()?;
        let mut stats = ProjectionStats::default();
        let mut current = std::collections::HashSet::new();
        for fingerprint in fingerprints {
            let key = (fingerprint.kind, fingerprint.id.clone());
            current.insert(key.clone());
            if stored.get(&key) != Some(&fingerprint.hash) {
                log::info!(
                    "Re-projecting {} '{}' (file changed)",
                    fingerprint.kind,
                    fingerprint.id
                );
                self.0.projection.project(
                    aggregate,
                    fingerprint.kind,
                    &fingerprint.id,
                    Some(fingerprint),
                )?;
                match fingerprint.kind {
                    EntityKind::Agent => stats.agents += 1,
                    EntityKind::Workflow => stats.workflows += 1,
                    EntityKind::Connectors => stats.connectors += 1,
                    _ => {}
                }
            }
        }
        for (kind, id) in stored.keys().filter(|key| !current.contains(*key)) {
            log::info!("Removing {kind} '{id}' from the projection (file gone)");
            self.0.projection.project(aggregate, *kind, id, None)?;
        }
        Ok(stats)
    }

    fn install(&self, aggregate: AgentRepo, fingerprints: &[Fingerprint]) {
        *self.hashes() = fingerprints
            .iter()
            .map(|f| ((f.kind, f.id.clone()), f.hash.clone()))
            .collect();
        self.write_aggregate(|repo| *repo = aggregate);
    }

    /// Re-reads one entity from its file(s), updates the aggregate and the
    /// projection, and publishes what changed.
    pub fn reload(&self, kind: EntityKind, id: &str, changed_by: ChangedBy) -> Reload {
        let mut notices = Vec::new();
        let reload = {
            let _writer = self.0.writer.lock().unwrap_or_else(|e| e.into_inner());
            self.reload_locked(kind, id, &mut notices)
        };
        self.publish(notices, changed_by);
        reload
    }

    /// The last step of a command's write: the files are saved, now update
    /// the aggregate and projection and publish. A file the command just
    /// wrote being invalid is an error.
    pub fn commit(&self, kind: EntityKind, id: &str) -> Result<Reload, String> {
        match self.reload(kind, id, ChangedBy::User) {
            Reload::Invalid(problem) => Err(problem.error.to_string()),
            reload => Ok(reload),
        }
    }

    fn reload_locked(&self, kind: EntityKind, id: &str, notices: &mut Vec<Notice>) -> Reload {
        let id = kind.singleton_id().unwrap_or(id).to_string();
        let key = (kind, id.clone());
        let fingerprint = self.0.repos.fingerprint(kind, &id);
        let (known, broken) = self.read(|repo| {
            (
                repo.get(kind, &id).is_some(),
                repo.has_load_problem(kind, &id),
            )
        });
        let cached = self.hashes().get(&key).cloned();

        let Some(fingerprint) = fingerprint else {
            return self.remove_locked(kind, &id, known || cached.is_some(), notices);
        };
        // A file reverted to its last good contents still clears its problem
        if known && !broken && cached.as_deref() == Some(fingerprint.hash.as_str()) {
            return Reload::Unchanged;
        }
        let entity = match self.0.repos.load_entity(kind, &id) {
            Ok(entity) => entity,
            Err(RepoError::NotFound { .. }) => {
                return self.remove_locked(kind, &id, known, notices)
            }
            Err(e) => {
                let path = std::path::PathBuf::from(&fingerprint.source_path);
                let problem = e.into_problem(kind, Some(id.clone()), &path);
                log::warn!("{kind} '{id}' not reloaded: {}", problem.error);
                self.write_aggregate(|repo| repo.record_problem(problem.clone()));
                notices.push(Notice::Invalid(problem.clone()));
                return Reload::Invalid(problem);
            }
        };
        let upserted = self.write_aggregate(|repo| repo.upsert(entity));
        let action = match upserted {
            Ok(action) => action,
            Err(error) => {
                let problem = Problem {
                    kind,
                    id: Some(id.clone()),
                    source_path: Some(fingerprint.source_path.clone().into()),
                    error,
                    severity: crate::services::domain::repo::Severity::Error,
                };
                self.write_aggregate(|repo| repo.record_problem(problem.clone()));
                notices.push(Notice::Invalid(problem.clone()));
                return Reload::Invalid(problem);
            }
        };
        if let Err(e) = self.read(|repo| {
            self.0
                .projection
                .project(repo, kind, &id, Some(&fingerprint))
        }) {
            log::error!("{e}");
        }
        self.hashes().insert(key, fingerprint.hash);
        match action {
            Some(action) => {
                notices.push(Notice::Changed(DomainEvent {
                    kind,
                    id,
                    action,
                    source_path: fingerprint.source_path.into(),
                    changed_by: ChangedBy::User,
                }));
                Reload::Changed(action)
            }
            None => Reload::Unchanged,
        }
    }

    fn remove_locked(
        &self,
        kind: EntityKind,
        id: &str,
        existed: bool,
        notices: &mut Vec<Notice>,
    ) -> Reload {
        if !existed {
            // Never loaded (or already gone): just forget any problem
            self.write_aggregate(|repo| repo.remove(kind, id));
            return Reload::Unchanged;
        }
        let removed = self.write_aggregate(|repo| repo.remove(kind, id));
        if let Err(e) = self.read(|repo| self.0.projection.project(repo, kind, id, None)) {
            log::error!("{e}");
        }
        self.hashes().remove(&(kind, id.to_string()));
        let scope = match &removed {
            Some(Entity::Workflow(workflow)) => Some(workflow.scope().clone()),
            _ => None,
        };
        notices.push(Notice::Changed(DomainEvent {
            kind,
            id: id.to_string(),
            action: ChangeAction::Deleted,
            source_path: self.layout().source_path(kind, id, scope.as_ref()),
            changed_by: ChangedBy::User,
        }));
        // An agent's scoped workflows went with its folder
        if let Some(Entity::Agent(agent)) = &removed {
            for workflow in agent.workflows() {
                self.reload_locked(EntityKind::Workflow, workflow.as_str(), notices);
            }
        }
        Reload::Changed(ChangeAction::Deleted)
    }

    /// Reloads every entity of `kind` on disk or in memory (new, changed,
    /// and removed files). Returns whether anything changed.
    pub fn rescan(&self, kind: EntityKind, changed_by: ChangedBy) -> bool {
        let mut notices = Vec::new();
        let changed = {
            let _writer = self.0.writer.lock().unwrap_or_else(|e| e.into_inner());
            self.rescan_locked(kind, &mut notices)
        };
        self.publish(notices, changed_by);
        changed
    }

    fn rescan_locked(&self, kind: EntityKind, notices: &mut Vec<Notice>) -> bool {
        let mut ids = self.0.repos.ids_on_disk(kind);
        self.read(|repo| match kind {
            EntityKind::Agent => ids.extend(repo.agents().map(|a| a.id().to_string())),
            EntityKind::Workflow => ids.extend(repo.workflows().map(|w| w.id().to_string())),
            EntityKind::Trigger => ids.extend(repo.triggers().map(|t| t.id().to_string())),
            _ => {}
        });
        ids.sort();
        ids.dedup();
        let mut changed = false;
        for id in ids {
            changed |= matches!(self.reload_locked(kind, &id, notices), Reload::Changed(_));
        }
        changed
    }

    // -- Agent reads ----------------------------------------------------------

    /// The agent, re-checked against its files.
    pub fn agent(&self, id: &str) -> Result<Agent, String> {
        crate::services::domain::AgentId::parse(id).map_err(|e| e.to_string())?;
        if let Reload::Invalid(problem) = self.reload(EntityKind::Agent, id, ChangedBy::Watcher) {
            // The last good version (if any) keeps running
            if let Some(agent) = self.read(|repo| repo.agent(id).cloned()) {
                return Ok(agent);
            }
            return Err(problem.error.to_string());
        }
        self.read(|repo| repo.agent(id).cloned())
            .ok_or_else(|| format!("Agent '{id}' not found"))
    }

    /// The agent's display name, falling back to its ID. Doesn't touch disk.
    pub fn display_name(&self, id: &str) -> String {
        self.read(|repo| repo.agent(id).map(|a| a.name().to_string()))
            .unwrap_or_else(|| id.to_string())
    }

    /// Fails unless the agent exists (re-checked against its files).
    pub fn require_agent(&self, id: &str) -> Result<(), String> {
        self.agent(id).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn store(root: &std::path::Path) -> RepoStore {
        let layout = RepoLayout::new(root, root.join("../trash"));
        RepoStore::sqlite(Repos::new(layout), Db::in_memory())
    }

    fn write(path: std::path::PathBuf, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn loads_reloads_and_keeps_the_last_good_version() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(&tmp.path().join("repo"));
        let skill = store.layout().skill_path("writer");
        write(skill.clone(), "---\nname: Writer\n---\nWrite.");
        store.load().unwrap();
        assert_eq!(store.display_name("writer"), "Writer");
        assert_eq!(
            store.reload(EntityKind::Agent, "writer", ChangedBy::Watcher),
            Reload::Unchanged
        );

        write(skill.clone(), "---\nname: Author\n---\nWrite.");
        assert_eq!(
            store.reload(EntityKind::Agent, "writer", ChangedBy::Watcher),
            Reload::Changed(ChangeAction::Updated)
        );
        assert_eq!(store.display_name("writer"), "Author");

        // Broken: the last good version stays, with a problem
        write(skill.clone(), "---\nname: [oops\n---\n");
        assert!(matches!(
            store.reload(EntityKind::Agent, "writer", ChangedBy::Watcher),
            Reload::Invalid(_)
        ));
        assert_eq!(store.agent("writer").unwrap().name(), "Author");
        assert!(store.read(|r| r
            .problems()
            .iter()
            .any(|p| p.id.as_deref() == Some("writer"))));

        fs::remove_dir_all(skill.parent().unwrap()).unwrap();
        assert_eq!(
            store.reload(EntityKind::Agent, "writer", ChangedBy::Watcher),
            Reload::Changed(ChangeAction::Deleted)
        );
        assert!(store.agent("writer").is_err());
    }

    #[test]
    fn startup_reprojects_only_drifted_entities() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let layout = RepoLayout::new(&root, tmp.path().join("trash"));
        let db = Db::in_memory();
        write(layout.skill_path("a"), "---\nname: A\n---\n");
        write(layout.skill_path("b"), "---\nname: B\n---\n");
        let first = RepoStore::sqlite(Repos::new(layout.clone()), db.clone());
        assert_eq!(
            first.load().unwrap().agents,
            2,
            "a fresh projection is fully built"
        );

        // While "closed": edit one file, delete another
        write(layout.skill_path("a"), "---\nname: A2\n---\n");
        fs::remove_dir_all(layout.agent_dir("b")).unwrap();
        let second = RepoStore::sqlite(Repos::new(layout), db.clone());
        assert_eq!(second.load().unwrap().agents, 1, "only the drifted agent");
        let names: Vec<String> = db
            .conn()
            .unwrap()
            .prepare("SELECT name FROM agents")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(names, ["A2"]);
    }

    #[test]
    fn rescans_pick_up_new_and_removed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(&tmp.path().join("repo"));
        store.load().unwrap();
        let path = store.layout().workflow_path("daily", &Default::default());
        write(
            path.clone(),
            "id: daily\nname: Daily\nsteps:\n  - {id: a, agent: w, input: hi}\n",
        );
        assert!(store.rescan(EntityKind::Workflow, ChangedBy::Watcher));
        assert!(!store.rescan(EntityKind::Workflow, ChangedBy::Watcher));
        fs::remove_file(path).unwrap();
        assert!(store.rescan(EntityKind::Workflow, ChangedBy::Watcher));
        assert!(store.read(|r| r.workflow("daily").is_none()));
    }
}
