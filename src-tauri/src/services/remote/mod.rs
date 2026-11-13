//! The optional git remote for `~/.nandi/repo` (wave final, Track A). Any
//! git URL; the credential lives in the OS keychain only.
//!
//! **Branch protection is load-bearing.** Every push goes to
//! `refs/heads/<prefix>/<slug>` (`nandi/alice-chen` by default), never to
//! `main` or `master`. `assert_pushable_branch` guards every push, with no
//! override: no flag, no config option.
//!
//! - `credentials`: keychain storage (`nandi.remote.credential`, JSON)
//! - `git_ops`: test, push, pull, list branches (git2)

pub mod credentials;
pub mod git_ops;

use std::path::{Path, PathBuf};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::Mutex;

use crate::model::Timestamp;
use crate::services::auth;
use crate::services::domain::ChangedBy;
use crate::services::events::{self as bus_events, EventBus, NandiEvent};
use crate::services::repo_store::RepoStore;
use crate::services::secrets::Secrets;
use crate::services::AppState;
use crate::utils::nandi_config;

/// The branch prefix unless configured otherwise
pub const DEFAULT_BRANCH_PREFIX: &str = "nandi";

/// `remote://status_changed`
pub const REMOTE_STATUS_CHANGED: &str = "remote://status_changed";
/// `remote://push_completed`
pub const REMOTE_PUSH_COMPLETED: &str = "remote://push_completed";
/// `remote://pull_completed`
pub const REMOTE_PULL_COMPLETED: &str = "remote://pull_completed";
/// `remote://error`
pub const REMOTE_ERROR: &str = "remote://error";

pub type RemoteError = String;

/// What `config.json` keeps about the remote (`remote`). No secrets:
/// `has_credentials` only says one is in the keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfig {
    pub url: String,
    pub has_credentials: bool,
    pub credential_kind: CredentialKind,
    pub branch_prefix: String,
    pub user_slug: String,
    /// Epoch ms
    pub last_push_at: Option<Timestamp>,
    pub last_pull_at: Option<Timestamp>,
    pub last_push_commit: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CredentialKind {
    #[default]
    None,
    Token,
    SshKey,
    UserPass,
    /// The GitHub sign-in's token (`auth::github`), read when it's used;
    /// github.com addresses only
    Github,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub configured: bool,
    pub config: Option<RemoteConfig>,
    pub reachable: bool,
    /// `<prefix>/<slug>`
    pub target_branch: String,
    /// Full SHA; empty for an empty repo
    pub local_head: String,
    pub remote_head: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// A pull left a merge in progress
    pub conflicts: bool,
    pub conflicted_files: Vec<String>,
}

/// Comes in from a client once (configure); stored in the keychain, never
/// echoed back. No `Debug`: it may hold a secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCredential {
    pub kind: CredentialKind,
    /// `Token`, or the password of `UserPass`. Never stored for `Github`:
    /// the sign-in's token is filled in when the credential is loaded.
    pub token: Option<String>,
    pub username: Option<String>,
    pub ssh_key_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub pushed_commits: u32,
    pub target_branch: String,
    pub remote_head: String,
    /// The remote branch has commits we don't: refused (pass `overwrite`)
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub pulled_commits: u32,
    /// The merge is left in progress; nothing was auto-resolved
    pub conflicts: bool,
    pub conflicted_files: Vec<String>,
}

/// `BoxFuture`-returning so `AppState` can hold an `Arc<dyn RemoteService>`.
pub trait RemoteService: Send + Sync {
    fn get_status(&self) -> BoxFuture<'_, Result<RemoteStatus, RemoteError>>;
    /// `prefix` / `slug` `None` keep the current (or default) ones
    fn configure(
        &self,
        url: String,
        credential: Option<RemoteCredential>,
        prefix: Option<String>,
        slug: Option<String>,
    ) -> BoxFuture<'_, Result<RemoteStatus, RemoteError>>;
    /// Forgets the remote and its credential
    fn clear(&self) -> BoxFuture<'_, Result<(), RemoteError>>;
    fn test_connection(&self) -> BoxFuture<'_, Result<(), RemoteError>>;
    /// To `<prefix>/<slug>` only; `overwrite` force-pushes that branch
    fn push(&self, overwrite: bool) -> BoxFuture<'_, Result<PushResult, RemoteError>>;
    fn pull(&self) -> BoxFuture<'_, Result<PullResult, RemoteError>>;
    fn list_remote_branches(&self) -> BoxFuture<'_, Result<Vec<String>, RemoteError>>;
}

