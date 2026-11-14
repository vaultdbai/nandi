//! First-use consent for marketplace agents.
//!
//! An installed (vendored) agent doesn't run until the user has approved,
//! category by category, everything its skill.md requests: tools, network
//! hosts, AWS services, pilots, connectors, shared folders with their
//! access, and taking tasks from others. What was approved is recorded in
//! `settings.yaml` (`consent.<agent>`), item by item, so a later version
//! that asks for more (a new host, a new method on a host, a folder made
//! writable) is blocked again until approved. Asking for less keeps the
//! approval. The user's own agents (created or forked) never need consent.
//!
//! The check runs where every run starts (`chat::submit`), so chat,
//! workflows, and triggers are all covered.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::connectors::{self, folders::Access};
use crate::services::domain::capability::NetworkPolicy;
use crate::services::domain::settings::AgentConsent;
use crate::services::domain::{Agent, AgentId, EntityKind};
use crate::services::repo_store::RepoStore;
use crate::services::repos::SingletonRepository;

/// Every runtime tool (an agent without `allowed-tools`).
const ALL_TOOLS: &str = "*";

/// The one item of the `delegation` category.
const ACCEPTS_DELEGATION: &str = "accepts_delegation";

/// One category the user approves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConsentCategory {
    /// tools | network | aws | pilots | connectors | folders | delegation
    pub key: String,
    /// What the agent requests (`*` in tools: every tool; `api.example.com
    /// (GET, POST)` in network: only those methods on that host;
    /// `docs:write` in folders: the folder `docs`, read and write)
    pub items: Vec<String>,
    /// Already approved (all of `items`)
    pub granted: bool,
}

/// A category the user approved, with the items the dialog showed them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedCategory {
    pub key: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConsentStatus {
    pub agent_id: String,
    pub agent_name: String,
    /// It's a marketplace agent (the user's own never need consent)
    pub required: bool,
    /// It may run
    pub satisfied: bool,
    /// The categories it requests (empty ones left out)
    pub categories: Vec<ConsentCategory>,
}

/// One kind of access the user approves, in the order the dialog lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Tools,
    Network,
    Aws,
    Pilots,
    Connectors,
    Folders,
    Delegation,
}

impl Category {
    const ALL: [Self; 7] = [
        Self::Tools,
        Self::Network,
        Self::Aws,
        Self::Pilots,
        Self::Connectors,
        Self::Folders,
        Self::Delegation,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Network => "network",
            Self::Aws => "aws",
            Self::Pilots => "pilots",
            Self::Connectors => "connectors",
            Self::Folders => "folders",
            Self::Delegation => "delegation",
        }
    }

    fn parse(key: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|category| category.key() == key)
            .ok_or_else(|| format!("'{key}' is not a capability category"))
    }

    /// This category's items in `consent`.
    fn items(self, consent: &AgentConsent) -> Vec<String> {
        match self {
            Self::Tools => consent.tools.clone(),
            Self::Network => consent.network.clone(),
            Self::Aws => consent.aws.clone(),
            Self::Pilots => consent.pilots.clone(),
            Self::Connectors => consent.connectors.clone(),
            Self::Folders => consent.folders.clone(),
            Self::Delegation if consent.accepts_delegation => {
                vec![ACCEPTS_DELEGATION.to_string()]
            }
            Self::Delegation => Vec::new(),
        }
    }

    /// Replaces this category's items in `consent`.
    fn set(self, consent: &mut AgentConsent, items: Vec<String>) {
        match self {
            Self::Tools => consent.tools = items,
            Self::Network => consent.network = items,
            Self::Aws => consent.aws = items,
            Self::Pilots => consent.pilots = items,
            Self::Connectors => consent.connectors = items,
            Self::Folders => consent.folders = items,
            Self::Delegation => {
                consent.accepts_delegation = items.iter().any(|item| item == ACCEPTS_DELEGATION);
            }
        }
    }

    /// Whether the `approved` items cover one `requested` item.
    fn covers(self, approved: &[String], requested: &str) -> bool {
        approved.iter().any(|item| match self {
            Self::Tools => item == ALL_TOOLS || item == requested,
            Self::Network => host_covers(item, requested),
            Self::Folders => folder_covers(item, requested),
            _ => item == requested,
        })
    }
}

