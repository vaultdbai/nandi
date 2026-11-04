//! The `github` connector: pull requests and files over the GitHub REST API.
//! Its token is the one the user pasted (a fine-grained personal access
//! token, in the OS keyring as `connectors.github.token`, or the
//! declaration's `secret_refs.token`), else the GitHub CLI's own sign-in
//! (`gh auth token`, asked for when needed and never stored). It never
//! leaves Rust: the calls run on the host, so an agent's container never
//! sees it.
//!
//! - `get_pull_request`: metadata and each changed file's patch
//! - `get_file`: a file's text at a ref (guidelines, surrounding code)
//! - `post_review`: a review with inline comments (sensitive: the user
//!   approves the exact comments first)

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::{Connector, ConnectorDecl, ToolDef};

/// Keyring entry for the GitHub token
pub const KEY_ENTRY: &str = "connectors.github.token";
const DEFAULT_API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
/// GitHub lists at most 3000 files per PR; past this the review says so
const MAX_FILES: usize = 300;
const FILES_PER_PAGE: usize = 100;
/// One file's patch beyond this is cut, so one huge file can't crowd out
/// the rest of the diff
const MAX_PATCH_CHARS: usize = 12_000;
/// What the model reads of one tool result (`history`). The whole answer
/// fits in it: the patches get what the rest leaves, and later files come
/// without their patch and say so, rather than the answer being cut mid-way
/// (which dropped the end of the file list)
const RESULT_LIMIT_CHARS: usize = crate::services::history::MAX_TOOL_RESULT_CHARS;
/// Room for the cut and left-out markers the patches gain
const RESULT_MARGIN_CHARS: usize = 2_000;
/// Files larger than this are refused rather than flooding the model
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_COMMENTS: usize = 50;

/// How long a `gh auth token` answer is reused: setup hints ask often
const CLI_CACHE: Duration = Duration::from_secs(60);
/// `gh` answers at once when signed in; anything slower is a hung prompt
const CLI_TIMEOUT: Duration = Duration::from_secs(5);

/// The GitHub CLI's token, when `gh` is installed and signed in to
/// github.com. Asked for on demand and kept only in memory.
pub fn cli_token() -> Option<String> {
    #[cfg(test)]
    {
        TEST_CLI_TOKEN.with(|cell| cell.borrow().clone())
    }
    #[cfg(not(test))]
    {
        static CACHE: std::sync::Mutex<Option<(Instant, Option<String>)>> =
            std::sync::Mutex::new(None);
        let mut cached = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, token)) = cached.as_ref() {
            if at.elapsed() < CLI_CACHE {
                return token.clone();
            }
        }
        let token = read_cli_token();
        *cached = Some((Instant::now(), token.clone()));
        token
    }
}

#[cfg(test)]
thread_local! {
    static TEST_CLI_TOKEN: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Tests: what `cli_token` returns on this thread (never the real `gh`)
#[cfg(test)]
pub fn set_cli_token_for_tests(token: Option<&str>) {
    TEST_CLI_TOKEN.with(|cell| *cell.borrow_mut() = token.map(str::to_string));
}

#[cfg_attr(test, allow(dead_code))]
fn read_cli_token() -> Option<String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new("gh");
    command
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for gh.exe
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > CLI_TIMEOUT => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    parse_cli_token(&out)
}

/// `gh auth token`'s output: one token on one line
fn parse_cli_token(out: &str) -> Option<String> {
    let token = out.trim();
    (!token.is_empty() && !token.contains(char::is_whitespace)).then(|| token.to_string())
}

/// Where this declaration's token lives in the keyring.
pub fn key_entry(decl: &ConnectorDecl) -> String {
    decl.secret_refs
        .get("token")
        .map_or_else(|| KEY_ENTRY.to_string(), |r| r.as_str().to_string())
}

/// No `Debug`: holds the token.
pub struct GithubConnector {
    token: String,
    /// `https://api.github.com`, or a GitHub Enterprise / test server
    api: String,
}

/// A pull request named by URL (`https://github.com/o/r/pull/7`, with any
/// suffix such as `/files`) or shorthand (`o/r#7`)
#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRef {
    owner: String,
    repo: String,
    number: u64,
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

