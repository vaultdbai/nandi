//! Agents: discovery, status, and lifecycle (create / start / stop /
//! restart / delete).
//!
//! An agent is a folder in the gold copy holding a `skill.md` (frontmatter +
//! system prompt) and an optional `instructions.md`; `domain::agent` defines
//! the format and `repos::AgentRepository` reads and writes it. Reads go
//! through the `RepoStore` (the in-memory aggregate). Runtime status comes
//! from runs and Docker: a running container labelled `nandi.agent.id=<id>`
//! means the agent is running.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bollard::query_parameters::ListContainersOptions;
use bollard::Docker;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, Db};
use crate::model::RunStatus;
use crate::runtime;
use crate::services::builtin_agents;
use crate::services::connectors::folders::SharedFolder;
use crate::services::domain::agent::AgentDraft;
use crate::services::domain::repo::Problem;
use crate::services::domain::{self, AgentId, ChangedBy, DomainError, EntityKind, ModelRef};
use crate::services::egress::capabilities::Capabilities;
use crate::services::repo_store::RepoStore;
use crate::services::repos::agent_locator::{self, AgentPlace, Origin};
use crate::services::repos::Repository;
use crate::services::AppState;

/// Docker label that links a container to an agent folder name.
pub(crate) const AGENT_ID_LABEL: &str = "nandi.agent.id";
/// The Nandi that started a container (`nandi_config::instance_tag`), so
/// another one on the machine doesn't count it as its own
pub(crate) const INSTANCE_LABEL: &str = "nandi.instance";
pub(crate) const SKILL_FILE: &str = "skill.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AgentStatus {
    Running,
    Idle,
    WaitingApproval,
    Error,
}

/// An agent discovered on disk, as shown in the Agent Console.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    /// Folder name under the agents folder
    pub id: String,
    pub name: String,
    pub description: String,
    /// Absolute path to the agent's skill.md
    pub skill_path: String,
    pub status: AgentStatus,
    pub current_task: Option<String>,
    /// Whose folder it is: the user's own, or a read-only marketplace one
    pub origin: AgentOrigin,
    /// Ships with Nandi (`services::builtin_agents`): listed first, and
    /// can't be deleted
    pub builtin: bool,
}

/// Where an agent came from. Marketplace agents are read-only until forked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentOrigin {
    Local,
    /// Installed individually from a publisher's repo
    Publisher {
        publisher: String,
    },
    /// Installed with a pack
    Pack {
        pack: String,
    },
}

impl From<Origin> for AgentOrigin {
    fn from(origin: Origin) -> Self {
        match origin {
            Origin::Local => Self::Local,
            Origin::Publisher { publisher } => Self::Publisher { publisher },
            Origin::Pack { pack } => Self::Pack { pack },
        }
    }
}

/// A built-in agent's own folder (not a pack copy with the same ID)
fn is_builtin(id: &str, origin: &AgentOrigin) -> bool {
    *origin == AgentOrigin::Local && builtin_agents::is_builtin(id)
}

/// What the orchestrator needs from an agent's files.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillConfig {
    pub id: String,
    /// Absolute path to the agent folder
    pub dir: PathBuf,
    pub model: String,
    /// Container image; `None` uses the default runtime image
    pub image: Option<String>,
    /// skill.md `memory_mb`; `None` uses `runtime::DEFAULT_MEMORY_MB`
    pub memory_mb: Option<u32>,
    /// skill.md `cpu` (cores); `None` uses `runtime::DEFAULT_CPU`
    pub cpu: Option<f32>,
    /// skill.md `shell_timeout_s`; `None` leaves the runtime's 30s
    pub shell_timeout_s: Option<u32>,
    /// skill.md without frontmatter, plus `instructions.md` if present —
    /// the agent's own part of the system prompt
    pub instructions: String,
    pub allowed_tools: Option<Vec<String>>,
    pub requires_approval: Vec<String>,
    /// What the egress proxy lets the agent reach
    pub capabilities: Capabilities,
    /// Shared folders mounted into its container at
    /// `/agent/folders/<id>` (read-only unless `write`)
    pub folders: Vec<SharedFolder>,
    /// Tools a policy denies, whatever `allowed_tools` says
    pub denied_tools: Vec<String>,
    /// The connectors it may use (`connectors:` in skill.md, narrowed)
    pub connectors: Vec<String>,
    /// Of those, the ones a policy lets run on the host rather than in a
    /// container (`connectors.run_on_host`)
    pub connectors_on_host: Vec<String>,
    /// The pilots it may use (`pilots:`, narrowed)
    pub pilots: Vec<String>,
    /// The policies that applied (`services::policies`)
    pub policies: Vec<String>,
    /// Every applying `models.allow` with its policy; a coding agent's CLI
    /// model is checked against them when its run starts
    pub model_rules: Vec<(Vec<String>, String)>,
}

