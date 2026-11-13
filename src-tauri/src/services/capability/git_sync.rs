//! Read-only git fetch for capability registries. Reuses
//! `services::remote::git_ops` (`callbacks`, credentials) instead of new
//! git2 plumbing. Never pushes: there is no push code here, and the clone
//! is only ever reset to what the remote has.
//!
//! All of this blocks: callers run it on `spawn_blocking`.

use std::fs;
use std::path::{Path, PathBuf};

use git2::build::CheckoutBuilder;
use git2::{FetchOptions, Oid, Repository};
use sha2::{Digest, Sha256};

use crate::services::remote::git_ops;

/// The remote name inside a registry clone
const REMOTE: &str = "origin";
/// Tried in order when a registry names no branch
const DEFAULT_BRANCHES: &[&str] = &["main", "master"];

/// A stable short hash of the registry URL: its row ID and clone folder
pub fn url_hash(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `<cache>/capability-registries/<hash of url>`
pub fn clone_dir(cache_dir: &Path, url: &str) -> PathBuf {
    cache_dir
        .join(super::registry::REGISTRY_CACHE_DIR)
        .join(url_hash(url))
}

/// Clones `url` into `dir`, or fetches and resets the existing clone to the
/// remote head (hard: the clone is never edited). `branch`: `None` picks
/// `main`, then `master`, then the first branch. `token` (the GitHub
/// sign-in's, for a github.com registry) is the password of
/// `x-access-token`; without one, git's own credentials. Returns the head
/// commit.
pub fn clone_or_pull(
    url: &str,
    dir: &Path,
    branch: Option<&str>,
    token: Option<&str>,
) -> Result<String, String> {
    let repo = open_or_init(dir)?;
    match repo.find_remote(REMOTE) {
        Ok(remote) if remote.url() == Some(url) => {}
        Ok(_) => repo.remote_set_url(REMOTE, url).map_err(git_error)?,
        Err(_) => {
            repo.remote(REMOTE, url).map_err(git_error)?;
        }
    }
    let mut remote = repo.find_remote(REMOTE).map_err(git_error)?;
    let mut options = FetchOptions::new();
    options.remote_callbacks(git_ops::callbacks(token.map(git_ops::token_credential)));
    options.prune(git2::FetchPrune::On);
    let refspec = format!("+refs/heads/*:refs/remotes/{REMOTE}/*");
    remote
        .fetch(&[refspec.as_str()], Some(&mut options), None)
        .map_err(|e| format!("Failed to fetch {url}: {}", e.message()))?;
    let _ = remote.disconnect();

    let head = pick_branch(&repo, branch)?;
    let commit = repo.find_commit(head).map_err(git_error)?;
    repo.checkout_tree(
        commit.as_object(),
        Some(CheckoutBuilder::new().force().remove_untracked(true)),
    )
    .and_then(|()| repo.set_head_detached(head))
    .map_err(git_error)?;
    Ok(head.to_string())
}

/// Deletes a clone. Git marks pack files read-only, which Windows refuses
/// to delete, so that is cleared first. A missing folder is fine.
pub fn remove_clone(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    clear_readonly(dir);
    fs::remove_dir_all(dir).map_err(|e| format!("Failed to remove {}: {e}", dir.display()))
}

fn open_or_init(dir: &Path) -> Result<Repository, String> {
    if let Ok(repo) = Repository::open(dir) {
        return Ok(repo);
    }
    // Not a repo (an interrupted first clone): start over
    remove_clone(dir)?;
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    Repository::init(dir).map_err(git_error)
}

fn pick_branch(repo: &Repository, branch: Option<&str>) -> Result<Oid, String> {
    let tracking = |name: &str| {
        repo.find_reference(&format!("refs/remotes/{REMOTE}/{name}"))
            .ok()
            .and_then(|r| r.target())
    };
    if let Some(name) = branch {
        return tracking(name).ok_or_else(|| format!("The registry has no branch '{name}'"));
    }
    if let Some(oid) = DEFAULT_BRANCHES.iter().find_map(|name| tracking(name)) {
        return Ok(oid);
    }
    let prefix = format!("refs/remotes/{REMOTE}/");
    let head = format!("{prefix}HEAD");
    let mut others: Vec<(String, Oid)> = repo
        .references_glob(&format!("{prefix}*"))
        .map_err(git_error)?
        .flatten()
        .filter_map(|r| Some((r.name()?.to_string(), r.target()?)))
        .filter(|(name, _)| *name != head)
        .collect();
    others.sort();
    others
        .into_iter()
        .next()
        .map(|(_, oid)| oid)
        .ok_or_else(|| "The registry has no branches".to_string())
}

fn clear_readonly(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            clear_readonly(&path);
        } else if let Ok(metadata) = entry.metadata() {
            let mut permissions = metadata.permissions();
            if permissions.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = fs::set_permissions(&path, permissions);
            }
        }
    }
}

fn git_error(e: git2::Error) -> String {
    e.message().to_string()
}
