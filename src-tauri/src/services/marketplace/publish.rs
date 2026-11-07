//! "Publish agent": share one of the user's own agents through a registry.
//!
//! 1. The agent's folder must be committed and pushed in the user's agents
//!    repo (Back up now, in Settings › Backup): the release is that exact
//!    commit.
//! 2. The release is tagged in the agents repo as `<id>-v<version>` (the
//!    repo holds many agents, so tags are prefixed).
//! 3. The registry entry is generated from skill.md (capabilities included)
//!    and, if the user has a publisher key, signed.
//! 4. A pull request adds or updates `agents/<id>.json` and `index.json` on
//!    the registry. It's opened from a fork when the user can't push to the
//!    registry. Community registries the user can push to are merged right
//!    away; the public registry waits for review.

use std::path::Path;

use serde_json::{json, Value};

use super::github::{FileCommit, GitHubApi};
use super::manifest::{parse_version, NANDI_VERSION};
use crate::services::domain::Agent;

/// The release being published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub agent_id: String,
    pub version: String,
    /// `<id>-v<version>`
    pub tag: String,
    /// The pushed commit the tag names
    pub commit: String,
    /// `github.com/<owner>/<repo>` (the user's agents repo)
    pub repo: String,
    /// The agent's folder in it, e.g. `.nandi/agents/writer`
    pub path: String,
    /// GitHub login, lowercased
    pub publisher: String,
}

/// What was published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub pr_url: String,
    pub merged: bool,
}

/// The tag for an agent's release in a multi-agent repo.
pub fn release_tag(agent_id: &str, version: &str) -> String {
    format!("{agent_id}-v{}", version.trim_start_matches('v'))
}

/// `https://github.com/o/r(.git)` / `git@github.com:o/r.git` → `(o, r)`.
pub fn github_repo(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = rest.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| (owner.to_string(), repo.to_string()))
}

/// The registry entry for a release, from the agent's skill.md.
pub fn entry(
    agent: &Agent,
    release: &Release,
    tags: &[String],
    has_readme: bool,
    signature: Option<&str>,
) -> Value {
    let capabilities = agent.capabilities();
    let tools: Vec<String> = capabilities.tools().allowed().map_or_else(
        || {
            ["shell", "read_file", "write_file", "list_dir"]
                .map(String::from)
                .to_vec()
        },
        <[String]>::to_vec,
    );
    let (owner_repo, path) = (
        release.repo.trim_start_matches("github.com/"),
        &release.path,
    );
    let mut value = json!({
        "id": release.agent_id,
        "name": agent.name(),
        "description": agent.description(),
        "publisher": release.publisher,
        "repo": release.repo,
        "path": path,
        "latest_version": release.version.trim_start_matches('v'),
        "ref": release.tag,
        "commit": release.commit,
        "min_nandi_version": NANDI_VERSION,
        "tags": tags,
        "capabilities": {
            "tools": tools,
            "network": capabilities.network().allow.iter().map(|r| r.host.clone()).collect::<Vec<_>>(),
            "pilots": capabilities.pilots().iter().map(|p| p.as_str().to_string()).collect::<Vec<_>>(),
            "requires_approval": capabilities.tools().approval_list(),
            "aws": capabilities.aws().map(|a| a.services.iter().map(|s| s.service.clone()).collect::<Vec<_>>()).unwrap_or_default(),
        },
    });
    if has_readme {
        value["readme_url"] = format!(
            "https://raw.githubusercontent.com/{owner_repo}/{}/{path}/README.md",
            release.tag
        )
        .into();
    }
    if let Some(signature) = signature {
        value["signature"] = signature.into();
    }
    value
}

/// Which list of the registry an entry goes in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryList {
    Agents,
    Connectors,
}

impl EntryList {
    /// The folder and the `index.json` list
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Connectors => "connectors",
        }
    }
}

/// `index.json` with `entry` added to (or replacing its ID in) `list`
/// (`agents` or `connectors`).
pub fn upsert_index(index: &str, list: &str, entry: &Value) -> Result<String, String> {
    let mut value: Value = if index.trim().is_empty() {
        json!({ "schema_version": 1, "agents": [], "packs": [] })
    } else {
        serde_json::from_str(index)
            .map_err(|e| format!("The registry's index.json isn't valid: {e}"))?
    };
    // A bare list is the agents
    if value.is_array() {
        value = json!({ "schema_version": 1, "agents": value, "packs": [] });
    }
    let id = entry["id"].as_str().unwrap_or_default().to_string();
    if value.get(list).is_none() {
        value[list] = json!([]);
    }
    let entries = value[list]
        .as_array_mut()
        .ok_or_else(|| format!("The registry's index.json has no {list} list"))?;
    match entries.iter_mut().find(|a| a["id"] == id.as_str()) {
        Some(existing) => *existing = entry.clone(),
        None => entries.push(entry.clone()),
    }
    Ok(format!("{value:#}\n"))
}

