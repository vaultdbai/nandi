//! The GitHub sign-in: one OAuth token (device flow, RFC 8628) for
//! everything that talks to github.com — the GitHub connector, Backup,
//! team and capability registries, marketplace sources, and publishing.
//!
//! The browser does the signing in: `start` returns a user code and
//! `https://github.com/login/device`, and `poll` stores the token once the
//! user approves. The token lives in the OS keyring (`nandi.github.token`)
//! and is only ever sent to `github.com` / `api.github.com`; the account's
//! login (not a secret) is kept in `config.json` for display.
//!
//! The OAuth App's client ID is public and baked in; a build can point at
//! another app with `NANDI_GITHUB_CLIENT_ID` (it needs "Enable Device Flow").

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::connectors::oauth::{self, OAuthProvider};
use crate::services::connectors::DeviceFlowState;
use crate::services::marketplace::github::GitHubApi;
use crate::services::secrets::Secrets;
use crate::services::AppState;
use crate::utils::nandi_config;

/// The sign-in's keyring entry. A marketplace registry whose `auth` is
/// this entry reads with the signed-in account.
pub const TOKEN_ENTRY: &str = "nandi.github.token";

/// The Nandi OAuth App
const DEFAULT_CLIENT_ID: &str = "Ov23li9pRWYWwxbtygys";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const DEVICE_URL: &str = "https://github.com/login/device/code";
const API_URL: &str = "https://api.github.com";
/// Read and write the user's repositories (Backup pushes, team registries,
/// pull requests through the connector)
const SCOPES: &[&str] = &["repo"];

/// Whether this Nandi is signed in to GitHub, and as whom
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GithubSignIn {
    pub signed_in: bool,
    /// The GitHub login, once known
    pub login: Option<String>,
}

fn client_id() -> String {
    option_env!("NANDI_GITHUB_CLIENT_ID")
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(DEFAULT_CLIENT_ID)
        .to_string()
}

/// GitHub's device flow, for the shared OAuth engine
pub fn provider() -> OAuthProvider {
    OAuthProvider {
        id: "github".into(),
        name: "GitHub".into(),
        description: "Sign in with GitHub once for every github.com repository Nandi uses.".into(),
        coming_soon: false,
        authorize_url: None,
        token_url: TOKEN_URL.into(),
        device_url: Some(DEVICE_URL.into()),
        client_id: Some(client_id()),
        client_secret: None,
        scopes: SCOPES.iter().map(|s| (*s).to_string()).collect(),
        auth_params: Vec::new(),
    }
}

/// The signed-in token, if any. Never leaves Rust.
pub fn token(secrets: &Secrets) -> Option<String> {
    oauth::stored_access_token(TOKEN_ENTRY, secrets)
}

/// `https://github.com/...` (the only addresses the token is sent to over
/// git; SSH remotes keep using SSH keys)
pub fn is_github_url(url: &str) -> bool {
    reqwest::Url::parse(url.trim()).is_ok_and(|url| {
        url.scheme() == "https" && matches!(url.host_str(), Some("github.com" | "www.github.com"))
    })
}

/// The signed-in token when `url` is a github.com address
pub fn token_for_url(secrets: &Secrets, url: &str) -> Option<String> {
    if is_github_url(url) {
        token(secrets)
    } else {
        None
    }
}

pub fn status(state: &AppState) -> GithubSignIn {
    let signed_in = token(state.secrets()).is_some();
    let login = signed_in
        .then(|| {
            nandi_config::load_from(state.nandi_dir())
                .ok()
                .and_then(|config| config.github_login)
        })
        .flatten();
    GithubSignIn { signed_in, login }
}

/// Starts (or restarts) the sign-in: show `userCode`, open `verificationUri`
pub async fn start(state: &AppState) -> Result<DeviceFlowState, String> {
    state
        .github_sign_in_flows()
        .start(TOKEN_ENTRY, provider(), state.secrets().clone())
        .await
}

/// Asks GitHub once; once the user approved, the token is stored and the
/// account's login looked up.
pub async fn poll(state: &AppState) -> Result<GithubSignIn, String> {
    let authorized = state
        .github_sign_in_flows()
        .poll(TOKEN_ENTRY, state.secrets())
        .await?;
    if authorized {
        remember_login(state).await;
    }
    Ok(status(state))
}

/// Forgets the token here (GitHub still lists the app under Settings ›
/// Applications, where the user can revoke it).
pub fn sign_out(state: &AppState) -> Result<GithubSignIn, String> {
    state.secrets().delete(TOKEN_ENTRY)?;
    nandi_config::update_in(state.nandi_dir(), |config| config.github_login = None)?;
    log::info!("Signed out of GitHub");
    Ok(status(state))
}

/// Best effort: the sign-in works without it
async fn remember_login(state: &AppState) {
    let Some(token) = token(state.secrets()) else {
        return;
    };
    match GitHubApi::new(API_URL, token).login().await {
        Ok(login) => {
            if let Err(e) = nandi_config::update_in(state.nandi_dir(), |config| {
                config.github_login = Some(login);
            }) {
                log::warn!("Couldn't save the GitHub login: {e}");
            }
        }
        Err(e) => log::warn!("Couldn't look up the GitHub login: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_github_addresses_get_the_token() {
        assert!(is_github_url("https://github.com/acme/agents.git"));
        assert!(is_github_url("https://www.github.com/acme/agents"));
        assert!(is_github_url(" https://github.com/acme/agents "));
        assert!(!is_github_url("http://github.com/acme/agents"));
        assert!(!is_github_url("git@github.com:acme/agents.git"));
        assert!(!is_github_url("ssh://git@github.com/acme/agents.git"));
        assert!(!is_github_url(
            "https://github.com.evil.example/acme/agents"
        ));
        assert!(!is_github_url("https://gitlab.com/acme/agents"));
        assert!(!is_github_url("/tmp/agents"));
    }

    #[test]
    fn token_for_url_needs_a_sign_in_and_a_github_address() {
        let secrets = Secrets::in_memory();
        assert_eq!(token_for_url(&secrets, "https://github.com/a/b"), None);
        secrets
            .set(TOKEN_ENTRY, r#"{"access_token":"gho_test"}"#)
            .unwrap();
        assert_eq!(
            token_for_url(&secrets, "https://github.com/a/b").as_deref(),
            Some("gho_test")
        );
        assert_eq!(token_for_url(&secrets, "https://gitlab.com/a/b"), None);
    }

    #[test]
    fn the_provider_uses_the_device_flow() {
        let provider = provider();
        assert_eq!(provider.flow_kind(), "device");
        assert!(provider.client_id.is_some_and(|id| !id.is_empty()));
        assert!(provider.client_secret.is_none());
    }
}
