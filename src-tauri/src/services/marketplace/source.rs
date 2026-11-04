//! Marketplace sources over git (git2): list a repo's refs (`ls-remote`),
//! fetch one ref into a throwaway bare repo, and read files straight out of
//! the commit's tree.
//!
//! Nothing is checked out: files are read from git objects, so a source
//! can't plant a symlink (rejected), a submodule (skipped), or a hook.
//! The scratch repo lives in `~/.nandi/local/cache/`, outside the gold copy,
//! so the file watcher never sees a half-fetched source.
//!
//! Sources are `github.com/<owner>/<repo>` (HTTPS, with an optional GitHub
//! token), an `https://` / `file://` URL, or a local path (test fixtures,
//! local registries).

use std::path::{Path, PathBuf};

use git2::{Cred, CredentialType, Direction, FetchOptions, RemoteCallbacks, Repository};

use crate::services::repos::vendor_repo::FileSet;

/// A ref on the remote and the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRef {
    /// Full name, e.g. `refs/tags/v1.2.0`
    pub name: String,
    /// Commit SHA (tags peeled)
    pub commit: String,
}

/// Fetch progress, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchProgress {
    pub received_objects: usize,
    pub total_objects: usize,
}

/// The clone URL for a repo location.
pub fn clone_url(repo: &str) -> String {
    if let Some(path) = repo.strip_prefix("github.com/") {
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        return format!("https://github.com/{path}.git");
    }
    repo.to_string()
}

/// `https://github.com/<owner>/<repo>`, for "view source" links.
pub fn web_url(repo: &str) -> Option<String> {
    repo.strip_prefix("github.com/")
        .map(|path| format!("https://github.com/{}", path.trim_end_matches(".git")))
}

fn git_error(url: &str, e: &git2::Error) -> String {
    log::warn!("Git operation on {url} failed: {e}");
    match e.code() {
        git2::ErrorCode::Auth => {
            format!("{url} refused access. If it's private, add a GitHub token to the registry.")
        }
        git2::ErrorCode::NotFound => format!("{url} wasn't found"),
        _ => format!("Couldn't fetch {url}: {}", e.message()),
    }
}

#[derive(Clone)]
pub struct GitSource {
    url: String,
    /// GitHub token for private repos. Never logged or written anywhere.
    token: Option<String>,
}

impl GitSource {
    pub fn new(repo: &str, token: Option<String>) -> Self {
        Self {
            url: clone_url(repo),
            token: token.filter(|t| !t.is_empty()),
        }
    }