/// Where the user backs up and pushes their agents repo
const BACK_UP: &str = "Use Back up now in Settings › Backup";

/// The pushed commit the agent's folder is at, after checking it has no
/// unsynced changes. Returns (`github.com/<owner>/<repo>`, commit).
pub fn synced_release(repo_root: &Path, agent_dir: &Path) -> Result<(String, String), String> {
    let repo = git2::Repository::open(repo_root).map_err(|_| {
        "Your agents repo isn't set up yet. Add a git remote in Settings › Backup.".to_string()
    })?;
    let relative = agent_dir
        .strip_prefix(repo_root)
        .map_err(|_| "The agent isn't in your agents repo".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .pathspec(format!("{relative}/"));
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|e| e.message().to_string())?;
    if !statuses.is_empty() {
        return Err(format!(
            "The agent has changes that aren't backed up yet. {BACK_UP}, then publish."
        ));
    }
    let commit = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| format!("Your agents repo has no commits yet. {BACK_UP} first."))?
        .id();
    // Back up now pushes to the user's own branch (`nandi/<slug>`): any
    // branch on origin at this commit makes it public
    let pushed = repo
        .references_glob("refs/remotes/origin/*")
        .map_err(|e| e.message().to_string())?
        .flatten()
        .any(|reference| reference.target() == Some(commit));
    if !pushed {
        return Err(format!(
            "Your latest changes aren't on GitHub yet. {BACK_UP}, then publish."
        ));
    }
    let commit = commit.to_string();
    let origin = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().map(str::to_string))
        .unwrap_or_default();
    let (owner, name) =
        github_repo(&origin).ok_or("Publishing needs your agents repo to be on GitHub")?;
    Ok((format!("github.com/{owner}/{name}"), commit))
}

