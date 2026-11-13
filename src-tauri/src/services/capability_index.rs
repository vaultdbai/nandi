//! The local capability index: which agent cards serve which capability.
//! A projection of the agents' `serves:` declarations
//! into the `capability_index` table, kept current from repo events.
//!
//! - `init` rebuilds it from the aggregate at startup and subscribes it to
//!   the repo store, so an agent created, edited, or deleted re-indexes
//!   only that agent.
//! - One row per (agent, capability); `agent_card` holds the card as JSON.
//!   Like the rest of SQLite, it is rebuildable from the gold copy.

use std::sync::{Arc, RwLock, RwLockReadGuard};

use rusqlite::params;

use super::domain::agent_card::{card_from_agent, OwnerRef};
use super::domain::{Agent, AgentCard, ChangeAction, ChangedBy, DomainEvent, EntityKind, Problem};
use super::network;
use super::repo_store::{EventPublisher, RepoStore};
use super::AppState;
use crate::db::{now_ms, Db};

pub struct CapabilityIndex {
    db: Db,
    instance_id: String,
    /// For card endpoints (`network::agent_endpoint`); empty without a relay
    relay_url: String,
    /// Who runs this instance, stamped on every card
    owner: OwnerRef,
}

impl CapabilityIndex {
    pub fn new(db: Db, instance_id: impl Into<String>, relay_url: impl Into<String>) -> Self {
        Self {
            db,
            instance_id: instance_id.into(),
            relay_url: relay_url.into(),
            owner: OwnerRef::default(),
        }
    }

