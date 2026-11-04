//! Studio artifact commands (Tauri adapter for `services::studio`).

use tauri::State;

pub use crate::services::ask::{ArtifactRef, EditPreview};
pub use crate::services::studio::{
    ArtifactLoad, ArtifactSave, ArtifactSaveResult, PushTargetOption,
};

use crate::services::studio;
use crate::services::AppState;

/// What's open in Studio's editor (`None`: nothing), for Studio's chat
#[tauri::command]
#[specta::specta]
pub async fn set_studio_open_artifact(
    state: State<'_, AppState>,
    artifact: Option<ArtifactRef>,
) -> Result<(), String> {
    studio::set_open_artifact(&state, artifact)
}

#[tauri::command]
#[specta::specta]
pub async fn load_artifact(
    state: State<'_, AppState>,
    artifact: ArtifactRef,
) -> Result<ArtifactLoad, String> {
    studio::load_artifact(&state, artifact).await
}

/// Writes through the artifact's repository, commits, and pushes per
/// `push_target`
#[tauri::command]
#[specta::specta]
pub async fn save_artifact(
    state: State<'_, AppState>,
    save: ArtifactSave,
) -> Result<ArtifactSaveResult, String> {
    studio::save_artifact(&state, save).await
}

/// The artifact as `instruction` would change it; nothing is written
#[tauri::command]
#[specta::specta]
pub async fn preview_edit(
    state: State<'_, AppState>,
    artifact: ArtifactRef,
    instruction: String,
) -> Result<EditPreview, String> {
    studio::preview_edit(&state, artifact, instruction).await
}

/// Whether `content` would save as `artifact`, with warnings (unknown
/// tools, a model without a key, missing agents)
#[tauri::command]
#[specta::specta]
pub async fn validate_artifact(
    state: State<'_, AppState>,
    artifact: ArtifactRef,
    content: String,
) -> Result<studio::validate::ArtifactValidation, String> {
    studio::validate_artifact(&state, artifact, content).await
}

#[tauri::command]
#[specta::specta]
pub async fn list_push_targets(
    state: State<'_, AppState>,
) -> Result<Vec<PushTargetOption>, String> {
    studio::list_push_targets(&state).await
}