impl SkillConfig {
    pub fn from_agent(agent: &domain::Agent, dir: PathBuf, default_model: &ModelRef) -> Self {
        let tools = agent.capabilities().tools();
        Self {
            id: agent.id().to_string(),
            dir,
            model: agent.effective_model(default_model).to_string(),
            image: agent.container().image().map(str::to_string),
            memory_mb: agent.container().memory_mb(),
            cpu: agent.container().cpu(),
            shell_timeout_s: agent.container().shell_timeout_s(),
            instructions: agent.system_prompt(),
            allowed_tools: tools.allowed().map(<[String]>::to_vec),
            requires_approval: tools.approval_list(),
            capabilities: Capabilities::from(agent.capabilities()),
            folders: Vec::new(),
            denied_tools: Vec::new(),
            connectors: crate::services::connectors::declared_connectors(agent),
            connectors_on_host: Vec::new(),
            pilots: agent
                .capabilities()
                .pilots()
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            policies: Vec::new(),
            model_rules: Vec::new(),
        }
    }

    pub fn allows_tool(&self, tool: &str) -> bool {
        !self.denied_tools.iter().any(|t| t == tool)
            && self
                .allowed_tools
                .as_ref()
                .is_none_or(|allowed| allowed.iter().any(|t| t == tool))
    }

    pub fn needs_approval(&self, tool: &str) -> bool {
        self.requires_approval.iter().any(|t| t == tool)
    }
}

/// Rejects IDs that aren't valid agent IDs (lowercase letters, digits, and
/// hyphens). Agent IDs arrive from the frontend and become folder names.
pub fn validate_agent_id(id: &str) -> Result<(), String> {
    AgentId::parse(id)
        .map(|_| ())
        .map_err(|_| format!("Invalid agent id '{id}'"))
}

/// An agent's error in the words the Agent Console has always used.
pub fn describe_error(id: &str, error: &DomainError) -> String {
    match error {
        DomainError::Parse { reason, .. } => {
            format!("Invalid {SKILL_FILE} frontmatter for agent '{id}': {reason}")
        }
        DomainError::Invalid { entity, .. } if entity == "model" => {
            format!("Agent '{id}' has an invalid model in {SKILL_FILE}: {error}")
        }
        DomainError::Invalid { entity, .. } if entity == "capabilities" => {
            format!("Agent '{id}' has invalid capabilities in {SKILL_FILE}: {error}")
        }
        other => other.to_string(),
    }
}

impl RepoStore {
    /// The orchestrator config for agent `id`, re-checked against its files
    /// on every call so edits apply to the next model call, and narrowed by
    /// the policies in `.nandi/policies/` (`services::policies`). A broken
    /// policy that may cover it, or a model a policy refuses, is an error.
    pub fn skill(&self, id: &str) -> Result<SkillConfig, String> {
        let (agent, skill) = self.declared_skill(id)?;
        crate::services::policies::narrow(&agent, skill, &self.policies()?)
    }

    /// The policies in `.nandi/policies/`, broken ones included
    pub fn policies(&self) -> Result<crate::services::domain::policy::PolicySet, String> {
        self.repos()
            .policies
            .load_all()
            .map_err(|e| format!("Couldn't read the policies: {e}"))
    }

