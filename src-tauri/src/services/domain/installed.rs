//! `.nandi/installed.yaml`: every marketplace source vendored into the repo.
//!
//! ```yaml
//! packs:
//!   - id: research-pack
//!     source: github.com/vaultdbai/nandi-research-pack
//!     ref: v1.0.0
//!     commit: a3f2c1b9…
//!     version: 1.0.0
//!     installed_at: 2026-10-06T14:32:00Z
//!     signature_verified: false
//!     tree_hash: 9f86d0…           # of the files as installed
//!     agents: [arxiv-researcher, paper-summarizer]
//!     workflows: [literature-review]
//!     forked_out: [citation-formatter]   # forked into the user's own agents
//! agents:
//!   - id: arxiv-researcher
//!     publisher: alice
//!     source: github.com/alice/nandi-agents
//!     ref: v1.2.0
//!     commit: e5d4c3b2…
//!     path: agents/arxiv-researcher
//!     version: 1.2.0
//!     installed_at: 2026-10-06T14:33:00Z
//!     signature_verified: false
//!     tree_hash: 2c26b4…
//! ```
//!
//! Safe to commit: sources are plain repo locations. A source carrying
//! credentials (`https://user:token@…`) is rejected.
//!
//! A pack installed from a folder or `.zip` on this computer has the source
//! `local:<absolute path>`, `ref: local`, the tree hash as its commit, no
//! registry and `signature_verified: false`. It has no source to check
//! for updates.

use serde::{Deserialize, Serialize};

use super::{AgentId, DomainError, DomainResult, PackId, PublisherHandle, WorkflowId};

const FILE: &str = "installed.yaml";

/// The scheme of a source on this computer (a folder or a `.zip`)
pub const LOCAL_SOURCE_PREFIX: &str = "local:";

/// The `ref` of a local install
pub const LOCAL_REF: &str = "local";

/// The source of a pack installed from `path` on this computer
pub fn local_source(path: &std::path::Path) -> String {
    format!("{LOCAL_SOURCE_PREFIX}{}", path.display())
}

/// The path of a local source (`local:<path>`), `None` for a repo location
pub fn local_path(source: &str) -> Option<&str> {
    source.strip_prefix(LOCAL_SOURCE_PREFIX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPack {
    pub id: PackId,
    pub source: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub commit: String,
    pub version: String,
    pub installed_at: String,
    #[serde(default)]
    pub signature_verified: bool,
    /// The registry it was installed from, if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    pub tree_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<WorkflowId>,
    /// Agents of the pack the user forked (their folders left the pack)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forked_out: Vec<AgentId>,
}

impl InstalledPack {
    /// Installed from a folder or `.zip` on this computer
    pub fn is_local(&self) -> bool {
        local_path(&self.source).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledAgent {
    pub id: AgentId,
    pub publisher: PublisherHandle,
    pub source: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub commit: String,
    /// The agent's folder within the source repo
    pub path: String,
    pub version: String,
    pub installed_at: String,
    #[serde(default)]
    pub signature_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    pub tree_hash: String,
}

/// What a vendored folder was installed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Pack,
    Agent,
}

impl InstallKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pack => "pack",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "pack" => Ok(Self::Pack),
            "agent" => Ok(Self::Agent),
            other => Err(DomainError::invalid(
                FILE,
                "kind",
                format!("'{other}' is not 'pack' or 'agent'"),
            )),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packs: Vec<InstalledPack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<InstalledAgent>,
}

impl InstalledFile {
    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        let file: Self = serde_yaml::from_str(content).map_err(|e| DomainError::parse(FILE, e))?;
        file.check()?;
        Ok(file)
    }

    pub fn to_yaml(&self) -> String {
        if self.packs.is_empty() && self.agents.is_empty() {
            return String::new();
        }
        serde_yaml::to_string(self).unwrap_or_default()
    }

    fn check(&self) -> DomainResult<()> {
        let sources = self
            .packs
            .iter()
            .map(|p| (p.id.as_str(), p.source.as_str(), None))
            .chain(
                self.agents
                    .iter()
                    .map(|a| (a.id.as_str(), a.source.as_str(), Some(a.path.as_str()))),
            );
        for (id, source, path) in sources {
            check_source(id, source)?;
            if let Some(path) = path {
                check_repo_path(id, path)?;
            }
        }
        let mut ids: Vec<&str> = self.agents.iter().map(|a| a.id.as_str()).collect();
        ids.sort_unstable();
        if let Some(pair) = ids.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(DomainError::Rule(format!(
                "{FILE} lists agent '{}' twice",
                pair[0]
            )));
        }
        let mut packs: Vec<&str> = self.packs.iter().map(|p| p.id.as_str()).collect();
        packs.sort_unstable();
        if let Some(pair) = packs.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(DomainError::Rule(format!(
                "{FILE} lists pack '{}' twice",
                pair[0]
            )));
        }
        Ok(())
    }

    pub fn pack(&self, id: &str) -> Option<&InstalledPack> {
        self.packs.iter().find(|p| p.id.as_str() == id)
    }

    pub fn agent(&self, id: &str) -> Option<&InstalledAgent> {
        self.agents.iter().find(|a| a.id.as_str() == id)
    }

    /// The installed pack that ships agent `id`, if any.
    pub fn pack_of(&self, agent: &str) -> Option<&InstalledPack> {
        self.packs
            .iter()
            .find(|p| p.agents.iter().any(|a| a.as_str() == agent))
    }

    pub fn upsert_pack(&mut self, pack: InstalledPack) -> DomainResult<()> {
        check_source(pack.id.as_str(), &pack.source)?;
        match self.packs.iter_mut().find(|p| p.id == pack.id) {
            Some(existing) => *existing = pack,
            None => self.packs.push(pack),
        }
        Ok(())
    }

    pub fn upsert_agent(&mut self, agent: InstalledAgent) -> DomainResult<()> {
        check_source(agent.id.as_str(), &agent.source)?;
        check_repo_path(agent.id.as_str(), &agent.path)?;
        match self.agents.iter_mut().find(|a| a.id == agent.id) {
            Some(existing) => *existing = agent,
            None => self.agents.push(agent),
        }
        Ok(())
    }

    pub fn remove_pack(&mut self, id: &str) -> Option<InstalledPack> {
        let index = self.packs.iter().position(|p| p.id.as_str() == id)?;
        Some(self.packs.remove(index))
    }

    pub fn remove_agent(&mut self, id: &str) -> Option<InstalledAgent> {
        let index = self.agents.iter().position(|a| a.id.as_str() == id)?;
        Some(self.agents.remove(index))
    }

    /// Records that a pack's agent was forked out of it.
    pub fn mark_forked_out(&mut self, agent: &AgentId) {
        if let Some(pack) = self
            .packs
            .iter_mut()
            .find(|p| p.agents.iter().any(|a| a == agent))
        {
            pack.agents.retain(|a| a != agent);
            if !pack.forked_out.contains(agent) {
                pack.forked_out.push(agent.clone());
            }
        }
    }
}

