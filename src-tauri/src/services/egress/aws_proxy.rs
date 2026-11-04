//! `POST /aws/{service}/{action}`: an AWS API call on an agent's behalf.
//!
//! Body: `{ agent_id?, params }`. The action must be granted by the agent's
//! skill.md `aws` capabilities. The call runs as the agent's AWS identity
//! (see `credentials`): long-term keys under an STS session scoped to the
//! same grants, so AWS enforces them a second time; temporary sign-in keys
//! as they are, so the check here is the only one.
//!
//! Supported (MVP): S3 `GetObject`, `PutObject`, `ListObjects`.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Value};

use super::credentials::{SessionUse, StsCredentials};
use super::{error, record, EgressKind, EgressLogEntry, EgressState, ProxyContext};
use crate::audit;

const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_OBJECT_BYTES: usize = 20 * 1024 * 1024;
/// Sessions for proxied calls; cached per agent and policy
const PROXY_SESSION_SECS: u32 = 60 * 60;

/// A validated S3 request.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum S3Request {
    GetObject {
        bucket: String,
        key: String,
    },
    PutObject {
        bucket: String,
        key: String,
        body: Vec<u8>,
        content_type: Option<String>,
    },
    ListObjects {
        bucket: String,
        prefix: Option<String>,
        max_keys: Option<i32>,
        continuation_token: Option<String>,
    },
}

impl S3Request {
    fn parse(action: &str, params: Value) -> Result<Self, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Params {
            #[serde(alias = "Bucket")]
            bucket: String,
            #[serde(default, alias = "Key")]
            key: Option<String>,
            #[serde(default, alias = "Body")]
            body: Option<String>,
            #[serde(default, alias = "body_base64")]
            body_base64: Option<String>,
            #[serde(default, alias = "ContentType", alias = "content_type")]
            content_type: Option<String>,
            #[serde(default, alias = "Prefix")]
            prefix: Option<String>,
            #[serde(default, alias = "MaxKeys", alias = "max_keys")]
            max_keys: Option<i32>,
            #[serde(default, alias = "ContinuationToken", alias = "continuation_token")]
            continuation_token: Option<String>,
        }
        let params: Params =
            serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))?;
        if params.bucket.is_empty() || params.bucket.contains('/') {
            return Err("params.bucket must be a bucket name".to_string());
        }
        let key = || {
            params
                .key
                .clone()
                .filter(|k| !k.is_empty())
                .ok_or_else(|| "params.key is required".to_string())
        };
        match action {
            "GetObject" => Ok(Self::GetObject {
                key: key()?,
                bucket: params.bucket,
            }),
            "PutObject" => {
                let body = match (params.body, params.body_base64) {
                    (Some(_), Some(_)) => return Err("send body or bodyBase64, not both".into()),
                    (Some(text), None) => text.into_bytes(),
                    (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|_| "bodyBase64 is not valid base64".to_string())?,
                    (None, None) => Vec::new(),
                };
                Ok(Self::PutObject {
                    key: key()?,
                    bucket: params.bucket,
                    body,
                    content_type: params.content_type,
                })
            }
            "ListObjects" | "ListObjectsV2" => Ok(Self::ListObjects {
                bucket: params.bucket,
                prefix: params.prefix,
                max_keys: params.max_keys,
                continuation_token: params.continuation_token,
            }),
            other => Err(format!("unsupported s3 action '{other}'")),
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::GetObject { .. } => "GetObject",
            Self::PutObject { .. } => "PutObject",
            Self::ListObjects { .. } => "ListObjects",
        }
    }

    /// The ARN IAM authorizes the action against.
    fn resource(&self) -> String {
        match self {
            Self::GetObject { bucket, key } | Self::PutObject { bucket, key, .. } => {
                format!("arn:aws:s3:::{bucket}/{key}")
            }
            Self::ListObjects { bucket, .. } => format!("arn:aws:s3:::{bucket}"),
        }
    }
}

/// Runs an allowed call: `(agent_id, AWS profile, session policy, request)`.
pub(super) type AwsCallFn = Arc<
    dyn Fn(String, Option<String>, String, S3Request) -> BoxFuture<'static, Result<Value, String>>
        + Send
        + Sync,
>;

#[derive(Clone)]
struct AwsProxyState {
    context: ProxyContext,
    call: AwsCallFn,
}

#[derive(Debug, Deserialize)]
struct AwsBody {
    #[serde(default, alias = "agentId")]
    agent_id: Option<String>,
    #[serde(default)]
    params: Value,
}

pub(super) fn router(context: ProxyContext, egress: Arc<EgressState>) -> Router {
    let call: AwsCallFn = Arc::new(move |agent_id, profile, policy, request| {
        let egress = egress.clone();
        Box::pin(
            async move { call_s3(&egress, &agent_id, profile.as_deref(), &policy, request).await },
        )
    });
    router_with(context, call)
}

