//! A custom harness: any executable that speaks ACP on stdio, registered
//! as `custom:<id>` with its command and arguments (`harness_preferences`).
//! It gets the run's credentials under the usual variable names
//! (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `AWS_*`) and
//! nothing else of Nandi's.

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::adapter::{
    common_credential_env, CliAdapter, CliProfile, HarnessConfig, HarnessHealth, Invocation,
    ModelRef, Placement,
};
use super::detection::{self, DetectEnv};

pub struct Custom {
    /// What the user registered, e.g. `my-agent --acp`
    invocation: Invocation,
    env: DetectEnv,
}

pub type CustomAdapter = CliAdapter<Custom>;

impl CustomAdapter {
    pub fn with_command(invocation: Invocation, env: DetectEnv) -> Self {
        CliAdapter::new(Custom { invocation, env })
    }
}

impl CliProfile for Custom {
    fn id(&self) -> &'static str {
        "custom"
    }

    fn display_name(&self) -> &'static str {
        "Custom harness"
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        Vec::new()
    }

    fn invocation(
        &self,
        _placement: &Placement,
        _host_version: Option<&semver::Version>,
        _model: Option<&str>,
    ) -> Invocation {
        self.invocation.clone()
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let credentials = &config.credentials;
        let mut env: Vec<(String, String)> = [
            ("ANTHROPIC_API_KEY", &credentials.anthropic_api_key),
            ("OPENAI_API_KEY", &credentials.openai_api_key),
            ("GEMINI_API_KEY", &credentials.google_api_key),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.clone().map(|v| (name.to_string(), v)))
        .collect();
        env.extend(common_credential_env(credentials));
        if let Some(model) = &config.model {
            env.push(("NANDI_MODEL".to_string(), model.clone()));
        }
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env
    }

    fn health(&self) -> BoxFuture<'_, HarnessHealth> {
        async move {
            let program = self.invocation.program.as_str();
            let path = detection::find_executable(&[program], &self.env)
                .or_else(|| Some(std::path::PathBuf::from(program)).filter(|p| p.is_file()));
            let version = match &path {
                Some(path) => detection::version_of(path).await.ok().flatten(),
                None => None,
            };
            HarnessHealth {
                installed: path.is_some(),
                version,
                // Unknown: the Test connection check tells
                host_authenticated: path.is_some(),
                container_authenticated: path.is_some(),
                configured_provider: None,
                auth: None,
                last_check: crate::db::ts(crate::db::now_ms()),
                error: path.is_none().then(|| format!("{program} was not found")),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::harness::adapter::HarnessCredentials;

    #[test]
    fn runs_the_registered_command_with_standard_credential_names() {
        let profile = Custom {
            invocation: Invocation::new("my-agent", &["--acp"]),
            env: DetectEnv::default(),
        };
        assert_eq!(
            profile.invocation(&Placement::Host, None, None),
            Invocation::new("my-agent", &["--acp"])
        );
        let config = HarnessConfig {
            model: Some("m".into()),
            credentials: HarnessCredentials {
                anthropic_api_key: Some("a".into()),
                aws_region: Some("us-east-1".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env: std::collections::HashMap<_, _> = profile.env(&config).into_iter().collect();
        assert_eq!(env["ANTHROPIC_API_KEY"], "a");
        assert_eq!(env["AWS_REGION"], "us-east-1");
        assert_eq!(env["NANDI_MODEL"], "m");
        assert!(!env.contains_key("OPENAI_API_KEY"));
    }

    #[tokio::test]
    async fn a_missing_command_is_not_installed() {
        let profile = Custom {
            invocation: Invocation::new("definitely-not-a-real-harness", &[]),
            env: DetectEnv {
                path: Some(std::ffi::OsString::new()),
                ..DetectEnv::default()
            },
        };
        let health = profile.health().await;
        assert!(!health.installed);
        assert!(health.error.is_some());
    }
}