/// `remote://status_changed`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatusChanged {
    pub configured: bool,
}

impl NandiEvent for RemoteStatusChanged {
    const NAME: &'static str = REMOTE_STATUS_CHANGED;
}

/// `remote://push_completed`
#[derive(Debug, Clone, Serialize)]
pub struct RemotePushCompleted {
    #[serde(flatten)]
    pub result: PushResult,
}

impl NandiEvent for RemotePushCompleted {
    const NAME: &'static str = REMOTE_PUSH_COMPLETED;
}

/// `remote://pull_completed`
#[derive(Debug, Clone, Serialize)]
pub struct RemotePullCompleted {
    #[serde(flatten)]
    pub result: PullResult,
}

impl NandiEvent for RemotePullCompleted {
    const NAME: &'static str = REMOTE_PULL_COMPLETED;
}

/// `remote://error`: an operation failed (the message never holds a
/// credential)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFailed {
    /// `push`, `pull`, `test`, …
    pub operation: String,
    pub message: String,
}

impl NandiEvent for RemoteFailed {
    const NAME: &'static str = REMOTE_ERROR;
}

fn is_protected(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "main"
        || lower == "master"
        || lower.starts_with("main/")
        || lower.starts_with("master/")
}

/// Refuses `main`, `master`, `main/…`, `master/…`, blank names, and
/// anything outside `<prefix>/` (or a prefix that is itself protected or
/// blank). There is deliberately no way around it.
pub fn assert_pushable_branch(branch: &str, prefix: &str) -> Result<(), RemoteError> {
    if branch.trim().is_empty() {
        return Err("Refusing to push: no branch name".to_string());
    }
    if prefix.trim().is_empty() || prefix != prefix.trim() || is_protected(prefix) {
        return Err(format!(
            "Refusing to push: '{prefix}' can't be the branch prefix"
        ));
    }
    if branch != branch.trim() || is_protected(branch) {
        return Err(format!(
            "Refusing to push to '{branch}': Nandi never pushes to main or master"
        ));
    }
    let Some(rest) = branch.strip_prefix(&format!("{prefix}/")) else {
        return Err(format!(
            "Refusing to push to '{branch}': Nandi only pushes to {prefix}/<your slug>"
        ));
    };
    if rest.trim().is_empty() || !git2::Reference::is_valid_name(&format!("refs/heads/{branch}")) {
        return Err(format!(
            "Refusing to push to '{branch}': not a valid branch name"
        ));
    }
    Ok(())
}

/// Any git URL: `https://`, `http://`, `ssh://`, `git://`, `file://`, an
/// scp-like `user@host:path`, or the path of an existing git repo. A URL
/// with a password in it is refused: the credential belongs in the
/// keychain, not in `config.json`.
pub(crate) fn validate_git_url(url: &str) -> Result<(), RemoteError> {
    let invalid = || {
        format!(
            "'{url}' isn't a git URL: use https://…, ssh://…, user@host:path, or the path of a git repository"
        )
    };
    if url.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    for scheme in ["https://", "http://", "ssh://", "git://", "file://"] {
        let Some(rest) = url
            .get(..scheme.len())
            .filter(|s| s.eq_ignore_ascii_case(scheme))
            .map(|_| &url[scheme.len()..])
        else {
            continue;
        };
        let authority = rest.split('/').next().unwrap_or_default();
        if scheme != "file://" && authority.is_empty() {
            return Err(invalid());
        }
        if rest.trim_matches('/').is_empty() {
            return Err(invalid());
        }
        if authority
            .rsplit_once('@')
            .is_some_and(|(userinfo, _)| userinfo.contains(':'))
        {
            return Err(
                "Don't put a password or token in the URL: add it as the credential instead"
                    .to_string(),
            );
        }
        return Ok(());
    }
    // scp-like: user@host:path
    if let Some((user_host, path)) = url.split_once(':') {
        if let Some((user, host)) = user_host.split_once('@') {
            let ok_part = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            };
            if ok_part(user) && ok_part(host) && !path.is_empty() {
                return Ok(());
            }
        }
    }
    let path = Path::new(url);
    if path.is_dir() && git2::Repository::open(path).is_ok() {
        return Ok(());
    }
    Err(invalid())
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// The `RemoteService`. Holds handles, not `AppState`, so its slot in
/// `AppState` makes no cycle.
pub struct GitRemoteService {
    bus: EventBus,
    secrets: Secrets,
    store: RepoStore,
    /// Where `config.json` lives
    nandi_dir: PathBuf,
    /// One git operation on `~/.nandi/repo` at a time
    lock: Mutex<()>,
}