/// A network item: `api.example.com` for any method, or
/// `api.example.com (GET, POST)` for only those methods. The methods are
/// sorted, so the same access always reads the same.
fn host_item(host: &str, methods: Option<&BTreeSet<String>>) -> String {
    match methods {
        None => host.to_string(),
        Some(methods) => {
            let methods: Vec<&str> = methods.iter().map(String::as_str).collect();
            format!("{host} ({})", methods.join(", "))
        }
    }
}

/// A network item's host and methods (`None`: any method).
fn parse_host_item(item: &str) -> (&str, Option<BTreeSet<&str>>) {
    match item
        .strip_suffix(')')
        .and_then(|rest| rest.split_once(" ("))
    {
        Some((host, methods)) => (host, Some(methods.split(", ").collect())),
        None => (item, None),
    }
}

/// The host a network item is about (`api.example.com (GET)` →
/// `api.example.com`), as the registry entry lists it.
pub fn network_host(item: &str) -> &str {
    parse_host_item(item).0
}

/// Approved any method covers every method on the host; approved methods
/// cover only a request for some of those methods, never any method.
fn host_covers(approved: &str, requested: &str) -> bool {
    let (approved_host, approved_methods) = parse_host_item(approved);
    let (requested_host, requested_methods) = parse_host_item(requested);
    approved_host == requested_host
        && match (approved_methods, requested_methods) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(approved), Some(requested)) => requested.is_subset(&approved),
        }
}

/// One item per host. Rules for the same host add up, as at the proxy,
/// where a call passes if any rule matches: a rule with no methods makes
/// the host any-method.
fn requested_hosts(network: &NetworkPolicy) -> Vec<String> {
    // Host → its methods (`None`: any method), in the order first named
    let mut hosts: Vec<(&str, Option<BTreeSet<String>>)> = Vec::new();
    for rule in &network.allow {
        let methods = (!rule.methods.is_empty()).then(|| rule.methods.iter().cloned().collect());
        match hosts.iter_mut().find(|(host, _)| *host == rule.host) {
            None => hosts.push((&rule.host, methods)),
            Some((_, known)) => match (known.as_mut(), methods) {
                (Some(known), Some(methods)) => known.extend(methods),
                _ => *known = None,
            },
        }
    }
    hosts
        .into_iter()
        .map(|(host, methods)| host_item(host, methods.as_ref()))
        .collect()
}

/// A folder item: `docs:read` or `docs:write`.
fn folder_item(id: &str, access: Access) -> String {
    format!("{id}:{}", access.as_str())
}

/// Approved read and write covers a later read-only request; approved
/// read-only never covers write.
fn folder_covers(approved: &str, requested: &str) -> bool {
    fn parse(item: &str) -> Option<(&str, Access)> {
        let (id, access) = item.rsplit_once(':')?;
        match access {
            "read" => Some((id, Access::Read)),
            "write" => Some((id, Access::Write)),
            _ => None,
        }
    }
    match (parse(approved), parse(requested)) {
        (Some((approved_id, approved)), Some((requested_id, requested))) => {
            approved_id == requested_id && approved >= requested
        }
        _ => false,
    }
}

