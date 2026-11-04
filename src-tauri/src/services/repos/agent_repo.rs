//! Agents: `<agents_dir>/<id>/skill.md` and the optional `instructions.md`.
//!
//! Marketplace agents (`<agents_dir>/marketplace/…`, see `agent_locator`)
//! load like any other but are read-only here: saving or deleting one is
//! `RepoError::ReadOnly`. The installer (`VendorRepository`) owns them.
//! Their `instructions.md` is the user's to change all the same: an
//! override in `.nandi/overrides/<id>/instructions.md` replaces the shipped
//! one, and updates never touch it (`save_instructions`).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use super::agent_locator::{self, AgentPlace};
use super::fs_atomic::{file_times, move_to_trash, remove_if_exists, write_if_changed};
use super::git::GitIndex;
use super::layout::{INSTRUCTIONS_FILE, MARKETPLACE_DIR, MEMORY_FILE, SKILL_FILE};
use super::{read_text, Loaded, RepoError, RepoLayout, RepoResult, Repository};
use crate::services::domain::repo::{Problem, Severity};
use crate::services::domain::{Agent, AgentId, DomainError, EntityKind};

#[derive(Clone)]
pub struct AgentRepository {
    layout: Arc<RepoLayout>,
    git: GitIndex,
}

impl AgentRepository {
    pub(super) fn new(layout: Arc<RepoLayout>, git: GitIndex) -> Self {
        Self { layout, git }
    }

    /// Saves a new agent. Fails if its folder exists: `create_dir` (not
    /// `_all`) makes two concurrent creates of the same ID race safely.
    pub fn create(&self, agent: &Agent) -> RepoResult<PathBuf> {
        let id = agent.id().as_str();
        if id == MARKETPLACE_DIR {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "'{MARKETPLACE_DIR}' is reserved for installed agents; choose another ID"
            ))));
        }
        if self.locate(id).is_some() {
            return Err(RepoError::AlreadyExists {
                kind: EntityKind::Agent,
                id: id.to_string(),
            });
        }
        let dir = self.layout.agent_dir(id);
        fs::create_dir_all(&self.layout.agents_dir)
            .map_err(|e| RepoError::io(&self.layout.agents_dir, e))?;
        fs::create_dir(&dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => RepoError::AlreadyExists {
                kind: EntityKind::Agent,
                id: agent.id().to_string(),
            },
            _ => RepoError::io(&dir, e),
        })?;
        self.save(agent)
    }

    /// Names of every agent folder holding a skill.md (local and vendored),
    /// valid or not.
    pub fn folder_names(&self) -> Vec<String> {
        let mut names: Vec<String> = agent_locator::all(&self.layout)
            .into_iter()
            .map(|place| place.id)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Where agent `id` lives, if it has a folder.
    pub fn locate(&self, id: &str) -> Option<AgentPlace> {
        agent_locator::locate(&self.layout, id)
    }

    /// The agent's folder (or where a new one would go).
    pub fn dir(&self, id: &str) -> PathBuf {
        agent_locator::agent_dir(&self.layout, id)
    }

    /// The agent's `memory.md`, if it has one (wave final, Track C)
    #[allow(dead_code)]
    pub fn read_memory(&self, id: &str) -> RepoResult<Option<String>> {
        let path = self.dir(id).join(MEMORY_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::io(&path, e)),
        }
    }

    /// Replaces the agent's `memory.md` atomically and stages it
    #[allow(dead_code)]
    pub fn write_memory(&self, id: &str, content: &str) -> RepoResult<PathBuf> {
        self.ensure_writable(id)?;
        if self.locate(id).is_none() {
            return Err(RepoError::NotFound {
                kind: EntityKind::Agent,
                id: id.to_string(),
            });
        }
        let path = self.dir(id).join(MEMORY_FILE);
        if write_if_changed(&path, content.as_bytes()).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    /// Fails if the agent is a vendored marketplace agent.
    fn ensure_writable(&self, id: &str) -> RepoResult<()> {
        match self.locate(id) {
            Some(place) if place.origin.is_vendored() => Err(RepoError::ReadOnly {
                kind: EntityKind::Agent,
                id: id.to_string(),
            }),
            _ => Ok(()),
        }
    }

    /// Where the instructions of the agent at `place` come from: the user's
    /// override for an installed agent when there is one, else its folder
    fn instructions_source(&self, id: &str, place: &AgentPlace) -> PathBuf {
        let override_path = self.layout.instructions_override_path(id);
        if place.origin.is_vendored() && override_path.is_file() {
            override_path
        } else {
            place.dir.join(INSTRUCTIONS_FILE)
        }
    }

    /// Whether installed agent `id` runs with the user's own instructions
    pub fn has_instructions_override(&self, id: &str) -> bool {
        self.locate(id)
            .is_some_and(|place| place.origin.is_vendored())
            && self.layout.instructions_override_path(id).is_file()
    }

    /// Replaces agent `id`'s instructions with `text` (empty removes them).
    /// A local agent's go in its folder; an installed agent's go in its
    /// override, so updates keep them.
    pub fn save_instructions(&self, id: &str, text: &str) -> RepoResult<PathBuf> {
        let place = self.locate(id).ok_or_else(|| RepoError::NotFound {
            kind: EntityKind::Agent,
            id: id.to_string(),
        })?;
        let path = if place.origin.is_vendored() {
            self.layout.instructions_override_path(id)
        } else {
            self.layout.instructions_path(id)
        };
        let text = text.trim();
        if text.is_empty() && !place.origin.is_vendored() {
            if remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))? {
                self.git.stage_remove(&path);
            }
            return Ok(path);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| RepoError::io(parent, e))?;
        }
        // An installed agent's empty override means "no instructions", not
        // "back to the shipped ones" (that's `reset_instructions`)
        let content = if text.is_empty() {
            String::new()
        } else {
            format!(
                "{text}
"
            )
        };
        if write_if_changed(&path, content.as_bytes()).map_err(|e| RepoError::io(&path, e))? {
            self.git.stage_add(&path);
        }
        Ok(path)
    }

    /// Drops installed agent `id`'s override: back to the shipped
    /// instructions. Returns whether there was one.
    pub fn reset_instructions(&self, id: &str) -> RepoResult<bool> {
        let path = self.layout.instructions_override_path(id);
        let removed = remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))?;
        if removed {
            self.git.stage_remove(&path);
        }
        Ok(removed)
    }

    fn read_place(&self, id: &AgentId, place: &AgentPlace) -> RepoResult<Agent> {
        let skill_path = place.dir.join(SKILL_FILE);
        let skill = read_text(&skill_path, EntityKind::Agent, id.as_str())?;
        let instructions_path = self.instructions_source(id.as_str(), place);
        let instructions = match fs::read_to_string(&instructions_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(RepoError::io(&instructions_path, e)),
        };
        let agent = Agent::from_files(id.clone(), &skill, instructions.as_deref())?;
        let (created, mut updated) = file_times(&skill_path);
        if instructions.is_some() {
            updated = updated.max(file_times(&instructions_path).1);
        }
        Ok(agent.with_timestamps(created, updated))
    }
}