    #[cfg(test)]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    pub fn owner(&self) -> &OwnerRef {
        &self.owner
    }

    /// A new owner (onboarding wrote NANDI.md, or it was edited) for the
    /// cards of `agents`; says whether it changed
    pub fn set_owner(&mut self, owner: OwnerRef, agents: &[Agent]) -> Result<bool, String> {
        if self.owner == owner {
            return Ok(false);
        }
        self.owner = owner;
        self.rebuild_from_local(agents)?;
        Ok(true)
    }

    /// The card for a local agent, served or not: endpoint from the relay
    /// URL (empty without one), this instance's owner, issued now.
    pub fn card_for(&self, agent: &Agent) -> AgentCard {
        let endpoint = if self.relay_url.trim().is_empty() {
            String::new()
        } else {
            network::agent_endpoint(&self.relay_url, &self.instance_id, agent.id().as_str())
        };
        card_from_agent(agent, &self.instance_id, &endpoint)
            .with_owner(self.owner.clone())
            .with_issued_at(now_ms() as f64)
    }

    /// Replaces the index with the cards of `agents` that serve something.
    pub fn rebuild_from_local(&mut self, agents: &[Agent]) -> Result<(), String> {
        let rows = agents
            .iter()
            .map(|agent| self.rows_for(agent))
            .collect::<Result<Vec<_>, _>>()?;
        let mut conn = self.db.conn()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute("DELETE FROM capability_index", [])
            .map_err(db_error)?;
        for row in rows.iter().flatten() {
            insert(&tx, row)?;
        }
        tx.commit().map_err(db_error)
    }

    /// Re-indexes one agent: replaces its rows with what it serves now.
    pub fn upsert_agent(&self, agent: &Agent) -> Result<(), String> {
        let rows = self.rows_for(agent)?;
        let mut conn = self.db.conn()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute(
            "DELETE FROM capability_index WHERE entity_id = ?1",
            [agent.id().as_str()],
        )
        .map_err(db_error)?;
        for row in &rows {
            insert(&tx, row)?;
        }
        tx.commit().map_err(db_error)
    }

    /// Drops one agent's rows.
    pub fn remove_agent(&self, agent_id: &str) -> Result<(), String> {
        self.db
            .conn()?
            .execute(
                "DELETE FROM capability_index WHERE entity_id = ?1",
                [agent_id],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Local cards serving `capability`
    pub fn find_local(&self, capability: &str) -> Vec<AgentCard> {
        self.query_cards(
            "SELECT agent_card FROM capability_index WHERE capability = ?1 ORDER BY entity_id",
            capability,
        )
        .unwrap_or_else(|e| {
            log::warn!("Capability lookup for '{capability}' failed: {e}");
            Vec::new()
        })
    }

    /// The indexed card of a local agent, if it serves anything
    #[cfg(test)]
    pub fn indexed_card(&self, agent_id: &str) -> Option<AgentCard> {
        self.query_cards(
            "SELECT agent_card FROM capability_index WHERE entity_id = ?1 LIMIT 1",
            agent_id,
        )
        .ok()
        .and_then(|cards| cards.into_iter().next())
    }

    /// Local and remote cards serving `capability`, deduplicated by card ID
    /// (local first)
    #[cfg(test)]
    pub fn find_with_remote(&self, capability: &str, remote_cards: &[AgentCard]) -> Vec<AgentCard> {
        let mut cards = self.find_local(capability);
        let mut seen: std::collections::HashSet<String> =
            cards.iter().map(|c| c.id.clone()).collect();
        for card in remote_cards {
            if card.serves(capability) && seen.insert(card.id.clone()) {
                cards.push(card.clone());
            }
        }
        cards
    }

    /// Every capability a local agent serves, sorted and distinct
    pub fn list_capabilities(&self) -> Vec<String> {
        self.query_names().unwrap_or_else(|e| {
            log::warn!("Listing capabilities failed: {e}");
            Vec::new()
        })
    }

    fn query_names(&self) -> Result<Vec<String>, String> {
        let conn = self.db.conn()?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT capability FROM capability_index ORDER BY capability")
            .map_err(db_error)?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(names)
    }

    fn rows_for(&self, agent: &Agent) -> Result<Vec<Row>, String> {
        let capabilities = agent.capabilities().serves();
        if capabilities.is_empty() {
            return Ok(Vec::new());
        }
        let card = self.card_for(agent);
        let json =
            serde_json::to_string(&card).map_err(|e| format!("Failed to encode card: {e}"))?;
        Ok(capabilities
            .iter()
            .map(|c| Row {
                entity_id: agent.id().to_string(),
                capability: c.name.clone(),
                card: json.clone(),
            })
            .collect())
    }

    fn query_cards(&self, sql: &str, arg: &str) -> Result<Vec<AgentCard>, String> {
        let conn = self.db.conn()?;
        let mut stmt = conn.prepare(sql).map_err(db_error)?;
        let rows = stmt
            .query_map([arg], |row| row.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(rows
            .iter()
            .filter_map(|json| match serde_json::from_str(json) {
                Ok(card) => Some(card),
                Err(e) => {
                    log::warn!("Skipping an unreadable agent card: {e}");
                    None
                }
            })
            .collect())
    }
}

struct Row {
    entity_id: String,
    capability: String,
    card: String,
}

fn insert(conn: &rusqlite::Connection, row: &Row) -> Result<(), String> {
    conn.execute(
        "INSERT INTO capability_index (entity_id, capability, agent_card, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(entity_id, capability) DO UPDATE SET
            agent_card = excluded.agent_card, updated_at = excluded.updated_at",
        params![row.entity_id, row.capability, row.card, now_ms()],
    )
    .map_err(db_error)?;
    Ok(())
}

fn db_error(e: rusqlite::Error) -> String {
    format!("Database error: {e}")
}

/// Keeps the index in step with agent changes, one agent at a time.
struct Reindexer {
    index: Arc<RwLock<CapabilityIndex>>,
    store: RepoStore,
}

impl EventPublisher for Reindexer {
    fn changed(&self, event: &DomainEvent) {
        if event.kind != EntityKind::Agent {
            return;
        }
        let agent = match event.action {
            ChangeAction::Deleted => None,
            ChangeAction::Created | ChangeAction::Updated => {
                self.store.read(|repo| repo.agent(&event.id).cloned())
            }
        };
        let index = self.index.read().unwrap_or_else(|e| e.into_inner());
        let result = match agent {
            Some(agent) => index.upsert_agent(&agent),
            None => index.remove_agent(&event.id),
        };
        if let Err(e) = result {
            log::warn!("Failed to re-index agent '{}': {e}", event.id);
        }
    }

    /// The last good version stays indexed, as it stays running
    fn invalid(&self, _problem: &Problem, _changed_by: ChangedBy) {}
}

/// Builds the index from the loaded agents and subscribes it to agent
/// changes. Local only: never touches the network.
pub fn init(state: &AppState) -> Option<Arc<RwLock<CapabilityIndex>>> {
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Capability index disabled: no instance ID ({e})");
            return None;
        }
    };
    let relay_url = super::relay::configured_url(state).unwrap_or_default();
    let mut index = CapabilityIndex::new(state.db().clone(), instance_id, relay_url);
    let agents: Vec<Agent> = state.store().read(|repo| repo.agents().cloned().collect());
    if let Err(e) = index.rebuild_from_local(&agents) {
        log::error!("Failed to build the capability index: {e}");
    }
    let index = Arc::new(RwLock::new(index));
    state.store().subscribe(Arc::new(Reindexer {
        index: index.clone(),
        store: state.store().clone(),
    }));
    Some(index)
}

