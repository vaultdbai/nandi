//! "Create my community registry": a private GitHub repo with the registry
//! layout, shared with collaborators. Zero new infrastructure: GitHub
//! hosts it, access is GitHub's collaborator list, and the app reads it
//! with the members' own sign-in.
//!
//! The repo (`nandi-community-<login>`) is created and seeded through the
//! REST API (Contents API commits), so nothing is cloned locally. Running
//! it again reuses an existing repo and only adds missing files.

use serde_json::json;

use super::github::FileCommit;
pub use super::github::GitHubApi;

/// What `setup` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunitySetup {
    /// `github.com/<owner>/<repo>`, the registry URL to share
    pub url: String,
    /// Where the owner invites collaborators
    pub invite_url: String,
    /// The repo was created now (not found from an earlier run)
    pub created: bool,
}

/// `nandi-community-<login>`; GitHub logins are already repo-name safe.
pub fn repo_name(login: &str) -> String {
    format!("nandi-community-{}", login.to_ascii_lowercase())
}

/// The registry skeleton: an empty index, the owner as a publisher, and a
/// README explaining how members add entries.
pub fn seed_files(owner: &str, repo: &str) -> Vec<(String, String)> {
    let index = json!({ "schema_version": 1, "agents": [], "packs": [] });
    let publisher = json!({
        "github_handle": owner.to_ascii_lowercase(),
        "public_key": null,
        "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    let readme = format!(
        "# {repo}\n\n\
         A private Nandi marketplace for @{owner} and collaborators.\n\n\
         ## Join\n\n\
         1. Ask @{owner} to add you as a collaborator on this repo.\n\
         2. In Nandi: **Marketplace → Registries → Add registry**, URL `github.com/{owner}/{repo}`, kind *community*, and tick *Use my GitHub sign-in*.\n\n\
         ## Share an agent\n\n\
         1. Push your agent folder (`skill.md`, `instructions.md`, optional `workflows/`) to any repo members can read, and tag a release (`git tag -a v1.0.0 -m v1.0.0 && git push --tags`).\n\
         2. Add `agents/<id>.json` here (same fields as the public registry: `id`, `name`, `description`, `publisher`, `repo`, `path`, `latest_version`, `capabilities`, …) and the same entry to the `agents` list in `index.json`.\n\
         3. Commit to `main`. Members see it in **Discover** within a few minutes.\n\n\
         Packs work the same way, under `packs/` and the index's `packs` list.\n"
    );
    vec![
        ("index.json".to_string(), format!("{index:#}\n")),
        (
            format!("publishers/{}.json", owner.to_ascii_lowercase()),
            format!("{publisher:#}\n"),
        ),
        ("agents/.gitkeep".to_string(), String::new()),
        ("packs/.gitkeep".to_string(), String::new()),
        ("README.md".to_string(), readme),
    ]
}

impl GitHubApi {
    /// Creates (or reuses) the user's private community registry and makes
    /// sure the registry skeleton is in it.
    pub async fn setup_community_registry(&self) -> Result<CommunitySetup, String> {
        let owner = self.login().await?;
        let repo = repo_name(&owner);
        let created = if self.repo(&owner, &repo).await?.is_some() {
            false
        } else {
            let body = json!({
                "name": repo,
                "private": true,
                "description": "Nandi community registry: agents and packs shared with collaborators",
                "has_issues": false,
                "has_wiki": false,
            });
            let (status, response) = self
                .send(reqwest::Method::POST, "/user/repos", Some(body))
                .await?;
            if !status.is_success() {
                return Err(Self::failed("create the repo", status, &response));
            }
            log::info!("Created community registry {owner}/{repo}");
            true
        };
        for (path, content) in seed_files(&owner, &repo) {
            if self.file(&owner, &repo, &path, None).await?.is_none() {
                let message = format!("Add {path}");
                self.put_file(
                    &owner,
                    &repo,
                    &path,
                    &content,
                    FileCommit {
                        message: &message,
                        branch: None,
                        sha: None,
                    },
                )
                .await?;
            }
        }
        Ok(CommunitySetup {
            url: format!("github.com/{owner}/{repo}"),
            invite_url: format!("https://github.com/{owner}/{repo}/settings/access"),
            created,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// A fake GitHub: one user, repos and files in memory.
    #[derive(Default)]
    struct Fake {
        repos: Vec<String>,
        files: BTreeMap<String, String>,
        private: Option<bool>,
    }

    async fn serve(fake: Arc<Mutex<Fake>>) -> String {
        use axum::extract::{Path, State};
        use axum::http::StatusCode;
        use axum::routing::{get, post};
        use axum::Json;
        type S = Arc<Mutex<Fake>>;
        let app = axum::Router::new()
            .route("/user", get(|| async { Json(json!({ "login": "Alice" })) }))
            .route(
                "/repos/{owner}/{repo}",
                get(|State(s): State<S>, Path((o, r)): Path<(String, String)>| async move {
                    let found = s.lock().unwrap().repos.contains(&format!("{o}/{r}"));
                    if found { StatusCode::OK } else { StatusCode::NOT_FOUND }
                }),
            )
            .route(
                "/user/repos",
                post(|State(s): State<S>, Json(body): Json<Value>| async move {
                    let mut fake = s.lock().unwrap();
                    fake.repos.push(format!("Alice/{}", body["name"].as_str().unwrap()));
                    fake.private = body["private"].as_bool();
                    (StatusCode::CREATED, Json(json!({})))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/contents/{*path}",
                get(|State(s): State<S>, Path((_, _, p)): Path<(String, String, String)>| async move {
                    if s.lock().unwrap().files.contains_key(&p) { StatusCode::OK } else { StatusCode::NOT_FOUND }
                })
                .put(
                    |State(s): State<S>,
                     Path((_, _, p)): Path<(String, String, String)>,
                     Json(body): Json<Value>| async move {
                        let content = base64::engine::general_purpose::STANDARD
                            .decode(body["content"].as_str().unwrap())
                            .unwrap();
                        s.lock().unwrap().files.insert(p, String::from_utf8(content).unwrap());
                        (StatusCode::CREATED, Json(json!({})))
                    },
                ),
            )
            .with_state(fake);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn creates_a_private_seeded_registry_once() {
        let fake = Arc::new(Mutex::new(Fake::default()));
        let api = GitHubApi::new(serve(fake.clone()).await, "token");

        let setup = api.setup_community_registry().await.unwrap();
        assert!(setup.created);
        assert_eq!(setup.url, "github.com/Alice/nandi-community-alice");
        assert_eq!(
            setup.invite_url,
            "https://github.com/Alice/nandi-community-alice/settings/access"
        );
        {
            let fake = fake.lock().unwrap();
            assert_eq!(fake.private, Some(true));
            let index: Value = serde_json::from_str(&fake.files["index.json"]).unwrap();
            assert_eq!(index["schema_version"], 1);
            assert!(fake.files.contains_key("publishers/alice.json"));
            assert!(fake.files["README.md"].contains("github.com/Alice/nandi-community-alice"));
        }

        // Again: reused, nothing rewritten
        fake.lock()
            .unwrap()
            .files
            .insert("index.json".into(), "custom".into());
        let again = api.setup_community_registry().await.unwrap();
        assert!(!again.created);
        assert_eq!(fake.lock().unwrap().files["index.json"], "custom");
        assert_eq!(fake.lock().unwrap().repos.len(), 1);
    }

    #[test]
    fn seeded_index_parses_as_a_registry() {
        let files = seed_files("bob", "nandi-community-bob");
        let index = &files.iter().find(|(p, _)| p == "index.json").unwrap().1;
        let parsed = crate::services::marketplace::manifest::parse_index(index, "r").unwrap();
        assert!(parsed.agents.is_empty() && parsed.packs.is_empty());
    }
}
