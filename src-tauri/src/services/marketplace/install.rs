//! The installer: vendors marketplace sources into the gold copy.
//!
//! It only writes files (through `VendorRepository` and the agent and
//! workflow repositories) and then asks the `RepoStore` to re-scan. SQLite
//! changes the same way as for any file edit, so the file watcher seeing
//! the same files afterwards is a no-op (equal hashes). Nothing here talks
//! to the database directly.
//!
//! Install, step by step:
//! 1. refuse if `min_nandi_version` is newer than this app
//! 2. resolve the ref (`ls-remote`) and fetch just it into a scratch repo
//!    outside the gold copy
//! 3. read the files from the commit's tree, keep only what an agent or
//!    pack may contain (allowlist), and validate every agent and workflow
//!    with the domain types
//! 4. refuse if any agent or workflow ID is already taken
//! 5. write the folder in one rename, record it in `installed.yaml`,
//!    re-scan
//!
//! Every method blocks (git, files); callers use `spawn_blocking`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::SecondsFormat;
use serde::Serialize;
use specta::Type;

use super::local::LocalPackSource;
use super::manifest::{
    is_compatible, parse_version, AgentManifest, ManifestCapabilities, PackAgentSummary, PackFile,
    PackManifest, NANDI_VERSION, PACK_MANIFEST_PATH,
};
use super::source::{FetchProgress, GitSource, RemoteRef};
use crate::services::agents::describe_error;
use crate::services::domain::installed::{
    local_source, InstallKind, InstalledAgent, InstalledFile, InstalledPack, LOCAL_REF,
};
use crate::services::domain::policy::Policy;
use crate::services::domain::{
    Agent, AgentId, ChangedBy, EntityKind, PackId, PublisherHandle, Workflow, WorkflowId,
};
use crate::services::repo_store::RepoStore;
use crate::services::repos::agent_locator;
use crate::services::repos::vendor_repo::{tree_hash, FileSet};
use crate::services::repos::{Repos, Repository};

/// A pack that was installed.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackInfo {
    pub id: String,
    pub version: String,
    pub commit: String,
    pub agents: Vec<String>,
    pub workflows: Vec<String>,
}

/// An agent that was installed.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAgentInfo {
    pub id: String,
    pub publisher: String,
    pub version: String,
    pub commit: String,
}

/// One row of the Installed tab.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledItem {
    /// pack | agent
    pub kind: String,
    pub id: String,
    pub name: String,
    pub source: String,
    pub version: String,
    pub git_ref: String,
    pub commit: String,
    pub installed_at: String,
    pub signature_verified: bool,
    pub registry: Option<String>,
    /// The agents it provides
    pub agents: Vec<String>,
    /// Its files changed since install (updates are refused until undone)
    pub modified: bool,
}

/// A newer version of something installed.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAvailable {
    pub kind: String,
    pub id: String,
    pub current_version: String,
    pub latest_version: String,
    /// What to pass to `apply_update`
    pub latest_ref: String,
    pub current_commit: String,
    pub latest_commit: String,
}

/// An update that was applied.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppliedUpdate {
    pub kind: String,
    pub id: String,
    pub from_version: String,
    pub to_version: String,
}

/// `(id, version)` of each installed pack or agent.
pub type InstalledVersions = Vec<(String, String)>;

/// Updates found, and `(id, error)` for sources that couldn't be checked.
pub type UpdateCheck = (Vec<UpdateAvailable>, Vec<(String, String)>);

/// A fork, and what may need the user's attention.
#[derive(Debug, Clone)]
pub struct Forked {
    pub agent: Agent,
    /// e.g. a pack workflow that still names the original agent
    pub warnings: Vec<String>,
}

/// Files an agent folder may contain; everything else in a source is left
/// behind.
fn agent_file_allowed(relative: &str) -> bool {
    match relative.split_once('/') {
        None => {
            matches!(
                relative,
                "skill.md" | "instructions.md" | "README.md" | "icon.svg" | "icon.png"
            ) || relative.starts_with("LICENSE")
        }
        Some(("workflows", file)) => {
            !file.contains('/') && (file.ends_with(".yaml") || file.ends_with(".yml"))
        }
        // Playbooks (`skills/<name>.md`)
        Some(("skills", file)) => !file.contains('/') && file.ends_with(".md"),
        Some(_) => false,
    }
}

fn select_agent_files(files: FileSet) -> FileSet {
    files
        .into_iter()
        .filter(|(path, _)| agent_file_allowed(path))
        .collect()
}

/// The pack's files: its manifest and top-level docs, the listed agents'
/// folders, and the listed workflows.
fn select_pack_files(files: &FileSet, agents: &[AgentId], workflows: &[WorkflowId]) -> FileSet {
    files
        .iter()
        .filter(|(path, _)| {
            if path.as_str() == PACK_MANIFEST_PATH
                || agent_file_allowed(path) && !path.contains('/')
            {
                return path.as_str() != "skill.md" && path.as_str() != "instructions.md";
            }
            if let Some(rest) = path.strip_prefix("agents/") {
                if let Some((id, inner)) = rest.split_once('/') {
                    return agents.iter().any(|a| a.as_str() == id) && agent_file_allowed(inner);
                }
            }
            if let Some(file) = path.strip_prefix("workflows/") {
                return workflows
                    .iter()
                    .any(|w| *file == format!("{w}.yaml") || *file == format!("{w}.yml"));
            }
            is_pack_policy(path)
        })
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect()
}

/// `policies/<id>.md`: a policy the pack ships for its own agents
fn is_pack_policy(path: &str) -> bool {
    path.strip_prefix("policies/")
        .and_then(|file| file.strip_suffix(".md"))
        .is_some_and(|id| AgentId::parse(id).is_ok())
}

/// A pack's policies must parse and name only its own agents: a pack can
/// narrow what its agents do, never anyone else
fn validate_pack_policies(files: &FileSet, agents: &[AgentId]) -> Result<(), String> {
    let ids: Vec<&str> = agents.iter().map(AgentId::as_str).collect();
    for path in files.keys().filter(|p| is_pack_policy(p)) {
        let id = path.trim_start_matches("policies/").trim_end_matches(".md");
        let policy = Policy::parse(id, text(files, path)?.unwrap_or_default())
            .map_err(|e| format!("The pack's {path} isn't valid: {e}"))?;
        if !policy.only_for(&ids) {
            return Err(format!(
                "The pack's {path} must apply only to its own agents (applies_to: [agent:<id>], one of {})",
                ids.join(", ")
            ));
        }
    }
    Ok(())
}

