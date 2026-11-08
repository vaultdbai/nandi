//! The `filesystem` connector: the container's file tools (`read_file`,
//! `write_file`, `list_dir`), but on the host folders the user shared
//! (`folders`). Each agent sees only the folders its skill.md names, at
//! the access it was given (`folders::granted`). Paths are
//! `<folder>/<path inside it>` and can't leave the folder, not even
//! through a symlink; with one folder, the folder can be left out.

use std::path::{Component, Path, PathBuf};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::folders::{self, Access, SharedFolder};
use super::{Connector, ToolDef};

/// Larger files are refused rather than flooding the model
const MAX_READ_BYTES: u64 = 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 1000;

/// One shared folder, opened
#[derive(Debug, Clone)]
struct Root {
    id: String,
    /// Canonical, so containment checks compare like with like
    path: PathBuf,
    access: Access,
}

pub struct FilesystemConnector {
    /// Every folder on the list that exists; agents get a subset
    roots: Vec<Root>,
    sensitive: Vec<String>,
}

/// `path` must be an existing absolute folder; `~` is the home folder
fn open(folder: &SharedFolder) -> Result<Root, String> {
    let expanded = expand_home(&folder.path);
    if !expanded.is_absolute() {
        return Err(format!(
            "The shared folder must be an absolute path: '{}'",
            folder.path
        ));
    }
    let path = std::fs::canonicalize(&expanded)
        .map_err(|e| format!("Can't open the shared folder '{}': {e}", folder.path))?;
    if !path.is_dir() {
        return Err(format!(
            "The shared folder '{}' isn't a folder",
            folder.path
        ));
    }
    Ok(Root {
        id: folder.id.clone(),
        path,
        access: folder.access,
    })
}

impl FilesystemConnector {
    /// Opens every folder on the list. Missing ones are left out (and
    /// logged); none at all is an error.
    pub fn new(list: &[SharedFolder], sensitive: &[String]) -> Result<Self, String> {
        let mut roots = Vec::new();
        let mut first_error = None;
        for folder in list {
            match open(folder) {
                Ok(root) => roots.push(root),
                Err(e) => {
                    log::warn!("Shared folder '{}' left out: {e}", folder.id);
                    first_error.get_or_insert(e);
                }
            }
        }
        if roots.is_empty() {
            return Err(
                first_error.unwrap_or_else(|| "Add a folder to share with agents".to_string())
            );
        }
        Ok(Self {
            roots,
            sensitive: sensitive.to_vec(),
        })
    }