impl GitRemoteService {
    pub fn new(bus: EventBus, secrets: Secrets, store: RepoStore, nandi_dir: PathBuf) -> Self {
        Self {
            bus,
            secrets,
            store,
            nandi_dir,
            lock: Mutex::new(()),
        }
    }

    fn root(&self) -> PathBuf {
        self.store.layout().root.clone()
    }

    fn config(&self) -> Result<Option<RemoteConfig>, RemoteError> {
        Ok(nandi_config::load_from(&self.nandi_dir)?.remote)
    }

    fn require_config(&self) -> Result<RemoteConfig, RemoteError> {
        self.config()?
            .ok_or_else(|| "No remote is configured: add one in Settings › Backup".to_string())
    }

    /// The stored credential; a `Github` one carries the sign-in's token
    /// (none once signed out, and the remote then says to sign in)
    fn credential(&self) -> Result<Option<RemoteCredential>, RemoteError> {
        Ok(credentials::load(&self.secrets)?.map(|mut credential| {
            if credential.kind == CredentialKind::Github {
                credential.token = auth::github::token(&self.secrets);
            }
            credential
        }))
    }

    /// `<prefix>/<slug>`, checked
    fn target(config: &RemoteConfig) -> Result<String, RemoteError> {
        let target = format!("{}/{}", config.branch_prefix, config.user_slug);
        assert_pushable_branch(&target, &config.branch_prefix)?;
        Ok(target)
    }

    fn emit_status(&self, configured: bool) {
        bus_events::emit(&self.bus, &RemoteStatusChanged { configured });
    }

    fn emit_error(&self, operation: &str, message: &str) {
        bus_events::emit(
            &self.bus,
            &RemoteFailed {
                operation: operation.to_string(),
                message: message.to_string(),
            },
        );
    }

    /// Reports a failure on `remote://error` and passes it on
    fn reported<T>(
        &self,
        operation: &str,
        result: Result<T, RemoteError>,
    ) -> Result<T, RemoteError> {
        if let Err(e) = &result {
            self.emit_error(operation, e);
        }
        result
    }

