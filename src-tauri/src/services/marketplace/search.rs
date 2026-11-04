//! Ranked search over the catalog (installed + available agents and packs):
//! name match > description match > tag match (> publisher match), recency
//! as the tie-break (wave 2B redesign, Track C).
//!
//! The registries carry no publish dates, so "recency" is the catalog order:
//! registries append entries as they're published, so a later entry is a
//! newer one. Installed items no registry lists any more (a removed
//! registry) come after every catalog entry, newest install first.
//!
//! The Discover tab ranks the same way client-side for instant results
//! (`src/components/marketplace/marketplace-index.ts`); this is what the
//! command palette and the web API use.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::install::InstalledItem;
use super::manifest::{AgentManifest, PackManifest};
use super::{list_available_agents, list_available_packs, list_installed, MarketplaceFilter};
use crate::services::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceSearchHit {
    /// `agent` | `pack`
    pub kind: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub score: f64,
    pub installed: bool,
}

/// The most hits one search returns
const MAX_HITS: usize = 50;

// How well one query word matched, by field (higher wins)
const NAME_EXACT: f64 = 1.0;
const NAME_PREFIX: f64 = 0.9;
const NAME_WORD: f64 = 0.8;
const NAME_CONTAINS: f64 = 0.7;
const DESCRIPTION: f64 = 0.4;
const TAG_EXACT: f64 = 0.25;
const TAG_CONTAINS: f64 = 0.2;
const PUBLISHER: f64 = 0.1;

/// One searchable catalog entry.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchDoc {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub tags: Vec<String>,
    pub installed: bool,
    /// Higher is newer (see the module docs)
    pub recency: u64,
}

impl SearchDoc {
    fn from_agent(agent: &AgentManifest, recency: u64) -> Self {
        Self {
            kind: "agent".to_string(),
            id: agent.id.clone(),
            name: agent.name.clone(),
            description: agent.description.clone(),
            publisher: agent.publisher.clone(),
            tags: agent.tags.clone(),
            installed: agent.installed_version.is_some(),
            recency,
        }
    }

    fn from_pack(pack: &PackManifest, recency: u64) -> Self {
        Self {
            kind: "pack".to_string(),
            id: pack.id.clone(),
            name: pack.name.clone(),
            description: pack.description.clone(),
            publisher: pack.publisher.clone(),
            tags: pack.tags.clone(),
            installed: pack.installed_version.is_some(),
            recency,
        }
    }
}

fn ordinal(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX)
}

/// The index over `agents` and `packs` (catalog order), plus installed items
/// the catalog doesn't list.
pub fn build_index(
    agents: &[AgentManifest],
    packs: &[PackManifest],
    installed: &[InstalledItem],
) -> Vec<SearchDoc> {
    // Installed-only items rank below every catalog entry on recency
    let base = ordinal(installed.len());
    let mut docs: Vec<SearchDoc> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| SearchDoc::from_agent(a, base.saturating_add(ordinal(i))))
        .chain(
            packs
                .iter()
                .enumerate()
                .map(|(i, p)| SearchDoc::from_pack(p, base.saturating_add(ordinal(i)))),
        )
        .collect();

    let mut orphans: Vec<&InstalledItem> = installed
        .iter()
        .filter(|item| {
            !docs
                .iter()
                .any(|doc| doc.kind == item.kind && doc.id == item.id)
        })
        .collect();
    // Oldest install first, so the newest gets the highest ordinal
    orphans.sort_by(|a, b| a.installed_at.cmp(&b.installed_at));
    let orphans: Vec<SearchDoc> = orphans
        .into_iter()
        .enumerate()
        .map(|(i, item)| SearchDoc {
            kind: item.kind.clone(),
            id: item.id.clone(),
            name: item.name.clone(),
            description: String::new(),
            publisher: String::new(),
            tags: Vec::new(),
            installed: true,
            recency: ordinal(i),
        })
        .collect();
    docs.extend(orphans);
    docs
}

