//! Saves an artifact from Studio. The text is validated as that
//! kind of artifact, then written by the repository or service that owns
//! it (gold copy: `services/repos/` are the only writers), so projections
//! and events follow as for any other edit. Then:
//!
//! - `LocalOnly`: commit just that file on the current branch;
//! - `RemoteBranch`: … and push to `<prefix>/<slug>` (the remote service,
//!   which never pushes to main or master);
//! - `RemoteBranchAndPr`: … and a pull request, which isn't implemented
//!   yet: the push happens and a warning says to open the PR by hand.
//!
//! Capability saves must bump the version as much as the change needs
//! (`version_bumper`).

use std::path::PathBuf;

use super::{artifact_loader, ArtifactSave, ArtifactSaveResult, PushTarget, StudioError};
use crate::services::ask::context::artifact_label;
use crate::services::ask::ArtifactRef;
use crate::services::capability::definition;
use crate::services::domain::{self, AgentId, EntityKind};
use crate::services::gateway::manifest;
use crate::services::repos::Repository;
use crate::services::AppState;

use super::version_bumper;

/// Writes, commits, and pushes `save`
pub async fn save(state: &AppState, save: ArtifactSave) -> Result<ArtifactSaveResult, StudioError> {
    let mut warnings = Vec::new();
    let message = match save.commit_message.trim() {
        "" => format!("Update {}", artifact_label(&save.artifact).replace('`', "")),
        message => message.to_string(),
    };
    // Refuse before anything is written
    if save.push_target != PushTarget::LocalOnly {
        remote_branch(state).ok_or("No remote is configured: add one in Settings › Backup")?;
    }
    let paths = write(state, &save.artifact, &save.content).await?;
    let relative: Vec<String> = paths
        .iter()
        .filter_map(|p| state.store().layout().relative(p))
        .collect();
    let git = state.git_service().clone();
    let commit = tokio::task::spawn_blocking(move || git.commit_files(&relative, &message, None))
        .await
        .map_err(|e| format!("Commit failed: {e}"))?;
    let commit = match commit {
        Ok(commit) => commit,
        Err(e) if e.contains("isn't a git repository") => {
            warnings.push(format!("Saved without a commit: {e}"));
            None
        }
        Err(e) => return Err(e),
    };
    let mut pushed_branch = None;
    if save.push_target != PushTarget::LocalOnly {
        let remote = state.remote().clone();
        let pushed = remote.push(false).await?;
        if pushed.conflict {
            warnings.push(format!(
                "{} has commits you don't: pull, then push again",
                pushed.target_branch
            ));
        } else {
            pushed_branch = Some(pushed.target_branch.clone());
        }
        if save.push_target == PushTarget::RemoteBranchAndPr {
            warnings.push(format!(
                "Opening a pull request isn't implemented yet: open one from {} on your git host",
                pushed.target_branch
            ));
        }
    }
    Ok(ArtifactSaveResult {
        artifact: save.artifact,
        commit,
        pushed_branch,
        warnings,
    })
}

/// `<prefix>/<slug>` when a remote is configured
pub fn remote_branch(state: &AppState) -> Option<String> {
    let config = crate::utils::nandi_config::load_from(state.nandi_dir())
        .ok()?
        .remote?;
    Some(format!("{}/{}", config.branch_prefix, config.user_slug))
}

/// Validates `content` and writes it through its owner; returns the
/// files written
async fn write(
    state: &AppState,
    artifact: &ArtifactRef,
    content: &str,
) -> Result<Vec<PathBuf>, StudioError> {
    match artifact {
        ArtifactRef::Capability(id) => {
            let mut new = definition::parse(content)?;
            if new.id.trim().is_empty() {
                new.id = id.clone();
            }
            if &new.id != id {
                return Err(format!(
                    "A capability's ID can't change ('{id}' → '{}'); create a new capability instead",
                    new.id
                ));
            }
            let service = state
                .capabilities()
                .ok_or("The capability service is unavailable")?;
            if let Ok(old) = service.repo().load(id) {
                let check = version_bumper::check(&old, &new)?;
                if check.missing() {
                    return Err(format!(
                        "This change needs a {} version bump: set version to {}",
                        check.required,
                        version_bumper::bumped(&old.version, check.required)?
                    ));
                }
            }
            service.save(&new).await?;
            Ok(vec![service.repo().path(id)])
        }
        ArtifactRef::Agent(id) => {
            let agent_id = AgentId::parse(id).map_err(|e| e.to_string())?;
            let store = state.store().clone();
            let content = content.to_string();
            let created = tokio::task::spawn_blocking(move || {
                let agents = &store.repos().agents;
                // A new ID is a new agent; an existing one keeps its
                // instructions.md
                let existing = match agents.locate(agent_id.as_str()) {
                    Some(_) => Some(agents.load(&agent_id).map_err(String::from)?),
                    None => None,
                };
                let agent = domain::Agent::from_files(
                    agent_id.clone(),
                    &content,
                    existing.as_ref().map(|a| a.instructions()),
                )
                .map_err(|e| format!("Not a valid agent: {e}"))?;
                let path = match existing {
                    Some(_) => agents.save(&agent),
                    None => agents.create(&agent),
                }
                .map_err(String::from)?;
                store.commit(EntityKind::Agent, agent_id.as_str())?;
                Ok::<_, String>(vec![path])
            })
            .await
            .map_err(|e| format!("Agent write failed: {e}"))??;
            // The reactor already did on `repo://agent-created`; idempotent
            state.orchestrators().ensure(id);
            Ok(created)
        }
        ArtifactRef::Workflow(id) => {
            let workflow = domain::Workflow::from_yaml(content)
                .map_err(|e| format!("Not a valid workflow: {e}"))?;
            if workflow.id().as_str() != id {
                return Err(format!(
                    "A workflow's ID can't change ('{id}' → '{}')",
                    workflow.id()
                ));
            }
            let store = state.store().clone();
            let content = content.to_string();
            tokio::task::spawn_blocking(move || {
                let workflows = &store.repos().workflows;
                let path = match workflows.locate(workflow.id()) {
                    Some(_) => workflows.save_source(&workflow, &content),
                    None => workflows.create_source(&workflow, &content),
                }
                .map_err(String::from)?;
                store.commit(EntityKind::Workflow, workflow.id().as_str())?;
                Ok(vec![path])
            })
            .await
            .map_err(|e| format!("Workflow write failed: {e}"))?
        }
        ArtifactRef::Policy(id) => {
            if !artifact_loader::valid_policy_id(id) {
                return Err(format!("'{id}' isn't a valid policy ID"));
            }
            // The parser the app enforces with: a policy that would block
            // agents (or apply to nobody) isn't saved
            crate::services::domain::policy::Policy::parse(id, content)
                .map_err(|e| format!("Not a valid policy: {e}"))?;
            let path = state
                .store()
                .repos()
                .policies
                .save(id, content)
                .map_err(String::from)?;
            Ok(vec![path])
        }
        ArtifactRef::Gateway => {
            let parsed =
                manifest::parse(content).map_err(|e| format!("Not a valid NANDI.md: {e}"))?;
            let gateway = state.gateway().ok_or("The gateway is unavailable")?;
            gateway.save_manifest(&parsed)?;
            Ok(vec![manifest::manifest_path(&state.store().layout().root)])
        }
    }
}
