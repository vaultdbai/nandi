//! Repo bootstrap and the git helpers shared by everything that commits.
//!
//! On every start, `~/.nandi/repo` is opened, or created as a git repo on
//! `main`. Whatever the repo lacks of `.nandi/settings.yaml`, `.gitignore`,
//! the built-in agents and `.nandi/manifest.yaml` is written (the manifest
//! last), and a freshly created repo gets an initial commit.

use std::path::Path;

use crate::services::builtin_agents;
use crate::services::domain::repo::RepoManifest;
use crate::services::domain::settings::{SecretRef, Settings};
use crate::services::egress::credentials::{ACCESS_KEY_ENTRY, SECRET_KEY_ENTRY};
use crate::services::llm_keys::KeyedProvider;
use crate::services::repos::fs_atomic::write_if_changed;
use crate::services::repos::{RepoLayout, Repos, SingletonRepository};

/// The message of the commit a fresh repo starts with
pub const INITIAL_COMMIT: &str = "Initialize Nandi repo";

pub const GITIGNORE: &str = "# Machine-local Nandi state: never commit
.nandi/local/
*.db
*.db-journal
*.db-wal
*.db-shm
*.log
work/
.*.nandi-tmp-*
# Team registry checkouts (each is its own git repo; see services::team)
/teams/
";

/// The layout of this install: `~/.nandi/repo`, with deleted agents moved
/// to `~/.nandi/trash`.
pub fn install_layout() -> Result<RepoLayout, String> {
    let nandi_dir = crate::utils::nandi_config::nandi_dir()?;
    Ok(RepoLayout::new(
        nandi_dir.join("repo"),
        nandi_dir.join("trash"),
    ))
}

/// Opens the repo at `layout.root`, creating it if needed, and writes what
/// it lacks. A repo created here (or one with no commit and no remote yet)
/// gets the initial commit.
pub fn bootstrap(layout: &RepoLayout) -> Result<(), String> {
    let (repo, fresh) = open_or_init(&layout.root)?;
    ensure_scaffolding(layout, &repo, None)?;
    if fresh {
        commit_all(&repo, INITIAL_COMMIT, None)?;
        log::info!("Created the Nandi repo at {}", layout.root.display());
    }
    Ok(())
}

/// Writes what a just-fetched repo lacks (settings, `.gitignore`, built-in
/// agents, manifest), staged. `owner` is recorded in a new manifest.
pub fn adopt_repo(layout: &RepoLayout, owner: Option<&str>) -> Result<(), String> {
    let repo = git2::Repository::open(&layout.root).map_err(|e| e.message().to_string())?;
    ensure_scaffolding(layout, &repo, owner)
}

/// The repo at `root` and whether it is new: just initialized, or without
/// a commit and a remote.
fn open_or_init(root: &Path) -> Result<(git2::Repository, bool), String> {
    std::fs::create_dir_all(root)
        .map_err(|e| format!("Failed to create {}: {e}", root.display()))?;
    if let Ok(repo) = git2::Repository::open(root) {
        let fresh = repo.head().is_err() && repo.find_remote("origin").is_err();
        return Ok((repo, fresh));
    }
    let mut options = git2::RepositoryInitOptions::new();
    options.initial_head("main");
    let repo = git2::Repository::init_opts(root, &options)
        .map_err(|e| format!("Failed to create a git repo at {}: {e}", root.display()))?;
    Ok((repo, true))
}