    fn callbacks<'a>(&self) -> RemoteCallbacks<'a> {
        let mut callbacks = RemoteCallbacks::new();
        if let Some(token) = self.token.clone() {
            let mut attempted = false;
            callbacks.credentials(move |_url, _user, allowed| {
                // libgit2 retries the callback on rejection; asking again loops
                if attempted || !allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
                    return Err(git2::Error::from_str("the token was rejected"));
                }
                attempted = true;
                Cred::userpass_plaintext("x-access-token", &token)
            });
        }
        callbacks
    }

    /// Every ref on the remote (`git ls-remote`), tags peeled to commits.
    pub fn refs(&self) -> Result<Vec<RemoteRef>, String> {
        let mut remote = git2::Remote::create_detached(self.url.as_str())
            .map_err(|e| git_error(&self.url, &e))?;
        let connection = remote
            .connect_auth(Direction::Fetch, Some(self.callbacks()), None)
            .map_err(|e| git_error(&self.url, &e))?;
        let heads = connection.list().map_err(|e| git_error(&self.url, &e))?;
        let mut refs: Vec<RemoteRef> = Vec::new();
        for head in heads {
            let name = head.name().to_string();
            let commit = head.oid().to_string();
            // `refs/tags/x^{}` is the commit an annotated tag points at
            if let Some(tag) = name.strip_suffix("^{}") {
                match refs.iter_mut().find(|r| r.name == tag) {
                    Some(existing) => existing.commit = commit,
                    None => refs.push(RemoteRef {
                        name: tag.to_string(),
                        commit,
                    }),
                }
            } else if !refs.iter().any(|r| r.name == name) {
                refs.push(RemoteRef { name, commit });
            }
        }
        Ok(refs)
    }

    /// The commit `git_ref` (a tag, branch, or full ref name) points at.
    pub fn resolve(&self, git_ref: &str) -> Result<RemoteRef, String> {
        let refs = self.refs()?;
        resolve_in(&refs, git_ref)
            .ok_or_else(|| format!("{} has no tag or branch named '{git_ref}'", self.url))
    }

    /// Release tags (`v1.2.0`, `1.2.0`, or `<prefix>v1.2.0`), newest first.
    pub fn release_tags(&self, prefix: &str) -> Result<Vec<(semver::Version, RemoteRef)>, String> {
        Ok(release_tags_with_prefix(self.refs()?, prefix))
    }

    /// Fetches `refs` (full names) into a new scratch repo under `scratch`.
    pub fn fetch(
        &self,
        refs: &[&str],
        scratch: &Path,
        mut progress: impl FnMut(FetchProgress),
    ) -> Result<Checkout, String> {
        let dir = scratch.join(format!("checkout-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let repo = Repository::init_bare(&dir).map_err(|e| git_error(&self.url, &e))?;
        let checkout = Checkout {
            repo: Some(repo),
            dir,
        };
        let refspecs: Vec<String> = refs.iter().map(|r| format!("+{r}:{r}")).collect();
        let run = |depth: Option<i32>, progress: &mut dyn FnMut(FetchProgress)| {
            let mut callbacks = self.callbacks();
            callbacks.transfer_progress(|stats| {
                progress(FetchProgress {
                    received_objects: stats.received_objects(),
                    total_objects: stats.total_objects(),
                });
                true
            });
            let mut options = FetchOptions::new();
            options.remote_callbacks(callbacks);
            options.download_tags(git2::AutotagOption::None);
            if let Some(depth) = depth {
                options.depth(depth);
            }
            let mut remote = checkout
                .repo()
                .remote_anonymous(&self.url)
                .map_err(|e| git_error(&self.url, &e))?;
            remote
                .fetch(&refspecs, Some(&mut options), None)
                .map_err(|e| git_error(&self.url, &e))
        };
        // Shallow when the transport supports it (GitHub does; local paths
        // may not)
        if let Err(shallow) = run(Some(1), &mut progress) {
            log::debug!("Shallow fetch failed ({shallow}); fetching in full");
            run(None, &mut progress)?;
        }
        Ok(checkout)
    }
}

pub(crate) fn resolve_in(refs: &[RemoteRef], git_ref: &str) -> Option<RemoteRef> {
    let candidates = [
        git_ref.to_string(),
        format!("refs/tags/{git_ref}"),
        format!("refs/heads/{git_ref}"),
    ];
    candidates
        .iter()
        .find_map(|name| refs.iter().find(|r| &r.name == name))
        .cloned()
}

/// Release tags with `prefix` (`""` for plain `vX.Y.Z`), newest first.
pub(crate) fn release_tags_with_prefix(
    refs: Vec<RemoteRef>,
    prefix: &str,
) -> Vec<(semver::Version, RemoteRef)> {
    let mut tags: Vec<(semver::Version, RemoteRef)> = refs
        .into_iter()
        .filter(|r| r.name.starts_with("refs/tags/"))
        .filter_map(|r| match super::manifest::split_tag(&r.name) {
            Some((p, version)) if p == prefix => Some((version, r)),
            _ => None,
        })
        .collect();
    tags.sort_by(|a, b| b.0.cmp(&a.0));
    tags
}

/// A fetched source in a throwaway bare repo; removed when dropped.
pub struct Checkout {
    repo: Option<Repository>,
    dir: PathBuf,
}

impl Drop for Checkout {
    fn drop(&mut self) {
        // Close the repo first: Windows won't delete open pack files
        drop(self.repo.take());
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            log::warn!("Failed to remove {}: {e}", self.dir.display());
        }
    }
}

impl Checkout {
    fn repo(&self) -> &Repository {
        self.repo.as_ref().expect("the repo is open until drop")
    }

    /// The commit a fetched ref points at.
    #[cfg(test)]
    pub fn commit_of(&self, name: &str) -> Result<String, String> {
        self.repo()
            .revparse_single(name)
            .and_then(|object| object.peel_to_commit())
            .map(|commit| commit.id().to_string())
            .map_err(|e| format!("'{name}' wasn't fetched: {}", e.message()))
    }

