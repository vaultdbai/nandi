//! My notes: the user's own Markdown notes (role, what they own, deadlines,
//! contacts), which Chief of Staff answers from. They are one folder,
//! shared with agents as [`NOTES_FOLDER_ID`] (read-only) through the
//! `filesystem` connector, so the My notes page, the share and the agents
//! all use the same files.
//!
//! The folder is the one already shared as `my-notes`, or, when there is
//! none, `~/.nandi/notes` (machine-local, never a gold-copy definition),
//! created empty on first use and shared read and write. Chief of Staff
//! decides what to keep there and how to arrange it (its skill.md says
//! how), and keeps it current as it learns things. Writes
//! there don't ask ([`is_notes_write`]): letting agents update the notes
//! is the user's one decision (`set_agents_can_write`). Notes are `.md`
//! files, named by their path inside the folder (`deadlines.md`,
//! `runbooks/etl.md`).
//!
//! What others may see about the user (what they own) is published from
//! NANDI.md's `responsibilities`, not from here: the notes stay private
//! unless Chief of Staff quotes them in an answer.

pub mod corrections;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::connectors::{self, folders};
use super::AppState;
use crate::model::Timestamp;
use crate::utils::nandi_config;

/// The ID agents know the notes folder by (Chief of Staff's skill.md)
pub const NOTES_FOLDER_ID: &str = "my-notes";

/// The managed folder, under the Nandi home
const MANAGED_DIR: &str = "notes";

/// Longest note name, folders included
const MAX_NAME_CHARS: usize = 120;
/// Deepest note: `a/b/c.md`
const MAX_DEPTH: usize = 3;
/// Largest note
const MAX_NOTE_BYTES: usize = 256 * 1024;
/// Most notes listed
const MAX_NOTES: usize = 500;

/// One note, as the My notes page lists it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoteFile {
    /// Its path in the folder, with `/` (`runbooks/etl.md`)
    pub name: String,
    /// Its first `# ` heading, when it has one
    pub title: Option<String>,
    pub updated_at: Timestamp,
    pub size: u32,
}

/// The notes folder and what's in it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MyNotes {
    /// Where the folder is on this computer
    pub path: String,
    /// `true`: Nandi's own folder; `false`: one the user shared
    pub managed: bool,
    /// Agents may update the notes (the share is read and write)
    pub agents_can_write: bool,
    /// Sorted by name
    pub notes: Vec<NoteFile>,
}

// -- The folder ---------------------------------------------------------------

/// A notes folder on disk. Every name is checked ([`check_name`]) before
/// it touches the disk, so nothing outside the folder is read or written.
#[derive(Debug, Clone)]
pub struct NotesFolder {
    root: PathBuf,
}

impl NotesFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every `.md` note, sorted by name (hidden files and folders skipped)
    pub fn list(&self) -> Result<Vec<NoteFile>, String> {
        let mut notes = Vec::new();
        self.walk(&self.root, "", 1, &mut notes)?;
        notes.sort_by(|a, b| a.name.cmp(&b.name));
        notes.truncate(MAX_NOTES);
        Ok(notes)
    }

    fn walk(
        &self,
        dir: &Path,
        prefix: &str,
        depth: usize,
        notes: &mut Vec<NoteFile>,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("Can't read the notes folder {}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') || notes.len() >= MAX_NOTES {
                continue;
            }
            let name = format!("{prefix}{file_name}");
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < MAX_DEPTH {
                self.walk(&entry.path(), &format!("{name}/"), depth + 1, notes)?;
            } else if kind.is_file() && check_name(&name).is_ok() {
                notes.push(self.describe(&name, &entry.path())?);
            }
        }
        Ok(())
    }

    fn describe(&self, name: &str, path: &Path) -> Result<NoteFile, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("Can't read {name}: {e}"))?;
        let updated_at = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0.0, |d| d.as_millis() as f64);
        let title = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| title_of(&text));
        Ok(NoteFile {
            name: name.to_string(),
            title,
            updated_at,
            size: meta.len().min(u32::MAX as u64) as u32,
        })
    }

    pub fn read(&self, name: &str) -> Result<String, String> {
        let path = self.path_of(name)?;
        std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("There's no note '{name}'"),
            _ => format!("Can't read '{name}': {e}"),
        })
    }

    /// Writes the note (creating its folders), ending it with one newline.
    /// Written to a temporary file first, so a crash never leaves half a
    /// note.
    pub fn save(&self, name: &str, content: &str) -> Result<NoteFile, String> {
        let path = self.path_of(name)?;
        if content.len() > MAX_NOTE_BYTES {
            return Err(format!(
                "'{name}' is too long: notes are at most {} KB",
                MAX_NOTE_BYTES / 1024
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Can't create the folder for '{name}': {e}"))?;
        }
        let text = format!("{}\n", content.trim_end());
        super::live::own_writes::note(&path);
        let temporary = path.with_extension("md.tmp");
        std::fs::write(&temporary, text).map_err(|e| format!("Can't save '{name}': {e}"))?;
        std::fs::rename(&temporary, &path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            format!("Can't save '{name}': {e}")
        })?;
        self.describe(name, &path)
    }

    pub fn delete(&self, name: &str) -> Result<(), String> {
        let path = self.path_of(name)?;
        super::live::own_writes::note(&path);
        std::fs::remove_file(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("There's no note '{name}'"),
            _ => format!("Can't delete '{name}': {e}"),
        })
    }

    fn path_of(&self, name: &str) -> Result<PathBuf, String> {
        check_name(name)?;
        Ok(name
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part)))
    }
}

