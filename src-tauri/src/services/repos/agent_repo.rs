//! Agents: `<agents_dir>/<id>/skill.md` and the optional `instructions.md`.
//!
//! Marketplace agents (`<agents_dir>/marketplace/…`, see `agent_locator`)
//! load like any other but are read-only here: saving or deleting one is
//! `RepoError::ReadOnly`. The installer (`VendorRepository`) owns them.
//! Their `instructions.md` is the user's to change all the same: an
//! override in `.nandi/overrides/<id>/instructions.md` replaces the shipped
//! one, and updates never touch it (`save_instructions`).
//!
//! An agent's playbooks are `skills/<name>.md` in its folder
//! (`load_playbooks`, `save_playbook`, `delete_playbook`); an installed
//! agent's are read-only like its skill.md.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::agent_locator::{self, AgentPlace};
use super::fs_atomic::{
    file_times, move_to_trash, remove_if_exists, write_if_changed, TEMP_MARKER,
};
use super::git::GitIndex;
use super::layout::{INSTRUCTIONS_FILE, MARKETPLACE_DIR, MEMORY_FILE, PLAYBOOKS_DIR, SKILL_FILE};
use super::{read_text, Loaded, RepoError, RepoLayout, RepoResult, Repository};
use crate::services::domain::playbook::{
    check_unique, is_valid_name as is_valid_playbook_name, Playbook,
};
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

    /// The agent's `memory.md`, if it has one
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

    /// The agent's folder, or `NotFound`
    fn place(&self, id: &str) -> RepoResult<AgentPlace> {
        self.locate(id).ok_or_else(|| RepoError::NotFound {
            kind: EntityKind::Agent,
            id: id.to_string(),
        })
    }

    /// Agent `id`'s `skills/` folder (an installed agent's ships with it)
    pub fn playbooks_dir(&self, id: &str) -> RepoResult<PathBuf> {
        Ok(self.place(id)?.dir.join(PLAYBOOKS_DIR))
    }

    /// Agent `id`'s playbooks (`skills/*.md`), by name, and why the broken
    /// ones don't load. No folder: none.
    pub fn load_playbooks(&self, id: &str) -> RepoResult<Loaded<Playbook>> {
        let dir = self.playbooks_dir(id)?;
        let mut loaded = Loaded::default();
        for (stem, path) in playbook_files(&dir) {
            let parsed = fs::read_to_string(&path)
                .map_err(|e| RepoError::io(&path, e))
                .and_then(|text| Ok(Playbook::from_md(&stem, &text)?));
            match parsed {
                Ok(playbook) => loaded.entities.push(playbook),
                Err(e) => loaded.problems.push(Problem {
                    kind: EntityKind::Agent,
                    id: Some(id.to_string()),
                    source_path: Some(path),
                    error: match e {
                        RepoError::Domain(error) => error,
                        other => DomainError::Rule(other.to_string()),
                    },
                    severity: Severity::Error,
                }),
            }
        }
        Ok(loaded)
    }

    /// Writes `playbook` into agent `id`'s `skills/`. `previous` is the name
    /// it had (`None`: a new one); when it differs, the playbook is renamed
    /// and the old file goes. A name another playbook has is refused.
    /// Installed agents' playbooks are read-only.
    pub fn save_playbook(
        &self,
        id: &str,
        playbook: &Playbook,
        previous: Option<&str>,
    ) -> RepoResult<PathBuf> {
        self.ensure_writable(id)?;
        let dir = self.playbooks_dir(id)?;
        let path = dir.join(format!("{}.md", playbook.name()));
        let previous = previous.map(str::trim);
        let loaded = self.load_playbooks(id)?;
        let mut all: Vec<Playbook> = loaded
            .entities
            .into_iter()
            .filter(|p| Some(p.name()) != previous)
            .collect();
        all.push(playbook.clone());
        check_unique(&all)?;
        // A broken file under the new name is still someone's text
        if previous != Some(playbook.name()) && path.exists() {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "A playbook named '{}' already exists: choose another name",
                playbook.name()
            ))));
        }
        fs::create_dir_all(&dir).map_err(|e| RepoError::io(&dir, e))?;
        if write_if_changed(&path, playbook.to_md().as_bytes())
            .map_err(|e| RepoError::io(&path, e))?
        {
            self.git.stage_add(&path);
        }
        if let Some(previous) = previous.filter(|name| *name != playbook.name()) {
            self.delete_playbook(id, previous)?;
        }
        Ok(path)
    }

    /// Removes agent `id`'s playbook `name`. The empty `skills/` folder
    /// stays, so a built-in agent's playbooks aren't seeded again.
    pub fn delete_playbook(&self, id: &str, name: &str) -> RepoResult<()> {
        self.ensure_writable(id)?;
        let name = name.trim();
        if !is_valid_playbook_name(name) {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "'{name}' isn't a playbook name"
            ))));
        }
        let path = self.playbooks_dir(id)?.join(format!("{name}.md"));
        if !remove_if_exists(&path).map_err(|e| RepoError::io(&path, e))? {
            return Err(RepoError::Domain(DomainError::Rule(format!(
                "Agent '{id}' has no playbook named '{name}'"
            ))));
        }
        self.git.stage_remove(&path);
        Ok(())
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

