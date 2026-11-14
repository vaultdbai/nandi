//! Repo-wide operations: rebuilding the projection and reporting the
//! repo's state. Shared by the Tauri commands, the HTTP routes, and the
//! `nandi rebuild` CLI.
//!
//! Rebuilding wipes the projected tables and re-reads every file. It's safe
//! at any time: run history, approvals, messages, and other runtime data
//! are untouched. Use it when `state.db` was deleted or damaged, when the
//! projection drifted, or to reset local state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::db::Db;
use crate::services::agents::{describe_error, AgentInfo};
use crate::services::domain::agent::split_frontmatter;
use crate::services::domain::capability::{HostRule, NetworkPolicy};
use crate::services::domain::playbook::Playbook;
use crate::services::domain::repo::Severity;
use crate::services::domain::{Agent, AgentId, ChangedBy, EntityKind, TriggerId, WorkflowId};
use crate::services::events::{self, NandiEvent};
use crate::services::projection::ProjectionStats;
use crate::services::repo_store::RepoStore;
use crate::services::repos::layout::SKILL_FILE;
use crate::services::repos::{RepoError, Repository};
use crate::services::AppState;

/// A rebuild started (`repo://rebuild_started`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoRebuildStarted {
    pub force: bool,
}

impl NandiEvent for RepoRebuildStarted {
    const NAME: &'static str = "repo://rebuild_started";
}

/// A rebuild finished (`repo://rebuild_completed`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoRebuildCompleted {
    pub stats: ProjectionStats,
}

impl NandiEvent for RepoRebuildCompleted {
    const NAME: &'static str = "repo://rebuild_completed";
}

/// A file that doesn't load, or loads with a broken reference.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoProblem {
    pub kind: String,
    pub id: Option<String>,
    pub source_path: Option<String>,
    pub error: String,
    /// Errors keep the file out; warnings don't
    pub is_error: bool,
}

/// What the repo looks like right now.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub root: String,
    pub agents: u32,
    pub workflows: u32,
    pub triggers: u32,
    pub problems: Vec<RepoProblem>,
    /// Edits to the files apply live
    pub watching: bool,
}

// --- Agent editor and git ---

/// Everything the agent editor saves. Optional fields fall back to the
/// defaults in skill.md (`memory_mb` 1024, `cpu` 1.0, the default model).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The skill.md body (below the frontmatter)
    #[serde(default)]
    pub skill_body: String,
    /// `instructions.md`, if any
    #[serde(default)]
    pub instructions: String,
    /// Routing prefix, e.g. `claude` (see `reserved-conventions.md`)
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub cpu: Option<f32>,
    #[serde(default)]
    pub capabilities: AgentDefinitionCapabilities,
    /// `native` (or `None`), `claude-code`, `codex`, `gemini-cli`,
    /// `custom:<id>`
    #[serde(default)]
    pub runtime: Option<String>,
    /// The harness's model (`runtime_config.model`); the rest of
    /// `runtime_config` is kept as it is
    #[serde(default)]
    pub runtime_model: Option<String>,
    /// The AWS profile the agent's AWS calls run as (`aws.profile`);
    /// `None`: Settings' access key, else the sign-in profile
    #[serde(default)]
    pub aws_profile: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionCapabilities {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub pilots: Vec<String>,
}

/// `git status` of `~/.nandi/repo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    /// Relative to the repo root, `/`-separated
    pub path: String,
    /// `added` | `modified` | `deleted` | `renamed` | `untracked`
    pub status: String,
}

/// Status, diff, and commit for the repo at the layout root. Staging of
/// single writes stays with `repos::git::GitIndex`; pushing stays with
/// "Sync now" (`auth::repo_sync`). Nothing here talks to a remote.
///
/// Only the repo at the root is opened, never one found further up, so an
/// unrelated repo around `~/.nandi` is left alone.
#[derive(Debug, Clone)]
pub struct GitService {
    root: PathBuf,
}

impl GitService {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn open(&self) -> Result<git2::Repository, String> {
        git2::Repository::open(&self.root).map_err(|_| {
            format!(
                "{} isn't a git repository yet. Choose an agents repo to start tracking changes.",
                self.root.display()
            )
        })
    }

    /// Branch, commits ahead of/behind its upstream, and every changed file
    /// (staged or not, untracked included; ignored files left out).
    pub fn status(&self) -> Result<GitStatus, String> {
        let repo = self.open()?;
        let (ahead, behind) = ahead_behind(&repo).unwrap_or((0, 0));
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .renames_head_to_index(true);
        let statuses = repo.statuses(Some(&mut options)).map_err(git_error)?;
        let mut files: Vec<GitFile> = statuses
            .iter()
            .filter_map(|entry| {
                let status = file_status(entry.status())?;
                let path = entry
                    .head_to_index()
                    .and_then(|d| d.new_file().path().map(PathBuf::from))
                    .or_else(|| entry.path().map(PathBuf::from))?;
                Some(GitFile {
                    path: slash_path(&path),
                    status: status.to_string(),
                })
            })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup_by(|a, b| a.path == b.path);
        Ok(GitStatus {
            branch: branch_name(&repo),
            ahead,
            behind,
            files,
        })
    }