/// What an agent's skill.md requests.
pub fn requested(agent: &Agent) -> AgentConsent {
    let capabilities = agent.capabilities();
    let connectors = connectors::declared_connectors(agent);
    AgentConsent {
        tools: capabilities
            .tools()
            .allowed()
            .map_or_else(|| vec![ALL_TOOLS.to_string()], <[String]>::to_vec),
        network: requested_hosts(capabilities.network()),
        aws: capabilities
            .aws()
            .map(|aws| aws.services.iter().map(|s| s.service.clone()).collect())
            .unwrap_or_default(),
        pilots: capabilities
            .pilots()
            .iter()
            .map(|p| p.as_str().to_string())
            .collect(),
        folders: requested_folders(agent, &connectors),
        connectors,
        accepts_delegation: capabilities.accepts_delegation(),
    }
}

/// The folders the agent names under its connectors, each at the most it
/// can get: a folder named without `access` gets the folder's own access,
/// which may be write.
fn requested_folders(agent: &Agent, connectors: &[String]) -> Vec<String> {
    let options = connectors::declared_options(agent);
    let mut items = Vec::new();
    for id in connectors {
        // A malformed list gives the agent no folders at run time either
        for grant in connectors::folders::grants(options.get(id)).unwrap_or_default() {
            let item = folder_item(&grant.id, grant.access.unwrap_or(Access::Write));
            if !items.contains(&item) {
                items.push(item);
            }
        }
    }
    items
}

fn categories(requested: &AgentConsent, approved: &AgentConsent) -> Vec<ConsentCategory> {
    Category::ALL
        .into_iter()
        .filter_map(|category| {
            let items = category.items(requested);
            if items.is_empty() {
                return None;
            }
            let approved = category.items(approved);
            Some(ConsentCategory {
                key: category.key().to_string(),
                granted: items.iter().all(|item| category.covers(&approved, item)),
                items,
            })
        })
        .collect()
}

/// Whether the agent needs consent and has it.
pub fn status(store: &RepoStore, agent_id: &str) -> Result<ConsentStatus, String> {
    let agent = store.agent(agent_id)?;
    let required = store
        .repos()
        .agents
        .locate(agent_id)
        .is_some_and(|place| place.origin.is_vendored());
    let granted = store
        .read(|repo| repo.settings().consent(agent_id).cloned())
        .unwrap_or_default();
    let categories = categories(&requested(&agent), &granted);
    Ok(ConsentStatus {
        agent_id: agent_id.to_string(),
        agent_name: agent.name().to_string(),
        satisfied: !required || categories.iter().all(|c| c.granted),
        required,
        categories,
    })
}

/// Fails unless the agent may run: a marketplace agent needs every
/// category it requests approved. Unknown IDs (Studio) pass.
pub fn check(store: &RepoStore, agent_id: &str) -> Result<(), String> {
    if AgentId::parse(agent_id).is_err() || store.repos().agents.locate(agent_id).is_none() {
        return Ok(());
    }
    let status = status(store, agent_id)?;
    if status.satisfied {
        return Ok(());
    }
    let missing: Vec<&str> = status
        .categories
        .iter()
        .filter(|c| !c.granted)
        .map(|c| c.key.as_str())
        .collect();
    Err(format!(
        "{} is a marketplace agent and needs your approval before it runs ({}). Open it to review what it can do.",
        status.agent_name,
        missing.join(", ")
    ))
}

/// Approves what the user was shown: each category's items, exactly as the
/// dialog listed them. Anything the agent asks for beyond that (an update
/// landed while the dialog was open) stays unapproved, and the status says
/// so.
pub fn grant(
    store: &RepoStore,
    agent_id: &str,
    approved: &[ApprovedCategory],
) -> Result<ConsentStatus, String> {
    store.agent(agent_id)?;
    let mut settings = store.read(|repo| repo.settings().clone());
    let mut consent = settings.consent(agent_id).cloned().unwrap_or_default();
    for shown in approved {
        Category::parse(&shown.key)?.set(&mut consent, shown.items.clone());
    }
    settings
        .set_consent(agent_id, consent)
        .map_err(|e| e.to_string())?;
    store.repos().settings.save(&settings)?;
    store.commit(EntityKind::Settings, "settings")?;
    status(store, agent_id)
}

