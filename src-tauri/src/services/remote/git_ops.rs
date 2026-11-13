//! git2 operations against the remote: test
//! connection, push to `<prefix>/<slug>` (fast-forward only unless
//! overwriting), pull (fast-forward, else a 3-way merge left in progress on
//! conflict), list branches. Every push calls
//! `super::assert_pushable_branch` first.
//!
//! The remote is a named git remote in `~/.nandi/repo` ([`REMOTE_NAME`]);
//! only the target branch is ever fetched or pushed. Credentials go only
//! through callbacks, never into URLs or git config. All of this blocks:
//! callers run it on `spawn_blocking`.

use std::cell::RefCell;
use std::fs;
use std::path::Path;

use git2::build::CheckoutBuilder;
use git2::{
    Cred, CredentialType, ErrorCode, FetchOptions, Oid, PushOptions, RemoteCallbacks, Repository,
    RepositoryState, StatusOptions,
};

use super::{assert_pushable_branch, CredentialKind, PullResult, PushResult, RemoteCredential};
use crate::services::repo_bootstrap;
use crate::services::repos::RepoLayout;

/// The git remote Nandi manages in `~/.nandi/repo` (`origin` stays the
/// user's)
pub const REMOTE_NAME: &str = "nandi-remote";

/// What `status` can tell without the config
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalStatus {
    pub reachable: bool,
    pub local_head: String,
    pub remote_head: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub conflicts: bool,
    pub conflicted_files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

pub fn open_or_init(root: &Path) -> Result<Repository, String> {
    if let Ok(repo) = Repository::open(root) {
        return Ok(repo);
    }
    fs::create_dir_all(root).map_err(|e| format!("Failed to create {}: {e}", root.display()))?;
    let mut options = git2::RepositoryInitOptions::new();
    options.initial_head("main");
    Repository::init_opts(root, &options).map_err(git_error)
}

/// Points [`REMOTE_NAME`] at `url`
pub fn set_remote(root: &Path, url: &str) -> Result<(), String> {
    let repo = open_or_init(root)?;
    let current = repo
        .find_remote(REMOTE_NAME)
        .ok()
        .map(|remote| remote.url().map(str::to_string));
    match current {
        Some(Some(existing)) if existing == url => Ok(()),
        Some(_) => repo.remote_set_url(REMOTE_NAME, url).map_err(git_error),
        None => repo.remote(REMOTE_NAME, url).map(|_| ()).map_err(git_error),
    }
}

/// Removes [`REMOTE_NAME`] (and its tracking refs); a missing one is fine
pub fn remove_remote(root: &Path) -> Result<(), String> {
    let Ok(repo) = Repository::open(root) else {
        return Ok(());
    };
    if repo.find_remote(REMOTE_NAME).is_err() {
        return Ok(());
    }
    repo.remote_delete(REMOTE_NAME).map_err(git_error)
}

// ---------------------------------------------------------------------------
// Remote queries
// ---------------------------------------------------------------------------

/// The remote's branches as `(refs/heads/<name>, oid)`, by fetching every
/// branch into `refs/remotes/nandi-remote/*` (pruned, so the tracking refs
/// mirror the remote exactly).
///
/// Not `connect` + `list`: git2 0.20 builds a slice from a null pointer
/// when the remote advertises nothing (an empty repo), which aborts.
fn ls_remote(
    repo: &Repository,
    credential: Option<&RemoteCredential>,
) -> Result<Vec<(String, Oid)>, String> {
    let mut remote = repo.find_remote(REMOTE_NAME).map_err(|_| no_remote())?;
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(credential.cloned()));
    options.prune(git2::FetchPrune::On);
    let refspec = format!("+refs/heads/*:refs/remotes/{REMOTE_NAME}/*");
    remote
        .fetch(&[refspec.as_str()], Some(&mut options), None)
        .map_err(git_error)?;
    let _ = remote.disconnect();
    let prefix = format!("refs/remotes/{REMOTE_NAME}/");
    let mut refs = Vec::new();
    for reference in repo
        .references_glob(&format!("{prefix}*"))
        .map_err(git_error)?
        .flatten()
    {
        let (Some(name), Some(oid)) = (reference.name(), reference.target()) else {
            continue;
        };
        if let Some(branch) = name.strip_prefix(&prefix) {
            if branch != "HEAD" {
                refs.push((format!("refs/heads/{branch}"), oid));
            }
        }
    }
    Ok(refs)
}