    /// Unified diff of one file, `HEAD` against the working tree (staged
    /// and unstaged changes together). Empty if the file is unchanged.
    pub fn diff(&self, path: &str) -> Result<String, String> {
        let path = repo_relative(path)?;
        let repo = self.open()?;
        let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
        let mut options = git2::DiffOptions::new();
        options
            .pathspec(&path)
            .disable_pathspec_match(true)
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        let diff = repo
            .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut options))
            .map_err(git_error)?;
        let mut patch = String::new();
        diff.print(git2::DiffFormat::Patch, |_, _, line| {
            if matches!(line.origin(), '+' | '-' | ' ') {
                patch.push(line.origin());
            }
            patch.push_str(&String::from_utf8_lossy(line.content()));
            true
        })
        .map_err(git_error)?;
        Ok(patch)
    }

    /// Stages everything (`git add -A`, `.gitignore` respected) and commits
    /// it on the current branch. Returns the new commit's full hash. Never
    /// pushes.
    pub fn commit(&self, message: &str, owner: Option<&str>) -> Result<String, String> {
        let message = message.trim();
        if message.is_empty() {
            return Err("Write a commit message first".to_string());
        }
        let repo = self.open()?;
        let mut index = repo.index().map_err(git_error)?;
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .map_err(git_error)?;
        index.update_all(["*"], None).map_err(git_error)?;
        index.write().map_err(git_error)?;
        let tree_id = index.write_tree().map_err(git_error)?;
        let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let unchanged = match &parent {
            Some(parent) => parent.tree_id() == tree_id,
            None => index.is_empty(),
        };
        if unchanged {
            return Err("Nothing to commit: the repo has no changes".to_string());
        }
        let tree = repo.find_tree(tree_id).map_err(git_error)?;
        let signature =
            crate::services::repo_bootstrap::signature(&repo, owner).map_err(git_error)?;
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        let oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .map_err(git_error)?;
        log::info!("Committed {oid} in {}", self.root.display());
        Ok(oid.to_string())
    }

    /// Commits only `paths` (repo-relative, as they are on disk) on top of
    /// `HEAD`, leaving anything else staged or changed out of the commit.
    /// `None` when those files already match `HEAD`. Never pushes.
    pub fn commit_files(
        &self,
        paths: &[String],
        message: &str,
        owner: Option<&str>,
    ) -> Result<Option<String>, String> {
        let message = message.trim();
        if message.is_empty() {
            return Err("Write a commit message first".to_string());
        }
        let repo = self.open()?;
        let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        // HEAD's tree in memory, plus the files
        let mut index = git2::Index::new().map_err(git_error)?;
        if let Some(parent) = &parent {
            index
                .read_tree(&parent.tree().map_err(git_error)?)
                .map_err(git_error)?;
        }
        for path in paths {
            let path = repo_relative(path)?;
            match std::fs::read(self.root.join(&path)) {
                Ok(content) => {
                    let blob = repo.blob(&content).map_err(git_error)?;
                    let entry = git2::IndexEntry {
                        ctime: git2::IndexTime::new(0, 0),
                        mtime: git2::IndexTime::new(0, 0),
                        dev: 0,
                        ino: 0,
                        mode: 0o100644,
                        uid: 0,
                        gid: 0,
                        file_size: content.len() as u32,
                        id: blob,
                        flags: path.len().min(0xfff) as u16,
                        flags_extended: 0,
                        path: path.clone().into_bytes(),
                    };
                    index.add(&entry).map_err(git_error)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let _ = index.remove_path(Path::new(&path));
                }
                Err(e) => return Err(format!("{path}: {e}")),
            }
        }
        let tree_id = index.write_tree_to(&repo).map_err(git_error)?;
        if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
            return Ok(None);
        }
        let tree = repo.find_tree(tree_id).map_err(git_error)?;
        let signature =
            crate::services::repo_bootstrap::signature(&repo, owner).map_err(git_error)?;
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        let oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .map_err(git_error)?;
        // Keep the real index in step for those files
        if let Ok(mut real) = repo.index() {
            for path in paths {
                let path = Path::new(path);
                let staged = if self.root.join(path).exists() {
                    real.add_path(path)
                } else {
                    real.remove_path(path)
                };
                if staged.is_err() {
                    log::warn!("Couldn't stage {} after committing it", path.display());
                }
            }
            let _ = real.write();
        }
        log::info!(
            "Committed {oid} ({}) in {}",
            paths.join(", "),
            self.root.display()
        );
        Ok(Some(oid.to_string()))
    }
}

fn git_error(error: git2::Error) -> String {
    format!("git: {}", error.message())
}

/// `refs/heads/main` → `main`, also before the first commit; `HEAD` when
/// detached.
fn branch_name(repo: &git2::Repository) -> String {
    if let Ok(head) = repo.head() {
        if head.is_branch() {
            if let Some(name) = head.shorthand() {
                return name.to_string();
            }
        }
        return "HEAD".to_string();
    }
    // Unborn: HEAD points at a branch that has no commits yet
    repo.find_reference("HEAD")
        .ok()
        .and_then(|head| head.symbolic_target().map(str::to_string))
        .map(|target| {
            target
                .strip_prefix("refs/heads/")
                .unwrap_or(&target)
                .to_string()
        })
        .unwrap_or_else(|| "HEAD".to_string())
}

/// Commits ahead of and behind the branch's upstream; `None` without one.
fn ahead_behind(repo: &git2::Repository) -> Option<(u32, u32)> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let local = head.target()?;
    let upstream = git2::Branch::wrap(head).upstream().ok()?.get().target()?;
    let (ahead, behind) = repo.graph_ahead_behind(local, upstream).ok()?;
    Some((ahead as u32, behind as u32))
}

/// The status `GitFile` reports, or `None` for files to leave out.
fn file_status(status: git2::Status) -> Option<&'static str> {
    use git2::Status as S;
    if status.is_ignored() || status.is_empty() {
        return None;
    }
    Some(if status.contains(S::INDEX_NEW) {
        "added"
    } else if status.intersects(S::INDEX_DELETED | S::WT_DELETED) {
        "deleted"
    } else if status.intersects(S::INDEX_RENAMED | S::WT_RENAMED) {
        "renamed"
    } else if status.contains(S::WT_NEW) {
        "untracked"
    } else {
        "modified"
    })
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A path relative to the repo root that can't climb out of it.
fn repo_relative(path: &str) -> Result<String, String> {
    let path = path.trim().replace('\\', "/");
    let invalid = path.is_empty()
        || path.starts_with('/')
        || path.contains(':')
        || path.split('/').any(|part| part == ".." || part.is_empty());
    if invalid {
        return Err(format!("'{path}' isn't a file path inside the repo"));
    }
    Ok(path)
}

// --- Saving and loading agent definitions -----------------------------------

/// The skill.md frontmatter keys the editor writes. Everything else
/// (`requires_approval`, `aws` but its `profile`, unknown keys) is kept as
/// it is.
const MODEL_KEY: &str = "model";
const TOOLS_KEY: &str = "allowed-tools";
/// Spellings `Frontmatter` also reads; dropped so the edited value wins
const ALIASES: [&str; 2] = ["allowed_tools", "memory-mb"];

