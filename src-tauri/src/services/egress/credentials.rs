//! AWS credentials: where an agent's AWS calls get their identity, and
//! short-lived STS sessions scoped to each agent's capabilities.
//!
//! An agent's identity is, in order: the profile its skill.md names
//! (`aws.profile`), the long-term key in the OS keychain, or the sign-in
//! profile in Settings → Cloud credentials (`AWS_PROFILE`, else the SDK's
//! default chain). Long-term keys are narrowed to the agent's grants with
//! GetFederationToken. Temporary keys (SSO, assume-role,
//! credential_process) can't be narrowed without a role to assume, so they
//! serve only the host's `/aws` proxy, which checks every call against the
//! grants, and never enter a container.
//!
//! Secrets never appear in logs, errors, or `Debug` output.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};

use crate::services::secrets::Secrets;

pub const ACCESS_KEY_ENTRY: &str = "nandi.aws.access_key";
pub const SECRET_KEY_ENTRY: &str = "nandi.aws.secret_key";
pub const REGION_ENTRY: &str = "nandi.aws.region";
pub const DEFAULT_REGION: &str = "us-east-1";

/// STS sessions are reused until this long before they expire
const REFRESH_MARGIN: Duration = Duration::from_secs(60);
/// How long temporary sign-in keys without an expiry are reused
const SIGN_IN_REUSE: Duration = Duration::from_secs(15 * 60);
/// GetFederationToken's allowed range
const MIN_DURATION_SECS: u32 = 900;
const MAX_DURATION_SECS: u32 = 129_600;

#[derive(Clone, PartialEq)]
pub struct AwsCredentials {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
}

impl fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsCredentials")
            .field("access_key", &redact(&self.access_key))
            .field("secret_key", &"<redacted>")
            .field("region", &self.region)
            .finish()
    }
}

/// Temporary credentials from STS.
#[derive(Clone, PartialEq)]
pub struct StsCredentials {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: String,
    pub expires_at: SystemTime,
}

impl fmt::Debug for StsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StsCredentials")
            .field("access_key", &redact(&self.access_key))
            .field("secret_key", &"<redacted>")
            .field("session_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl StsCredentials {
    fn is_fresh(&self, now: SystemTime) -> bool {
        self.expires_at
            .checked_sub(REFRESH_MARGIN)
            .is_some_and(|refresh_at| now < refresh_at)
    }
}

/// Keys from an AWS profile or the environment, as the SDK resolved them.
#[derive(Clone, PartialEq)]
pub struct SignInKeys {
    pub access_key: String,
    pub secret_key: String,
    /// Set for temporary keys (SSO, assume-role, credential_process)
    pub session_token: Option<String>,
    pub expires_at: Option<SystemTime>,
    pub region: String,
}

impl fmt::Debug for SignInKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignInKeys")
            .field("access_key", &redact(&self.access_key))
            .field("secret_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .field("region", &self.region)
            .finish()
    }
}

/// Resolves a profile's keys; `None`: the sign-in profile in Settings.
pub type SignInFn =
    Arc<dyn Fn(Option<String>) -> BoxFuture<'static, Result<SignInKeys, String>> + Send + Sync>;

/// Where a session is used: only keys narrowed to the agent's grants may
/// leave the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionUse {
    /// Calls made by the host's `/aws` proxy, which checks the grants
    Proxy,
    /// Injected into an `sts`-mode agent's container
    Container,
}

/// An agent's AWS session and the region its calls go to.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSession {
    pub credentials: StsCredentials,
    pub region: String,
}

/// `AKIA…WXYZ`: enough to recognize a key, not to use it.
pub fn redact(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "…".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// Stores the long-term key. Empty `access_key` and `secret_key` remove it.
pub fn set_aws_credentials(
    secrets: &Secrets,
    access_key: &str,
    secret_key: &str,
) -> Result<(), String> {
    let (access_key, secret_key) = (access_key.trim(), secret_key.trim());
    if access_key.is_empty() && secret_key.is_empty() {
        secrets.delete(ACCESS_KEY_ENTRY)?;
        return secrets.delete(SECRET_KEY_ENTRY);
    }
    if access_key.is_empty() || secret_key.is_empty() {
        return Err("Both the access key ID and the secret access key are required".to_string());
    }
    if !access_key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("The access key ID should contain only letters and digits".to_string());
    }
    secrets.set(ACCESS_KEY_ENTRY, access_key)?;
    secrets.set(SECRET_KEY_ENTRY, secret_key)
}

