//! Capabilities: `.nandi/capabilities/<id>.yaml`. The only
//! writer of capability files: atomic write + `git add`, like the other
//! repositories. The file name is the ID (a file that omits `id:` takes it
//! from the name; one that names another ID is rejected).

use std::path::{Path, PathBuf};

use super::fs_atomic::{remove_if_exists, write_if_changed};
use super::git::GitIndex;
use super::RepoLayout;
use crate::services::capability::definition::{self, CapabilityDefinition};

/// Keeps the empty folder in git
pub const GITKEEP_FILE: &str = ".gitkeep";

#[derive(Debug, Clone)]
pub struct CapabilityRepository {
    layout: RepoLayout,
    git: GitIndex,
}

impl CapabilityRepository {
    pub fn new(layout: RepoLayout) -> Self {
        let git = GitIndex::new(layout.root.clone());
        Self { layout, git }
    }

    pub fn dir(&self) -> PathBuf {
        self.layout.capabilities_dir()
    }

    pub fn path(&self, id: &str) -> PathBuf {
        self.layout.capability_path(id)
    }

    /// Creates `.nandi/capabilities/.gitkeep` (staged) when the folder is
    /// missing. Only in a repo that is already a git repo (so the file is
    /// staged, not left untracked); otherwise the next start retries.
    /// `true` when it created it.
    pub fn ensure_dir(&self) -> std::io::Result<bool> {
        let dir = self.dir();
        if !self.layout.root.join(".git").exists() || dir.is_dir() {
            return Ok(false);
        }
        let keep = dir.join(GITKEEP_FILE);
        write_if_changed(&keep, b"")?;
        self.git.stage_add(&keep);
        Ok(true)
    }

    /// Every `<id>.yaml` (or `.yml`), parsed and validated; problems are
    /// logged and skipped. Sorted by ID.
    pub fn load_all(&self) -> Result<Vec<CapabilityDefinition>, String> {
        let entries = match std::fs::read_dir(self.dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("Failed to read {}: {e}", self.dir().display())),
        };
        let mut definitions = Vec::new();
        for path in entries.filter_map(Result::ok).map(|e| e.path()) {
            if self.id_of(&path).is_none() {
                continue;
            }
            match self.load_path(&path) {
                Ok(definition) => definitions.push(definition),
                Err(e) => log::warn!("Skipping capability {}: {e}", path.display()),
            }
        }
        definitions.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(definitions)
    }

    pub fn load(&self, id: &str) -> Result<CapabilityDefinition, String> {
        let path = self.existing_path(id);
        if !path.is_file() {
            return Err(format!("Capability '{id}' not found"));
        }
        self.load_path(&path)
    }

    /// Parses and validates one file
    pub fn load_path(&self, path: &Path) -> Result<CapabilityDefinition, String> {
        let stem = self
            .id_of(path)
            .ok_or_else(|| format!("{} isn't a capability file", path.display()))?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        load_text(&text, &stem)
    }

    /// Validate, write atomically, stage. `true` when the file changed.
    pub fn save(&self, definition: &CapabilityDefinition) -> Result<bool, String> {
        definition::validate(definition)?;
        let text = definition::serialize(definition)?;
        let path = self.path(&definition.id);
        let changed = write_if_changed(&path, text.as_bytes())
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        if changed {
            self.git.stage_add(&path);
        }
        // A hand-written `.yml` of the same ID is replaced by the `.yaml`
        let yml = path.with_extension("yml");
        if remove_if_exists(&yml).unwrap_or(false) {
            self.git.stage_remove(&yml);
        }
        Ok(changed)
    }

    /// Remove the file and stage the removal. `true` when it existed.
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut removed = false;
        for path in [self.path(id), self.path(id).with_extension("yml")] {
            if remove_if_exists(&path)
                .map_err(|e| format!("Failed to remove {}: {e}", path.display()))?
            {
                self.git.stage_remove(&path);
                removed = true;
            }
        }
        Ok(removed)
    }

    /// The ID of a path directly under `dir()`, if it is a capability file
    /// (`<id>.yaml` / `<id>.yml` with a valid ID; not hidden or temp files)
    pub fn id_of(&self, path: &Path) -> Option<String> {
        let dir = self.dir();
        let parent = path.parent()?;
        let same_dir = parent == dir
            || parent
                .canonicalize()
                .ok()
                .zip(dir.canonicalize().ok())
                .is_some_and(|(a, b)| a == b);
        if !same_dir {
            return None;
        }
        let name = path.file_name()?.to_str()?;
        let stem = name
            .strip_suffix(".yaml")
            .or_else(|| name.strip_suffix(".yml"))?;
        definition::valid_id(stem).then(|| stem.to_string())
    }

    /// The `.yaml` path, or the `.yml` one when only that exists
    fn existing_path(&self, id: &str) -> PathBuf {
        let path = self.path(id);
        let yml = path.with_extension("yml");
        if !path.is_file() && yml.is_file() {
            yml
        } else {
            path
        }
    }
}