/// The agent the editor's input describes, on top of what its skill.md
/// already holds (`existing`; `None` for a new agent). The domain validates
/// the result, so an invalid input never reaches a file.
pub fn definition_agent(
    id: &AgentId,
    existing: Option<&str>,
    input: &AgentDefinitionInput,
) -> Result<Agent, String> {
    use serde_yaml::{Mapping, Value};

    // skill.md without a name falls back to the ID; the editor must name it
    if input.name.trim().is_empty() {
        return Err("Agent name is required".to_string());
    }
    let mut frontmatter = match existing.map(split_frontmatter).transpose()? {
        Some((Some(yaml), _)) if !yaml.trim().is_empty() => serde_yaml::from_str::<Mapping>(yaml)
            .map_err(|e| {
            format!(
                "skill.md for agent '{id}' isn't valid YAML; fix it in a text editor first: {e}"
            )
        })?,
        _ => Mapping::new(),
    };
    let previous_rules: Vec<HostRule> = frontmatter
        .get("network")
        .and_then(|v| serde_yaml::from_value::<NetworkPolicy>(v.clone()).ok())
        .map(|policy| policy.allow)
        .unwrap_or_default();
    // A pilot listed with options (`{id: browser, allow: [...]}`) keeps them
    let previous_pilots: Vec<Value> = frontmatter
        .get("pilots")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let previous_runtime_config = frontmatter
        .get("runtime_config")
        .or_else(|| frontmatter.get("runtime-config"))
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    frontmatter.remove("runtime-config");
    for alias in ALIASES {
        frontmatter.remove(alias);
    }
    let mut put = |key: &str, value: Option<Value>| match value {
        Some(value) => {
            frontmatter.insert(Value::from(key), value);
        }
        None => {
            frontmatter.remove(key);
        }
    };
    let text = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(Value::from);
    let list = |items: &[String]| {
        let items: Vec<Value> = items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(Value::from)
            .collect();
        (!items.is_empty()).then_some(Value::Sequence(items))
    };

    put("name", Some(Value::from(input.name.trim())));
    put("description", text(&input.description));
    put(MODEL_KEY, model_value(input)?.map(Value::from));
    put("image", input.image.as_deref().and_then(text));
    put("memory_mb", input.memory_mb.map(Value::from));
    put("cpu", input.cpu.map(|cpu| Value::from(f64::from(cpu))));
    put(TOOLS_KEY, list(&input.capabilities.tools));
    let pilots: Vec<Value> = input
        .capabilities
        .pilots
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|id| {
            previous_pilots
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
                .cloned()
                .unwrap_or_else(|| Value::from(id))
        })
        .collect();
    put(
        "pilots",
        (!pilots.is_empty()).then_some(Value::Sequence(pilots)),
    );
    let rules: Vec<HostRule> = input
        .capabilities
        .network
        .iter()
        .map(|host| host.trim())
        .filter(|host| !host.is_empty())
        .map(|host| {
            let key = host.trim_end_matches('.').to_ascii_lowercase();
            previous_rules
                .iter()
                .find(|rule| rule.host == key)
                .cloned()
                .unwrap_or(HostRule {
                    host: host.to_string(),
                    methods: Vec::new(),
                })
        })
        .collect();
    let network = (!rules.is_empty())
        .then(|| serde_yaml::to_value(NetworkPolicy { allow: rules }))
        .transpose()
        .map_err(|e| e.to_string())?;
    put("network", network);

    // Absent means native; a harness keeps the rest of its runtime_config
    let runtime = input
        .runtime
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty() && *r != "native");
    let mut runtime_config = previous_runtime_config;
    match input.runtime_model.as_deref().map(str::trim) {
        Some(model) if !model.is_empty() => {
            runtime_config.insert(Value::from("model"), Value::from(model));
        }
        _ => {
            runtime_config.remove("model");
        }
    }
    put("runtime", runtime.map(Value::from));
    put(
        "runtime_config",
        (runtime.is_some() && !runtime_config.is_empty()).then_some(Value::Mapping(runtime_config)),
    );

    // `aws.profile`; an `aws` with neither grants nor a profile goes
    let mut aws = frontmatter
        .get("aws")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    match input.aws_profile.as_deref().and_then(text) {
        Some(profile) => {
            aws.insert(Value::from("profile"), profile);
        }
        None => {
            aws.remove("profile");
        }
    }
    let grants = aws
        .get("services")
        .and_then(Value::as_sequence)
        .is_some_and(|services| !services.is_empty());
    if !grants && !aws.contains_key("profile") {
        frontmatter.remove("aws");
    } else {
        frontmatter.insert(Value::from("aws"), Value::Mapping(aws));
    }

    let yaml = serde_yaml::to_string(&frontmatter).map_err(|e| e.to_string())?;
    let skill_md = format!("---\n{yaml}---\n\n{}\n", input.skill_body.trim());
    Agent::from_files(id.clone(), &skill_md, Some(&input.instructions))
        .map_err(|e| describe_error(id.as_str(), &e))
}

/// `claude` + `claude-sonnet-5` → `claude:claude-sonnet-5`; neither → the
/// settings default (`None`).
fn model_value(input: &AgentDefinitionInput) -> Result<Option<String>, String> {
    let clean = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    match (clean(&input.model_provider), clean(&input.model_name)) {
        (None, None) => Ok(None),
        (Some(provider), Some(name)) => Ok(Some(format!("{provider}:{name}"))),
        (None, Some(name)) if name.contains(':') => Ok(Some(name)),
        (None, Some(_)) => Err("Choose the model's provider".to_string()),
        (Some(_), None) => Err("Enter the model's name".to_string()),
    }
}

