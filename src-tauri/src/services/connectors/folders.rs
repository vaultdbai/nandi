//! Shared folders: the folders the user lets agents use, each read-only or
//! read and write, and which of them each agent gets. Pure.
//!
//! The list is the `filesystem` connector's config in `.nandi/connectors.yaml`:
//!
//! ```yaml
//! - id: filesystem
//!   kind: filesystem
//!   config:
//!     folders:
//!       - {id: docs, path: ~/Documents/agent-share, access: read}
//!       - {id: reports, path: ~/Reports, access: write}
//! ```
//!
//! An agent names the folders it gets in its skill.md, optionally narrowed
//! to read-only, never widened:
//!
//! ```yaml
//! connectors:
//!   - id: filesystem
//!     folders: [docs, {id: reports, access: read}]
//! ```
//!
//! An agent gets only the folders it names: none without `folders`.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::ConnectorDecl;

/// Longest folder ID
const MAX_ID_CHARS: usize = 40;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    #[default]
    Read,
    /// Read and write
    Write,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// One folder on the list
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolder {
    /// What agents and tool paths call it (`docs/notes.md`)
    pub id: String,
    /// On the host; `~` is the home folder
    pub path: String,
    #[serde(default)]
    pub access: Access,
}

/// A folder an agent names, with the access it asks for
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderGrant {
    pub id: String,
    /// `None`: the folder's own access
    pub access: Option<Access>,
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
}

