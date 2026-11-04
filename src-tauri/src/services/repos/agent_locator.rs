//! Where an agent's folder is. Agent IDs are flat and unique across the
//! repo, but folders live in three places:
//!
//! ```text
//! .nandi/agents/<id>/                                   Local (the user's own)
//! .nandi/agents/marketplace/<publisher>/<id>/           Vendored from a publisher
//! .nandi/agents/marketplace/packs/<pack>/agents/<id>/   Vendored with a pack
//! ```
//!
//! Vendored folders are read-only to the app (only the marketplace installer
//! writes them) until the user forks the agent into `.nandi/agents/<new-id>/`.
//! A local folder wins over a vendored one with the same ID; the shadowed
//! one is reported as a duplicate.

use std::fs;
use std::path::{Path, PathBuf};

use super::fs_atomic::TEMP_MARKER;
use super::layout::{MARKETPLACE_DIR, PACKS_DIR, SKILL_FILE};
use super::RepoLayout;

/// Who owns an agent or workflow folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The user's own: fully editable
    Local,
    /// Installed individually from `publisher`'s repo
    Publisher { publisher: String },
    /// Installed as part of a pack
    Pack { pack: String },
}

impl Origin {
    pub fn is_vendored(&self) -> bool {
        !matches!(self, Self::Local)
    }
}

/// An agent folder on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPlace {
    pub id: String,
    pub dir: PathBuf,
    pub origin: Origin,
}

/// Folder names that are never agents or publishers: hidden, the
/// installer's staging folders, and editor/OS clutter.
fn visible(name: &str) -> bool {
    !name.starts_with('.') && !name.contains(TEMP_MARKER)
}

/// Sub-folders of `dir` (sorted), with their names.
fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| visible(name))
        .collect();
    dirs.sort();
    dirs
}

/// Agent folders (holding a skill.md) directly under `dir`.
fn agent_folders(dir: &Path, origin: &Origin) -> Vec<AgentPlace> {
    subdirs(dir)
        .into_iter()
        .filter(|(_, path)| path.join(SKILL_FILE).is_file())
        .map(|(id, dir)| AgentPlace {
            id,
            dir,
            origin: origin.clone(),
        })
        .collect()
}

/// Every vendored agent folder, publishers first, then packs.
pub fn vendored(layout: &RepoLayout) -> Vec<AgentPlace> {
    let mut places = Vec::new();
    let marketplace = layout.marketplace_dir();
    for (publisher, dir) in subdirs(&marketplace) {
        if publisher == PACKS_DIR {
            continue;
        }
        places.extend(agent_folders(&dir, &Origin::Publisher { publisher }));
    }
    for (pack, dir) in subdirs(&marketplace.join(PACKS_DIR)) {
        places.extend(agent_folders(&dir.join("agents"), &Origin::Pack { pack }));
    }
    places
}

/// Every agent folder: local ones (sorted by ID), then vendored ones. An ID
/// can appear more than once; the first is the one that loads.
pub fn all(layout: &RepoLayout) -> Vec<AgentPlace> {
    let mut places: Vec<AgentPlace> = agent_folders(&layout.agents_dir, &Origin::Local)
        .into_iter()
        .filter(|place| place.id != MARKETPLACE_DIR)
        .collect();
    places.extend(vendored(layout));
    places
}

/// Where agent `id` lives, if it has a folder with a skill.md.
pub fn locate(layout: &RepoLayout, id: &str) -> Option<AgentPlace> {
    if id == MARKETPLACE_DIR {
        return None;
    }
    let local = layout.agent_dir(id);
    if local.join(SKILL_FILE).is_file() {
        return Some(AgentPlace {
            id: id.to_string(),
            dir: local,
            origin: Origin::Local,
        });
    }
    vendored(layout).into_iter().find(|place| place.id == id)
}

/// The agent's folder: where it is, or where a new local one would go.
pub fn agent_dir(layout: &RepoLayout, id: &str) -> PathBuf {
    locate(layout, id).map_or_else(|| layout.agent_dir(id), |place| place.dir)
}

/// Global-scope workflow folders shipped with packs
/// (`marketplace/packs/<pack>/workflows/`), with their pack.
pub fn pack_workflow_dirs(layout: &RepoLayout) -> Vec<(PathBuf, Origin)> {
    subdirs(&layout.marketplace_dir().join(PACKS_DIR))
        .into_iter()
        .map(|(pack, dir)| (dir.join("workflows"), Origin::Pack { pack }))
        .filter(|(dir, _)| dir.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(path: PathBuf) {
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(SKILL_FILE), "---\nname: X\n---\n").unwrap();
    }

    #[test]
    fn finds_local_and_vendored_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path(), tmp.path().join("trash"));
        skill(layout.agent_dir("mine"));
        skill(layout.vendored_agent_dir("alice", "arxiv"));
        skill(layout.pack_dir("research").join("agents/summarizer"));
        skill(layout.vendored_agent_dir("bob", "mine")); // shadowed
        fs::create_dir_all(layout.marketplace_dir().join(".staging.nandi-tmp-1/x")).unwrap();
        fs::create_dir_all(layout.pack_dir("research").join("workflows")).unwrap();

        let ids: Vec<String> = all(&layout).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["mine", "arxiv", "mine", "summarizer"]);
        assert_eq!(locate(&layout, "mine").unwrap().origin, Origin::Local);
        assert_eq!(
            locate(&layout, "arxiv").unwrap().origin,
            Origin::Publisher {
                publisher: "alice".into()
            }
        );
        let summarizer = locate(&layout, "summarizer").unwrap();
        assert_eq!(
            summarizer.origin,
            Origin::Pack {
                pack: "research".into()
            }
        );
        assert!(locate(&layout, "marketplace").is_none());
        assert_eq!(agent_dir(&layout, "new"), layout.agent_dir("new"));
        assert_eq!(pack_workflow_dirs(&layout).len(), 1);
    }
}