    /// Agent `id` and its config as its own files say, before policies
    pub fn declared_skill(&self, id: &str) -> Result<(domain::Agent, SkillConfig), String> {
        let agent = self.agent(id).map_err(|e| match AgentId::parse(id) {
            Err(_) => format!("Invalid agent id '{id}'"),
            Ok(_) => e,
        })?;
        let (default_model, manifest) = self.read(|repo| {
            (
                repo.settings().default_model().clone(),
                repo.connectors().clone(),
            )
        });
        let skill = SkillConfig {
            folders: crate::services::connectors::agent_folders(&agent, &manifest),
            ..SkillConfig::from_agent(&agent, self.repos().agents.dir(id), &default_model)
        };
        Ok((agent, skill))
    }

    /// Every agent folder (re-scanned), valid or not, sorted by name. Broken
    /// ones are `Error`, with why in the description.
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        // Without the watcher, files may have changed unseen
        if !self.is_watching() {
            self.rescan(EntityKind::Agent, ChangedBy::Watcher);
        }
        self.agent_infos()
    }

    /// One agent (re-checked against its files), if it has a folder.
    pub fn agent_info(&self, id: &str) -> Option<AgentInfo> {
        AgentId::parse(id).ok()?;
        self.reload(EntityKind::Agent, id, ChangedBy::Watcher);
        self.agent_infos().into_iter().find(|a| a.id == id)
    }

    pub(crate) fn agent_infos(&self) -> Vec<AgentInfo> {
        let places: HashMap<String, AgentPlace> = agent_locator::all(self.layout())
            .into_iter()
            .rev() // the first folder with an ID is the one that loads
            .map(|place| (place.id.clone(), place))
            .collect();
        let skill_path = |id: &str| match places.get(id) {
            Some(place) => place.dir.join(SKILL_FILE),
            None => self.layout().skill_path(id),
        };
        let origin = |id: &str| {
            places
                .get(id)
                .map_or(AgentOrigin::Local, |place| place.origin.clone().into())
        };
        self.read(|repo| {
            let problems: Vec<Problem> = repo
                .problems()
                .into_iter()
                .filter(|p| p.kind == EntityKind::Agent && p.id.is_some())
                .collect();
            let mut agents: Vec<AgentInfo> = repo
                .agents()
                .map(|agent| {
                    let id = agent.id().to_string();
                    let problem = problems
                        .iter()
                        .find(|p| p.id.as_deref() == Some(id.as_str()));
                    AgentInfo {
                        name: agent.name().to_string(),
                        description: match problem {
                            Some(p) => describe_error(&id, &p.error),
                            None => agent.description().to_string(),
                        },
                        skill_path: skill_path(&id).display().to_string(),
                        origin: origin(&id),
                        status: if problem.is_some() {
                            AgentStatus::Error
                        } else {
                            AgentStatus::Idle
                        },
                        current_task: None,
                        builtin: is_builtin(&id, &origin(&id)),
                        id,
                    }
                })
                .collect();
            for problem in &problems {
                let id = problem.id.clone().unwrap_or_default();
                if repo.agent(&id).is_none() {
                    agents.push(AgentInfo {
                        name: id.clone(),
                        description: describe_error(&id, &problem.error),
                        skill_path: skill_path(&id).display().to_string(),
                        origin: origin(&id),
                        status: AgentStatus::Error,
                        current_task: None,
                        builtin: is_builtin(&id, &origin(&id)),
                        id,
                    });
                }
            }
            agents.sort_by_key(|agent| agent.name.to_lowercase());
            agents
        })
    }
}

/// An agent's orchestrator config straight from the repo files, without
/// the app running (`nandi verify`).
pub fn load_skill_from_config(id: &str) -> Result<SkillConfig, String> {
    use crate::services::repos::{Repos, SingletonRepository};
    let repos = Repos::new(crate::services::repo_bootstrap::install_layout()?);
    let agent_id = AgentId::parse(id).map_err(|_| format!("Invalid agent id '{id}'"))?;
    let agent = repos.agents.load(&agent_id).map_err(|e| match e {
        crate::services::repos::RepoError::Domain(error) => describe_error(id, &error),
        other => other.to_string(),
    })?;
    let settings = repos.settings.load().unwrap_or_default();
    Ok(SkillConfig::from_agent(
        &agent,
        repos.agents.dir(id),
        settings.default_model(),
    ))
}