impl Repository for AgentRepository {
    type Entity = Agent;
    type Id = AgentId;

    fn load(&self, id: &AgentId) -> RepoResult<Agent> {
        let place = self
            .locate(id.as_str())
            .ok_or_else(|| RepoError::NotFound {
                kind: EntityKind::Agent,
                id: id.to_string(),
            })?;
        self.read_place(id, &place)
    }

    fn load_all(&self) -> Loaded<Agent> {
        let mut loaded = Loaded::default();
        let mut seen = std::collections::HashSet::new();
        for place in agent_locator::all(&self.layout) {
            let name = place.id.clone();
            let path = place.dir.join(SKILL_FILE);
            if !seen.insert(name.clone()) {
                // The same ID in another folder: the first one loads
                log::warn!("Skipping {}: agent '{name}' already exists", path.display());
                loaded.problems.push(Problem {
                    kind: EntityKind::Agent,
                    id: Some(name.clone()),
                    source_path: Some(path),
                    error: DomainError::Duplicate {
                        kind: EntityKind::Agent,
                        id: name,
                    },
                    severity: Severity::Warning,
                });
                continue;
            }
            let result = AgentId::parse(&name)
                .map_err(RepoError::from)
                .and_then(|id| self.read_place(&id, &place));
            match result {
                Ok(agent) => loaded.entities.push(agent),
                Err(e) => {
                    log::warn!("Skipping agent '{name}': {e}");
                    loaded
                        .problems
                        .push(e.into_problem(EntityKind::Agent, Some(name), &path));
                }
            }
        }
        loaded
    }

