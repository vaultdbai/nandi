//! Registry entries and pack manifests: parsing and validation (no I/O).
//!
//! A registry repo holds `index.json` (every entry), plus one file per
//! entry under `agents/`, `packs/`, and `publishers/`. Entries use
//! `snake_case` on the wire (`latest_version`); the types sent to the UI
//! are `camelCase`. Every entry is validated on the way in: IDs become
//! folder names and `path` is joined onto a checkout, so a bad entry is
//! skipped (with a warning), never trusted.
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "agents": [{ "id": "arxiv-researcher", "name": "ArXiv Researcher",
//!                "publisher": "alice", "repo": "github.com/alice/nandi-agents",
//!                "path": "agents/arxiv-researcher", "latest_version": "1.2.0",
//!                "min_nandi_version": "0.1.0", "tags": ["research"],
//!                "capabilities": { "tools": ["shell"], "network": ["arxiv.org"], "pilots": [] } }],
//!   "packs": [{ "id": "research-pack", "name": "Research Pack", "publisher": "vaultdbai",
//!               "repo": "github.com/vaultdbai/nandi-research-pack", "latest_version": "1.0.0",
//!               "agents": [{ "id": "arxiv-researcher", "name": "ArXiv Researcher" }],
//!               "workflows": ["literature-review"] }]
//! }
//! ```
//!
//! A bare JSON array is read as a list of agent entries.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::domain::installed::{check_repo_path, check_source};
use crate::services::domain::settings::AgentConsent;
use crate::services::domain::{AgentId, PackId, PublisherHandle, WorkflowId};

/// This app's version, for `min_nandi_version`.
pub const NANDI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The newest index format this version reads.
pub const INDEX_SCHEMA_VERSION: u32 = 1;

/// Parses `1.2.0` or `v1.2.0`.
pub fn parse_version(value: &str) -> Result<semver::Version, String> {
    let trimmed = value.trim();
    semver::Version::parse(trimmed.strip_prefix('v').unwrap_or(trimmed))
        .map_err(|e| format!("'{value}' is not a version (x.y.z): {e}"))
}

/// Whether this app is at least `min` (an unparseable `min` is not).
pub fn is_compatible(min: &str) -> bool {
    match (parse_version(min), parse_version(NANDI_VERSION)) {
        (Ok(min), Ok(current)) => current >= min,
        _ => false,
    }
}

/// A release tag split into its prefix and version: `v1.2.0` → `("", 1.2.0)`,
/// `arxiv-researcher-v1.2.0` → `("arxiv-researcher-", 1.2.0)`. A repo
/// holding several agents (like a user's agents repo) tags each agent's
/// releases with its own prefix.
pub fn split_tag(tag: &str) -> Option<(String, semver::Version)> {
    let name = tag.strip_prefix("refs/tags/").unwrap_or(tag);
    let name = name.strip_suffix("^{}").unwrap_or(name);
    if let Ok(version) = parse_version(name) {
        return Some((String::new(), version));
    }
    let start = name.rfind("-v")? + 1;
    let version = parse_version(&name[start..]).ok()?;
    Some((name[..start].to_string(), version))
}

/// What an agent (or a pack's agents together) will be able to use.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ManifestCapabilities {
    pub tools: Vec<String>,
    /// Hosts it may reach through the egress proxy
    pub network: Vec<String>,
    pub pilots: Vec<String>,
    /// Tools that pause for the user's approval
    #[serde(alias = "requires_approval")]
    pub requires_approval: Vec<String>,
    /// AWS services
    pub aws: Vec<String>,
    /// Connectors from Library › Tools it uses, by ID (`s3_backup`)
    pub connectors: Vec<String>,
}

impl ManifestCapabilities {
    /// Adds what one agent requests (its first-use consent items) to the
    /// list, each item once.
    pub fn include(&mut self, requested: AgentConsent) {
        for (into, items) in [
            (&mut self.tools, requested.tools),
            (&mut self.network, requested.network),
            (&mut self.aws, requested.aws),
            (&mut self.pilots, requested.pilots),
            (&mut self.connectors, requested.connectors),
        ] {
            for item in items {
                if !into.contains(&item) {
                    into.push(item);
                }
            }
        }
    }

    fn check(&self, what: &str) -> Result<(), String> {
        let lists = [
            &self.tools,
            &self.network,
            &self.pilots,
            &self.requires_approval,
            &self.aws,
            &self.connectors,
        ];
        let bad = lists.iter().flat_map(|l| l.iter()).find(|item| {
            item.is_empty()
                || item.len() > 253
                || item.chars().any(|c| c.is_whitespace() || c.is_control())
        });
        match bad {
            Some(item) => Err(format!("{what}: capability '{item}' is not a plain name")),
            None => Ok(()),
        }
    }
}

/// `publishers/<handle>.json` in a registry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PublisherFile {
    pub github_handle: String,
    /// `ed25519:<base64>`; `None`: the publisher doesn't sign
    #[serde(default)]
    pub public_key: Option<String>,
}