/// The editor's view of an agent: `AgentDefinitionInput`'s fields, plus
/// what it shows but doesn't edit.
fn definition_json(
    agent: &Agent,
    read_only: bool,
    workflows: &[String],
    skill_md: &str,
    source_path: &Path,
) -> Value {
    let input = AgentDefinitionInput {
        name: agent.name().to_string(),
        description: agent.description().to_string(),
        skill_body: agent.skill_body().to_string(),
        instructions: agent.instructions().to_string(),
        model_provider: agent
            .model()
            .map(|m| m.provider_kind().prefix().to_string()),
        model_name: agent.model().map(|m| m.name().to_string()),
        image: agent.container().image().map(str::to_string),
        memory_mb: agent.container().memory_mb(),
        cpu: agent.container().cpu(),
        capabilities: AgentDefinitionCapabilities {
            tools: agent
                .capabilities()
                .tools()
                .allowed()
                .map(<[String]>::to_vec)
                .unwrap_or_default(),
            network: agent
                .capabilities()
                .network()
                .allow
                .iter()
                .map(|rule| rule.host.clone())
                .collect(),
            pilots: agent
                .capabilities()
                .pilots()
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
        },
        runtime: Some(agent.runtime().as_str().into_owned()),
        runtime_model: agent.runtime_config().and_then(|c| c.model.clone()),
        aws_profile: agent
            .capabilities()
            .aws()
            .and_then(|aws| aws.profile.clone()),
    };
    let mut value = serde_json::to_value(input).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        let tools = agent.capabilities().tools();
        object.insert("id".into(), json!(agent.id().as_str()));
        // Saving can't change these; marketplace agents must be forked
        object.insert("readOnly".into(), json!(read_only));
        // `null`: every runtime tool
        object.insert("allowedTools".into(), json!(tools.allowed()));
        // With the default applied (`shell`, `write_file`)
        object.insert("requiresApproval".into(), json!(tools.approval_list()));
        object.insert(
            "networkRules".into(),
            json!(agent.capabilities().network().allow),
        );
        object.insert("hasAws".into(), json!(agent.capabilities().aws().is_some()));
        object.insert("workflows".into(), json!(workflows));
        // The file as it is now, for the editor's diff preview
        object.insert("skillMd".into(), json!(skill_md));
        // What saving renders for this agent unchanged
        object.insert("renderedSkillMd".into(), json!(agent.to_skill_md()));
        object.insert(
            "sourcePath".into(),
            json!(source_path.display().to_string()),
        );
    }
    value
}

impl RepoStore {
    /// One entity as stored in its file(s), as JSON. Agents come back in the
    /// editor's shape (see `definition_json`); everything else is its YAML
    /// file as JSON (`{}` for a singleton without a file).
    pub fn load_entity_json(&self, kind: &str, id: &str) -> Result<Value, String> {
        let kind = match kind {
            "connector" => EntityKind::Connectors,
            other => other.parse().map_err(|_| {
                format!("Unknown entity kind '{other}': use agent, workflow, trigger, connector, or settings")
            })?,
        };
        match kind {
            EntityKind::Agent => {
                let agent_id = AgentId::parse(id).map_err(|e| e.to_string())?;
                let place = self
                    .repos()
                    .agents
                    .locate(id)
                    .ok_or_else(|| format!("Agent '{id}' not found"))?;
                let path = place.dir.join(SKILL_FILE);
                let skill_md = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let agent = self.repos().agents.load(&agent_id).map_err(|e| match e {
                    RepoError::Domain(error) => describe_error(id, &error),
                    other => other.to_string(),
                })?;
                // Agent-scoped workflows are linked by the aggregate
                let workflows: Vec<String> = self.read(|repo| {
                    repo.agent(id)
                        .map(|a| a.workflows().iter().map(|w| w.to_string()).collect())
                        .unwrap_or_default()
                });
                let mut value = definition_json(
                    &agent,
                    place.origin.is_vendored(),
                    &workflows,
                    &skill_md,
                    &path,
                );
                // An installed agent running with the user's own rules
                value["instructionsOverridden"] =
                    json!(self.repos().agents.has_instructions_override(id));
                Ok(value)
            }
            EntityKind::Workflow | EntityKind::Trigger => {
                let path = match kind {
                    EntityKind::Workflow => {
                        WorkflowId::parse(id).map_err(|e| e.to_string())?;
                        let scope = self
                            .read(|repo| repo.workflow(id).map(|w| w.scope().clone()))
                            .ok_or_else(|| format!("Workflow '{id}' not found"))?;
                        self.layout().workflow_path(id, &scope)
                    }
                    _ => {
                        TriggerId::parse(id).map_err(|e| e.to_string())?;
                        self.layout().trigger_path(id)
                    }
                };
                match std::fs::read_to_string(&path) {
                    Ok(text) => yaml_to_json(&text, &path),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Err(format!("{} '{id}' not found", kind.as_str()))
                    }
                    Err(e) => Err(format!("{}: {e}", path.display())),
                }
            }
            singleton => {
                let path = self.layout().source_path(singleton, id, None);
                match std::fs::read_to_string(&path) {
                    Ok(text) => yaml_to_json(&text, &path),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
                    Err(e) => Err(format!("{}: {e}", path.display())),
                }
            }
        }
    }

    /// Replaces agent `id`'s `instructions.md` (the user's rules), then
    /// commits. An installed agent's go to its override, so it stays
    /// read-only and updates keep them.
    pub fn save_agent_instructions(&self, id: &AgentId, text: &str) -> Result<(), String> {
        self.repos().agents.save_instructions(id.as_str(), text)?;
        self.commit(EntityKind::Agent, id.as_str()).map(|_| ())
    }

    /// Back to an installed agent's shipped instructions. Returns whether
    /// the user had their own.
    pub fn reset_agent_instructions(&self, id: &AgentId) -> Result<bool, String> {
        let removed = self.repos().agents.reset_instructions(id.as_str())?;
        if removed {
            self.commit(EntityKind::Agent, id.as_str())?;
        }
        Ok(removed)
    }

    /// Agent `id`'s playbooks, for its Playbooks tab
    pub fn agent_playbooks(&self, id: &AgentId) -> Result<AgentPlaybooks, String> {
        let place = self
            .repos()
            .agents
            .locate(id.as_str())
            .ok_or_else(|| format!("Agent '{id}' not found"))?;
        let loaded = self.repos().agents.load_playbooks(id.as_str())?;
        Ok(AgentPlaybooks {
            read_only: place.origin.is_vendored(),
            playbooks: loaded.entities,
            problems: loaded
                .problems
                .into_iter()
                .map(|problem| PlaybookProblem {
                    file: problem
                        .source_path
                        .as_deref()
                        .and_then(Path::file_name)
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    error: problem.error.to_string(),
                })
                .collect(),
        })
    }

    /// Validates and writes one of agent `id`'s playbooks, then commits.
    /// `previous`: the name it had (`None` for a new one).
    pub fn save_agent_playbook(
        &self,
        id: &AgentId,
        input: &PlaybookInput,
        previous: Option<&str>,
    ) -> Result<Playbook, String> {
        let playbook = Playbook::new(&input.name, &input.description, &input.when, &input.body)
            .map_err(|e| e.to_string())?;
        self.repos()
            .agents
            .save_playbook(id.as_str(), &playbook, previous)?;
        self.commit(EntityKind::Agent, id.as_str())?;
        Ok(playbook)
    }

    /// Removes agent `id`'s playbook `name`, then commits
    pub fn delete_agent_playbook(&self, id: &AgentId, name: &str) -> Result<(), String> {
        self.repos().agents.delete_playbook(id.as_str(), name)?;
        self.commit(EntityKind::Agent, id.as_str()).map(|_| ())
    }

    /// Writes the editor's input to agent `id`'s files (creating the agent
    /// if it has no folder), then commits: the aggregate and projection
    /// update and `repo://agent-updated` (or `-created`) is published.
    ///
    /// The domain validates before anything is written. If the write or
    /// the commit fails, the previous files are written back, so a failed
    /// save leaves the agent as it was.
    pub fn save_agent_definition(
        &self,
        id: &AgentId,
        input: &AgentDefinitionInput,
    ) -> Result<(), String> {
        let place = self.repos().agents.locate(id.as_str());
        if let Some(place) = &place {
            if place.origin.is_vendored() {
                return Err(RepoError::ReadOnly {
                    kind: EntityKind::Agent,
                    id: id.to_string(),
                }
                .to_string());
            }
        }
        let existing = match &place {
            Some(place) => {
                let path = place.dir.join(SKILL_FILE);
                Some(
                    std::fs::read_to_string(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?,
                )
            }
            None => None,
        };
        let agent = definition_agent(id, existing.as_deref(), input)?;
        if place.is_none() {
            self.repos().agents.create(&agent)?;
            return self.commit(EntityKind::Agent, id.as_str()).map(|_| ());
        }
        let previous = self.repos().agents.load(id).ok();
        let saved = self
            .repos()
            .agents
            .save(&agent)
            .map_err(String::from)
            .and_then(|_| self.commit(EntityKind::Agent, id.as_str()).map(|_| ()));
        if let Err(error) = saved {
            if let Some(previous) = previous {
                if let Err(e) = self.repos().agents.save(&previous) {
                    log::error!("Couldn't restore agent '{id}' after a failed save: {e}");
                }
                let _ = self.reload(EntityKind::Agent, id.as_str(), ChangedBy::User);
            }
            return Err(error);
        }
        Ok(())
    }
}

