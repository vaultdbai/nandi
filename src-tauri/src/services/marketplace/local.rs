//! Starter packs from this computer: a folder, or a `.zip` of one, read
//! into the same [`FileSet`] a registry install reads from its git
//! checkout. No network, no relay: the installer validates, vendors and
//! records it like any other pack (`Installer::install_local_pack`).
//!
//! Reading is defensive: `.git/` is skipped, nothing may point outside the
//! pack (no `..`, no absolute names, no symlinks leaving the folder), and
//! the whole pack must fit in [`MAX_FOLDER_BYTES`].

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::manifest::PACK_MANIFEST_PATH;
use crate::services::repos::vendor_repo::{FileSet, MAX_FOLDER_BYTES};

/// Folders that never belong to a pack
const SKIPPED_DIRS: [&str; 2] = [".git", "__MACOSX"];

/// A pack on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalPackSource {
    Folder(PathBuf),
    Zip(PathBuf),
}

impl LocalPackSource {
    /// A folder, or a file ending in `.zip`. The path is made absolute.
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.as_os_str().is_empty() {
            return Err("Choose a pack folder or .zip".to_string());
        }
        let path =
            fs::canonicalize(path).map_err(|e| format!("Can't open {}: {e}", path.display()))?;
        let path = plain_path(path);
        if path.is_dir() {
            return Ok(Self::Folder(path));
        }
        let is_zip = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
        if path.is_file() && is_zip {
            return Ok(Self::Zip(path));
        }
        Err(format!(
            "{} isn't a folder or a .zip of a pack",
            path.display()
        ))
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Folder(path) | Self::Zip(path) => path,
        }
    }

    /// Every file of the pack, by `/`-separated path from the pack's root
    pub fn files(&self) -> Result<FileSet, String> {
        match self {
            Self::Folder(root) => {
                let mut files = FileSet::new();
                let mut total = 0;
                read_folder(root, root, &mut files, &mut total)?;
                Ok(files)
            }
            Self::Zip(path) => read_zip(path).map(unwrap_single_folder),
        }
    }
}

/// `\\?\C:\…` (what `canonicalize` gives on Windows) → `C:\…`
fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

fn too_large() -> String {
    format!(
        "The pack is larger than {} MB",
        MAX_FOLDER_BYTES / (1024 * 1024)
    )
}

fn add(files: &mut FileSet, total: &mut usize, name: String, bytes: Vec<u8>) -> Result<(), String> {
    *total += bytes.len();
    if *total > MAX_FOLDER_BYTES {
        return Err(too_large());
    }
    files.insert(name, bytes);
    Ok(())
}

fn read_folder(
    root: &Path,
    dir: &Path,
    files: &mut FileSet,
    total: &mut usize,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("Can't read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Can't read {}: {e}", dir.display()))?;
        let name = entry.file_name();
        if SKIPPED_DIRS.iter().any(|skipped| name == *skipped) {
            continue;
        }
        let mut path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("Can't read {}: {e}", path.display()))?;
        if kind.is_symlink() {
            let target = fs::canonicalize(&path)
                .map(plain_path)
                .map_err(|e| format!("Can't follow {}: {e}", path.display()))?;
            if !target.starts_with(root) {
                return Err(format!(
                    "{} points outside the pack folder",
                    relative(root, &path)?
                ));
            }
            path = target;
        }
        if path.is_dir() {
            if kind.is_symlink() {
                // Its files are read where it points, inside the pack
                continue;
            }
            read_folder(root, &path, files, total)?;
        } else {
            let bytes =
                fs::read(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
            add(files, total, relative(root, &entry.path())?, bytes)?;
        }
    }
    Ok(())
}

/// `path` under `root`, `/`-separated
fn relative(root: &Path, path: &Path) -> Result<String, String> {
    let inner = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is outside the pack folder", path.display()))?;
    let parts: Option<Vec<&str>> = inner
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    parts
        .map(|parts| parts.join("/"))
        .ok_or_else(|| format!("{} has a name Nandi can't read", path.display()))
}

