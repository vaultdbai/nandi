//! Claude Code over ACP, through Anthropic's official ACP wrapper
//! `@agentclientprotocol/claude-agent-acp` (Claude Code itself has no ACP
//! flag: `claude --help` in 2.1.301 lists none).
//!
//! Tested with Claude Code 2.1.301/2.1.302 and claude-agent-acp 0.93.0.
//!
//! The provider is Claude Code's own: the switches it reads (from
//! `~/.claude/settings.json` `env` and the environment) are forwarded as
//! they are, so a machine where Claude Code already runs on Bedrock needs
//! no Nandi setup. Its keys and tokens never are: the container reaches
//! its model through the egress proxy (`egress::model_proxy`) with only
//! the session's proxy token, and the host adds the real credential:
//! - Anthropic, a Claude subscription, a team gateway:
//!   `ANTHROPIC_BASE_URL=http://nandi-egress:11435/anthropic` and the
//!   token in `ANTHROPIC_AUTH_TOKEN` ([`ModelRoute::Anthropic`])
//! - Microsoft Foundry: `CLAUDE_CODE_USE_FOUNDRY=1`, the same base in
//!   `ANTHROPIC_FOUNDRY_BASE_URL` and the token in
//!   `ANTHROPIC_FOUNDRY_API_KEY` ([`ModelRoute::Foundry`])
//! - Bedrock: `CLAUDE_CODE_USE_BEDROCK=1`, `AWS_REGION`, and exported STS
//!   keys or `AWS_BEARER_TOKEN_BEDROCK`
//! - The LiteLLM gateway (third-party Bedrock models): `ANTHROPIC_BASE_URL`
//!   and no provider switch
//! - The model: `ANTHROPIC_MODEL`
//!
//! What the host signs in with is [`host_upstream`]'s: a key or gateway
//! token in Claude Code's settings, else a subscription token
//! (`CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`, else the access
//! token of the host's current sign-in, `detection::claude_code_oauth_token`).

use std::collections::{BTreeMap, HashMap};

use super::adapter::{
    common_credential_env, forward_env, proxy_token, CliAdapter, CliProfile, HarnessConfig,
    Invocation, ModelRef, ModelRoute,
};
use super::detection::{self, DetectEnv};
use crate::services::egress::model_proxy::{ModelUpstream, Secret, UpstreamAuth};

/// The ACP wrapper the tools volume installs (`images::tools`, which pins
/// its package and version)
pub const ACP_BINARY: &str = "claude-agent-acp";

/// Claude Code settings Nandi forwards: provider switches, regions, models
/// and limits. Never a key, a token or an endpoint: those stay on the host
/// ([`host_upstream`])
pub const FORWARDED: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "AWS_REGION",
    "AWS_PROFILE",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "CLOUD_ML_REGION",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "DISABLE_PROMPT_CACHING",
];

/// Whether Claude Code's own settings (`~/.claude/settings.json` `env`
/// and the environment) carry credentials or a provider a container
/// session can use. The host's sign-in (`claude login`) stays on the host,
/// but its token may serve the session (`detection::container_auth`).
pub fn forwards_credentials(env: &DetectEnv) -> bool {
    const AUTH: &[&str] = &[
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "AWS_BEARER_TOKEN_BEDROCK",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_FOUNDRY_API_KEY",
    ];
    !forward_env(&detection::claude_code_env(env), AUTH).is_empty()
}