fn router_with(context: ProxyContext, call: AwsCallFn) -> Router {
    Router::new()
        .route("/aws/{service}/{action}", post(handle_aws))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(AwsProxyState { context, call })
}

async fn handle_aws(
    State(state): State<AwsProxyState>,
    Path((service, action)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<AwsBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let started = Instant::now();
    // Authenticate before looking at the body
    if let Err(response) = state.context.authenticate(&headers, None) {
        return response;
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, &rejection.body_text()),
    };
    let agent_id = match state
        .context
        .authenticate(&headers, body.agent_id.as_deref())
    {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    let service = service.to_ascii_lowercase();
    if service != "s3" {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("unsupported AWS service '{service}' (supported: s3)"),
        );
    }
    let request = match S3Request::parse(&action, body.params) {
        Ok(request) => request,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e),
    };
    let resource = request.resource();
    let target = format!("s3:{} {resource}", request.action());
    let entry = |allowed: bool, status: u16, error: Option<String>| EgressLogEntry {
        ts: audit::now_ms() as f64,
        agent_id: agent_id.clone(),
        kind: EgressKind::Aws,
        target: target.clone(),
        allowed,
        status: Some(status),
        duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
        error,
    };

    let capabilities = match state.context.capabilities(&agent_id) {
        Ok(capabilities) => capabilities,
        Err(response) => return response,
    };
    let policy = capabilities.session_policy();
    let (true, Some(policy)) = (
        capabilities.allows_aws(&service, request.action(), &resource),
        policy,
    ) else {
        let message = format!(
            "s3:{} on {resource} is not granted by this agent's aws capabilities",
            request.action()
        );
        record(entry(false, 403, Some(message.clone()))).await;
        return error(StatusCode::FORBIDDEN, &message);
    };

    let profile = capabilities
        .aws
        .as_ref()
        .and_then(|aws| aws.profile.clone());
    match (state.call)(agent_id.clone(), profile, policy, request).await {
        Ok(result) => {
            record(entry(true, 200, None)).await;
            Json(result).into_response()
        }
        Err(e) => {
            record(entry(true, 502, Some(e.clone()))).await;
            error(StatusCode::BAD_GATEWAY, &e)
        }
    }
}

async fn call_s3(
    egress: &EgressState,
    agent_id: &str,
    profile: Option<&str>,
    policy: &str,
    request: S3Request,
) -> Result<Value, String> {
    let session = egress
        .credentials()
        .session(
            agent_id,
            profile,
            policy,
            PROXY_SESSION_SECS,
            SessionUse::Proxy,
        )
        .await?;
    let client = s3_client(session.credentials, session.region);
    run_s3(&client, request).await
}

fn s3_client(session: StsCredentials, region: String) -> aws_sdk_s3::Client {
    use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region))
        .credentials_provider(Credentials::new(
            session.access_key,
            session.secret_key,
            Some(session.session_token),
            Some(session.expires_at),
            "nandi-sts",
        ))
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

