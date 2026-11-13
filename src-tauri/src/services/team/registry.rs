//! A team's private registry: any git repo, checked out at
//! `~/.nandi/repo/teams/<id>/`.
//!
//! ```text
//! index.json                 what's shared: agents (with publisher) and packs
//! agents/<id>/skill.md …     copies of shared agent folders
//! packs/<pack>/…
//! publishers/<handle>.json   who shares (handle, display name, instance)
//! ```
//!
//! The checkout is a mirror: every sync resets it to the remote's default
//! branch, and changes go in only as commits pushed to a `nandi/…` branch
//! (never the default branch: `push_head` checks every push with
//! `remote::assert_pushable_branch`). Someone with write access merges.
//!
//! **Shares are derived from the registry**, not stored locally: an agent
//! is shared by me when `index.json` lists it with my handle as its
//! publisher. The registry is what teammates see, so it's the one record.
//!
//! Git is libgit2, with a token (if any) passed only through credential
//! callbacks; without one, the SSH agent or git's credential helper.

use std::fs;
use std::path::{Path, PathBuf};

use git2::build::CheckoutBuilder;
use git2::{BranchType, FetchOptions, PushOptions, RemoteCallbacks, Repository};
use serde::{Deserialize, Serialize};

use crate::model::Timestamp;
use crate::services::remote::{self, git_ops};
use crate::services::repo_bootstrap;
use crate::services::repos::layout::SKILL_FILE;

/// Registry pushes go under this prefix only (`nandi/<me>`,
/// `nandi/share-…`, `nandi/unshare-…`)
const PUSH_PREFIX: &str = remote::DEFAULT_BRANCH_PREFIX;

pub const INDEX_FILE: &str = "index.json";
pub const AGENTS_DIR: &str = "agents";
pub const PACKS_DIR: &str = "packs";
pub const PUBLISHERS_DIR: &str = "publishers";
/// Inside `.git/`, so it's never committed
const LAST_SYNCED_FILE: &str = "nandi-last-synced";
const KEEP_FILE: &str = ".gitkeep";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryIndex {
    #[serde(default = "schema_version")]
    pub version: u32,
    #[serde(default)]
    pub agents: Vec<IndexAgent>,
    #[serde(default)]
    pub packs: Vec<String>,
}

