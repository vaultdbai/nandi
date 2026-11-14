//! AWS auth for Bedrock harness sessions and agents' AWS calls (see
//! `egress::credentials`), entirely through what the user already has. Nandi stores no AWS secret and parses no AWS file itself:
//! - profiles: the AWS SDK's own parser (`aws_config_parser`)
//! - credentials: the SDK's default chain for a profile (`aws-config`):
//!   SSO token cache, static keys, assume-role, credential_process, env.
//!   Resolved on the host to temporary keys, so containers never see
//!   `~/.aws`
//! - SSO login: `aws sso login`, console sign-in: `aws login` (the AWS CLI
//!   opens the browser)
//! - new profiles and replaced keys: through the AWS CLI too (`setup`)
//! - identity: STS `GetCallerIdentity`
//! - a Bedrock API key: `AWS_BEARER_TOKEN_BEDROCK` from the environment or
//!   Claude Code's settings `env`, passed through as is
//!
//! Credentials go into `HarnessCredentials` or `SignInKeys` (never
//! serialized, never logged) and reach only a harness's environment or
//! the egress layer. What the UI gets
//! (`CredentialStatus`, `AwsProfile`, …) holds no key material.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_credential_types::provider::ProvideCredentials;
use aws_runtime::env_config::file::{EnvConfigFileKind, EnvConfigFiles};
use aws_types::os_shim_internal::{Env, Fs};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{EnumString, IntoStaticStr};

mod setup;
pub(crate) use setup::{configure_answers, valid_region};
pub use setup::{KeyProfileInput, SsoSetup, SsoSetupFinish, SsoSetupInput};

use super::adapter::HarnessCredentials;
use super::detection::{self, DetectEnv};
use crate::db::{now_ms, ts, Db};
use crate::model::Timestamp;
use crate::services::egress::credentials::{SignInFn, SignInKeys};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;

/// Where Bedrock sessions go when nothing names a region
pub const DEFAULT_REGION: &str = "us-east-1";
/// An SSO login that hasn't finished by then is given up
const SSO_LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AwsCliInfo {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    /// SSO needs AWS CLI v2
    pub supports_sso: bool,
    /// `aws login` (console sign-in) exists in this CLI
    pub supports_login: bool,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum AwsProfileKind {
    /// `sso_session` (or legacy `sso_start_url`) with an account and role
    Sso,
    /// Keys in `~/.aws/credentials`
    StaticKeys,
    /// `role_arn` + `source_profile` / `credential_source`
    AssumeRole,
    CredentialProcess,
    /// `login_session`: console sign-in with `aws login`
    ConsoleLogin,
    /// Region or output only; credentials come from elsewhere
    Other,
}

impl AwsProfileKind {
    /// The kind's name
    fn as_str(self) -> &'static str {
        self.into()
    }

    fn source(self) -> CredentialSource {
        match self {
            Self::Sso => CredentialSource::Sso,
            Self::StaticKeys => CredentialSource::SharedCredentials,
            Self::AssumeRole => CredentialSource::AssumeRole,
            Self::CredentialProcess => CredentialSource::CredentialProcess,
            Self::ConsoleLogin => CredentialSource::ConsoleLogin,
            Self::Other => CredentialSource::Environment,
        }
    }
}

/// A profile from `~/.aws/config` + `~/.aws/credentials`. Never carries
/// key material: only whether it has some.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AwsProfile {
    pub name: String,
    pub kind: AwsProfileKind,
    pub region: Option<String>,
    pub sso_session: Option<String>,
    pub sso_start_url: Option<String>,
    pub sso_region: Option<String>,
    pub sso_account_id: Option<String>,
    pub sso_role_name: Option<String>,
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
    /// Static keys exist in `~/.aws/credentials` (values never read out)
    pub has_static_keys: bool,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SsoLoginStatus {
    Pending,
    Succeeded,
    Failed,
    Expired,
}

/// An `aws sso login` in progress (the CLI opens the browser)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SsoLoginResult {
    pub login_id: String,
    pub profile: String,
    pub status: SsoLoginStatus,
    /// Where the user approves the login, if the CLI printed it
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub started_at: Timestamp,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CredentialSource {
    /// A cached SSO token for the profile
    Sso,
    /// `~/.aws/credentials` keys
    SharedCredentials,
    AssumeRole,
    CredentialProcess,
    /// A cached `aws login` session for the profile
    ConsoleLogin,
    /// `AWS_ACCESS_KEY_ID` etc. in Nandi's environment
    Environment,
    /// `AWS_BEARER_TOKEN_BEDROCK` (environment or Claude Code settings)
    BedrockApiKey,
    None,
}

/// Whether usable credentials exist right now (never the credentials)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub source: CredentialSource,
    pub valid: bool,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub account_id: Option<String>,
    /// The caller identity ARN (`sts get-caller-identity`)
    pub arn: Option<String>,
    pub expires_at: Option<Timestamp>,
    /// The SSO token expired (or was never there): log in again
    pub expired: bool,
    pub error: Option<String>,
    pub checked_at: Timestamp,
}