pub fn set_aws_region(secrets: &Secrets, region: &str) -> Result<(), String> {
    let region = region.trim();
    let valid = !region.is_empty()
        && region.len() <= 32
        && region
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(format!("'{region}' is not a valid AWS region"));
    }
    secrets.set(REGION_ENTRY, region)
}

pub fn get_aws_region(secrets: &Secrets) -> Result<String, String> {
    Ok(secrets
        .get(REGION_ENTRY)?
        .unwrap_or_else(|| DEFAULT_REGION.to_string()))
}

/// The configured long-term key, if both halves are present.
pub fn get_aws_credentials(secrets: &Secrets) -> Result<Option<AwsCredentials>, String> {
    let (Some(access_key), Some(secret_key)) = (
        secrets.get(ACCESS_KEY_ENTRY)?,
        secrets.get(SECRET_KEY_ENTRY)?,
    ) else {
        return Ok(None);
    };
    Ok(Some(AwsCredentials {
        access_key,
        secret_key,
        region: get_aws_region(secrets)?,
    }))
}

/// Exchanges the long-term key for a scoped session:
/// `(credentials, session name, policy, duration)`.
pub type IssueFn = Arc<
    dyn Fn(
            AwsCredentials,
            String,
            String,
            u32,
        ) -> BoxFuture<'static, Result<StsCredentials, String>>
        + Send
        + Sync,
>;

/// `(agent_id, policy_hash, source)`: a session never outlives a change
/// of the agent's grants or profile
type SessionKey = (String, String, String);

/// A cached session, and whether it's narrowed to the agent's grants
/// (only those may enter a container)
type Cached = (AgentSession, bool);

/// Issues and caches agents' AWS sessions.
pub struct CredentialsManager {
    secrets: Secrets,
    cache: Mutex<HashMap<SessionKey, Cached>>,
    issue: IssueFn,
    /// Set once AWS sign-in is available (after start-up)
    sign_in: OnceLock<SignInFn>,
}

impl CredentialsManager {
    pub fn new(secrets: Secrets) -> Self {
        Self::with_issuer(
            secrets,
            Arc::new(|creds, name, policy, duration| {
                Box::pin(get_federation_token(creds, name, policy, duration))
            }),
        )
    }

    pub fn with_issuer(secrets: Secrets, issue: IssueFn) -> Self {
        Self {
            secrets,
            cache: Mutex::default(),
            issue,
            sign_in: OnceLock::new(),
        }
    }

    /// Where profile and sign-in keys come from. Only the first call counts.
    pub fn set_sign_in(&self, sign_in: SignInFn) {
        let _ = self.sign_in.set(sign_in);
    }

    pub fn secrets(&self) -> &Secrets {
        &self.secrets
    }