fn read_zip(path: &Path) -> Result<FileSet, String> {
    let file = fs::File::open(path).map_err(|e| format!("Can't open {}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} isn't a readable .zip: {e}", path.display()))?;
    let mut files = FileSet::new();
    let mut total = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("{} isn't a readable .zip: {e}", path.display()))?;
        if entry.is_dir() {
            continue;
        }
        let raw = entry
            .name()
            .map_err(|e| format!("{} has an unreadable file name: {e}", path.display()))?
            .to_string();
        let name = entry
            .enclosed_name()
            .and_then(|inner| relative(Path::new(""), &inner).ok())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("The .zip has a file outside the pack: {raw}"))?;
        if name.split('/').any(|part| SKIPPED_DIRS.contains(&part)) {
            continue;
        }
        if entry.is_symlink() {
            return Err(format!(
                "The .zip has a link, which a pack can't have: {name}"
            ));
        }
        if usize::try_from(entry.size()).map_or(true, |size| size > MAX_FOLDER_BYTES - total) {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        // Never trust the declared size: read at most what's still allowed
        let limit = u64::try_from(MAX_FOLDER_BYTES - total + 1).unwrap_or(u64::MAX);
        (&mut entry)
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Can't read {name} from the .zip: {e}"))?;
        add(&mut files, &mut total, name, bytes)?;
    }
    Ok(files)
}

/// A .zip of the pack folder itself ("Download ZIP" on GitHub): the pack
/// is the one folder at its root
fn unwrap_single_folder(files: FileSet) -> FileSet {
    if files.contains_key(PACK_MANIFEST_PATH) {
        return files;
    }
    let mut tops = files
        .keys()
        .map(|path| path.split_once('/').map(|(top, _)| top));
    let Some(Some(top)) = tops.next() else {
        return files;
    };
    let top = top.to_string();
    if !tops.all(|other| other == Some(top.as_str())) {
        return files;
    }
    let prefix = format!("{top}/");
    let inner: FileSet = files
        .iter()
        .filter_map(|(path, bytes)| Some((path.strip_prefix(&prefix)?.to_string(), bytes.clone())))
        .collect();
    if inner.contains_key(PACK_MANIFEST_PATH) {
        inner
    } else {
        files
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    fn write(root: &Path, files: &[(&str, &str)]) {
        for (path, content) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }

    /// A .zip with `files` (`name` as written, so `../x` can be tested)
    pub(crate) fn zip_of(path: &Path, files: &[(&str, &str)]) {
        let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn a_folder_reads_as_relative_paths_without_git() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            &[
                (".nandi/manifest.yaml", "id: p"),
                ("agents/a/skill.md", "x"),
                (".git/HEAD", "ref"),
            ],
        );
        let source = LocalPackSource::open(tmp.path()).unwrap();
        assert!(matches!(source, LocalPackSource::Folder(_)));
        assert!(source.path().is_absolute());
        let files = source.files().unwrap();
        let names: Vec<&str> = files.keys().map(String::as_str).collect();
        assert_eq!(names, [".nandi/manifest.yaml", "agents/a/skill.md"]);
    }

    #[test]
    fn a_zip_with_one_wrapping_folder_is_unwrapped() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("pack.zip");
        zip_of(
            &zip,
            &[
                ("pack-main/.nandi/manifest.yaml", "id: p"),
                ("pack-main/agents/a/skill.md", "x"),
                ("pack-main/.git/HEAD", "ref"),
            ],
        );
        let files = LocalPackSource::open(&zip).unwrap().files().unwrap();
        let names: Vec<&str> = files.keys().map(String::as_str).collect();
        assert_eq!(names, [".nandi/manifest.yaml", "agents/a/skill.md"]);
    }

    #[test]
    fn a_zip_entry_outside_the_pack_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let zip = tmp.path().join("evil.zip");
        zip_of(
            &zip,
            &[(".nandi/manifest.yaml", "id: p"), ("../escape.txt", "x")],
        );
        let err = LocalPackSource::open(&zip).unwrap().files().unwrap_err();
        assert!(err.contains("outside the pack"), "{err}");
    }

    #[test]
    fn other_files_and_missing_paths_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let text = tmp.path().join("notes.txt");
        fs::write(&text, "x").unwrap();
        assert!(LocalPackSource::open(&text)
            .unwrap_err()
            .contains("isn't a folder or a .zip"));
        assert!(LocalPackSource::open(&tmp.path().join("missing")).is_err());
        assert!(LocalPackSource::open(Path::new("")).is_err());
    }
}
