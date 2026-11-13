//! AWS auth commands (`services::harness::aws_auth`). No command returns
//! key material: credentials stay in the user's AWS config. Only
//! `save_aws_key_profile` takes a secret, which goes straight to the AWS
//! CLI's stdin.

use crate::services::harness::aws_auth;
use crate::services::harness::aws_auth::{
    AwsCliInfo, AwsProfile, BedrockApiKeyStatus, CredentialStatus, KeyProfileInput, SsoLoginResult,
    SsoSetup, SsoSetupFinish, SsoSetupInput,
};

super::commands! {
    fn detect_aws_cli(state) -> AwsCliInfo => aws_auth::detect_aws_cli(&state).await;

    /// Profiles from `~/.aws/config` and `~/.aws/credentials`
    fn list_aws_profiles(state) -> Vec<AwsProfile> => aws_auth::list_aws_profiles(&state).await;

    fn get_aws_profile(state, name: String) -> AwsProfile => aws_auth::get_aws_profile(&state, &name).await;

    /// Runs `aws sso login`, or `aws login` for a console sign-in profile (it
    /// opens the browser); poll for the outcome
    fn start_aws_sso_login(state, profile: String) -> SsoLoginResult => aws_auth::start_aws_sso_login(&state, &profile).await;

    fn poll_aws_sso_login(state, login_id: String) -> SsoLoginResult => aws_auth::poll_aws_sso_login(&state, &login_id).await;

    /// A new access-key profile, or new keys for one, through `aws configure`
    fn save_aws_key_profile(state, input: KeyProfileInput) -> AwsProfile => aws_auth::save_aws_key_profile(&state, input).await;

    /// Starts the IAM Identity Center sign-in for a new SSO profile
    fn start_aws_sso_setup(state, input: SsoSetupInput) -> SsoSetup => aws_auth::start_aws_sso_setup(&state, input).await;

    /// Pending until the sign-in is approved; then the accounts and roles
    fn poll_aws_sso_setup(state, setup_id: String) -> SsoSetup => aws_auth::poll_aws_sso_setup(&state, &setup_id).await;

    /// Writes the SSO profile for the picked account and role
    fn finish_aws_sso_setup(state, finish: SsoSetupFinish) -> AwsProfile => aws_auth::finish_aws_sso_setup(&state, finish).await;

    fn cancel_aws_sso_setup(state, setup_id: String) -> () => aws_auth::cancel_aws_sso_setup(&state, &setup_id).await;

    /// `None`s: `AWS_PROFILE` / the profile's region, as the AWS CLI resolves them
    fn check_aws_credentials(state, profile: Option<String>, region: Option<String>) -> CredentialStatus => aws_auth::check_aws_credentials(&state, profile, region).await;

    fn get_aws_bedrock_api_key_status(state) -> BedrockApiKeyStatus => aws_auth::get_aws_bedrock_api_key_status(&state).await;
}
