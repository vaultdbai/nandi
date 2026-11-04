//! The GitHub REST API calls the marketplace makes with the signed-in
//! user's token: community registry setup and publishing (branches, files,
//! tags, forks, pull requests). The base URL is injectable for tests.

use base64::Engine;
use serde_json::{json, Value};

const USER_AGENT: &str = concat!("nandi/", env!("CARGO_PKG_VERSION"));
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub struct GitHubApi {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// What we need to know about a repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    /// The signed-in user can push to it
    pub can_push: bool,
}

/// How a file write is committed.
#[derive(Debug, Clone, Copy)]
pub struct FileCommit<'a> {
    pub message: &'a str,
    /// The branch to commit to (default: the repo's default branch)
    pub branch: Option<&'a str>,
    /// The current blob's SHA, when updating an existing file
    pub sha: Option<&'a str>,
}

/// A file read through the Contents API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub content: String,
    /// Blob SHA, needed to update it
    pub sha: String,
}

impl GitHubApi {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            base: base.into(),
            token: token.into(),
        }
    }

    pub(crate) async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(reqwest::StatusCode, Value), String> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
        let status = response.status();
        let value = response.json().await.unwrap_or(Value::Null);
        Ok((status, value))
    }

    pub(crate) fn failed(what: &str, status: reqwest::StatusCode, body: &Value) -> String {
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        log::warn!("GitHub {what} failed: {status} {message}");
        match status.as_u16() {
            401 => "GitHub rejected your sign-in. Sign in again.".to_string(),
            403 => format!(
                "GitHub refused to {what} ({message}). Your sign-in may lack the 'repo' scope."
            ),
            _ => format!("Couldn't {what}: GitHub returned {status} {message}"),
        }
    }

    /// The signed-in user's login.
    pub async fn login(&self) -> Result<String, String> {
        let (status, body) = self.send(reqwest::Method::GET, "/user", None).await?;
        if !status.is_success() {
            return Err(Self::failed("read your GitHub account", status, &body));
        }
        body.get("login")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "Unexpected response from GitHub".to_string())
    }

    pub async fn repo(&self, owner: &str, repo: &str) -> Result<Option<RepoInfo>, String> {
        let (status, body) = self
            .send(
                reqwest::Method::GET,
                &format!("/repos/{owner}/{repo}"),
                None,
            )
            .await?;
        match status.as_u16() {
            200 => Ok(Some(RepoInfo {
                owner: body["owner"]["login"].as_str().unwrap_or(owner).to_string(),
                name: body["name"].as_str().unwrap_or(repo).to_string(),
                default_branch: body["default_branch"]
                    .as_str()
                    .unwrap_or("main")
                    .to_string(),
                can_push: body["permissions"]["push"].as_bool().unwrap_or(false),
            })),
            404 => Ok(None),
            _ => Err(Self::failed("look up the repo", status, &body)),
        }
    }

    /// Forks a repo into the signed-in account (or finds the existing
    /// fork) and waits until it can be used.
    pub async fn fork(&self, owner: &str, repo: &str) -> Result<RepoInfo, String> {
        let (status, body) = self
            .send(
                reqwest::Method::POST,
                &format!("/repos/{owner}/{repo}/forks"),
                Some(json!({ "default_branch_only": true })),
            )
            .await?;
        if !status.is_success() {
            return Err(Self::failed("fork the registry", status, &body));
        }
        let fork_owner = body["owner"]["login"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let fork_name = body["name"].as_str().unwrap_or(repo).to_string();
        // GitHub creates forks asynchronously
        for _ in 0..20 {
            if let Some(info) = self.repo(&fork_owner, &fork_name).await? {
                if self
                    .branch_sha(&info.owner, &info.name, &info.default_branch)
                    .await
                    .is_ok()
                {
                    return Ok(info);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Err("GitHub is still creating your fork of the registry; try again in a minute".to_string())
    }

    /// The SHA a ref points at (`heads/main`, `tags/v1.0.0`), if it exists.
    pub async fn ref_sha(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
    ) -> Result<Option<String>, String> {
        let (status, body) = self
            .send(
                reqwest::Method::GET,
                &format!("/repos/{owner}/{repo}/git/ref/{git_ref}"),
                None,
            )
            .await?;
        match status.as_u16() {
            200 => Ok(body["object"]["sha"].as_str().map(str::to_string)),
            404 => Ok(None),
            _ => Err(Self::failed("read a ref", status, &body)),
        }
    }

    pub async fn branch_sha(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<String, String> {
        self.ref_sha(owner, repo, &format!("heads/{branch}"))
            .await?
            .ok_or_else(|| format!("{owner}/{repo} has no branch '{branch}'"))
    }

    /// Creates `refs/<git_ref>` at `sha`. An existing ref at the same SHA is
    /// fine; one elsewhere is an error.
    pub async fn create_ref(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
        sha: &str,
    ) -> Result<(), String> {
        if let Some(existing) = self.ref_sha(owner, repo, git_ref).await? {
            if existing.eq_ignore_ascii_case(sha) {
                return Ok(());
            }
            return Err(format!(
                "{owner}/{repo} already has '{git_ref}' at another commit; choose a new version"
            ));
        }
        let (status, body) = self
            .send(
                reqwest::Method::POST,
                &format!("/repos/{owner}/{repo}/git/refs"),
                Some(json!({ "ref": format!("refs/{git_ref}"), "sha": sha })),
            )
            .await?;
        if !status.is_success() {
            return Err(Self::failed(&format!("create {git_ref}"), status, &body));
        }
        Ok(())
    }

    pub async fn file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        git_ref: Option<&str>,
    ) -> Result<Option<RemoteFile>, String> {
        let query = git_ref.map(|r| format!("?ref={r}")).unwrap_or_default();
        let (status, body) = self
            .send(
                reqwest::Method::GET,
                &format!("/repos/{owner}/{repo}/contents/{path}{query}"),
                None,
            )
            .await?;
        match status.as_u16() {
            200 => {
                let encoded: String = body["content"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                let bytes = B64
                    .decode(encoded)
                    .map_err(|_| format!("{path} on GitHub isn't valid base64"))?;
                Ok(Some(RemoteFile {
                    content: String::from_utf8_lossy(&bytes).into_owned(),
                    sha: body["sha"].as_str().unwrap_or_default().to_string(),
                }))
            }
            404 => Ok(None),
            _ => Err(Self::failed(&format!("read {path}"), status, &body)),
        }
    }

    /// Creates or updates a file (one commit).
    pub async fn put_file(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        content: &str,
        commit: FileCommit<'_>,
    ) -> Result<(), String> {
        let mut body = json!({ "message": commit.message, "content": B64.encode(content) });
        if let Some(branch) = commit.branch {
            body["branch"] = branch.into();
        }
        if let Some(sha) = commit.sha {
            body["sha"] = sha.into();
        }
        let (status, response) = self
            .send(
                reqwest::Method::PUT,
                &format!("/repos/{owner}/{repo}/contents/{path}"),
                Some(body),
            )
            .await?;
        if !status.is_success() {
            return Err(Self::failed(&format!("write {path}"), status, &response));
        }
        Ok(())
    }

    /// Opens a pull request; returns its number and web URL.
    pub async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<(u64, String), String> {
        let (status, response) = self
            .send(
                reqwest::Method::POST,
                &format!("/repos/{owner}/{repo}/pulls"),
                Some(json!({ "title": title, "body": body, "head": head, "base": base })),
            )
            .await?;
        if !status.is_success() {
            return Err(Self::failed("open the pull request", status, &response));
        }
        Ok((
            response["number"].as_u64().unwrap_or_default(),
            response["html_url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ))
    }

    pub async fn merge_pr(&self, owner: &str, repo: &str, number: u64) -> Result<(), String> {
        let (status, response) = self
            .send(
                reqwest::Method::PUT,
                &format!("/repos/{owner}/{repo}/pulls/{number}/merge"),
                Some(json!({ "merge_method": "squash" })),
            )
            .await?;
        if !status.is_success() {
            return Err(Self::failed("merge the pull request", status, &response));
        }
        Ok(())
    }
}