    pub fn tool_defs() -> Vec<ToolDef> {
        let path = json!({"type": "string", "description": "<folder>/<path inside it>, e.g. docs/notes.md. With one folder, the folder can be left out."});
        vec![
            ToolDef {
                name: "read_file".into(),
                description: "Read a UTF-8 text file from a folder the user shared with you.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"path": path},
                    "required": ["path"],
                }),
                sensitive: false,
            },
            ToolDef {
                name: "write_file".into(),
                description:
                    "Create or overwrite a text file in a folder the user shared with you for writing (creates parent folders)."
                        .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"path": path, "content": {"type": "string"}},
                    "required": ["path", "content"],
                }),
                sensitive: true,
            },
            ToolDef {
                name: "list_dir".into(),
                description:
                    "List a folder (folders end with '/'). Without a path, lists the folders shared with you and whether you can write to each."
                        .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"path": path},
                }),
                sensitive: false,
            },
        ]
    }

    /// The roots an agent may use (`options`: its `connectors:` item)
    fn scope(&self, options: Option<&serde_yaml::Value>) -> Result<Vec<Root>, String> {
        let list: Vec<SharedFolder> = self
            .roots
            .iter()
            .map(|root| SharedFolder {
                id: root.id.clone(),
                path: String::new(),
                access: root.access,
            })
            .collect();
        let grants = folders::grants(options)?;
        Ok(folders::granted(&list, &grants)
            .into_iter()
            .filter_map(|granted| {
                let root = self.roots.iter().find(|r| r.id == granted.id)?;
                Some(Root {
                    access: granted.access,
                    ..root.clone()
                })
            })
            .collect())
    }

    fn run(&self, name: &str, args: &Value, roots: &[Root]) -> Result<Value, String> {
        if roots.is_empty() {
            return Err(
                "No folders are shared with you. The user gives an agent folders on its page, or in its skill.md (`connectors: [{id: filesystem, folders: [...]}]`)."
                    .to_string(),
            );
        }
        let path = args.get("path").and_then(Value::as_str).map(str::trim);
        match name {
            "list_dir" if path.is_none_or(|p| p.is_empty() || p == "." || p == "/") => {
                Ok(Value::String(
                    roots
                        .iter()
                        .map(|r| format!("{}/ ({})", r.id, r.access.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ))
            }
            "read_file" | "write_file" | "list_dir" => {
                let (root, relative) = locate(roots, path.unwrap_or_default())?;
                match name {
                    "read_file" => root.read_file(relative),
                    "list_dir" => root.list_dir(relative),
                    _ => {
                        if root.access != Access::Write {
                            return Err(format!(
                                "'{}' is shared with you read-only: you can't write to it",
                                root.id
                            ));
                        }
                        let content = args
                            .get("content")
                            .and_then(Value::as_str)
                            .ok_or("Missing string argument 'content'")?;
                        // The user's notes never keep card numbers or passwords
                        if root.id == crate::services::notes::NOTES_FOLDER_ID {
                            crate::services::domain::note_guard::check(content)?;
                        }
                        root.write_file(relative, content)
                    }
                }
            }
            other => Err(format!("Unknown tool '{other}'")),
        }
    }
}

/// The folder a tool path names, and the path inside it
fn locate<'a>(roots: &'a [Root], path: &'a str) -> Result<(&'a Root, &'a str), String> {
    let path = path.trim_start_matches(['/', '\\']);
    let (first, rest) = match path.find(['/', '\\']) {
        Some(at) => (&path[..at], &path[at + 1..]),
        None => (path, ""),
    };
    if let Some(root) = roots.iter().find(|r| r.id == first) {
        return Ok((root, if rest.is_empty() { "." } else { rest }));
    }
    match roots {
        [only] => Ok((only, path)),
        _ => Err(format!(
            "Start the path with one of your folders: {}",
            roots
                .iter()
                .map(|r| format!("{}/", r.id))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

impl Root {
    /// `relative` inside the root, refusing absolute paths and `..`.
    fn lexical(&self, relative: &str) -> Result<PathBuf, String> {
        let relative = relative.trim();
        let mut path = self.path.clone();
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(part) => path.push(part),
                Component::CurDir => {}
                _ => {
                    return Err(format!(
                        "Path '{relative}' must be inside the shared folder, without '..'"
                    ))
                }
            }
        }
        Ok(path)
    }

    /// An existing path, resolved through symlinks, still inside the root.
    fn existing(&self, relative: &str) -> Result<PathBuf, String> {
        let path = self.lexical(relative)?;
        let resolved =
            std::fs::canonicalize(&path).map_err(|e| format!("Can't open '{relative}': {e}"))?;
        self.contain(relative, resolved)
    }

    fn contain(&self, relative: &str, resolved: PathBuf) -> Result<PathBuf, String> {
        if resolved.starts_with(&self.path) {
            Ok(resolved)
        } else {
            Err(format!("Path '{relative}' is outside the shared folder"))
        }
    }

    fn read_file(&self, relative: &str) -> Result<Value, String> {
        let path = self.existing(relative)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("Can't read '{relative}': {e}"))?;
        if !meta.is_file() {
            return Err(format!("'{relative}' isn't a file"));
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(format!(
                "'{relative}' is {} bytes; the limit is {MAX_READ_BYTES}",
                meta.len()
            ));
        }
        std::fs::read_to_string(&path)
            .map(Value::String)
            .map_err(|e| format!("Can't read '{relative}' as text: {e}"))
    }

    fn write_file(&self, relative: &str, content: &str) -> Result<Value, String> {
        let path = self.lexical(relative)?;
        if path == self.path {
            return Err("Give a file path to write".to_string());
        }
        let parent = path.parent().unwrap_or(&self.path);
        // Check the deepest existing ancestor before creating anything
        let mut ancestor = parent;
        while !ancestor.exists() {
            ancestor = ancestor.parent().unwrap_or(&self.path);
        }
        let resolved = std::fs::canonicalize(ancestor).map_err(|e| e.to_string())?;
        self.contain(relative, resolved)?;
        std::fs::create_dir_all(parent).map_err(|e| format!("Can't create folders: {e}"))?;
        if path.exists() {
            // Refuse writing through a symlink that points outside
            self.existing(relative)?;
        }
        std::fs::write(&path, content).map_err(|e| format!("Can't write '{relative}': {e}"))?;
        Ok(json!({"written": content.len(), "path": format!("{}/{relative}", self.id)}))
    }

    fn list_dir(&self, relative: &str) -> Result<Value, String> {
        let path = self.existing(relative)?;
        let mut entries: Vec<String> = std::fs::read_dir(&path)
            .map_err(|e| format!("Can't list '{relative}': {e}"))?
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        entries.sort();
        let truncated = entries.len() > MAX_LIST_ENTRIES;
        entries.truncate(MAX_LIST_ENTRIES);
        let mut listing = entries.join("\n");
        if truncated {
            listing.push_str(&format!("\n[... more than {MAX_LIST_ENTRIES} entries]"));
        }
        Ok(Value::String(listing))
    }
}

/// A shared folder's path on this computer: `~` is the user's home, and a
/// relative path is under the Nandi home (`notes` is `~/.nandi/notes`),
/// so the same `connectors.yaml` works on another machine
/// (`folders::portable` stores paths that way)
pub fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => {
            match dirs::home_dir() {
                Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
                None => PathBuf::from(path),
            }
        }
        _ if is_relative(path) => match crate::utils::nandi_config::nandi_dir() {
            Ok(nandi) => nandi.join(path),
            Err(_) => PathBuf::from(path),
        },
        _ => PathBuf::from(path),
    }
}