fn text<'a>(files: &'a FileSet, path: &str) -> Result<Option<&'a str>, String> {
    match files.get(path) {
        None => Ok(None),
        Some(bytes) => std::str::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{path} isn't UTF-8 text")),
    }
}

/// Parses an agent folder's files; the agent and its scoped workflows,
/// which may drive only the agents in `drives` (the agent itself, or its
/// pack's agents).
fn validate_agent(
    id: &AgentId,
    files: &FileSet,
    drives: &[AgentId],
) -> Result<(Agent, Vec<WorkflowId>), String> {
    let skill =
        text(files, "skill.md")?.ok_or_else(|| format!("The agent '{id}' has no skill.md"))?;
    let agent = Agent::from_files(id.clone(), skill, text(files, "instructions.md")?)
        .map_err(|e| describe_error(id.as_str(), &e))?;
    let mut workflows = Vec::new();
    for path in files.keys().filter(|p| p.starts_with("workflows/")) {
        workflows.push(validate_workflow(path, files, drives)?);
    }
    Ok((agent, workflows))
}

/// Parses a workflow. Its agent steps may drive only the agents in
/// `drives`: a pack's workflow never sends work to the user's own agents
/// (the Chief of Staff) or another install's, which it has no consent for.
fn validate_workflow(
    path: &str,
    files: &FileSet,
    drives: &[AgentId],
) -> Result<WorkflowId, String> {
    let yaml = text(files, path)?.unwrap_or_default();
    let workflow =
        Workflow::from_yaml(yaml).map_err(|e| format!("The workflow {path} isn't valid: {e}"))?;
    if let Some(outside) = workflow.agents().find(|agent| !drives.contains(agent)) {
        return Err(format!(
            "The workflow {path} drives the agent '{outside}', which it doesn't ship. A workflow may use only its own agents ({}).",
            drives.iter().map(AgentId::as_str).collect::<Vec<_>>().join(", ")
        ));
    }
    let stem = path
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".yaml").or_else(|| f.strip_suffix(".yml")))
        .unwrap_or_default();
    if workflow.id().as_str() != stem {
        return Err(format!(
            "{path} declares id '{}'; it must match the file name",
            workflow.id()
        ));
    }
    Ok(workflow.id().clone())
}

/// The agents and workflows a pack ships: its manifest's lists, or every
/// `agents/<id>/skill.md` and `workflows/<w>.yaml` if it lists none.
fn pack_contents(
    pack: &PackFile,
    files: &FileSet,
) -> Result<(Vec<AgentId>, Vec<WorkflowId>), String> {
    let agents = if pack.agents.is_empty() {
        files
            .keys()
            .filter_map(|p| p.strip_prefix("agents/")?.strip_suffix("/skill.md"))
            .filter(|id| !id.contains('/'))
            .map(|id| AgentId::parse(id).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        pack.agents.clone()
    };
    let workflows = if pack.workflows.is_empty() {
        files
            .keys()
            .filter_map(|p| p.strip_prefix("workflows/"))
            .filter(|f| !f.contains('/'))
            .filter_map(|f| f.strip_suffix(".yaml").or_else(|| f.strip_suffix(".yml")))
            .map(|id| WorkflowId::parse(id).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        pack.workflows.clone()
    };
    if agents.is_empty() && workflows.is_empty() {
        return Err("The pack has no agents or workflows".to_string());
    }
    Ok((agents, workflows))
}

/// What a pack checkout holds, validated.
struct PackContents {
    file: PackFile,
    files: FileSet,
    agents: Vec<AgentId>,
    workflows: Vec<WorkflowId>,
    /// What its agents together request
    requested: ManifestCapabilities,
}

fn read_pack(
    all: &FileSet,
    expected_id: &str,
    forked_out: &[AgentId],
) -> Result<PackContents, String> {
    let manifest = text(all, PACK_MANIFEST_PATH)?
        .ok_or_else(|| format!("The pack has no {PACK_MANIFEST_PATH}"))?;
    let file = PackFile::from_yaml(manifest)?;
    if file.id.as_str() != expected_id {
        return Err(format!(
            "The pack's manifest says it's '{}', not '{expected_id}'",
            file.id
        ));
    }
    if !is_compatible(&file.min_nandi_version) {
        return Err(newer_nandi(&file.name, &file.min_nandi_version));
    }
    let (mut agents, workflows) = pack_contents(&file, all)?;
    // Workflows may still drive an agent the user forked out: it is that
    // agent, now theirs
    let drives = agents.clone();
    agents.retain(|a| !forked_out.contains(a));
    let files = select_pack_files(all, &agents, &workflows);
    let mut requested = ManifestCapabilities::default();
    for agent in &agents {
        let prefix = format!("agents/{agent}/");
        let own: FileSet = files
            .iter()
            .filter_map(|(p, b)| Some((p.strip_prefix(&prefix)?.to_string(), b.clone())))
            .collect();
        let (agent, _) = validate_agent(agent, &own, &drives)?;
        requested.include(super::consent::requested(&agent));
    }
    for workflow in &workflows {
        let path = ["yaml", "yml"]
            .iter()
            .map(|ext| format!("workflows/{workflow}.{ext}"))
            .find(|p| files.contains_key(p))
            .ok_or_else(|| {
                format!("The pack lists workflow '{workflow}' but has no file for it")
            })?;
        validate_workflow(&path, &files, &drives)?;
    }
    validate_pack_policies(&files, &agents)?;
    Ok(PackContents {
        file,
        files,
        agents,
        workflows,
        requested,
    })
}

/// Refuses content that asks for more than its registry entry lists: the
/// Library card and the install dialog showed the entry, so that is all
/// the user agreed to (connectors already check this, `matches_entry` in
/// `connectors.rs`).
fn matches_entry(
    name: &str,
    listed: &ManifestCapabilities,
    requested: &ManifestCapabilities,
) -> Result<(), String> {
    let unlisted = requested.not_in(listed);
    if unlisted.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{name} asks for more than its registry entry lists ({}). It wasn't installed; tell its publisher.",
        unlisted.join(", ")
    ))
}

/// The `registry` of a pack read from this computer
pub const LOCAL_REGISTRY: &str = "local";

/// Where a pack's files came from, as `installed.yaml` records it
struct PackOrigin {
    source: String,
    git_ref: String,
    commit: Option<String>,
    registry: Option<String>,
    signature_verified: bool,
}

