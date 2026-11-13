//! Local identity commands (Tauri adapter for `services::auth`): the
//! display name and slug in `NANDI.md`, and the optional GitHub sign-in.

use crate::services::auth;
use crate::services::auth::github::GithubSignIn;
use crate::services::auth::oauth::DeviceFlowState;
use crate::services::auth::IdentityStatus;
use crate::services::identity::VerifiedIdentity;

super::commands! {
    fn get_identity_status(state) -> IdentityStatus => auth::get_identity_status(&state).await;

    /// This Nandi's person, locked from the first mailbox connected (shown
    /// read-only); `None` before any
    fn get_verified_identity(state) -> Option<VerifiedIdentity> => Ok(crate::services::identity::identity(&state));

    fn set_display_name(state, name: String) -> () => auth::set_display_name(&state, name).await;

    /// `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$`; names the branch pushes go to
    fn set_user_slug(state, slug: String) -> () => auth::set_user_slug(&state, slug).await;

    /// Where first-run setup is, so a relaunch resumes there
    fn set_onboarding_step(state, step: auth::OnboardingStep) -> () => auth::set_onboarding_step(&state, step).await;

    /// Whether this Nandi is signed in to GitHub, and as whom
    fn get_github_sign_in(state) -> GithubSignIn => Ok(auth::github::status(&state));

    /// Starts the GitHub sign-in: show `userCode` and open `verificationUri`
    fn start_github_sign_in(state) -> DeviceFlowState => auth::github::start(&state).await;

    /// Asks GitHub once; `signedIn` once the user approved
    fn poll_github_sign_in(state) -> GithubSignIn => auth::github::poll(&state).await;

    fn sign_out_github(state) -> GithubSignIn => auth::github::sign_out(&state);
}
