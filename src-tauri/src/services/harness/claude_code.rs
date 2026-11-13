//! Claude Code over ACP, through Anthropic's official ACP wrapper
//! `@agentclientprotocol/claude-agent-acp` (Claude Code itself has no ACP
//! flag: `claude --help` in 2.1.301 lists none).
//!
//! Tested with Claude Code 2.1.301/2.1.302 and claude-agent-acp 0.93.0.
//!
//! The provider is Claude Code's own: the switches it reads (from
//! `~/.claude/settings.json` `env` and the environment) are forwarded as
//! they are, so a machine where Claude Code already runs on Bedrock needs
//! no Nandi setup. Nandi's credentials, when the run has any, override them:
//! - Anthropic: `ANTHROPIC_API_KEY`
//! - Bedrock: `CLAUDE_CODE_USE_BEDROCK=1`, `AWS_REGION`, and exported STS
//!   keys or `AWS_BEARER_TOKEN_BEDROCK`
//! - Azure AI Foundry: `CLAUDE_CODE_USE_FOUNDRY=1`,
//!   `ANTHROPIC_FOUNDRY_BASE_URL`, `ANTHROPIC_FOUNDRY_API_KEY`
//! - A gateway (LiteLLM for third-party Bedrock models):
//!   `ANTHROPIC_BASE_URL` and no provider switch
//! - The model: `ANTHROPIC_MODEL`
//!
//! A container can't use the host's `claude login` files. With none of the
//! above, it gets a subscription token instead: `CLAUDE_CODE_OAUTH_TOKEN`
//! (from `claude setup-token`), else the access token of the host's
//! current sign-in (`detection::claude_code_oauth_token`).

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::adapter::{
    common_credential_env, forward_env, CliAdapter, CliProfile, HarnessConfig, HarnessHealth,
    Invocation, ModelRef, Placement,
};
use super::detection::{self, DetectEnv};

/// The wrapper's npm package and the version Nandi is tested with (also
/// pinned in `runtimes/nandi-harness/Dockerfile`)
pub const ACP_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp";
pub const ACP_VERSION: &str = "0.93.0";
pub const ACP_BINARY: &str = "claude-agent-acp";

/// Claude Code settings Nandi forwards: provider switches, regions,
/// models, endpoints, and the provider keys the user put there themselves
pub const FORWARDED: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "AWS_REGION",
    "AWS_PROFILE",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "CLOUD_ML_REGION",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "DISABLE_PROMPT_CACHING",
];

/// Whether Claude Code's own settings (`~/.claude/settings.json` `env`
/// and the environment) carry credentials or a provider that a container
/// receives. The host's sign-in (`claude login`) stays on the host, but its
/// token may not (`detection::container_auth`).
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

pub struct ClaudeCode {
    env: DetectEnv,
}

pub type ClaudeCodeAdapter = CliAdapter<ClaudeCode>;

impl ClaudeCodeAdapter {
    pub fn with_env(env: DetectEnv) -> Self {
        CliAdapter::new(ClaudeCode { env })
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

    fn invocation(
        &self,
        placement: &Placement,
        _host_version: Option<&semver::Version>,
        _model: Option<&str>,
    ) -> Invocation {
        match placement {
            Placement::Container { .. } => Invocation::new(ACP_BINARY, &[]),
            // On the host: the wrapper if it's installed, else npx runs the
            // pinned release
            Placement::Host => {
                if detection::find_executable(&[ACP_BINARY], &self.env).is_some() {
                    Invocation::new(ACP_BINARY, &[])
                } else {
                    let package = format!("{ACP_PACKAGE}@{ACP_VERSION}");
                    Invocation::new(npx(), &["-y", &package])
                }
            }
        }
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let own = detection::claude_code_env(&self.env);
        let mut env: std::collections::BTreeMap<String, String> =
            forward_env(&own, FORWARDED).into_iter().collect();
        let credentials = &config.credentials;
        let mut provider: Option<&str> = None;
        if credentials.aws_access_key_id.is_some() || credentials.aws_bearer_token.is_some() {
            provider = Some("CLAUDE_CODE_USE_BEDROCK");
        } else if credentials.azure_api_key.is_some() || credentials.azure_endpoint.is_some() {
            provider = Some("CLAUDE_CODE_USE_FOUNDRY");
        } else if credentials.anthropic_api_key.is_some() {
            provider = Some("");
        }
        if let Some(switch) = provider {
            for name in PROVIDER_SWITCHES {
                env.remove(*name);
            }
            if !switch.is_empty() {
                env.insert(switch.to_string(), "1".to_string());
            }
        }
        if let Some(key) = &credentials.anthropic_api_key {
            env.insert("ANTHROPIC_API_KEY".into(), key.clone());
        }
        if let Some(endpoint) = &credentials.azure_endpoint {
            env.insert("ANTHROPIC_FOUNDRY_BASE_URL".into(), endpoint.clone());
        }
        if let Some(key) = &credentials.azure_api_key {
            env.insert("ANTHROPIC_FOUNDRY_API_KEY".into(), key.clone());
        }
        env.extend(common_credential_env(credentials));
        if matches!(config.placement, Placement::Container { .. }) {
            // No ~/.aws in the container: a profile name would only fail
            if credentials.aws_access_key_id.is_some() {
                env.remove("AWS_PROFILE");
            }
            // Nothing else to sign in with: the host's subscription token
            const SIGN_IN: &[&str] = &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
            ];
            let signed_in = SIGN_IN.iter().any(|name| env.contains_key(*name));
            if !signed_in && config.gateway.is_none() {
                if let Some(token) = detection::claude_code_oauth_token(&self.env) {
                    env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), token);
                }
            }
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

