//! Sandbox image commands (Tauri adapter for `services::images`).

use crate::services::images::tools::{self, ToolBundle, ToolsStatus};
use crate::services::images::{self, ImageStatus, SandboxImage};

super::commands! {
    /// Whether `image` is here and current, its download size, and how a
    /// download (or the advanced local build) is going
    fn get_image_status(image: SandboxImage) -> ImageStatus => Ok(images::image_status(image).await);

    /// The advanced option: builds `image` here in the background; output
    /// arrives as `images://pull`. Nothing requires it
    fn build_image(state, image: SandboxImage) -> () => images::start_build(&state, image);

    /// Downloads `image` (pinned to this app's sandbox version) in the
    /// background; progress arrives as `images://pull`
    fn pull_image(state, image: SandboxImage) -> () => images::start_download(&state, image);

    /// Whether a first-use tool bundle (Claude Code) is installed, and how its
    /// install is going
    fn get_tools_status(bundle: ToolBundle) -> ToolsStatus => Ok(tools::status(bundle).await);

    /// Gets a first-use tool bundle now (or again, after a failure) in the
    /// background; output arrives as `images://tools`
    fn install_tools(bundle: ToolBundle) -> () => tools::start_install(bundle);
}
