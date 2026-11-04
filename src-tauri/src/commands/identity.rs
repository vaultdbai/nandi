//! Local identity commands (Tauri adapter for `services::auth`): the
//! display name and slug in `NANDI.md`, and the optional GitHub sign-in.

use tauri::State;

pub use crate::services::auth::github::GithubSignIn;
pub use crate::services::auth::IdentityStatus;

use crate::services::auth;
use crate::services::connectors::DeviceFlowState;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_identity_status(state: State<'_, AppState>) -> Result<IdentityStatus, String> {
    auth::get_identity_status(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_display_name(state: State<'_, AppState>, name: String) -> Result<(), String> {
    auth::set_display_name(&state, name).await
}

/// `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$`; names the branch pushes go to
#[tauri::command]
#[specta::specta]
pub async fn set_user_slug(state: State<'_, AppState>, slug: String) -> Result<(), String> {
    auth::set_user_slug(&state, slug).await
}

/// Where first-run setup is, so a relaunch resumes there
#[tauri::command]
#[specta::specta]
pub async fn set_onboarding_step(
    state: State<'_, AppState>,
    step: auth::OnboardingStep,
) -> Result<(), String> {
    auth::set_onboarding_step(&state, step).await
}

/// Whether this Nandi is signed in to GitHub, and as whom
#[tauri::command]
#[specta::specta]
pub async fn get_github_sign_in(state: State<'_, AppState>) -> Result<GithubSignIn, String> {
    Ok(auth::github::status(&state))
}

/// Starts the GitHub sign-in: show `userCode` and open `verificationUri`
#[tauri::command]
#[specta::specta]
pub async fn start_github_sign_in(state: State<'_, AppState>) -> Result<DeviceFlowState, String> {
    auth::github::start(&state).await
}

/// Asks GitHub once; `signedIn` once the user approved
#[tauri::command]
#[specta::specta]
pub async fn poll_github_sign_in(state: State<'_, AppState>) -> Result<GithubSignIn, String> {
    auth::github::poll(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn sign_out_github(state: State<'_, AppState>) -> Result<GithubSignIn, String> {
    auth::github::sign_out(&state)
}