async fn run_s3(client: &aws_sdk_s3::Client, request: S3Request) -> Result<Value, String> {
    use aws_sdk_s3::error::DisplayErrorContext;
    let aws_error =
        |e: &dyn std::error::Error| format!("AWS S3 request failed: {}", DisplayErrorContext(e));

    match request {
        S3Request::GetObject { bucket, key } => {
            let output = client
                .get_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| aws_error(&e))?;
            if output
                .content_length
                .is_some_and(|len| len > MAX_OBJECT_BYTES as i64)
            {
                return Err(format!(
                    "object is larger than {} MB",
                    MAX_OBJECT_BYTES / 1024 / 1024
                ));
            }
            let bytes = output
                .body
                .collect()
                .await
                .map_err(|e| format!("Failed to read the S3 object: {e}"))?
                .into_bytes();
            Ok(json!({
                "contentType": output.content_type,
                "contentLength": bytes.len(),
                "eTag": output.e_tag,
                "bodyBase64": base64::engine::general_purpose::STANDARD.encode(&bytes),
            }))
        }
        S3Request::PutObject {
            bucket,
            key,
            body,
            content_type,
        } => {
            let output = client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(body.into())
                .set_content_type(content_type)
                .send()
                .await
                .map_err(|e| aws_error(&e))?;
            Ok(json!({ "eTag": output.e_tag, "versionId": output.version_id }))
        }
        S3Request::ListObjects {
            bucket,
            prefix,
            max_keys,
            continuation_token,
        } => {
            let output = client
                .list_objects_v2()
                .bucket(bucket)
                .set_prefix(prefix)
                .set_max_keys(max_keys)
                .set_continuation_token(continuation_token)
                .send()
                .await
                .map_err(|e| aws_error(&e))?;
            let objects: Vec<Value> = output
                .contents()
                .iter()
                .map(|object| {
                    json!({
                        "key": object.key,
                        "size": object.size,
                        "eTag": object.e_tag,
                        "lastModified": object.last_modified.map(|t| t.to_string()),
                    })
                })
                .collect();
            Ok(json!({
                "objects": objects,
                "isTruncated": output.is_truncated.unwrap_or(false),
                "nextContinuationToken": output.next_continuation_token,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::context;
    use super::*;
    use crate::runtime::ProxyTokens;
    use crate::services::egress::capabilities::Capabilities;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use std::sync::Mutex;
    use tower::ServiceExt;

    fn capabilities() -> Capabilities {
        serde_yaml::from_str::<Capabilities>(
            r#"
aws:
  services:
    - service: s3
      actions: [GetObject, ListObjects]
      resources: ["arn:aws:s3:::reports", "arn:aws:s3:::reports/*"]
"#,
        )
        .unwrap()
        .validate()
        .unwrap()
    }

    /// `(agent_id, request)` for each call that reached the executor
    type Calls = Arc<Mutex<Vec<(String, S3Request)>>>;

    /// Records calls instead of reaching AWS.
    fn test_router(tokens: ProxyTokens) -> (Router, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let call: AwsCallFn = Arc::new(move |agent_id, _profile, policy, request| {
            assert!(policy.contains("s3:ListBucket"));
            seen.lock().unwrap().push((agent_id, request));
            Box::pin(async { Ok(json!({ "ok": true })) })
        });
        (router_with(context(tokens, capabilities()), call), calls)
    }

    fn post_aws(path: &str, token: Option<&str>, body: Value) -> Request<Body> {
        let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    #[tokio::test]
    async fn runs_granted_actions() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let (app, calls) = test_router(tokens);

        let response = app
            .clone()
            .oneshot(post_aws(
                "/aws/s3/GetObject",
                Some(&token),
                json!({ "agent_id": "writer", "params": { "Bucket": "reports", "Key": "q3.csv" } }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["ok"], true);

        let response = app
            .oneshot(post_aws(
                "/aws/s3/ListObjects",
                Some(&token),
                json!({ "params": { "bucket": "reports", "prefix": "2026/" } }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0],
            (
                "writer".to_string(),
                S3Request::GetObject {
                    bucket: "reports".into(),
                    key: "q3.csv".into()
                }
            )
        );
        assert!(
            matches!(&calls[1].1, S3Request::ListObjects { prefix: Some(p), .. } if p == "2026/")
        );
    }

    #[tokio::test]
    async fn rejects_ungranted_and_unauthenticated_calls() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let (app, calls) = test_router(tokens);
        let send = |path: &'static str, token: Option<&str>, body: Value| {
            app.clone().oneshot(post_aws(path, token, body))
        };
        let get = json!({ "params": { "bucket": "reports", "key": "a" } });

        let response = send("/aws/s3/GetObject", None, get.clone()).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let impersonating =
            json!({ "agent_id": "admin", "params": { "bucket": "reports", "key": "a" } });
        let response = send("/aws/s3/GetObject", Some(&token), impersonating)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // Not granted: action, then resource
        let put = json!({ "params": { "bucket": "reports", "key": "a", "body": "x" } });
        let response = send("/aws/s3/PutObject", Some(&token), put).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let other = json!({ "params": { "bucket": "secrets", "key": "a" } });
        let response = send("/aws/s3/GetObject", Some(&token), other)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        // Unsupported service, action, or params
        for (path, body) in [
            ("/aws/sqs/SendMessage", get.clone()),
            ("/aws/s3/DeleteObject", get.clone()),
            (
                "/aws/s3/GetObject",
                json!({ "params": { "bucket": "reports" } }),
            ),
        ] {
            let response = send(path, Some(&token), body).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        }

        assert!(calls.lock().unwrap().is_empty(), "nothing reached AWS");
    }

    #[test]
    fn parses_put_bodies_and_resources() {
        let put = S3Request::parse(
            "PutObject",
            json!({ "bucket": "b", "key": "k/x.bin", "bodyBase64": "AAE=" }),
        )
        .unwrap();
        assert!(matches!(&put, S3Request::PutObject { body, .. } if body == &[0, 1]));
        assert_eq!(put.resource(), "arn:aws:s3:::b/k/x.bin");

        let list = S3Request::parse("ListObjectsV2", json!({ "bucket": "b" })).unwrap();
        assert_eq!(list.resource(), "arn:aws:s3:::b");
        assert!(S3Request::parse("GetObject", json!({ "bucket": "a/b", "key": "k" })).is_err());
    }
}