/// Lists all agents, with status from their runs (and Docker).
pub async fn list_agents(state: &AppState) -> Result<Vec<AgentInfo>, String> {
    let mut agents = scan_agents(state.store()).await?;
    apply_run_status(state.db(), &mut agents).await?;
    Ok(agents)
}

/// Overlays persisted state: an active run makes an agent running (or
/// waiting on approval); a crashed orchestrator makes it errored.
pub async fn apply_run_status(db: &Db, agents: &mut [AgentInfo]) -> Result<(), String> {
    let (active, errors) = db
        .call(|conn| {
            Ok((
                crate::db::active_runs(conn)?,
                crate::db::agents_in_error(conn)?,
            ))
        })
        .await?;
    for agent in agents.iter_mut() {
        if let Some((_, error)) = errors.iter().find(|(id, _)| *id == agent.id) {
            agent.status = AgentStatus::Error;
            if let Some(error) = error {
                agent.description = error.clone();
            }
        } else if let Some(run) = active.iter().find(|run| run.agent_id == agent.id) {
            agent.status = match run.status {
                RunStatus::AwaitingApproval => AgentStatus::WaitingApproval,
                _ => AgentStatus::Running,
            };
            agent.current_task = Some(run.input.clone());
        }
    }
    Ok(())
}

/// Re-scans the agents and resolves each one's status against running
/// Docker containers.
pub async fn scan_agents(store: &RepoStore) -> Result<Vec<AgentInfo>, String> {
    let scan = store.clone();
    let mut agents = tokio::task::spawn_blocking(move || scan.list_agents())
        .await
        .map_err(|e| format!("Agent scan task failed: {e}"))?;

    let running = running_agent_ids().await;
    for agent in &mut agents {
        if agent.status == AgentStatus::Idle && running.contains(&agent.id) {
            agent.status = AgentStatus::Running;
        }
    }

    log::debug!("Listed {} agents", agents.len());
    Ok(agents)
}