/// Whether a Bedrock API key is available (never the key)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BedrockApiKeyStatus {
    pub configured: bool,
    /// `environment` or `claude-code-settings`
    pub source: Option<String>,
}

/// `aws://profile_changed`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsProfileChanged {
    pub profiles: Vec<String>,
}

impl NandiEvent for AwsProfileChanged {
    const NAME: &'static str = super::AWS_PROFILE_CHANGED;
}

/// `aws://credentials_expired`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsCredentialsExpired {
    pub profile: Option<String>,
    pub error: Option<String>,
}

impl NandiEvent for AwsCredentialsExpired {
    const NAME: &'static str = super::AWS_CREDENTIALS_EXPIRED;
}

/// Credentials resolved through the SDK's chain (host only)
struct Resolved {
    credentials: aws_credential_types::Credentials,
    region: String,
    sdk: SdkConfig,
}

struct Login {
    result: SsoLoginResult,
    child: Option<tokio::process::Child>,
    output: Arc<Mutex<String>>,
    started: Instant,
}

pub struct AwsAuthService {
    db: Db,
    bus: EventBus,
    env: DetectEnv,
    /// `None`: `aws` on PATH
    aws_cli: Option<PathBuf>,
    /// Tests: these AWS config and credentials files instead of the user's
    profile_files: Option<(PathBuf, PathBuf)>,
    logins: Mutex<HashMap<String, Login>>,
    /// SSO profiles being set up, by setup ID (`setup`)
    setups: Mutex<HashMap<String, setup::Setup>>,
    /// IAM Identity Center's sign-in and accounts (a fake in tests)
    sso: Arc<dyn setup::SsoDirectory>,
}

fn not_empty(value: Option<&String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty()).cloned()
}

/// The first `https://` URL and an `ABCD-EFGH` code in the CLI's output
pub fn parse_sso_output(output: &str) -> (Option<String>, Option<String>) {
    let url = output
        .split_whitespace()
        .find(|word| word.starts_with("https://"))
        .map(str::to_string);
    let code = regex::Regex::new(r"\b[A-Z0-9]{4}-[A-Z0-9]{4}\b")
        .ok()
        .and_then(|re| re.find(output).map(|m| m.as_str().to_string()));
    (url, code)
}

/// Resolution errors that a new `aws sso login` / `aws login` fixes
fn needs_login(kind: Option<AwsProfileKind>, error: &str) -> bool {
    let error = error.to_lowercase();
    matches!(
        kind,
        Some(AwsProfileKind::Sso | AwsProfileKind::ConsoleLogin)
    ) || error.contains("expired")
        || (error.contains("sso") && error.contains("token"))
}

fn timestamp(time: SystemTime) -> Option<Timestamp> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as f64)
}

impl AwsAuthService {
    pub fn new(db: Db, bus: EventBus, env: DetectEnv) -> Self {
        Self {
            db,
            bus,
            env,
            aws_cli: None,
            profile_files: None,
            logins: Mutex::default(),
            setups: Mutex::default(),
            sso: setup::aws_sso(),
        }
    }

    /// Tests: a fake IAM Identity Center
    #[cfg(test)]
    fn with_sso(mut self, sso: Arc<dyn setup::SsoDirectory>) -> Self {
        self.sso = sso;
        self
    }

    #[cfg(test)]
    fn with_test_files(
        mut self,
        aws_cli: Option<PathBuf>,
        config: PathBuf,
        credentials: PathBuf,
    ) -> Self {
        self.aws_cli = aws_cli;
        self.profile_files = Some((config, credentials));
        self
    }

    /// Claude Code's settings env under the process env: where a user who
    /// set up Claude Code for Bedrock keeps `AWS_PROFILE` / `AWS_REGION`
    fn user_env(&self) -> HashMap<String, String> {
        detection::claude_code_env(&self.env)
    }

    fn aws_cli(&self) -> Option<PathBuf> {
        self.aws_cli
            .clone()
            .or_else(|| detection::find_executable(&["aws"], &self.env))
    }

    /// The AWS CLI, windowless, on this service's AWS files (tests: the
    /// fixture files; otherwise the user's, as the CLI finds them)
    fn cli_command(&self) -> Result<tokio::process::Command, String> {
        let cli = self
            .aws_cli()
            .ok_or_else(|| "The AWS CLI (v2) is not installed".to_string())?;
        let mut command = tokio::process::Command::new(cli);
        command.env("AWS_PAGER", "").kill_on_drop(true);
        if let Some((config, credentials)) = &self.profile_files {
            command
                .env("AWS_CONFIG_FILE", config)
                .env("AWS_SHARED_CREDENTIALS_FILE", credentials);
        }
        #[cfg(windows)]
        {
            command.creation_flags(0x0800_0000);
        }
        Ok(command)
    }