/// A repo location: `github.com/owner/repo`, an `https://` URL, or a local
/// `file://` path. Never credentials.
pub fn check_source(id: &str, source: &str) -> DomainResult<()> {
    let invalid = |reason: &str| {
        Err(DomainError::invalid(
            format!("{FILE} entry '{id}'"),
            "source",
            reason.to_string(),
        ))
    };
    // A path on this computer: any name (spaces included), no credentials
    if let Some(path) = local_path(source) {
        return if path.trim().is_empty() {
            invalid("must name the folder or .zip it came from")
        } else {
            Ok(())
        };
    }
    if source.trim().is_empty() || source.chars().any(char::is_whitespace) {
        return invalid("must be a repo location without spaces");
    }
    let without_scheme = source.split_once("://").map_or(source, |(_, rest)| rest);
    let authority = without_scheme.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return invalid("must not contain credentials");
    }
    Ok(())
}

/// A relative path inside a source repo: `/`-separated plain names, no
/// `..`, no leading `/`, and no hidden folders except `.nandi` (where an
/// agents repo keeps its agents).
pub fn check_repo_path(id: &str, path: &str) -> DomainResult<()> {
    let valid = !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && path.split('/').all(|part| {
            !part.is_empty()
                && (!part.starts_with('.') || part == ".nandi")
                && !part.contains(['\\', ':'])
                && part.chars().all(|c| !c.is_control())
        });
    if !valid {
        return Err(DomainError::invalid(
            format!("{FILE} entry '{id}'"),
            "path",
            format!("'{path}' must be a relative folder inside the repo"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
packs:
  - id: research-pack
    source: github.com/vaultdbai/nandi-research-pack
    ref: v1.0.0
    commit: a3f2c1b9
    version: 1.0.0
    installed_at: 2026-10-06T14:32:00Z
    signature_verified: true
    tree_hash: abc
    agents: [arxiv-researcher, paper-summarizer]
    workflows: [literature-review]
agents:
  - id: code-reviewer
    publisher: alice
    source: github.com/alice/nandi-agents
    ref: v1.2.0
    commit: e5d4c3b2
    path: agents/code-reviewer
    version: 1.2.0
    installed_at: 2026-10-06T14:33:00Z
    tree_hash: def
"#;

    #[test]
    fn round_trips_and_tracks_forks() {
        let mut file = InstalledFile::from_yaml(SAMPLE).unwrap();
        assert!(file.pack("research-pack").unwrap().signature_verified);
        assert!(!file.agent("code-reviewer").unwrap().signature_verified);
        assert_eq!(
            file.pack_of("paper-summarizer").unwrap().id.as_str(),
            "research-pack"
        );
        assert_eq!(InstalledFile::from_yaml(&file.to_yaml()).unwrap(), file);

        file.mark_forked_out(&AgentId::parse("paper-summarizer").unwrap());
        let pack = file.pack("research-pack").unwrap();
        assert_eq!(pack.agents.len(), 1);
        assert_eq!(pack.forked_out[0].as_str(), "paper-summarizer");
        assert!(file.pack_of("paper-summarizer").is_none());

        assert!(file.remove_agent("code-reviewer").is_some());
        assert!(file.remove_pack("research-pack").is_some());
        assert_eq!(file.to_yaml(), "");
    }

    #[test]
    fn rejects_credentials_traversal_and_duplicates() {
        let bad_source = SAMPLE.replace(
            "github.com/alice/nandi-agents",
            "https://alice:ghp_secret@github.com/alice/nandi-agents",
        );
        assert!(InstalledFile::from_yaml(&bad_source).is_err());
        let bad_path = SAMPLE.replace("agents/code-reviewer", "../../etc");
        assert!(InstalledFile::from_yaml(&bad_path).is_err());
        let bad_publisher = SAMPLE.replace("publisher: alice", "publisher: packs");
        assert!(InstalledFile::from_yaml(&bad_publisher).is_err());
        let twice = format!(
            "{SAMPLE}{}",
            &SAMPLE[SAMPLE.find("  - id: code-reviewer").unwrap()..]
        );
        assert!(InstalledFile::from_yaml(&twice).is_err());
        assert!(check_source("x", "file:///tmp/fixture").is_ok());
        assert!(check_repo_path("x", ".git/config").is_err());
        assert!(check_repo_path("x", ".nandi/agents/writer").is_ok());
    }
}