/// Switches that pick a provider: a run that brings its own drops them
const PROVIDER_SWITCHES: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// Where a Claude Code session's model calls go when the host signs them
/// in with what Claude Code itself is set up with on the host, in order:
/// - Foundry (`CLAUDE_CODE_USE_FOUNDRY` with its endpoint and key)
/// - a gateway token or key in its settings (`ANTHROPIC_AUTH_TOKEN`, then
///   `ANTHROPIC_API_KEY`, at `ANTHROPIC_BASE_URL` when set)
/// - a subscription token (`claude setup-token`, else the host's sign-in)
///
/// `None` for Bedrock and Vertex, which sign in their own way, and when
/// the host has nothing to sign in with.
pub fn host_upstream(env: &DetectEnv) -> Option<(ModelRoute, ModelUpstream)> {
    let own = detection::claude_code_env(env);
    let set = |name: &str| setting(&own, name);
    if set("CLAUDE_CODE_USE_BEDROCK").is_some() || set("CLAUDE_CODE_USE_VERTEX").is_some() {
        return None;
    }
    if set("CLAUDE_CODE_USE_FOUNDRY").is_some() {
        let key = set("ANTHROPIC_FOUNDRY_API_KEY")?;
        let upstream = ModelUpstream::anthropic(
            foundry_endpoint(&own)?,
            UpstreamAuth::ApiKey(Secret::new(key)),
        );
        return Some((ModelRoute::Foundry, upstream));
    }
    let base = set("ANTHROPIC_BASE_URL").unwrap_or_else(|| ModelUpstream::ANTHROPIC_URL.into());
    if let Some(token) = set("ANTHROPIC_AUTH_TOKEN") {
        let upstream = ModelUpstream::anthropic(base, UpstreamAuth::Bearer(Secret::new(token)));
        return Some((ModelRoute::Anthropic, upstream));
    }
    if let Some(key) = set("ANTHROPIC_API_KEY") {
        let upstream = ModelUpstream::anthropic(base, UpstreamAuth::ApiKey(Secret::new(key)));
        return Some((ModelRoute::Anthropic, upstream));
    }
    let token = detection::claude_code_oauth_token(env)?;
    let upstream = ModelUpstream::anthropic(
        ModelUpstream::ANTHROPIC_URL,
        UpstreamAuth::ClaudeSubscription(Secret::new(token)),
    );
    Some((ModelRoute::Anthropic, upstream))
}

/// Foundry's Anthropic endpoint from Claude Code's settings: its base URL,
/// else the one its resource name stands for
pub fn foundry_endpoint(own: &HashMap<String, String>) -> Option<String> {
    setting(own, "ANTHROPIC_FOUNDRY_BASE_URL").or_else(|| {
        setting(own, "ANTHROPIC_FOUNDRY_RESOURCE")
            .map(|resource| format!("https://{resource}.services.ai.azure.com/anthropic"))
    })
}

/// A Claude Code setting, when it's set to something
fn setting(own: &HashMap<String, String>, name: &str) -> Option<String> {
    own.get(name).filter(|v| !v.trim().is_empty()).cloned()
}

pub struct ClaudeCode {
    env: DetectEnv,
}

pub type ClaudeCodeAdapter = CliAdapter<ClaudeCode>;

impl ClaudeCodeAdapter {
    pub fn with_env(env: DetectEnv) -> Self {
        CliAdapter::new(ClaudeCode { env })
    }
}

/// Points Claude Code at the egress proxy for `route`, with the session's
/// proxy token as its only credential
fn route_through_proxy(env: &mut BTreeMap<String, String>, route: ModelRoute, token: &str) {
    for name in PROVIDER_SWITCHES {
        env.remove(*name);
    }
    let url = crate::services::egress::model_proxy_url(route.api());
    match route {
        ModelRoute::Anthropic => {
            env.insert("ANTHROPIC_BASE_URL".into(), url);
            env.insert("ANTHROPIC_AUTH_TOKEN".into(), token.into());
        }
        ModelRoute::Foundry => {
            env.insert("CLAUDE_CODE_USE_FOUNDRY".into(), "1".into());
            env.insert("ANTHROPIC_FOUNDRY_BASE_URL".into(), url);
            env.insert("ANTHROPIC_FOUNDRY_API_KEY".into(), token.into());
        }
    }
}