    pub async fn detect_cli(&self) -> AwsCliInfo {
        let Some(path) = self.aws_cli() else {
            return AwsCliInfo::default();
        };
        let mut command = tokio::process::Command::new(&path);
        command
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            command.creation_flags(0x0800_0000);
        }
        let version = tokio::time::timeout(VERSION_TIMEOUT, command.output())
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|out| {
                let text = String::from_utf8_lossy(&out.stdout).to_string()
                    + &String::from_utf8_lossy(&out.stderr);
                super::adapter::parse_version(&text)
            });
        // `aws login` came in a 2.x release: ask the CLI, not its version
        let supports_login = match self.cli_command() {
            Ok(mut help) => {
                help.args(["login", "help"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                tokio::time::timeout(VERSION_TIMEOUT, help.status())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .is_some_and(|status| status.success())
            }
            Err(_) => false,
        };
        AwsCliInfo {
            installed: true,
            supports_login,
            supports_sso: version.as_ref().is_some_and(|v| v.major >= 2),
            version: version.map(|v| v.to_string()),
            path: Some(path.display().to_string()),
        }
    }

    fn sdk_env(&self) -> Env {
        match &self.profile_files {
            Some((config, credentials)) => {
                let mut vars: HashMap<String, String> = self.env.vars.clone();
                vars.insert("AWS_CONFIG_FILE".into(), config.display().to_string());
                vars.insert(
                    "AWS_SHARED_CREDENTIALS_FILE".into(),
                    credentials.display().to_string(),
                );
                Env::from(vars)
            }
            None => Env::real(),
        }
    }

    /// The user's profiles (cached in `aws_profiles`; a change emits
    /// `aws://profile_changed`)
    pub async fn profiles(&self) -> Result<Vec<AwsProfile>, String> {
        let profiles =
            super::aws_config_parser::load_profiles(&Fs::real(), &self.sdk_env()).await?;
        let rows = profiles.clone();
        let changed = self
            .db
            .call(move |conn| {
                let tx = conn.transaction()?;
                let before: Vec<String> = {
                    let mut stmt = tx.prepare("SELECT name FROM aws_profiles ORDER BY name")?;
                    let names = stmt
                        .query_map([], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<_>>()?;
                    names
                };
                tx.execute("DELETE FROM aws_profiles", [])?;
                for p in &rows {
                    tx.execute(
                        "INSERT INTO aws_profiles (name, kind, region, sso_session, sso_start_url,
                             sso_region, sso_account_id, sso_role_name, role_arn, source_profile,
                             updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            p.name,
                            p.kind.as_str(),
                            p.region,
                            p.sso_session,
                            p.sso_start_url,
                            p.sso_region,
                            p.sso_account_id,
                            p.sso_role_name,
                            p.role_arn,
                            p.source_profile,
                            now_ms()
                        ],
                    )?;
                }
                tx.commit()?;
                let after: Vec<String> = rows.iter().map(|p| p.name.clone()).collect();
                Ok((before != after).then_some(after))
            })
            .await?;
        if let Some(names) = changed {
            events::emit(&self.bus, &AwsProfileChanged { profiles: names });
        }
        Ok(profiles)
    }

    pub async fn profile(&self, name: &str) -> Result<AwsProfile, String> {
        self.profiles()
            .await?
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("No AWS profile named '{name}' in your AWS config"))
    }

    /// `profile`, else `AWS_PROFILE` (environment or Claude Code settings)
    fn profile_name(&self, profile: Option<String>) -> Option<String> {
        profile
            .filter(|p| !p.trim().is_empty())
            .or_else(|| not_empty(self.user_env().get("AWS_PROFILE")))
    }

    /// `AWS_REGION` / `AWS_DEFAULT_REGION` (environment or Claude Code
    /// settings), else `us-east-1`. Without resolving credentials.
    pub fn default_region(&self) -> String {
        let env = self.user_env();
        not_empty(env.get("AWS_REGION"))
            .or_else(|| not_empty(env.get("AWS_DEFAULT_REGION")))
            .unwrap_or_else(|| DEFAULT_REGION.to_string())
    }

    fn bearer_token(&self) -> Option<(String, &'static str)> {
        if let Some(token) = not_empty(self.env.vars.get("AWS_BEARER_TOKEN_BEDROCK")) {
            return Some((token, "environment"));
        }
        not_empty(self.user_env().get("AWS_BEARER_TOKEN_BEDROCK"))
            .map(|token| (token, "claude-code-settings"))
    }

    pub fn bedrock_api_key_status(&self) -> BedrockApiKeyStatus {
        let token = self.bearer_token();
        BedrockApiKeyStatus {
            configured: token.is_some(),
            source: token.map(|(_, source)| source.to_string()),
        }
    }

    /// The SDK's default chain for the profile and region
    async fn resolve(
        &self,
        profile: Option<&str>,
        region: Option<&str>,
    ) -> Result<Resolved, String> {
        use aws_config::meta::region::ProvideRegion;
        use aws_config::profile::{ProfileFileCredentialsProvider, ProfileFileRegionProvider};

        let files = self.profile_files.as_ref().map(|(config, credentials)| {
            EnvConfigFiles::builder()
                .with_file(EnvConfigFileKind::Config, config)
                .with_file(EnvConfigFileKind::Credentials, credentials)
                .build()
        });
        let mut loader = aws_config::defaults(BehaviorVersion::latest());
        // A named profile means exactly that profile: the default chain
        // would let AWS_ACCESS_KEY_ID in the environment win over it
        let profile_region = match profile {
            Some(profile) => {
                let mut credentials =
                    ProfileFileCredentialsProvider::builder().profile_name(profile);
                let mut region = ProfileFileRegionProvider::builder().profile_name(profile);
                if let Some(files) = &files {
                    credentials = credentials.profile_files(files.clone());
                    region = region.profile_files(files.clone());
                }
                loader = loader.credentials_provider(credentials.build());
                region.build().region().await.map(|r| r.to_string())
            }
            None => {
                if let Some(files) = &files {
                    loader = loader.profile_files(files.clone());
                }
                None
            }
        };
        // The caller's region, then the profile's, then the environment's
        let region = region
            .map(str::to_string)
            .or(profile_region)
            .or_else(|| not_empty(self.user_env().get("AWS_REGION")));
        if let Some(region) = &region {
            loader = loader.region(Region::new(region.clone()));
        }
        let sdk = loader.load().await;
        let region = sdk
            .region()
            .map(|r| r.to_string())
            .unwrap_or_else(|| DEFAULT_REGION.to_string());
        let provider = sdk
            .credentials_provider()
            .ok_or_else(|| "No AWS credentials are configured".to_string())?;
        let credentials = provider.provide_credentials().await.map_err(|e| {
            let detail = aws_smithy_error(&e);
            match profile {
                Some(profile) => format!("AWS profile '{profile}': {detail}"),
                None => detail,
            }
        })?;
        Ok(Resolved {
            credentials,
            region,
            sdk,
        })
    }

    async fn kind_of(&self, profile: Option<&str>) -> Option<AwsProfileKind> {
        let name = profile.unwrap_or("default");
        self.profiles()
            .await
            .ok()?
            .into_iter()
            .find(|p| p.name == name)
            .map(|p| p.kind)
    }

    /// Whether the profile's credentials resolve, and (with `verify`) whose
    /// they are. An SSO profile that doesn't resolve emits
    /// `aws://credentials_expired`.
    pub async fn check(
        &self,
        profile: Option<String>,
        region: Option<String>,
        verify: bool,
    ) -> CredentialStatus {
        let profile = self.profile_name(profile);
        let kind = self.kind_of(profile.as_deref()).await;
        let checked_at = ts(now_ms());
        match self.resolve(profile.as_deref(), region.as_deref()).await {
            Ok(resolved) => {
                let (account_id, arn, error) = if verify {
                    match aws_sdk_sts::Client::new(&resolved.sdk)
                        .get_caller_identity()
                        .send()
                        .await
                    {
                        Ok(identity) => (
                            identity.account().map(str::to_string),
                            identity.arn().map(str::to_string),
                            None,
                        ),
                        Err(e) => (None, None, Some(identity_error(&aws_smithy_error(&e)))),
                    }
                } else {
                    (None, None, None)
                };
                CredentialStatus {
                    source: kind.map_or(CredentialSource::Environment, AwsProfileKind::source),
                    valid: error.is_none(),
                    profile,
                    region: Some(resolved.region),
                    account_id,
                    arn,
                    expires_at: resolved.credentials.expiry().and_then(timestamp),
                    expired: false,
                    error,
                    checked_at,
                }
            }
            Err(error) if self.bearer_token().is_some() => CredentialStatus {
                source: CredentialSource::BedrockApiKey,
                valid: true,
                profile,
                region: region.or_else(|| not_empty(self.user_env().get("AWS_REGION"))),
                account_id: None,
                arn: None,
                expires_at: None,
                expired: false,
                // The chain failed; the API key is what sessions will use
                error: Some(error),
                checked_at,
            },
            Err(error) => {
                let expired = needs_login(kind, &error);
                if expired {
                    events::emit(
                        &self.bus,
                        &AwsCredentialsExpired {
                            profile: profile.clone(),
                            error: Some(error.clone()),
                        },
                    );
                }
                CredentialStatus {
                    source: kind.map_or(CredentialSource::None, AwsProfileKind::source),
                    valid: false,
                    profile,
                    region,
                    account_id: None,
                    arn: None,
                    expires_at: None,
                    expired,
                    error: Some(error),
                    checked_at,
                }
            }
        }
    }

    /// What a Bedrock session's environment gets: the profile's credentials
    /// exported to temporary keys, else the Bedrock API key
    pub async fn bedrock_credentials(
        &self,
        profile: Option<String>,
        region: Option<String>,
    ) -> Result<HarnessCredentials, String> {
        let profile = self.profile_name(profile);
        let bearer = self.bearer_token().map(|(token, _)| token);
        match self.resolve(profile.as_deref(), region.as_deref()).await {
            Ok(resolved) => Ok(HarnessCredentials {
                aws_region: Some(resolved.region),
                aws_profile: profile,
                aws_access_key_id: Some(resolved.credentials.access_key_id().to_string()),
                aws_secret_access_key: Some(resolved.credentials.secret_access_key().to_string()),
                aws_session_token: resolved.credentials.session_token().map(str::to_string),
                aws_bearer_token: bearer,
                ..HarnessCredentials::default()
            }),
            Err(_) if bearer.is_some() => Ok(HarnessCredentials {
                aws_region: region
                    .or_else(|| not_empty(self.user_env().get("AWS_REGION")))
                    .or_else(|| Some(DEFAULT_REGION.to_string())),
                aws_bearer_token: bearer,
                ..HarnessCredentials::default()
            }),
            Err(error) => Err(self.explain(profile, error).await),
        }
    }

    /// The keys an agent's AWS calls start from: `profile`'s, else the
    /// sign-in profile's (`AWS_PROFILE`, else the SDK's default chain)
    pub async fn agent_keys(&self, profile: Option<String>) -> Result<SignInKeys, String> {
        let profile = self.profile_name(profile);
        match self.resolve(profile.as_deref(), None).await {
            Ok(resolved) => Ok(SignInKeys {
                access_key: resolved.credentials.access_key_id().to_string(),
                secret_key: resolved.credentials.secret_access_key().to_string(),
                session_token: resolved.credentials.session_token().map(str::to_string),
                expires_at: resolved.credentials.expiry(),
                region: resolved.region,
            }),
            Err(error) => Err(self.explain(profile, error).await),
        }
    }

    /// A resolution error, with how to log in again when that fixes it
    /// (which also emits `aws://credentials_expired`)
    async fn explain(&self, profile: Option<String>, error: String) -> String {
        let kind = self.kind_of(profile.as_deref()).await;
        if !needs_login(kind, &error) {
            return error;
        }
        events::emit(
            &self.bus,
            &AwsCredentialsExpired {
                profile: profile.clone(),
                error: Some(error.clone()),
            },
        );
        format!(
            "{error}. Log in again from Settings → Cloud credentials, or run: aws sso login{}",
            profile
                .map(|p| format!(" --profile {p}"))
                .unwrap_or_default()
        )
    }

    /// An SDK config for `region` with the session's identity (Bedrock
    /// catalog calls)
    pub async fn sdk_config(
        &self,
        profile: Option<String>,
        region: Option<String>,
    ) -> Result<SdkConfig, String> {
        let profile = self.profile_name(profile);
        Ok(self
            .resolve(profile.as_deref(), region.as_deref())
            .await?
            .sdk)
    }

    /// Runs `aws sso login --profile <profile>`, or `aws login` for a
    /// console sign-in profile; the CLI opens the browser
    pub async fn start_sso_login(&self, profile: &str) -> Result<SsoLoginResult, String> {
        let found = self.profile(profile).await?;
        let login: &[&str] = match found.kind {
            AwsProfileKind::Sso => &["sso", "login"],
            AwsProfileKind::ConsoleLogin => &["login"],
            _ => {
                return Err(format!(
                    "AWS profile '{profile}' doesn't sign in with SSO or aws login"
                ))
            }
        };
        let mut command = self.cli_command()?;
        command
            .args(login)
            .args(["--profile", profile])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start aws {}: {e}", login.join(" ")))?;
        let output = Arc::new(Mutex::new(String::new()));
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ]
        .into_iter()
        .flatten()
        {
            let output = output.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut output) = output.lock() {
                        output.push_str(&line);
                        output.push('\n');
                    }
                }
            });
        }
        let result = SsoLoginResult {
            login_id: uuid::Uuid::new_v4().to_string(),
            profile: profile.to_string(),
            status: SsoLoginStatus::Pending,
            verification_uri: None,
            user_code: None,
            started_at: ts(now_ms()),
            error: None,
        };
        if let Ok(mut logins) = self.logins.lock() {
            logins.insert(
                result.login_id.clone(),
                Login {
                    result: result.clone(),
                    child: Some(child),
                    output,
                    started: Instant::now(),
                },
            );
        }
        Ok(result)
    }

    pub fn poll_sso_login(&self, login_id: &str) -> Result<SsoLoginResult, String> {
        let mut logins = self.logins.lock().map_err(|e| e.to_string())?;
        let login = logins
            .get_mut(login_id)
            .ok_or_else(|| format!("No SSO login '{login_id}'"))?;
        let text = login.output.lock().map(|o| o.clone()).unwrap_or_default();
        let (url, code) = parse_sso_output(&text);
        login.result.verification_uri = url.or(login.result.verification_uri.take());
        login.result.user_code = code.or(login.result.user_code.take());
        if let Some(child) = login.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    login.child = None;
                    if status.success() {
                        login.result.status = SsoLoginStatus::Succeeded;
                    } else {
                        login.result.status = SsoLoginStatus::Failed;
                        let last = text.lines().rev().find(|l| !l.trim().is_empty());
                        login.result.error =
                            Some(last.unwrap_or("aws sso login failed").trim().to_string());
                    }
                }
                Ok(None) if login.started.elapsed() > SSO_LOGIN_TIMEOUT => {
                    let _ = child.start_kill();
                    login.child = None;
                    login.result.status = SsoLoginStatus::Expired;
                    login.result.error = Some("The login wasn't approved in time".to_string());
                }
                Ok(None) => {}
                Err(e) => {
                    login.child = None;
                    login.result.status = SsoLoginStatus::Failed;
                    login.result.error = Some(e.to_string());
                }
            }
        }
        Ok(login.result.clone())
    }
}