    async fn blocking<T, F>(f: F) -> Result<T, RemoteError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, RemoteError> + Send + 'static,
    {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| format!("Remote task failed: {e}"))?
    }

    pub async fn get_status(&self) -> Result<RemoteStatus, RemoteError> {
        let root = self.root();
        let Some(config) = self.config()? else {
            let target_branch = auth::user_slug_at(&root)
                .map(|slug| format!("{DEFAULT_BRANCH_PREFIX}/{slug}"))
                .unwrap_or_default();
            let local_head = Self::blocking(move || Ok(local_head(&root))).await?;
            return Ok(RemoteStatus {
                configured: false,
                config: None,
                reachable: false,
                target_branch,
                local_head,
                remote_head: None,
                ahead: 0,
                behind: 0,
                conflicts: false,
                conflicted_files: Vec::new(),
            });
        };
        let target = Self::target(&config)?;
        let credential = self.credential()?;
        let _guard = self.lock.lock().await;
        let branch = target.clone();
        let local =
            Self::blocking(move || git_ops::status(&root, credential.as_ref(), &branch)).await?;
        Ok(RemoteStatus {
            configured: true,
            config: Some(config),
            reachable: local.reachable,
            target_branch: target,
            local_head: local.local_head,
            remote_head: local.remote_head,
            ahead: local.ahead,
            behind: local.behind,
            conflicts: local.conflicts,
            conflicted_files: local.conflicted_files,
        })
    }

    pub async fn configure(
        &self,
        url: String,
        credential: Option<RemoteCredential>,
        prefix: Option<String>,
        slug: Option<String>,
    ) -> Result<RemoteStatus, RemoteError> {
        let url = url.trim().to_string();
        validate_git_url(&url)?;
        let root = self.root();
        let current = self.config()?;
        let prefix = prefix
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .or_else(|| current.as_ref().map(|c| c.branch_prefix.clone()))
            .unwrap_or_else(|| DEFAULT_BRANCH_PREFIX.to_string());
        let identity_slug = auth::user_slug_at(&root);
        let slug = slug
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| identity_slug.clone())
            .or_else(|| current.as_ref().map(|c| c.user_slug.clone()))
            .filter(|s| !s.is_empty())
            .ok_or("Set your user slug first: it names the branch Nandi pushes to")?;
        auth::validate_slug(&slug)?;
        assert_pushable_branch(&format!("{prefix}/{slug}"), &prefix)?;

        if let Some(credential) = &credential {
            if credential.kind == CredentialKind::Github {
                if !auth::github::is_github_url(&url) {
                    return Err(
                        "The GitHub sign-in works with https://github.com/... addresses"
                            .to_string(),
                    );
                }
                if auth::github::token(&self.secrets).is_none() {
                    return Err("Sign in to GitHub first".to_string());
                }
            }
            let stored = RemoteCredential {
                // The sign-in's token stays in its own entry
                token: credential
                    .token
                    .clone()
                    .filter(|_| credential.kind != CredentialKind::Github),
                ..credential.clone()
            };
            credentials::save(&self.secrets, &stored)?;
        }
        let stored = self.credential()?;
        {
            let _guard = self.lock.lock().await;
            let repo_root = root.clone();
            let remote_url = url.clone();
            Self::blocking(move || git_ops::set_remote(&repo_root, &remote_url)).await?;
        }
        let same_target = current
            .as_ref()
            .is_some_and(|c| c.url == url && c.branch_prefix == prefix && c.user_slug == slug);
        let config = RemoteConfig {
            url,
            has_credentials: stored.is_some(),
            credential_kind: stored.as_ref().map_or(CredentialKind::None, |c| c.kind),
            branch_prefix: prefix,
            user_slug: slug.clone(),
            last_push_at: current
                .as_ref()
                .filter(|_| same_target)
                .and_then(|c| c.last_push_at),
            last_pull_at: current
                .as_ref()
                .filter(|_| same_target)
                .and_then(|c| c.last_pull_at),
            last_push_commit: current
                .as_ref()
                .filter(|_| same_target)
                .and_then(|c| c.last_push_commit.clone()),
        };
        nandi_config::update_in(&self.nandi_dir, |c| c.remote = Some(config))?;
        // The identity's slug follows the remote's
        if identity_slug.as_deref() != Some(slug.as_str()) {
            auth::write_slug_at(&root, &slug)?;
            auth::emit_changed_at(&self.bus, &root, &self.nandi_dir);
        }
        self.emit_status(true);
        self.get_status().await
    }

    pub async fn clear(&self) -> Result<(), RemoteError> {
        credentials::clear(&self.secrets)?;
        {
            let _guard = self.lock.lock().await;
            let root = self.root();
            Self::blocking(move || git_ops::remove_remote(&root)).await?;
        }
        nandi_config::update_in(&self.nandi_dir, |c| c.remote = None)?;
        self.emit_status(false);
        Ok(())
    }

    pub async fn test_connection(&self) -> Result<(), RemoteError> {
        let result = async {
            self.require_config()?;
            let credential = self.credential()?;
            let root = self.root();
            Self::blocking(move || git_ops::test_connection(&root, credential.as_ref())).await
        }
        .await;
        self.reported("test", result)
    }

    pub async fn push(&self, overwrite: bool) -> Result<PushResult, RemoteError> {
        let result = self.push_inner(overwrite).await;
        let result = self.reported("push", result)?;
        bus_events::emit(
            &self.bus,
            &RemotePushCompleted {
                result: result.clone(),
            },
        );
        self.emit_status(true);
        Ok(result)
    }

    async fn push_inner(&self, overwrite: bool) -> Result<PushResult, RemoteError> {
        let config = self.require_config()?;
        let target = Self::target(&config)?;
        assert_pushable_branch(&target, &config.branch_prefix)?;
        let credential = self.credential()?;
        let root = self.root();
        let prefix = config.branch_prefix.clone();
        let owner = config.user_slug.clone();
        let result = {
            let _guard = self.lock.lock().await;
            Self::blocking(move || {
                // Push what's on disk, not only what was committed by hand
                git_ops::commit_local_changes(&root, Some(&owner))?;
                git_ops::push(&root, credential.as_ref(), &target, &prefix, overwrite)
            })
            .await?
        };
        if !result.conflict {
            let commit = result.remote_head.clone();
            nandi_config::update_in(&self.nandi_dir, |c| {
                if let Some(remote) = c.remote.as_mut() {
                    remote.last_push_at = Some(crate::db::ts(crate::db::now_ms()));
                    remote.last_push_commit = Some(commit);
                }
            })?;
        }
        Ok(result)
    }

    pub async fn pull(&self) -> Result<PullResult, RemoteError> {
        let result = self.pull_inner().await;
        let result = self.reported("pull", result)?;
        bus_events::emit(
            &self.bus,
            &RemotePullCompleted {
                result: result.clone(),
            },
        );
        self.emit_status(true);
        Ok(result)
    }

    async fn pull_inner(&self) -> Result<PullResult, RemoteError> {
        let config = self.require_config()?;
        let target = Self::target(&config)?;
        let credential = self.credential()?;
        let layout = self.store.layout().clone();
        let owner = config.user_slug.clone();
        let result = {
            let _guard = self.lock.lock().await;
            Self::blocking(move || {
                git_ops::pull(&layout, credential.as_ref(), &target, Some(&owner))
            })
            .await?
        };
        nandi_config::update_in(&self.nandi_dir, |c| {
            if let Some(remote) = c.remote.as_mut() {
                remote.last_pull_at = Some(crate::db::ts(crate::db::now_ms()));
            }
        })?;
        if !result.conflicts && result.pulled_commits > 0 {
            // Files changed under the store: re-project them as git's doing
            let store = self.store.clone();
            Self::blocking(move || {
                store.rescan_all(ChangedBy::Git);
                Ok(())
            })
            .await?;
        }
        Ok(result)
    }

    pub async fn list_remote_branches(&self) -> Result<Vec<String>, RemoteError> {
        self.require_config()?;
        let credential = self.credential()?;
        let root = self.root();
        Self::blocking(move || git_ops::list_remote_branches(&root, credential.as_ref())).await
    }
}