impl PullRef {
    fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        let bad = || {
            format!(
                "'{input}' isn't a pull request: use https://github.com/<owner>/<repo>/pull/<number> or <owner>/<repo>#<number>"
            )
        };
        let (owner, repo, number) = if let Some((repo_part, number)) = input.split_once('#') {
            let (owner, repo) = repo_part.split_once('/').ok_or_else(bad)?;
            (owner, repo, number)
        } else {
            let path = input
                .strip_prefix("https://")
                .or_else(|| input.strip_prefix("http://"))
                .ok_or_else(bad)?;
            let mut parts = path.split('/');
            let _host = parts.next();
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(owner), Some(repo), Some("pull"), Some(number)) => (owner, repo, number),
                _ => return Err(bad()),
            }
        };
        let number = number.parse::<u64>().map_err(|_| bad())?;
        if !valid_name(owner) || !valid_name(repo) || number == 0 {
            return Err(bad());
        }
        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        })
    }

    fn path(&self) -> String {
        format!("repos/{}/{}/pulls/{}", self.owner, self.repo, self.number)
    }
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("nandi-app/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client")
    })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Missing string argument '{key}'"))
}

/// Cuts `text` at `max` characters (on a char boundary) and says so
fn truncated(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((at, _)) => (
            format!("{}\n[... cut at {max} characters]", &text[..at]),
            true,
        ),
        None => (text.to_string(), false),
    }
}

impl GithubConnector {
    /// `api` overrides the API base URL (GitHub Enterprise, tests).
    pub fn new(token: String, api: Option<String>) -> Self {
        Self {
            token,
            api: api
                .unwrap_or_else(|| DEFAULT_API.to_string())
                .trim_end_matches('/')
                .to_string(),
        }
    }

