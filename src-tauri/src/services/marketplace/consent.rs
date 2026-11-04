//! First-use consent for marketplace agents.
//!
//! An installed (vendored) agent doesn't run until the user has approved,
//! category by category, everything its skill.md requests: tools, network
//! hosts, AWS services, pilots. What was approved is recorded in
//! `settings.yaml` (`consent.<agent>`), item by item, so a later version
//! that asks for more is blocked again until approved. The user's own
//! agents (created or forked) never need consent.
//!
//! The check runs where every run starts (`chat::submit`), so chat,
//! workflows, and triggers are all covered.

use serde::Serialize;
use specta::Type;

use crate::services::domain::settings::AgentConsent;
use crate::services::domain::{Agent, AgentId, EntityKind};
use crate::services::repo_store::RepoStore;
use crate::services::repos::SingletonRepository;

/// Every runtime tool (an agent without `allowed-tools`).
const ALL_TOOLS: &str = "*";

/// One category the user approves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConsentCategory {
    /// tools | network | aws | pilots
    pub key: String,
    /// What the agent requests (`*` in tools: every tool)
    pub items: Vec<String>,
    /// Already approved (all of `items`)
    pub granted: bool,
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

/// What an agent's skill.md requests.
pub fn requested(agent: &Agent) -> AgentConsent {
    let capabilities = agent.capabilities();
    AgentConsent {
        tools: capabilities
            .tools()
            .allowed()
            .map_or_else(|| vec![ALL_TOOLS.to_string()], <[String]>::to_vec),
        network: capabilities
            .network()
            .allow
            .iter()
            .map(|rule| rule.host.clone())
            .collect(),
        aws: capabilities
            .aws()
            .map(|aws| aws.services.iter().map(|s| s.service.clone()).collect())
            .unwrap_or_default(),
        pilots: capabilities
            .pilots()
            .iter()
            .map(|p| p.as_str().to_string())
            .collect(),
    }
}

fn covered(granted: &[String], requested: &[String]) -> bool {
    granted.iter().any(|g| g == ALL_TOOLS) || requested.iter().all(|r| granted.contains(r))
}

fn categories(requested: &AgentConsent, granted: &AgentConsent) -> Vec<ConsentCategory> {
    [
        ("tools", &requested.tools, &granted.tools),
        ("network", &requested.network, &granted.network),
        ("aws", &requested.aws, &granted.aws),
        ("pilots", &requested.pilots, &granted.pilots),
    ]
    .into_iter()
    .filter(|(_, items, _)| !items.is_empty())
    .map(|(key, items, granted)| ConsentCategory {
        key: key.to_string(),
        items: items.clone(),
        granted: covered(granted, items),
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

/// Approves `categories` (the current requests in them) for the agent.
pub fn grant(
    store: &RepoStore,
    agent_id: &str,
    categories: &[String],
) -> Result<ConsentStatus, String> {
    let agent = store.agent(agent_id)?;
    let requested = requested(&agent);
    let mut settings = store.read(|repo| repo.settings().clone());
    let mut consent = settings.consent(agent_id).cloned().unwrap_or_default();
    for category in categories {
        match category.as_str() {
            "tools" => consent.tools = requested.tools.clone(),
            "network" => consent.network = requested.network.clone(),
            "aws" => consent.aws = requested.aws.clone(),
            "pilots" => consent.pilots = requested.pilots.clone(),
            other => return Err(format!("'{other}' is not a capability category")),
        }
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

        let partial = grant(&store, "arxiv", &["tools".into()]).unwrap();
        assert!(!partial.satisfied);
        let all = grant(&store, "arxiv", &["network".into()]).unwrap();
        assert!(all.satisfied);
        check(&store, "arxiv").unwrap();

        // A new version asking for more is blocked again
        let skill = store
            .repos()
            .agents
            .locate("arxiv")
            .unwrap()
            .dir
            .join("skill.md");
        fs::write(
            &skill,
            SKILL.replace("export.arxiv.org", "evil.example.com"),
        )
        .unwrap();
        let current = super::status(&store, "arxiv").unwrap();
        assert!(!current.satisfied);
        assert!(current.categories[0].granted, "tools unchanged");
        assert!(!current.categories[1].granted);

        forget(&store, "arxiv").unwrap();
        assert!(store.read(|r| r.settings().consent("arxiv").is_none()));
        drop(tmp);
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
