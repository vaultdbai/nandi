//! Studio artifact commands (Tauri adapter for `services::studio`).

use crate::services::ask::{ArtifactRef, EditPreview};
use crate::services::studio;
use crate::services::studio::{ArtifactLoad, ArtifactSave, ArtifactSaveResult, PushTargetOption};

super::commands! {
    /// What's open in Studio's editor (`None`: nothing), for Studio's chat
    fn set_studio_open_artifact(state, artifact: Option<ArtifactRef>) -> () => studio::set_open_artifact(&state, artifact);

    fn load_artifact(state, artifact: ArtifactRef) -> ArtifactLoad => studio::load_artifact(&state, artifact).await;

    /// Writes through the artifact's repository, commits, and pushes per
    /// `push_target`
    fn save_artifact(state, save: ArtifactSave) -> ArtifactSaveResult => studio::save_artifact(&state, save).await;

    /// The artifact as `instruction` would change it; nothing is written
    fn preview_edit(state, artifact: ArtifactRef, instruction: String) -> EditPreview => studio::preview_edit(&state, artifact, instruction).await;

    /// Whether `content` would save as `artifact`, with warnings (unknown
    /// tools, a model without a key, missing agents)
    fn validate_artifact(state, artifact: ArtifactRef, content: String) -> studio::validate::ArtifactValidation => studio::validate_artifact(&state, artifact, content).await;

    fn list_push_targets(state) -> Vec<PushTargetOption> => studio::list_push_targets(&state).await;
}