fn yaml_to_json(text: &str, path: &Path) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_yaml::from_str::<Value>(text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Saves the agent editor's input (see `RepoStore::save_agent_definition`)
/// and returns the saved agent.
pub async fn save_agent(
    state: &AppState,
    agent_id: &str,
    input: AgentDefinitionInput,
) -> Result<AgentInfo, String> {
    let id = AgentId::parse(agent_id).map_err(|_| format!("Invalid agent id '{agent_id}'"))?;
    let store = state.store().clone();
    let info = tokio::task::spawn_blocking(move || {
        store.save_agent_definition(&id, &input)?;
        store
            .agent_infos()
            .into_iter()
            .find(|a| a.id == id.as_str())
            .ok_or_else(|| format!("Agent '{id}' was saved but couldn't be read back"))
    })
    .await
    .map_err(|e| format!("Save task failed: {e}"))??;
    // The reactor already did on `repo://agent-created`; idempotent
    state.orchestrators().ensure(&info.id);
    log::info!("Saved agent '{}'", info.id);
    Ok(info)
}

/// Saves agent `agent_id`'s instructions (`RepoStore::save_agent_instructions`)
pub async fn save_agent_instructions(
    state: &AppState,
    agent_id: &str,
    instructions: String,
) -> Result<(), String> {
    let id = AgentId::parse(agent_id).map_err(|_| format!("Invalid agent id '{agent_id}'"))?;
    let store = state.store().clone();
    tokio::task::spawn_blocking(move || store.save_agent_instructions(&id, &instructions))
        .await
        .map_err(|e| format!("Save task failed: {e}"))?
}

/// Drops an installed agent's own instructions (`RepoStore::reset_agent_instructions`)
pub async fn reset_agent_instructions(state: &AppState, agent_id: &str) -> Result<bool, String> {
    let id = AgentId::parse(agent_id).map_err(|_| format!("Invalid agent id '{agent_id}'"))?;
    let store = state.store().clone();
    tokio::task::spawn_blocking(move || store.reset_agent_instructions(&id))
        .await
        .map_err(|e| format!("Reset task failed: {e}"))?
}

/// An agent's playbooks: its `skills/` folder, and whether it's read-only
/// (an installed agent's)
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlaybooks {
    pub read_only: bool,
    pub playbooks: Vec<Playbook>,
    /// Files that don't load, and why
    pub problems: Vec<PlaybookProblem>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookProblem {
    /// The file name in `skills/`
    pub file: String,
    pub error: String,
}

/// A playbook as the Playbooks tab saves it
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookInput {
    pub name: String,
    pub description: String,
    pub when: String,
    pub body: String,
}

fn parse_agent_id(agent_id: &str) -> Result<AgentId, String> {
    AgentId::parse(agent_id).map_err(|_| format!("Invalid agent id '{agent_id}'"))
}

/// Agent `agent_id`'s playbooks (`RepoStore::agent_playbooks`)
pub async fn agent_playbooks(state: &AppState, agent_id: &str) -> Result<AgentPlaybooks, String> {
    let (id, store) = (parse_agent_id(agent_id)?, state.store().clone());
    tokio::task::spawn_blocking(move || store.agent_playbooks(&id))
        .await
        .map_err(|e| format!("Load task failed: {e}"))?
}

/// Saves one playbook (`RepoStore::save_agent_playbook`)
pub async fn save_agent_playbook(
    state: &AppState,
    agent_id: &str,
    input: PlaybookInput,
    previous: Option<String>,
) -> Result<Playbook, String> {
    let (id, store) = (parse_agent_id(agent_id)?, state.store().clone());
    tokio::task::spawn_blocking(move || store.save_agent_playbook(&id, &input, previous.as_deref()))
        .await
        .map_err(|e| format!("Save task failed: {e}"))?
}

/// Removes one playbook (`RepoStore::delete_agent_playbook`)
pub async fn delete_agent_playbook(
    state: &AppState,
    agent_id: &str,
    name: String,
) -> Result<(), String> {
    let (id, store) = (parse_agent_id(agent_id)?, state.store().clone());
    tokio::task::spawn_blocking(move || store.delete_agent_playbook(&id, &name))
        .await
        .map_err(|e| format!("Delete task failed: {e}"))?
}

pub async fn load_entity(state: &AppState, kind: &str, id: &str) -> Result<Value, String> {
    let (store, kind, id) = (state.store().clone(), kind.to_string(), id.to_string());
    tokio::task::spawn_blocking(move || store.load_entity_json(&kind, &id))
        .await
        .map_err(|e| format!("Load task failed: {e}"))?
}

pub async fn git_status(state: &AppState) -> Result<GitStatus, String> {
    let git = state.git_service().clone();
    tokio::task::spawn_blocking(move || git.status())
        .await
        .map_err(|e| format!("git status failed: {e}"))?
}

pub async fn git_diff(state: &AppState, path: &str) -> Result<String, String> {
    let (git, path) = (state.git_service().clone(), path.to_string());
    tokio::task::spawn_blocking(move || git.diff(&path))
        .await
        .map_err(|e| format!("git diff failed: {e}"))?
}

/// Stages and commits everything in the repo. Pushing stays with "Sync now".
pub async fn git_commit(state: &AppState, message: &str) -> Result<String, String> {
    let (git, store, message) = (
        state.git_service().clone(),
        state.store().clone(),
        message.to_string(),
    );
    tokio::task::spawn_blocking(move || {
        let owner = store.repos().load_manifest().ok().and_then(|m| m.owner);
        git.commit(&message, owner.as_deref())
    })
    .await
    .map_err(|e| format!("git commit failed: {e}"))?
}

impl RepoStore {
    /// Wipes the projection and rebuilds it from the files. Unless `force`,
    /// refuses (changing nothing) while any file is invalid, so a typo can't
    /// silently drop an entity; with `force`, invalid files are skipped.
    pub fn rebuild_checked(&self, force: bool) -> Result<ProjectionStats, String> {
        if !force {
            let aggregate = self.repos().load_aggregate();
            let errors: Vec<String> = aggregate
                .problems()
                .into_iter()
                .filter(|p| p.severity == Severity::Error)
                .map(|p| match &p.source_path {
                    Some(path) => format!("{}: {}", path.display(), p.error),
                    None => p.error.to_string(),
                })
                .collect();
            if !errors.is_empty() {
                return Err(format!(
                    "{} file(s) are invalid; fix them or rebuild with --force to skip them:\n{}",
                    errors.len(),
                    errors.join("\n")
                ));
            }
        }
        self.rebuild()
    }

    pub fn status(&self) -> RepoStatus {
        self.read(|repo| RepoStatus {
            root: self.layout().root.display().to_string(),
            agents: repo.agents().count() as u32,
            workflows: repo.workflows().count() as u32,
            triggers: repo.triggers().count() as u32,
            problems: repo
                .problems()
                .into_iter()
                .map(|p| RepoProblem {
                    kind: p.kind.as_str().to_string(),
                    id: p.id,
                    source_path: p.source_path.map(|path| path.display().to_string()),
                    error: p.error.to_string(),
                    is_error: p.severity == Severity::Error,
                })
                .collect(),
            watching: self.is_watching(),
        })
    }
}

/// Rebuilds the projection, then brings the running parts in line: every
/// agent has an orchestrator and trigger sources re-arm.
pub async fn rebuild(state: &AppState, force: bool) -> Result<ProjectionStats, String> {
    events::emit(state.bus(), &RepoRebuildStarted { force });
    let store = state.store().clone();
    let stats = tokio::task::spawn_blocking(move || store.rebuild_checked(force))
        .await
        .map_err(|e| format!("Rebuild failed: {e}"))??;
    let agents: Vec<String> = state
        .store()
        .read(|repo| repo.agents().map(|a| a.id().to_string()).collect());
    for id in &agents {
        state.orchestrators().ensure(id);
    }
    state.triggers().reload().await?;
    // Lists in the UI reload on these
    events::emit(
        state.bus(),
        &events::AgentsChanged {
            agent_id: String::new(),
        },
    );
    events::emit(
        state.bus(),
        &crate::services::workflows::WorkflowsChanged {},
    );
    events::emit(state.bus(), &crate::services::triggers::TriggersChanged {});
    events::emit(
        state.bus(),
        &RepoRebuildCompleted {
            stats: stats.clone(),
        },
    );
    log::info!(
        "Rebuilt the projection: {} agents, {} workflows, {} triggers ({} skipped)",
        stats.agents,
        stats.workflows,
        stats.triggers,
        stats.skipped
    );
    Ok(stats)
}

pub async fn status(state: &AppState) -> Result<RepoStatus, String> {
    Ok(state.store().status())
}

/// `nandi rebuild [--force]`: rebuilds `~/.nandi/state.db`'s projection
/// without starting the app. Returns the exit code.
pub fn rebuild_cli(force: bool) -> i32 {
    match rebuild_offline(force) {
        Ok(stats) => {
            println!(
                "Rebuilt the projection: {} agents, {} workflows, {} triggers, {} connectors.",
                stats.agents, stats.workflows, stats.triggers, stats.connectors
            );
            if stats.skipped > 0 {
                println!("Skipped {} invalid file(s).", stats.skipped);
            }
            0
        }
        Err(e) => {
            eprintln!("Rebuild failed: {e}");
            1
        }
    }
}

fn rebuild_offline(force: bool) -> Result<ProjectionStats, String> {
    let db = Db::open(&crate::utils::nandi_config::state_db_path()?)?;
    let repos =
        crate::services::repos::Repos::new(crate::services::repo_bootstrap::install_layout()?);
    RepoStore::sqlite(repos, db).rebuild_checked(force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;
    use std::fs;
    use std::sync::Arc;

    #[tokio::test]
    async fn rebuild_reconstructs_a_wiped_projection() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let layout = state.store().layout().clone();
        let skill = layout.skill_path("writer");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "---\nname: Writer\n---\n").unwrap();
        fs::create_dir_all(&layout.workflows_dir).unwrap();
        fs::write(
            layout.workflows_dir.join("daily.yaml"),
            "id: daily\nname: Daily\nsteps:\n  - {id: a, agent: writer, input: hi}\n",
        )
        .unwrap();
        state.store().load().unwrap();

        // As if state.db had been deleted: every projected row is gone
        state
            .db()
            .call(|conn| {
                conn.execute_batch(
                    "DELETE FROM agents; DELETE FROM workflows; DELETE FROM sync_state;",
                )
            })
            .await
            .unwrap();

        let stats = rebuild(&state, false).await.unwrap();
        assert_eq!((stats.agents, stats.workflows), (1, 1));
        let count: i64 = state
            .db()
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 1);
        let names = recorder.names();
        assert!(names.contains(&"repo://rebuild_started"));
        assert!(names.contains(&"repo://rebuild_completed"));

        // An invalid file blocks a plain rebuild but not a forced one
        fs::write(layout.workflows_dir.join("broken.yaml"), "id: [").unwrap();
        let error = rebuild(&state, false).await.unwrap_err();
        assert!(error.contains("broken.yaml"), "{error}");
        let stats = rebuild(&state, true).await.unwrap();
        assert_eq!((stats.workflows, stats.skipped), (1, 1));
        let status = state.store().status();
        assert!(status
            .problems
            .iter()
            .any(|p| p.is_error && p.id.as_deref() == Some("broken")));
        state.shutdown();
    }

    fn input(name: &str) -> AgentDefinitionInput {
        AgentDefinitionInput {
            name: name.into(),
            description: String::new(),
            skill_body: String::new(),
            instructions: String::new(),
            model_provider: None,
            model_name: None,
            image: None,
            memory_mb: None,
            cpu: None,
            capabilities: AgentDefinitionCapabilities::default(),
            runtime: None,
            runtime_model: None,
            aws_profile: None,
        }
    }

    fn id(s: &str) -> AgentId {
        AgentId::parse(s).unwrap()
    }

    #[test]
    fn definitions_keep_what_the_editor_does_not_edit() {
        let existing = "---\nname: Old\nrequires_approval: [shell]\ncustom: kept\nmemory-mb: 64\nnetwork:\n  allow:\n    - host: api.github.com\n      methods: [GET]\n---\nOld body\n";
        let mut edit = input("New");
        edit.description = "Writes".into();
        edit.skill_body = "New body".into();
        edit.instructions = "Cite sources.".into();
        edit.model_provider = Some("claude".into());
        edit.model_name = Some("claude-sonnet-5".into());
        edit.memory_mb = Some(2048);
        edit.cpu = Some(1.5);
        edit.capabilities = AgentDefinitionCapabilities {
            tools: vec!["shell".into(), " ".into()],
            network: vec!["API.github.com".into(), "pypi.org".into()],
            pilots: vec!["browser".into()],
        };
        let agent = definition_agent(&id("w"), Some(existing), &edit).unwrap();
        assert_eq!(agent.name(), "New");
        assert_eq!(agent.skill_body(), "New body");
        assert_eq!(agent.instructions(), "Cite sources.");
        assert_eq!(agent.model().unwrap().to_string(), "claude:claude-sonnet-5");
        assert_eq!(agent.container().memory_mb(), Some(2048));
        assert_eq!(agent.capabilities().tools().allowed().unwrap(), ["shell"]);
        let rules = &agent.capabilities().network().allow;
        assert_eq!(rules[0].methods, ["GET"], "existing methods are kept");
        assert_eq!(rules[1].host, "pypi.org");
        let md = agent.to_skill_md();
        assert!(md.contains("custom: kept"), "{md}");
        assert!(md.contains("requires_approval"), "{md}");
        assert!(!md.contains("memory-mb"), "the alias is replaced: {md}");

        // Clearing fields removes their keys (back to the defaults)
        let cleared = definition_agent(&id("w"), Some(&md), &input("New")).unwrap();
        assert!(cleared.model().is_none());
        assert!(cleared.capabilities().tools().allowed().is_none());
        assert!(cleared.capabilities().network().allow.is_empty());
        assert!(cleared.to_skill_md().contains("custom: kept"));
    }

    #[test]
    fn the_aws_profile_is_set_and_cleared_without_touching_the_grants() {
        let existing = "---\nname: Ops\naws:\n  mode: sts\n  services:\n    - service: s3\n      actions: [GetObject]\n---\n\nBody\n";
        let with_profile = AgentDefinitionInput {
            aws_profile: Some(" work ".into()),
            ..input("Ops")
        };
        let agent = definition_agent(&id("ops"), Some(existing), &with_profile).unwrap();
        let aws = agent.capabilities().aws().unwrap();
        assert_eq!(aws.profile.as_deref(), Some("work"));
        assert_eq!(aws.services[0].actions, ["GetObject"]);

        let cleared =
            definition_agent(&id("ops"), Some(&agent.to_skill_md()), &input("Ops")).unwrap();
        let aws = cleared.capabilities().aws().unwrap();
        assert!(aws.profile.is_none() && aws.services.len() == 1);
        assert!(!cleared.to_skill_md().contains("profile"));

        // A profile alone is an `aws` section; clearing it removes the section
        let alone = definition_agent(&id("ops"), None, &with_profile).unwrap();
        assert!(alone.capabilities().aws().is_some());
        let none = definition_agent(&id("ops"), Some(&alone.to_skill_md()), &input("Ops")).unwrap();
        assert!(none.capabilities().aws().is_none());

        let spaced = AgentDefinitionInput {
            aws_profile: Some("my work".into()),
            ..input("Ops")
        };
        assert!(definition_agent(&id("ops"), None, &spaced).is_err());
    }

    #[test]
    fn invalid_definitions_are_rejected_before_any_write() {
        for (edit, expected) in [
            (input("  "), "name"),
            (
                AgentDefinitionInput {
                    model_provider: Some("gemini".into()),
                    model_name: Some("pro".into()),
                    ..input("A")
                },
                "unknown",
            ),
            (
                AgentDefinitionInput {
                    model_provider: Some("claude".into()),
                    ..input("A")
                },
                "model's name",
            ),
            (
                AgentDefinitionInput {
                    cpu: Some(0.0),
                    ..input("A")
                },
                "cpu",
            ),
            (
                AgentDefinitionInput {
                    capabilities: AgentDefinitionCapabilities {
                        network: vec!["a.*.com".into()],
                        ..Default::default()
                    },
                    ..input("A")
                },
                "invalid host",
            ),
        ] {
            let error = definition_agent(&id("a"), None, &edit).unwrap_err();
            assert!(error.contains(expected), "{expected}: {error}");
        }
        let error = definition_agent(&id("a"), Some("---\nname: [\n---\n"), &input("A"));
        assert!(error.unwrap_err().contains("valid YAML"));
    }

    #[tokio::test]
    async fn saving_writes_the_files_projects_and_publishes() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        git2::Repository::init(&state.store().layout().root).unwrap();
        let layout = state.store().layout().clone();
        let skill = layout.skill_path("writer");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "---\nname: Writer\ncustom: kept\n---\nWrite.\n").unwrap();
        state.store().load().unwrap();

        // Load what the editor shows
        let loaded = load_entity(&state, "agent", "writer").await.unwrap();
        assert_eq!(loaded["name"], "Writer");
        assert_eq!(loaded["skillBody"], "Write.");
        assert_eq!(loaded["readOnly"], false);
        assert_eq!(loaded["requiresApproval"], json!(["shell", "write_file"]));
        let mut edit: AgentDefinitionInput = serde_json::from_value(loaded).unwrap();

        // Edit and save
        edit.name = "Ghostwriter".into();
        edit.instructions = "Be brief.".into();
        edit.capabilities.tools = vec!["read_file".into()];
        recorder.take();
        let info = save_agent(&state, "writer", edit).await.unwrap();
        assert_eq!(info.name, "Ghostwriter");

        // The files changed on disk (unknown keys kept)
        let md = fs::read_to_string(&skill).unwrap();
        assert!(md.contains("name: Ghostwriter") && md.contains("custom: kept"));
        assert!(md.contains("- read_file"), "{md}");
        assert_eq!(
            fs::read_to_string(layout.instructions_path("writer")).unwrap(),
            "Be brief.\n"
        );
        // The projection follows
        let name: String = state
            .db()
            .call(|conn| {
                conn.query_row("SELECT name FROM agents WHERE id = 'writer'", [], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(name, "Ghostwriter");
        // And the change was published
        assert!(recorder.names().contains(&"repo://agent-updated"));

        // git status shows the change; a commit clears it
        let status = git_status(&state).await.unwrap();
        let paths: Vec<&str> = status.files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.contains(&".nandi/agents/writer/skill.md"),
            "{paths:?}"
        );
        assert!(paths.contains(&".nandi/agents/writer/instructions.md"));
        assert!(status.files.iter().all(|f| f.status == "added"));
        let diff = git_diff(&state, ".nandi/agents/writer/skill.md")
            .await
            .unwrap();
        assert!(diff.contains("+name: Ghostwriter"), "{diff}");
        let hash = git_commit(&state, "Edit writer").await.unwrap();
        assert_eq!(hash.len(), 40);
        assert!(git_status(&state).await.unwrap().files.is_empty());
        assert!(git_commit(&state, "Again")
            .await
            .unwrap_err()
            .contains("Nothing"));

        // Edits after the commit show as modified, with a real diff
        let mut edit: AgentDefinitionInput =
            serde_json::from_value(load_entity(&state, "agent", "writer").await.unwrap()).unwrap();
        edit.description = "Writes things".into();
        save_agent(&state, "writer", edit).await.unwrap();
        let status = git_status(&state).await.unwrap();
        assert_eq!(status.files.len(), 1);
        assert_eq!(status.files[0].status, "modified");
        let diff = git_diff(&state, &status.files[0].path).await.unwrap();
        assert!(diff.contains("+description: Writes things"), "{diff}");
        assert!(git_diff(&state, "../etc/passwd").await.is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn saving_an_unknown_agent_creates_it_and_invalid_saves_change_nothing() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let mut edit = input("Fresh");
        edit.skill_body = "Hello.".into();
        save_agent(&state, "fresh", edit).await.unwrap();
        let skill = state.store().layout().skill_path("fresh");
        assert!(skill.is_file());
        assert!(recorder.names().contains(&"repo://agent-created"));
        let before = fs::read_to_string(&skill).unwrap();

        let mut bad = input("Fresh");
        bad.memory_mb = Some(1);
        assert!(save_agent(&state, "fresh", bad).await.is_err());
        assert_eq!(fs::read_to_string(&skill).unwrap(), before);
        assert!(save_agent(&state, "Not Valid", input("x")).await.is_err());
        // Without a git repo, git commands explain why
        assert!(git_status(&state)
            .await
            .unwrap_err()
            .contains("isn't a git repository"));
        state.shutdown();
    }

    #[tokio::test]
    async fn deleting_removes_the_folder_and_the_projected_row() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        save_agent(&state, "temp", input("Temp")).await.unwrap();
        let dir = state.store().layout().agent_dir("temp");
        assert!(dir.is_dir());

        crate::services::agents::delete_agent(&state, "temp")
            .await
            .unwrap();
        assert!(!dir.exists());
        let count: i64 = state
            .db()
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM agents WHERE id = 'temp'", [], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(recorder.names().contains(&"repo://agent-deleted"));
        // A rebuild doesn't bring it back
        let stats = rebuild(&state, false).await.unwrap();
        assert_eq!(stats.agents, 0);
        assert!(load_entity(&state, "agent", "temp").await.is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn loads_other_entities_as_json() {
        let state = AppState::for_tests(vec![]);
        let layout = state.store().layout().clone();
        fs::create_dir_all(&layout.workflows_dir).unwrap();
        fs::write(
            layout.workflows_dir.join("daily.yaml"),
            "id: daily\nname: Daily\nsteps:\n  - {id: a, agent: writer, input: hi}\n",
        )
        .unwrap();
        state.store().load().unwrap();
        let workflow = load_entity(&state, "workflow", "daily").await.unwrap();
        assert_eq!(workflow["name"], "Daily");
        assert_eq!(
            load_entity(&state, "settings", "").await.unwrap(),
            json!({})
        );
        assert_eq!(
            load_entity(&state, "connector", "").await.unwrap(),
            json!({})
        );
        assert!(load_entity(&state, "bogus", "x").await.is_err());
        assert!(load_entity(&state, "trigger", "../x").await.is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn a_fresh_database_is_rebuilt_from_files_on_load() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = crate::services::repos::RepoLayout::new(
            tmp.path().join("repo"),
            tmp.path().join("trash"),
        );
        let skill = layout.skill_path("writer");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "---\nname: Writer\n---\n").unwrap();
        let db_path = tmp.path().join("state.db");

        let first = RepoStore::sqlite(
            crate::services::repos::Repos::new(layout.clone()),
            Db::open(&db_path).unwrap(),
        );
        first.load().unwrap();
        drop(first);
        // Delete state.db (and its WAL files) entirely
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", db_path.display()));
        }
        let db = Db::open(&db_path).unwrap();
        let second = RepoStore::sqlite(crate::services::repos::Repos::new(layout), db.clone());
        let stats = second.load().unwrap();
        assert_eq!(stats.agents, 1, "everything is reconstructed");
        let name: String = db
            .conn()
            .unwrap()
            .query_row("SELECT name FROM agents WHERE id = 'writer'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "Writer");
    }
}