/// Settings, `.gitignore`, the built-in agents and (last) the manifest,
/// where missing.
fn ensure_scaffolding(
    layout: &RepoLayout,
    repo: &git2::Repository,
    owner: Option<&str>,
) -> Result<(), String> {
    if !layout.settings_path().is_file() {
        Repos::new(layout.clone())
            .settings
            .save(&default_settings())?;
    }
    let gitignore = layout.root.join(".gitignore");
    if !gitignore.is_file() {
        write_if_changed(&gitignore, GITIGNORE.as_bytes()).map_err(|e| e.to_string())?;
        stage_add(repo, &layout.root, &gitignore);
    }
    for path in builtin_agents::seed(layout)? {
        stage_add(repo, &layout.root, &path);
    }
    if !layout.manifest_path().is_file() {
        let manifest = RepoManifest {
            owner: owner.map(str::to_string),
            ..RepoManifest::default()
        };
        let path = layout.manifest_path();
        write_if_changed(&path, manifest.to_yaml().as_bytes()).map_err(|e| e.to_string())?;
        stage_add(repo, &layout.root, &path);
    }
    Ok(())
}

/// Default settings, with references to where the LLM and AWS keys live
/// in the keyring.
fn default_settings() -> Settings {
    let mut settings = Settings::default();
    for (key, entry) in KeyedProvider::all()
        .iter()
        .map(|p| (p.setting_key(), p.keyring_user()))
        .chain([
            ("aws_access_key", ACCESS_KEY_ENTRY),
            ("aws_secret_key", SECRET_KEY_ENTRY),
        ])
    {
        if let Ok(secret_ref) = SecretRef::parse(entry) {
            let _ = settings.set_secret_ref(key, secret_ref);
        }
    }
    settings
}

fn stage_add(repo: &git2::Repository, root: &Path, path: &Path) {
    let Ok(relative) = path.strip_prefix(root) else {
        return;
    };
    let result = repo.index().and_then(|mut index| {
        index.add_path(relative)?;
        index.write()
    });
    if let Err(e) = result {
        log::warn!("Failed to stage {}: {e}", relative.display());
    }
}

/// Commits everything staged (and new files) on the current branch.
pub(crate) fn commit_all(
    repo: &git2::Repository,
    message: &str,
    owner: Option<&str>,
) -> Result<Option<git2::Oid>, String> {
    let error = |e: git2::Error| format!("Git error: {}", e.message());
    let mut index = repo.index().map_err(error)?;
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .map_err(error)?;
    index.update_all(["*"], None).map_err(error)?;
    index.write().map_err(error)?;
    let tree_id = index.write_tree().map_err(error)?;
    let tree = repo.find_tree(tree_id).map_err(error)?;
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
        return Ok(None); // nothing to commit
    }
    let signature = signature(repo, owner).map_err(error)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(error)?;
    Ok(Some(oid))
}

/// The repo's configured identity, else the owner, else "Nandi".
pub(crate) fn signature(
    repo: &git2::Repository,
    owner: Option<&str>,
) -> Result<git2::Signature<'static>, git2::Error> {
    if let Ok(signature) = repo.signature() {
        return Ok(signature.to_owned());
    }
    match owner {
        Some(login) => git2::Signature::now(login, &format!("{login}@users.noreply.github.com")),
        None => git2::Signature::now("Nandi", "nandi@localhost"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_install_gets_a_fresh_committed_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        bootstrap(&layout).unwrap();
        let repo = git2::Repository::open(&layout.root).unwrap();
        let head = repo.head().unwrap();
        assert_eq!(head.shorthand(), Some("main"));
        assert_eq!(
            head.peel_to_commit().unwrap().message(),
            Some(INITIAL_COMMIT)
        );
        assert!(layout.settings_path().is_file());
        assert!(layout.manifest_path().is_file());
        assert!(layout.skill_path(builtin_agents::CHIEF_OF_STAFF).is_file());
        assert!(
            repo.statuses(None).unwrap().is_empty(),
            "nothing left uncommitted"
        );

        bootstrap(&layout).unwrap();
        assert_eq!(
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent_count(),
            0,
            "a second start commits nothing"
        );
    }

    #[test]
    fn settings_refer_to_keyring_entries_only() {
        let settings = default_settings();
        assert_eq!(
            settings.secret_refs()["deepseek_api_key"].as_str(),
            "nandi.llm.deepseek"
        );
    }
}