/// A publisher-declared estimate of what one run costs on paid models.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CostRange {
    #[serde(alias = "min")]
    pub min_usd: f64,
    #[serde(alias = "max")]
    pub max_usd: f64,
}

/// One agent of a pack, as listed in its registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackAgentSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// An installable agent, from a registry.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    /// GitHub handle (lowercase)
    pub publisher: String,
    /// `github.com/<owner>/<repo>` (or a URL / `file://` path)
    pub repo: String,
    /// The agent's folder inside `repo`
    pub path: String,
    pub latest_version: String,
    /// The git ref to install (`v<latest_version>` unless the entry says)
    pub git_ref: String,
    pub tags: Vec<String>,
    pub min_nandi_version: String,
    pub capabilities: ManifestCapabilities,
    pub signature: Option<String>,
    /// The release commit the signature names (pinned at install)
    pub commit: Option<String>,
    pub icon_url: Option<String>,
    pub readme_url: Option<String>,
    pub estimated_cost_usd: Option<CostRange>,
    /// The registry this entry came from
    pub registry: String,
    /// This app is at least `min_nandi_version`
    pub compatible: bool,
    /// Set when listing: the installed version, if installed
    pub installed_version: Option<String>,
    /// The publisher's signature was verified (Phase 6; always false now)
    pub verified: bool,
}

/// An installable pack (several agents and workflows in one repo).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub repo: String,
    pub latest_version: String,
    pub git_ref: String,
    pub tags: Vec<String>,
    pub min_nandi_version: String,
    pub agents: Vec<PackAgentSummary>,
    pub workflows: Vec<String>,
    /// Things to try first once the pack is installed ("Summarize my
    /// unread email"), shown on Home after onboarding
    pub suggested_asks: Vec<String>,
    /// Everything the pack's agents together will request
    pub capabilities: ManifestCapabilities,
    /// Connectors (registry IDs) the pack needs; installing the pack offers
    /// to install and sign in to the ones that aren't here
    pub requires_connectors: Vec<String>,
    pub install_size_kb: Option<u32>,
    pub estimated_cost_usd: Option<CostRange>,
    pub signature: Option<String>,
    /// The release commit the signature names (pinned at install)
    pub commit: Option<String>,
    pub icon_url: Option<String>,
    pub readme_url: Option<String>,
    pub registry: String,
    pub compatible: bool,
    pub installed_version: Option<String>,
    pub verified: bool,
}

// -- Wire format --------------------------------------------------------------

#[derive(Deserialize)]
struct AgentEntry {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    publisher: String,
    repo: String,
    path: String,
    latest_version: String,
    #[serde(default, rename = "ref")]
    git_ref: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "zero_version")]
    min_nandi_version: String,
    #[serde(default)]
    capabilities: ManifestCapabilities,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    readme_url: Option<String>,
    #[serde(default)]
    estimated_cost_usd: Option<CostRange>,
}

#[derive(Deserialize)]
struct PackEntry {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    publisher: String,
    repo: String,
    latest_version: String,
    #[serde(default, rename = "ref")]
    git_ref: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "zero_version")]
    min_nandi_version: String,
    #[serde(default, deserialize_with = "pack_agents")]
    agents: Vec<PackAgentSummary>,
    #[serde(default)]
    workflows: Vec<String>,
    #[serde(default)]
    suggested_asks: Vec<String>,
    #[serde(default)]
    capabilities: ManifestCapabilities,
    #[serde(default)]
    requires_connectors: Vec<String>,
    #[serde(default)]
    install_size_kb: Option<u32>,
    #[serde(default)]
    estimated_cost_usd: Option<CostRange>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    readme_url: Option<String>,
}