/// A note's name: a relative `.md` path with `/`, at most
/// [`MAX_DEPTH`] deep. Each part is letters, digits, spaces, `-`, `_` or
/// `.`, and doesn't start with a dot or a space.
pub fn check_name(name: &str) -> Result<(), String> {
    let bad = || {
        format!(
            "'{name}' isn't a note name: use letters, digits, spaces, '-' and '_', ending in .md"
        )
    };
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(bad());
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return Err(format!("'{name}' is too deep: at most {MAX_DEPTH} levels"));
    }
    let valid_part = |part: &str| {
        !part.is_empty()
            && !part.starts_with(['.', ' '])
            && !part.ends_with(' ')
            && part
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
    };
    let file = parts.last().copied().unwrap_or_default();
    if !parts.iter().all(|part| valid_part(part))
        || !file.to_ascii_lowercase().ends_with(".md")
        || file.len() <= 3
    {
        return Err(bad());
    }
    Ok(())
}

/// The text of the first `# ` heading
fn title_of(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

// -- Agents keep the notes ---------------------------------------------------

/// Whether a planned tool call writes inside the notes folder: those don't
/// ask. Writing there at all needs the user to have let agents update their
/// notes (the share is read-only otherwise, and the connector refuses).
/// The path must name the folder (`my-notes/...`) and can't climb out.
pub fn is_notes_write(tool: &str, args: &serde_json::Value) -> bool {
    let Some(("filesystem", "write_file")) = tool.split_once("__") else {
        return false;
    };
    let Some(path) = args.get("path").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let path = path.trim().replace('\\', "/");
    path.strip_prefix(&format!("{NOTES_FOLDER_ID}/"))
        .is_some_and(|rest| check_name(rest).is_ok())
}

// -- The service --------------------------------------------------------------

/// The notes folder: the one shared as `my-notes`, else the managed one,
/// created and shared now. Returns it and whether it's the managed one.
pub async fn folder(state: &AppState) -> Result<(NotesFolder, bool), String> {
    if let Some(shared) = connectors::shared_folder(state, NOTES_FOLDER_ID) {
        let root = connectors::expand_home(&shared.path);
        let managed = managed_dir().is_ok_and(|dir| dir == root);
        if !root.is_dir() {
            std::fs::create_dir_all(&root).map_err(|e| {
                format!(
                    "The notes folder {} is missing and can't be created: {e}",
                    root.display()
                )
            })?;
        }
        return Ok((NotesFolder::new(root), managed));
    }
    let root = managed_dir()?;
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Can't create the notes folder {}: {e}", root.display()))?;
    let folder = NotesFolder::new(root.clone());
    connectors::share_folder(
        state,
        folders::SharedFolder {
            id: NOTES_FOLDER_ID.to_string(),
            path: root.to_string_lossy().to_string(),
            access: folders::Access::Write,
        },
    )
    .await?;
    log::info!("Created the notes folder {}", root.display());
    Ok((folder, true))
}

/// Lets agents update the notes (without asking each time), or makes the
/// share read-only again
pub async fn set_agents_can_write(state: &AppState, allow: bool) -> Result<MyNotes, String> {
    self::folder(state).await?;
    let mut shared =
        connectors::shared_folder(state, NOTES_FOLDER_ID).ok_or("The notes folder isn't shared")?;
    shared.access = if allow {
        folders::Access::Write
    } else {
        folders::Access::Read
    };
    connectors::share_folder(state, shared).await?;
    my_notes(state).await
}

fn managed_dir() -> Result<PathBuf, String> {
    Ok(nandi_config::nandi_dir()?.join(MANAGED_DIR))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("Notes task failed: {e}"))?
}