impl GitHubApi {
    /// Tags the release in the agents repo and opens (or, for a registry
    /// the user may merge into, merges) the registry pull request.
    pub async fn publish(
        &self,
        release: &Release,
        entry: &Value,
        registry: &str,
        auto_merge: bool,
        public_key: Option<&str>,
        entry_list: EntryList,
    ) -> Result<Published, String> {
        parse_version(&release.version)?;
        let (agents_owner, agents_repo) =
            github_repo(&release.repo).ok_or("The agents repo isn't on GitHub")?;
        self.create_ref(
            &agents_owner,
            &agents_repo,
            &format!("tags/{}", release.tag),
            &release.commit,
        )
        .await?;

        let (owner, name) = github_repo(registry).ok_or("Publishing needs a GitHub registry")?;
        let upstream = self
            .repo(&owner, &name)
            .await?
            .ok_or_else(|| format!("{registry} wasn't found (or you can't see it)"))?;
        let target = if upstream.can_push {
            upstream.clone()
        } else {
            self.fork(&owner, &name).await?
        };
        let base_sha = self
            .branch_sha(&upstream.owner, &upstream.name, &upstream.default_branch)
            .await?;
        let branch = format!("nandi/publish-{}", release.tag);
        self.create_ref(
            &target.owner,
            &target.name,
            &format!("heads/{branch}"),
            &base_sha,
        )
        .await?;

        let what = format!(
            "{} v{}",
            release.agent_id,
            release.version.trim_start_matches('v')
        );
        let entry_path = format!("{}/{}.json", entry_list.as_str(), release.agent_id);
        let existing = self
            .file(&target.owner, &target.name, &entry_path, Some(&branch))
            .await?;
        self.put_file(
            &target.owner,
            &target.name,
            &entry_path,
            &format!("{entry:#}\n"),
            FileCommit {
                message: &format!("Publish {what}"),
                branch: Some(&branch),
                sha: existing.as_ref().map(|f| f.sha.as_str()),
            },
        )
        .await?;
        let index = self
            .file(&target.owner, &target.name, "index.json", Some(&branch))
            .await?;
        let updated = upsert_index(
            index.as_ref().map_or("", |f| f.content.as_str()),
            entry_list.as_str(),
            entry,
        )?;
        self.put_file(
            &target.owner,
            &target.name,
            "index.json",
            &updated,
            FileCommit {
                message: &format!("List {what}"),
                branch: Some(&branch),
                sha: index.as_ref().map(|f| f.sha.as_str()),
            },
        )
        .await?;
        let publisher_path = format!("publishers/{}.json", release.publisher);
        if self
            .file(&target.owner, &target.name, &publisher_path, Some(&branch))
            .await?
            .is_none()
        {
            let publisher = json!({
                "github_handle": release.publisher,
                "public_key": public_key,
                "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            });
            self.put_file(
                &target.owner,
                &target.name,
                &publisher_path,
                &format!("{publisher:#}\n"),
                FileCommit {
                    message: &format!("Add publisher {}", release.publisher),
                    branch: Some(&branch),
                    sha: None,
                },
            )
            .await?;
        }

        let head = if target.owner == upstream.owner {
            branch.clone()
        } else {
            format!("{}:{branch}", target.owner)
        };
        let body = format!(
            "Publishes **{}** v{} from [{}](https://{}/tree/{}/{}).\n\n\
             - Release: `{}` at `{}`\n\
             - Tools: {}\n\
             - Network: {}\n\
             - Signed: {}\n\n\
             _Opened by Nandi's Publish agent._",
            entry["name"].as_str().unwrap_or(&release.agent_id),
            release.version.trim_start_matches('v'),
            release.repo,
            release.repo,
            release.tag,
            release.path,
            release.tag,
            &release.commit[..12.min(release.commit.len())],
            match entry_list {
                EntryList::Agents => list(&entry["capabilities"]["tools"]),
                EntryList::Connectors => list(&Value::Array(
                    entry["tools"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|t| Value::from(format!(
                            "{} ({})",
                            t["name"].as_str().unwrap_or_default(),
                            t["access"].as_str().unwrap_or_default()
                        )))
                        .collect(),
                )),
            },
            match entry_list {
                EntryList::Agents => list(&entry["capabilities"]["network"]),
                EntryList::Connectors => list(&entry["hosts"]),
            },
            if entry.get("signature").is_some() {
                "yes"
            } else {
                "no"
            },
        );
        let (number, pr_url) = self
            .create_pr(
                &upstream.owner,
                &upstream.name,
                &format!("Publish {what}"),
                &body,
                &head,
                &upstream.default_branch,
            )
            .await?;
        let merged = auto_merge && upstream.can_push;
        if merged {
            self.merge_pr(&upstream.owner, &upstream.name, number)
                .await?;
        }
        Ok(Published { pr_url, merged })
    }
}

fn list(value: &Value) -> String {
    let items: Vec<&str> = value
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::AgentId;
    use base64::Engine;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    fn agent() -> Agent {
        Agent::from_skill_md(
            AgentId::parse("writer").unwrap(),
            "---\nname: Writer\ndescription: Writes things\nallowed-tools: [read_file, write_file]\nrequires_approval: [write_file]\nnetwork:\n  allow:\n    - host: api.example.com\n---\nWrite.",
        )
        .unwrap()
    }

    fn release() -> Release {
        Release {
            agent_id: "writer".into(),
            version: "1.0.0".into(),
            tag: release_tag("writer", "1.0.0"),
            commit: "a".repeat(40),
            repo: "github.com/alice/nandi-agents".into(),
            path: ".nandi/agents/writer".into(),
            publisher: "alice".into(),
        }
    }

    #[test]
    fn builds_a_valid_entry_and_index() {
        let entry = entry(
            &agent(),
            &release(),
            &["writing".into()],
            true,
            Some("ed25519:sig"),
        );
        assert_eq!(entry["ref"], "writer-v1.0.0");
        assert_eq!(entry["capabilities"]["network"][0], "api.example.com");
        assert_eq!(entry["capabilities"]["requires_approval"][0], "write_file");
        assert!(entry["readme_url"]
            .as_str()
            .unwrap()
            .ends_with("/writer-v1.0.0/.nandi/agents/writer/README.md"));
        // It parses as a registry entry (the installer accepts its path)
        let parsed = super::super::manifest::parse_agent_entry(&entry.to_string(), "r").unwrap();
        assert_eq!(parsed.git_ref, "writer-v1.0.0");
        assert_eq!(parsed.commit.as_deref(), Some("a".repeat(40).as_str()));

        let index = upsert_index("", "agents", &entry).unwrap();
        let again = upsert_index(&index, "agents", &entry).unwrap();
        let parsed = super::super::manifest::parse_index(&again, "r").unwrap();
        assert_eq!(parsed.agents.len(), 1, "replaced, not duplicated");
        assert!(upsert_index("not json", "agents", &entry).is_err());
        // Connectors get their own list next to the agents
        let both = upsert_index(&index, "connectors", &json!({"id": "fetch"})).unwrap();
        let parsed: Value = serde_json::from_str(&both).unwrap();
        assert_eq!(parsed["connectors"][0]["id"], "fetch");
        assert_eq!(parsed["agents"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn parses_github_urls() {
        for url in [
            "https://github.com/a/b.git",
            "git@github.com:a/b.git",
            "github.com/a/b/",
        ] {
            assert_eq!(github_repo(url), Some(("a".into(), "b".into())), "{url}");
        }
        assert_eq!(github_repo("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn needs_the_agent_backed_up_and_pushed() {
        let tmp = tempfile::tempdir().unwrap();
        let (origin_dir, root) = (tmp.path().join("origin.git"), tmp.path().join("repo"));
        git2::Repository::init_bare(&origin_dir).unwrap();
        let repo = git2::Repository::init(&root).unwrap();
        repo.remote("origin", "https://github.com/alice/nandi-agents.git")
            .unwrap();
        let agent_dir = root.join(".nandi/agents/writer");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("skill.md"), "---\nname: W\n---\n").unwrap();
        let unsaved = synced_release(&root, &agent_dir).unwrap_err();
        assert!(
            unsaved.contains("Back up now in Settings › Backup"),
            "{unsaved}"
        );

        // Committed but not pushed
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("A", "a@example.com").unwrap();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "agent", &tree, &[])
            .unwrap();
        assert!(synced_release(&root, &agent_dir)
            .unwrap_err()
            .contains("aren't on GitHub"));

        // Backed up: Back up now pushed HEAD to the user's own branch
        repo.reference("refs/remotes/origin/nandi/alice", commit, true, "push")
            .unwrap();
        let (url, sha) = synced_release(&root, &agent_dir).unwrap();
        assert_eq!(url, "github.com/alice/nandi-agents");
        assert_eq!(sha, commit.to_string());
    }

    /// A fake GitHub with an agents repo (alice's) and a registry owned by
    /// someone else (alice can't push: a fork is needed).
    #[derive(Default)]
    struct Fake {
        refs: BTreeMap<String, String>,
        files: BTreeMap<String, String>,
        pulls: Vec<(String, String, String)>,
        merged: Vec<u64>,
        forked: bool,
        can_push_registry: bool,
    }

    async fn serve(fake: Arc<Mutex<Fake>>) -> String {
        use axum::extract::{Path as P, Query, State};
        use axum::http::StatusCode;
        use axum::routing::{get, post, put};
        use axum::Json;
        type S = Arc<Mutex<Fake>>;
        let app = axum::Router::new()
            .route("/user", get(|| async { Json(json!({ "login": "alice" })) }))
            .route(
                "/repos/{owner}/{repo}",
                get(|State(s): State<S>, P((o, r)): P<(String, String)>| async move {
                    let fake = s.lock().unwrap();
                    let exists = o == "alice" && (r == "nandi-agents" || (r == "registry" && fake.forked)) || o == "bob";
                    if !exists {
                        return (StatusCode::NOT_FOUND, Json(json!({})));
                    }
                    let push = o == "alice" || fake.can_push_registry;
                    (StatusCode::OK, Json(json!({ "owner": { "login": o }, "name": r, "default_branch": "main", "permissions": { "push": push } })))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/forks",
                post(|State(s): State<S>| async move {
                    s.lock().unwrap().forked = true;
                    (StatusCode::ACCEPTED, Json(json!({ "owner": { "login": "alice" }, "name": "registry" })))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/git/ref/{*git_ref}",
                get(|State(s): State<S>, P((o, r, g)): P<(String, String, String)>| async move {
                    let fake = s.lock().unwrap();
                    let key = format!("{o}/{r}:{g}");
                    let sha = fake.refs.get(&key).cloned().or_else(|| (g == "heads/main").then(|| "b".repeat(40)));
                    match sha {
                        Some(sha) => (StatusCode::OK, Json(json!({ "object": { "sha": sha } }))),
                        None => (StatusCode::NOT_FOUND, Json(json!({}))),
                    }
                }),
            )
            .route(
                "/repos/{owner}/{repo}/git/refs",
                post(|State(s): State<S>, P((o, r)): P<(String, String)>, Json(body): Json<Value>| async move {
                    let git_ref = body["ref"].as_str().unwrap().trim_start_matches("refs/").to_string();
                    s.lock().unwrap().refs.insert(format!("{o}/{r}:{git_ref}"), body["sha"].as_str().unwrap().to_string());
                    (StatusCode::CREATED, Json(json!({})))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/contents/{*path}",
                get(|State(s): State<S>, P((o, r, p)): P<(String, String, String)>, Query(q): Query<BTreeMap<String, String>>| async move {
                    let key = format!("{o}/{r}@{}:{p}", q.get("ref").cloned().unwrap_or_default());
                    match s.lock().unwrap().files.get(&key) {
                        Some(content) => (StatusCode::OK, Json(json!({ "content": base64::engine::general_purpose::STANDARD.encode(content), "sha": "blob" }))),
                        None => (StatusCode::NOT_FOUND, Json(json!({}))),
                    }
                })
                .put(|State(s): State<S>, P((o, r, p)): P<(String, String, String)>, Json(body): Json<Value>| async move {
                    let content = base64::engine::general_purpose::STANDARD.decode(body["content"].as_str().unwrap()).unwrap();
                    let key = format!("{o}/{r}@{}:{p}", body["branch"].as_str().unwrap_or_default());
                    s.lock().unwrap().files.insert(key, String::from_utf8(content).unwrap());
                    (StatusCode::CREATED, Json(json!({})))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/pulls",
                post(|State(s): State<S>, P((o, r)): P<(String, String)>, Json(body): Json<Value>| async move {
                    let mut fake = s.lock().unwrap();
                    fake.pulls.push((format!("{o}/{r}"), body["head"].as_str().unwrap().into(), body["title"].as_str().unwrap().into()));
                    let n = fake.pulls.len();
                    (StatusCode::CREATED, Json(json!({ "number": n, "html_url": format!("https://github.com/{o}/{r}/pull/{n}") })))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/pulls/{number}/merge",
                put(|State(s): State<S>, P((_, _, n)): P<(String, String, u64)>| async move {
                    s.lock().unwrap().merged.push(n);
                    (StatusCode::OK, Json(json!({ "merged": true })))
                }),
            )
            .with_state(fake);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn tags_the_release_and_opens_a_pr_from_a_fork() {
        let fake = Arc::new(Mutex::new(Fake::default()));
        let api = GitHubApi::new(serve(fake.clone()).await, "token");
        let release = release();
        let entry = entry(&agent(), &release, &[], false, None);

        let published = api
            .publish(
                &release,
                &entry,
                "github.com/bob/registry",
                true,
                Some("ed25519:key"),
                EntryList::Agents,
            )
            .await
            .unwrap();
        assert_eq!(published.pr_url, "https://github.com/bob/registry/pull/1");
        assert!(
            !published.merged,
            "can't push to the public registry: review"
        );

        let fake = fake.lock().unwrap();
        assert_eq!(
            fake.refs["alice/nandi-agents:tags/writer-v1.0.0"],
            "a".repeat(40)
        );
        assert!(fake.forked);
        assert_eq!(fake.pulls[0].1, "alice:nandi/publish-writer-v1.0.0");
        let branch = "alice/registry@nandi/publish-writer-v1.0.0";
        let index = &fake.files[&format!("{branch}:index.json")];
        assert_eq!(
            super::super::manifest::parse_index(index, "r")
                .unwrap()
                .agents[0]
                .id,
            "writer"
        );
        assert!(fake.files[&format!("{branch}:publishers/alice.json")].contains("ed25519:key"));
        assert!(fake
            .files
            .contains_key(&format!("{branch}:agents/writer.json")));
    }

    #[tokio::test]
    async fn merges_right_away_into_a_community_registry() {
        let fake = Arc::new(Mutex::new(Fake {
            can_push_registry: true,
            ..Default::default()
        }));
        let api = GitHubApi::new(serve(fake.clone()).await, "token");
        let release = release();
        let entry = entry(&agent(), &release, &[], false, None);
        let published = api
            .publish(
                &release,
                &entry,
                "github.com/bob/registry",
                true,
                None,
                EntryList::Agents,
            )
            .await
            .unwrap();
        assert!(published.merged);
        {
            let fake = fake.lock().unwrap();
            assert!(!fake.forked);
            assert_eq!(fake.pulls[0].1, "nandi/publish-writer-v1.0.0");
            assert_eq!(fake.merged, [1]);
        }

        // The same version at another commit is refused
        let mut moved = release.clone();
        moved.commit = "c".repeat(40);
        let err = api
            .publish(
                &moved,
                &entry,
                "github.com/bob/registry",
                true,
                None,
                EntryList::Agents,
            )
            .await
            .unwrap_err();
        assert!(err.contains("another commit"), "{err}");
    }
}