/// A pack entry's agents: `{id, name, description}` objects (the registry
/// schema's form) or bare IDs, which are named after their ID.
fn pack_agents<'de, D>(deserializer: D) -> Result<Vec<PackAgentSummary>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PackAgentRef {
        Id(String),
        Summary(PackAgentSummary),
    }

    let refs = Vec::<PackAgentRef>::deserialize(deserializer)?;
    Ok(refs
        .into_iter()
        .map(|agent| match agent {
            PackAgentRef::Id(id) => PackAgentSummary {
                name: id.clone(),
                id,
                description: String::new(),
            },
            PackAgentRef::Summary(summary) => summary,
        })
        .collect())
}

fn zero_version() -> String {
    "0.0.0".to_string()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawIndex {
    Catalog {
        #[serde(default = "one")]
        schema_version: u32,
        #[serde(default)]
        agents: Vec<serde_json::Value>,
        #[serde(default)]
        packs: Vec<serde_json::Value>,
        #[serde(default)]
        connectors: Vec<serde_json::Value>,
    },
    Agents(Vec<serde_json::Value>),
}

fn one() -> u32 {
    1
}

/// Every valid entry in an `index.json`, and why the others were skipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedIndex {
    pub agents: Vec<AgentManifest>,
    pub packs: Vec<PackManifest>,
    pub connectors: Vec<super::connectors::ConnectorEntry>,
    pub skipped: Vec<String>,
}

/// Parses an `index.json` from `registry`. Fails only if the file as a
/// whole isn't an index; bad entries are skipped.
pub fn parse_index(json: &str, registry: &str) -> Result<ParsedIndex, String> {
    let raw: RawIndex =
        serde_json::from_str(json).map_err(|e| format!("The registry index isn't valid: {e}"))?;
    let (agents, packs, connectors) = match raw {
        RawIndex::Catalog {
            schema_version,
            agents,
            packs,
            connectors,
        } => {
            if schema_version > INDEX_SCHEMA_VERSION {
                return Err(format!(
                    "The registry uses index format {schema_version}; this version of Nandi reads up to {INDEX_SCHEMA_VERSION}. Update Nandi."
                ));
            }
            (agents, packs, connectors)
        }
        RawIndex::Agents(agents) => (agents, Vec::new(), Vec::new()),
    };
    let mut parsed = ParsedIndex::default();
    for value in agents {
        match parse_agent_value(value, registry) {
            Ok(agent) => parsed.agents.push(agent),
            Err(e) => parsed.skipped.push(e),
        }
    }
    for value in packs {
        match parse_pack_value(value, registry) {
            Ok(pack) => parsed.packs.push(pack),
            Err(e) => parsed.skipped.push(e),
        }
    }
    for value in connectors {
        match super::connectors::ConnectorEntry::parse(value, registry) {
            Ok(entry) => parsed.connectors.push(entry),
            Err(e) => parsed.skipped.push(e),
        }
    }
    for skipped in &parsed.skipped {
        log::warn!("Registry {registry}: skipped an entry: {skipped}");
    }
    Ok(parsed)
}

fn entry_id(value: &serde_json::Value) -> String {
    value
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("(no id)")
        .to_string()
}

fn parse_agent_value(value: serde_json::Value, registry: &str) -> Result<AgentManifest, String> {
    let id = entry_id(&value);
    let entry: AgentEntry =
        serde_json::from_value(value).map_err(|e| format!("agent '{id}': {e}"))?;
    agent_from_entry(entry, registry)
}

fn parse_pack_value(value: serde_json::Value, registry: &str) -> Result<PackManifest, String> {
    let id = entry_id(&value);
    let entry: PackEntry =
        serde_json::from_value(value).map_err(|e| format!("pack '{id}': {e}"))?;
    pack_from_entry(entry, registry)
}