/// How well `word` (lowercase) matches `doc`: its best field, or `None`.
fn word_score(doc: &SearchDoc, word: &str) -> Option<f64> {
    let name = doc.name.to_lowercase();
    let name_score = if name == word {
        Some(NAME_EXACT)
    } else if name.starts_with(word) {
        Some(NAME_PREFIX)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|part| part.starts_with(word))
    {
        Some(NAME_WORD)
    } else if name.contains(word) {
        Some(NAME_CONTAINS)
    } else {
        None
    };
    name_score
        .or_else(|| {
            doc.description
                .to_lowercase()
                .contains(word)
                .then_some(DESCRIPTION)
        })
        .or_else(|| {
            let tags: Vec<String> = doc.tags.iter().map(|t| t.to_lowercase()).collect();
            if tags.iter().any(|t| t == word) {
                Some(TAG_EXACT)
            } else if tags.iter().any(|t| t.contains(word)) {
                Some(TAG_CONTAINS)
            } else {
                None
            }
        })
        .or_else(|| {
            doc.publisher
                .to_lowercase()
                .contains(word)
                .then_some(PUBLISHER)
        })
}

/// Docs matching every word of `query`, best first. The score is the mean of
/// each word's best field, in (0, 1]; ties go to the newer entry, then the
/// name.
pub fn rank(docs: &[SearchDoc], query: &str) -> Vec<MarketplaceSearchHit> {
    let query = query.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    #[allow(clippy::cast_precision_loss)] // a handful of words
    let count = words.len() as f64;
    let mut scored: Vec<(f64, &SearchDoc)> = docs
        .iter()
        .filter_map(|doc| {
            let total = words
                .iter()
                .map(|word| word_score(doc, word))
                .sum::<Option<f64>>()?;
            Some((total / count, doc))
        })
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sb.total_cmp(sa)
            .then_with(|| b.recency.cmp(&a.recency))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    scored
        .into_iter()
        .take(MAX_HITS)
        .map(|(score, doc)| MarketplaceSearchHit {
            kind: doc.kind.clone(),
            id: doc.id.clone(),
            name: doc.name.clone(),
            description: doc.description.clone(),
            score,
            installed: doc.installed,
        })
        .collect()
}