/// `(stem, path)` of each `*.md` directly in `dir`, by name; none if it
/// doesn't exist
pub(super) fn playbook_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let stem = path.file_name()?.to_str()?.strip_suffix(".md")?.to_string();
            (!stem.starts_with('.') && !stem.contains(TEMP_MARKER)).then_some((stem, path))
        })
        .collect();
    files.sort();
    files
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

    fn playbook(name: &str, body: &str) -> Playbook {
        Playbook::new(name, "Does a thing", "The user asks", body).unwrap()
    }

    #[test]
    fn playbooks_are_written_read_renamed_and_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        repos
            .agents
            .create(&Agent::new(AgentDraft::named(id("a"), "A")).unwrap())
            .unwrap();
        assert!(repos
            .agents
            .load_playbooks("a")
            .unwrap()
            .entities
            .is_empty());

        let path = repos
            .agents
            .save_playbook("a", &playbook("web-task", "Plan."), None)
            .unwrap();
        assert!(path.ends_with(".nandi/agents/a/skills/web-task.md"));
        repos
            .agents
            .save_playbook("a", &playbook("pay-a-bill", "Pay."), None)
            .unwrap();
        let names = |repos: &Repos| -> Vec<String> {
            repos
                .agents
                .load_playbooks("a")
                .unwrap()
                .entities
                .iter()
                .map(|p| p.name().to_string())
                .collect()
        };
        assert_eq!(names(&repos), ["pay-a-bill", "web-task"]);

        // A new one can't take a name; an edit keeps it
        let taken = repos
            .agents
            .save_playbook("a", &playbook("web-task", "Other."), None)
            .unwrap_err();
        assert!(taken.to_string().contains("web-task"), "{taken}");
        repos
            .agents
            .save_playbook("a", &playbook("web-task", "Edited."), Some("web-task"))
            .unwrap();
        let loaded = repos.agents.load_playbooks("a").unwrap().entities;
        assert_eq!(loaded[1].body(), "Edited.");

        // Renaming moves the file, but never onto another playbook
        assert!(repos
            .agents
            .save_playbook("a", &playbook("pay-a-bill", "x"), Some("web-task"))
            .is_err());
        repos
            .agents
            .save_playbook("a", &playbook("browse", "Edited."), Some("web-task"))
            .unwrap();
        assert_eq!(names(&repos), ["browse", "pay-a-bill"]);

        repos.agents.delete_playbook("a", "browse").unwrap();
        assert_eq!(names(&repos), ["pay-a-bill"]);
        assert!(repos.agents.delete_playbook("a", "browse").is_err());
        assert!(repos.agents.delete_playbook("a", "../skill").is_err());
        assert!(repos.agents.load_playbooks("nope").is_err());
    }

    #[test]
    fn a_broken_playbook_is_a_problem_not_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        repos
            .agents
            .create(&Agent::new(AgentDraft::named(id("a"), "A")).unwrap())
            .unwrap();
        let dir = repos.agents.playbooks_dir("a").unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("good.md"), playbook("good", "Do.").to_md()).unwrap();
        fs::write(
            dir.join("bad.md"),
            "---
name: bad
description: d
when: w
model: claude:x
---
Do.",
        )
        .unwrap();
        fs::write(dir.join("notes.txt"), "not a playbook").unwrap();
        let loaded = repos.agents.load_playbooks("a").unwrap();
        assert_eq!(loaded.entities.len(), 1);
        assert_eq!(loaded.problems.len(), 1);
        assert!(loaded.problems[0].error.to_string().contains("model"));
        // Its name stays taken
        assert!(repos
            .agents
            .save_playbook("a", &playbook("bad", "x"), None)
            .is_err());
    }

    #[test]
    fn an_installed_agents_playbooks_are_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let repos = repos(tmp.path());
        let dir = repos.layout().vendored_agent_dir("alice", "arxiv");
        fs::create_dir_all(dir.join("skills")).unwrap();
        fs::write(
            dir.join("skill.md"),
            "---
name: ArXiv
---
Search.",
        )
        .unwrap();
        fs::write(
            dir.join("skills/search.md"),
            playbook("search", "Search.").to_md(),
        )
        .unwrap();
        assert_eq!(
            repos.agents.load_playbooks("arxiv").unwrap().entities.len(),
            1
        );
        assert!(matches!(
            repos
                .agents
                .save_playbook("arxiv", &playbook("x", "y"), None),
            Err(RepoError::ReadOnly { .. })
        ));
        assert!(matches!(
            repos.agents.delete_playbook("arxiv", "search"),
            Err(RepoError::ReadOnly { .. })
        ));
    }
}