fn schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexAgent {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// GitHub handle of who shared it
    pub publisher: String,
    /// The publisher's Nandi instance (cards are `<instance>/<agent>`)
    #[serde(default)]
    pub instance_id: Option<String>,
    pub shared_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publisher {
    pub github_handle: String,
    pub display_name: String,
    #[serde(default)]
    pub instance_id: Option<String>,
}

/// `<teams_root>/<org>`
pub fn checkout_dir(teams_root: &Path, team_id: &str) -> PathBuf {
    teams_root.join(team_id)
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

pub fn read_index(dir: &Path) -> RegistryIndex {
    fs::read_to_string(dir.join(INDEX_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| RegistryIndex {
            version: schema_version(),
            ..RegistryIndex::default()
        })
}

pub fn write_index(dir: &Path, index: &RegistryIndex) -> Result<(), String> {
    let text = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
    fs::write(dir.join(INDEX_FILE), format!("{text}\n"))
        .map_err(|e| format!("Failed to write {INDEX_FILE}: {e}"))
}

/// Adds whatever's missing of the layout. Returns whether anything was.
pub fn scaffold(dir: &Path) -> Result<bool, String> {
    let mut changed = false;
    for sub in [AGENTS_DIR, PACKS_DIR, PUBLISHERS_DIR] {
        let path = dir.join(sub);
        if !path.is_dir() {
            fs::create_dir_all(&path).map_err(|e| format!("Failed to create {sub}/: {e}"))?;
            fs::write(path.join(KEEP_FILE), "").map_err(|e| e.to_string())?;
            changed = true;
        }
    }
    if !dir.join(INDEX_FILE).is_file() {
        write_index(dir, &read_index(dir))?;
        changed = true;
    }
    Ok(changed)
}

/// Replaces `agents/<id>/` in the checkout with a copy of `source`
pub fn copy_agent(source: &Path, dir: &Path, id: &str) -> Result<(), String> {
    if !source.join(SKILL_FILE).is_file() {
        return Err(format!("Agent '{id}' has no {SKILL_FILE}"));
    }
    let target = dir.join(AGENTS_DIR).join(id);
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| format!("Failed to replace {id}: {e}"))?;
    }
    copy_tree(source, &target)
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target)
        .map_err(|e| format!("Failed to create {}: {e}", target.display()))?;
    let entries =
        fs::read_dir(source).map_err(|e| format!("Failed to read {}: {e}", source.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Machine-local state and editor/installer clutter stay behind
        if name_str.starts_with('.') || name_str.contains(".nandi-tmp-") {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let to = target.join(&name);
        if kind.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &to)
                .map_err(|e| format!("Failed to copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

pub fn remove_agent(dir: &Path, id: &str) -> Result<(), String> {
    let target = dir.join(AGENTS_DIR).join(id);
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| format!("Failed to remove {id}: {e}"))?;
    }
    Ok(())
}

pub fn write_publisher(dir: &Path, publisher: &Publisher) -> Result<(), String> {
    let path = dir
        .join(PUBLISHERS_DIR)
        .join(format!("{}.json", publisher.github_handle));
    fs::create_dir_all(dir.join(PUBLISHERS_DIR)).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(publisher).map_err(|e| e.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|e| format!("Failed to write publisher: {e}"))
}

pub fn read_publisher(dir: &Path, handle: &str) -> Option<Publisher> {
    let text = fs::read_to_string(dir.join(PUBLISHERS_DIR).join(format!("{handle}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

/// `(id, skill.md)` for every agent folder in the checkout, sorted by ID
pub fn agent_folders(dir: &Path) -> Vec<(String, String)> {
    let mut agents: Vec<(String, String)> = subdirs(&dir.join(AGENTS_DIR))
        .into_iter()
        .filter_map(|(id, path)| {
            fs::read_to_string(path.join(SKILL_FILE))
                .ok()
                .map(|skill| (id, skill))
        })
        .collect();
    agents.sort();
    agents
}

/// Pack IDs: folders under `packs/`, plus any the index lists
pub fn packs(dir: &Path) -> Vec<String> {
    let mut packs: Vec<String> = subdirs(&dir.join(PACKS_DIR))
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for pack in read_index(dir).packs {
        if !packs.contains(&pack) {
            packs.push(pack);
        }
    }
    packs.sort();
    packs
}

fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| !name.starts_with('.'))
        .collect()
}

pub fn last_synced(dir: &Path) -> Option<Timestamp> {
    fs::read_to_string(dir.join(".git").join(LAST_SYNCED_FILE))
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

fn mark_synced(dir: &Path, now: Timestamp) {
    let _ = fs::write(dir.join(".git").join(LAST_SYNCED_FILE), format!("{now}"));
}

/// Keeps `teams/` out of the agents repo (`~/.nandi/repo`): each checkout
/// is its own git repo, and would otherwise show up as an uncommitted
/// change there (blocking pulls) and be swept into "Sync now".
pub fn exclude_from_parent(teams_root: &Path) {
    let Some(parent) = teams_root.parent() else {
        return;
    };
    // `open` doesn't search upward: only a repo rooted at `parent` counts
    let Ok(repo) = Repository::open(parent) else {
        return;
    };
    let name = teams_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "teams".to_string());
    let entry = format!("/{name}/");
    let exclude = repo.path().join("info").join("exclude");
    let current = fs::read_to_string(&exclude).unwrap_or_default();
    if current.lines().any(|line| line.trim() == entry) {
        return;
    }
    let _ = fs::create_dir_all(exclude.parent().unwrap_or(repo.path()));
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "# Nandi team registries (their own git repos)\n{entry}\n"
    ));
    if let Err(e) = fs::write(&exclude, text) {
        log::warn!("Failed to exclude {entry} from the agents repo: {e}");
    }
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

/// What a sync did
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Synced {
    /// New commits came in (or this was the first clone)
    pub updated: bool,
    /// The remote has no commits yet
    pub empty: bool,
}

/// Clones the registry into `dir`, or pulls it: the checkout ends up at the
/// remote's default branch, whatever was there before.
pub fn sync(dir: &Path, url: &str, token: Option<&str>, now: Timestamp) -> Result<Synced, String> {
    let repo = open_or_init(dir)?;
    ensure_origin(&repo, url)?;
    fetch(&repo, token)?;
    let Some(branch) = remote_default_branch(&repo) else {
        mark_synced(dir, now);
        return Ok(Synced {
            updated: false,
            empty: true,
        });
    };
    let upstream = upstream_oid(&repo, &branch).ok_or("The registry has no default branch")?;
    let head = repo.head().ok().and_then(|h| h.target());
    let updated = head != Some(upstream) || current_branch(&repo) != branch;
    if updated {
        let branch_ref = format!("refs/heads/{branch}");
        let target = repo.find_commit(upstream).map_err(git_error)?;
        repo.reference(&branch_ref, upstream, true, "nandi: sync registry")
            .and_then(|_| repo.set_head(&branch_ref))
            .and_then(|()| {
                repo.checkout_tree(target.as_object(), Some(CheckoutBuilder::new().force()))
            })
            .and_then(|()| repo.reset(target.as_object(), git2::ResetType::Hard, None))
            .map_err(git_error)?;
        if let Ok(mut local) = repo.find_branch(&branch, BranchType::Local) {
            let _ = local.set_upstream(Some(&format!("origin/{branch}")));
        }
    }
    mark_synced(dir, now);
    Ok(Synced {
        updated,
        empty: false,
    })
}

/// Commits every change in the checkout. `None` if there was nothing.
pub fn commit(dir: &Path, message: &str, owner: Option<&str>) -> Result<Option<String>, String> {
    let repo = Repository::open(dir).map_err(git_error)?;
    Ok(repo_bootstrap::commit_all(&repo, message, owner)?.map(|oid| oid.to_string()))
}

/// Pushes HEAD to `remote_branch` on origin: a `nandi/…` branch only, never
/// the default branch. A rejected push (someone pushed first) is an error;
/// sync and try again. `force` replaces the branch (branches Nandi owns).
pub fn push_head(
    dir: &Path,
    remote_branch: &str,
    token: Option<&str>,
    force: bool,
) -> Result<(), String> {
    remote::assert_pushable_branch(remote_branch, PUSH_PREFIX)?;
    push_unchecked(dir, remote_branch, token, force)
}

/// [`push_head`] without the branch check: only for tests acting as a
/// teammate with write access to the default branch
fn push_unchecked(
    dir: &Path,
    remote_branch: &str,
    token: Option<&str>,
    force: bool,
) -> Result<(), String> {
    let repo = Repository::open(dir).map_err(git_error)?;
    let head = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .ok_or("Nothing to push")?;
    let mut remote = repo.find_remote("origin").map_err(git_error)?;
    let rejected = std::cell::RefCell::new(None::<String>);
    let mut callbacks = callbacks(token);
    callbacks.push_update_reference(|reference, status| {
        if let Some(status) = status {
            *rejected.borrow_mut() = Some(format!("{reference}: {status}"));
        }
        Ok(())
    });
    let mut options = PushOptions::new();
    options.remote_callbacks(callbacks);
    let plus = if force { "+" } else { "" };
    let refspec = format!("{plus}HEAD:refs/heads/{remote_branch}");
    remote
        .push(&[refspec.as_str()], Some(&mut options))
        .map_err(git_error)?;
    drop(options);
    if let Some(reason) = rejected.into_inner() {
        return Err(format!(
            "The registry rejected the push ({reason}). Sync the team and try again."
        ));
    }
    repo.reference(
        &format!("refs/remotes/origin/{remote_branch}"),
        head,
        true,
        "nandi: pushed",
    )
    .map_err(git_error)?;
    Ok(())
}

/// Moves the checkout back to what origin has (after pushing a PR branch,
/// or a failed push)
pub fn reset_to_origin(dir: &Path) -> Result<(), String> {
    let repo = Repository::open(dir).map_err(git_error)?;
    let branch = current_branch(&repo);
    let Some(upstream) = upstream_oid(&repo, &branch) else {
        return Ok(());
    };
    let target = repo.find_object(upstream, None).map_err(git_error)?;
    repo.reset(&target, git2::ResetType::Hard, None)
        .map_err(git_error)?;
    // A reset leaves new files behind; drop them so the mirror is exact
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let untracked: Vec<PathBuf> = repo
        .statuses(Some(&mut options))
        .map_err(git_error)?
        .iter()
        .filter(|e| e.status().contains(git2::Status::WT_NEW))
        .filter_map(|e| e.path().map(|p| dir.join(p)))
        .collect();
    for path in untracked {
        let _ = fs::remove_file(&path);
    }
    Ok(())
}

fn open_or_init(dir: &Path) -> Result<Repository, String> {
    if let Ok(repo) = Repository::open(dir) {
        return Ok(repo);
    }
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    let mut options = git2::RepositoryInitOptions::new();
    options.initial_head("main");
    Repository::init_opts(dir, &options).map_err(git_error)
}

fn ensure_origin(repo: &Repository, url: &str) -> Result<(), String> {
    match repo.find_remote("origin") {
        Ok(remote) if remote.url() == Some(url) => Ok(()),
        Ok(_) => repo.remote_set_url("origin", url).map_err(git_error),
        Err(_) => repo.remote("origin", url).map(|_| ()).map_err(git_error),
    }
}

fn fetch(repo: &Repository, token: Option<&str>) -> Result<(), String> {
    let mut remote = repo.find_remote("origin").map_err(git_error)?;
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(token));
    options.prune(git2::FetchPrune::On);
    remote
        .fetch::<&str>(&[], Some(&mut options), None)
        .map_err(git_error)?;
    let _ = remote.disconnect();
    Ok(())
}

fn current_branch(repo: &Repository) -> String {
    repo.find_reference("HEAD")
        .ok()
        .and_then(|head| head.symbolic_target().map(str::to_string))
        .and_then(|target| target.strip_prefix("refs/heads/").map(str::to_string))
        .unwrap_or_else(|| "main".to_string())
}

fn upstream_oid(repo: &Repository, branch: &str) -> Option<git2::Oid> {
    repo.find_reference(&format!("refs/remotes/origin/{branch}"))
        .ok()
        .and_then(|r| r.target())
}

/// origin/HEAD, else `main`/`master`, else the only branch
fn remote_default_branch(repo: &Repository) -> Option<String> {
    if let Some(target) = repo
        .find_reference("refs/remotes/origin/HEAD")
        .ok()
        .and_then(|r| r.symbolic_target().map(str::to_string))
    {
        return target
            .strip_prefix("refs/remotes/origin/")
            .map(str::to_string);
    }
    let branches: Vec<String> = repo
        .branches(Some(BranchType::Remote))
        .ok()?
        .flatten()
        .filter_map(|(branch, _)| branch.name().ok().flatten().map(str::to_string))
        .filter_map(|name| name.strip_prefix("origin/").map(str::to_string))
        .filter(|name| name != "HEAD")
        .collect();
    branches
        .iter()
        .find(|b| *b == "main")
        .or_else(|| branches.iter().find(|b| *b == "master"))
        .cloned()
        .or_else(|| match branches.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        })
}

/// A token (the GitHub sign-in's, for a github.com registry) is the
/// password of `x-access-token`; without one, the SSH agent, git's
/// credential helper, or the platform default (local remotes in tests ask
/// for nothing). The remote's own credential is never sent to a registry:
/// it may live on another host.
fn callbacks<'a>(token: Option<&str>) -> RemoteCallbacks<'a> {
    git_ops::callbacks(token.map(git_ops::token_credential))
}

fn git_error(e: git2::Error) -> String {
    log::warn!("Registry git operation failed: {e}");
    if e.code() == git2::ErrorCode::Auth {
        return "The team registry refused access. For a github.com registry, sign in to GitHub (Settings › Account); otherwise check that you can reach it with git (an SSH key or a credential helper).".to_string();
    }
    format!("Git error: {}", e.message())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// An empty bare repo at `<root>/<name>.git` (a team with ID `name`)
    pub fn bare_registry(root: &Path, name: &str) -> PathBuf {
        let path = root.join(format!("{name}.git"));
        let mut options = git2::RepositoryInitOptions::new();
        options.bare(true).initial_head("main");
        Repository::init_opts(&path, &options).unwrap();
        path
    }

    /// Commits `files` to the bare repo from a scratch clone, as a teammate
    /// would
    pub fn push_files(bare: &Path, scratch: &Path, files: &[(&str, &str)]) {
        let url = bare.to_string_lossy().into_owned();
        sync(scratch, &url, None, 0.0).unwrap();
        for (rel, content) in files {
            let path = scratch.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        commit(scratch, "teammate change", Some("hubot")).unwrap();
        push_unchecked(scratch, "main", None, false).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn sync_clones_an_empty_remote_then_pulls() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = bare_registry(tmp.path(), "acme");
        let url = bare.to_string_lossy().into_owned();
        let dir = tmp.path().join("teams").join("acme");

        let synced = sync(&dir, &url, None, 1.0).unwrap();
        assert!(synced.empty);
        assert_eq!(last_synced(&dir), Some(1.0));

        // Scaffold and push, as an admin would (Nandi itself never pushes
        // the default branch)
        assert!(scaffold(&dir).unwrap());
        assert!(!scaffold(&dir).unwrap());
        commit(&dir, "Create registry", None).unwrap();
        push_unchecked(&dir, "main", None, false).unwrap();

        push_files(
            &bare,
            &tmp.path().join("scratch"),
            &[("agents/writer/skill.md", "---\nname: Writer\n---\n")],
        );
        let synced = sync(&dir, &url, None, 2.0).unwrap();
        assert!(synced.updated && !synced.empty);
        assert_eq!(agent_folders(&dir)[0].0, "writer");
        assert!(dir.join(INDEX_FILE).is_file());
    }

    #[test]
    fn push_head_never_pushes_the_default_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = bare_registry(tmp.path(), "acme");
        let url = bare.to_string_lossy().into_owned();
        let dir = tmp.path().join("teams").join("acme");
        sync(&dir, &url, None, 1.0).unwrap();
        scaffold(&dir).unwrap();
        commit(&dir, "Create registry", None).unwrap();
        for branch in [
            "main", "master", "MAIN", "main/x", "share-me", "", "other/me",
        ] {
            assert!(push_head(&dir, branch, None, true).is_err(), "{branch}");
        }
        push_head(&dir, "nandi/share-me-papers", None, false).unwrap();
        let remote = Repository::open(&bare).unwrap();
        assert!(remote.find_reference("refs/heads/main").is_err());
        assert!(remote
            .find_reference("refs/heads/nandi/share-me-papers")
            .is_ok());
    }

    #[test]
    fn copy_agent_copies_the_folder_without_local_clutter() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("src").join("papers");
        fs::create_dir_all(source.join("workflows")).unwrap();
        fs::write(source.join(SKILL_FILE), "---\nname: Papers\n---\n").unwrap();
        fs::write(source.join("instructions.md"), "Find papers").unwrap();
        fs::write(source.join("workflows").join("w.yaml"), "name: w\n").unwrap();
        fs::write(source.join(".DS_Store"), "x").unwrap();
        let dir = tmp.path().join("reg");
        fs::create_dir_all(dir.join(AGENTS_DIR).join("papers")).unwrap();
        fs::write(dir.join(AGENTS_DIR).join("papers").join("stale.md"), "old").unwrap();

        copy_agent(&source, &dir, "papers").unwrap();
        let target = dir.join(AGENTS_DIR).join("papers");
        assert!(target.join(SKILL_FILE).is_file());
        assert_eq!(
            fs::read_to_string(target.join("instructions.md")).unwrap(),
            "Find papers"
        );
        assert!(target.join("workflows").join("w.yaml").is_file());
        assert!(!target.join(".DS_Store").exists());
        assert!(!target.join("stale.md").exists());

        assert!(copy_agent(&tmp.path().join("missing"), &dir, "x").is_err());
        remove_agent(&dir, "papers").unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn index_round_trips_and_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_index(tmp.path()).version, 1);
        let index = RegistryIndex {
            version: 1,
            agents: vec![IndexAgent {
                id: "papers".into(),
                name: "Papers".into(),
                publisher: "octocat".into(),
                instance_id: Some("i1".into()),
                shared_at: 5.0,
            }],
            packs: vec!["research".into()],
        };
        write_index(tmp.path(), &index).unwrap();
        assert_eq!(read_index(tmp.path()), index);
        assert_eq!(packs(tmp.path()), ["research"]);
    }

    #[test]
    fn teams_are_excluded_from_the_agents_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        Repository::init(&root).unwrap();
        let teams = root.join("teams");
        exclude_from_parent(&teams);
        exclude_from_parent(&teams);
        let text = fs::read_to_string(root.join(".git/info/exclude")).unwrap();
        assert_eq!(text.matches("/teams/").count(), 1);
    }
}