/// Ranked catalog matches for `query` (none for a blank query). A registry
/// that can't be read is skipped; if nothing can be read at all, its error.
pub async fn search(state: &AppState, query: &str) -> Result<Vec<MarketplaceSearchHit>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let filter = MarketplaceFilter::default();
    let (agents, packs, installed) = tokio::join!(
        list_available_agents(state, &filter),
        list_available_packs(state),
        list_installed(state),
    );
    let installed = installed
        .inspect_err(|e| log::debug!("Searching without installed items: {e}"))
        .unwrap_or_default();
    let (agents, packs) = match (agents, packs) {
        (Err(error), Err(_)) if installed.is_empty() => return Err(error),
        (agents, packs) => (agents.unwrap_or_default(), packs.unwrap_or_default()),
    };
    Ok(rank(&build_index(&agents, &packs, &installed), query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::marketplace::manifest::ManifestCapabilities;

    fn doc(id: &str, name: &str, description: &str, tags: &[&str], recency: u64) -> SearchDoc {
        SearchDoc {
            kind: "agent".to_string(),
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            publisher: "alice".to_string(),
            tags: tags.iter().map(ToString::to_string).collect(),
            installed: false,
            recency,
        }
    }

    fn ids(hits: &[MarketplaceSearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn name_beats_description_beats_tag() {
        let docs = [
            doc("tagged", "Helper", "Does things", &["research"], 9),
            doc("described", "Paper Bot", "Research assistant", &[], 5),
            doc("named", "Research Pal", "Finds papers", &[], 1),
        ];
        let hits = rank(&docs, "research");
        assert_eq!(ids(&hits), ["named", "described", "tagged"]);
        assert!(hits[0].score > hits[1].score && hits[1].score > hits[2].score);
    }

    #[test]
    fn exact_and_prefix_names_rank_above_substrings() {
        let docs = [
            doc("inner", "Myresearch", "", &[], 3),
            doc("word", "Deep Research", "", &[], 2),
            doc("prefix", "Researcher", "", &[], 1),
            doc("exact", "Research", "", &[], 0),
        ];
        assert_eq!(
            ids(&rank(&docs, "research")),
            ["exact", "prefix", "word", "inner"]
        );
    }

    #[test]
    fn recency_breaks_ties_then_name() {
        let docs = [
            doc("old", "Code Reviewer", "", &[], 1),
            doc("new", "Code Formatter", "", &[], 7),
            doc("b", "Code B", "", &[], 0),
            doc("a", "Code A", "", &[], 0),
        ];
        assert_eq!(ids(&rank(&docs, "code")), ["new", "old", "a", "b"]);
    }

    #[test]
    fn every_word_must_match_case_insensitively() {
        let docs = [
            doc("both", "ArXiv Researcher", "Finds papers", &[], 0),
            doc("one", "ArXiv Mirror", "Copies files", &[], 0),
        ];
        assert_eq!(ids(&rank(&docs, "ARXIV papers")), ["both"]);
        assert!(rank(&docs, "   ").is_empty());
        assert!(rank(&docs, "nothing").is_empty());
    }

    #[test]
    fn publisher_matches_rank_below_tags() {
        let docs = [
            doc("by-alice", "Helper", "", &[], 9),
            doc("tag", "Other", "", &["alice-tools"], 0),
        ];
        assert_eq!(ids(&rank(&docs, "alice")), ["tag", "by-alice"]);
    }

    fn agent(id: &str, installed: bool) -> AgentManifest {
        AgentManifest {
            id: id.to_string(),
            name: "ArXiv".to_string(),
            description: String::new(),
            publisher: "alice".to_string(),
            repo: "github.com/alice/a".to_string(),
            path: format!("agents/{id}"),
            latest_version: "1.0.0".to_string(),
            git_ref: "v1.0.0".to_string(),
            tags: vec![],
            min_nandi_version: "0.0.0".to_string(),
            capabilities: ManifestCapabilities::default(),
            signature: None,
            commit: None,
            icon_url: None,
            readme_url: None,
            estimated_cost_usd: None,
            registry: "r".to_string(),
            compatible: true,
            installed_version: installed.then(|| "1.0.0".to_string()),
            verified: false,
        }
    }

    fn installed(id: &str, at: &str) -> InstalledItem {
        InstalledItem {
            kind: "agent".to_string(),
            id: id.to_string(),
            name: format!("{id} agent"),
            source: String::new(),
            version: "1.0.0".to_string(),
            git_ref: String::new(),
            commit: String::new(),
            installed_at: at.to_string(),
            signature_verified: false,
            registry: None,
            agents: vec![],
            modified: false,
        }
    }

    #[test]
    fn index_covers_the_catalog_and_installed_only_items() {
        let docs = build_index(
            &[agent("arxiv", true)],
            &[],
            &[
                installed("arxiv", "2026-01-01T00:00:00Z"),
                installed("gone-new", "2026-03-01T00:00:00Z"),
                installed("gone-old", "2026-02-01T00:00:00Z"),
            ],
        );
        assert_eq!(docs.len(), 3);
        assert!(docs.iter().all(|d| d.installed));
        // Installed-only items: the newer install ranks first
        assert_eq!(ids(&rank(&docs, "agent")), ["gone-new", "gone-old"]);
        let catalog = docs.iter().find(|d| d.id == "arxiv").unwrap();
        assert!(docs
            .iter()
            .filter(|d| d.id != "arxiv")
            .all(|d| d.recency < catalog.recency));
    }

    #[test]
    fn later_catalog_entries_are_newer() {
        let docs = build_index(&[agent("first", false), agent("second", false)], &[], &[]);
        // Same name: recency decides
        assert_eq!(ids(&rank(&docs, "arxiv")), ["second", "first"]);
    }
}
