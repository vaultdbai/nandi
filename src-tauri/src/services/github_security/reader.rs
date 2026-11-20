//! Reads what the GitHub security check needs, with the GitHub sign-in's
//! token: settings and metadata only (`GET` calls), never secret values or
//! repository contents. Every call goes to one base (`api.github.com` in a
//! build, a fake in tests) through `GitHubApi`, which only takes paths.
//!
//! At most [`MAX_CALLS`] calls per check and [`MAX_REPOS_PER_ORG`] public
//! repositories per org. A rate limit or a rejected sign-in stops reading:
//! what wasn't read is `Unread`, and its checks are unknown.

use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde_json::Value;

use crate::services::domain::github_security::{
    is_branch_name, is_github_name, BackupSnapshot, Fetched, OrgSnapshot, RepoSnapshot, Snapshot,
};
use crate::services::marketplace::github::GitHubApi;

/// The only address the token is sent to in a build
pub const API_URL: &str = "https://api.github.com";

/// The most API calls one check makes
pub const MAX_CALLS: usize = 150;

/// The most public repositories read per org (the to-do says "and N more")
pub const MAX_REPOS_PER_ORG: usize = 30;

pub struct Reader {
    api: GitHubApi,
    calls_left: usize,
}

impl Reader {
    pub fn new(base: &str, token: String) -> Self {
        Self {
            api: GitHubApi::new(base, token),
            calls_left: MAX_CALLS,
        }
    }

    /// Calls made so far
    pub fn calls(&self) -> usize {
        MAX_CALLS - self.calls_left
    }

    async fn get_with_headers(&mut self, path: &str) -> (Fetched, Option<HeaderMap>) {
        if self.calls_left == 0 {
            return (Fetched::Unread, None);
        }
        self.calls_left -= 1;
        let (status, body, headers) = match self
            .api
            .send_with_headers(reqwest::Method::GET, path, None)
            .await
        {
            Ok(response) => response,
            Err(e) => {
                log::warn!("GitHub security check: {e}");
                return (Fetched::Unread, None);
            }
        };
        let rate_limited = status == StatusCode::TOO_MANY_REQUESTS
            || (status == StatusCode::FORBIDDEN
                && headers
                    .get("x-ratelimit-remaining")
                    .and_then(|v| v.to_str().ok())
                    == Some("0"));
        let fetched = if status.is_success() {
            Fetched::Got(body)
        } else if rate_limited || status == StatusCode::UNAUTHORIZED {
            log::warn!("GitHub security check stopped: GitHub answered {status} for {path}");
            self.calls_left = 0;
            Fetched::Unread
        } else if matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND) {
            Fetched::Hidden
        } else {
            log::warn!("GitHub security check: GitHub answered {status} for {path}");
            Fetched::Unread
        };
        (fetched, Some(headers))
    }

    async fn get(&mut self, path: &str) -> Fetched {
        self.get_with_headers(path).await.0
    }

    /// Everything the check needs. `backup`: the Backup remote's owner and
    /// repository, when it is on github.com.
    pub async fn read(&mut self, backup: Option<(String, String)>) -> Snapshot {
        let (user, headers) = self.get_with_headers("/user").await;
        let scopes = headers
            .as_ref()
            .and_then(|h| h.get("x-oauth-scopes"))
            .and_then(|v| v.to_str().ok())
            .map(|scopes| {
                scopes
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            });
        let backup = match backup {
            Some((owner, repo)) if is_github_name(&owner) && is_github_name(&repo) => {
                let info = self.get(&format!("/repos/{owner}/{repo}")).await;
                Some(BackupSnapshot { owner, repo, info })
            }
            _ => None,
        };
        let orgs = self.owned_orgs().await;
        Snapshot {
            scopes,
            user,
            backup,
            orgs,
        }
    }

    async fn owned_orgs(&mut self) -> Fetched<Vec<OrgSnapshot>> {
        let memberships = match self
            .get("/user/memberships/orgs?state=active&per_page=100")
            .await
        {
            Fetched::Got(list) => list,
            Fetched::Hidden => return Fetched::Hidden,
            Fetched::Unread => return Fetched::Unread,
        };
        let logins: Vec<String> = memberships
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("admin"))
            .filter_map(|m| m["organization"]["login"].as_str())
            .filter(|login| is_github_name(login))
            .map(str::to_string)
            .collect();
        let mut orgs = Vec::with_capacity(logins.len());
        for login in logins {
            let info = self.get(&format!("/orgs/{login}")).await;
            let public = info
                .got()
                .and_then(|i| i.get("public_repos"))
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let (repos, listed) = self.public_repos(&login).await;
            orgs.push(OrgSnapshot {
                login,
                info,
                repos,
                unread_repos: public.saturating_sub(listed),
            });
        }
        Fetched::Got(orgs)
    }

    /// The org's public, not archived repositories, and how many GitHub
    /// listed (archived ones included)
    async fn public_repos(&mut self, org: &str) -> (Fetched<Vec<RepoSnapshot>>, usize) {
        let path =
            format!("/orgs/{org}/repos?type=public&sort=pushed&per_page={MAX_REPOS_PER_ORG}");
        let list = match self.get(&path).await {
            Fetched::Got(list) => list,
            Fetched::Hidden => return (Fetched::Hidden, 0),
            Fetched::Unread => return (Fetched::Unread, 0),
        };
        let listed = list.as_array().map_or(0, Vec::len);
        let public = list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|repo| repo.get("private").and_then(Value::as_bool) == Some(false))
            .filter(|repo| repo.get("archived").and_then(Value::as_bool) != Some(true))
            .take(MAX_REPOS_PER_ORG)
            .cloned()
            .collect::<Vec<_>>();
        let mut repos = Vec::with_capacity(public.len());
        for info in public {
            let name = info.get("name").and_then(Value::as_str).unwrap_or_default();
            let branch = info
                .get("default_branch")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (branch, rules) = if is_github_name(name) && is_branch_name(branch) {
                let base = format!("/repos/{org}/{name}");
                (
                    self.get(&format!("{base}/branches/{branch}")).await,
                    self.get(&format!("{base}/rules/branches/{branch}")).await,
                )
            } else {
                (Fetched::Unread, Fetched::Unread)
            };
            repos.push(RepoSnapshot {
                info,
                branch,
                rules,
            });
        }
        (Fetched::Got(repos), listed)
    }
}