fn remote_branch_oid(refs: &[(String, Oid)], target: &str) -> Option<Oid> {
    let name = format!("refs/heads/{target}");
    refs.iter().find(|(n, _)| *n == name).map(|(_, oid)| *oid)
}

pub fn test_connection(root: &Path, credential: Option<&RemoteCredential>) -> Result<(), String> {
    let repo = Repository::open(root).map_err(git_error)?;
    ls_remote(&repo, credential).map(|_| ())
}

/// Branch names on the remote, sorted
pub fn list_remote_branches(
    root: &Path,
    credential: Option<&RemoteCredential>,
) -> Result<Vec<String>, String> {
    let repo = Repository::open(root).map_err(git_error)?;
    let mut branches: Vec<String> = ls_remote(&repo, credential)?
        .into_iter()
        .filter_map(|(name, _)| name.strip_prefix("refs/heads/").map(str::to_string))
        .collect();
    branches.sort();
    branches.dedup();
    Ok(branches)
}

fn tracking_ref(target: &str) -> String {
    format!("refs/remotes/{REMOTE_NAME}/{target}")
}

fn head_oid(repo: &Repository) -> Option<Oid> {
    repo.head().ok().and_then(|h| h.target())
}

/// `refs/heads/<branch>` HEAD points at (even when unborn)
fn head_ref_name(repo: &Repository) -> String {
    repo.find_reference("HEAD")
        .ok()
        .and_then(|head| head.symbolic_target().map(str::to_string))
        .unwrap_or_else(|| "refs/heads/main".to_string())
}

fn count_commits(repo: &Repository, from: Oid) -> u32 {
    let Ok(mut walk) = repo.revwalk() else {
        return 0;
    };
    if walk.push(from).is_err() {
        return 0;
    }
    walk.count() as u32
}