    fn save(&self, agent: &Agent) -> RepoResult<PathBuf> {
        let id = agent.id().as_str();
        self.ensure_writable(id)?;
        let skill_path = self.layout.skill_path(id);
        if write_if_changed(&skill_path, agent.to_skill_md().as_bytes())
            .map_err(|e| RepoError::io(&skill_path, e))?
        {
            self.git.stage_add(&skill_path);
        }
        let instructions_path = self.layout.instructions_path(id);
        let instructions = agent.to_instructions_md();
        if instructions.is_empty() {
            if remove_if_exists(&instructions_path)
                .map_err(|e| RepoError::io(&instructions_path, e))?
            {
                self.git.stage_remove(&instructions_path);
            }
        } else if write_if_changed(&instructions_path, instructions.as_bytes())
            .map_err(|e| RepoError::io(&instructions_path, e))?
        {
            self.git.stage_add(&instructions_path);
        }
        Ok(skill_path)
    }

    /// Moves the folder (agent-scoped workflows included) to the trash and
    /// stages the removal. Nothing is deleted outright.
    fn delete(&self, id: &AgentId) -> RepoResult<()> {
        if !self.exists(id) {
            return Err(RepoError::NotFound {
                kind: EntityKind::Agent,
                id: id.to_string(),
            });
        }
        self.ensure_writable(id.as_str())?;
        let dir = self.layout.agent_dir(id.as_str());
        let target =
            move_to_trash(&dir, &self.layout.trash_dir).map_err(|e| RepoError::io(&dir, e))?;
        self.git.stage_remove_dir(&dir);
        log::info!("Moved agent '{id}' to {}", target.display());
        Ok(())
    }

    fn exists(&self, id: &AgentId) -> bool {
        self.locate(id.as_str()).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent::AgentDraft;
    use crate::services::repos::Repos;

    fn repos(dir: &std::path::Path) -> Repos {
        git2::Repository::init(dir).unwrap();
        Repos::new(RepoLayout::new(dir, dir.join("../trash")))
    }

    fn id(s: &str) -> AgentId {
        AgentId::parse(s).unwrap()
    }

    #[test]
    fn creates_loads_and_deletes_with_git_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let repos = repos(&root);
        let mut draft = AgentDraft::named(id("writer"), "Writer");
        draft.skill_body = "Write well.".into();
        draft.instructions = "Be brief.".into();
        let agent = Agent::new(draft).unwrap();

        let path = repos.agents.create(&agent).unwrap();
        assert!(
            path.ends_with(".nandi/agents/writer/skill.md")
                || path.ends_with(".nandi\\agents\\writer\\skill.md")
        );
        assert!(matches!(
            repos.agents.create(&agent),
            Err(RepoError::AlreadyExists { .. })
        ));
        let loaded = repos.agents.load(&id("writer")).unwrap();
        assert_eq!(loaded.system_prompt(), "Write well.\n\nBe brief.");
        let git = git2::Repository::open(&root).unwrap();
        let index = git.index().unwrap();
        assert!(index
            .get_path(std::path::Path::new(".nandi/agents/writer/skill.md"), 0)
            .is_some());
        assert!(index
            .get_path(
                std::path::Path::new(".nandi/agents/writer/instructions.md"),
                0
            )
            .is_some());

        repos.agents.delete(&id("writer")).unwrap();
        assert!(!repos.agents.exists(&id("writer")));
        assert!(matches!(
            repos.agents.delete(&id("writer")),
            Err(RepoError::NotFound { .. })
        ));
    }