/// A local pack's files, validated; its ID is the one its manifest names
fn read_local_pack(source: &LocalPackSource) -> Result<(PackId, PackContents), String> {
    let all = source.files()?;
    let manifest = text(&all, PACK_MANIFEST_PATH)?.ok_or_else(|| {
        format!(
            "{} isn't a Nandi pack: it has no {PACK_MANIFEST_PATH}",
            source.path().display()
        )
    })?;
    let id = PackFile::from_yaml(manifest)?.id;
    let pack = read_pack(&all, id.as_str(), &[])?;
    Ok((id, pack))
}

fn newer_nandi(name: &str, min: &str) -> String {
    format!(
        "{name} needs Nandi {min} or newer (this is {NANDI_VERSION}). Update Nandi to install it."
    )
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The workflow IDs of an agent folder's scoped workflows.
fn scoped_workflow_ids(files: &FileSet, prefix: &str) -> Vec<String> {
    files
        .keys()
        .filter_map(|p| p.strip_prefix(prefix)?.strip_prefix("workflows/"))
        .filter_map(|f| f.strip_suffix(".yaml").or_else(|| f.strip_suffix(".yml")))
        .map(str::to_string)
        .collect()
}

pub struct Installer {
    store: RepoStore,
    /// Where sources are fetched (never inside the gold copy)
    scratch_dir: PathBuf,
}

impl Installer {
    pub fn new(store: RepoStore, scratch_dir: impl Into<PathBuf>) -> Self {
        Self {
            store,
            scratch_dir: scratch_dir.into(),
        }
    }

    fn repos(&self) -> &Repos {
        self.store.repos()
    }

    fn installed(&self) -> Result<InstalledFile, String> {
        self.repos().vendor.load_installed().map_err(String::from)
    }

    /// Projects whatever the files now say. Idempotent: the watcher doing
    /// the same afterwards finds nothing new.
    fn project(&self) {
        self.store.rescan_all(ChangedBy::User);
    }

    /// Fails if any of `agents` / `workflows` already exists, other than
    /// the ones `own` (a re-install or update) already provides.
    fn check_ids_free(
        &self,
        agents: &[String],
        workflows: &[String],
        own: &BTreeSet<String>,
    ) -> Result<(), String> {
        let mut taken = Vec::new();
        for id in agents.iter().filter(|id| !own.contains(*id)) {
            if self.repos().agents.locate(id).is_some() {
                taken.push(format!("agent '{id}'"));
            }
        }
        for id in workflows.iter().filter(|id| !own.contains(*id)) {
            let exists = WorkflowId::parse(id.as_str())
                .is_ok_and(|wf| self.repos().workflows.exists(&wf))
                || self.store.read(|repo| repo.workflow(id).is_some());
            if exists {
                taken.push(format!("workflow '{id}'"));
            }
        }
        if taken.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Can't install: you already have {}. Rename or remove yours first.",
                taken.join(", ")
            ))
        }
    }

    // -- Install --------------------------------------------------------------

    pub fn install_pack(
        &self,
        manifest: &PackManifest,
        token: Option<String>,
        progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<InstalledPackInfo, String> {
        let id = PackId::parse(&manifest.id).map_err(|e| e.to_string())?;
        if !is_compatible(&manifest.min_nandi_version) {
            return Err(newer_nandi(&manifest.name, &manifest.min_nandi_version));
        }
        self.refuse_installed(&id, &manifest.name)?;
        let source = GitSource::new(&manifest.repo, token);
        let resolved = source.resolve(&manifest.git_ref)?;
        check_pinned(
            &manifest.name,
            &manifest.git_ref,
            manifest.commit.as_deref(),
            &resolved,
        )?;
        let checkout = source.fetch(&[&resolved.name], &self.scratch_dir, progress)?;
        let all = checkout.files(&resolved.commit, None)?;
        let pack = read_pack(&all, id.as_str(), &[])?;
        matches_entry(&manifest.name, &manifest.capabilities, &pack.requested)?;
        self.vendor_pack(
            &id,
            pack,
            PackOrigin {
                source: manifest.repo.clone(),
                git_ref: resolved_short(&resolved),
                commit: Some(resolved.commit),
                registry: Some(manifest.registry.clone()),
                signature_verified: manifest.verified,
            },
        )
    }

    /// Installs a pack from a folder or `.zip` on this computer: the same
    /// checks and files as a registry install, recorded as an unverified
    /// local install (`local:<path>`, ref `local`, its tree hash as the
    /// commit, no registry). No network.
    pub fn install_local_pack(
        &self,
        source: &LocalPackSource,
    ) -> Result<InstalledPackInfo, String> {
        let (id, pack) = read_local_pack(source)?;
        self.refuse_installed(&id, &pack.file.name)?;
        self.vendor_pack(
            &id,
            pack,
            PackOrigin {
                source: local_source(source.path()),
                git_ref: LOCAL_REF.to_string(),
                commit: None,
                registry: None,
                signature_verified: false,
            },
        )
    }

    /// The packs installed from this computer, described from their
    /// vendored copies. No registry lists them, so without this Library
    /// would never show them (nor let you uninstall them). A copy that no
    /// longer reads as a pack is left out, with a log line.
    pub fn installed_local_packs(&self) -> Result<Vec<PackManifest>, String> {
        let installed = self.installed()?;
        Ok(installed
            .packs
            .iter()
            .filter(|pack| pack.is_local())
            .filter_map(|pack| {
                let dir = self.repos().layout().pack_dir(pack.id.as_str());
                let described = self
                    .inspect_local_pack(&LocalPackSource::Folder(dir))
                    .map_err(|e| log::warn!("Installed local pack {}: {e}", pack.id))
                    .ok()?;
                Some(PackManifest {
                    repo: pack.source.clone(),
                    latest_version: pack.version.clone(),
                    commit: Some(pack.commit.clone()),
                    installed_version: Some(pack.version.clone()),
                    ..described
                })
            })
            .collect())
    }

    /// What a local pack would install and request, for the consent dialog
    /// shown before installing (the same `PackManifest` a registry lists;
    /// `registry` is `local`, never verified). Validates like an install.
    pub fn inspect_local_pack(&self, source: &LocalPackSource) -> Result<PackManifest, String> {
        let (id, pack) = read_local_pack(source)?;
        let mut agents = Vec::new();
        for agent_id in &pack.agents {
            let prefix = format!("agents/{agent_id}/");
            let own: FileSet = pack
                .files
                .iter()
                .filter_map(|(p, b)| Some((p.strip_prefix(&prefix)?.to_string(), b.clone())))
                .collect();
            let (agent, _) = validate_agent(agent_id, &own, &pack.agents)?;
            agents.push(PackAgentSummary {
                id: agent_id.to_string(),
                name: agent.name().to_string(),
                description: agent.description().to_string(),
            });
        }
        let size: usize = pack.files.values().map(Vec::len).sum();
        let version = pack.file.version.trim_start_matches('v').to_string();
        let installed_version = self
            .installed()?
            .pack(id.as_str())
            .map(|p| p.version.clone());
        Ok(PackManifest {
            id: id.to_string(),
            name: pack.file.name.clone(),
            description: pack.file.description.clone(),
            publisher: String::new(),
            repo: local_source(source.path()),
            latest_version: version,
            git_ref: LOCAL_REF.to_string(),
            tags: Vec::new(),
            min_nandi_version: pack.file.min_nandi_version.clone(),
            agents,
            workflows: pack.workflows.iter().map(ToString::to_string).collect(),
            suggested_asks: Vec::new(),
            capabilities: pack.requested.clone(),
            requires_connectors: super::manifest::connector_ids(
                pack.file.requires_connectors.clone(),
                id.as_str(),
            )?,
            install_size_kb: Some(u32::try_from(size.div_ceil(1024)).unwrap_or(u32::MAX)),
            estimated_cost_usd: None,
            signature: None,
            commit: Some(tree_hash(&pack.files)),
            icon_url: None,
            readme_url: None,
            registry: LOCAL_REGISTRY.to_string(),
            // `read_pack` refused anything this Nandi can't run
            compatible: true,
            installed_version,
            verified: false,
        })
    }

    fn refuse_installed(&self, id: &PackId, name: &str) -> Result<(), String> {
        match self.installed()?.pack(id.as_str()) {
            Some(existing) => Err(format!(
                "{name} is already installed (v{}). {}",
                existing.version,
                if existing.is_local() {
                    "Uninstall it first to install it again."
                } else {
                    "Check for updates instead."
                }
            )),
            None => Ok(()),
        }
    }

    /// Writes a validated pack's files, records it, and re-scans.
    /// `origin.commit` `None`: the tree hash names it.
    fn vendor_pack(
        &self,
        id: &PackId,
        pack: PackContents,
        origin: PackOrigin,
    ) -> Result<InstalledPackInfo, String> {
        let agent_ids: Vec<String> = pack.agents.iter().map(ToString::to_string).collect();
        let mut workflow_ids: Vec<String> =
            pack.workflows.iter().map(ToString::to_string).collect();
        for agent in &agent_ids {
            workflow_ids.extend(scoped_workflow_ids(
                &pack.files,
                &format!("agents/{agent}/"),
            ));
        }
        self.check_ids_free(&agent_ids, &workflow_ids, &BTreeSet::new())?;

        let mut installed = self.installed()?;
        let dest = self.repos().layout().pack_dir(id.as_str());
        self.repos().vendor.put(&dest, &pack.files)?;
        let hash = tree_hash(&pack.files);
        let commit = origin.commit.unwrap_or_else(|| hash.clone());
        let version = pack.file.version.trim_start_matches('v').to_string();
        installed
            .upsert_pack(InstalledPack {
                id: id.clone(),
                source: origin.source.clone(),
                git_ref: origin.git_ref,
                commit: commit.clone(),
                version: version.clone(),
                installed_at: now_rfc3339(),
                registry: origin.registry,
                tree_hash: hash,
                signature_verified: origin.signature_verified,
                agents: pack.agents.clone(),
                workflows: pack.workflows.clone(),
                forked_out: Vec::new(),
            })
            .map_err(|e| e.to_string())?;
        self.repos().vendor.save_installed(&installed)?;
        self.project();
        log::info!(
            "Installed pack '{id}' {} from {} ({commit})",
            pack.file.version,
            origin.source
        );
        Ok(InstalledPackInfo {
            id: id.to_string(),
            version,
            commit,
            agents: agent_ids,
            workflows: pack.workflows.iter().map(ToString::to_string).collect(),
        })
    }

    pub fn install_agent(
        &self,
        manifest: &AgentManifest,
        token: Option<String>,
        progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<InstalledAgentInfo, String> {
        let id = AgentId::parse(&manifest.id).map_err(|e| e.to_string())?;
        let publisher = PublisherHandle::parse(manifest.publisher.to_ascii_lowercase())
            .map_err(|e| e.to_string())?;
        if !is_compatible(&manifest.min_nandi_version) {
            return Err(newer_nandi(&manifest.name, &manifest.min_nandi_version));
        }
        let mut installed = self.installed()?;
        if let Some(existing) = installed.agent(id.as_str()) {
            return Err(format!(
                "{} is already installed (v{} from {}). Check for updates instead.",
                manifest.name, existing.version, existing.publisher
            ));
        }
        if let Some(pack) = installed.pack_of(id.as_str()) {
            return Err(format!(
                "'{id}' is already installed with the pack '{}'",
                pack.id
            ));
        }
        let source = GitSource::new(&manifest.repo, token);
        let resolved = source.resolve(&manifest.git_ref)?;
        check_pinned(
            &manifest.name,
            &manifest.git_ref,
            manifest.commit.as_deref(),
            &resolved,
        )?;
        let checkout = source.fetch(&[&resolved.name], &self.scratch_dir, progress)?;
        let files = select_agent_files(checkout.files(&resolved.commit, Some(&manifest.path))?);
        let (agent, workflows) = validate_agent(&id, &files, std::slice::from_ref(&id))?;
        let mut requested = ManifestCapabilities::default();
        requested.include(super::consent::requested(&agent));
        matches_entry(&manifest.name, &manifest.capabilities, &requested)?;
        let workflow_ids: Vec<String> = workflows.iter().map(ToString::to_string).collect();
        self.check_ids_free(&[id.to_string()], &workflow_ids, &BTreeSet::new())?;

        let dest = self
            .repos()
            .layout()
            .vendored_agent_dir(publisher.as_str(), id.as_str());
        self.repos().vendor.put(&dest, &files)?;
        installed
            .upsert_agent(InstalledAgent {
                id: id.clone(),
                publisher: publisher.clone(),
                source: manifest.repo.clone(),
                git_ref: resolved_short(&resolved),
                commit: resolved.commit.clone(),
                path: manifest.path.clone(),
                version: manifest.latest_version.clone(),
                installed_at: now_rfc3339(),
                registry: Some(manifest.registry.clone()),
                tree_hash: tree_hash(&files),
                signature_verified: manifest.verified,
            })
            .map_err(|e| e.to_string())?;
        self.repos().vendor.save_installed(&installed)?;
        self.project();
        log::info!(
            "Installed agent '{id}' {} from {publisher}",
            manifest.latest_version
        );
        Ok(InstalledAgentInfo {
            id: id.to_string(),
            publisher: publisher.to_string(),
            version: manifest.latest_version.clone(),
            commit: resolved.commit,
        })
    }

    // -- Listing --------------------------------------------------------------

    fn current_tree_hash(
        &self,
        kind: InstallKind,
        id: &str,
        publisher: Option<&str>,
    ) -> Option<String> {
        let layout = self.repos().layout();
        let dir = match kind {
            InstallKind::Pack => layout.pack_dir(id),
            InstallKind::Agent => layout.vendored_agent_dir(publisher?, id),
        };
        self.repos()
            .vendor
            .read(&dir)
            .ok()
            .map(|files| tree_hash(&files))
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledItem>, String> {
        let installed = self.installed()?;
        let name_of = |id: &str| self.store.display_name(id);
        let mut items = Vec::new();
        for pack in &installed.packs {
            let modified = self
                .current_tree_hash(InstallKind::Pack, pack.id.as_str(), None)
                .is_some_and(|hash| hash != pack.tree_hash);
            let dir = self.repos().layout().pack_dir(pack.id.as_str());
            let name = std::fs::read_to_string(dir.join(PACK_MANIFEST_PATH))
                .ok()
                .and_then(|yaml| PackFile::from_yaml(&yaml).ok())
                .map_or_else(|| pack.id.to_string(), |file| file.name);
            items.push(InstalledItem {
                kind: InstallKind::Pack.as_str().to_string(),
                id: pack.id.to_string(),
                name,
                source: pack.source.clone(),
                version: pack.version.clone(),
                git_ref: pack.git_ref.clone(),
                commit: pack.commit.clone(),
                installed_at: pack.installed_at.clone(),
                signature_verified: pack.signature_verified,
                registry: pack.registry.clone(),
                agents: pack.agents.iter().map(ToString::to_string).collect(),
                modified,
            });
        }
        for agent in &installed.agents {
            let modified = self
                .current_tree_hash(
                    InstallKind::Agent,
                    agent.id.as_str(),
                    Some(agent.publisher.as_str()),
                )
                .is_some_and(|hash| hash != agent.tree_hash);
            items.push(InstalledItem {
                kind: InstallKind::Agent.as_str().to_string(),
                id: agent.id.to_string(),
                name: name_of(agent.id.as_str()),
                source: agent.source.clone(),
                version: agent.version.clone(),
                git_ref: agent.git_ref.clone(),
                commit: agent.commit.clone(),
                installed_at: agent.installed_at.clone(),
                signature_verified: agent.signature_verified,
                registry: agent.registry.clone(),
                agents: vec![agent.id.to_string()],
                modified,
            });
        }
        Ok(items)
    }

    /// The installed versions of packs, and of agents.
    pub fn installed_versions(&self) -> (InstalledVersions, InstalledVersions) {
        let installed = self.installed().unwrap_or_default();
        (
            installed
                .packs
                .iter()
                .map(|p| (p.id.to_string(), p.version.clone()))
                .collect(),
            installed
                .agents
                .iter()
                .map(|a| (a.id.to_string(), a.version.clone()))
                .collect(),
        )
    }

    // -- Uninstall --------------------------------------------------------------

    /// The agents uninstalling `kind`/`id` would remove. Refuses forked
    /// agents (they're the user's own), agents that came with a pack (the
    /// pack is uninstalled as a whole), and anything the user's own
    /// workflows or triggers still use.
    pub fn uninstall_plan(&self, kind: InstallKind, id: &str) -> Result<Vec<String>, String> {
        let installed = self.installed()?;
        let (agents, own_workflows): (Vec<String>, Vec<String>) = match kind {
            InstallKind::Agent => {
                if installed.agent(id).is_none() {
                    if let Some(pack) = installed.pack_of(id) {
                        return Err(format!(
                            "'{id}' came with the pack '{}'. Uninstall the pack, or fork the agent to keep it.",
                            pack.id
                        ));
                    }
                    return Err(match self.repos().agents.locate(id) {
                        Some(place) if !place.origin.is_vendored() => format!(
                            "'{id}' is your own agent (forked or created here), not a marketplace install. Delete it from the agent's menu instead."
                        ),
                        _ => format!("'{id}' isn't installed from the marketplace"),
                    });
                }
                let workflows = self.store.read(|repo| {
                    repo.workflows()
                        .filter(|w| matches!(w.scope(), crate::services::domain::workflow::WorkflowScope::Agent(a) if a.as_str() == id))
                        .map(|w| w.id().to_string())
                        .collect()
                });
                (vec![id.to_string()], workflows)
            }
            InstallKind::Pack => {
                let pack = installed
                    .pack(id)
                    .ok_or_else(|| format!("The pack '{id}' isn't installed"))?;
                let agents: Vec<String> = pack.agents.iter().map(ToString::to_string).collect();
                let mut workflows: Vec<String> =
                    pack.workflows.iter().map(ToString::to_string).collect();
                self.store.read(|repo| {
                    for workflow in repo.workflows() {
                        if let crate::services::domain::workflow::WorkflowScope::Agent(a) =
                            workflow.scope()
                        {
                            if agents.iter().any(|x| x == a.as_str()) {
                                workflows.push(workflow.id().to_string());
                            }
                        }
                    }
                });
                (agents, workflows)
            }
        };
        let users: Vec<String> = self.store.read(|repo| {
            let mut users = Vec::new();
            let removed_workflows: Vec<String> = own_workflows
                .iter()
                .map(|w| format!("workflow '{w}'"))
                .collect();
            for agent in &agents {
                users.extend(
                    repo.references_to(EntityKind::Agent, agent)
                        .into_iter()
                        .filter(|by| !removed_workflows.contains(by)),
                );
            }
            for workflow in &own_workflows {
                users.extend(repo.references_to(EntityKind::Workflow, workflow));
            }
            users.sort();
            users.dedup();
            users
        });
        if !users.is_empty() {
            return Err(format!(
                "Can't uninstall '{id}': it's still used by {}. Change or remove those first.",
                users.join(", ")
            ));
        }
        Ok(agents)
    }

    /// Removes the folder (to the trash), its `installed.yaml` entry, and
    /// re-scans. The caller has stopped the agents (`uninstall_plan`).
    pub fn uninstall(&self, kind: InstallKind, id: &str) -> Result<Vec<String>, String> {
        let agents = self.uninstall_plan(kind, id)?;
        let mut installed = self.installed()?;
        let layout = self.repos().layout();
        let dir = match kind {
            InstallKind::Pack => {
                installed.remove_pack(id);
                layout.pack_dir(id)
            }
            InstallKind::Agent => {
                let entry = installed
                    .remove_agent(id)
                    .ok_or_else(|| format!("'{id}' isn't installed"))?;
                layout.vendored_agent_dir(entry.publisher.as_str(), id)
            }
        };
        if dir.exists() {
            self.repos().vendor.remove(&dir)?;
        }
        self.repos().vendor.save_installed(&installed)?;
        self.project();
        log::info!("Uninstalled {} '{id}'", kind.as_str());
        Ok(agents)
    }

    // -- Fork -------------------------------------------------------------------

    /// Moves a marketplace agent into the user's own agents as `new_id`:
    /// copies its folder to `.nandi/agents/<new_id>/`, rewrites skill.md
    /// (new ID and name, source metadata dropped) and its own workflows
    /// (pointed at the new ID), then removes the vendored copy and its
    /// `installed.yaml` entry. Updates stop for it.
    pub fn fork_agent(
        &self,
        installed_id: &str,
        new_id: &str,
        new_name: &str,
    ) -> Result<Forked, String> {
        let source_id = AgentId::parse(installed_id).map_err(|e| e.to_string())?;
        let new_id = AgentId::parse(new_id).map_err(|e| e.to_string())?;
        let place = self
            .repos()
            .agents
            .locate(installed_id)
            .ok_or_else(|| format!("Agent '{installed_id}' not found"))?;
        if !place.origin.is_vendored() {
            return Err(format!(
                "'{installed_id}' is already your own agent; there's nothing to fork"
            ));
        }
        if new_id == source_id {
            return Err("Choose a new ID for the fork".to_string());
        }
        if new_id.as_str() == crate::services::repos::layout::MARKETPLACE_DIR
            || self.repos().agents.locate(new_id.as_str()).is_some()
        {
            return Err(format!("An agent with id '{new_id}' already exists"));
        }
        let original = self.repos().agents.load(&source_id).map_err(String::from)?;
        let fork = original
            .forked(new_id.clone(), new_name)
            .map_err(|e| describe_error(new_id.as_str(), &e))?;

        // Others that name the original: they'll need pointing at the fork
        let own_workflows: Vec<String> =
            scoped_workflow_ids(&self.repos().vendor.read(&place.dir)?, "");
        let warnings: Vec<String> = self.store.read(|repo| {
            repo.references_to(EntityKind::Agent, installed_id)
                .into_iter()
                .filter(|by| {
                    !own_workflows
                        .iter()
                        .any(|w| *by == format!("workflow '{w}'"))
                })
                .map(|by| format!("{by} still uses '{installed_id}'"))
                .collect()
        });

        let mut installed = self.installed()?;
        self.repos().vendor.copy_to_local(&place.dir, &new_id)?;
        self.repos().vendor.remove(&place.dir)?;
        self.repos().agents.save(&fork)?;
        for workflow in &own_workflows {
            let Ok(workflow_id) = WorkflowId::parse(workflow.as_str()) else {
                continue;
            };
            match self.repos().workflows.load(&workflow_id) {
                Ok(mut loaded) => {
                    if loaded.rename_agent(&source_id, &new_id) {
                        self.repos().workflows.save(&loaded)?;
                    }
                }
                Err(e) => log::warn!("Forked workflow '{workflow}' not rewritten: {e}"),
            }
        }
        if installed.remove_agent(installed_id).is_none() {
            installed.mark_forked_out(&source_id);
            if let agent_locator::Origin::Pack { pack } = &place.origin {
                // The pack's folder changed by design: record its new hash
                if let (Some(hash), Some(entry)) = (
                    self.current_tree_hash(InstallKind::Pack, pack, None),
                    installed.packs.iter_mut().find(|p| p.id.as_str() == pack),
                ) {
                    entry.tree_hash = hash;
                }
            }
        }
        self.repos().vendor.save_installed(&installed)?;
        self.project();
        log::info!("Forked '{installed_id}' into '{new_id}'");
        let agent = self.repos().agents.load(&new_id).map_err(String::from)?;
        Ok(Forked { agent, warnings })
    }

    // -- Updates ----------------------------------------------------------------

    /// Compares every install with its source's newest release tag (or, for
    /// a branch install, the branch's head). Never changes anything.
    /// Returns the updates, and the sources that couldn't be checked.
    pub fn check_for_updates(
        &self,
        token_for: &dyn Fn(Option<&str>) -> Option<String>,
    ) -> Result<UpdateCheck, String> {
        let installed = self.installed()?;
        // A local install has no source to check
        let entries = installed
            .packs
            .iter()
            .filter(|p| !p.is_local())
            .map(|p| {
                (
                    InstallKind::Pack,
                    p.id.to_string(),
                    p.source.as_str(),
                    p.git_ref.as_str(),
                    p.version.as_str(),
                    p.commit.as_str(),
                    p.registry.as_deref(),
                )
            })
            .chain(installed.agents.iter().map(|a| {
                (
                    InstallKind::Agent,
                    a.id.to_string(),
                    a.source.as_str(),
                    a.git_ref.as_str(),
                    a.version.as_str(),
                    a.commit.as_str(),
                    a.registry.as_deref(),
                )
            }));
        let mut updates = Vec::new();
        let mut failures = Vec::new();
        for (kind, id, source, git_ref, version, commit, registry) in entries {
            let source = GitSource::new(source, token_for(registry));
            let result = (|| {
                let refs = source.refs()?;
                if let Some((prefix, _)) = super::manifest::split_tag(git_ref) {
                    let tags = super::source::release_tags_with_prefix(refs, &prefix);
                    let current = parse_version(version)?;
                    Ok(tags
                        .into_iter()
                        .next()
                        .filter(|(latest, _)| *latest > current)
                        .map(|(latest, r)| (latest.to_string(), r)))
                } else {
                    // A branch install: any new commit is an update
                    Ok(super::source::resolve_in(&refs, git_ref)
                        .filter(|r| r.commit != commit)
                        .map(|r| {
                            (
                                format!("{version}+{}", &r.commit[..7.min(r.commit.len())]),
                                r,
                            )
                        }))
                }
            })();
            match result {
                Ok(Some((latest_version, latest))) => updates.push(UpdateAvailable {
                    kind: kind.as_str().to_string(),
                    id,
                    current_version: version.to_string(),
                    latest_version,
                    latest_ref: resolved_short(&latest),
                    current_commit: commit.to_string(),
                    latest_commit: latest.commit,
                }),
                Ok(None) => {}
                Err::<_, String>(e) => failures.push((id, e)),
            }
        }
        Ok((updates, failures))
    }

    /// Files under an install that differ from what was installed (added,
    /// changed, or removed), by fetching the installed commit again.
    fn modified_files(
        &self,
        source: &GitSource,
        git_ref: &str,
        commit: &str,
        current: &FileSet,
        expected: impl FnOnce(&FileSet) -> Result<FileSet, String>,
    ) -> Vec<String> {
        let result = (|| {
            let resolved = source.resolve(git_ref)?;
            let checkout = source.fetch(&[&resolved.name], &self.scratch_dir, &mut |_| {})?;
            expected(&checkout.files(commit, None)?)
        })();
        let Ok(original) = result else {
            return vec!["(the installed version can no longer be fetched to compare)".to_string()];
        };
        let mut changed: Vec<String> = current
            .iter()
            .filter(|(path, bytes)| original.get(*path) != Some(*bytes))
            .map(|(path, _)| path.clone())
            .chain(
                original
                    .keys()
                    .filter(|path| !current.contains_key(*path))
                    .map(|path| format!("{path} (removed)")),
            )
            .collect();
        changed.sort();
        changed
    }

    /// Updates an install to `new_ref`. Refused if its files were changed
    /// since install (listing them), or if the new version needs a newer
    /// Nandi. Never runs on its own: only when the user asks.
    pub fn apply_update(
        &self,
        installed_id: &str,
        new_ref: &str,
        token: Option<String>,
        verified_commit: Option<&str>,
        progress: &mut dyn FnMut(FetchProgress),
    ) -> Result<AppliedUpdate, String> {
        let mut installed = self.installed()?;
        let kind = if installed.agent(installed_id).is_some() {
            InstallKind::Agent
        } else if installed.pack(installed_id).is_some() {
            InstallKind::Pack
        } else {
            return Err(format!(
                "'{installed_id}' isn't installed from the marketplace"
            ));
        };
        if installed
            .pack(installed_id)
            .is_some_and(InstalledPack::is_local)
        {
            return Err(format!(
                "'{installed_id}' was installed from a folder on this computer, so it has no updates. Uninstall it, then install the new version from its folder."
            ));
        }
        let layout = self.repos().layout();
        let (source_repo, git_ref, commit, version, hash, dir, path, forked_out) = match kind {
            InstallKind::Agent => {
                let a = installed.agent(installed_id).expect("checked above");
                (
                    a.source.clone(),
                    a.git_ref.clone(),
                    a.commit.clone(),
                    a.version.clone(),
                    a.tree_hash.clone(),
                    layout.vendored_agent_dir(a.publisher.as_str(), installed_id),
                    Some(a.path.clone()),
                    Vec::new(),
                )
            }
            InstallKind::Pack => {
                let p = installed.pack(installed_id).expect("checked above");
                (
                    p.source.clone(),
                    p.git_ref.clone(),
                    p.commit.clone(),
                    p.version.clone(),
                    p.tree_hash.clone(),
                    layout.pack_dir(installed_id),
                    None,
                    p.forked_out.clone(),
                )
            }
        };
        let source = GitSource::new(&source_repo, token);
        let current = self.repos().vendor.read(&dir).unwrap_or_default();
        if tree_hash(&current) != hash {
            let changed =
                self.modified_files(&source, &git_ref, &commit, &current, |all| match &path {
                    Some(path) => {
                        let prefix = format!("{path}/");
                        Ok(select_agent_files(
                            all.iter()
                                .filter_map(|(p, b)| {
                                    Some((p.strip_prefix(&prefix)?.to_string(), b.clone()))
                                })
                                .collect(),
                        ))
                    }
                    None => read_pack(all, installed_id, &forked_out).map(|p| p.files),
                });
            return Err(format!(
                "Can't update '{installed_id}': its files were changed since it was installed: {}. Fork it to keep your changes, or uninstall and reinstall it.",
                changed.join(", ")
            ));
        }

        let resolved = source.resolve(new_ref)?;
        let checkout = source.fetch(&[&resolved.name], &self.scratch_dir, progress)?;
        let (files, new_version, agents, workflows) = match &path {
            Some(path) => {
                let files = select_agent_files(checkout.files(&resolved.commit, Some(path))?);
                let id = AgentId::parse(installed_id).map_err(|e| e.to_string())?;
                let (_, workflows) = validate_agent(&id, &files, std::slice::from_ref(&id))?;
                let version = super::manifest::split_tag(&resolved.name)
                    .map_or_else(|| version.clone(), |(_, v)| v.to_string());
                (files, version, vec![id], workflows)
            }
            None => {
                let pack = read_pack(
                    &checkout.files(&resolved.commit, None)?,
                    installed_id,
                    &forked_out,
                )?;
                let mut workflows = pack.workflows.clone();
                for agent in &pack.agents {
                    for id in scoped_workflow_ids(&pack.files, &format!("agents/{agent}/")) {
                        workflows.extend(WorkflowId::parse(id).ok());
                    }
                }
                let version = pack.file.version.trim_start_matches('v').to_string();
                (pack.files, version, pack.agents, workflows)
            }
        };
        // IDs this install already provides may stay; new ones must be free
        let own: BTreeSet<String> = self.store.read(|repo| {
            repo.agents()
                .filter(|a| {
                    agent_locator::locate(layout, a.id().as_str())
                        .is_some_and(|p| p.dir.starts_with(&dir))
                })
                .map(|a| a.id().to_string())
                .chain(
                    repo.workflows()
                        .filter(|w| match w.scope() {
                            crate::services::domain::workflow::WorkflowScope::Agent(a) => {
                                agent_locator::locate(layout, a.as_str())
                                    .is_some_and(|p| p.dir.starts_with(&dir))
                            }
                            crate::services::domain::workflow::WorkflowScope::Global => self
                                .repos()
                                .workflows
                                .locate(w.id())
                                .is_some_and(|(p, _)| p.starts_with(&dir)),
                        })
                        .map(|w| w.id().to_string()),
                )
                .collect()
        });
        let agent_ids: Vec<String> = agents.iter().map(ToString::to_string).collect();
        let workflow_ids: Vec<String> = workflows.iter().map(ToString::to_string).collect();
        self.check_ids_free(&agent_ids, &workflow_ids, &own)?;

        self.repos().vendor.put(&dir, &files)?;
        let new_hash = tree_hash(&files);
        // Signed for exactly this commit (checked by the caller against the
        // publisher's key)
        let verified = verified_commit.is_some_and(|c| c.eq_ignore_ascii_case(&resolved.commit));
        match kind {
            InstallKind::Agent => {
                let entry = installed
                    .agents
                    .iter_mut()
                    .find(|a| a.id.as_str() == installed_id)
                    .expect("checked above");
                entry.git_ref = resolved_short(&resolved);
                entry.commit = resolved.commit.clone();
                entry.version = new_version.clone();
                entry.tree_hash = new_hash;
                entry.installed_at = now_rfc3339();
                entry.signature_verified = verified;
            }
            InstallKind::Pack => {
                let entry = installed
                    .packs
                    .iter_mut()
                    .find(|p| p.id.as_str() == installed_id)
                    .expect("checked above");
                entry.git_ref = resolved_short(&resolved);
                entry.commit = resolved.commit.clone();
                entry.version = new_version.clone();
                entry.tree_hash = new_hash;
                entry.installed_at = now_rfc3339();
                entry.signature_verified = verified;
                entry.agents = agents;
                entry.workflows = workflows
                    .into_iter()
                    .filter(|w| {
                        files.contains_key(&format!("workflows/{w}.yaml"))
                            || files.contains_key(&format!("workflows/{w}.yml"))
                    })
                    .collect();
            }
        }
        self.repos().vendor.save_installed(&installed)?;
        self.project();
        log::info!(
            "Updated {} '{installed_id}' {version} → {new_version}",
            kind.as_str()
        );
        Ok(AppliedUpdate {
            kind: kind.as_str().to_string(),
            id: installed_id.to_string(),
            from_version: version,
            to_version: new_version,
        })
    }
}

/// An entry that names its release commit must still resolve to it: a tag
/// that moved since it was listed (and signed) is refused.
fn check_pinned(
    name: &str,
    git_ref: &str,
    pinned: Option<&str>,
    resolved: &RemoteRef,
) -> Result<(), String> {
    match pinned {
        Some(pinned) if !pinned.eq_ignore_ascii_case(&resolved.commit) => Err(format!(
            "Refusing to install {name}: its '{git_ref}' tag now points to commit {} but the registry lists {pinned}. The source may have been tampered with; ask the publisher.",
            &resolved.commit[..12.min(resolved.commit.len())]
        )),
        _ => Ok(()),
    }
}

/// `refs/tags/v1.2.0` → `v1.2.0`; `refs/heads/main` → `main`.
fn resolved_short(resolved: &RemoteRef) -> String {
    resolved
        .name
        .strip_prefix("refs/tags/")
        .or_else(|| resolved.name.strip_prefix("refs/heads/"))
        .unwrap_or(&resolved.name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file under `dir`, keyed by its `/`-separated relative path
    fn read_dir(dir: &std::path::Path) -> FileSet {
        let mut files = FileSet::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(current) = pending.pop() {
            for entry in std::fs::read_dir(&current).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().is_some_and(|name| name == ".git") {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let relative = path.strip_prefix(dir).unwrap();
                    let key = relative.to_string_lossy().replace('\\', "/");
                    files.insert(key, std::fs::read(&path).unwrap());
                }
            }
        }
        files
    }

    fn pack(files: &[(&str, &str)]) -> FileSet {
        files
            .iter()
            .map(|(path, text)| (path.to_string(), text.as_bytes().to_vec()))
            .collect()
    }

    const MANIFEST: &str = "id: social
name: Social
version: 0.1.0
agents: [poster]
";
    const POSTER: &str = "---
name: Poster
description: Posts
---
You post.
";

    #[test]
    fn an_agent_brings_its_playbooks_but_no_scripts() {
        let files = pack(&[
            ("skill.md", POSTER),
            ("skills/post.md", "playbook"),
            ("skills/deep/x.md", "nested"),
            ("skills/run.sh", "script"),
            ("scripts/x.sh", "script"),
        ]);
        let kept: Vec<String> = select_agent_files(files).into_keys().collect();
        assert_eq!(kept, ["skill.md", "skills/post.md"]);
    }

    #[test]
    fn a_pack_ships_policies_for_its_own_agents_only() {
        let own = "---
applies_to: [agent:poster]
browser: { refuse: [{ controls: [post] }] }
---
Why.
";
        let files = pack(&[
            (PACK_MANIFEST_PATH, MANIFEST),
            ("agents/poster/skill.md", POSTER),
            ("policies/posting.md", own),
            ("policies/notes.txt", "left behind"),
        ]);
        let read = read_pack(&files, "social", &[]).unwrap();
        assert!(read.files.contains_key("policies/posting.md"));
        assert!(!read.files.contains_key("policies/notes.txt"));

        for (bad, why) in [
            (
                "---
applies_to: ['*']
tools: { deny: [shell] }
---
",
                "everyone",
            ),
            (
                "---
applies_to: [agent:po*]
tools: { deny: [shell] }
---
",
                "a pattern",
            ),
            (
                "---
applies_to: [tag:social]
tools: { deny: [shell] }
---
",
                "a tag",
            ),
            (
                "---
applies_to: [agent:other]
tools: { deny: [shell] }
---
",
                "another agent",
            ),
            (
                "Be kind.
",
                "prose: no rules",
            ),
            (
                "---
applies_to: [agent:poster]
browser: { refuse: [{}] }
---
",
                "broken",
            ),
        ] {
            let files = pack(&[
                (PACK_MANIFEST_PATH, MANIFEST),
                ("agents/poster/skill.md", POSTER),
                ("policies/posting.md", bad),
            ]);
            assert!(read_pack(&files, "social", &[]).is_err(), "{why}");
        }
    }

    /// For pack authors: `NANDI_PACK_DIR=<checkout> cargo test --lib
    /// local_pack_checkout_installs -- --ignored` validates a pack the way
    /// an install does (manifest, agents, workflows)
    #[test]
    #[ignore = "reads a local pack checkout named by NANDI_PACK_DIR"]
    fn local_pack_checkout_installs() {
        let dir = std::env::var("NANDI_PACK_DIR").expect("set NANDI_PACK_DIR");
        let all = read_dir(std::path::Path::new(&dir));
        let manifest = text(&all, PACK_MANIFEST_PATH).unwrap().unwrap();
        let id = PackFile::from_yaml(manifest).unwrap().id.to_string();
        let pack = read_pack(&all, &id, &[]).unwrap();
        assert!(!pack.agents.is_empty() || !pack.workflows.is_empty());
        println!(
            "{id} {}: agents {:?}, workflows {:?}",
            pack.file.version, pack.agents, pack.workflows
        );
    }
}