/// Parses one `agents/<id>.json` registry file.
#[cfg(test)]
pub fn parse_agent_entry(json: &str, registry: &str) -> Result<AgentManifest, String> {
    let value = serde_json::from_str(json).map_err(|e| format!("Invalid agent entry: {e}"))?;
    parse_agent_value(value, registry)
}

/// Parses one `packs/<id>.json` registry file.
#[cfg(test)]
pub fn parse_pack_entry(json: &str, registry: &str) -> Result<PackManifest, String> {
    let value = serde_json::from_str(json).map_err(|e| format!("Invalid pack entry: {e}"))?;
    parse_pack_value(value, registry)
}

/// An icon or README location: `https://…`, or a path relative to the
/// registry (no `..`).
fn check_asset_url(what: &str, url: &Option<String>) -> Result<(), String> {
    let Some(url) = url else { return Ok(()) };
    let ok = url.starts_with("https://") || check_repo_path(what, url).is_ok();
    if ok && !url.chars().any(char::is_whitespace) {
        Ok(())
    } else {
        Err(format!(
            "{what}: '{url}' must be an https:// URL or a relative path"
        ))
    }
}

fn check_common(
    what: &str,
    name: &str,
    publisher: &str,
    repo: &str,
    latest_version: &str,
    min_nandi_version: &str,
) -> Result<String, String> {
    if name.trim().is_empty() || name.len() > 200 {
        return Err(format!("{what}: a name of 1-200 characters is required"));
    }
    let publisher = PublisherHandle::parse(publisher.to_ascii_lowercase())
        .map_err(|e| format!("{what}: {e}"))?;
    check_source(what, repo).map_err(|e| e.to_string())?;
    parse_version(latest_version).map_err(|e| format!("{what}: latest_version {e}"))?;
    parse_version(min_nandi_version).map_err(|e| format!("{what}: min_nandi_version {e}"))?;
    Ok(publisher.to_string())
}

/// A full commit SHA (40 hex characters), lowercased.
fn check_commit(what: &str, commit: Option<String>) -> Result<Option<String>, String> {
    match commit {
        None => Ok(None),
        Some(commit) if commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()) => {
            Ok(Some(commit.to_ascii_lowercase()))
        }
        Some(commit) => Err(format!(
            "{what}: commit '{commit}' must be a full 40-character SHA"
        )),
    }
}

fn check_ref(what: &str, git_ref: &str) -> Result<(), String> {
    let ok = !git_ref.is_empty()
        && git_ref.len() <= 200
        && !git_ref.starts_with('-')
        && !git_ref.contains("..")
        && git_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'));
    if ok {
        Ok(())
    } else {
        Err(format!("{what}: '{git_ref}' is not a branch or tag name"))
    }
}

fn agent_from_entry(entry: AgentEntry, registry: &str) -> Result<AgentManifest, String> {
    let what = format!("agent '{}'", entry.id);
    AgentId::parse(&entry.id).map_err(|e| format!("{what}: {e}"))?;
    let publisher = check_common(
        &what,
        &entry.name,
        &entry.publisher,
        &entry.repo,
        &entry.latest_version,
        &entry.min_nandi_version,
    )?;
    check_repo_path(&entry.id, &entry.path).map_err(|e| e.to_string())?;
    entry.capabilities.check(&what)?;
    check_asset_url(&what, &entry.icon_url)?;
    check_asset_url(&what, &entry.readme_url)?;
    let git_ref = entry
        .git_ref
        .unwrap_or_else(|| format!("v{}", entry.latest_version.trim_start_matches('v')));
    check_ref(&what, &git_ref)?;
    let commit = check_commit(&what, entry.commit)?;
    Ok(AgentManifest {
        commit,
        compatible: is_compatible(&entry.min_nandi_version),
        id: entry.id,
        name: entry.name.trim().to_string(),
        description: entry.description.trim().to_string(),
        publisher,
        repo: entry.repo,
        path: entry.path,
        latest_version: entry.latest_version.trim_start_matches('v').to_string(),
        git_ref,
        tags: entry.tags,
        min_nandi_version: entry.min_nandi_version,
        capabilities: entry.capabilities,
        signature: entry.signature,
        icon_url: entry.icon_url,
        readme_url: entry.readme_url,
        estimated_cost_usd: entry.estimated_cost_usd,
        registry: registry.to_string(),
        installed_version: None,
        verified: false,
    })
}