/// Watches the notes folder (and the agents' work folders) for the live
/// UI (`services::live`); a no-op when they're watched already
pub fn watch(state: &AppState, folder: &NotesFolder) {
    // `~/.nandi/local/work` (`nandi_config::work_dir`), or the test's own
    let work = state.nandi_dir().join("local").join("work");
    if let Err(e) = std::fs::create_dir_all(&work) {
        log::warn!("Can't create {}: {e}", work.display());
    }
    state
        .live()
        .watch_folders(state, folder.root().to_path_buf(), work);
}

/// The folder and its notes (creating and sharing the folder on first use)
pub async fn my_notes(state: &AppState) -> Result<MyNotes, String> {
    let (folder, managed) = self::folder(state).await?;
    // The folder may have been shared anew since the app started
    watch(state, &folder);
    // One kind of separator, however the home was given (`D:/x` + `notes`)
    let path = folder
        .root()
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .to_string();
    let notes = blocking(move || folder.list()).await?;
    let agents_can_write = connectors::shared_folder(state, NOTES_FOLDER_ID)
        .is_some_and(|shared| shared.access == folders::Access::Write);
    Ok(MyNotes {
        path,
        managed,
        agents_can_write,
        notes,
    })
}

pub async fn read_note(state: &AppState, name: String) -> Result<String, String> {
    let (folder, _) = self::folder(state).await?;
    blocking(move || folder.read(&name)).await
}

pub async fn save_note(
    state: &AppState,
    name: String,
    content: String,
) -> Result<NoteFile, String> {
    let (folder, _) = self::folder(state).await?;
    let saved = {
        let name = name.clone();
        blocking(move || {
            // The person removing or changing a cited line is a correction
            // the Chief of Staff remembers
            let tracked = name != corrections::CORRECTIONS && name != corrections::SUGGESTIONS;
            let before = if tracked {
                folder.read(&name).ok()
            } else {
                None
            };
            let saved = folder.save(&name, &content)?;
            if let Some(before) = before {
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                let lines: Vec<String> = corrections::removed_lines(&before, &content)
                    .into_iter()
                    .map(|line| format!("- Removed from {name}: {line} ({today})"))
                    .collect();
                if !lines.is_empty() {
                    let current = folder.read(corrections::CORRECTIONS).unwrap_or_default();
                    folder.save(
                        corrections::CORRECTIONS,
                        &corrections::append(&current, &lines),
                    )?;
                }
            }
            Ok(saved)
        })
        .await?
    };
    Ok(saved)
}

/// The Chief of Staff's open capability suggestions (`suggestions.md`)
pub async fn suggestions(state: &AppState) -> Result<Vec<corrections::Suggestion>, String> {
    let (folder, _) = self::folder(state).await?;
    blocking(move || {
        Ok(corrections::open_suggestions(
            &folder.read(corrections::SUGGESTIONS).unwrap_or_default(),
        ))
    })
    .await
}

/// Accepts or dismisses a suggestion; either is remembered in
/// `corrections.md`. Returns the suggestion's text (accepted: what to build)
pub async fn answer_suggestion(
    state: &AppState,
    id: String,
    accept: bool,
) -> Result<String, String> {
    let (folder, _) = self::folder(state).await?;
    let text = blocking(move || {
        let content = folder.read(corrections::SUGGESTIONS).unwrap_or_default();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let (file, line, text) = corrections::answer(&content, &id, accept, &today)
            .ok_or("That suggestion isn't open any more")?;
        folder.save(corrections::SUGGESTIONS, &file)?;
        let current = folder.read(corrections::CORRECTIONS).unwrap_or_default();
        folder.save(
            corrections::CORRECTIONS,
            &corrections::append(&current, &[line]),
        )?;
        Ok(text)
    })
    .await?;
    Ok(text)
}