/// IDs of this Nandi's agents with a running container. Docker being unavailable is not
/// an error — every agent is simply treated as idle.
async fn running_agent_ids() -> HashSet<String> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(e) => {
            log::warn!("Docker unavailable, treating all agents as idle: {e}");
            return HashSet::new();
        }
    };

    let options = ListContainersOptions {
        filters: Some(HashMap::from([(
            "label".to_string(),
            vec![
                AGENT_ID_LABEL.to_string(),
                format!(
                    "{INSTANCE_LABEL}={}",
                    crate::utils::nandi_config::instance_tag()
                ),
            ],
        )])),
        ..Default::default()
    };

    match docker.list_containers(Some(options)).await {
        Ok(containers) => containers
            .into_iter()
            .filter_map(|c| c.labels?.remove(AGENT_ID_LABEL))
            .collect(),
        Err(e) => {
            log::warn!("Failed to list agent containers, treating all agents as idle: {e}");
            HashSet::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentInput {
    /// Folder name: lowercase letters, digits, and hyphens
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// e.g. `claude:claude-sonnet-5`
    #[serde(default)]
    pub model: Option<String>,
    /// The skill.md body (the agent's instructions)
    #[serde(default)]
    pub instructions: Option<String>,
}

const MAX_INSTRUCTIONS_CHARS: usize = 200_000;

/// The domain agent a create request describes (validated).
fn new_agent(input: &CreateAgentInput) -> Result<domain::Agent, String> {
    let mut draft = AgentDraft::named(AgentId::parse(input.id.as_str())?, input.name.as_str());
    draft.description = input.description.clone().unwrap_or_default();
    draft.skill_body = input.instructions.clone().unwrap_or_default();
    draft.model = input
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(ModelRef::parse)
        .transpose()?;
    Ok(domain::Agent::new(draft)?)
}

/// Creates an agent: the domain validates it, the agent repository writes
/// `<agents dir>/<id>/skill.md` (staged in git), and the commit updates the
/// aggregate and projection and publishes `repo://agent-created` (which
/// also starts its orchestrator). Fails if the agent already exists.
pub async fn create_agent(state: &AppState, input: CreateAgentInput) -> Result<AgentInfo, String> {
    if input.name.trim().is_empty() {
        return Err("Agent name is required".to_string());
    }
    if input
        .instructions
        .as_ref()
        .is_some_and(|i| i.chars().count() > MAX_INSTRUCTIONS_CHARS)
    {
        return Err(format!(
            "Instructions are longer than {MAX_INSTRUCTIONS_CHARS} characters"
        ));
    }
    let agent = new_agent(&input)?;
    let store = state.store().clone();
    let id = agent.id().to_string();
    let info = tokio::task::spawn_blocking(move || {
        store.repos().agents.create(&agent)?;
        store.commit(EntityKind::Agent, agent.id().as_str())?;
        Ok::<_, String>(
            store
                .agent_infos()
                .into_iter()
                .find(|a| a.id == agent.id().as_str()),
        )
    })
    .await
    .map_err(|e| format!("Create task failed: {e}"))??
    .ok_or_else(|| format!("Agent '{id}' was created but couldn't be read back"))?;

    // The reactor already did on `repo://agent-created`; idempotent
    state.orchestrators().ensure(&info.id);
    log::info!("Created agent '{}'", info.id);
    Ok(info)
}

/// Starts the agent's runtime container (replacing any previous one).
/// Returns the container ID.
pub async fn start_agent(state: &AppState, agent_id: &str) -> Result<String, String> {
    let skill = state.store().skill(agent_id)?;
    let runtime = state.runtimes().restart(&skill).await?;
    Ok(runtime.container.container_id.clone())
}

/// Stops and removes the agent's runtime container, if any.
pub async fn stop_agent(state: &AppState, agent_id: &str) -> Result<(), String> {
    validate_agent_id(agent_id)?;
    state.runtimes().stop(agent_id).await;
    // Also catch a container left over from an earlier app session
    runtime::stop_container(&runtime::container_name(agent_id)).await;
    Ok(())
}

pub async fn restart_agent(state: &AppState, agent_id: &str) -> Result<String, String> {
    stop_agent(state, agent_id).await?;
    start_agent(state, agent_id).await
}

/// Removes an agent: stops its container and orchestrator, moves its folder
/// to the trash (not deleted), stages the removal, and publishes
/// `repo://agent-deleted`. Run history stays in the DB. Refused while the
/// agent has an active run or queued requests, or while a workflow or
/// trigger still uses it.
pub async fn delete_agent(state: &AppState, agent_id: &str) -> Result<(), String> {
    state.store().require_agent(agent_id)?;
    if builtin_agents::is_builtin(agent_id) {
        return Err(format!(
            "'{agent_id}' comes with Nandi and can't be deleted. Edit it instead; Backup keeps its history in git."
        ));
    }
    state
        .store()
        .read(|repo| repo.check_removable(EntityKind::Agent, agent_id))?;
    if state
        .store()
        .repos()
        .agents
        .locate(agent_id)
        .is_some_and(|place| place.origin.is_vendored())
    {
        return Err(format!(
            "'{agent_id}' is installed from the marketplace. Uninstall it from Library › Installed."
        ));
    }
    retire_agent(state, agent_id).await?;

    let (store, id) = (state.store().clone(), AgentId::parse(agent_id)?);
    tokio::task::spawn_blocking(move || {
        store.repos().agents.delete(&id)?;
        store.commit(EntityKind::Agent, id.as_str())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Delete task failed: {e}"))??;
    log::info!("Deleted agent '{agent_id}' (moved to trash)");
    Ok(())
}

/// Before an agent's folder goes away (delete, uninstall, fork): refuses
/// while it has an active run or queued requests, then stops its container
/// and orchestrator.
pub(crate) async fn retire_agent(state: &AppState, agent_id: &str) -> Result<(), String> {
    let id = agent_id.to_string();
    let (active, queue_empty) = state
        .db()
        .call(move |conn| {
            Ok((
                db::active_run_of_agent(conn, &id)?,
                db::queue_is_empty(conn, &id)?,
            ))
        })
        .await?;
    if active.is_some() {
        return Err("Stop the agent's current run first".to_string());
    }
    if !queue_empty {
        return Err("Clear the agent's queue first".to_string());
    }

    stop_agent(state, agent_id).await?;
    state.orchestrators().remove(agent_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::repos::{RepoLayout, Repos};
    use std::fs;
    use std::path::Path;

    /// The orchestrator config a skill.md (and instructions.md) yields.
    fn parse_skill(
        id: &str,
        dir: PathBuf,
        content: &str,
        extra: Option<&str>,
    ) -> Result<SkillConfig, String> {
        let agent_id = AgentId::parse(id)?;
        let agent = domain::Agent::from_files(agent_id, content, extra)
            .map_err(|e| describe_error(id, &e))?;
        let default = ModelRef::parse(domain::settings::DEFAULT_MODEL).unwrap();
        Ok(SkillConfig::from_agent(&agent, dir, &default))
    }

    fn store(dir: &Path) -> RepoStore {
        RepoStore::sqlite(
            Repos::new(RepoLayout::new(dir, dir.join("trash"))),
            Db::in_memory(),
        )
    }

    fn write_skill(store: &RepoStore, id: &str, content: &str) {
        let path = store.layout().skill_path(id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn creates_missing_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        store.load().unwrap();
        assert!(store.list_agents().is_empty());
        assert!(store.layout().agents_dir.is_dir());
    }

    #[test]
    fn lists_agents_sorted_and_skips_folders_without_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        write_skill(&store, "writer", "---\nname: Writer\n---\n");
        write_skill(&store, "alpha", "No frontmatter");
        fs::create_dir_all(store.layout().agent_dir("no-skill")).unwrap();
        fs::write(store.layout().agents_dir.join("stray-file.md"), "ignored").unwrap();

        let agents = store.list_agents();

        let ids: Vec<_> = agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["alpha", "writer"]);
        // Name falls back to the folder name
        assert_eq!(agents[0].name, "alpha");
        assert_eq!(agents[0].description, "");
        assert_eq!(agents[0].status, AgentStatus::Idle);
        assert!(agents[0].current_task.is_none());
        assert!(agents[1].skill_path.ends_with(SKILL_FILE));
    }

    #[test]
    fn marks_invalid_skills_and_ids_as_error() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        write_skill(&store, "broken", "---\nname: [oops\n---\n");
        write_skill(&store, "Legacy_Id", "---\nname: Legacy\n---\n");

        let agents = store.list_agents();

        let broken = agents.iter().find(|a| a.id == "broken").unwrap();
        assert_eq!(broken.status, AgentStatus::Error);
        assert_eq!(broken.name, "broken");
        assert!(broken.description.contains("Invalid skill.md frontmatter"));
        let legacy = agents.iter().find(|a| a.id == "Legacy_Id").unwrap();
        assert_eq!(legacy.status, AgentStatus::Error);
        assert!(
            legacy.description.contains("Invalid agent id"),
            "{}",
            legacy.description
        );
    }

    #[test]
    fn skill_reads_see_edits_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        write_skill(&store, "a", "---\nname: A\n---\nOne.");
        store.load().unwrap();
        assert_eq!(store.skill("a").unwrap().instructions, "One.");
        write_skill(&store, "a", "---\nname: A\n---\nTwo.");
        assert_eq!(store.skill("a").unwrap().instructions, "Two.");
        assert!(store
            .skill("../x")
            .unwrap_err()
            .contains("Invalid agent id"));
        assert!(store.skill("missing").unwrap_err().contains("not found"));
    }

    #[test]
    fn parses_skill_config() {
        let content = "---\nmodel: claude:claude-sonnet-5\nallowed-tools: [shell, read_file]\nrequires_approval: [shell]\n---\n\n# Researcher\nFind papers.\n";
        let skill = parse_skill("r", PathBuf::from("/a/r"), content, None).unwrap();

        assert_eq!(skill.model, "claude:claude-sonnet-5");
        assert_eq!(skill.instructions, "# Researcher\nFind papers.");
        assert!(skill.allows_tool("read_file"));
        assert!(!skill.allows_tool("write_file"));
        assert!(skill.needs_approval("shell"));
        assert!(!skill.needs_approval("read_file"));
        assert!(skill.image.is_none());
    }

    #[test]
    fn parses_capabilities() {
        let content = r#"---
aws:
  mode: sts
  services:
    - service: s3
      actions: [GetObject]
      resources: ["arn:aws:s3:::b/*"]
network:
  allow:
    - host: API.example.com
      methods: [GET]
---
Body"#;
        let skill = parse_skill("a", PathBuf::from("/a"), content, None).unwrap();
        let caps = &skill.capabilities;

        assert_eq!(
            caps.aws.as_ref().unwrap().mode,
            crate::services::egress::capabilities::AwsMode::Sts
        );
        assert!(caps.allows_aws("s3", "GetObject", "arn:aws:s3:::b/x"));
        assert!(caps.allows_request("api.example.com", None, "GET"));
        assert!(!caps.allows_request("api.example.com", None, "POST"));

        // No blocks: nothing allowed
        let skill = parse_skill("a", PathBuf::from("/a"), "---\nname: A\n---\n", None).unwrap();
        assert_eq!(skill.capabilities, Capabilities::default());

        let bad_host = "---\nnetwork:\n  allow:\n    - host: \"a.*.com\"\n---\n";
        let err = parse_skill("a", PathBuf::from("/a"), bad_host, None).unwrap_err();
        assert!(err.contains("invalid capabilities"), "{err}");
        let bad_mode = "---\naws:\n  mode: root\n---\n";
        let err = parse_skill("a", PathBuf::from("/a"), bad_mode, None).unwrap_err();
        assert!(err.contains("frontmatter"), "{err}");
    }

    #[test]
    fn parses_the_e2e_echo_agent_fixture() {
        let content = include_str!("../../../test-fixtures/echo-agent/skill.md");
        let skill = parse_skill("echo-agent", PathBuf::new(), content, None).unwrap();
        assert_eq!(skill.model, "deepseek:deepseek-flash");
        assert_eq!(
            skill.image.as_deref(),
            Some(crate::runtime::DEFAULT_RUNTIME_IMAGE)
        );
        assert!(skill.allows_tool("shell") && !skill.allows_tool("write_file"));
        assert!(!skill.needs_approval("shell"));
        assert!(skill
            .instructions
            .starts_with("You are a minimal test agent."));
    }

    #[test]
    fn skill_defaults_to_approving_mutating_tools() {
        let content = "---\nmodel: ollama:llama3.2\n---\nBody";
        let skill = parse_skill("a", PathBuf::from("/a"), content, None).unwrap();

        assert!(skill.allows_tool("anything"));
        assert!(skill.needs_approval("shell"));
        assert!(skill.needs_approval("write_file"));
        assert!(!skill.needs_approval("list_dir"));
    }

    #[test]
    fn skill_defaults_to_the_default_model_and_validates_models() {
        let skill = parse_skill("a", PathBuf::from("/a"), "---\nname: A\n---\n", None).unwrap();
        assert_eq!(skill.model, "claude:claude-sonnet-5");

        let err = parse_skill(
            "a",
            PathBuf::from("/a"),
            "---\nmodel: gemini:pro\n---\n",
            None,
        )
        .unwrap_err();
        assert!(err.contains("invalid model"), "{err}");
    }

    #[test]
    fn appends_instructions_md() {
        let skill = parse_skill(
            "a",
            PathBuf::from("/a"),
            "---\nname: A\n---\nBe kind.",
            Some("  Cite sources.\n"),
        )
        .unwrap();
        assert_eq!(skill.instructions, "Be kind.\n\nCite sources.");
    }

    #[test]
    fn renders_a_parseable_skill_file() {
        let input = CreateAgentInput {
            id: "writer".into(),
            name: "Writer: drafts".into(),
            description: Some("  ".into()),
            model: Some("ollama:llama3.2".into()),
            instructions: Some("Write well.".into()),
        };
        let content = new_agent(&input).unwrap().to_skill_md();
        let skill = parse_skill("writer", PathBuf::from("/a"), &content, None).unwrap();

        assert_eq!(skill.model, "ollama:llama3.2");
        assert_eq!(skill.instructions, "Write well.");
        let agent =
            domain::Agent::from_skill_md(AgentId::parse("writer").unwrap(), &content).unwrap();
        assert_eq!(agent.name(), "Writer: drafts");
        assert!(!content.contains("description"), "blank fields are omitted");

        // New agents get strict IDs
        for bad in ["Writer", "code_reviewer", "v1.2"] {
            let input = CreateAgentInput {
                id: bad.into(),
                ..input.clone()
            };
            assert!(new_agent(&input).is_err(), "{bad}");
        }
    }

    #[test]
    fn validates_agent_ids() {
        for ok in ["writer", "code-reviewer-2", "v1"] {
            assert!(validate_agent_id(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "..", ".hidden", "a/b", "a\\b", "C:", "a b", "Writer", "a_b", "v1.2",
        ] {
            assert!(validate_agent_id(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn creating_and_deleting_go_through_the_repo_and_publish() {
        use crate::services::events::tests::RecordingSink;
        let recorder = std::sync::Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let git = git2::Repository::init(&state.store().layout().root).unwrap();
        let input = CreateAgentInput {
            id: "writer".into(),
            name: "Writer".into(),
            description: None,
            model: None,
            instructions: Some("Write.".into()),
        };

        let info = create_agent(&state, input.clone()).await.unwrap();
        assert_eq!(info.name, "Writer");
        assert!(state.store().layout().skill_path("writer").is_file());
        let mut index = git.index().unwrap();
        index.read(true).unwrap();
        assert!(
            index
                .get_path(std::path::Path::new(".nandi/agents/writer/skill.md"), 0)
                .is_some(),
            "the new file is staged"
        );
        let names = recorder.names();
        assert!(names.contains(&"repo://agent-created"), "{names:?}");
        assert!(names.contains(&"agent://agents_changed"), "{names:?}");
        let projected: String = state
            .db()
            .call(|conn| {
                conn.query_row("SELECT name FROM agents WHERE id = 'writer'", [], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(projected, "Writer");
        assert_eq!(
            create_agent(&state, input).await.unwrap_err(),
            "An agent with id 'writer' already exists"
        );

        // A trigger still targets it: refused
        let trigger = crate::services::triggers::schema::TriggerInput {
            id: Some("hook".into()),
            name: "Hook".into(),
            enabled: None,
            source_type: crate::services::triggers::schema::TriggerSourceType::Webhook,
            source_config: serde_json::json!({}),
            target_type: crate::services::triggers::schema::TriggerTargetType::Agent,
            target_id: "writer".into(),
            input_template: None,
            priority: None,
            rate_limit_seconds: None,
        };
        state.triggers().create(trigger).await.unwrap();
        let error = delete_agent(&state, "writer").await.unwrap_err();
        assert!(error.contains("trigger 'hook'"), "{error}");

        state.triggers().delete("hook").await.unwrap();
        delete_agent(&state, "writer").await.unwrap();
        assert!(!state.store().layout().skill_path("writer").exists());
        assert!(recorder.names().contains(&"repo://agent-deleted"));
        assert!(state.store().agent("writer").is_err());
        state.shutdown();
    }

    #[test]
    fn serializes_camel_case() {
        let agent = AgentInfo {
            id: "a".to_string(),
            name: "A".to_string(),
            description: String::new(),
            skill_path: "/a/skill.md".to_string(),
            status: AgentStatus::WaitingApproval,
            current_task: None,
            origin: crate::services::agents::AgentOrigin::Local,
            builtin: false,
        };
        let json = serde_json::to_value(agent).unwrap();

        assert_eq!(json["status"], "waitingApproval");
        assert_eq!(json["skillPath"], "/a/skill.md");
        assert!(json["currentTask"].is_null());
    }
}
