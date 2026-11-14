//! A custom harness: any executable that speaks ACP on stdio, registered
//! as `custom:<id>` with its command and arguments (`harness_preferences`).
//! It gets the run's credentials under the usual variable names
//! (`OPENAI_API_KEY`, `GEMINI_API_KEY`, `AWS_*`) and
//! nothing else of Nandi's.

use super::adapter::{
    common_credential_env, CliAdapter, CliProfile, HarnessConfig, Invocation, ModelRef,
};

pub struct Custom {
    /// What the user registered, e.g. `my-agent --acp`
    invocation: Invocation,
}

pub type CustomAdapter = CliAdapter<Custom>;

impl CustomAdapter {
    pub fn with_command(invocation: Invocation) -> Self {
        CliAdapter::new(Custom { invocation })
    }
}

impl CliProfile for Custom {
    fn id(&self) -> &'static str {
        "custom"
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        Vec::new()
    }

    fn invocation(&self, _model: Option<&str>) -> Invocation {
        self.invocation.clone()
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let credentials = &config.credentials;
        let mut env: Vec<(String, String)> = [
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::harness::adapter::{HarnessCredentials, Placement};

    #[test]
    fn runs_the_registered_command_with_standard_credential_names() {
        let profile = Custom {
            invocation: Invocation::new("my-agent", &["--acp"]),
        };
        assert_eq!(
            profile.invocation(None),
            Invocation::new("my-agent", &["--acp"])
        );
        let config = HarnessConfig {
            model: Some("m".into()),
            credentials: HarnessCredentials {
                openai_api_key: Some("o".into()),
                aws_region: Some("us-east-1".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::new("/w".into(), Placement::for_tests())
        };
        let env: std::collections::HashMap<_, _> = profile.env(&config).into_iter().collect();
        assert_eq!(env["OPENAI_API_KEY"], "o");
        assert_eq!(env["AWS_REGION"], "us-east-1");
        assert_eq!(env["NANDI_MODEL"], "m");
        assert!(!env.contains_key("GEMINI_API_KEY"));
    }
}
