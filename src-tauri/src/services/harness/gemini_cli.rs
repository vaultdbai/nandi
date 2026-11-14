//! Gemini CLI over ACP: native, `gemini --acp` (the harness image pins a
//! release that has it).
//!
//! Tested with Gemini CLI 0.64.0.
//!
//! Not supported in the container yet: every way Gemini CLI signs in
//! (`GEMINI_API_KEY`, Vertex AI's credentials, a Google login) would put
//! a credential in the container, and the egress proxy has no Gemini route
//! to sign for it on the host. `runtime_router::plan` refuses Gemini CLI
//! sessions with a reason until it does. Newer Gemini CLI releases read
//! `GOOGLE_GEMINI_BASE_URL`, which is the way in.

use super::adapter::{forward_env, CliAdapter, CliProfile, HarnessConfig, Invocation, ModelRef};
use super::detection::DetectEnv;

/// Why a Gemini CLI session can't start (see the module docs)
pub const UNSUPPORTED: &str = "Gemini CLI can't run in the container yet: it would need your      Google key or login inside it, and Nandi keeps credentials out of containers. Use Claude      Code or Codex, or the native runtime.";

/// What Gemini CLI reads from the environment: no keys
pub const FORWARDED: &[&str] = &["GEMINI_MODEL"];

pub struct GeminiCli {
    env: DetectEnv,
}

pub type GeminiCliAdapter = CliAdapter<GeminiCli>;

impl GeminiCliAdapter {
    pub fn with_env(env: DetectEnv) -> Self {
        CliAdapter::new(GeminiCli { env })
    }
}

impl CliProfile for GeminiCli {
    fn id(&self) -> &'static str {
        "gemini-cli"
    }

    fn supported_models(&self) -> Vec<ModelRef> {
        vec![
            ModelRef::new("gemini-2.5-pro", "Gemini 2.5 Pro", "google"),
            ModelRef::new("gemini-2.5-flash", "Gemini 2.5 Flash", "google"),
        ]
    }

    fn invocation(&self, model: Option<&str>) -> Invocation {
        let mut invocation = Invocation::new("gemini", &["--acp"]);
        if let Some(model) = model {
            invocation
                .args
                .extend(["--model".to_string(), model.to_string()]);
        }
        invocation
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let mut env = forward_env(&self.env.vars, FORWARDED);
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::harness::adapter::Placement;

    fn in_container() -> HarnessConfig {
        HarnessConfig::new("/w".into(), Placement::for_tests())
    }

    fn gemini(vars: &[(&str, &str)]) -> GeminiCli {
        GeminiCli {
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
    fn runs_acp_with_the_model() {
        let profile = gemini(&[]);
        assert_eq!(profile.invocation(None).args, vec!["--acp"]);
        assert_eq!(
            profile.invocation(Some("gemini-2.5-pro")).args,
            vec!["--acp", "--model", "gemini-2.5-pro"]
        );
    }

    #[test]
    fn no_key_reaches_the_container() {
        let profile = gemini(&[
            ("GEMINI_API_KEY", "g-key"),
            ("GEMINI_MODEL", "gemini-2.5-pro"),
        ]);
        let env = profile.env(&in_container());
        assert_eq!(
            env,
            vec![("GEMINI_MODEL".to_string(), "gemini-2.5-pro".to_string())]
        );
    }
}
