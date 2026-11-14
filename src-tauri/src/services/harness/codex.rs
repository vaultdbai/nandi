//! Codex over ACP, through Zed's `@zed-industries/codex-acp` (built on the
//! Codex crates).
//!
//! Tested with codex-acp 0.16.0 (Codex 0.163.0).
//!
//! Codex reaches OpenAI only through the egress proxy: the invocation
//! defines a `nandi` model provider at
//! `http://nandi-egress:11435/openai/v1` that reads its key from
//! `OPENAI_API_KEY`, and that variable holds the session's proxy token.
//! The host adds the real key (`egress::model_proxy`): Nandi's OpenAI key,
//! else `OPENAI_API_KEY` / `CODEX_API_KEY` from Nandi's environment
//! ([`host_upstream`]). A ChatGPT login (`~/.codex/auth.json`) and Azure
//! OpenAI don't work in the container: their credentials would have to go
//! in.
//!
//! Codex wants an ACP `authenticate` before `session/new` on an API key
//! (`openai-api-key`). The model goes in as a config override
//! (`-c model="…"`), the same flag the Codex CLI takes.

use super::adapter::{proxy_token, CliAdapter, CliProfile, HarnessConfig, Invocation, ModelRef};
use super::detection::DetectEnv;
use crate::services::egress::model_proxy::{ModelApi, ModelUpstream, Secret, UpstreamAuth};

pub const ACP_BINARY: &str = "codex-acp";

/// The model provider the invocation defines, served by the proxy
const PROVIDER: &str = "nandi";

/// The OpenAI keys Codex reads from Nandi's environment, in order. They
/// sign the session's calls on the host; none goes into the container
const HOST_KEYS: &[&str] = &["CODEX_API_KEY", "OPENAI_API_KEY"];

/// Where a Codex session's model calls go when the host signs them with
/// an OpenAI key from Nandi's environment
pub fn host_upstream(env: &DetectEnv) -> Option<ModelUpstream> {
    let key = HOST_KEYS.iter().find_map(|name| env.var(name))?.to_string();
    Some(ModelUpstream::openai(UpstreamAuth::Bearer(Secret::new(
        key,
    ))))
}

/// Codex reads nothing from the host: its key is the host's to add
pub struct Codex;

pub type CodexAdapter = CliAdapter<Codex>;

impl CodexAdapter {
    pub fn create() -> Self {
        CliAdapter::new(Codex)
    }
}

impl CliProfile for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        vec![
            ModelRef::new("gpt-5-codex", "GPT-5 Codex", "openai"),
            ModelRef::new("gpt-5", "GPT-5", "openai"),
        ]
    }

    fn invocation(&self, model: Option<&str>) -> Invocation {
        let base_url = format!(
            "{}/v1",
            crate::services::egress::model_proxy_url(ModelApi::OpenAi)
        );
        let mut overrides = vec![
            format!("model_provider={}", toml_string(PROVIDER)),
            format!("model_providers.{PROVIDER}.name={}", toml_string("Nandi")),
            format!(
                "model_providers.{PROVIDER}.base_url={}",
                toml_string(&base_url)
            ),
            format!(
                "model_providers.{PROVIDER}.env_key={}",
                toml_string("OPENAI_API_KEY")
            ),
            format!(
                "model_providers.{PROVIDER}.wire_api={}",
                toml_string("responses")
            ),
        ];
        if let Some(model) = model {
            overrides.push(format!("model={}", toml_string(model)));
        }
        let mut invocation = Invocation::new(ACP_BINARY, &[]);
        for value in overrides {
            invocation.args.extend(["-c".to_string(), value]);
        }
        invocation
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = Vec::new();
        if let (Some(_), Some(token)) = (config.model_route, proxy_token(config)) {
            env.push(("OPENAI_API_KEY".to_string(), token.to_string()));
        }
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env
    }

    fn auth_method(&self, env: &[(String, String)]) -> Option<String> {
        env.iter()
            .any(|(k, _)| k == "OPENAI_API_KEY")
            .then(|| "openai-api-key".to_string())
    }
}

/// A TOML string literal (for `-c key=value`)
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::harness::adapter::{ModelRoute, Placement};

    const PROXY_TOKEN: &str = "proxy-token-for-codex";

    fn host(vars: &[(&str, &str)]) -> DetectEnv {
        DetectEnv {
            home: std::env::temp_dir(),
            path: Some(std::ffi::OsString::new()),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn routed() -> HarnessConfig {
        HarnessConfig {
            model_route: Some(ModelRoute::OpenAi),
            env: [(
                crate::services::egress::PROXY_TOKEN_ENV.to_string(),
                PROXY_TOKEN.to_string(),
            )]
            .into(),
            ..HarnessConfig::new("/w".into(), Placement::for_tests())
        }
    }

    #[test]
    fn codex_reaches_openai_only_through_the_proxy() {
        let invocation = Codex.invocation(Some("gpt-5-codex"));
        assert_eq!(invocation.program, "codex-acp");
        let args = invocation.args.join(" ");
        assert!(args.contains("-c model_provider=\"nandi\""), "{args}");
        assert!(
            args.contains(
                "-c model_providers.nandi.base_url=\"http://nandi-egress:11435/openai/v1\""
            ),
            "{args}"
        );
        assert!(args.contains("-c model_providers.nandi.env_key=\"OPENAI_API_KEY\""));
        assert!(args.ends_with("-c model=\"gpt-5-codex\""), "{args}");
    }

    #[test]
    fn the_container_gets_the_proxy_token_never_the_key() {
        let profile = Codex;
        assert_eq!(
            host_upstream(&host(&[
                ("OPENAI_API_KEY", "sk-host-openai"),
                ("UNRELATED", "x")
            ])),
            Some(ModelUpstream::openai(UpstreamAuth::Bearer(Secret::new(
                "sk-host-openai"
            ))))
        );
        let env = profile.env(&routed());
        assert!(env.contains(&("OPENAI_API_KEY".to_string(), PROXY_TOKEN.to_string())));
        assert!(!env.iter().any(|(_, v)| v.contains("sk-host-openai")));
        assert_eq!(profile.auth_method(&env).as_deref(), Some("openai-api-key"));
    }

    #[test]
    fn without_a_route_there_is_nothing_to_sign_in_with() {
        let profile = Codex;
        let plain = HarnessConfig::new("/w".into(), Placement::for_tests());
        let env = profile.env(&plain);
        assert!(env.is_empty());
        assert_eq!(profile.auth_method(&env), None);
        assert_eq!(host_upstream(&host(&[])), None);
    }
}
