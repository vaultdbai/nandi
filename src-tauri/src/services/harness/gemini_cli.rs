//! Gemini CLI over ACP: native, `gemini --acp` (`--experimental-acp`
//! before 0.30).
//!
//! Tested with Gemini CLI 0.64.0.
//!
//! Gemini wants an ACP `authenticate` before `session/new`: `gemini-api-key`
//! with `GEMINI_API_KEY`, `vertex-ai` on Vertex, else the host's Google
//! login (`oauth-personal`).

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::adapter::{
    common_credential_env, forward_env, CliAdapter, CliProfile, HarnessConfig, HarnessHealth,
    Invocation, ModelRef, Placement,
};
use super::detection::{self, DetectEnv};

/// The first release whose ACP flag is `--acp`
const STABLE_ACP_SINCE: semver::Version = semver::Version::new(0, 30, 0);

pub const FORWARDED: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_LOCATION",
    "GEMINI_MODEL",
];

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

    fn invocation(
        &self,
        placement: &Placement,
        host_version: Option<&semver::Version>,
        model: Option<&str>,
    ) -> Invocation {
        // The image pins a current release; on the host, follow its version
        let flag = match (placement, host_version) {
            (Placement::Host, Some(version)) if *version < STABLE_ACP_SINCE => "--experimental-acp",
            _ => "--acp",
        };
        let mut invocation = Invocation::new("gemini", &[flag]);
        if let Some(model) = model {
            invocation
                .args
                .extend(["--model".to_string(), model.to_string()]);
        }
        invocation
    }

    fn env(&self, config: &HarnessConfig) -> Vec<(String, String)> {
        let mut env: std::collections::BTreeMap<String, String> =
            forward_env(&self.env.vars, FORWARDED).into_iter().collect();
        if let Some(key) = &config.credentials.google_api_key {
            env.insert("GEMINI_API_KEY".into(), key.clone());
        }
        env.extend(common_credential_env(&config.credentials));
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env.into_iter().collect()
    }

    fn health(&self) -> BoxFuture<'_, HarnessHealth> {
        async move {
            let spec = detection::spec("gemini-cli").expect("built-in");
            detection::detect(spec, &self.env).await.health
        }
        .boxed()
    }

    fn auth_method(&self, env: &[(String, String)]) -> Option<String> {
        let value = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let method = if value("GOOGLE_GENAI_USE_VERTEXAI").is_some_and(|v| v == "true" || v == "1")
        {
            "vertex-ai"
        } else if value("GEMINI_API_KEY").is_some() || value("GOOGLE_API_KEY").is_some() {
            "gemini-api-key"
        } else {
            "oauth-personal"
        };
        Some(method.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::harness::adapter::HarnessCredentials;

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
    fn old_releases_use_the_experimental_flag() {
        let profile = gemini(&[]);
        let old = semver::Version::new(0, 12, 0);
        let new = semver::Version::new(0, 64, 0);
        assert_eq!(
            profile.invocation(&Placement::Host, Some(&old), None).args,
            vec!["--experimental-acp"]
        );
        assert_eq!(
            profile
                .invocation(&Placement::Host, Some(&new), Some("gemini-2.5-pro"))
                .args,
            vec!["--acp", "--model", "gemini-2.5-pro"]
        );
    }

    #[test]
    fn credentials_pick_the_auth_method() {
        let profile = gemini(&[]);
        let config = HarnessConfig {
            credentials: HarnessCredentials {
                google_api_key: Some("g".into()),
                ..HarnessCredentials::default()
            },
            ..HarnessConfig::default()
        };
        let env = profile.env(&config);
        assert_eq!(env, vec![("GEMINI_API_KEY".to_string(), "g".to_string())]);
        assert_eq!(profile.auth_method(&env).as_deref(), Some("gemini-api-key"));
        let vertex = gemini(&[("GOOGLE_GENAI_USE_VERTEXAI", "true")]);
        let env = vertex.env(&HarnessConfig::default());
        assert_eq!(vertex.auth_method(&env).as_deref(), Some("vertex-ai"));
        assert_eq!(profile.auth_method(&[]).as_deref(), Some("oauth-personal"));
    }
}