/// HEAD's full SHA; empty for an empty (or missing) repo
fn local_head(root: &Path) -> String {
    git2::Repository::open(root)
        .ok()
        .and_then(|repo| repo.head().ok().and_then(|h| h.target()))
        .map(|oid| oid.to_string())
        .unwrap_or_default()
}

impl RemoteService for GitRemoteService {
    fn get_status(&self) -> BoxFuture<'_, Result<RemoteStatus, RemoteError>> {
        Box::pin(GitRemoteService::get_status(self))
    }

    fn configure(
        &self,
        url: String,
        credential: Option<RemoteCredential>,
        prefix: Option<String>,
        slug: Option<String>,
    ) -> BoxFuture<'_, Result<RemoteStatus, RemoteError>> {
        Box::pin(GitRemoteService::configure(
            self, url, credential, prefix, slug,
        ))
    }

    fn clear(&self) -> BoxFuture<'_, Result<(), RemoteError>> {
        Box::pin(GitRemoteService::clear(self))
    }

    fn test_connection(&self) -> BoxFuture<'_, Result<(), RemoteError>> {
        Box::pin(GitRemoteService::test_connection(self))
    }

    fn push(&self, overwrite: bool) -> BoxFuture<'_, Result<PushResult, RemoteError>> {
        Box::pin(GitRemoteService::push(self, overwrite))
    }

    fn pull(&self) -> BoxFuture<'_, Result<PullResult, RemoteError>> {
        Box::pin(GitRemoteService::pull(self))
    }

    fn list_remote_branches(&self) -> BoxFuture<'_, Result<Vec<String>, RemoteError>> {
        Box::pin(GitRemoteService::list_remote_branches(self))
    }
}