    fn health(&self) -> BoxFuture<'_, HarnessHealth> {
        async move {
            let spec = detection::spec("claude-code").expect("built-in");
            detection::detect(spec, &self.env).await.health
        }
        .boxed()
    }
}

/// `npx` (`npx.cmd` on Windows)
pub fn npx() -> &'static str {
    if cfg!(windows) {
        "npx.cmd"
    } else {
        "npx"
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::services::harness::adapter::{
        GatewayConfig, GatewayKind, HarnessAdapter, HarnessCredentials, TestSpawn, UserMessage,
    };

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

    fn env_map(profile: &ClaudeCode, config: &HarnessConfig) -> HashMap<String, String> {
        profile.env(config).into_iter().collect()
    }

    fn container() -> Placement {
        Placement::Container {
            image: "nandi-harness:latest".into(),
            name: "n".into(),
            host_workdir: "/w".into(),
            memory_mb: None,
            cpu: None,
            network: None,
            folders: Vec::new(),
            tools: Vec::new(),
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
    fn a_container_gets_the_hosts_subscription_token_when_it_has_nothing_else() {
        let expires = crate::db::now_ms() + 60 * 60 * 1000;
        let (signed_in, home) = profile(None, &[]);
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/.credentials.json"),
            format!(
                r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat","refreshToken":"never","expiresAt":{expires}}}}}"#
            ),
        )
        .unwrap();
        assert!(detection::container_auth("claude-code", &signed_in.env));
        let in_container = HarnessConfig {
            placement: container(),
            ..HarnessConfig::default()
        };
        let env = env_map(&signed_in, &in_container);
        assert_eq!(env["CLAUDE_CODE_OAUTH_TOKEN"], "sk-ant-oat");
        assert!(
            !env.values().any(|v| v == "never"),
            "the refresh token stays"
        );
        // On the host the CLI reads its own login
        let on_host = env_map(&signed_in, &HarnessConfig::default());
        assert!(!on_host.contains_key("CLAUDE_CODE_OAUTH_TOKEN"));
        // A Nandi key wins
        let keyed = HarnessConfig {
            placement: container(),
            credentials: HarnessCredentials {
                anthropic_api_key: Some("sk-ant".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        assert!(!env_map(&signed_in, &keyed).contains_key("CLAUDE_CODE_OAUTH_TOKEN"));

        // An expired login isn't forwarded
        std::fs::write(
            home.path().join(".claude/.credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"old","expiresAt":1}}"#,
        )
        .unwrap();
        assert!(!detection::container_auth("claude-code", &signed_in.env));
        assert!(!env_map(&signed_in, &in_container).contains_key("CLAUDE_CODE_OAUTH_TOKEN"));

        // A `claude setup-token` token in the environment is forwarded as is
        let (setup_token, _home) = profile(None, &[("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-long")]);
        assert!(forwards_credentials(&setup_token.env));
        assert_eq!(
            env_map(&setup_token, &in_container)["CLAUDE_CODE_OAUTH_TOKEN"],
            "sk-ant-long"
        );
    }

    #[test]
    fn zero_config_bedrock_comes_from_claude_codes_settings() {
        let (profile, _home) = profile(
            Some(
                r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1","AWS_REGION":"us-west-2","ANTHROPIC_MODEL":"us.anthropic.claude-sonnet-5"}}"#,
            ),
            &[("HOME_SECRET", "not forwarded")],
        );
        let env = env_map(&profile, &HarnessConfig::default());
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
            placement: container(),
            model: Some("us.anthropic.claude-sonnet-5".into()),
            credentials: HarnessCredentials {
                aws_region: Some("eu-west-1".into()),
                aws_access_key_id: Some("AKIA".into()),
                aws_secret_access_key: Some("secret".into()),
                aws_session_token: Some("token".into()),
                aws_profile: Some("work".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env = env_map(&profile, &config);
        assert_eq!(env["CLAUDE_CODE_USE_BEDROCK"], "1");
        assert_eq!(env["AWS_REGION"], "eu-west-1");
        assert_eq!(env["AWS_SESSION_TOKEN"], "token");
        assert_eq!(env["ANTHROPIC_MODEL"], "us.anthropic.claude-sonnet-5");
        assert!(!env.contains_key("AWS_PROFILE"));
    }

    #[test]
    fn anthropic_and_azure_credentials() {
        let (profile, _home) = profile(Some(r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#), &[]);
        let anthropic = HarnessConfig {
            credentials: HarnessCredentials {
                anthropic_api_key: Some("sk-ant".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env = env_map(&profile, &anthropic);
        assert_eq!(env["ANTHROPIC_API_KEY"], "sk-ant");
        assert!(
            !env.contains_key("CLAUDE_CODE_USE_BEDROCK"),
            "the run's provider wins"
        );

        let azure = HarnessConfig {
            credentials: HarnessCredentials {
                azure_endpoint: Some("https://r.services.ai.azure.com".into()),
                azure_api_key: Some("az".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env = env_map(&profile, &azure);
        assert_eq!(env["CLAUDE_CODE_USE_FOUNDRY"], "1");
        assert_eq!(env["ANTHROPIC_FOUNDRY_API_KEY"], "az");
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
            ..HarnessConfig::default()
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
            profile.invocation(&container(), None, None),
            Invocation::new("claude-agent-acp", &[])
        );
        let host = profile.invocation(&Placement::Host, None, None);
        assert_eq!(host.program, npx());
        assert_eq!(
            host.args,
            vec!["-y".to_string(), format!("{ACP_PACKAGE}@{ACP_VERSION}")]
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
        let config = HarnessConfig {
            placement: container(),
            working_dir: "/agent/workdir".into(),
            ..HarnessConfig::default()
        };
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
