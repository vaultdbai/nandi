//! Studio's artifact editor (wave CD, Tracks C and D): load any artifact
//! (capability, agent, workflow, policy, NANDI.md) as its raw file plus a
//! parsed view, save it through the repository that owns it, commit, and
//! optionally push to `<prefix>/<slug>`.

pub mod artifact_loader;
pub mod artifact_saver;
pub mod connector;
pub mod guides;
pub mod prompt;
pub mod save;
pub mod start;
pub mod tools;
pub mod validate;
pub mod version_bumper;

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::ask::{self, ArtifactRef, AskContext, EditPreview};
use crate::services::events::{self, NandiEvent};
use crate::services::{AppState, WeakAppState};

pub type StudioError = String;

/// `studio://artifact_loaded`
pub const STUDIO_ARTIFACT_LOADED: &str = "studio://artifact_loaded";
/// `studio://artifact_saved`
pub const STUDIO_ARTIFACT_SAVED: &str = "studio://artifact_saved";
/// `studio://edit_previewed`
pub const STUDIO_EDIT_PREVIEWED: &str = "studio://edit_previewed";

/// An artifact opened for editing
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactLoad {
    pub artifact: ArtifactRef,
    /// The raw file
    pub content: String,
    /// `{ frontmatter, body }` for structured editing
    pub parsed: serde_json::Value,
    pub path: PathBuf,
}

/// Where a save goes after the local commit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PushTarget {
    /// `git add` + `git commit`
    LocalOnly,
    /// … + `git push` to `<prefix>/<slug>`
    RemoteBranch,
    /// … + open a pull request (not implemented yet)
    RemoteBranchAndPr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSave {
    pub artifact: ArtifactRef,
    pub content: String,
    pub commit_message: String,
    pub push_target: PushTarget,
}

/// The outcome of a save
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSaveResult {
    pub artifact: ArtifactRef,
    /// The new commit, or `None` when the file was unchanged
    pub commit: Option<String>,
    /// The branch pushed to, when pushed
    pub pushed_branch: Option<String>,
    /// Not fatal: e.g. the PR step isn't available
    pub warnings: Vec<String>,
}

/// A push target and whether it can be used now
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PushTargetOption {
    pub target: PushTarget,
    pub available: bool,
    /// The branch it pushes to, when it pushes
    pub branch: Option<String>,
    /// Why it isn't available
    pub reason: Option<String>,
}

/// `studio://artifact_loaded`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StudioArtifactLoaded {
    pub artifact: ArtifactRef,
}

impl NandiEvent for StudioArtifactLoaded {
    const NAME: &'static str = STUDIO_ARTIFACT_LOADED;
}

/// `studio://artifact_saved`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StudioArtifactSaved {
    pub result: ArtifactSaveResult,
}

impl NandiEvent for StudioArtifactSaved {
    const NAME: &'static str = STUDIO_ARTIFACT_SAVED;
}

/// `studio://edit_previewed`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StudioEditPreviewed {
    pub preview: EditPreview,
}

impl NandiEvent for StudioEditPreviewed {
    const NAME: &'static str = STUDIO_EDIT_PREVIEWED;
}

/// Studio's artifact editor. Holds the state weakly: the state owns it.
pub struct StudioService {
    state: WeakAppState,
    /// The file open in Studio's editor, which Studio's chat reads as
    /// "this" (`prompt::PromptFacts::open_artifact`)
    open_artifact: std::sync::Mutex<Option<ArtifactRef>>,
}

impl StudioService {
    pub fn new(state: WeakAppState) -> Self {
        Self {
            state,
            open_artifact: std::sync::Mutex::default(),
        }
    }