    pub fn tool_defs() -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "get_pull_request".into(),
                description: "Get a pull request: title, description, author, base and head, and every changed file with its patch (unified diff). Patches of very large files are cut.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "https://github.com/<owner>/<repo>/pull/<number> or <owner>/<repo>#<number>"},
                    },
                    "required": ["url"],
                }),
                sensitive: false,
            },
            ToolDef {
                name: "get_file".into(),
                description: "Read a text file from a repository at a branch, tag or commit, e.g. the base branch's CONTRIBUTING.md, or code around a change.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "repo": {"type": "string", "description": "<owner>/<repo>"},
                        "path": {"type": "string"},
                        "ref": {"type": "string", "description": "branch, tag or commit SHA; omitted: the default branch"},
                    },
                    "required": ["repo", "path"],
                }),
                sensitive: false,
            },
            ToolDef {
                name: "post_review".into(),
                description: "Post a review on a pull request: a summary and inline comments on lines of the new code. Waits for the user's approval.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "the pull request"},
                        "body": {"type": "string", "description": "the review summary (Markdown)"},
                        "event": {"type": "string", "enum": ["COMMENT", "REQUEST_CHANGES", "APPROVE"], "description": "default COMMENT"},
                        "comments": {
                            "type": "array",
                            "maxItems": MAX_COMMENTS,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "line": {"type": "integer", "minimum": 1, "description": "line number in the new version of the file, inside a changed hunk"},
                                    "body": {"type": "string"},
                                },
                                "required": ["path", "line", "body"],
                            },
                        },
                    },
                    "required": ["url", "body"],
                }),
                sensitive: true,
            },
        ]
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        http_client()
            .request(method, format!("{}/{path}", self.api))
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
    }

    /// Sends and checks the status, in words that never include the token
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response, String> {
        let response = request
            .send()
            .await
            .map_err(|e| format!("Could not reach GitHub: {}", e.without_url()))?;
        let status = response.status();
        match status.as_u16() {
            200..=299 => Ok(response),
            401 => Err("GitHub rejected the token. Replace it in Library › GitHub › Setup.".to_string()),
            403 | 404 => Err(format!(
                "GitHub says {what} doesn't exist or the token can't see it ({status}). For a private repository the token needs access to it, with Contents and Pull requests permissions."
            )),
            422 => {
                let detail = response.text().await.unwrap_or_default();
                let (detail, _) = truncated(&detail, 500);
                Err(format!("GitHub refused it ({status}): {detail}"))
            }
            _ => Err(format!("GitHub failed ({status}) for {what}")),
        }
    }

    async fn get_json(&self, path: &str, what: &str) -> Result<Value, String> {
        self.send(self.request(reqwest::Method::GET, path), what)
            .await?
            .json()
            .await
            .map_err(|e| format!("Unexpected response from GitHub: {e}"))
    }

    async fn get_pull_request(&self, args: &Value) -> Result<Value, String> {
        let pull = PullRef::parse(str_arg(args, "url")?)?;
        let what = format!("{}/{}#{}", pull.owner, pull.repo, pull.number);
        let pr = self.get_json(&pull.path(), &what).await?;

        let mut files = Vec::new();
        let mut page = 1;
        while files.len() < MAX_FILES {
            let batch = self
                .get_json(
                    &format!(
                        "{}/files?per_page={FILES_PER_PAGE}&page={page}",
                        pull.path()
                    ),
                    &what,
                )
                .await?;
            let batch = batch.as_array().cloned().unwrap_or_default();
            let last = batch.len() < FILES_PER_PAGE;
            files.extend(batch);
            if last {
                break;
            }
            page += 1;
        }
        let total_files = pr["changed_files"].as_u64().unwrap_or(files.len() as u64);
        files.truncate(MAX_FILES);
        // Every file, first without its patch, so the answer's size without
        // patches is known; the patches then get what's left
        let mut listed: Vec<Value> = files
            .iter()
            .map(|f| {
                json!({
                    "path": f["filename"],
                    "status": f["status"],
                    "additions": f["additions"],
                    "deletions": f["deletions"],
                    "previous_path": f.get("previous_filename"),
                    "patch": Value::Null,
                    "patch_cut": false,
                    "patch_left_out": false,
                })
            })
            .collect();
        let mut answer = json!({
            "repo": format!("{}/{}", pull.owner, pull.repo),
            "number": pull.number,
            "title": pr["title"],
            "description": pr["body"],
            "author": pr["user"]["login"],
            "state": pr["state"],
            "draft": pr["draft"],
            "base": {"ref": pr["base"]["ref"], "sha": pr["base"]["sha"]},
            "head": {"ref": pr["head"]["ref"], "sha": pr["head"]["sha"]},
            "additions": pr["additions"],
            "deletions": pr["deletions"],
            "changed_files": total_files,
            "files_listed": listed.len(),
            "patches_left_out": 0,
            "note": "Some patches were left out to keep this answer whole: read those files with get_file at the head SHA.",
            "files": listed,
        });
        let mut budget = RESULT_LIMIT_CHARS
            .saturating_sub(answer.to_string().chars().count())
            .saturating_sub(RESULT_MARGIN_CHARS);
        let mut omitted = 0;
        listed = std::mem::take(&mut answer["files"])
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (entry, file) in listed.iter_mut().zip(&files) {
            // No patch: binary, or too large for GitHub to show
            let Some(patch) = file["patch"].as_str() else {
                continue;
            };
            let (patch, cut) = truncated(patch, MAX_PATCH_CHARS.min(budget));
            // What it costs in the answer, escapes included
            let cost = Value::from(patch.as_str()).to_string().chars().count();
            if budget == 0 || cost > budget {
                entry["patch_left_out"] = Value::Bool(true);
                omitted += 1;
                budget = 0;
            } else {
                budget -= cost;
                entry["patch"] = Value::from(patch);
                entry["patch_cut"] = Value::Bool(cut);
            }
        }
        answer["files"] = Value::Array(listed);
        answer["patches_left_out"] = Value::from(omitted);
        if omitted == 0 {
            answer["note"] = Value::Null;
        }
        Ok(answer)
    }

    async fn get_file(&self, args: &Value) -> Result<Value, String> {
        let repo = str_arg(args, "repo")?;
        let (owner, name) = repo
            .split_once('/')
            .filter(|(o, n)| valid_name(o) && valid_name(n))
            .ok_or_else(|| format!("'{repo}' isn't <owner>/<repo>"))?;
        let path = str_arg(args, "path")?.trim_start_matches('/');
        if path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(format!("'{path}' isn't a file path in the repository"));
        }
        let mut url = reqwest::Url::parse(&format!("{}/repos/{owner}/{name}/contents/", self.api))
            .map_err(|e| format!("Invalid GitHub API address: {e}"))?;
        url.path_segments_mut()
            .map_err(|_| "Invalid GitHub API address".to_string())?
            .pop_if_empty()
            .extend(path.split('/'));
        if let Some(git_ref) = args
            .get("ref")
            .and_then(Value::as_str)
            .filter(|r| !r.trim().is_empty())
        {
            url.query_pairs_mut().append_pair("ref", git_ref.trim());
        }
        let request = http_client()
            .get(url)
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/vnd.github.raw+json")
            .header("X-GitHub-Api-Version", API_VERSION);
        let bytes = self
            .send(request, &format!("{repo}/{path}"))
            .await?
            .bytes()
            .await
            .map_err(|e| format!("Could not read {path} from GitHub: {e}"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(format!(
                "{path} is {} KB; only files up to {} KB are read",
                bytes.len() / 1024,
                MAX_FILE_BYTES / 1024
            ));
        }
        let text =
            String::from_utf8(bytes.to_vec()).map_err(|_| format!("{path} isn't a text file"))?;
        Ok(json!({"repo": repo, "path": path, "content": text}))
    }

    async fn post_review(&self, args: &Value) -> Result<Value, String> {
        let pull = PullRef::parse(str_arg(args, "url")?)?;
        let body = str_arg(args, "body")?;
        let event = args
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("COMMENT");
        if !["COMMENT", "REQUEST_CHANGES", "APPROVE"].contains(&event) {
            return Err(format!(
                "event '{event}' is unknown: use COMMENT, REQUEST_CHANGES or APPROVE"
            ));
        }
        let comments = args
            .get("comments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if comments.len() > MAX_COMMENTS {
            return Err(format!("At most {MAX_COMMENTS} inline comments per review"));
        }
        let comments = comments
            .iter()
            .map(|c| {
                let path = str_arg(c, "path")?;
                let line = c
                    .get("line")
                    .and_then(Value::as_u64)
                    .filter(|l| *l > 0)
                    .ok_or("Each comment needs a 'line' (a positive number)")?;
                let text = str_arg(c, "body")?;
                Ok(json!({"path": path, "line": line, "side": "RIGHT", "body": text}))
            })
            .collect::<Result<Vec<Value>, String>>()?;
        let what = format!("{}/{}#{}", pull.owner, pull.repo, pull.number);
        // Pin the review to the commit the agent read, so comments land on
        // the lines it saw
        let pr = self.get_json(&pull.path(), &what).await?;
        let commit = pr["head"]["sha"].as_str().unwrap_or_default().to_string();
        let review = self
            .send(
                self.request(reqwest::Method::POST, &format!("{}/reviews", pull.path()))
                    .json(&json!({
                        "commit_id": commit,
                        "body": body,
                        "event": event,
                        "comments": comments,
                    })),
                &what,
            )
            .await?
            .json::<Value>()
            .await
            .map_err(|e| format!("Unexpected response from GitHub: {e}"))?;
        Ok(json!({
            "posted": true,
            "url": review["html_url"],
            "state": review["state"],
            "comments": comments.len(),
        }))
    }
}