    /// Every file under `subpath` (the whole repo if `None`) at `commit`,
    /// relative to `subpath`. Symlinks are an error; submodules are skipped.
    pub fn files(&self, commit: &str, subpath: Option<&str>) -> Result<FileSet, String> {
        let repo = self.repo();
        let oid = git2::Oid::from_str(commit).map_err(|e| e.message().to_string())?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| format!("Commit {commit} wasn't fetched: {}", e.message()))?;
        let mut tree = commit.tree().map_err(|e| e.message().to_string())?;
        if let Some(subpath) = subpath {
            let entry = tree
                .get_path(Path::new(subpath))
                .map_err(|_| format!("The source has no folder '{subpath}'"))?;
            tree = entry
                .to_object(repo)
                .and_then(|object| object.peel_to_tree())
                .map_err(|_| format!("'{subpath}' in the source is not a folder"))?;
        }
        let mut files = FileSet::new();
        let mut problem = None;
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            let path = format!("{dir}{}", entry.name().unwrap_or_default());
            match entry.filemode() {
                0o100644 | 0o100755 => match entry.to_object(repo).and_then(|o| o.peel_to_blob()) {
                    Ok(blob) => {
                        files.insert(path, blob.content().to_vec());
                    }
                    Err(e) => {
                        problem = Some(format!("Couldn't read {path}: {}", e.message()));
                        return git2::TreeWalkResult::Abort;
                    }
                },
                0o120000 => {
                    problem = Some(format!(
                        "The source contains a symbolic link ({path}); refusing to install it"
                    ));
                    return git2::TreeWalkResult::Abort;
                }
                _ => {} // folders (walked into) and submodules (skipped)
            }
            git2::TreeWalkResult::Ok
        })
        .ok();
        match problem {
            Some(problem) => Err(problem),
            None => Ok(files),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    /// A local git repo with `files` committed and tagged `tag` (annotated).
    pub fn fixture_repo(dir: &Path, files: &[(&str, &str)], tag: &str) -> String {
        let repo = match Repository::open(dir) {
            Ok(repo) => repo,
            Err(_) => Repository::init(dir).unwrap(),
        };
        // Replace the working tree with exactly `files`
        for entry in fs::read_dir(dir).unwrap().flatten() {
            if entry.file_name() != ".git" {
                let path = entry.path();
                if path.is_dir() {
                    fs::remove_dir_all(path).unwrap();
                } else {
                    fs::remove_file(path).unwrap();
                }
            }
        }
        for (path, content) in files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let mut index = repo.index().unwrap();
        index.clear().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, tag, &tree, &parents)
            .unwrap();
        let object = repo.find_object(oid, None).unwrap();
        repo.tag(tag, &object, &sig, tag, true).unwrap();
        oid.to_string()
    }

    #[test]
    fn lists_resolves_fetches_and_reads_a_local_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        let v1 = fixture_repo(
            &origin,
            &[
                ("agents/a/skill.md", "---\nname: A\n---\n"),
                ("README.md", "hi"),
            ],
            "v1.0.0",
        );
        let v2 = fixture_repo(&origin, &[("agents/a/skill.md", "two")], "v1.1.0");
        let source = GitSource::new(&origin.display().to_string(), None);

        let tags = source.release_tags("").unwrap();
        assert_eq!(tags[0].0.to_string(), "1.1.0");
        assert_eq!(tags[0].1.commit, v2, "annotated tags peel to commits");
        assert_eq!(source.resolve("v1.0.0").unwrap().commit, v1);
        assert!(source.resolve("v9.9.9").is_err());

        let scratch = tmp.path().join("scratch");
        let mut seen = false;
        let checkout = source
            .fetch(&["refs/tags/v1.0.0", "refs/tags/v1.1.0"], &scratch, |_| {
                seen = true
            })
            .unwrap();
        assert!(seen, "progress is reported");
        assert_eq!(checkout.commit_of("refs/tags/v1.0.0").unwrap(), v1);
        let old = checkout.files(&v1, None).unwrap();
        assert_eq!(old.len(), 2);
        let agent = checkout.files(&v2, Some("agents/a")).unwrap();
        assert_eq!(agent["skill.md"], b"two");
        assert!(checkout.files(&v2, Some("agents/missing")).is_err());
        drop(checkout);
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0, "cleaned up");
    }

    #[test]
    fn clone_urls() {
        assert_eq!(clone_url("github.com/a/b"), "https://github.com/a/b.git");
        assert_eq!(
            clone_url("github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(clone_url("file:///tmp/x"), "file:///tmp/x");
        assert_eq!(web_url("github.com/a/b").unwrap(), "https://github.com/a/b");
        let refs = vec![
            RemoteRef {
                name: "refs/heads/main".into(),
                commit: "1".into(),
            },
            RemoteRef {
                name: "refs/tags/v0.9.0".into(),
                commit: "2".into(),
            },
            RemoteRef {
                name: "refs/tags/v1.10.0".into(),
                commit: "3".into(),
            },
            RemoteRef {
                name: "refs/tags/v1.9.0".into(),
                commit: "4".into(),
            },
        ];
        assert_eq!(resolve_in(&refs, "main").unwrap().commit, "1");
        let tags = release_tags_with_prefix(refs, "");
        assert_eq!(tags[0].0.to_string(), "1.10.0", "semver, not string order");
    }
}