    /// A session for `agent_id`'s AWS calls, as `profile` (`None`: the
    /// keychain key, else the sign-in profile), limited to `session_policy`
    /// where AWS allows it. Reused until 60s before it expires.
    ///
    /// Long-term keys go through STS GetFederationToken, which needs no
    /// role to assume: the session gets the intersection of the key's
    /// permissions and the policy. Temporary keys are refused for a
    /// container, since nothing would narrow them there.
    pub async fn session(
        &self,
        agent_id: &str,
        profile: Option<&str>,
        session_policy: &str,
        duration_secs: u32,
        usage: SessionUse,
    ) -> Result<AgentSession, String> {
        let source = profile.map_or_else(|| "default".to_string(), |p| format!("profile:{p}"));
        let key = (agent_id.to_string(), policy_hash(session_policy), source);
        if let Some((cached, _)) = self.lock().get(&key).filter(|(s, narrowed)| {
            s.credentials.is_fresh(SystemTime::now()) && (*narrowed || usage == SessionUse::Proxy)
        }) {
            return Ok(cached.clone());
        }

        let keychain = match profile {
            Some(_) => None,
            None => get_aws_credentials(&self.secrets)?,
        };
        let (session, narrowed) = match keychain {
            Some(long_term) => (
                self.federate(agent_id, long_term, session_policy, duration_secs)
                    .await?,
                true,
            ),
            None => {
                let keys = self.sign_in(profile).await?;
                match keys.session_token {
                    None => {
                        let long_term = AwsCredentials {
                            access_key: keys.access_key,
                            secret_key: keys.secret_key,
                            region: keys.region,
                        };
                        (
                            self.federate(agent_id, long_term, session_policy, duration_secs)
                                .await?,
                            true,
                        )
                    }
                    Some(_) if usage == SessionUse::Container => {
                        let who = profile.map_or_else(
                            || "The sign-in profile".to_string(),
                            |p| format!("AWS profile '{p}'"),
                        );
                        return Err(format!(
                            "{who} gives temporary sign-in keys, which can't be limited to this agent's AWS grants, so they don't go into its container. Use proxy mode (aws.mode: proxy), or add an access key in Settings → Cloud credentials"
                        ));
                    }
                    Some(session_token) => (
                        AgentSession {
                            credentials: StsCredentials {
                                access_key: keys.access_key,
                                secret_key: keys.secret_key,
                                session_token,
                                expires_at: keys
                                    .expires_at
                                    .unwrap_or_else(|| SystemTime::now() + SIGN_IN_REUSE),
                            },
                            region: keys.region,
                        },
                        false,
                    ),
                }
            }
        };
        self.lock().insert(key, (session.clone(), narrowed));
        Ok(session)
    }

    async fn federate(
        &self,
        agent_id: &str,
        long_term: AwsCredentials,
        session_policy: &str,
        duration_secs: u32,
    ) -> Result<AgentSession, String> {
        let region = long_term.region.clone();
        let duration = duration_secs.clamp(MIN_DURATION_SECS, MAX_DURATION_SECS);
        let credentials = (self.issue)(
            long_term,
            session_name(agent_id),
            session_policy.to_string(),
            duration,
        )
        .await?;
        log::info!(
            "Issued AWS session {} for agent '{agent_id}'",
            redact(&credentials.access_key)
        );
        Ok(AgentSession {
            credentials,
            region,
        })
    }

    async fn sign_in(&self, profile: Option<&str>) -> Result<SignInKeys, String> {
        const NOT_CONFIGURED: &str = "AWS credentials are not configured. Add an access key or sign in under Settings → Cloud credentials";
        let sign_in = self.sign_in.get().ok_or(NOT_CONFIGURED)?;
        sign_in(profile.map(str::to_string))
            .await
            .map_err(|e| match profile {
                Some(_) => e,
                None => format!("{NOT_CONFIGURED} ({e})"),
            })
    }

    /// Unexpired cached sessions.
    pub fn active_sessions(&self) -> usize {
        let now = SystemTime::now();
        self.lock()
            .values()
            .filter(|(s, _)| s.credentials.is_fresh(now))
            .count()
    }