/// At most this many suggestions per pack are shown
const MAX_SUGGESTED_ASKS: usize = 3;
const MAX_SUGGESTED_ASK_CHARS: usize = 120;

/// A registry's suggestions, trimmed: blank ones dropped, long ones cut,
/// at most [`MAX_SUGGESTED_ASKS`]. Never an error: they're only hints.
fn suggested_asks(asks: Vec<String>) -> Vec<String> {
    asks.into_iter()
        .map(|ask| {
            ask.trim()
                .chars()
                .take(MAX_SUGGESTED_ASK_CHARS)
                .collect::<String>()
        })
        .filter(|ask| !ask.is_empty())
        .take(MAX_SUGGESTED_ASKS)
        .collect()
}

fn pack_from_entry(entry: PackEntry, registry: &str) -> Result<PackManifest, String> {
    let what = format!("pack '{}'", entry.id);
    PackId::parse(&entry.id).map_err(|e| format!("{what}: {e}"))?;
    let publisher = check_common(
        &what,
        &entry.name,
        &entry.publisher,
        &entry.repo,
        &entry.latest_version,
        &entry.min_nandi_version,
    )?;
    for agent in &entry.agents {
        AgentId::parse(&agent.id).map_err(|e| format!("{what}: {e}"))?;
    }
    for workflow in &entry.workflows {
        WorkflowId::parse(workflow).map_err(|e| format!("{what}: {e}"))?;
    }
    entry.capabilities.check(&what)?;
    check_asset_url(&what, &entry.icon_url)?;
    check_asset_url(&what, &entry.readme_url)?;
    let git_ref = entry
        .git_ref
        .unwrap_or_else(|| format!("v{}", entry.latest_version.trim_start_matches('v')));
    check_ref(&what, &git_ref)?;
    let commit = check_commit(&what, entry.commit)?;
    let requires_connectors = connector_ids(entry.requires_connectors, &what)?;
    Ok(PackManifest {
        commit,
        compatible: is_compatible(&entry.min_nandi_version),
        id: entry.id,
        name: entry.name.trim().to_string(),
        description: entry.description.trim().to_string(),
        publisher,
        repo: entry.repo,
        latest_version: entry.latest_version.trim_start_matches('v').to_string(),
        git_ref,
        tags: entry.tags,
        min_nandi_version: entry.min_nandi_version,
        agents: entry.agents,
        workflows: entry.workflows,
        suggested_asks: suggested_asks(entry.suggested_asks),
        capabilities: entry.capabilities,
        requires_connectors,
        install_size_kb: entry.install_size_kb,
        estimated_cost_usd: entry.estimated_cost_usd,
        signature: entry.signature,
        icon_url: entry.icon_url,
        readme_url: entry.readme_url,
        registry: registry.to_string(),
        installed_version: None,
        verified: false,
    })
}

// -- Pack repos ---------------------------------------------------------------

/// A pack repo's `.nandi/manifest.yaml`.
///
/// ```yaml
/// id: research-pack
/// name: Research Pack
/// version: 1.0.0
/// min_nandi_version: 0.1.0
/// agents: [arxiv-researcher, paper-summarizer, citation-formatter]
/// workflows: [literature-review]
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PackFile {
    pub id: PackId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    #[serde(default = "zero_version")]
    pub min_nandi_version: String,
    /// Empty: every folder under `agents/`
    #[serde(default)]
    pub agents: Vec<AgentId>,
    /// Empty: every file under `workflows/`
    #[serde(default)]
    pub workflows: Vec<WorkflowId>,
    /// Connectors the pack's agents need, by registry ID
    #[serde(default)]
    pub requires_connectors: Vec<String>,
}