impl Connector for GithubConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        Self::tool_defs()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            match name {
                "get_pull_request" => self.get_pull_request(args).await,
                "get_file" => self.get_file(args).await,
                "post_review" => self.post_review(args).await,
                other => Err(format!("Unknown tool '{other}'")),
            }
        })
    }

    /// Doesn't call the API: a token must be present.
    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if self.token.trim().is_empty() {
                Err("No GitHub token".to_string())
            } else {
                Ok(())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn authorized(headers: &HeaderMap) -> bool {
        headers["authorization"] == "Bearer gh-token"
    }

    #[test]
    fn the_cli_token_is_one_word_on_one_line() {
        assert_eq!(
            parse_cli_token(
                "gho_abc123
"
            )
            .as_deref(),
            Some("gho_abc123")
        );
        assert_eq!(
            parse_cli_token(
                "  
"
            ),
            None
        );
        assert_eq!(parse_cli_token("no oauth token found"), None);
    }

    #[test]
    fn pull_requests_parse_from_urls_and_shorthand() {
        let expected = PullRef {
            owner: "acme".into(),
            repo: "web.app".into(),
            number: 42,
        };
        for input in [
            "https://github.com/acme/web.app/pull/42",
            "https://github.com/acme/web.app/pull/42/files",
            " acme/web.app#42 ",
        ] {
            assert_eq!(PullRef::parse(input).unwrap(), expected, "{input}");
        }
        for input in [
            "https://github.com/acme/web/issues/42",
            "acme/web#x",
            "acme#42",
            "https://github.com/../web/pull/1",
            "acme/web#0",
        ] {
            assert!(PullRef::parse(input).is_err(), "{input}");
        }
    }

    #[tokio::test]
    async fn a_pull_request_comes_with_its_files_and_cut_patches() {
        let big = "+x\n".repeat(MAX_PATCH_CHARS);
        let router = Router::new()
            .route(
                "/repos/acme/web/pulls/7",
                get(|headers: HeaderMap| async move {
                    assert!(authorized(&headers));
                    Json(json!({
                        "title": "Fix login", "body": "Closes #3", "user": {"login": "dev"},
                        "state": "open", "draft": false, "changed_files": 2,
                        "additions": 5, "deletions": 1,
                        "base": {"ref": "main", "sha": "b1"}, "head": {"ref": "fix", "sha": "h1"},
                    }))
                }),
            )
            .route(
                "/repos/acme/web/pulls/7/files",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    let big = big.clone();
                    async move {
                        assert_eq!(q["page"], "1");
                        Json(json!([
                            {"filename": "src/login.ts", "status": "modified", "additions": 4, "deletions": 1, "patch": "@@ -1 +1 @@\n-a\n+b"},
                            {"filename": "dist/app.js", "status": "modified", "additions": 1, "deletions": 0, "patch": big},
                        ]))
                    }
                }),
            );
        let github = GithubConnector::new("gh-token".into(), Some(serve(router).await));
        let pr = github
            .call_tool("get_pull_request", &json!({"url": "acme/web#7"}))
            .await
            .unwrap();
        assert_eq!(pr["title"], "Fix login");
        assert_eq!(pr["head"]["sha"], "h1");
        assert_eq!(pr["files"][0]["path"], "src/login.ts");
        assert_eq!(pr["files"][0]["patch_cut"], false);
        assert_eq!(pr["files"][1]["patch_cut"], true);
        assert!(pr["files"][1]["patch"]
            .as_str()
            .unwrap()
            .ends_with("characters]"));
    }

    #[tokio::test]
    async fn a_large_pull_request_leaves_out_later_patches_instead_of_cutting_the_answer() {
        let patch = "+line
"
        .repeat(4_000); // 24,000 chars each, cut to 20,000
        let router = Router::new()
            .route(
                "/repos/acme/web/pulls/9",
                get(|| async { Json(json!({"changed_files": 6, "head": {"sha": "h"}})) }),
            )
            .route(
                "/repos/acme/web/pulls/9/files",
                get(move || {
                    let patch = patch.clone();
                    async move {
                        Json(Value::Array(
                            (0..6)
                                .map(|i| json!({"filename": format!("f{i}.rs"), "status": "modified", "patch": patch}))
                                .collect(),
                        ))
                    }
                }),
            );
        let github = GithubConnector::new("gh-token".into(), Some(serve(router).await));
        let pr = github
            .call_tool("get_pull_request", &json!({"url": "acme/web#9"}))
            .await
            .unwrap();
        let total: usize = pr["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["patch"].as_str().map_or(0, |p| p.chars().count()))
            .sum();
        assert!(total <= RESULT_LIMIT_CHARS, "{total}");
        assert_eq!(pr["files"][5]["patch_left_out"], true);
        assert_eq!(pr["files"][0]["patch_left_out"], false);
        assert!(pr["patches_left_out"].as_u64().unwrap() >= 1);
        assert!(pr["note"].as_str().unwrap().contains("get_file"));
        // Whole as the model reads it: every file is listed, nothing is cut
        assert!(
            pr.to_string().chars().count() <= RESULT_LIMIT_CHARS,
            "fits what the model reads of a tool result"
        );
        assert_eq!(pr["files"].as_array().unwrap().len(), 6);
        assert_eq!(pr["files"][5]["path"], "f5.rs");
    }

    #[tokio::test]
    async fn files_are_read_raw_at_a_ref_and_paths_cant_escape() {
        let router = Router::new().route(
            "/repos/acme/web/contents/{*path}",
            get(
                |Path(path): Path<String>,
                 Query(q): Query<HashMap<String, String>>,
                 headers: HeaderMap| async move {
                    assert!(authorized(&headers));
                    assert_eq!(headers["accept"], "application/vnd.github.raw+json");
                    format!("{path}@{}", q.get("ref").map_or("default", String::as_str))
                },
            ),
        );
        let github = GithubConnector::new("gh-token".into(), Some(serve(router).await));
        let file = github
            .call_tool(
                "get_file",
                &json!({"repo": "acme/web", "path": "docs/CONTRIBUTING.md", "ref": "main"}),
            )
            .await
            .unwrap();
        assert_eq!(file["content"], "docs/CONTRIBUTING.md@main");
        for path in ["../secrets", "a//b", "a/./b"] {
            let error = github
                .call_tool("get_file", &json!({"repo": "acme/web", "path": path}))
                .await
                .unwrap_err();
            assert!(error.contains("isn't a file path"), "{path}: {error}");
        }
    }

    #[tokio::test]
    async fn a_review_is_pinned_to_the_head_commit() {
        let posted: Arc<Mutex<Option<Value>>> = Arc::default();
        let router = Router::new()
            .route(
                "/repos/acme/web/pulls/7",
                get(|| async { Json(json!({"head": {"sha": "h1"}})) }),
            )
            .route(
                "/repos/acme/web/pulls/7/reviews",
                post(
                    |State(posted): State<Arc<Mutex<Option<Value>>>>, headers: HeaderMap, Json(body): Json<Value>| async move {
                        assert!(authorized(&headers));
                        *posted.lock().unwrap() = Some(body);
                        Json(json!({"html_url": "https://github.com/acme/web/pull/7#r1", "state": "COMMENTED"}))
                    },
                ),
            )
            .with_state(posted.clone());
        let github = GithubConnector::new("gh-token".into(), Some(serve(router).await));
        let result = github
            .call_tool(
                "post_review",
                &json!({
                    "url": "https://github.com/acme/web/pull/7",
                    "body": "Two findings",
                    "comments": [{"path": "src/login.ts", "line": 12, "body": "Unchecked error"}],
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["posted"], true);
        let body = posted.lock().unwrap().clone().unwrap();
        assert_eq!(body["commit_id"], "h1");
        assert_eq!(body["event"], "COMMENT");
        assert_eq!(body["comments"][0]["side"], "RIGHT");

        let error = github
            .call_tool(
                "post_review",
                &json!({"url": "acme/web#7", "body": "x", "event": "MERGE"}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("unknown"));
    }

    #[tokio::test]
    async fn errors_explain_access_without_echoing_the_token() {
        let router = Router::new()
            .route(
                "/repos/acme/private/pulls/1",
                get(|| async { StatusCode::NOT_FOUND }),
            )
            .route(
                "/repos/acme/web/pulls/1",
                get(|| async { StatusCode::UNAUTHORIZED }),
            );
        let github = GithubConnector::new("gh-token".into(), Some(serve(router).await));
        let hidden = github
            .call_tool("get_pull_request", &json!({"url": "acme/private#1"}))
            .await
            .unwrap_err();
        assert!(hidden.contains("private repository"), "{hidden}");
        let rejected = github
            .call_tool("get_pull_request", &json!({"url": "acme/web#1"}))
            .await
            .unwrap_err();
        assert!(rejected.contains("rejected the token"), "{rejected}");
        assert!(!hidden.contains("gh-token") && !rejected.contains("gh-token"));
    }
}