/// Parses `text` as the file named `stem`, and validates it
pub fn load_text(text: &str, stem: &str) -> Result<CapabilityDefinition, String> {
    let mut definition = definition::parse(text)?;
    if definition.id.trim().is_empty() {
        definition.id = stem.to_string();
    } else if definition.id != stem {
        return Err(format!(
            "the file is named '{stem}' but its id is '{}'",
            definition.id
        ));
    }
    definition::validate(&definition)?;
    Ok(definition)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> CapabilityDefinition {
        definition::parse(&format!(
            "id: {id}\nname: Test\ndescription: A test\nversion: 1.0.0\nowner: {{ kind: user, id: me, display_name: Me }}\n"
        ))
        .unwrap()
    }

    #[test]
    fn ensure_dir_creates_the_gitkeep_once() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        let repo = CapabilityRepository::new(layout.clone());
        // Not a git repo yet: left for the next start
        assert!(!repo.ensure_dir().unwrap());
        git2::Repository::init(&layout.root).unwrap();
        assert!(repo.ensure_dir().unwrap());
        assert!(repo.dir().join(GITKEEP_FILE).is_file());
        assert!(!repo.ensure_dir().unwrap());
    }

    #[test]
    fn saves_loads_and_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        let repo = CapabilityRepository::new(layout);
        let def = sample("test-cap");
        assert!(repo.save(&def).unwrap());
        assert!(!repo.save(&def).unwrap(), "same contents: unchanged");
        assert!(repo.path("test-cap").is_file());
        assert_eq!(repo.load("test-cap").unwrap(), def);
        assert_eq!(repo.load_all().unwrap(), vec![def]);
        assert!(repo.delete("test-cap").unwrap());
        assert!(!repo.delete("test-cap").unwrap());
        assert!(repo.load("test-cap").is_err());
        assert!(repo.load_all().unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_definitions_and_skips_bad_files() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        let repo = CapabilityRepository::new(layout);
        let mut bad = sample("ok");
        bad.version = "one".into();
        assert!(repo.save(&bad).is_err());
        assert!(!repo.path("ok").exists());

        std::fs::create_dir_all(repo.dir()).unwrap();
        std::fs::write(repo.dir().join("broken.yaml"), "id: [").unwrap();
        std::fs::write(
            repo.dir().join("other.yaml"),
            definition::serialize(&sample("mismatch")).unwrap(),
        )
        .unwrap();
        // No id: taken from the file name
        std::fs::write(
            repo.dir().join("named.yml"),
            "name: N\ndescription: d\nversion: 1.0.0\nowner: { kind: user, id: me, display_name: Me }\n",
        )
        .unwrap();
        std::fs::write(repo.dir().join("notes.txt"), "x").unwrap();
        let ids: Vec<String> = repo.load_all().unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["named"]);
        assert_eq!(repo.load("named").unwrap().name, "N");
    }

    #[test]
    fn id_of_only_accepts_capability_files() {
        let layout = RepoLayout::new("/r", "/trash");
        let repo = CapabilityRepository::new(layout);
        let dir = repo.dir();
        assert_eq!(repo.id_of(&dir.join("a.b.yaml")).as_deref(), Some("a.b"));
        assert_eq!(repo.id_of(&dir.join("x.yml")).as_deref(), Some("x"));
        assert_eq!(repo.id_of(&dir.join(".gitkeep")), None);
        assert_eq!(repo.id_of(&dir.join("Bad.yaml")), None);
        assert_eq!(repo.id_of(&dir.join("sub").join("a.yaml")), None);
        assert_eq!(repo.id_of(Path::new("/r/a.yaml")), None);
    }
}