pub async fn get_remote_status(state: &AppState) -> Result<RemoteStatus, RemoteError> {
    state.remote().get_status().await
}

pub async fn configure_remote(
    state: &AppState,
    url: String,
    credential: Option<RemoteCredential>,
    prefix: Option<String>,
    slug: Option<String>,
) -> Result<RemoteStatus, RemoteError> {
    state
        .remote()
        .configure(url, credential, prefix, slug)
        .await
}

pub async fn clear_remote(state: &AppState) -> Result<(), RemoteError> {
    state.remote().clear().await
}

pub async fn test_remote_connection(state: &AppState) -> Result<(), RemoteError> {
    state.remote().test_connection().await
}

pub async fn push_to_remote(state: &AppState, overwrite: bool) -> Result<PushResult, RemoteError> {
    state.remote().push(overwrite).await
}

pub async fn pull_from_remote(state: &AppState) -> Result<PullResult, RemoteError> {
    state.remote().pull().await
}

pub async fn list_remote_branches(state: &AppState) -> Result<Vec<String>, RemoteError> {
    state.remote().list_remote_branches().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_main_and_master_with_no_way_around() {
        for (branch, prefix) in [
            ("main", "nandi"),
            ("master", "nandi"),
            ("MAIN", "nandi"),
            ("Master", "nandi"),
            ("main/x", "nandi"),
            ("master/x", "nandi"),
            ("MAIN/alice", "MAIN"),
            ("", "nandi"),
            ("  ", "nandi"),
            (" nandi/alice", "nandi"),
            ("nandi/alice ", "nandi"),
            ("other/alice", "nandi"),
            ("nandi", "nandi"),
            ("nandi/", "nandi"),
            ("nandi/a..b", "nandi"),
            ("nandi/a b", "nandi"),
            ("refs/heads/main", "nandi"),
            ("nandix/alice", "nandi"),
            // A protected or blank prefix is refused outright
            ("main/alice", "main"),
            ("master/alice", "master"),
            ("/alice", ""),
            ("alice", ""),
            ("  /alice", "  "),
        ] {
            assert!(
                assert_pushable_branch(branch, prefix).is_err(),
                "{branch:?} with prefix {prefix:?}"
            );
        }
        assert!(assert_pushable_branch("nandi/alice", "nandi").is_ok());
        assert!(assert_pushable_branch("nandi/alice-chen", "nandi").is_ok());
        // The slug "main" is fine under a safe prefix
        assert!(assert_pushable_branch("nandi/main", "nandi").is_ok());
        assert!(assert_pushable_branch("team/alice", "team").is_ok());
    }

    #[test]
    fn accepts_only_git_urls() {
        let tmp = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(tmp.path().join("bare.git")).unwrap();
        std::fs::create_dir_all(tmp.path().join("plain")).unwrap();
        let bare = tmp.path().join("bare.git").to_string_lossy().into_owned();
        for ok in [
            "https://github.com/acme/agents.git",
            "http://git.local/agents",
            "ssh://git@host.example/agents.git",
            "git://host/agents.git",
            "file:///srv/git/agents.git",
            "git@github.com:acme/agents.git",
            "HTTPS://example.com/x.git",
            "https://user@example.com/x.git",
            &bare,
        ] {
            assert!(validate_git_url(ok).is_ok(), "{ok}");
        }
        let plain = tmp.path().join("plain").to_string_lossy().into_owned();
        for bad in [
            "",
            "not a url",
            "ftp://example.com/x",
            "https://",
            "https:///path",
            "github.com/acme/agents",
            "acme/agents",
            "https://user:token@example.com/x.git",
            "@host:path",
            "user@:path",
            &plain,
        ] {
            assert!(validate_git_url(bad).is_err(), "{bad}");
        }
    }

    fn bare_remote(dir: &Path) -> String {
        let mut options = git2::RepositoryInitOptions::new();
        options.bare(true).initial_head("main");
        git2::Repository::init_opts(dir, &options).unwrap();
        dir.to_string_lossy().into_owned()
    }

    async fn state_with_identity() -> AppState {
        let state = AppState::for_tests(vec![]);
        auth::set_display_name(&state, "Alice Chen".into())
            .await
            .unwrap();
        let root = state.store().layout().root.clone();
        let repo = git_ops::open_or_init(&root).unwrap();
        crate::services::repo_bootstrap::commit_all(&repo, "Setup", Some("alice")).unwrap();
        state
    }

    #[tokio::test]
    async fn configure_validates_url_slug_and_prefix() {
        let state = state_with_identity().await;
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(&tmp.path().join("remote.git"));

        assert!(configure_remote(&state, "nope".into(), None, None, None)
            .await
            .is_err());
        assert!(
            configure_remote(&state, url.clone(), None, None, Some("Bad Slug".into()))
                .await
                .is_err()
        );
        assert!(
            configure_remote(&state, url.clone(), None, Some("main".into()), None)
                .await
                .is_err()
        );
        assert!(
            configure_remote(&state, url.clone(), None, Some("MASTER".into()), None)
                .await
                .is_err()
        );
        // Nothing was saved by the refusals
        assert!(!get_remote_status(&state).await.unwrap().configured);

        let credential = RemoteCredential {
            kind: CredentialKind::Token,
            token: Some("secret-token".into()),
            username: None,
            ssh_key_path: None,
        };
        let status = configure_remote(&state, url.clone(), Some(credential), None, None)
            .await
            .unwrap();
        assert!(status.configured && status.reachable);
        assert_eq!(status.target_branch, "nandi/alice-chen");
        let config = status.config.unwrap();
        assert!(config.has_credentials);
        assert_eq!(config.credential_kind, CredentialKind::Token);
        // The secret is in the keychain only
        let saved = std::fs::read_to_string(state.nandi_dir().join("config.json")).unwrap();
        assert!(!saved.contains("secret-token"));
        assert!(saved.contains("alice-chen"));

        // A slug given here becomes the identity's too
        configure_remote(&state, url, None, None, Some("alice".into()))
            .await
            .unwrap();
        let identity = auth::get_identity_status(&state).await.unwrap();
        assert_eq!(identity.branch.as_deref(), Some("nandi/alice"));

        clear_remote(&state).await.unwrap();
        assert!(!get_remote_status(&state).await.unwrap().configured);
        assert!(credentials::load(state.secrets()).unwrap().is_none());
        assert!(push_to_remote(&state, false).await.is_err());
    }

    #[tokio::test]
    async fn push_and_pull_through_the_service() {
        let state = state_with_identity().await;
        let tmp = tempfile::tempdir().unwrap();
        let url = bare_remote(&tmp.path().join("remote.git"));
        configure_remote(&state, url.clone(), None, None, None)
            .await
            .unwrap();

        let pushed = push_to_remote(&state, false).await.unwrap();
        assert!(!pushed.conflict);
        assert_eq!(pushed.target_branch, "nandi/alice-chen");
        let status = get_remote_status(&state).await.unwrap();
        assert_eq!((status.ahead, status.behind), (0, 0));
        assert_eq!(
            status.remote_head.as_deref(),
            Some(status.local_head.as_str())
        );
        let config = status.config.clone().unwrap();
        assert_eq!(config.last_push_commit, Some(pushed.remote_head.clone()));
        assert!(config.last_push_at.is_some());
        assert_eq!(
            list_remote_branches(&state).await.unwrap(),
            ["nandi/alice-chen"]
        );

        // Another machine rewrites the branch
        let other = tmp.path().join("other");
        let repo = git_ops::open_or_init(&other).unwrap();
        std::fs::write(other.join("x.md"), "x").unwrap();
        crate::services::repo_bootstrap::commit_all(&repo, "other", None).unwrap();
        git_ops::set_remote(&other, &url).unwrap();
        git_ops::push(&other, None, "nandi/alice-chen", "nandi", true).unwrap();

        let refused = push_to_remote(&state, false).await.unwrap();
        assert!(refused.conflict);
        let forced = push_to_remote(&state, true).await.unwrap();
        assert!(!forced.conflict);
        assert_eq!(forced.remote_head, status.local_head);

        let pulled = pull_from_remote(&state).await.unwrap();
        assert_eq!(pulled.pulled_commits, 0);
        assert!(get_remote_status(&state)
            .await
            .unwrap()
            .config
            .unwrap()
            .last_pull_at
            .is_some());
    }
}