fn read_index(state: &AppState) -> Result<RwLockReadGuard<'_, CapabilityIndex>, String> {
    let index = state
        .capability_index()
        .ok_or_else(|| "The capability index is unavailable".to_string())?;
    Ok(index.read().unwrap_or_else(|e| e.into_inner()))
}

/// Local agents serving `capability`
pub async fn find_agents_by_capability(
    state: &AppState,
    capability: &str,
) -> Result<Vec<AgentCard>, String> {
    let capability = capability.trim();
    if capability.is_empty() {
        return Err("Capability is required".to_string());
    }
    Ok(read_index(state)?.find_local(capability))
}

/// Every capability a local agent serves, sorted and distinct
pub async fn list_declared_capabilities(state: &AppState) -> Result<Vec<String>, String> {
    Ok(read_index(state)?.list_capabilities())
}

/// The card of a local agent (whether or not it serves anything)
pub async fn get_agent_card(state: &AppState, agent_id: &str) -> Result<AgentCard, String> {
    let agent = state.store().agent(agent_id)?;
    Ok(read_index(state)?.card_for(&agent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agents::{self, CreateAgentInput};
    use crate::services::domain::AgentId;
    use crate::services::repos::Repository;

    fn agent(id: &str, frontmatter: &str) -> Agent {
        Agent::from_skill_md(
            AgentId::parse(id).unwrap(),
            &format!("---\nname: {id}\n{frontmatter}---\nBody\n"),
        )
        .unwrap()
    }

    fn papers() -> Agent {
        agent(
            "papers",
            "serves:\n  - name: find-papers\n  - name: summarize-paper\naccepts_delegation: true\n",
        )
    }

    #[test]
    fn parses_serves_from_frontmatter() {
        let agent = agent(
            "r",
            "serves: [{name: find-papers, description: Finds papers}]\nmax_delegation_concurrent: 3\n",
        );
        let serves = agent.capabilities().serves();
        assert_eq!(serves.len(), 1);
        assert_eq!(serves[0].name, "find-papers");
        assert_eq!(serves[0].description, "Finds papers");
        assert!(!agent.capabilities().accepts_delegation());
        assert_eq!(agent.capabilities().delegation_limit(), 3);
        assert!(Agent::from_skill_md(
            AgentId::parse("bad").unwrap(),
            "---\nserves: [{name: has space}]\n---\n"
        )
        .is_err());
    }

    #[test]
    fn looks_up_cards_by_capability() {
        let mut index = CapabilityIndex::new(Db::in_memory(), "inst", "wss://relay.example/relay");
        let writer = agent("writer", "serves: [{name: summarize-paper}]\n");
        let plain = agent("plain", "");
        index
            .rebuild_from_local(&[papers(), writer, plain])
            .unwrap();

        let found = index.find_local("summarize-paper");
        let ids: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["inst/papers", "inst/writer"]);
        assert_eq!(found[0].endpoint, "wss://relay.example/a/inst/papers");
        assert!(found[0].accepts_delegation);
        assert_eq!(index.find_local("find-papers").len(), 1);
        assert!(index.find_local("nothing").is_empty());
        assert_eq!(
            index.list_capabilities(),
            ["find-papers", "summarize-paper"]
        );
        assert!(index.indexed_card("plain").is_none());
        assert_eq!(index.indexed_card("writer").unwrap().name, "writer");

        // A rebuild replaces everything
        index.rebuild_from_local(&[]).unwrap();
        assert!(index.list_capabilities().is_empty());
    }

    #[test]
    fn upserts_and_removes_one_agent() {
        let mut index = CapabilityIndex::new(Db::in_memory(), "inst", "");
        index.rebuild_from_local(&[papers()]).unwrap();
        index
            .upsert_agent(&agent("papers", "serves: [{name: find-papers}]\n"))
            .unwrap();
        assert!(index.find_local("summarize-paper").is_empty());
        assert_eq!(index.find_local("find-papers")[0].endpoint, "");
        index.remove_agent("papers").unwrap();
        assert!(index.list_capabilities().is_empty());
    }

    #[test]
    fn merges_remote_cards_local_first() {
        let mut index = CapabilityIndex::new(Db::in_memory(), "inst", "");
        index.rebuild_from_local(&[papers()]).unwrap();
        let local = index.find_local("find-papers").remove(0);
        let remote = card_from_agent(&papers(), "other", "wss://r/a/other/papers");
        let unrelated = card_from_agent(&agent("x", "serves: [{name: x}]\n"), "other", "");
        let mut duplicate = local.clone();
        duplicate.name = "stale copy".to_string();

        let merged = index.find_with_remote("find-papers", &[duplicate, remote, unrelated]);
        let ids: Vec<&str> = merged.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["inst/papers", "other/papers"]);
        assert_eq!(merged[0], local);
    }

    fn input(id: &str) -> CreateAgentInput {
        CreateAgentInput {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            model: None,
            instructions: None,
        }
    }

    fn save(state: &AppState, agent: &Agent) {
        let store = state.store();
        store.repos().agents.save(agent).unwrap();
        store
            .commit(EntityKind::Agent, agent.id().as_str())
            .unwrap();
    }

    fn updated_at(state: &AppState, agent_id: &str) -> i64 {
        state
            .db()
            .conn()
            .unwrap()
            .query_row(
                "SELECT updated_at FROM capability_index WHERE entity_id = ?1 LIMIT 1",
                [agent_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn follows_agents_as_they_are_created_edited_and_deleted() {
        let state = AppState::for_tests(vec![]);
        assert!(state.capability_index().is_some());
        assert!(list_declared_capabilities(&state).await.unwrap().is_empty());

        // Created without serves, then edited to serve
        agents::create_agent(&state, input("papers")).await.unwrap();
        assert!(find_agents_by_capability(&state, "find-papers")
            .await
            .unwrap()
            .is_empty());
        save(&state, &papers());
        let found = find_agents_by_capability(&state, "find-papers")
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].agent_id(), "papers");
        assert_eq!(
            list_declared_capabilities(&state).await.unwrap(),
            ["find-papers", "summarize-paper"]
        );
        let card = get_agent_card(&state, "papers").await.unwrap();
        assert_eq!(card.id, found[0].id);
        assert!(card.serves("summarize-paper"));

        // Edited to drop a capability
        save(&state, &agent("papers", "serves: [{name: find-papers}]\n"));
        assert!(find_agents_by_capability(&state, "summarize-paper")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            list_declared_capabilities(&state).await.unwrap(),
            ["find-papers"]
        );

        // Deleted
        agents::delete_agent(&state, "papers").await.unwrap();
        assert!(find_agents_by_capability(&state, "find-papers")
            .await
            .unwrap()
            .is_empty());
        assert!(list_declared_capabilities(&state).await.unwrap().is_empty());
        assert!(get_agent_card(&state, "papers").await.is_err());
        assert!(find_agents_by_capability(&state, " ").await.is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn only_the_changed_agent_is_reindexed() {
        let state = AppState::for_tests(vec![]);
        save(&state, &papers());
        save(
            &state,
            &agent("writer", "serves: [{name: summarize-paper}]\n"),
        );
        assert_eq!(
            find_agents_by_capability(&state, "summarize-paper")
                .await
                .unwrap()
                .len(),
            2
        );
        let before = updated_at(&state, "papers");
        std::thread::sleep(std::time::Duration::from_millis(5));
        save(&state, &agent("writer", "serves: [{name: other}]\n"));
        assert_eq!(updated_at(&state, "papers"), before);
        assert_eq!(
            list_declared_capabilities(&state).await.unwrap(),
            ["find-papers", "other", "summarize-paper"]
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn init_indexes_agents_already_on_disk() {
        let state = AppState::for_tests(vec![]);
        save(&state, &papers());
        // A second index over the same state rebuilds from the aggregate
        let index = init(&state).unwrap();
        let index = index.read().unwrap();
        assert_eq!(index.find_local("find-papers").len(), 1);
        assert_eq!(
            index.instance_id(),
            network::local_instance_id(state.db()).unwrap()
        );
        assert_eq!(index.relay_url(), "");
        drop(index);
        state.shutdown();
    }
}
