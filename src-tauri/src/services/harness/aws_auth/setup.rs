//! Setting up AWS profiles from Settings › Cloud credentials, always through
//! the AWS CLI, so Nandi still writes no AWS file itself:
//!
//! - **Access keys** (a new profile, or new keys for one): `aws configure
//!   --profile <name>`, with the answers on stdin, so the secret is never on
//!   a command line or in a log
//! - **SSO** (IAM Identity Center): the device sign-in (SSO OIDC), only to
//!   list the accounts and roles the user may pick from; the token stays in
//!   memory and is dropped once the profile is written. The profile is
//!   written with `aws configure set` (`sso_start_url`, `sso_region`,
//!   `sso_account_id`, `sso_role_name`, `region`), and then `aws sso login`
//!   signs the CLI in (`start_sso_login`), since that's where the SDK reads
//!   the sign-in from
//!
//! `aws configure set` can't write `sso-session` sections, so new SSO
//! profiles use the profile's own `sso_*` keys, which the CLI and the SDK
//! both read.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{AwsAuthService, AwsProfile};

/// How long `aws configure` may take
const CLI_TIMEOUT: Duration = Duration::from_secs(30);
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// Accounts listed at most (each costs a roles call)
const MAX_ACCOUNTS: usize = 100;

/// A new access-key profile, or new keys for an existing one. No `Debug`:
/// it holds the secret.
#[derive(Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyProfileInput {
    pub profile: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// `None`: keep the profile's region (none for a new profile)
    #[serde(default)]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SsoSetupInput {
    /// e.g. `https://my-company.awsapps.com/start`
    pub start_url: String,
    /// Where IAM Identity Center runs, e.g. `us-east-1`
    pub sso_region: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SsoSetupFinish {
    pub setup_id: String,
    pub profile: String,
    pub account_id: String,
    pub role_name: String,
    /// The profile's region for AWS calls; `None`: the SSO region
    #[serde(default)]
    pub region: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SsoAccount {
    pub account_id: String,
    pub account_name: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SsoSetupStatus {
    /// Waiting for the user to approve the sign-in in the browser
    Pending,
    /// Signed in: pick an account and role
    Ready,
    Failed,
    Expired,
}

/// An SSO profile being set up. Never carries the token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SsoSetup {
    pub setup_id: String,
    pub status: SsoSetupStatus,
    pub start_url: String,
    pub sso_region: String,
    /// Where the user approves the sign-in (with the code filled in)
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub accounts: Vec<SsoAccount>,
    pub error: Option<String>,
}

/// A device sign-in in progress. No `Debug`: it holds the client secret.
#[derive(Clone)]
pub(super) struct DeviceAuth {
    client_id: String,
    client_secret: String,
    device_code: String,
    pub(super) verification_uri: String,
    pub(super) user_code: String,
    interval: Duration,
    expires_in: Duration,
}

pub(super) enum TokenPoll {
    Pending,
    SlowDown,
    /// The access token (in memory only)
    Token(String),
    Expired,
    Denied,
}

/// The SSO calls a setup makes; real AWS in production, a fake in tests.
pub(super) trait SsoDirectory: Send + Sync {
    fn start(&self, input: SsoSetupInput) -> BoxFuture<'static, Result<DeviceAuth, String>>;
    fn poll(
        &self,
        region: String,
        auth: DeviceAuth,
    ) -> BoxFuture<'static, Result<TokenPoll, String>>;
    fn accounts(
        &self,
        region: String,
        token: String,
    ) -> BoxFuture<'static, Result<Vec<SsoAccount>, String>>;
}

/// What one poll of a setup came to
enum Outcome {
    Ready(Vec<SsoAccount>),
    /// Poll again after this long
    Wait(Duration),
    Ended(SsoSetupStatus, String),
}

pub(super) struct Setup {
    public: SsoSetup,
    auth: Option<DeviceAuth>,
    next_poll: Instant,
    expires_at: Instant,
}

/// Profile names the AWS CLI and Nandi's own fields accept
pub fn valid_profile_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if ok {
        Ok(name)
    } else {
        Err("Name the profile with letters, digits, '-', '_' or '.' (no spaces)".to_string())
    }
}

pub(crate) fn valid_region(region: &str) -> Result<&str, String> {
    let region = region.trim();
    let ok = (4..=32).contains(&region.len())
        && region
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && region.contains('-');
    if ok {
        Ok(region)
    } else {
        Err(format!("'{region}' is not an AWS region (like us-east-1)"))
    }
}

