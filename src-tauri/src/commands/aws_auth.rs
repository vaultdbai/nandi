//! AWS auth commands (Tauri adapter for `services::harness::aws_auth`).
//! No command returns key material: credentials stay in the user's AWS
//! config. Only `save_aws_key_profile` takes a secret, which goes straight
//! to the AWS CLI's stdin.

use tauri::State;

pub use crate::services::harness::aws_auth::{
    AwsCliInfo, AwsProfile, BedrockApiKeyStatus, CredentialStatus, KeyProfileInput, SsoLoginResult,
    SsoSetup, SsoSetupFinish, SsoSetupInput,
};

use crate::services::harness::aws_auth;
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn detect_aws_cli(state: State<'_, AppState>) -> Result<AwsCliInfo, String> {
    aws_auth::detect_aws_cli(&state).await
}

/// Profiles from `~/.aws/config` and `~/.aws/credentials`
#[tauri::command]
#[specta::specta]
pub async fn list_aws_profiles(state: State<'_, AppState>) -> Result<Vec<AwsProfile>, String> {
    aws_auth::list_aws_profiles(&state).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_aws_profile(
    state: State<'_, AppState>,
    name: String,
) -> Result<AwsProfile, String> {
    aws_auth::get_aws_profile(&state, &name).await
}

/// Runs `aws sso login`, or `aws login` for a console sign-in profile (it
/// opens the browser); poll for the outcome
#[tauri::command]
#[specta::specta]
pub async fn start_aws_sso_login(
    state: State<'_, AppState>,
    profile: String,
) -> Result<SsoLoginResult, String> {
    aws_auth::start_aws_sso_login(&state, &profile).await
}

#[tauri::command]
#[specta::specta]
pub async fn poll_aws_sso_login(
    state: State<'_, AppState>,
    login_id: String,
) -> Result<SsoLoginResult, String> {
    aws_auth::poll_aws_sso_login(&state, &login_id).await
}

/// A new access-key profile, or new keys for one, through `aws configure`
#[tauri::command]
#[specta::specta]
pub async fn save_aws_key_profile(
    state: State<'_, AppState>,
    input: KeyProfileInput,
) -> Result<AwsProfile, String> {
    aws_auth::save_aws_key_profile(&state, input).await
}

/// Starts the IAM Identity Center sign-in for a new SSO profile
#[tauri::command]
#[specta::specta]
pub async fn start_aws_sso_setup(
    state: State<'_, AppState>,
    input: SsoSetupInput,
) -> Result<SsoSetup, String> {
    aws_auth::start_aws_sso_setup(&state, input).await
}

/// Pending until the sign-in is approved; then the accounts and roles
#[tauri::command]
#[specta::specta]
pub async fn poll_aws_sso_setup(
    state: State<'_, AppState>,
    setup_id: String,
) -> Result<SsoSetup, String> {
    aws_auth::poll_aws_sso_setup(&state, &setup_id).await
}

/// Writes the SSO profile for the picked account and role
#[tauri::command]
#[specta::specta]
pub async fn finish_aws_sso_setup(
    state: State<'_, AppState>,
    finish: SsoSetupFinish,
) -> Result<AwsProfile, String> {
    aws_auth::finish_aws_sso_setup(&state, finish).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_aws_sso_setup(
    state: State<'_, AppState>,
    setup_id: String,
) -> Result<(), String> {
    aws_auth::cancel_aws_sso_setup(&state, &setup_id).await
}

/// `None`s: `AWS_PROFILE` / the profile's region, as the AWS CLI resolves them
#[tauri::command]
#[specta::specta]
pub async fn check_aws_credentials(
    state: State<'_, AppState>,
    profile: Option<String>,
    region: Option<String>,
) -> Result<CredentialStatus, String> {
    aws_auth::check_aws_credentials(&state, profile, region).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_aws_bedrock_api_key_status(
    state: State<'_, AppState>,
) -> Result<BedrockApiKeyStatus, String> {
    aws_auth::get_aws_bedrock_api_key_status(&state).await
}
