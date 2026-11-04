//! Codex over ACP, through Zed's `@zed-industries/codex-acp` (built on the
//! Codex crates; it reads Codex's own `~/.codex/config.toml` on the host).
//!
//! Tested with codex-acp 0.16.0 (Codex 0.163.0).
//!
//! Codex sessions want an ACP `authenticate` before `session/new` when they
//! run on an API key (`openai-api-key` / `codex-api-key`); a ChatGPT login
//! on the host needs none. The model goes in as a config override
//! (`-c model="…"`), the same flag the Codex CLI takes.

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::adapter::{
    common_credential_env, forward_env, CliAdapter, CliProfile, HarnessConfig, HarnessHealth,
    Invocation, ModelRef, Placement,
};
use super::claude_code::npx;
use super::detection::{self, DetectEnv};

pub const ACP_PACKAGE: &str = "@zed-industries/codex-acp";
pub const ACP_VERSION: &str = "0.16.0";
pub const ACP_BINARY: &str = "codex-acp";

/// What Codex reads from the environment
pub const FORWARDED: &[&str] = &[
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "OPENAI_BASE_URL",
    "AZURE_OPENAI_API_KEY",
    "AZURE_OPENAI_ENDPOINT",
];

pub struct Codex {
    env: DetectEnv,
}

pub type CodexAdapter = CliAdapter<Codex>;

impl CodexAdapter {
    pub fn with_env(env: DetectEnv) -> Self {
        CliAdapter::new(Codex { env })
    }
}

impl CliProfile for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        vec![
            ModelRef::new("gpt-5-codex", "GPT-5 Codex", "openai"),
            ModelRef::new("gpt-5", "GPT-5", "openai"),
        ]
    }

    fn invocation(
        &self,
        placement: &Placement,
        _host_version: Option<&semver::Version>,
        model: Option<&str>,
    ) -> Invocation {
        let mut invocation = match placement {
            Placement::Host if detection::find_executable(&[ACP_BINARY], &self.env).is_none() => {
                let package = format!("{ACP_PACKAGE}@{ACP_VERSION}");
                Invocation::new(npx(), &["-y", &package])
            }
            _ => Invocation::new(ACP_BINARY, &[]),
        };
        if let Some(model) = model {
            invocation.args.push("-c".to_string());
            invocation
                .args
                .push(format!("model={}", toml_string(model)));
        }
        invocation
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let mut env: std::collections::BTreeMap<String, String> =
            forward_env(&self.env.vars, FORWARDED).into_iter().collect();
        let credentials = &config.credentials;
        if let Some(key) = &credentials.openai_api_key {
            env.insert("OPENAI_API_KEY".into(), key.clone());
        }
        if let Some(key) = &credentials.azure_api_key {
            env.insert("AZURE_OPENAI_API_KEY".into(), key.clone());
        }
        if let Some(endpoint) = &credentials.azure_endpoint {
            env.insert("AZURE_OPENAI_ENDPOINT".into(), endpoint.clone());
        }
        env.extend(common_credential_env(credentials));
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env.into_iter().collect()
    }

    fn health(&self) -> BoxFuture<'_, HarnessHealth> {
        async move {
            let spec = detection::spec("codex").expect("built-in");
            detection::detect(spec, &self.env).await.health
        }
        .boxed()
    }

    fn auth_method(&self, env: &[(String, String)]) -> Option<String> {
        let has = |name: &str| env.iter().any(|(k, _)| k == name);
        if has("CODEX_API_KEY") {
            Some("codex-api-key".to_string())
        } else if has("OPENAI_API_KEY") {
            Some("openai-api-key".to_string())
        } else {
            None
        }
    }
}

/// A TOML string literal (for `-c key=value`)
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::harness::adapter::HarnessCredentials;

    fn codex(vars: &[(&str, &str)]) -> Codex {
        Codex {
            env: DetectEnv {
                home: std::env::temp_dir(),
                path: Some(std::ffi::OsString::new()),
                vars: vars
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
        }
    }

    #[test]
    fn the_model_is_a_config_override() {
        let invocation = codex(&[]).invocation(
            &Placement::Container {
                image: "i".into(),
                name: "n".into(),
                host_workdir: "/w".into(),
                memory_mb: None,
                cpu: None,
                network: None,
                folders: Vec::new(),
            },
            None,
            Some("gpt-5-codex"),
        );
        assert_eq!(invocation.program, "codex-acp");
        assert_eq!(invocation.args, vec!["-c", "model=\"gpt-5-codex\""]);
    }

    #[test]
    fn api_keys_pick_the_auth_method() {
        let profile = codex(&[("OPENAI_API_KEY", "from-env"), ("UNRELATED", "x")]);
        let env = profile.env(&HarnessConfig::default());
        assert_eq!(
            env,
            vec![("OPENAI_API_KEY".to_string(), "from-env".to_string())]
        );
        assert_eq!(profile.auth_method(&env).as_deref(), Some("openai-api-key"));

        let config = HarnessConfig {
            credentials: HarnessCredentials {
                openai_api_key: Some("from-nandi".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env: std::collections::HashMap<_, _> = profile.env(&config).into_iter().collect();
        assert_eq!(env["OPENAI_API_KEY"], "from-nandi");
        assert_eq!(codex(&[]).auth_method(&[]), None, "ChatGPT login");
    }

    #[test]
    fn azure_credentials() {
        let config = HarnessConfig {
            credentials: HarnessCredentials {
                azure_api_key: Some("az".into()),
                azure_endpoint: Some("https://x.openai.azure.com".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env: std::collections::HashMap<_, _> = codex(&[]).env(&config).into_iter().collect();
        assert_eq!(env["AZURE_OPENAI_API_KEY"], "az");
        assert_eq!(env["AZURE_OPENAI_ENDPOINT"], "https://x.openai.azure.com");
    }
}