/// What `aws configure` reads on stdin: key ID, secret, region, output.
/// A blank answer keeps the profile's current value.
pub(crate) fn configure_answers(input: &KeyProfileInput) -> Result<String, String> {
    let key = input.access_key_id.trim();
    if !(16..=128).contains(&key.len()) || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("The access key ID is letters and digits, like AKIA…".to_string());
    }
    let secret = input.secret_access_key.trim();
    if secret.is_empty() || secret.contains(['\n', '\r']) {
        return Err("Paste the secret access key".to_string());
    }
    let region = match input.region.as_deref().map(str::trim) {
        Some(region) if !region.is_empty() => valid_region(region)?,
        _ => "",
    };
    Ok(format!("{key}\n{secret}\n{region}\n\n"))
}

/// `aws configure set <key> <value> --profile <profile>` for a new SSO profile
fn sso_settings(
    setup: &SsoSetup,
    finish: &SsoSetupFinish,
) -> Result<Vec<(String, String)>, String> {
    let account = setup
        .accounts
        .iter()
        .find(|a| a.account_id == finish.account_id)
        .ok_or("Pick one of the accounts this sign-in can use")?;
    if !account.roles.iter().any(|r| r == &finish.role_name) {
        return Err(format!(
            "'{}' isn't a role you have in {}",
            finish.role_name, account.account_name
        ));
    }
    let region = match finish.region.as_deref().map(str::trim) {
        Some(region) if !region.is_empty() => valid_region(region)?.to_string(),
        _ => setup.sso_region.clone(),
    };
    Ok([
        ("sso_start_url", setup.start_url.clone()),
        ("sso_region", setup.sso_region.clone()),
        ("sso_account_id", account.account_id.clone()),
        ("sso_role_name", finish.role_name.clone()),
        ("region", region),
        ("output", "json".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect())
}

fn valid_start_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    match reqwest::Url::parse(url) {
        Ok(parsed) if parsed.scheme() == "https" && parsed.host_str().is_some() => {
            Ok(url.to_string())
        }
        _ => Err("The start URL is the https:// address of your AWS access portal".to_string()),
    }
}

impl AwsAuthService {
    /// Runs the AWS CLI with `args`, writing `stdin` to it; the error is the
    /// CLI's last line of output (never what was written to it)
    async fn run_cli(&self, args: &[&str], stdin: Option<&str>) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let mut command = self.cli_command()?;
        command
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not run the AWS CLI: {e}"))?;
        if let Some(mut pipe) = child.stdin.take() {
            if let Some(text) = stdin {
                pipe.write_all(text.as_bytes())
                    .await
                    .map_err(|e| format!("Could not talk to the AWS CLI: {e}"))?;
            }
        }
        let output = tokio::time::timeout(CLI_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| "The AWS CLI didn't finish".to_string())?
            .map_err(|e| format!("The AWS CLI failed: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&output.stderr);
        let last = text.lines().rev().find(|l| !l.trim().is_empty());
        Err(last.unwrap_or("The AWS CLI failed").trim().to_string())
    }

    /// Writes an access-key profile (new, or new keys for an existing one)
    /// with `aws configure`, and returns it as the SDK now reads it
    pub async fn save_key_profile(&self, input: KeyProfileInput) -> Result<AwsProfile, String> {
        let name = valid_profile_name(&input.profile)?.to_string();
        let answers = configure_answers(&input)?;
        self.run_cli(&["configure", "--profile", &name], Some(&answers))
            .await?;
        log::info!("AWS profile '{name}' keys saved through the AWS CLI");
        self.profile(&name).await
    }

    /// Starts the SSO device sign-in for `input`'s access portal
    pub async fn start_sso_setup(&self, input: SsoSetupInput) -> Result<SsoSetup, String> {
        let input = SsoSetupInput {
            start_url: valid_start_url(&input.start_url)?,
            sso_region: valid_region(&input.sso_region)?.to_string(),
        };
        let auth = self.sso.start(input.clone()).await?;
        let public = SsoSetup {
            setup_id: uuid::Uuid::new_v4().to_string(),
            status: SsoSetupStatus::Pending,
            start_url: input.start_url,
            sso_region: input.sso_region,
            verification_uri: Some(auth.verification_uri.clone()),
            user_code: Some(auth.user_code.clone()),
            accounts: Vec::new(),
            error: None,
        };
        let now = Instant::now();
        let setup = Setup {
            public: public.clone(),
            next_poll: now + auth.interval,
            expires_at: now + auth.expires_in,
            auth: Some(auth),
        };
        self.setups
            .lock()
            .map_err(|e| e.to_string())?
            .insert(public.setup_id.clone(), setup);
        Ok(public)
    }

    /// Checks whether the sign-in was approved; once it is, lists the
    /// accounts and roles (and forgets the token)
    pub async fn poll_sso_setup(&self, setup_id: &str) -> Result<SsoSetup, String> {
        let (auth, region) = {
            let mut setups = self.setups.lock().map_err(|e| e.to_string())?;
            let setup = setups
                .get_mut(setup_id)
                .ok_or_else(|| "This sign-in is no longer open; start again".to_string())?;
            if setup.public.status != SsoSetupStatus::Pending {
                return Ok(setup.public.clone());
            }
            if Instant::now() >= setup.expires_at {
                setup.public.status = SsoSetupStatus::Expired;
                setup.public.error = Some("The sign-in wasn't approved in time".to_string());
                setup.auth = None;
                return Ok(setup.public.clone());
            }
            match setup.auth.clone() {
                Some(auth) if Instant::now() >= setup.next_poll => {
                    (auth, setup.public.sso_region.clone())
                }
                _ => return Ok(setup.public.clone()),
            }
        };
        let interval = auth.interval;
        let outcome = match self.sso.poll(region.clone(), auth).await {
            Ok(TokenPoll::Token(token)) => match self.sso.accounts(region, token).await {
                Ok(accounts) => Outcome::Ready(accounts),
                Err(e) => Outcome::Ended(SsoSetupStatus::Failed, e),
            },
            Ok(TokenPoll::Pending) => Outcome::Wait(interval),
            // RFC 8628: poll 5s less often from now on
            Ok(TokenPoll::SlowDown) => Outcome::Wait(interval + Duration::from_secs(5)),
            Ok(TokenPoll::Expired) => Outcome::Ended(
                SsoSetupStatus::Expired,
                "The sign-in wasn't approved in time".to_string(),
            ),
            Ok(TokenPoll::Denied) => {
                Outcome::Ended(SsoSetupStatus::Failed, "The sign-in was denied".to_string())
            }
            Err(e) => Outcome::Ended(SsoSetupStatus::Failed, e),
        };
        let mut setups = self.setups.lock().map_err(|e| e.to_string())?;
        let setup = setups
            .get_mut(setup_id)
            .ok_or_else(|| "This sign-in is no longer open; start again".to_string())?;
        match outcome {
            Outcome::Ready(accounts) => {
                setup.public.status = SsoSetupStatus::Ready;
                setup.public.accounts = accounts;
                setup.public.verification_uri = None;
                setup.public.user_code = None;
                setup.auth = None;
            }
            Outcome::Wait(interval) => {
                if let Some(auth) = setup.auth.as_mut() {
                    auth.interval = interval;
                }
                setup.next_poll = Instant::now() + interval;
            }
            Outcome::Ended(status, error) => {
                setup.public.status = status;
                setup.public.error = Some(error);
                setup.auth = None;
            }
        }
        Ok(setup.public.clone())
    }

    /// Writes the SSO profile for the account and role the user picked.
    /// The caller then runs `aws sso login` for it (`start_sso_login`).
    pub async fn finish_sso_setup(&self, finish: SsoSetupFinish) -> Result<AwsProfile, String> {
        let name = valid_profile_name(&finish.profile)?.to_string();
        let settings = {
            let setups = self.setups.lock().map_err(|e| e.to_string())?;
            let setup = setups
                .get(&finish.setup_id)
                .filter(|s| s.public.status == SsoSetupStatus::Ready)
                .ok_or_else(|| "Sign in first, then pick the account and role".to_string())?;
            sso_settings(&setup.public, &finish)?
        };
        for (key, value) in &settings {
            self.run_cli(&["configure", "set", key, value, "--profile", &name], None)
                .await?;
        }
        if let Ok(mut setups) = self.setups.lock() {
            setups.remove(&finish.setup_id);
        }
        log::info!("AWS SSO profile '{name}' written through the AWS CLI");
        self.profile(&name).await
    }

    /// Drops an SSO setup the user closed
    pub fn cancel_sso_setup(&self, setup_id: &str) {
        if let Ok(mut setups) = self.setups.lock() {
            setups.remove(setup_id);
        }
    }
}

/// The real IAM Identity Center: SSO OIDC for the device sign-in, the SSO
/// portal API for accounts and roles. Neither call needs AWS credentials.
pub(super) struct AwsSso;

impl SsoDirectory for AwsSso {
    fn start(&self, input: SsoSetupInput) -> BoxFuture<'static, Result<DeviceAuth, String>> {
        Box::pin(async move {
            let client = oidc_client(&input.sso_region);
            let registered = client
                .register_client()
                .client_name("Nandi")
                .client_type("public")
                .scopes("sso:account:access")
                .send()
                .await
                .map_err(|e| sso_error("Could not reach IAM Identity Center", &e))?;
            let (Some(client_id), Some(client_secret)) =
                (registered.client_id, registered.client_secret)
            else {
                return Err("IAM Identity Center didn't register Nandi".to_string());
            };
            let started = client
                .start_device_authorization()
                .client_id(&client_id)
                .client_secret(&client_secret)
                .start_url(&input.start_url)
                .send()
                .await
                .map_err(|e| {
                    use aws_sdk_ssooidc::operation::start_device_authorization::StartDeviceAuthorizationError;
                    match e.as_service_error() {
                        // What a start URL that isn't a portal in this region gets
                        Some(StartDeviceAuthorizationError::InvalidRequestException(_)) => format!(
                            "IAM Identity Center in {} doesn't know {}. Check the start URL and the SSO region with your AWS admin",
                            input.sso_region, input.start_url
                        ),
                        _ => sso_error("Could not start the sign-in", &e),
                    }
                })?;
            let (Some(device_code), Some(user_code)) = (started.device_code, started.user_code)
            else {
                return Err("IAM Identity Center didn't start the sign-in".to_string());
            };
            Ok(DeviceAuth {
                client_id,
                client_secret,
                device_code,
                verification_uri: started
                    .verification_uri_complete
                    .or(started.verification_uri)
                    .unwrap_or_default(),
                user_code,
                interval: Duration::from_secs(u64::try_from(started.interval.max(1)).unwrap_or(5)),
                expires_in: Duration::from_secs(
                    u64::try_from(started.expires_in.max(60)).unwrap_or(600),
                ),
            })
        })
    }

    fn poll(
        &self,
        region: String,
        auth: DeviceAuth,
    ) -> BoxFuture<'static, Result<TokenPoll, String>> {
        Box::pin(async move {
            use aws_sdk_ssooidc::operation::create_token::CreateTokenError;
            let result = oidc_client(&region)
                .create_token()
                .client_id(auth.client_id)
                .client_secret(auth.client_secret)
                .grant_type(DEVICE_GRANT)
                .device_code(auth.device_code)
                .send()
                .await;
            match result {
                Ok(token) => token
                    .access_token
                    .map(TokenPoll::Token)
                    .ok_or_else(|| "IAM Identity Center returned no token".to_string()),
                Err(e) => match e.as_service_error() {
                    Some(CreateTokenError::AuthorizationPendingException(_)) => {
                        Ok(TokenPoll::Pending)
                    }
                    Some(CreateTokenError::SlowDownException(_)) => Ok(TokenPoll::SlowDown),
                    Some(CreateTokenError::ExpiredTokenException(_)) => Ok(TokenPoll::Expired),
                    Some(CreateTokenError::AccessDeniedException(_)) => Ok(TokenPoll::Denied),
                    _ => Err(sso_error("The sign-in failed", &e)),
                },
            }
        })
    }

    fn accounts(
        &self,
        region: String,
        token: String,
    ) -> BoxFuture<'static, Result<Vec<SsoAccount>, String>> {
        Box::pin(async move {
            let config = aws_sdk_sso::Config::builder()
                .behavior_version(aws_sdk_sso::config::BehaviorVersion::latest())
                .region(aws_sdk_sso::config::Region::new(region))
                .build();
            let client = aws_sdk_sso::Client::from_conf(config);
            let accounts = client
                .list_accounts()
                .access_token(&token)
                .into_paginator()
                .items()
                .send()
                .collect::<Result<Vec<_>, _>>()
                .await
                .map_err(|e| sso_error("Could not list your AWS accounts", &e))?;
            let mut listed = Vec::new();
            for account in accounts.into_iter().take(MAX_ACCOUNTS) {
                let Some(account_id) = account.account_id else {
                    continue;
                };
                let mut roles: Vec<String> = client
                    .list_account_roles()
                    .access_token(&token)
                    .account_id(&account_id)
                    .into_paginator()
                    .items()
                    .send()
                    .collect::<Result<Vec<_>, _>>()
                    .await
                    .map_err(|e| sso_error("Could not list your roles", &e))?
                    .into_iter()
                    .filter_map(|role| role.role_name)
                    .collect();
                roles.sort();
                listed.push(SsoAccount {
                    account_name: account.account_name.unwrap_or_else(|| account_id.clone()),
                    account_id,
                    email: account.email_address,
                    roles,
                });
            }
            listed.sort_by(|a, b| a.account_name.cmp(&b.account_name));
            Ok(listed)
        })
    }
}