/// Relative on every OS: no drive, no leading slash, backslash or `~`
fn is_relative(path: &str) -> bool {
    !path.is_empty() && !path.starts_with(['/', '\\', '~']) && !path.contains(':')
}

impl Connector for FilesystemConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        Self::tool_defs()
            .into_iter()
            .map(|mut tool| {
                tool.sensitive |= self.sensitive.contains(&tool.name);
                tool
            })
            .collect()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.call_tool_for(name, args, None)
    }

    fn call_tool_for<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
        options: Option<&'a serde_yaml::Value>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let roots = self.scope(options)?;
            let this = Self {
                roots: Vec::new(),
                sensitive: Vec::new(),
            };
            let (args, name) = (args.clone(), name.to_string());
            // Blocking file I/O stays off the async workers
            tokio::task::spawn_blocking(move || this.run(&name, &args, &roots))
                .await
                .map_err(|e| format!("Tool failed: {e}"))?
        })
    }

    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            for root in &self.roots {
                match tokio::fs::metadata(&root.path).await {
                    Ok(meta) if meta.is_dir() => {}
                    Ok(_) => {
                        return Err(format!(
                            "The shared folder '{}' isn't a folder anymore",
                            root.id
                        ))
                    }
                    Err(e) => {
                        return Err(format!(
                            "The shared folder '{}' is unavailable: {e}",
                            root.id
                        ))
                    }
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str, path: &Path, access: Access) -> SharedFolder {
        SharedFolder {
            id: id.into(),
            path: path.to_str().unwrap().into(),
            access,
        }
    }

    /// One read and write folder, `shared`
    fn connector() -> (tempfile::TempDir, FilesystemConnector) {
        let dir = tempfile::tempdir().unwrap();
        let fs =
            FilesystemConnector::new(&[folder("shared", dir.path(), Access::Write)], &[]).unwrap();
        (dir, fs)
    }

    fn options(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    /// A tool call from an agent given `shared`
    async fn call_shared(
        fs: &FilesystemConnector,
        name: &str,
        args: Value,
    ) -> Result<Value, String> {
        fs.call_tool_for(name, &args, Some(&options("{folders: [shared]}")))
            .await
    }

    #[tokio::test]
    async fn reads_writes_and_lists_inside_the_root() {
        let (_dir, fs) = connector();
        call_shared(
            &fs,
            "write_file",
            json!({"path": "notes/a.txt", "content": "hi"}),
        )
        .await
        .unwrap();
        let read = call_shared(&fs, "read_file", json!({"path": "notes/a.txt"})).await;
        assert_eq!(read.unwrap(), json!("hi"));
        // With the folder named too
        let read = call_shared(&fs, "read_file", json!({"path": "shared/notes/a.txt"})).await;
        assert_eq!(read.unwrap(), json!("hi"));
        let listed = call_shared(&fs, "list_dir", json!({"path": "shared"}))
            .await
            .unwrap();
        assert_eq!(listed, json!("notes/"));
        let top = call_shared(&fs, "list_dir", json!({})).await.unwrap();
        assert_eq!(top, json!("shared/ (write)"));
        fs.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn an_agent_sees_only_its_folders_at_its_access() {
        let (docs, reports, secret) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        std::fs::write(docs.path().join("a.md"), "doc").unwrap();
        std::fs::write(secret.path().join("s.md"), "no").unwrap();
        let fs = FilesystemConnector::new(
            &[
                folder("docs", docs.path(), Access::Read),
                folder("reports", reports.path(), Access::Write),
                folder("secret", secret.path(), Access::Write),
            ],
            &[],
        )
        .unwrap();
        let mine = options("{id: filesystem, folders: [docs, reports]}");
        let call = |name: &'static str, args: Value| {
            let fs = &fs;
            let mine = &mine;
            async move { fs.call_tool_for(name, &args, Some(mine)).await }
        };

        assert_eq!(
            call("list_dir", json!({})).await.unwrap(),
            json!("docs/ (read)\nreports/ (write)")
        );
        assert_eq!(
            call("read_file", json!({"path": "docs/a.md"}))
                .await
                .unwrap(),
            json!("doc")
        );
        assert!(
            call("write_file", json!({"path": "docs/b.md", "content": "x"}))
                .await
                .unwrap_err()
                .contains("read-only")
        );
        call(
            "write_file",
            json!({"path": "reports/r.md", "content": "x"}),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(reports.path().join("r.md")).unwrap(),
            "x"
        );
        // A folder it didn't name doesn't exist for it
        assert!(call("read_file", json!({"path": "secret/s.md"}))
            .await
            .is_err());
        // Two folders: the folder must be named
        assert!(call("read_file", json!({"path": "a.md"}))
            .await
            .unwrap_err()
            .contains("Start the path"));

        // Narrowed per agent
        let narrow = options("{folders: [{id: reports, access: read}]}");
        assert!(fs
            .call_tool_for(
                "write_file",
                &json!({"path": "r2.md", "content": "x"}),
                Some(&narrow)
            )
            .await
            .unwrap_err()
            .contains("read-only"));

        // Naming no folders gets nothing
        let none = fs.call_tool("list_dir", &json!({})).await;
        assert!(none.unwrap_err().contains("No folders"));
    }

    /// Site profiles go in `my-notes/private/sites/`; card numbers and
    /// passwords never go into the notes, though other folders aren't checked
    #[tokio::test]
    async fn the_notes_keep_site_profiles_but_never_cards_or_passwords() {
        let (notes, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let fs = FilesystemConnector::new(
            &[
                folder("my-notes", notes.path(), Access::Write),
                folder("other", other.path(), Access::Write),
            ],
            &[],
        )
        .unwrap();
        let mine = options("{folders: [my-notes, other]}");
        let write = |path: &'static str, content: &'static str| {
            let (fs, mine) = (&fs, &mine);
            async move {
                fs.call_tool_for(
                    "write_file",
                    &json!({"path": path, "content": content}),
                    Some(mine),
                )
                .await
            }
        };
        let profile = "# acme.com\n\n- Customer number 55-1234\n- Secret name: ACME_PASSWORD\n";
        write("my-notes/private/sites/acme.com.md", profile)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(notes.path().join("private/sites/acme.com.md")).unwrap(),
            profile
        );
        for refused in [
            "Card 4242 4242 4242 4242",
            "- password: hunter2",
            "CVV: 123",
        ] {
            let error = write("my-notes/private/sites/acme.com.md", refused)
                .await
                .unwrap_err();
            assert!(error.starts_with("Not written"), "{refused}: {error}");
        }
        assert_eq!(
            std::fs::read_to_string(notes.path().join("private/sites/acme.com.md")).unwrap(),
            profile,
            "a refused write leaves the file as it was"
        );
        write("other/data.txt", "4242424242424242").await.unwrap();
    }

    #[tokio::test]
    async fn refuses_paths_outside_the_root() {
        let (dir, fs) = connector();
        std::fs::write(dir.path().join("inside.txt"), "x").unwrap();
        for path in ["../x.txt", "shared/../x.txt", "a/../../x"] {
            let read = call_shared(&fs, "read_file", json!({"path": path})).await;
            assert!(read.is_err(), "{path} must be refused");
            let write = call_shared(&fs, "write_file", json!({"path": path, "content": "x"})).await;
            assert!(write.is_err(), "{path} must be refused");
        }
        #[cfg(unix)]
        assert!(
            call_shared(&fs, "read_file", json!({"path": "/etc/passwd"}))
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_symlinks_that_escape() {
        let (dir, fs) = connector();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        let read = call_shared(&fs, "read_file", json!({"path": "link/secret.txt"})).await;
        assert!(read.unwrap_err().contains("outside"));
        let write = call_shared(
            &fs,
            "write_file",
            json!({"path": "link/new.txt", "content": "x"}),
        )
        .await;
        assert!(write.unwrap_err().contains("outside"));
    }

    #[test]
    fn needs_an_existing_absolute_folder() {
        let relative = SharedFolder {
            id: "a".into(),
            path: "relative/path".into(),
            access: Access::Read,
        };
        assert!(FilesystemConnector::new(&[relative], &[]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let missing = folder("m", &dir.path().join("missing"), Access::Read);
        assert!(FilesystemConnector::new(std::slice::from_ref(&missing), &[]).is_err());
        assert!(FilesystemConnector::new(&[], &[]).is_err());
        // A missing folder next to a good one leaves only it out
        let fs = FilesystemConnector::new(&[missing, folder("ok", dir.path(), Access::Read)], &[])
            .unwrap();
        assert_eq!(fs.roots.len(), 1);
    }

    #[test]
    fn write_file_is_sensitive() {
        let (_dir, fs) = connector();
        let sensitive: Vec<_> = fs
            .list_tools()
            .into_iter()
            .filter(|t| t.sensitive)
            .map(|t| t.name)
            .collect();
        assert_eq!(sensitive, ["write_file"]);
    }
}