/// What a failed `GetCallerIdentity` means, in words; the raw error when
/// it's none of the usual ones
fn identity_error(raw: &str) -> String {
    let says = |code: &str| raw.contains(code);
    if says("InvalidClientTokenId") {
        "AWS doesn't know this access key: it was deleted or mistyped. Replace the keys."
            .to_string()
    } else if says("SignatureDoesNotMatch") {
        "The secret access key doesn't match the key ID. Replace the keys.".to_string()
    } else if says("ExpiredToken") {
        "The sign-in has expired: log in again.".to_string()
    } else if says("AccessDenied") {
        "AWS refused this identity (access denied).".to_string()
    } else {
        raw.to_string()
    }
}

/// An SDK error with its causes (the top-level message is often generic)
fn aws_smithy_error(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

impl AwsAuthService {
    /// Reads this machine's AWS config. Tests get an empty home inside
    /// `app_data_dir` instead.
    pub fn for_app(db: Db, bus: EventBus, app_data_dir: &Path) -> Self {
        let env = if cfg!(test) {
            let empty = app_data_dir.join("aws-test-home");
            DetectEnv {
                path: Some(empty.clone().into_os_string()),
                home: empty,
                vars: HashMap::new(),
            }
        } else {
            DetectEnv::current()
        };
        Self::new(db, bus, env)
    }
}

/// Agents' AWS identity from `service`'s profiles (`egress::credentials`)
pub fn agent_sign_in(service: Arc<AwsAuthService>) -> SignInFn {
    Arc::new(move |profile| {
        let service = service.clone();
        Box::pin(async move { service.agent_keys(profile).await })
    })
}

/// The AWS credentials a Bedrock session (or the gateway) gets
pub async fn bedrock_credentials(
    state: &AppState,
    profile: Option<String>,
    region: Option<String>,
) -> Result<HarnessCredentials, String> {
    state.aws_auth().bedrock_credentials(profile, region).await
}

pub async fn detect_aws_cli(state: &AppState) -> Result<AwsCliInfo, String> {
    Ok(state.aws_auth().detect_cli().await)
}

pub async fn list_aws_profiles(state: &AppState) -> Result<Vec<AwsProfile>, String> {
    state.aws_auth().profiles().await
}

pub async fn get_aws_profile(state: &AppState, name: &str) -> Result<AwsProfile, String> {
    state.aws_auth().profile(name).await
}

pub async fn start_aws_sso_login(
    state: &AppState,
    profile: &str,
) -> Result<SsoLoginResult, String> {
    state.aws_auth().start_sso_login(profile).await
}

pub async fn poll_aws_sso_login(
    state: &AppState,
    login_id: &str,
) -> Result<SsoLoginResult, String> {
    state.aws_auth().poll_sso_login(login_id)
}

pub async fn save_aws_key_profile(
    state: &AppState,
    input: KeyProfileInput,
) -> Result<AwsProfile, String> {
    state.aws_auth().save_key_profile(input).await
}

pub async fn start_aws_sso_setup(
    state: &AppState,
    input: SsoSetupInput,
) -> Result<SsoSetup, String> {
    state.aws_auth().start_sso_setup(input).await
}

pub async fn poll_aws_sso_setup(state: &AppState, setup_id: &str) -> Result<SsoSetup, String> {
    state.aws_auth().poll_sso_setup(setup_id).await
}

pub async fn finish_aws_sso_setup(
    state: &AppState,
    finish: SsoSetupFinish,
) -> Result<AwsProfile, String> {
    state.aws_auth().finish_sso_setup(finish).await
}

pub async fn cancel_aws_sso_setup(state: &AppState, setup_id: &str) -> Result<(), String> {
    state.aws_auth().cancel_sso_setup(setup_id);
    Ok(())
}

pub async fn check_aws_credentials(
    state: &AppState,
    profile: Option<String>,
    region: Option<String>,
) -> Result<CredentialStatus, String> {
    Ok(state.aws_auth().check(profile, region, true).await)
}

pub async fn get_aws_bedrock_api_key_status(
    state: &AppState,
) -> Result<BedrockApiKeyStatus, String> {
    Ok(state.aws_auth().bedrock_api_key_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;

    const CONFIG: &str = "[profile keys]\nregion = eu-west-1\n\n[profile sso]\nsso_start_url = https://nandi-test-RANDOM.awsapps.com/start\nsso_region = us-east-1\nsso_account_id = 111122223333\nsso_role_name = Bedrock\nregion = us-west-2\n";
    const CREDENTIALS: &str =
        "[keys]\naws_access_key_id = AKIATESTKEY\naws_secret_access_key = test-secret-value\n";

    pub(super) struct Fixture {
        pub(super) service: AwsAuthService,
        pub(super) sink: Arc<RecordingSink>,
        pub(super) _dir: tempfile::TempDir,
    }

    pub(super) fn fixture(vars: &[(&str, &str)], aws_cli: Option<PathBuf>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let credentials = dir.path().join("credentials");
        // A start URL no real SSO cache can have a token for
        let unique = CONFIG.replace("RANDOM", &uuid::Uuid::new_v4().simple().to_string());
        std::fs::write(&config, unique).unwrap();
        std::fs::write(&credentials, CREDENTIALS).unwrap();
        let db = Db::in_memory();
        let sink = Arc::new(RecordingSink::default());
        let bus = EventBus::new(db.clone(), sink.clone());
        let env = DetectEnv {
            home: dir.path().to_path_buf(),
            path: Some(dir.path().as_os_str().to_owned()),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        let service =
            AwsAuthService::new(db, bus, env).with_test_files(aws_cli, config, credentials);
        Fixture {
            service,
            sink,
            _dir: dir,
        }
    }

    #[test]
    fn identity_errors_read_as_what_to_do() {
        let raw = "service error: unhandled error (InvalidClientTokenId): Error { code: \"InvalidClientTokenId\" }";
        assert!(identity_error(raw).contains("Replace the keys"));
        assert!(identity_error("SignatureDoesNotMatch: x").contains("doesn't match"));
        assert!(identity_error("ExpiredToken").contains("log in again"));
        assert_eq!(identity_error("dns error"), "dns error");
    }

    #[tokio::test]
    async fn static_keys_resolve_and_export_to_the_container_env() {
        let f = fixture(&[], None);
        let credentials = f
            .service
            .bedrock_credentials(Some("keys".into()), None)
            .await
            .unwrap();
        assert_eq!(credentials.aws_region.as_deref(), Some("eu-west-1"));
        let env: HashMap<_, _> = super::super::adapter::common_credential_env(&credentials)
            .into_iter()
            .collect();
        assert_eq!(env["AWS_ACCESS_KEY_ID"], "AKIATESTKEY");
        assert_eq!(env["AWS_SECRET_ACCESS_KEY"], "test-secret-value");
        assert_eq!(env["AWS_REGION"], "eu-west-1");
        assert!(!env.contains_key("AWS_PROFILE"), "exported keys replace it");
        let status = f.service.check(Some("keys".into()), None, false).await;
        assert!(status.valid, "{status:?}");
        assert_eq!(status.source, CredentialSource::SharedCredentials);
        assert!(!status.expired);
    }

    #[tokio::test]
    async fn an_sso_profile_without_a_valid_token_needs_a_login() {
        let f = fixture(&[], None);
        let status = f.service.check(Some("sso".into()), None, false).await;
        assert!(!status.valid);
        assert!(status.expired, "{status:?}");
        assert_eq!(status.source, CredentialSource::Sso);
        assert!(f
            .sink
            .names()
            .contains(&super::super::AWS_CREDENTIALS_EXPIRED));
        let error = f
            .service
            .bedrock_credentials(Some("sso".into()), None)
            .await
            .unwrap_err();
        assert!(error.contains("aws sso login --profile sso"), "{error}");
    }

    #[tokio::test]
    async fn a_bedrock_api_key_carries_a_session_alone() {
        let f = fixture(
            &[
                ("AWS_BEARER_TOKEN_BEDROCK", "bedrock-key-value"),
                ("AWS_REGION", "us-east-2"),
            ],
            None,
        );
        assert_eq!(
            f.service.bedrock_api_key_status(),
            BedrockApiKeyStatus {
                configured: true,
                source: Some("environment".into())
            }
        );
        let credentials = f
            .service
            .bedrock_credentials(Some("sso".into()), None)
            .await
            .unwrap();
        assert_eq!(
            credentials.aws_bearer_token.as_deref(),
            Some("bedrock-key-value")
        );
        assert_eq!(credentials.aws_access_key_id, None);
        assert_eq!(credentials.aws_region.as_deref(), Some("us-east-2"));
        let status = f.service.check(Some("sso".into()), None, false).await;
        assert_eq!(status.source, CredentialSource::BedrockApiKey);
        assert!(status.valid);
    }

    #[tokio::test]
    async fn the_profile_list_is_cached_and_changes_are_announced() {
        let f = fixture(&[("AWS_PROFILE", "sso")], None);
        let profiles = f.service.profiles().await.unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().any(|p| p.name == "sso" && p.is_default));
        assert!(f.sink.names().contains(&super::super::AWS_PROFILE_CHANGED));
        f.sink.take();
        f.service.profiles().await.unwrap();
        assert!(f.sink.names().is_empty(), "no change, no event");
        let cached: i64 = f
            .service
            .db
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM aws_profiles", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(cached, 2);
    }

    /// A fake `aws` that prints what `aws sso login` prints, then exits
    fn fake_aws(dir: &std::path::Path, exit: i32) -> PathBuf {
        #[cfg(windows)]
        {
            let path = dir.join("aws.cmd");
            std::fs::write(
                &path,
                format!(
                    "@echo off\r\necho Attempting to open the SSO authorization page in your browser.\r\necho https://device.sso.us-east-1.amazonaws.com/\r\necho Then enter the code: WDDD-HRQV\r\nexit /b {exit}\r\n"
                ),
            )
            .unwrap();
            path
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("aws");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho 'Attempting to open the SSO authorization page in your browser.'\necho 'https://device.sso.us-east-1.amazonaws.com/'\necho 'Then enter the code: WDDD-HRQV'\nexit {exit}\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    async fn poll_until_done(service: &AwsAuthService, id: &str) -> SsoLoginResult {
        for _ in 0..400 {
            let result = service.poll_sso_login(id).unwrap();
            if result.status != SsoLoginStatus::Pending {
                // Let the output readers finish
                tokio::time::sleep(Duration::from_millis(50)).await;
                return service.poll_sso_login(id).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the login never finished")
    }

    #[tokio::test]
    async fn sso_login_runs_the_aws_cli_and_reports_the_code() {
        let bin = tempfile::tempdir().unwrap();
        let f = fixture(&[], Some(fake_aws(bin.path(), 0)));
        let started = f.service.start_sso_login("sso").await.unwrap();
        assert_eq!(started.status, SsoLoginStatus::Pending);
        let done = poll_until_done(&f.service, &started.login_id).await;
        assert_eq!(done.status, SsoLoginStatus::Succeeded);
        assert_eq!(done.user_code.as_deref(), Some("WDDD-HRQV"));
        assert_eq!(
            done.verification_uri.as_deref(),
            Some("https://device.sso.us-east-1.amazonaws.com/")
        );
        assert!(
            f.service.start_sso_login("keys").await.is_err(),
            "not an SSO profile"
        );
        assert!(f.service.start_sso_login("missing").await.is_err());
    }

    #[tokio::test]
    async fn a_failed_sso_login_says_why() {
        let bin = tempfile::tempdir().unwrap();
        let f = fixture(&[], Some(fake_aws(bin.path(), 1)));
        let started = f.service.start_sso_login("sso").await.unwrap();
        let done = poll_until_done(&f.service, &started.login_id).await;
        assert_eq!(done.status, SsoLoginStatus::Failed);
        assert!(done.error.is_some());
        assert!(f.service.poll_sso_login("nope").is_err());
    }

    #[test]
    fn sso_output_parsing() {
        let (url, code) = parse_sso_output(
            "If the browser does not open, open the following URL:\n\nhttps://device.sso.eu-west-1.amazonaws.com/\n\nThen enter the code:\n\nABCD-EFGH\n",
        );
        assert_eq!(
            url.as_deref(),
            Some("https://device.sso.eu-west-1.amazonaws.com/")
        );
        assert_eq!(code.as_deref(), Some("ABCD-EFGH"));
        assert_eq!(parse_sso_output("nothing"), (None, None));
    }

    #[tokio::test]
    async fn nothing_the_ui_sees_carries_key_material() {
        let f = fixture(&[("AWS_BEARER_TOKEN_BEDROCK", "bedrock-key-value")], None);
        let status = f.service.check(Some("keys".into()), None, false).await;
        let profiles = f.service.profiles().await.unwrap();
        let key = f.service.bedrock_api_key_status();
        let json = serde_json::to_string(&(status, profiles, key)).unwrap();
        for secret in ["test-secret-value", "AKIATESTKEY", "bedrock-key-value"] {
            assert!(!json.contains(secret), "{secret} leaked: {json}");
        }
    }

    /// No log or print statement in the harness services mentions a secret
    #[test]
    fn harness_code_never_logs_credentials() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/harness");
        let secret = regex::Regex::new(
            r"(?i)(log::\w+!|println!|eprintln!|tracing::\w+!)\([^;]*(api_key|secret|session_token|bearer|password|master_key)",
        )
        .unwrap();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                assert!(
                    !secret.is_match(&source),
                    "{} logs something secret",
                    path.display()
                );
            }
        }
    }
}
