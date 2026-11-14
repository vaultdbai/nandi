//! A custom harness: any executable that speaks ACP on stdio, registered
//! as `custom:<id>` with its command and arguments (`harness_preferences`).
//! It gets no provider credential: credentials never go into the
//! container. It gets the egress proxy's variables (`egress::proxy_env`),
//! so it can call `NANDI_LLM_PROXY` with `NANDI_LLM_PROXY_TOKEN` and the host
//! adds the key, as Nandi's own runtime does, and the model in
//! `NANDI_MODEL`.

use super::adapter::{CliAdapter, CliProfile, HarnessConfig, Invocation, ModelRef};

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
        let mut env = Vec::new();
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
    use crate::services::harness::adapter::Placement;

    #[test]
    fn runs_the_registered_command_with_the_model_and_no_credentials() {
        let profile = Custom {
            invocation: Invocation::new("my-agent", &["--acp"]),
        };
        assert_eq!(
            profile.invocation(None),
            Invocation::new("my-agent", &["--acp"])
        );
        let config = HarnessConfig {
            model: Some("m".into()),
            env: [("NANDI_LLM_PROXY_TOKEN".to_string(), "tok".to_string())].into(),
            ..HarnessConfig::new("/w".into(), Placement::for_tests())
        };
        let env: std::collections::HashMap<_, _> = profile.env(&config).into_iter().collect();
        assert_eq!(env["NANDI_MODEL"], "m");
        assert_eq!(env["NANDI_LLM_PROXY_TOKEN"], "tok");
        assert_eq!(env.len(), 2);
    }
}