impl CliProfile for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        // Claude Code's aliases resolve per provider (Anthropic or Bedrock);
        // Bedrock IDs come from the Bedrock catalog
        vec![
            ModelRef::new("sonnet", "Claude Sonnet (latest)", "anthropic"),
            ModelRef::new("opus", "Claude Opus (latest)", "anthropic"),
            ModelRef::new("haiku", "Claude Haiku (latest)", "anthropic"),
        ]
    }

    fn invocation(&self, _model: Option<&str>) -> Invocation {
        Invocation::new(ACP_BINARY, &[])
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let own = detection::claude_code_env(&self.env);
        let mut env: BTreeMap<String, String> = forward_env(&own, FORWARDED).into_iter().collect();
        // The container reaches only the proxy, not Anthropic's other hosts:
        // no update checks, telemetry or release notes
        env.insert(
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
            "1".into(),
        );
        let credentials = &config.credentials;
        if credentials.aws_access_key_id.is_some() || credentials.aws_bearer_token.is_some() {
            for name in PROVIDER_SWITCHES {
                env.remove(*name);
            }
            env.insert("CLAUDE_CODE_USE_BEDROCK".into(), "1".into());
        }
        env.extend(common_credential_env(credentials));
        // No ~/.aws in the container: a profile name would only fail
        if credentials.aws_access_key_id.is_some() {
            env.remove("AWS_PROFILE");
        }
        if let (Some(route), Some(token)) = (config.model_route, proxy_token(config)) {
            route_through_proxy(&mut env, route, token);
        }
        if let Some(gateway) = config.gateway.as_ref().and_then(|g| g.endpoint.as_ref()) {
            // The gateway speaks Anthropic Messages and owns the provider
            for name in PROVIDER_SWITCHES {
                env.remove(*name);
            }
            env.insert("ANTHROPIC_BASE_URL".into(), gateway.clone());
            env.entry("ANTHROPIC_AUTH_TOKEN".into())
                .or_insert_with(|| "nandi-gateway".into());
        }
        if let Some(model) = &config.model {
            env.insert("ANTHROPIC_MODEL".into(), model.clone());
        }
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::services::harness::adapter::{
        GatewayConfig, GatewayKind, HarnessAdapter, HarnessCredentials, Placement, TestSpawn,
        UserMessage,
    };

    const PROXY_TOKEN: &str = "proxy-token-for-this-session";

    fn profile(settings: Option<&str>, vars: &[(&str, &str)]) -> (ClaudeCode, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        if let Some(settings) = settings {
            std::fs::create_dir_all(home.path().join(".claude")).unwrap();
            std::fs::write(home.path().join(".claude/settings.json"), settings).unwrap();
        }
        let env = DetectEnv {
            home: home.path().to_path_buf(),
            path: Some(home.path().as_os_str().to_owned()),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        (ClaudeCode { env }, home)
    }

    fn sign_in_on_host(home: &tempfile::TempDir, access_token: &str, expires_at: i64) {
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/.credentials.json"),
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"{access_token}","refreshToken":"never","expiresAt":{expires_at}}}}}"#
            ),
        )
        .unwrap();
    }

    fn env_map(profile: &ClaudeCode, config: &HarnessConfig) -> HashMap<String, String> {
        profile.env(config).into_iter().collect()
    }

    /// A session config in the test container
    fn in_container() -> HarnessConfig {
        HarnessConfig::new("/w".into(), Placement::for_tests())
    }

    /// A session in the test container that reaches its model on `route`
    fn routed(route: ModelRoute) -> HarnessConfig {
        HarnessConfig {
            model_route: Some(route),
            env: crate::services::egress::proxy_env(PROXY_TOKEN)
                .into_iter()
                .filter_map(|pair| {
                    let (name, value) = pair.split_once('=')?;
                    Some((name.to_string(), value.to_string()))
                })
                .collect(),
            ..in_container()
        }
    }

    fn assert_no_value(env: &HashMap<String, String>, secrets: &[&str]) {
        for secret in secrets {
            assert!(
                !env.values().any(|value| value.contains(secret)),
                "{secret} reached the container"
            );
        }
    }

    #[test]
    fn only_settings_and_env_credentials_reach_a_container() {
        let (signed_in_on_host, _a) = profile(Some(r#"{"env":{"AWS_REGION":"us-west-2"}}"#), &[]);
        assert!(!forwards_credentials(&signed_in_on_host.env));
        let (bedrock, _b) = profile(Some(r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#), &[]);
        assert!(forwards_credentials(&bedrock.env));
        let (key, _c) = profile(None, &[("ANTHROPIC_API_KEY", "sk-ant")]);
        assert!(forwards_credentials(&key.env));
    }

    #[test]
    fn the_hosts_subscription_signs_in_on_the_host_only() {
        let (signed_in, home) = profile(None, &[]);
        sign_in_on_host(&home, "sk-ant-oat", crate::db::now_ms() + 60 * 60 * 1000);
        assert!(detection::container_auth("claude-code", &signed_in.env));

        let (route, upstream) = host_upstream(&signed_in.env).unwrap();
        assert_eq!(route, ModelRoute::Anthropic);
        assert_eq!(
            upstream,
            ModelUpstream::anthropic(
                ModelUpstream::ANTHROPIC_URL,
                UpstreamAuth::ClaudeSubscription(Secret::new("sk-ant-oat")),
            )
        );
        let env = env_map(&signed_in, &routed(route));
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            "http://nandi-egress:11435/anthropic"
        );
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], PROXY_TOKEN);
        assert!(!env.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
        assert_no_value(&env, &["sk-ant-oat", "never"]);

        // An expired login signs nothing in
        sign_in_on_host(&home, "old", 1);
        assert!(!detection::container_auth("claude-code", &signed_in.env));
        assert!(host_upstream(&signed_in.env).is_none());

        // A `claude setup-token` token serves the same way
        let (setup_token, _home) = profile(None, &[("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-long")]);
        assert!(forwards_credentials(&setup_token.env));
        let (_, upstream) = host_upstream(&setup_token.env).unwrap();
        assert_eq!(
            upstream.auth,
            UpstreamAuth::ClaudeSubscription(Secret::new("sk-ant-long"))
        );
        let env = env_map(&setup_token, &routed(ModelRoute::Anthropic));
        assert_no_value(&env, &["sk-ant-long"]);
    }

    #[test]
    fn keys_and_gateways_in_claude_codes_settings_stay_on_the_host() {
        let (keyed, _a) = profile(None, &[("ANTHROPIC_API_KEY", "sk-ant-key")]);
        let (_, upstream) = host_upstream(&keyed.env).unwrap();
        assert_eq!(
            upstream,
            ModelUpstream::anthropic(
                ModelUpstream::ANTHROPIC_URL,
                UpstreamAuth::ApiKey(Secret::new("sk-ant-key")),
            )
        );
        assert_no_value(
            &env_map(&keyed, &routed(ModelRoute::Anthropic)),
            &["sk-ant-key"],
        );

        let (gateway, _b) = profile(
            Some(
                r#"{"env":{"ANTHROPIC_BASE_URL":"https://llm.example.com","ANTHROPIC_AUTH_TOKEN":"sk-gw"}}"#,
            ),
            &[],
        );
        let (_, upstream) = host_upstream(&gateway.env).unwrap();
        assert_eq!(
            upstream,
            ModelUpstream::anthropic(
                "https://llm.example.com",
                UpstreamAuth::Bearer(Secret::new("sk-gw")),
            )
        );
        let env = env_map(&gateway, &routed(ModelRoute::Anthropic));
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            "http://nandi-egress:11435/anthropic"
        );
        assert_no_value(&env, &["sk-gw", "llm.example.com"]);
    }

    #[test]
    fn foundry_goes_through_the_proxy_in_foundry_mode() {
        let (foundry, _home) = profile(
            Some(
                r#"{"env":{"CLAUDE_CODE_USE_FOUNDRY":"1","ANTHROPIC_FOUNDRY_RESOURCE":"r","ANTHROPIC_FOUNDRY_API_KEY":"az-key"}}"#,
            ),
            &[],
        );
        let (route, upstream) = host_upstream(&foundry.env).unwrap();
        assert_eq!(route, ModelRoute::Foundry);
        assert_eq!(
            upstream,
            ModelUpstream::anthropic(
                "https://r.services.ai.azure.com/anthropic",
                UpstreamAuth::ApiKey(Secret::new("az-key")),
            )
        );
        let env = env_map(&foundry, &routed(route));
        assert_eq!(env["CLAUDE_CODE_USE_FOUNDRY"], "1");
        assert_eq!(
            env["ANTHROPIC_FOUNDRY_BASE_URL"],
            "http://nandi-egress:11435/anthropic"
        );
        assert_eq!(env["ANTHROPIC_FOUNDRY_API_KEY"], PROXY_TOKEN);
        assert_no_value(&env, &["az-key", "services.ai.azure.com"]);
    }

    #[test]
    fn bedrock_settings_sign_in_their_own_way() {
        let (bedrock, home) = profile(Some(r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#), &[]);
        sign_in_on_host(&home, "sk-ant-oat", crate::db::now_ms() + 60 * 60 * 1000);
        assert!(host_upstream(&bedrock.env).is_none());
    }

    #[test]
    fn zero_config_bedrock_comes_from_claude_codes_settings() {
        let (profile, _home) = profile(
            Some(
                r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1","AWS_REGION":"us-west-2","ANTHROPIC_MODEL":"us.anthropic.claude-sonnet-5"}}"#,
            ),
            &[("HOME_SECRET", "not forwarded")],
        );
        let env = env_map(&profile, &in_container());
        assert_eq!(
            env.get("CLAUDE_CODE_USE_BEDROCK").map(String::as_str),
            Some("1")
        );
        assert_eq!(env.get("AWS_REGION").map(String::as_str), Some("us-west-2"));
        assert!(env.contains_key("ANTHROPIC_MODEL"));
        assert!(!env.contains_key("HOME_SECRET"), "only known switches");
    }

    #[test]
    fn bedrock_credentials_switch_the_provider_and_drop_the_profile_in_containers() {
        let (profile, _home) = profile(None, &[("AWS_PROFILE", "work")]);
        let config = HarnessConfig {
            model: Some("us.anthropic.claude-sonnet-5".into()),
            credentials: HarnessCredentials {
                aws_region: Some("eu-west-1".into()),
                aws_access_key_id: Some("AKIA".into()),
                aws_secret_access_key: Some("secret".into()),
                aws_session_token: Some("token".into()),
                aws_profile: Some("work".into()),
                ..HarnessCredentials::default()
            },
            ..in_container()
        };
        let env = env_map(&profile, &config);
        assert_eq!(env["CLAUDE_CODE_USE_BEDROCK"], "1");
        assert_eq!(env["AWS_REGION"], "eu-west-1");
        assert_eq!(env["AWS_SESSION_TOKEN"], "token");
        assert_eq!(env["ANTHROPIC_MODEL"], "us.anthropic.claude-sonnet-5");
        assert!(!env.contains_key("AWS_PROFILE"));
    }

    #[test]
    fn a_gateway_owns_the_provider() {
        let (profile, _home) = profile(Some(r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#), &[]);
        let config = HarnessConfig {
            model: Some("bedrock-llama".into()),
            gateway: Some(GatewayConfig {
                kind: GatewayKind::Litellm,
                endpoint: Some("http://host.docker.internal:4000".into()),
            }),
            ..in_container()
        };
        let env = env_map(&profile, &config);
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            "http://host.docker.internal:4000"
        );
        assert!(!env.contains_key("CLAUDE_CODE_USE_BEDROCK"));
        assert_eq!(env["ANTHROPIC_MODEL"], "bedrock-llama");
    }

    #[test]
    fn invocations() {
        let (profile, _home) = profile(None, &[]);
        assert_eq!(
            profile.invocation(None),
            Invocation::new("claude-agent-acp", &[])
        );
    }

    #[tokio::test]
    async fn starts_a_session_sends_and_cancels() {
        let (profile, _home) = profile(Some(r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#), &[]);
        let mut adapter: Box<dyn HarnessAdapter> = {
            let adapter = CliAdapter::new(profile);
            *adapter.test_spawn.lock().unwrap() = Some(TestSpawn {
                agent: Some(crate::services::harness::acp::tests::mock_agent()),
                invocation: None,
                env: vec![],
            });
            Box::new(adapter)
        };
        let config = HarnessConfig::new("/agent/workdir".into(), Placement::for_tests());
        let session = adapter.start(config.clone()).await.unwrap();
        assert_eq!(session.adapter_id, "claude-code");
        assert!(
            adapter.start(config).await.is_err(),
            "one session at a time"
        );

        let mut stream = adapter.send(UserMessage::text("hi")).await.unwrap();
        let mut last = None;
        while let Some(update) = stream.recv().await {
            last = Some(update);
        }
        assert!(matches!(
            last,
            Some(crate::services::harness::acp::AcpSessionUpdate::TurnFinished { .. })
        ));

        let stream = adapter.send(UserMessage::text("wait")).await.unwrap();
        adapter.cancel().await.unwrap();
        drop(stream);
        adapter.stop().await.unwrap();
    }
}
