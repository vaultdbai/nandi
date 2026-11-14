//! AWS Signature V4 for model calls the host makes on a session's behalf
//! (`model_proxy`'s Bedrock route).
//!
//! The keys are the user's own AWS identity, resolved on the host (SSO,
//! static keys, assume-role, `credential_process`) and reused until shortly
//! before they expire. They sign the call on the host and never leave it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;

use super::credentials::{SignInFn, SignInKeys};

/// Keys without an expiry (static ones) are resolved again after this
const REUSE_WITHOUT_EXPIRY: Duration = Duration::from_secs(15 * 60);
/// Temporary keys are resolved again this long before they expire
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

/// Signs requests to one AWS service in one region as the host's identity
#[derive(Clone)]
pub struct AwsSigner {
    service: &'static str,
    region: String,
    profile: Option<String>,
    resolve: SignInFn,
    cached: Arc<Mutex<Option<(SignInKeys, SystemTime)>>>,
}

impl std::fmt::Debug for AwsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsSigner")
            .field("service", &self.service)
            .field("region", &self.region)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl PartialEq for AwsSigner {
    /// The same identity for the same service and region (the cache aside)
    fn eq(&self, other: &Self) -> bool {
        self.service == other.service
            && self.region == other.region
            && self.profile == other.profile
            && Arc::ptr_eq(&self.resolve, &other.resolve)
    }
}

impl AwsSigner {
    /// `resolve` gives `profile`'s keys (`aws_auth::agent_sign_in`)
    pub fn new(
        service: &'static str,
        region: &str,
        profile: Option<String>,
        resolve: SignInFn,
    ) -> Self {
        Self {
            service,
            region: region.to_string(),
            profile,
            resolve,
            cached: Arc::default(),
        }
    }

    /// The headers that sign `method url` with `body`: `authorization`,
    /// `x-amz-date`, and for temporary keys `x-amz-security-token`. `url` is
    /// already percent-encoded, as it goes on the wire.
    pub async fn sign(
        &self,
        method: &str,
        url: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<Vec<(String, String)>, String> {
        let keys = self.keys().await?;
        sign_with(
            &keys,
            self.service,
            &self.region,
            method,
            url,
            content_type,
            body,
            SystemTime::now(),
        )
    }

    async fn keys(&self) -> Result<SignInKeys, String> {
        let now = SystemTime::now();
        if let Some((keys, until)) = self.cached.lock().ok().and_then(|c| c.clone()) {
            if now < until {
                return Ok(keys);
            }
        }
        let keys = (self.resolve)(self.profile.clone()).await?;
        let until = match keys.expires_at {
            Some(expires) => expires.checked_sub(REFRESH_MARGIN).unwrap_or(now),
            None => now + REUSE_WITHOUT_EXPIRY,
        };
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some((keys.clone(), until));
        }
        Ok(keys)
    }
}

/// SigV4 headers for one request (pure, so a fixed clock tests it)
#[allow(clippy::too_many_arguments)]
fn sign_with(
    keys: &SignInKeys,
    service: &str,
    region: &str,
    method: &str,
    url: &str,
    content_type: Option<&str>,
    body: &[u8],
    time: SystemTime,
) -> Result<Vec<(String, String)>, String> {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .ok_or_else(|| format!("'{url}' has no host"))?;
    let identity = Credentials::new(
        &keys.access_key,
        &keys.secret_key,
        keys.session_token.clone(),
        keys.expires_at,
        "nandi",
    )
    .into();
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(service)
        .time(time)
        .settings(SigningSettings::default())
        .build()
        .map_err(|e| format!("Could not sign the AWS call: {e}"))?
        .into();
    let mut headers = vec![("host", host.as_str())];
    if let Some(content_type) = content_type {
        headers.push(("content-type", content_type));
    }
    let request = SignableRequest::new(method, url, headers.into_iter(), SignableBody::Bytes(body))
        .map_err(|e| format!("Could not sign the AWS call: {e}"))?;
    let (instructions, _signature) = sign(request, &params)
        .map_err(|e| format!("Could not sign the AWS call: {e}"))?
        .into_parts();
    Ok(instructions
        .headers()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(session_token: Option<&str>, expires_at: Option<SystemTime>) -> SignInKeys {
        SignInKeys {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: session_token.map(str::to_string),
            expires_at,
            region: "us-east-1".into(),
        }
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn signs_a_bedrock_call_with_the_hosts_keys() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let url = "https://bedrock-runtime.us-east-1.amazonaws.com/model/us.anthropic.claude-sonnet-5%3A0/invoke";
        let headers = sign_with(
            &keys(Some("session"), None),
            "bedrock",
            "us-east-1",
            "POST",
            url,
            Some("application/json"),
            b"{}",
            time,
        )
        .unwrap();

        let authorization = header(&headers, "authorization").unwrap();
        assert!(authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-east-1/bedrock/aws4_request"
        ));
        assert!(authorization.contains("SignedHeaders=content-type;host;x-amz-date"));
        assert_eq!(header(&headers, "x-amz-date"), Some("20231114T221320Z"));
        assert_eq!(header(&headers, "x-amz-security-token"), Some("session"));
        // The same call signs the same way: no randomness to break a retry
        let again = sign_with(
            &keys(Some("session"), None),
            "bedrock",
            "us-east-1",
            "POST",
            url,
            Some("application/json"),
            b"{}",
            time,
        )
        .unwrap();
        assert_eq!(headers, again);
    }

    #[tokio::test]
    async fn reuses_keys_until_shortly_before_they_expire() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let resolve = |expires_at: Option<SystemTime>| -> SignInFn {
            let calls = calls.clone();
            Arc::new(move |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move { Ok(keys(Some("t"), expires_at)) })
            })
        };
        let fresh = AwsSigner::new("bedrock", "us-east-1", None, resolve(None));
        fresh.keys().await.unwrap();
        fresh.keys().await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let expiring = AwsSigner::new(
            "bedrock",
            "us-east-1",
            None,
            resolve(Some(SystemTime::now() + Duration::from_secs(30))),
        );
        expiring.keys().await.unwrap();
        expiring.keys().await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn the_signer_never_prints_keys() {
        let signer = AwsSigner::new(
            "bedrock",
            "us-east-1",
            Some("work".into()),
            Arc::new(|_| Box::pin(async { Err("unused".to_string()) })),
        );
        let printed = format!("{signer:?}");
        assert!(printed.contains("work") && !printed.contains("AKID"));
    }
}