    /// Drops every cached session (e.g. after the key changes).
    pub fn clear_sessions(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionKey, Cached>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn policy_hash(policy: &str) -> String {
    Sha256::digest(policy.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// STS federated user names: 2–32 of `[\w+=,.@-]`.
fn session_name(agent_id: &str) -> String {
    let name: String = format!("nandi-{agent_id}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_+=,.@-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    name
}

async fn get_federation_token(
    creds: AwsCredentials,
    name: String,
    policy: String,
    duration_secs: u32,
) -> Result<StsCredentials, String> {
    use aws_sdk_sts::config::{BehaviorVersion, Credentials, Region};
    use aws_sdk_sts::error::DisplayErrorContext;

    let config = aws_sdk_sts::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(creds.region))
        .credentials_provider(Credentials::new(
            creds.access_key,
            creds.secret_key,
            None,
            None,
            "nandi-keychain",
        ))
        .build();
    let output = aws_sdk_sts::Client::from_conf(config)
        .get_federation_token()
        .name(name)
        .policy(policy)
        .duration_seconds(duration_secs as i32)
        .send()
        .await
        .map_err(|e| format!("AWS STS request failed: {}", DisplayErrorContext(&e)))?;
    let session = output
        .credentials
        .ok_or("AWS STS returned no credentials")?;
    let expires_at = UNIX_EPOCH
        + Duration::from_secs(u64::try_from(session.expiration.secs()).unwrap_or_default());
    Ok(StsCredentials {
        access_key: session.access_key_id,
        secret_key: session.secret_access_key,
        session_token: session.session_token,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn configured() -> Secrets {
        let secrets = Secrets::in_memory();
        set_aws_credentials(&secrets, "AKIAEXAMPLE12345", "s3cr3t").unwrap();
        secrets
    }

    /// Counts issues; sessions last `lifetime`.
    fn counting_manager(
        secrets: Secrets,
        lifetime: Duration,
    ) -> (CredentialsManager, Arc<AtomicU32>) {
        let count = Arc::new(AtomicU32::new(0));
        let counter = count.clone();
        let manager = CredentialsManager::with_issuer(
            secrets,
            Arc::new(move |creds, name, _policy, _duration| {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Ok(StsCredentials {
                        access_key: format!("ASIA{n}{name}"),
                        secret_key: format!("from-{}", creds.access_key),
                        session_token: "token".into(),
                        expires_at: SystemTime::now() + lifetime,
                    })
                })
            }),
        );
        (manager, count)
    }

    #[test]
    fn stores_and_removes_credentials() {
        let secrets = Secrets::in_memory();
        assert!(get_aws_credentials(&secrets).unwrap().is_none());

        set_aws_credentials(&secrets, " AKIAEXAMPLE12345 ", "s3cr3t").unwrap();
        set_aws_region(&secrets, "eu-west-2").unwrap();
        let creds = get_aws_credentials(&secrets).unwrap().unwrap();
        assert_eq!(creds.access_key, "AKIAEXAMPLE12345");
        assert_eq!(creds.region, "eu-west-2");
        assert_eq!(
            secrets.get(SECRET_KEY_ENTRY).unwrap().as_deref(),
            Some("s3cr3t")
        );

        set_aws_credentials(&secrets, "", "").unwrap();
        assert!(get_aws_credentials(&secrets).unwrap().is_none());
    }

    #[test]
    fn validates_input() {
        let secrets = Secrets::in_memory();
        assert!(set_aws_credentials(&secrets, "AKIA", "").is_err());
        assert!(set_aws_credentials(&secrets, "AKIA X", "s").is_err());
        assert!(set_aws_region(&secrets, "Not A Region").is_err());
        assert_eq!(get_aws_region(&secrets).unwrap(), DEFAULT_REGION);
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let creds = StsCredentials {
            access_key: "ASIAEXAMPLE98765".into(),
            secret_key: "very-secret".into(),
            session_token: "session-token".into(),
            expires_at: UNIX_EPOCH,
        };
        let shown = format!("{creds:?}");
        assert!(!shown.contains("very-secret"));
        assert!(!shown.contains("session-token"));
        assert!(!shown.contains("ASIAEXAMPLE98765"));
        assert!(shown.contains("ASIA…8765"));

        let creds = get_aws_credentials(&configured()).unwrap().unwrap();
        assert!(!format!("{creds:?}").contains("s3cr3t"));
    }

    /// A session for the proxy, as the keychain key or sign-in profile
    async fn proxy_session(
        manager: &CredentialsManager,
        agent_id: &str,
        policy: &str,
    ) -> Result<AgentSession, String> {
        manager
            .session(agent_id, None, policy, 3600, SessionUse::Proxy)
            .await
    }

    /// Sign-in that resolves `profile` (`None` → `"signed-in"`) to
    /// temporary keys, or to long-term ones for `"static"`
    fn sign_in() -> SignInFn {
        Arc::new(|profile: Option<String>| {
            Box::pin(async move {
                let name = profile.unwrap_or_else(|| "signed-in".into());
                if name == "missing" {
                    return Err(format!("AWS profile '{name}': not found"));
                }
                Ok(SignInKeys {
                    access_key: format!("KEY-{name}"),
                    secret_key: "profile-secret".into(),
                    session_token: (name != "static").then(|| format!("token-{name}")),
                    expires_at: None,
                    region: "eu-west-1".into(),
                })
            })
        })
    }

    #[tokio::test]
    async fn caches_per_agent_and_policy() {
        let (manager, count) = counting_manager(configured(), Duration::from_secs(3600));

        let first = proxy_session(&manager, "writer", "{}").await.unwrap();
        let again = proxy_session(&manager, "writer", "{}").await.unwrap();
        assert_eq!(first, again);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(first.credentials.secret_key.ends_with("AKIAEXAMPLE12345"));
        assert_eq!(first.region, DEFAULT_REGION);

        proxy_session(&manager, "writer", "{\"x\":1}")
            .await
            .unwrap();
        proxy_session(&manager, "reader", "{}").await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(manager.active_sessions(), 3);

        manager.clear_sessions();
        proxy_session(&manager, "writer", "{}").await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn refreshes_sessions_near_expiry() {
        // Inside the 60s margin, so never reused
        let (manager, count) = counting_manager(configured(), Duration::from_secs(30));
        proxy_session(&manager, "writer", "{}").await.unwrap();
        proxy_session(&manager, "writer", "{}").await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(manager.active_sessions(), 0);
    }

    #[tokio::test]
    async fn requires_configured_credentials() {
        let (manager, count) = counting_manager(Secrets::in_memory(), Duration::from_secs(3600));
        let err = proxy_session(&manager, "writer", "{}").await.unwrap_err();
        assert!(err.contains("not configured"), "{err}");
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn without_a_key_agents_use_the_sign_in() {
        let (manager, count) = counting_manager(Secrets::in_memory(), Duration::from_secs(3600));
        manager.set_sign_in(sign_in());

        // Temporary keys: as they are, for the proxy only
        let session = proxy_session(&manager, "writer", "{}").await.unwrap();
        assert_eq!(session.credentials.session_token, "token-signed-in");
        assert_eq!(session.region, "eu-west-1");
        assert_eq!(count.load(Ordering::SeqCst), 0, "nothing to federate");
        // Not even from the proxy's cache
        let err = manager
            .session("writer", None, "{}", 3600, SessionUse::Container)
            .await
            .unwrap_err();
        assert!(
            err.contains("sign-in profile") && err.contains("proxy"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn an_agent_s_own_profile_wins_over_the_key() {
        let (manager, count) = counting_manager(configured(), Duration::from_secs(3600));
        manager.set_sign_in(sign_in());

        let own = manager
            .session("writer", Some("work"), "{}", 3600, SessionUse::Proxy)
            .await
            .unwrap();
        assert_eq!(own.credentials.access_key, "KEY-work");
        assert_eq!(count.load(Ordering::SeqCst), 0);

        // Long-term profile keys are narrowed like the keychain key, so
        // they may enter a container
        let narrowed = manager
            .session("writer", Some("static"), "{}", 3600, SessionUse::Container)
            .await
            .unwrap();
        assert_eq!(narrowed.credentials.secret_key, "from-KEY-static");
        assert_eq!(narrowed.region, "eu-west-1");
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // Cached per profile: the keychain key's session is separate
        let keychain = proxy_session(&manager, "writer", "{}").await.unwrap();
        assert!(keychain
            .credentials
            .secret_key
            .ends_with("AKIAEXAMPLE12345"));
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(manager.active_sessions(), 3);

        let err = manager
            .session("writer", Some("missing"), "{}", 3600, SessionUse::Proxy)
            .await
            .unwrap_err();
        assert!(err.contains("'missing'"), "{err}");
    }

    #[test]
    fn sign_in_keys_debug_redacts_secrets() {
        let keys = SignInKeys {
            access_key: "ASIAEXAMPLE98765".into(),
            secret_key: "very-secret".into(),
            session_token: Some("session-token".into()),
            expires_at: None,
            region: "eu-west-1".into(),
        };
        let shown = format!("{keys:?}");
        assert!(!shown.contains("very-secret") && !shown.contains("session-token"));
        assert!(!shown.contains("ASIAEXAMPLE98765"));
    }

    #[test]
    fn session_names_fit_sts_rules() {
        assert_eq!(session_name("writer"), "nandi-writer");
        let long = session_name(&"a".repeat(64));
        assert_eq!(long.len(), 32);
        assert_eq!(redact("short"), "…");
    }
}