fn oidc_client(region: &str) -> aws_sdk_ssooidc::Client {
    let config = aws_sdk_ssooidc::Config::builder()
        .behavior_version(aws_sdk_ssooidc::config::BehaviorVersion::latest())
        .region(aws_sdk_ssooidc::config::Region::new(region.to_string()))
        .build();
    aws_sdk_ssooidc::Client::from_conf(config)
}

fn sso_error(what: &str, error: &(dyn std::error::Error + 'static)) -> String {
    format!("{what}: {}", super::aws_smithy_error(error))
}

impl fmt::Debug for KeyProfileInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyProfileInput")
            .field("profile", &self.profile)
            .field(
                "access_key_id",
                &crate::services::egress::credentials::redact(&self.access_key_id),
            )
            .field("secret_access_key", &"<redacted>")
            .field("region", &self.region)
            .finish()
    }
}

/// The SSO directory in production
pub(super) fn aws_sso() -> Arc<dyn SsoDirectory> {
    Arc::new(AwsSso)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture, Fixture};
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    fn key_input(key: &str, secret: &str, region: Option<&str>) -> KeyProfileInput {
        KeyProfileInput {
            profile: "work".into(),
            access_key_id: key.into(),
            secret_access_key: secret.into(),
            region: region.map(str::to_string),
        }
    }

    #[test]
    fn checks_names_regions_and_start_urls() {
        assert_eq!(valid_profile_name(" dev-1.eu_x ").unwrap(), "dev-1.eu_x");
        let long = "a".repeat(65);
        for bad in ["", "my work", "a/b", "[x]", long.as_str()] {
            assert!(valid_profile_name(bad).is_err(), "{bad}");
        }
        assert_eq!(valid_region(" eu-west-1 ").unwrap(), "eu-west-1");
        for bad in ["", "EU-WEST-1", "useast", "us east-1"] {
            assert!(valid_region(bad).is_err(), "{bad}");
        }
        assert_eq!(
            valid_start_url(" https://acme.awsapps.com/start/ ").unwrap(),
            "https://acme.awsapps.com/start"
        );
        for bad in ["http://acme.awsapps.com/start", "acme", "https://"] {
            assert!(valid_start_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn configure_reads_the_keys_on_stdin() {
        let answers = configure_answers(&key_input(
            " AKIAEXAMPLE12345 ",
            " s3cr3t ",
            Some("eu-west-1"),
        ))
        .unwrap();
        assert_eq!(answers, "AKIAEXAMPLE12345\ns3cr3t\neu-west-1\n\n");
        // No region: a blank answer keeps the profile's
        let answers = configure_answers(&key_input("AKIAEXAMPLE12345", "s", None)).unwrap();
        assert_eq!(answers, "AKIAEXAMPLE12345\ns\n\n\n");
        for (key, secret, region) in [
            ("AKIA", "s", None),
            ("AKIA EXAMPLE 1234", "s", None),
            ("AKIAEXAMPLE12345", " ", None),
            ("AKIAEXAMPLE12345", "two\nlines", None),
            ("AKIAEXAMPLE12345", "s", Some("Mars")),
        ] {
            assert!(configure_answers(&key_input(key, secret, region)).is_err());
        }
        let shown = format!("{:?}", key_input("AKIAEXAMPLE12345", "s3cr3t", None));
        assert!(!shown.contains("s3cr3t") && !shown.contains("AKIAEXAMPLE12345"));
    }

    fn ready_setup() -> SsoSetup {
        SsoSetup {
            setup_id: "s1".into(),
            status: SsoSetupStatus::Ready,
            start_url: "https://acme.awsapps.com/start".into(),
            sso_region: "us-east-1".into(),
            verification_uri: None,
            user_code: None,
            accounts: vec![SsoAccount {
                account_id: "111122223333".into(),
                account_name: "Prod".into(),
                email: None,
                roles: vec!["ReadOnly".into()],
            }],
            error: None,
        }
    }

    fn finish(setup_id: &str, account: &str, role: &str, region: Option<&str>) -> SsoSetupFinish {
        SsoSetupFinish {
            setup_id: setup_id.into(),
            profile: "prod".into(),
            account_id: account.into(),
            role_name: role.into(),
            region: region.map(str::to_string),
        }
    }

    #[test]
    fn an_sso_profile_gets_only_a_listed_account_and_role() {
        let settings = sso_settings(
            &ready_setup(),
            &finish("s1", "111122223333", "ReadOnly", None),
        )
        .unwrap();
        let get = |key: &str| {
            settings
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("sso_start_url"), Some("https://acme.awsapps.com/start"));
        assert_eq!(get("sso_account_id"), Some("111122223333"));
        assert_eq!(get("sso_role_name"), Some("ReadOnly"));
        // The SSO region when none is picked
        assert_eq!(get("region"), Some("us-east-1"));
        let picked = finish("s1", "111122223333", "ReadOnly", Some("eu-west-2"));
        let with_region = sso_settings(&ready_setup(), &picked).unwrap();
        assert!(with_region.contains(&("region".into(), "eu-west-2".into())));

        let other_account = sso_settings(
            &ready_setup(),
            &finish("s1", "999988887777", "ReadOnly", None),
        );
        assert!(other_account
            .unwrap_err()
            .contains("Pick one of the accounts"));
        let other_role = sso_settings(&ready_setup(), &finish("s1", "111122223333", "Admin", None));
        assert!(other_role
            .unwrap_err()
            .contains("isn't a role you have in Prod"));
    }

    /// Answers each token poll from `polls` in turn
    struct FakeSso {
        polls: Mutex<Vec<TokenPoll>>,
        calls: AtomicU32,
    }

    impl SsoDirectory for FakeSso {
        fn start(&self, input: SsoSetupInput) -> BoxFuture<'static, Result<DeviceAuth, String>> {
            Box::pin(async move {
                assert_eq!(input.start_url, "https://acme.awsapps.com/start");
                Ok(DeviceAuth {
                    client_id: "client".into(),
                    client_secret: "client-secret".into(),
                    device_code: "device".into(),
                    verification_uri:
                        "https://device.sso.us-east-1.amazonaws.com/?user_code=ABCD-EFGH".into(),
                    user_code: "ABCD-EFGH".into(),
                    interval: Duration::ZERO,
                    expires_in: Duration::from_secs(600),
                })
            })
        }

        fn poll(
            &self,
            _region: String,
            _auth: DeviceAuth,
        ) -> BoxFuture<'static, Result<TokenPoll, String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self.polls.lock().unwrap().remove(0);
            Box::pin(async move { Ok(next) })
        }

        fn accounts(
            &self,
            region: String,
            token: String,
        ) -> BoxFuture<'static, Result<Vec<SsoAccount>, String>> {
            Box::pin(async move {
                assert_eq!(
                    (region.as_str(), token.as_str()),
                    ("us-east-1", "sso-access-token")
                );
                Ok(ready_setup().accounts)
            })
        }
    }

    fn with_fake_sso(polls: Vec<TokenPoll>) -> (Fixture, Arc<FakeSso>) {
        let sso = Arc::new(FakeSso {
            polls: Mutex::new(polls),
            calls: AtomicU32::new(0),
        });
        let f = fixture(&[], None);
        let f = Fixture {
            service: f.service.with_sso(sso.clone()),
            ..f
        };
        (f, sso)
    }

    fn start_input() -> SsoSetupInput {
        SsoSetupInput {
            start_url: "https://acme.awsapps.com/start/".into(),
            sso_region: "us-east-1".into(),
        }
    }

    #[tokio::test]
    async fn the_sign_in_waits_for_approval_then_lists_accounts() {
        let (f, sso) = with_fake_sso(vec![
            TokenPoll::Pending,
            TokenPoll::SlowDown,
            TokenPoll::Token("sso-access-token".into()),
        ]);
        let started = f.service.start_sso_setup(start_input()).await.unwrap();
        assert_eq!(started.status, SsoSetupStatus::Pending);
        assert_eq!(started.user_code.as_deref(), Some("ABCD-EFGH"));
        assert_eq!(started.start_url, "https://acme.awsapps.com/start");
        let id = started.setup_id;
        let poll = || f.service.poll_sso_setup(&id);

        assert_eq!(poll().await.unwrap().status, SsoSetupStatus::Pending);
        assert_eq!(poll().await.unwrap().status, SsoSetupStatus::Pending);
        // Slow down: the next poll waits 5s, so this one doesn't call AWS
        assert_eq!(poll().await.unwrap().status, SsoSetupStatus::Pending);
        assert_eq!(sso.calls.load(Ordering::SeqCst), 2);

        // Past the wait: approved
        f.service
            .setups
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .next_poll = Instant::now();
        let ready = poll().await.unwrap();
        assert_eq!(ready.status, SsoSetupStatus::Ready, "{ready:?}");
        assert_eq!(ready.accounts[0].roles, ["ReadOnly"]);
        assert_eq!(ready.user_code, None);
        let shown = serde_json::to_string(&ready).unwrap();
        assert!(!shown.contains("sso-access-token") && !shown.contains("client-secret"));
        // Ready stays ready without asking AWS again
        poll().await.unwrap();
        assert_eq!(sso.calls.load(Ordering::SeqCst), 3);

        // Only a listed account and role can be written
        let wrong = finish(&id, "111122223333", "Admin", None);
        assert!(f.service.finish_sso_setup(wrong).await.is_err());
        f.service.cancel_sso_setup(&id);
        assert!(poll().await.is_err());
    }

    #[tokio::test]
    async fn a_denied_sign_in_fails_and_finishing_needs_one() {
        let (f, _) = with_fake_sso(vec![TokenPoll::Denied]);
        let id = f
            .service
            .start_sso_setup(start_input())
            .await
            .unwrap()
            .setup_id;
        let failed = f.service.poll_sso_setup(&id).await.unwrap();
        assert_eq!(failed.status, SsoSetupStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("The sign-in was denied"));
        let early = finish(&id, "111122223333", "ReadOnly", None);
        let error = f.service.finish_sso_setup(early).await.unwrap_err();
        assert!(error.contains("Sign in first"), "{error}");

        let bad = SsoSetupInput {
            start_url: "acme".into(),
            sso_region: "us-east-1".into(),
        };
        assert!(f.service.start_sso_setup(bad).await.is_err());
    }

    /// Against the real AWS CLI, on throwaway files: `cargo test -- --ignored`
    #[tokio::test]
    #[ignore = "needs the AWS CLI on PATH"]
    async fn the_aws_cli_writes_both_kinds_of_profile() {
        let env = super::super::detection::DetectEnv::current();
        let cli = super::super::detection::find_executable(&["aws"], &env).expect("the AWS CLI");
        let (f, _) = with_fake_sso(vec![TokenPoll::Token("sso-access-token".into())]);
        let dir = f._dir.path().to_path_buf();
        let f = Fixture {
            service: f.service.with_test_files(
                Some(cli),
                dir.join("config"),
                dir.join("credentials"),
            ),
            ..f
        };

        let saved = f
            .service
            .save_key_profile(key_input("AKIAEXAMPLE12345", "s3cr3t", Some("eu-west-1")))
            .await
            .unwrap();
        assert_eq!(saved.kind, super::super::AwsProfileKind::StaticKeys);
        assert_eq!(saved.region.as_deref(), Some("eu-west-1"));
        let credentials = std::fs::read_to_string(dir.join("credentials")).unwrap();
        assert!(credentials.contains("[work]") && credentials.contains("s3cr3t"));

        let id = f
            .service
            .start_sso_setup(start_input())
            .await
            .unwrap()
            .setup_id;
        f.service.poll_sso_setup(&id).await.unwrap();
        let pick = finish(&id, "111122223333", "ReadOnly", Some("eu-west-2"));
        let written = f.service.finish_sso_setup(pick).await.unwrap();
        assert_eq!(written.kind, super::super::AwsProfileKind::Sso);
        assert_eq!(written.sso_account_id.as_deref(), Some("111122223333"));
        assert_eq!(written.sso_role_name.as_deref(), Some("ReadOnly"));
        assert_eq!(written.region.as_deref(), Some("eu-west-2"));
    }
}