fn conflicted_files(repo: &Repository) -> Result<Vec<String>, String> {
    let index = repo.index().map_err(git_error)?;
    if !index.has_conflicts() {
        return Ok(Vec::new());
    }
    let mut files: Vec<String> = index
        .conflicts()
        .map_err(git_error)?
        .flatten()
        .filter_map(|c| c.our.or(c.their).or(c.ancestor))
        .map(|entry| String::from_utf8_lossy(&entry.path).into_owned())
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// Commits whatever isn't committed yet (as pull does before merging), so
/// a push carries the agents and NANDI.md as they are on disk; whether it
/// committed anything
pub fn commit_local_changes(root: &Path, owner: Option<&str>) -> Result<bool, String> {
    let repo = Repository::open(root).map_err(git_error)?;
    if repo.state() != RepositoryState::Clean || !has_local_changes(&repo)? {
        return Ok(false);
    }
    repo_bootstrap::commit_all(&repo, "Local changes before pushing", owner)?;
    Ok(true)
}

/// Uncommitted changes (staged, unstaged, untracked; not ignored)
fn has_local_changes(repo: &Repository) -> Result<bool, String> {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    Ok(!repo
        .statuses(Some(&mut options))
        .map_err(git_error)?
        .is_empty())
}

/// Ahead/behind against the remote branch; the remote's state is read live
/// (`reachable` false and the last fetched state when it can't be reached)
pub fn status(
    root: &Path,
    credential: Option<&RemoteCredential>,
    target: &str,
) -> Result<LocalStatus, String> {
    let repo = open_or_init(root)?;
    let head = head_oid(&repo);
    let mut status = LocalStatus {
        local_head: head.map(|oid| oid.to_string()).unwrap_or_default(),
        conflicted_files: conflicted_files(&repo)?,
        ..LocalStatus::default()
    };
    status.conflicts = !status.conflicted_files.is_empty();
    let tracked = repo
        .find_reference(&tracking_ref(target))
        .ok()
        .and_then(|r| r.target());
    let remote_oid = match ls_remote(&repo, credential) {
        Ok(refs) => {
            status.reachable = true;
            remote_branch_oid(&refs, target)
        }
        Err(e) => {
            log::info!("The remote is unreachable: {e}");
            tracked
        }
    };
    status.remote_head = remote_oid.map(|oid| oid.to_string());
    match (head, remote_oid) {
        (Some(head), Some(remote)) => {
            // Known locally once fetched; else compare with the last fetch
            let known = if repo.find_commit(remote).is_ok() {
                Some(remote)
            } else {
                tracked
            };
            if let Some(known) = known {
                if let Ok((ahead, behind)) = repo.graph_ahead_behind(head, known) {
                    status.ahead = ahead as u32;
                    status.behind = behind as u32;
                }
            }
        }
        (Some(head), None) => status.ahead = count_commits(&repo, head),
        (None, Some(_)) => status.behind = 1,
        (None, None) => {}
    }
    Ok(status)
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

/// Pushes HEAD to `refs/heads/<target>`: fast-forward only, unless
/// `overwrite` (force). A non-fast-forward without `overwrite` pushes
/// nothing and reports `conflict`.
pub fn push(
    root: &Path,
    credential: Option<&RemoteCredential>,
    target: &str,
    prefix: &str,
    overwrite: bool,
) -> Result<PushResult, String> {
    // Load-bearing: never main, never master, never outside `<prefix>/`
    assert_pushable_branch(target, prefix)?;
    let repo = Repository::open(root).map_err(git_error)?;
    let head = head_oid(&repo).ok_or("Nothing to push yet: commit your changes first")?;
    let refs = ls_remote(&repo, credential)?;
    let remote_oid = remote_branch_oid(&refs, target);

    let pushed_commits = match remote_oid {
        Some(remote) if remote == head => {
            return Ok(PushResult {
                pushed_commits: 0,
                target_branch: target.to_string(),
                remote_head: remote.to_string(),
                conflict: false,
            });
        }
        Some(remote) => {
            let fast_forward = repo.graph_descendant_of(head, remote).map_err(git_error)?;
            if !fast_forward && !overwrite {
                return Ok(PushResult {
                    pushed_commits: 0,
                    target_branch: target.to_string(),
                    remote_head: remote.to_string(),
                    conflict: true,
                });
            }
            repo.graph_ahead_behind(head, remote)
                .map(|(ahead, _)| ahead as u32)
                .map_err(git_error)?
        }
        None => count_commits(&repo, head),
    };

    let mut remote = repo.find_remote(REMOTE_NAME).map_err(|_| no_remote())?;
    let rejected = RefCell::new(None::<String>);
    let mut callbacks = callbacks(credential.cloned());
    callbacks.push_update_reference(|reference, status| {
        if let Some(status) = status {
            *rejected.borrow_mut() = Some(format!("{reference}: {status}"));
        }
        Ok(())
    });
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    let plus = if overwrite { "+" } else { "" };
    let refspec = format!("{plus}HEAD:refs/heads/{target}");
    // Checked again right where the refspec is built
    assert_pushable_branch(target, prefix)?;
    remote
        .push(&[refspec.as_str()], Some(&mut options))
        .map_err(git_error)?;
    drop(options);
    if let Some(reason) = rejected.into_inner() {
        return Err(format!(
            "The remote rejected the push ({reason}). Pull and try again."
        ));
    }
    repo.reference(&tracking_ref(target), head, true, "nandi: pushed")
        .map_err(git_error)?;
    Ok(PushResult {
        pushed_commits,
        target_branch: target.to_string(),
        remote_head: head.to_string(),
        conflict: false,
    })
}

// ---------------------------------------------------------------------------
// Pull
// ---------------------------------------------------------------------------

/// Brings in `refs/heads/<target>`. Local changes are committed first.
/// Fast-forwards when it can, else merges; a conflicting merge is left in
/// progress (conflicts in the index, `MERGE_HEAD` set) and nothing is
/// resolved automatically. A fresh install (only Nandi's initial commit)
/// takes the remote's history and adopts its layout.
pub fn pull(
    layout: &RepoLayout,
    credential: Option<&RemoteCredential>,
    target: &str,
    owner: Option<&str>,
) -> Result<PullResult, String> {
    let root = &layout.root;
    let repo = open_or_init(root)?;
    if repo.state() != RepositoryState::Clean || repo.index().is_ok_and(|i| i.has_conflicts()) {
        return Err(
            "A merge is in progress in ~/.nandi/repo: resolve its conflicts and commit, then pull again"
                .to_string(),
        );
    }
    let refs = ls_remote(&repo, credential)?;
    let Some(upstream) = remote_branch_oid(&refs, target) else {
        return Ok(PullResult {
            pulled_commits: 0,
            conflicts: false,
            conflicted_files: Vec::new(),
        });
    };
    let upstream_commit = repo.find_commit(upstream).map_err(git_error)?;
    let branch_ref = head_ref_name(&repo);

    let initial_only = is_initial_only(&repo);
    if has_local_changes(&repo)? {
        repo_bootstrap::commit_all(&repo, "Local changes before pulling", owner)?;
    }

    let Some(head) = head_oid(&repo) else {
        // Nothing local at all: start from the remote
        repo.reference(&branch_ref, upstream, true, "nandi: start from the remote")
            .and_then(|_| repo.set_head(&branch_ref))
            .and_then(|()| repo.checkout_head(Some(CheckoutBuilder::new().force())))
            .map_err(git_error)?;
        return Ok(PullResult {
            pulled_commits: count_commits(&repo, upstream),
            conflicts: false,
            conflicted_files: Vec::new(),
        });
    };

    if initial_only && head != upstream {
        return adopt_remote(layout, &repo, head, &upstream_commit, &branch_ref, owner);
    }

    let annotated = repo.find_annotated_commit(upstream).map_err(git_error)?;
    let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(git_error)?;
    if analysis.is_up_to_date() {
        return Ok(PullResult {
            pulled_commits: 0,
            conflicts: false,
            conflicted_files: Vec::new(),
        });
    }
    let (pulled, _) = repo.graph_ahead_behind(upstream, head).map_err(git_error)?;
    let pulled_commits = pulled as u32;

    if analysis.is_fast_forward() {
        // The tree is clean (local changes were just committed)
        repo.checkout_tree(
            upstream_commit.as_object(),
            Some(CheckoutBuilder::new().force()),
        )
        .and_then(|()| repo.reference(&branch_ref, upstream, true, "nandi: fast-forward"))
        .and_then(|_| repo.set_head(&branch_ref))
        .map_err(git_error)?;
        return Ok(PullResult {
            pulled_commits,
            conflicts: false,
            conflicted_files: Vec::new(),
        });
    }

    // A 3-way merge. On conflict, git's merge state stays for the user.
    let mut checkout = CheckoutBuilder::new();
    checkout.allow_conflicts(true).conflict_style_merge(true);
    repo.merge(&[&annotated], None, Some(&mut checkout))
        .map_err(git_error)?;
    let files = conflicted_files(&repo)?;
    if !files.is_empty() {
        log::info!("Pull left a merge in progress: {}", files.join(", "));
        return Ok(PullResult {
            pulled_commits,
            conflicts: true,
            conflicted_files: files,
        });
    }
    let mut index = repo.index().map_err(git_error)?;
    let tree_id = index.write_tree().map_err(git_error)?;
    let tree = repo.find_tree(tree_id).map_err(git_error)?;
    let signature = repo_bootstrap::signature(&repo, owner).map_err(git_error)?;
    let ours = repo.find_commit(head).map_err(git_error)?;
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        &format!("Merge {target} from the remote"),
        &tree,
        &[&ours, &upstream_commit],
    )
    .map_err(git_error)?;
    repo.cleanup_state().map_err(git_error)?;
    Ok(PullResult {
        pulled_commits,
        conflicts: false,
        conflicted_files: Vec::new(),
    })
}

/// A fresh install's HEAD is Nandi's initial commit (plus, maybe, a commit
/// of onboarding's changes): nothing a merge should weigh against the
/// remote's history.
fn is_initial_only(repo: &Repository) -> bool {
    repo.head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok())
        .is_some_and(|commit| {
            commit.parent_count() == 0 && commit.message() == Some(repo_bootstrap::INITIAL_COMMIT)
        })
}