    #[test]
    fn load_all_reports_bad_folders_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        let dir = &repos.layout().agents_dir;
        for (name, content) in [
            ("good", "---\nname: Good\n---\nHi"),
            ("Bad_Name", "---\nname: Bad\n---\n"),
            ("broken", "---\nname: [oops\n---\n"),
        ] {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("skill.md"), content).unwrap();
        }
        fs::create_dir_all(dir.join("no-skill")).unwrap();

        let loaded = repos.agents.load_all();
        let ids: Vec<_> = loaded.entities.iter().map(|a| a.id().to_string()).collect();
        assert_eq!(ids, ["good"]);
        let mut problems: Vec<_> = loaded
            .problems
            .iter()
            .filter_map(|p| p.id.clone())
            .collect();
        problems.sort();
        assert_eq!(problems, ["Bad_Name", "broken"]);
    }

    #[test]
    fn vendored_agents_load_but_are_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        let dir = repos.layout().vendored_agent_dir("alice", "arxiv");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("skill.md"), "---\nname: ArXiv\n---\nSearch.").unwrap();

        let agent = repos.agents.load(&id("arxiv")).unwrap();
        assert_eq!(agent.name(), "ArXiv");
        assert_eq!(repos.agents.folder_names(), ["arxiv"]);
        assert!(matches!(
            repos.agents.save(&agent),
            Err(RepoError::ReadOnly { .. })
        ));
        assert!(matches!(
            repos.agents.delete(&id("arxiv")),
            Err(RepoError::ReadOnly { .. })
        ));
        // A new local agent can't take a vendored ID, or the reserved one
        assert!(matches!(
            repos.agents.create(&agent),
            Err(RepoError::AlreadyExists { .. })
        ));
        let reserved = Agent::new(AgentDraft::named(id("marketplace"), "M")).unwrap();
        assert!(repos.agents.create(&reserved).is_err());
    }

    #[test]
    fn an_installed_agents_rules_are_the_users_and_survive_an_update() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        let dir = repos.layout().vendored_agent_dir("alice", "arxiv");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("skill.md"), "---\nname: ArXiv\n---\nSearch.").unwrap();
        fs::write(dir.join("instructions.md"), "Shipped rules").unwrap();
        assert_eq!(
            repos.agents.load(&id("arxiv")).unwrap().instructions(),
            "Shipped rules"
        );
        assert!(!repos.agents.has_instructions_override("arxiv"));

        // Saved as an override; the installed folder is untouched, and the
        // agent's fingerprint changes so the running app reloads it
        let before = repos.fingerprint(EntityKind::Agent, "arxiv").unwrap().hash;
        repos
            .agents
            .save_instructions("arxiv", "  My rules  ")
            .unwrap();
        assert_ne!(
            repos.fingerprint(EntityKind::Agent, "arxiv").unwrap().hash,
            before
        );
        let path = repos.layout().instructions_override_path("arxiv");
        assert_eq!(fs::read_to_string(&path).unwrap(), "My rules\n");
        assert_eq!(
            fs::read_to_string(dir.join("instructions.md")).unwrap(),
            "Shipped rules"
        );
        assert_eq!(
            repos.agents.load(&id("arxiv")).unwrap().instructions(),
            "My rules"
        );
        assert!(repos.agents.has_instructions_override("arxiv"));

        // An update rewrites the installed folder; the user's rules stay
        fs::write(
            dir.join("skill.md"),
            "---\nname: ArXiv 2\n---\nSearch better.",
        )
        .unwrap();
        fs::write(dir.join("instructions.md"), "New shipped rules").unwrap();
        let agent = repos.agents.load(&id("arxiv")).unwrap();
        assert_eq!(
            (agent.name(), agent.instructions()),
            ("ArXiv 2", "My rules")
        );

        // Reset: back to what ships
        assert!(repos.agents.reset_instructions("arxiv").unwrap());
        assert_eq!(
            repos.agents.load(&id("arxiv")).unwrap().instructions(),
            "New shipped rules"
        );
        assert!(!repos.agents.reset_instructions("arxiv").unwrap());
    }

    #[test]
    fn a_local_agents_rules_go_in_its_own_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        let agent = Agent::new(AgentDraft::named(id("a"), "A")).unwrap();
        repos.agents.create(&agent).unwrap();
        repos.agents.save_instructions("a", "Be brief").unwrap();
        assert_eq!(
            fs::read_to_string(repos.layout().instructions_path("a")).unwrap(),
            "Be brief\n"
        );
        assert!(!repos.layout().instructions_override_path("a").exists());
        assert!(!repos.agents.has_instructions_override("a"));
        // Empty removes the file
        repos.agents.save_instructions("a", " ").unwrap();
        assert!(!repos.layout().instructions_path("a").exists());
        assert!(repos.agents.save_instructions("nope", "x").is_err());
    }

    #[test]
    fn saving_unchanged_content_leaves_files_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        let agent = Agent::new(AgentDraft::named(id("a"), "A")).unwrap();
        repos.agents.create(&agent).unwrap();
        let path = repos.layout().skill_path("a");
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        repos.agents.save(&agent).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert!(!repos.layout().instructions_path("a").exists());
    }
}