pub async fn delete_note(state: &AppState, name: String) -> Result<(), String> {
    let (folder, _) = self::folder(state).await?;
    blocking(move || folder.delete(&name)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> (tempfile::TempDir, NotesFolder) {
        let tmp = tempfile::tempdir().unwrap();
        let folder = NotesFolder::new(tmp.path());
        (tmp, folder)
    }

    #[test]
    fn notes_are_saved_listed_read_and_deleted() {
        let (_tmp, notes) = folder();
        let saved = notes
            .save(
                "deadlines.md",
                "# Deadlines\n\n- SOC 1 walkthrough: Nov 12\n\n\n",
            )
            .unwrap();
        assert_eq!(saved.title.as_deref(), Some("Deadlines"));
        notes.save("runbooks/etl nightly.md", "Restart it").unwrap();
        assert_eq!(
            notes.read("deadlines.md").unwrap(),
            "# Deadlines\n\n- SOC 1 walkthrough: Nov 12\n"
        );
        let names: Vec<String> = notes.list().unwrap().into_iter().map(|n| n.name).collect();
        assert_eq!(names, ["deadlines.md", "runbooks/etl nightly.md"]);

        notes.delete("deadlines.md").unwrap();
        assert!(notes.read("deadlines.md").unwrap_err().contains("no note"));
        assert!(notes.delete("deadlines.md").is_err());
    }

    #[test]
    fn only_markdown_notes_are_listed() {
        let (tmp, notes) = folder();
        std::fs::write(tmp.path().join("photo.png"), "x").unwrap();
        std::fs::write(tmp.path().join(".hidden.md"), "x").unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join(".git/HEAD.md"), "x").unwrap();
        std::fs::write(tmp.path().join("Role.MD"), "no heading").unwrap();
        let listed = notes.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Role.MD");
        assert_eq!(listed[0].title, None);
    }

    #[test]
    fn names_stay_inside_the_folder() {
        for bad in [
            "",
            "../secrets.md",
            "a/../../b.md",
            "/etc/passwd.md",
            "C:\\x.md",
            ".hidden.md",
            "notes.txt",
            ".md",
            "a/b/c/d.md",
            "a//b.md",
            " x.md",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        for good in [
            "about-me.md",
            "runbooks/etl nightly.md",
            "Q4 plan.v2.md",
            "a/b/c.md",
        ] {
            assert!(check_name(good).is_ok(), "{good:?}");
        }
        let (_tmp, notes) = folder();
        assert!(notes.save("../out.md", "x").is_err());
        assert!(notes.read("../out.md").is_err());
    }

    #[test]
    fn writes_inside_the_notes_folder_dont_ask() {
        use serde_json::json;
        let write = |path: &str| {
            is_notes_write(
                "filesystem__write_file",
                &json!({"path": path, "content": "x"}),
            )
        };
        assert!(write("my-notes/deadlines.md"));
        assert!(write("my-notes/runbooks/etl.md"));
        assert!(write("my-notes\\owners.md"));
        // Anywhere else, or anything else, still asks
        assert!(!write("docs/deadlines.md"));
        assert!(!write("deadlines.md"));
        assert!(!write("my-notes/../docs/x.md"));
        assert!(!write("my-notes/script.sh"));
        assert!(!is_notes_write(
            "filesystem__read_file",
            &json!({"path": "my-notes/a.md"})
        ));
        assert!(!is_notes_write(
            "github__write_file",
            &json!({"path": "my-notes/a.md"})
        ));
        assert!(!is_notes_write("filesystem__write_file", &json!({})));
    }

    #[test]
    fn a_note_has_a_size_limit() {
        let (_tmp, notes) = folder();
        let error = notes
            .save("big.md", &"x".repeat(MAX_NOTE_BYTES + 1))
            .unwrap_err();
        assert!(error.contains("too long"), "{error}");
    }

    #[tokio::test]
    async fn the_notes_folder_is_shared_with_agents_on_first_use() {
        let state = AppState::for_tests(vec![]);
        // A folder the user already shared as my-notes is the notes folder
        let tmp = tempfile::tempdir().unwrap();
        connectors::share_folder(
            &state,
            folders::SharedFolder {
                id: NOTES_FOLDER_ID.into(),
                path: tmp.path().to_string_lossy().to_string(),
                access: folders::Access::Read,
            },
        )
        .await
        .unwrap();
        save_note(&state, "about-me.md".into(), "# About me".into())
            .await
            .unwrap();
        let mine = my_notes(&state).await.unwrap();
        assert!(!mine.managed);
        assert_eq!(mine.notes.len(), 1);
        assert!(tmp.path().join("about-me.md").is_file());
        let shared = connectors::shared_folder(&state, NOTES_FOLDER_ID).unwrap();
        assert_eq!(shared.access, folders::Access::Read);
        assert!(!mine.agents_can_write);
        // The user lets Chief of Staff keep them current
        let mine = set_agents_can_write(&state, true).await.unwrap();
        assert!(mine.agents_can_write);
        let shared = connectors::shared_folder(&state, NOTES_FOLDER_ID).unwrap();
        assert_eq!(shared.access, folders::Access::Write);
        // Stored portably (`~/…` here), the same folder on this machine
        assert_eq!(connectors::expand_home(&shared.path), tmp.path());
        state.shutdown();
    }
}