    pub fn open_artifact(&self) -> Option<ArtifactRef> {
        self.open_artifact
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_open_artifact(&self, artifact: Option<ArtifactRef>) {
        *self.open_artifact.lock().unwrap_or_else(|e| e.into_inner()) = artifact;
    }

    fn state(&self) -> Result<AppState, StudioError> {
        self.state
            .upgrade()
            .ok_or_else(|| "The app is shutting down".to_string())
    }

    pub async fn load(&self, artifact: ArtifactRef) -> Result<ArtifactLoad, StudioError> {
        let state = self.state()?;
        let loaded = artifact_loader::load(&state, &artifact)?;
        events::emit(state.bus(), &StudioArtifactLoaded { artifact });
        Ok(loaded)
    }

    pub async fn save(&self, save: ArtifactSave) -> Result<ArtifactSaveResult, StudioError> {
        let state = self.state()?;
        let result = artifact_saver::save(&state, save).await?;
        events::emit(
            state.bus(),
            &StudioArtifactSaved {
                result: result.clone(),
            },
        );
        Ok(result)
    }

    /// The edit as the ask box's editor would make it; nothing is written
    pub async fn preview(
        &self,
        artifact: ArtifactRef,
        instruction: String,
    ) -> Result<EditPreview, StudioError> {
        let state = self.state()?;
        let ask = state.ask().ok_or("The ask box is unavailable")?.clone();
        let context = ask::context::load(
            &state,
            &AskContext {
                current_view: Some("studio".to_string()),
                focused_artifact: Some(artifact.clone()),
                recent_runs: Vec::new(),
            },
        )
        .await;
        let preview = ask::editor::preview(
            &state,
            ask.llm().as_ref(),
            &artifact,
            &instruction,
            Some(&context),
        )
        .await?;
        events::emit(
            state.bus(),
            &StudioEditPreviewed {
                preview: preview.clone(),
            },
        );
        Ok(preview)
    }

    /// Local commits always (in a git repo); pushes once a remote is set
    pub fn push_targets(&self) -> Result<Vec<PushTargetOption>, StudioError> {
        let state = self.state()?;
        let is_repo = git2::Repository::open(&state.store().layout().root).is_ok();
        let branch = artifact_saver::remote_branch(&state);
        let no_remote =
            || Some("No remote is configured: add one in Settings › Backup".to_string());
        Ok(vec![
            PushTargetOption {
                target: PushTarget::LocalOnly,
                available: true,
                branch: None,
                reason: (!is_repo)
                    .then(|| "The repo isn't under git yet: saves won't be committed".to_string()),
            },
            PushTargetOption {
                target: PushTarget::RemoteBranch,
                available: branch.is_some(),
                branch: branch.clone(),
                reason: if branch.is_some() { None } else { no_remote() },
            },
            PushTargetOption {
                target: PushTarget::RemoteBranchAndPr,
                available: branch.is_some(),
                branch: branch.clone(),
                reason: if branch.is_some() {
                    Some("Pushes the branch; open the pull request on your git host (automatic pull requests aren't available yet)".to_string())
                } else {
                    no_remote()
                },
            },
        ])
    }
}

pub fn init(state: &AppState) -> Option<Arc<StudioService>> {
    Some(Arc::new(StudioService::new(state.downgrade())))
}

fn service(state: &AppState) -> Result<&Arc<StudioService>, StudioError> {
    state
        .studio()
        .ok_or_else(|| "Studio is unavailable".to_string())
}

/// What's open in Studio's editor (`None`: nothing), for Studio's chat
pub fn set_open_artifact(
    state: &AppState,
    artifact: Option<ArtifactRef>,
) -> Result<(), StudioError> {
    service(state)?.set_open_artifact(artifact);
    Ok(())
}

pub async fn load_artifact(
    state: &AppState,
    artifact: ArtifactRef,
) -> Result<ArtifactLoad, StudioError> {
    service(state)?.load(artifact).await
}

pub async fn save_artifact(
    state: &AppState,
    save: ArtifactSave,
) -> Result<ArtifactSaveResult, StudioError> {
    service(state)?.save(save).await
}

pub async fn preview_edit(
    state: &AppState,
    artifact: ArtifactRef,
    instruction: String,
) -> Result<EditPreview, StudioError> {
    service(state)?.preview(artifact, instruction).await
}

pub async fn list_push_targets(state: &AppState) -> Result<Vec<PushTargetOption>, StudioError> {
    service(state)?.push_targets()
}

/// Checks `content` as `artifact` the way saving would, plus unknown tools,
/// models without a key and missing agents (`validate`)
pub async fn validate_artifact(
    state: &AppState,
    artifact: ArtifactRef,
    content: String,
) -> Result<validate::ArtifactValidation, StudioError> {
    let facts = tools::validation_facts(state).await;
    Ok(validate::validate(&artifact, &content, &facts))
}

#[cfg(test)]
mod tests;