fn text(value: &serde_yaml::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn access(value: Option<&serde_yaml::Value>) -> Result<Option<Access>, String> {
    match value.map(|v| v.as_str().map(str::trim)) {
        None => Ok(None),
        Some(Some("read")) => Ok(Some(Access::Read)),
        Some(Some("write")) => Ok(Some(Access::Write)),
        Some(other) => Err(format!(
            "access must be read or write, not {}",
            other.unwrap_or("that")
        )),
    }
}

/// The list in a `filesystem` connector's config (`folders`). IDs are
/// unique and valid; paths aren't checked here.
pub fn shared_folders(decl: &ConnectorDecl) -> Result<Vec<SharedFolder>, String> {
    let mut folders = Vec::new();
    match decl.config.get("folders") {
        Some(serde_yaml::Value::Sequence(items)) => {
            for item in items {
                let id = item
                    .get("id")
                    .and_then(text)
                    .ok_or("Each folder needs an id")?;
                let path = item
                    .get("path")
                    .and_then(text)
                    .ok_or_else(|| format!("Folder '{id}' needs a path"))?;
                let access = access(item.get("access"))
                    .map_err(|e| format!("Folder '{id}': {e}"))?
                    .unwrap_or_default();
                folders.push(SharedFolder { id, path, access });
            }
        }
        Some(serde_yaml::Value::Null) | None => {}
        Some(_) => return Err("folders must be a list".to_string()),
    }
    let mut seen = HashSet::new();
    for folder in &folders {
        if !valid_id(&folder.id) {
            return Err(format!(
                "Folder ID '{}' must use lowercase letters, digits, '-' and '_'",
                folder.id
            ));
        }
        if !seen.insert(folder.id.as_str()) {
            return Err(format!("Two folders are called '{}'", folder.id));
        }
    }
    Ok(folders)
}

/// `path` as it's stored, so the repo works on another machine: under the
/// Nandi home, relative to it (`notes`); under the user's home, `~/…`;
/// anything else as it is, since it's only on this machine. Always `/`.
pub fn portable(path: &str, nandi: Option<&Path>, home: Option<&Path>) -> String {
    let slashes = |p: &str| p.trim().replace('\\', "/");
    let path = slashes(path);
    let under = |base: Option<&Path>| -> Option<String> {
        let base = slashes(&base?.to_string_lossy());
        let base = base.trim_end_matches('/');
        let head = path.get(..base.len())?;
        let rest = &path[base.len()..];
        (head.eq_ignore_ascii_case(base) && (rest.is_empty() || rest.starts_with('/')))
            .then(|| rest.trim_start_matches('/').to_string())
    };
    if let Some(rest) = under(nandi).filter(|rest| !rest.is_empty()) {
        return rest;
    }
    match under(home) {
        Some(rest) if rest.is_empty() => "~".to_string(),
        Some(rest) => format!("~/{rest}"),
        None => path,
    }
}

/// The config that stores `folders`, with portable paths
pub fn folders_config(
    folders: &[SharedFolder],
) -> std::collections::BTreeMap<String, serde_yaml::Value> {
    let nandi = crate::utils::nandi_config::nandi_dir().ok();
    let home = dirs::home_dir();
    let list = folders
        .iter()
        .map(|folder| {
            let mut map = serde_yaml::Mapping::new();
            map.insert("id".into(), folder.id.clone().into());
            let path = portable(&folder.path, nandi.as_deref(), home.as_deref());
            map.insert("path".into(), path.into());
            map.insert("access".into(), folder.access.as_str().into());
            serde_yaml::Value::Mapping(map)
        })
        .collect();
    std::collections::BTreeMap::from([("folders".to_string(), serde_yaml::Value::Sequence(list))])
}

/// The folders an agent's declaration names (`options` is its
/// `connectors:` item); empty when it names none
pub fn grants(options: Option<&serde_yaml::Value>) -> Result<Vec<FolderGrant>, String> {
    let Some(list) = options.and_then(|o| o.get("folders")) else {
        return Ok(Vec::new());
    };
    let serde_yaml::Value::Sequence(items) = list else {
        return Err("folders must be a list of folder IDs".to_string());
    };
    items
        .iter()
        .map(|item| {
            // `docs` or `{id: docs, access: read}`
            let id = text(item)
                .or_else(|| item.get("id").and_then(text))
                .ok_or("Each folder is an ID, or {id, access}")?;
            let access = access(item.get("access")).map_err(|e| format!("Folder '{id}': {e}"))?;
            Ok(FolderGrant { id, access })
        })
        .collect()
}

/// What an agent may use: the folders it names that are on the list, at
/// the lower of the two accesses
pub fn granted(folders: &[SharedFolder], grants: &[FolderGrant]) -> Vec<SharedFolder> {
    grants
        .iter()
        .filter_map(|grant| {
            let folder = folders.iter().find(|f| f.id == grant.id)?;
            Some(SharedFolder {
                access: grant.access.map_or(folder.access, |a| a.min(folder.access)),
                ..folder.clone()
            })
        })
        .collect()
}

/// What an agent's prompt says about its folders: where they are in its
/// container (`mount`, e.g. `/agent/folders`) and which it may write.
/// Empty without folders.
pub fn prompt(folders: &[SharedFolder], mount: &str) -> String {
    if folders.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = folders
        .iter()
        .map(|f| match f.access {
            Access::Write => format!("- `{mount}/{}` (read and write)", f.id),
            Access::Read => format!("- `{mount}/{}` (read only)", f.id),
        })
        .collect();
    format!(
        "\n\nThe user shared these folders with you:\n{}\nUse only these outside your work folder, and never write to a read-only one.",
        lines.join("\n")
    )
}

/// Folder IDs an agent names that aren't on the list
pub fn unknown(folders: &[SharedFolder], grants: &[FolderGrant]) -> Vec<String> {
    grants
        .iter()
        .filter(|g| !folders.iter().any(|f| f.id == g.id))
        .map(|g| g.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::ConnectorId;

    fn decl(config: &str) -> ConnectorDecl {
        ConnectorDecl {
            id: ConnectorId::parse("filesystem").unwrap(),
            kind: "filesystem".into(),
            config: serde_yaml::from_str(config).unwrap(),
            secret_refs: Default::default(),
        }
    }

    fn yaml(text: &str) -> serde_yaml::Value {
        serde_yaml::from_str(text).unwrap()
    }

    const LIST: &str = "folders:\n  - {id: docs, path: ~/Docs, access: read}\n  - {id: reports, path: /r, access: write}\n";

    #[test]
    fn reads_the_list() {
        let folders = shared_folders(&decl(LIST)).unwrap();
        assert_eq!(folders.len(), 2);
        assert_eq!(
            (folders[0].id.as_str(), folders[0].access),
            ("docs", Access::Read)
        );
        assert_eq!(folders[1].access, Access::Write);

        assert!(shared_folders(&decl("{}")).unwrap().is_empty());
        // Missing access is read: the safer one
        let plain = shared_folders(&decl("folders: [{id: a, path: /a}]")).unwrap();
        assert_eq!(plain[0].access, Access::Read);
    }

    #[test]
    fn the_prompt_lists_each_folder_and_its_access() {
        assert_eq!(prompt(&[], "/m"), "");
        let folders = shared_folders(&decl(LIST)).unwrap();
        let text = prompt(&folders, "/agent/folders");
        assert!(text.contains("`/agent/folders/docs` (read only)"));
        assert!(text.contains("`/agent/folders/reports` (read and write)"));
    }

    #[test]
    fn refuses_bad_lists() {
        for bad in [
            "folders: [{id: a, path: /a}, {id: a, path: /b}]",
            "folders: [{id: 'A B', path: /a}]",
            "folders: [{id: a}]",
            "folders: [{path: /a}]",
            "folders: [{id: a, path: /a, access: admin}]",
            "folders: nope",
        ] {
            assert!(shared_folders(&decl(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_are_stored_portably() {
        let nandi = Some(Path::new("C:\\Users\\ada\\.nandi"));
        let home = Some(Path::new("C:\\Users\\ada"));
        let stored = |p: &str| portable(p, nandi, home);
        assert_eq!(stored("C:\\Users\\ada\\.nandi\\notes"), "notes");
        assert_eq!(stored("C:/Users/ada/.nandi/notes/work"), "notes/work");
        assert_eq!(
            stored("c:\\users\\ADA\\Documents\\share"),
            "~/Documents/share"
        );
        assert_eq!(stored("C:\\Users\\ada"), "~");
        assert_eq!(
            stored("D:\\work\\reports"),
            "D:/work/reports",
            "only on this machine"
        );
        assert_eq!(
            stored("C:\\Users\\adam\\x"),
            "C:/Users/adam/x",
            "another user's folder"
        );
        assert_eq!(stored("~/Docs"), "~/Docs");
        assert_eq!(stored("notes"), "notes");
    }

    #[test]
    fn round_trips_through_config() {
        let folders = shared_folders(&decl(LIST)).unwrap();
        let config = folders_config(&folders);
        let again = shared_folders(&ConnectorDecl {
            config,
            ..decl("{}")
        })
        .unwrap();
        assert_eq!(again, folders);
    }

    #[test]
    fn an_agent_gets_only_what_it_names_and_never_more_access() {
        let folders = shared_folders(&decl(LIST)).unwrap();
        let named = grants(Some(&yaml(
            "{id: filesystem, folders: [docs, {id: reports, access: read}, gone]}",
        )))
        .unwrap();
        let got = granted(&folders, &named);
        let got: Vec<(&str, Access)> = got.iter().map(|f| (f.id.as_str(), f.access)).collect();
        assert_eq!(got, [("docs", Access::Read), ("reports", Access::Read)]);
        assert_eq!(unknown(&folders, &named), ["gone"]);

        // Asking for write on a read folder stays read
        let widen = grants(Some(&yaml("{folders: [{id: docs, access: write}]}"))).unwrap();
        assert_eq!(granted(&folders, &widen)[0].access, Access::Read);

        // Naming none gets none
        assert!(granted(&folders, &[]).is_empty());
        assert!(grants(None).unwrap().is_empty());
        assert!(grants(Some(&yaml("filesystem"))).unwrap().is_empty());
        assert!(grants(Some(&yaml("{folders: docs}"))).is_err());
    }
}