/// Takes the remote's history; files only the local side has (NANDI.md
/// written during onboarding, say) are kept, then the layout is adopted
/// and the result committed.
fn adopt_remote(
    layout: &RepoLayout,
    repo: &Repository,
    local: Oid,
    upstream: &git2::Commit<'_>,
    branch_ref: &str,
    owner: Option<&str>,
) -> Result<PullResult, String> {
    let local_tree = repo
        .find_commit(local)
        .and_then(|c| c.tree())
        .map_err(git_error)?;
    let upstream_tree = upstream.tree().map_err(git_error)?;
    let mut local_only = Vec::new();
    local_tree
        .walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let path = format!("{dir}{}", entry.name().unwrap_or_default());
                if upstream_tree.get_path(Path::new(&path)).is_err() {
                    local_only.push(path);
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(git_error)?;

    repo.reference(
        branch_ref,
        upstream.id(),
        true,
        "nandi: start from the remote",
    )
    .and_then(|_| repo.set_head(branch_ref))
    .and_then(|()| repo.checkout_head(Some(CheckoutBuilder::new().force())))
    .map_err(git_error)?;
    if !local_only.is_empty() {
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        for path in &local_only {
            checkout.path(path.as_str());
        }
        repo.checkout_tree(local_tree.as_object(), Some(&mut checkout))
            .map_err(git_error)?;
    }
    repo_bootstrap::adopt_repo(layout, owner)?;
    if has_local_changes(repo)? {
        repo_bootstrap::commit_all(repo, "Set up the Nandi repo layout", owner)?;
    }
    Ok(PullResult {
        pulled_commits: count_commits(repo, upstream.id()),
        conflicts: false,
        conflicted_files: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// A token credential (the password of `x-access-token`)
pub fn token_credential(token: &str) -> RemoteCredential {
    RemoteCredential {
        kind: CredentialKind::Token,
        token: Some(token.to_string()),
        username: None,
        ssh_key_path: None,
    }
}

/// The stored credential through libgit2's callback: a token (or the
/// GitHub sign-in's) is the password of `x-access-token` (or the given
/// username); a username and password; an SSH key file (the token, if any,
/// is its passphrase). With none, the SSH agent, git's credential helper,
/// or the platform default.
pub fn callbacks<'a>(credential: Option<RemoteCredential>) -> RemoteCallbacks<'a> {
    let mut attempts = 0u8;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |url, username_from_url, allowed| {
        attempts += 1;
        if attempts > 3 {
            return Err(git2::Error::from_str("The remote rejected the credentials"));
        }
        let username = |default: &str| {
            credential
                .as_ref()
                .and_then(|c| c.username.clone())
                .filter(|u| !u.trim().is_empty())
                .or_else(|| username_from_url.map(str::to_string))
                .unwrap_or_else(|| default.to_string())
        };
        let Some(credential) = credential.as_ref() else {
            return default_credentials(url, username_from_url, allowed);
        };
        match credential.kind {
            CredentialKind::Github if credential.token.is_none() => Err(git2::Error::from_str(
                "Sign in to GitHub (Settings › Account) to use it for this remote",
            )),
            CredentialKind::Token | CredentialKind::Github | CredentialKind::UserPass => {
                if !allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
                    return Err(git2::Error::from_str(
                        "The remote doesn't take a token or password (use an SSH key)",
                    ));
                }
                let user = if credential.kind == CredentialKind::UserPass {
                    username("git")
                } else {
                    username("x-access-token")
                };
                Cred::userpass_plaintext(&user, credential.token.as_deref().unwrap_or_default())
            }
            CredentialKind::SshKey => {
                let user = username("git");
                if allowed.contains(CredentialType::USERNAME) {
                    return Cred::username(&user);
                }
                let Some(path) = credential.ssh_key_path.as_deref() else {
                    return Err(git2::Error::from_str("No SSH key file is set"));
                };
                Cred::ssh_key(
                    &user,
                    None,
                    Path::new(path),
                    credential.token.as_deref().filter(|p| !p.is_empty()),
                )
            }
            CredentialKind::None => default_credentials(url, username_from_url, allowed),
        }
    });
    callbacks
}

fn default_credentials(
    url: &str,
    username_from_url: Option<&str>,
    allowed: CredentialType,
) -> Result<Cred, git2::Error> {
    let user = username_from_url.unwrap_or("git");
    if allowed.contains(CredentialType::USERNAME) {
        return Cred::username(user);
    }
    if allowed.contains(CredentialType::SSH_KEY) {
        return Cred::ssh_key_from_agent(user);
    }
    if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
        let config = git2::Config::open_default()?;
        return Cred::credential_helper(&config, url, username_from_url);
    }
    if allowed.contains(CredentialType::DEFAULT) {
        return Cred::default();
    }
    Err(git2::Error::from_str(
        "The remote needs a credential: add one in Settings › Backup",
    ))
}

fn no_remote() -> String {
    "No remote is configured: add one in Settings › Backup".to_string()
}

fn git_error(e: git2::Error) -> String {
    log::warn!("Remote git operation failed: {e}");
    if e.code() == ErrorCode::Auth {
        return "The remote refused the credentials. Check them in Settings › Backup.".to_string();
    }
    format!("Git error: {}", e.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::services::repos::RepoLayout;

    fn bare(path: &Path) -> String {
        let mut options = git2::RepositoryInitOptions::new();
        options.bare(true).initial_head("main");
        Repository::init_opts(path, &options).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn commit_file(root: &Path, rel: &str, content: &str, message: &str) -> Oid {
        let repo = open_or_init(root).unwrap();
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        repo_bootstrap::commit_all(&repo, message, Some("tester"))
            .unwrap()
            .unwrap()
    }

    fn layout(root: &Path) -> RepoLayout {
        RepoLayout::new(root.to_path_buf(), root.parent().unwrap().join("trash"))
    }

    const TARGET: &str = "nandi/alice";

    #[test]
    fn uncommitted_files_are_committed_before_a_push() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        commit_file(&a, "one.md", "1", "one");
        // Written by the app (staged) or by hand (untracked), never committed
        fs::create_dir_all(a.join(".nandi/agents/helper")).unwrap();
        fs::write(
            a.join(".nandi/agents/helper/skill.md"),
            "---
name: H
---
",
        )
        .unwrap();
        fs::write(
            a.join("NANDI.md"),
            "---
name: N
---
",
        )
        .unwrap();
        assert!(commit_local_changes(&a, Some("tester")).unwrap());
        let repo = Repository::open(&a).unwrap();
        assert!(!has_local_changes(&repo).unwrap());
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary(), Some("Local changes before pushing"));
        // Nothing left: no empty commit
        assert!(!commit_local_changes(&a, Some("tester")).unwrap());
    }

    #[test]
    fn push_fast_forwards_and_refuses_diverged_history_without_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare(&tmp.path().join("remote.git"));
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        commit_file(&a, "one.md", "1", "one");
        set_remote(&a, &url).unwrap();

        let first = push(&a, None, TARGET, "nandi", false).unwrap();
        assert!(!first.conflict);
        assert_eq!(first.pushed_commits, 1);
        assert_eq!(first.target_branch, TARGET);
        assert_eq!(list_remote_branches(&a, None).unwrap(), [TARGET]);

        let head = commit_file(&a, "two.md", "2", "two");
        let second = push(&a, None, TARGET, "nandi", false).unwrap();
        assert_eq!(second.pushed_commits, 1);
        assert_eq!(second.remote_head, head.to_string());

        // Another machine pushes unrelated history to the same branch
        commit_file(&b, "other.md", "x", "other");
        set_remote(&b, &url).unwrap();
        let refused = push(&b, None, TARGET, "nandi", false).unwrap();
        assert!(refused.conflict);
        assert_eq!(refused.pushed_commits, 0);
        assert_eq!(refused.remote_head, head.to_string());

        let forced = push(&b, None, TARGET, "nandi", true).unwrap();
        assert!(!forced.conflict);
        let remote = Repository::open(tmp.path().join("remote.git")).unwrap();
        let tip = remote
            .find_reference(&format!("refs/heads/{TARGET}"))
            .unwrap()
            .target()
            .unwrap();
        assert_eq!(tip.to_string(), forced.remote_head);
        // Nothing but the target branch was ever written
        assert!(remote.find_reference("refs/heads/main").is_err());
    }

    #[test]
    fn push_refuses_protected_branches_before_touching_the_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare(&tmp.path().join("remote.git"));
        let a = tmp.path().join("a");
        commit_file(&a, "one.md", "1", "one");
        set_remote(&a, &url).unwrap();
        for (target, prefix) in [
            ("main", "nandi"),
            ("master", "nandi"),
            ("main/alice", "main"),
            ("other/alice", "nandi"),
        ] {
            assert!(push(&a, None, target, prefix, true).is_err(), "{target}");
        }
        assert!(list_remote_branches(&a, None).unwrap().is_empty());
    }

    #[test]
    fn pull_fast_forwards() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare(&tmp.path().join("remote.git"));
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        commit_file(&a, "one.md", "1", "one");
        set_remote(&a, &url).unwrap();
        push(&a, None, TARGET, "nandi", false).unwrap();

        // b starts from the remote, then a moves ahead
        set_remote(&b, &url).unwrap();
        let first = pull(&layout(&b), None, TARGET, None).unwrap();
        assert!(!first.conflicts);
        assert_eq!(fs::read_to_string(b.join("one.md")).unwrap(), "1");

        commit_file(&a, "two.md", "2", "two");
        push(&a, None, TARGET, "nandi", false).unwrap();
        let before = status(&b, None, TARGET).unwrap();
        assert!(before.reachable);
        assert_eq!(before.behind, 1);

        let pulled = pull(&layout(&b), None, TARGET, None).unwrap();
        assert_eq!(pulled.pulled_commits, 1);
        assert!(!pulled.conflicts);
        assert_eq!(fs::read_to_string(b.join("two.md")).unwrap(), "2");
        let after = status(&b, None, TARGET).unwrap();
        assert_eq!((after.ahead, after.behind), (0, 0));

        // Nothing new: a no-op
        let again = pull(&layout(&b), None, TARGET, None).unwrap();
        assert_eq!(again.pulled_commits, 0);
    }

    #[test]
    fn pull_merges_and_leaves_conflicts_unresolved() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare(&tmp.path().join("remote.git"));
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        commit_file(&a, "shared.md", "base\n", "base");
        set_remote(&a, &url).unwrap();
        push(&a, None, TARGET, "nandi", false).unwrap();
        set_remote(&b, &url).unwrap();
        pull(&layout(&b), None, TARGET, None).unwrap();

        // Both sides change the same line
        commit_file(&a, "shared.md", "theirs\n", "theirs");
        push(&a, None, TARGET, "nandi", false).unwrap();
        commit_file(&b, "shared.md", "ours\n", "ours");

        let pulled = pull(&layout(&b), None, TARGET, None).unwrap();
        assert!(pulled.conflicts);
        assert_eq!(pulled.conflicted_files, ["shared.md"]);
        let repo = Repository::open(&b).unwrap();
        assert_eq!(repo.state(), RepositoryState::Merge);
        assert!(repo.index().unwrap().has_conflicts());
        let text = fs::read_to_string(b.join("shared.md")).unwrap();
        assert!(text.contains("<<<<<<<") && text.contains("ours") && text.contains("theirs"));

        let status = status(&b, None, TARGET).unwrap();
        assert!(status.conflicts);
        assert_eq!(status.conflicted_files, ["shared.md"]);
        // A second pull refuses while the merge is open
        assert!(pull(&layout(&b), None, TARGET, None).is_err());
    }

    #[test]
    fn pull_merges_cleanly_when_changes_do_not_overlap() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare(&tmp.path().join("remote.git"));
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        commit_file(&a, "base.md", "base", "base");
        set_remote(&a, &url).unwrap();
        push(&a, None, TARGET, "nandi", false).unwrap();
        set_remote(&b, &url).unwrap();
        pull(&layout(&b), None, TARGET, None).unwrap();
        commit_file(&a, "a.md", "a", "a");
        push(&a, None, TARGET, "nandi", false).unwrap();
        // An uncommitted local change is committed before merging
        fs::write(b.join("b.md"), "b").unwrap();

        let pulled = pull(&layout(&b), None, TARGET, None).unwrap();
        assert!(!pulled.conflicts);
        assert!(b.join("a.md").is_file() && b.join("b.md").is_file());
        let repo = Repository::open(&b).unwrap();
        assert_eq!(repo.state(), RepositoryState::Clean);
        assert_eq!(
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent_count(),
            2
        );
    }

    #[test]
    fn pull_of_a_missing_branch_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let url = bare(&tmp.path().join("remote.git"));
        let a = tmp.path().join("a");
        commit_file(&a, "one.md", "1", "one");
        set_remote(&a, &url).unwrap();
        let pulled = pull(&layout(&a), None, TARGET, None).unwrap();
        assert_eq!(pulled.pulled_commits, 0);
        let status = status(&a, None, TARGET).unwrap();
        assert!(status.reachable);
        assert_eq!(status.remote_head, None);
        assert_eq!(status.ahead, 1);
    }

    #[test]
    fn an_unreachable_remote_reports_unreachable() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        commit_file(&a, "one.md", "1", "one");
        set_remote(&a, &tmp.path().join("missing.git").to_string_lossy()).unwrap();
        assert!(test_connection(&a, None).is_err());
        assert!(!status(&a, None, TARGET).unwrap().reachable);
        remove_remote(&a).unwrap();
        remove_remote(&a).unwrap();
        assert!(test_connection(&a, None).is_err());
    }
}