/// Forgets the agent's consent (it was uninstalled).
pub fn forget(store: &RepoStore, agent_id: &str) -> Result<(), String> {
    let mut settings = store.read(|repo| repo.settings().clone());
    if settings.consent(agent_id).is_none() {
        return Ok(());
    }
    settings
        .set_consent(agent_id, AgentConsent::default())
        .map_err(|e| e.to_string())?;
    store.repos().settings.save(&settings)?;
    store.commit(EntityKind::Settings, "settings")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::services::marketplace::manifest::ManifestCapabilities;
    use crate::services::repos::{RepoLayout, Repos};
    use std::fs;

    fn store_with(skill: &str, vendored: bool) -> (tempfile::TempDir, RepoStore) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        let dir = if vendored {
            layout.vendored_agent_dir("alice", "arxiv")
        } else {
            layout.agent_dir("arxiv")
        };
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("skill.md"), skill).unwrap();
        let store = RepoStore::sqlite(Repos::new(layout), Db::in_memory());
        store.load().unwrap();
        (tmp, store)
    }

    /// A new version of the agent lands on disk.
    fn update_skill(store: &RepoStore, skill: &str) {
        let place = store.repos().agents.locate("arxiv").unwrap();
        fs::write(place.dir.join("skill.md"), skill).unwrap();
    }

    /// What the dialog shows for `keys`, as the user approves it.
    fn shown(status: &ConsentStatus, keys: &[&str]) -> Vec<ApprovedCategory> {
        status
            .categories
            .iter()
            .filter(|c| keys.contains(&c.key.as_str()))
            .map(|c| ApprovedCategory {
                key: c.key.clone(),
                items: c.items.clone(),
            })
            .collect()
    }

    /// Approves everything the agent asks for now.
    fn approve_all(store: &RepoStore) -> ConsentStatus {
        let current = super::status(store, "arxiv").unwrap();
        let keys: Vec<&str> = current.categories.iter().map(|c| c.key.as_str()).collect();
        grant(store, "arxiv", &shown(&current, &keys)).unwrap()
    }

    fn ungranted(status: &ConsentStatus) -> Vec<&str> {
        status
            .categories
            .iter()
            .filter(|c| !c.granted)
            .map(|c| c.key.as_str())
            .collect()
    }

    fn skill(frontmatter: &str) -> String {
        format!("---\nname: ArXiv\n{frontmatter}---\nGo.")
    }

    /// `network.allow` frontmatter: one rule per (host, methods) pair, with
    /// `[]` for any method.
    fn network(rules: &[(&str, &str)]) -> String {
        let rules: String = rules
            .iter()
            .map(|(host, methods)| format!("    - host: {host}\n      methods: {methods}\n"))
            .collect();
        format!("network:\n  allow:\n{rules}")
    }

    fn category<'a>(status: &'a ConsentStatus, key: &str) -> &'a ConsentCategory {
        status.categories.iter().find(|c| c.key == key).unwrap()
    }

    const SKILL: &str = "---\nname: ArXiv\nallowed-tools: [shell, read_file]\nnetwork:\n  allow:\n    - host: export.arxiv.org\n---\nGo.";

    #[test]
    fn marketplace_agents_need_every_category_approved() {
        let (tmp, store) = store_with(SKILL, true);
        let current = super::status(&store, "arxiv").unwrap();
        assert!(current.required && !current.satisfied);
        let keys: Vec<&str> = current.categories.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["tools", "network"]);
        assert!(check(&store, "arxiv")
            .unwrap_err()
            .contains("tools, network"));

        let partial = grant(&store, "arxiv", &shown(&current, &["tools"])).unwrap();
        assert!(!partial.satisfied);
        let all = grant(&store, "arxiv", &shown(&current, &["network"])).unwrap();
        assert!(all.satisfied);
        check(&store, "arxiv").unwrap();

        // A new version asking for more is blocked again
        update_skill(
            &store,
            &SKILL.replace("export.arxiv.org", "evil.example.com"),
        );
        let current = super::status(&store, "arxiv").unwrap();
        assert!(!current.satisfied);
        assert_eq!(ungranted(&current), ["network"], "tools unchanged");

        forget(&store, "arxiv").unwrap();
        assert!(store.read(|r| r.settings().consent("arxiv").is_none()));
        drop(tmp);
    }

    #[test]
    fn updates_that_widen_access_need_approval_again() {
        let cases = [
            (
                "a new connector",
                "",
                "connectors: [filesystem]\n",
                "connectors",
            ),
            (
                "a new folder",
                "connectors:\n  - id: filesystem\n    folders: [docs]\n",
                "connectors:\n  - id: filesystem\n    folders: [docs, reports]\n",
                "folders",
            ),
            (
                "a read-only folder made writable",
                "connectors:\n  - id: filesystem\n    folders: [{id: docs, access: read}]\n",
                "connectors:\n  - id: filesystem\n    folders: [docs]\n",
                "folders",
            ),
            (
                "taking tasks from others",
                "",
                "accepts_delegation: true\n",
                "delegation",
            ),
            (
                "a new tool",
                "allowed-tools: [read_file]\n",
                "allowed-tools: [read_file, shell]\n",
                "tools",
            ),
        ];
        for (change, before, after, widened) in cases {
            let (_tmp, store) = store_with(&skill(before), true);
            assert!(approve_all(&store).satisfied, "{change}: approved");
            update_skill(&store, &skill(after));
            let current = super::status(&store, "arxiv").unwrap();
            assert_eq!(ungranted(&current), [widened], "{change}");
            assert!(check(&store, "arxiv").is_err(), "{change}: blocked");
        }
    }

    #[test]
    fn updates_that_widen_network_access_need_approval_again() {
        let cases = [
            (
                "a new method on a host",
                network(&[("api.example.com", "[GET]")]),
                network(&[("api.example.com", "[GET, POST]")]),
            ),
            (
                "a method-limited host opened to every method",
                network(&[("api.example.com", "[GET]")]),
                network(&[("api.example.com", "[]")]),
            ),
            (
                "a second rule adding a method to a host",
                network(&[("api.example.com", "[GET]")]),
                network(&[
                    ("api.example.com", "[GET]"),
                    ("api.example.com", "[DELETE]"),
                ]),
            ),
            (
                "a new host",
                network(&[("api.example.com", "[GET]")]),
                network(&[("api.example.com", "[GET]"), ("cdn.example.com", "[GET]")]),
            ),
        ];
        for (change, before, after) in cases {
            let (_tmp, store) = store_with(&skill(&before), true);
            assert!(approve_all(&store).satisfied, "{change}: approved");
            update_skill(&store, &skill(&after));
            let current = super::status(&store, "arxiv").unwrap();
            assert_eq!(ungranted(&current), ["network"], "{change}");
            assert!(check(&store, "arxiv").is_err(), "{change}: blocked");
        }
    }

    #[test]
    fn updates_that_narrow_network_access_keep_their_approval() {
        let cases = [
            (
                "every method narrowed to some",
                network(&[("api.example.com", "[]")]),
                network(&[("api.example.com", "[GET]")]),
            ),
            (
                "a method dropped",
                network(&[("api.example.com", "[GET, POST]")]),
                network(&[("api.example.com", "[POST]")]),
            ),
            (
                "the same methods in another order and case",
                network(&[("api.example.com", "[GET, POST]")]),
                network(&[("api.example.com", "[post, get]")]),
            ),
            (
                "a host dropped",
                network(&[("api.example.com", "[]"), ("cdn.example.com", "[GET]")]),
                network(&[("api.example.com", "[]")]),
            ),
        ];
        for (change, before, after) in cases {
            let (_tmp, store) = store_with(&skill(&before), true);
            assert!(approve_all(&store).satisfied, "{change}: approved");
            update_skill(&store, &skill(&after));
            assert!(
                super::status(&store, "arxiv").unwrap().satisfied,
                "{change}"
            );
        }
    }

    #[test]
    fn network_items_name_each_host_once_with_its_methods() {
        let cases = [
            (
                "any method",
                network(&[("a.example.com", "[]")]),
                vec!["a.example.com"],
            ),
            (
                "sorted methods",
                network(&[("a.example.com", "[post, GET]")]),
                vec!["a.example.com (GET, POST)"],
            ),
            (
                "rules for one host add up",
                network(&[("a.example.com", "[GET]"), ("a.example.com", "[POST]")]),
                vec!["a.example.com (GET, POST)"],
            ),
            (
                "a rule with no methods opens the host",
                network(&[("a.example.com", "[GET]"), ("a.example.com", "[]")]),
                vec!["a.example.com"],
            ),
            (
                "hosts in the order named",
                network(&[("b.example.com", "[GET]"), ("a.example.com", "[]")]),
                vec!["b.example.com (GET)", "a.example.com"],
            ),
        ];
        for (case, frontmatter, expected) in cases {
            let (_tmp, store) = store_with(&skill(&frontmatter), true);
            let current = super::status(&store, "arxiv").unwrap();
            assert_eq!(category(&current, "network").items, expected, "{case}");
        }
    }

    #[test]
    fn registry_capabilities_list_hosts_without_their_methods() {
        let rules = network(&[("a.example.com", "[GET]"), ("b.example.com", "[]")]);
        let (_tmp, store) = store_with(&skill(&rules), true);
        let mut listed = ManifestCapabilities::default();
        listed.include(requested(&store.agent("arxiv").unwrap()));
        assert_eq!(listed.network, ["a.example.com", "b.example.com"]);
    }

    #[test]
    fn a_folder_narrowed_to_read_only_keeps_its_approval() {
        let writable = skill("connectors:\n  - id: filesystem\n    folders: [docs]\n");
        let (_tmp, store) = store_with(&writable, true);
        let approved = approve_all(&store);
        assert_eq!(category(&approved, "folders").items, ["docs:write"]);
        update_skill(
            &store,
            &writable.replace("[docs]", "[{id: docs, access: read}]"),
        );
        assert!(super::status(&store, "arxiv").unwrap().satisfied);
    }

    #[test]
    fn grant_approves_only_what_was_shown() {
        let (_tmp, store) = store_with(SKILL, true);
        let shown_before = super::status(&store, "arxiv").unwrap();
        // An update lands while the dialog is open
        update_skill(
            &store,
            &SKILL.replace("[shell, read_file]", "[shell, read_file, write_file]"),
        );
        let keys = ["tools", "network"];
        let after = grant(&store, "arxiv", &shown(&shown_before, &keys)).unwrap();
        assert_eq!(ungranted(&after), ["tools"]);
        assert!(check(&store, "arxiv").is_err());
    }

    #[test]
    fn grant_refuses_an_unknown_category() {
        let (_tmp, store) = store_with(SKILL, true);
        let bad = ApprovedCategory {
            key: "everything".into(),
            items: vec!["*".into()],
        };
        let error = grant(&store, "arxiv", &[bad]).unwrap_err();
        assert!(error.contains("'everything' is not a capability category"));
    }

    #[test]
    fn own_agents_and_unknown_ids_never_need_it() {
        let (_tmp, store) = store_with(SKILL, false);
        let current = super::status(&store, "arxiv").unwrap();
        assert!(!current.required && current.satisfied);
        check(&store, "arxiv").unwrap();
        check(&store, "studio").unwrap();
        // No allowed-tools means every tool
        let (_tmp, store) = store_with("---\nname: A\n---\n", true);
        let current = super::status(&store, "arxiv").unwrap();
        assert_eq!(current.categories[0].items, ["*"]);
    }
}