/// Valid connector IDs, without duplicates
pub fn connector_ids(ids: Vec<String>, pack: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        crate::services::domain::ConnectorId::parse(id.clone())
            .map_err(|e| format!("Pack '{pack}' requires_connectors: {e}"))?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Where a pack repo's manifest lives.
pub const PACK_MANIFEST_PATH: &str = ".nandi/manifest.yaml";

impl PackFile {
    pub fn from_yaml(content: &str) -> Result<Self, String> {
        let file: Self = serde_yaml::from_str(content)
            .map_err(|e| format!("The pack's {PACK_MANIFEST_PATH} isn't valid: {e}"))?;
        parse_version(&file.version).map_err(|e| format!("{PACK_MANIFEST_PATH}: version {e}"))?;
        parse_version(&file.min_nandi_version)
            .map_err(|e| format!("{PACK_MANIFEST_PATH}: min_nandi_version {e}"))?;
        Ok(file)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const SAMPLE_INDEX: &str = r#"{
      "schema_version": 1,
      "agents": [
        {
          "id": "arxiv-researcher",
          "name": "ArXiv Researcher",
          "description": "Searches arXiv, summarizes papers, tracks citations",
          "publisher": "Alice",
          "repo": "github.com/alice/nandi-agents",
          "path": "agents/arxiv-researcher",
          "latest_version": "1.2.0",
          "tags": ["research", "academic", "papers"],
          "min_nandi_version": "0.1.0",
          "capabilities": {
            "tools": ["shell", "read_file"],
            "network": ["arxiv.org", "export.arxiv.org"],
            "pilots": []
          },
          "signature": "ed25519:abc"
        },
        { "id": "../escape", "name": "Bad", "publisher": "x", "repo": "github.com/x/y",
          "path": "a", "latest_version": "1.0.0" },
        { "id": "traversal", "name": "Bad", "publisher": "x", "repo": "github.com/x/y",
          "path": "../../etc", "latest_version": "1.0.0" },
        { "id": "future", "name": "Future", "publisher": "x", "repo": "github.com/x/y",
          "path": "future", "latest_version": "2.0.0", "min_nandi_version": "99.0.0" }
      ],
      "packs": [
        {
          "id": "research-pack",
          "name": "Research Pack",
          "publisher": "vaultdbai",
          "repo": "github.com/vaultdbai/nandi-research-pack",
          "latest_version": "1.0.0",
          "agents": [{ "id": "arxiv-researcher", "name": "ArXiv Researcher" }],
          "workflows": ["literature-review"],
          "install_size_kb": 24,
          "estimated_cost_usd": { "min": 0.01, "max": 0.25 }
        }
      ]
    }"#;

    #[test]
    fn parses_a_sample_index_and_skips_bad_entries() {
        let index = parse_index(SAMPLE_INDEX, "github.com/r/registry").unwrap();
        let ids: Vec<&str> = index.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["arxiv-researcher", "future"]);
        assert_eq!(index.skipped.len(), 2);

        let arxiv = &index.agents[0];
        assert_eq!(arxiv.publisher, "alice", "handles are lowercased");
        assert_eq!(arxiv.git_ref, "v1.2.0");
        assert_eq!(
            arxiv.capabilities.network,
            ["arxiv.org", "export.arxiv.org"]
        );
        assert!(arxiv.compatible);
        assert_eq!(arxiv.registry, "github.com/r/registry");
        assert!(!index.agents[1].compatible, "needs a newer Nandi");

        let pack = &index.packs[0];
        assert_eq!(pack.agents[0].id, "arxiv-researcher");
        assert_eq!(pack.estimated_cost_usd.unwrap().max_usd, 0.25);
    }

    #[test]
    fn reads_a_bare_agent_list_and_rejects_newer_formats() {
        let list = r#"[{"id":"a","name":"A","publisher":"p","repo":"github.com/p/r","path":"a","latest_version":"0.1.0"}]"#;
        assert_eq!(parse_index(list, "r").unwrap().agents.len(), 1);
        assert!(parse_index(r#"{"schema_version": 2}"#, "r").is_err());
        assert!(parse_index("not json", "r").is_err());
    }

    #[test]
    fn versions_and_tags() {
        assert!(is_compatible("0.0.1"));
        assert!(!is_compatible("99.0.0"));
        assert!(!is_compatible("garbage"));
        assert_eq!(
            split_tag("refs/tags/v1.2.0").unwrap().1.to_string(),
            "1.2.0"
        );
        assert_eq!(split_tag("refs/tags/v1.2.0^{}").unwrap().1.minor, 2);
        assert!(split_tag("refs/tags/latest").is_none());
        let (prefix, version) = split_tag("refs/tags/arxiv-researcher-v1.2.0").unwrap();
        assert_eq!(
            (prefix.as_str(), version.to_string().as_str()),
            ("arxiv-researcher-", "1.2.0")
        );
        assert_eq!(split_tag("v2.0.0").unwrap().0, "");
        assert!(split_tag("main").is_none());
    }

    #[test]
    fn rejects_unsafe_refs_and_asset_urls() {
        let entry = |extra: &str| {
            format!(
                r#"{{"id":"a","name":"A","publisher":"p","repo":"github.com/p/r","path":"a","latest_version":"1.0.0"{extra}}}"#
            )
        };
        assert!(parse_agent_entry(&entry(r#","ref":"--upload-pack=x""#), "r").is_err());
        assert!(parse_agent_entry(&entry(r#","icon_url":"javascript:alert(1)""#), "r").is_err());
        assert!(parse_agent_entry(&entry(r#","readme_url":"agents/a/README.md""#), "r").is_ok());
        assert!(parse_agent_entry(&entry(r#","ref":"main""#), "r").is_ok());
        assert!(parse_agent_entry(&entry(r#","commit":"abc""#), "r").is_err());
        let sha = "E5D4C3B2A1F0E5D4C3B2A1F0E5D4C3B2A1F0E5D4";
        let pinned = parse_agent_entry(&entry(&format!(r#","commit":"{sha}""#)), "r").unwrap();
        assert_eq!(pinned.commit.unwrap(), sha.to_ascii_lowercase());
    }

    #[test]
    fn pack_agents_may_be_objects_or_ids() {
        let pack = |agents: &str| {
            parse_pack_entry(
                &format!(
                    r#"{{"id": "p", "name": "P", "publisher": "x",
                        "repo": "github.com/x/y", "latest_version": "1.0.0",
                        "agents": {agents}}}"#
                ),
                "r",
            )
        };

        let mixed = pack(
            r#"[{"id": "arxiv-researcher", "name": "ArXiv Researcher",
                 "description": "Searches arXiv"}, "paper-summarizer"]"#,
        )
        .unwrap();
        assert_eq!(
            mixed.agents,
            [
                PackAgentSummary {
                    id: "arxiv-researcher".into(),
                    name: "ArXiv Researcher".into(),
                    description: "Searches arXiv".into(),
                },
                PackAgentSummary {
                    id: "paper-summarizer".into(),
                    name: "paper-summarizer".into(),
                    description: String::new(),
                },
            ]
        );
        // IDs are validated either way
        assert!(pack(r#"["../escape"]"#).is_err());
        assert!(pack(r#"[{"id": "../escape", "name": "x"}]"#).is_err());
        assert!(pack("[42]").is_err());
    }

    #[test]
    fn pack_suggested_asks_are_optional_and_trimmed() {
        let pack = |extra: &str| {
            parse_pack_entry(
                &format!(
                    r#"{{"id": "p", "name": "P", "publisher": "x",
                        "repo": "github.com/x/y", "latest_version": "1.0.0"{extra}}}"#
                ),
                "r",
            )
            .unwrap()
        };
        assert!(pack("").suggested_asks.is_empty());
        let long = "x".repeat(500);
        let asks = pack(&format!(
            r#", "suggested_asks": ["  Summarize my email ", "", "{long}", "Draft a status update", "a fourth"]"#
        ))
        .suggested_asks;
        assert_eq!(asks.len(), 3);
        assert_eq!(asks[0], "Summarize my email");
        assert_eq!(asks[1].chars().count(), MAX_SUGGESTED_ASK_CHARS);
        assert_eq!(asks[2], "Draft a status update");
    }

    #[test]
    fn parses_pack_manifests() {
        let pack = PackFile::from_yaml(
            "id: research-pack\nname: Research\nversion: 1.0.0\nagents: [arxiv]\n",
        )
        .unwrap();
        assert_eq!(pack.min_nandi_version, "0.0.0");
        assert!(PackFile::from_yaml("id: Bad_Id\nname: x\nversion: 1.0.0\n").is_err());
        assert!(PackFile::from_yaml("id: p\nname: x\nversion: one\n").is_err());
    }
}
