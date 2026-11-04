//! The SQLite projection of the gold copy: a materialized, queryable view
//! of the repo files. Delete `state.db` and `rebuild_all` restores it.
//!
//! Only this module writes the projected tables (`agents`, `workflows`,
//! `triggers`, `settings`, `connectors`, and `sync_state`). Every change is
//! one transaction: a failed rebuild rolls back and leaves the previous
//! projection in place. Runtime tables (runs, fires, `trigger_state`, …)
//! are never touched.
//!
//! `sync_state` records the hash of the file(s) each entity was projected
//! from. At startup, entities whose files changed while the app was closed
//! (drift) are re-projected; a projection built by a different
//! `PROJECTION_VERSION` (or an empty one) is rebuilt from scratch.

use std::collections::HashMap;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::db::{self, Db};
use crate::services::domain::{AgentRepo, Entity, EntityKind};
use crate::services::egress::capabilities::Capabilities;
use crate::services::workflows::WorkflowDefinition;

/// Bump when the shape of projected rows changes: every install then
/// rebuilds its projection on the next start.
pub const PROJECTION_VERSION: i64 = 2;

/// The kinds projected (the manifest only versions the repo).
pub const PROJECTED_KINDS: [EntityKind; 5] = [
    EntityKind::Agent,
    EntityKind::Workflow,
    EntityKind::Trigger,
    EntityKind::Settings,
    EntityKind::Connectors,
];

/// What an entity was projected from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub kind: EntityKind,
    pub id: String,
    /// The entity's main file
    pub source_path: String,
    /// SHA-256 of its file(s)
    pub hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionStats {
    pub agents: u32,
    pub workflows: u32,
    pub triggers: u32,
    pub connectors: u32,
    /// Files that couldn't be loaded and were left out
    pub skipped: u32,
}

/// The port the repo store projects through. SQLite implements it; tests
/// can substitute their own.
pub trait ProjectionSink: Send + Sync {
    /// Replaces the whole projection with the aggregate's contents.
    fn rebuild_all(
        &self,
        repo: &AgentRepo,
        fingerprints: &[Fingerprint],
    ) -> Result<ProjectionStats, String>;
    /// Re-projects one entity: upserts it, or deletes it if the aggregate
    /// no longer has it (`fingerprint` is then `None`).
    fn project(
        &self,
        repo: &AgentRepo,
        kind: EntityKind,
        id: &str,
        fingerprint: Option<&Fingerprint>,
    ) -> Result<(), String>;
    /// The hashes in `sync_state`, by (kind, id).
    fn hashes(&self) -> Result<HashMap<(EntityKind, String), String>, String>;
    /// Whether the projection must be rebuilt from scratch: never built, or
    /// built by another projection version.
    fn needs_rebuild(&self) -> Result<bool, String>;
}

#[derive(Clone)]
pub struct SqliteProjection {
    db: Db,
}

impl SqliteProjection {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

impl ProjectionSink for SqliteProjection {
    fn rebuild_all(
        &self,
        repo: &AgentRepo,
        fingerprints: &[Fingerprint],
    ) -> Result<ProjectionStats, String> {
        let mut conn = self.db.conn()?;
        rebuild_all(&mut conn, repo, fingerprints)
            .map_err(|e| format!("Projection rebuild failed: {e}"))
    }

    fn project(
        &self,
        repo: &AgentRepo,
        kind: EntityKind,
        id: &str,
        fingerprint: Option<&Fingerprint>,
    ) -> Result<(), String> {
        let mut conn = self.db.conn()?;
        project_entity(&mut conn, repo, kind, id, fingerprint)
            .map_err(|e| format!("Failed to project {kind} '{id}': {e}"))
    }

    fn hashes(&self) -> Result<HashMap<(EntityKind, String), String>, String> {
        let conn = self.db.conn()?;
        hashes(&conn).map_err(|e| format!("Failed to read sync state: {e}"))
    }

    fn needs_rebuild(&self) -> Result<bool, String> {
        let conn = self.db.conn()?;
        needs_rebuild(&conn).map_err(|e| format!("Failed to read the projection: {e}"))
    }
}

// ---------------------------------------------------------------------------
// SQL
// ---------------------------------------------------------------------------

pub fn rebuild_all(
    conn: &mut Connection,
    repo: &AgentRepo,
    fingerprints: &[Fingerprint],
) -> rusqlite::Result<ProjectionStats> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "DELETE FROM agents;
         DELETE FROM workflows;
         DELETE FROM triggers;
         DELETE FROM settings;
         DELETE FROM connectors;",
    )?;
    for kind in PROJECTED_KINDS {
        tx.execute(
            "DELETE FROM sync_state WHERE entity_kind = ?1",
            [kind.as_str()],
        )?;
    }
    let paths: HashMap<(EntityKind, &str), &str> = fingerprints
        .iter()
        .map(|f| ((f.kind, f.id.as_str()), f.source_path.as_str()))
        .collect();
    let path = |kind, id: &str| paths.get(&(kind, id)).copied().unwrap_or_default();

    let mut stats = ProjectionStats::default();
    for agent in repo.agents() {
        let id = agent.id().as_str();
        upsert_row(
            &tx,
            &Entity::from(agent.clone()),
            path(EntityKind::Agent, id),
        )?;
        stats.agents += 1;
    }
    for workflow in repo.workflows() {
        let id = workflow.id().as_str();
        upsert_row(
            &tx,
            &Entity::Workflow(workflow.clone()),
            path(EntityKind::Workflow, id),
        )?;
        stats.workflows += 1;
    }
    for trigger in repo.triggers() {
        let id = trigger.id().as_str();
        upsert_row(
            &tx,
            &Entity::Trigger(trigger.clone()),
            path(EntityKind::Trigger, id),
        )?;
        stats.triggers += 1;
    }
    upsert_row(
        &tx,
        &Entity::Settings(repo.settings().clone()),
        path(EntityKind::Settings, "settings"),
    )?;
    upsert_row(
        &tx,
        &Entity::Connectors(repo.connectors().clone()),
        path(EntityKind::Connectors, "connectors"),
    )?;
    stats.connectors = repo.connectors().connectors().len() as u32;
    for fingerprint in fingerprints
        .iter()
        .filter(|f| PROJECTED_KINDS.contains(&f.kind))
    {
        set_sync(&tx, fingerprint)?;
    }
    stats.skipped = repo
        .problems()
        .iter()
        .filter(|p| p.severity == crate::services::domain::repo::Severity::Error)
        .count() as u32;
    tx.commit()?;
    Ok(stats)
}

pub fn project_entity(
    conn: &mut Connection,
    repo: &AgentRepo,
    kind: EntityKind,
    id: &str,
    fingerprint: Option<&Fingerprint>,
) -> rusqlite::Result<()> {
    if !PROJECTED_KINDS.contains(&kind) {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match (repo.get(kind, id), fingerprint) {
        (Some(entity), Some(fingerprint)) => {
            upsert_row(&tx, &entity, &fingerprint.source_path)?;
            set_sync(&tx, fingerprint)?;
        }
        _ => {
            delete_row(&tx, kind, id)?;
            tx.execute(
                "DELETE FROM sync_state WHERE entity_kind = ?1 AND entity_id = ?2",
                params![kind.as_str(), id],
            )?;
        }
    }
    tx.commit()
}

fn set_sync(conn: &Connection, fingerprint: &Fingerprint) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_state (entity_kind, entity_id, source_path, file_hash, last_projected_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(entity_kind, entity_id) DO UPDATE SET
             source_path = excluded.source_path, file_hash = excluded.file_hash,
             last_projected_at = excluded.last_projected_at",
        params![
            fingerprint.kind.as_str(),
            fingerprint.id,
            fingerprint.source_path,
            fingerprint.hash,
            db::now_ms()
        ],
    )?;
    Ok(())
}

fn ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn upsert_row(conn: &Connection, entity: &Entity, source_path: &str) -> rusqlite::Result<()> {
    match entity {
        Entity::Agent(agent) => {
            let tools = agent.capabilities().tools();
            let definition = json!({
                "allowedTools": tools.allowed(),
                "requiresApproval": tools.approval_list(),
                "pilots": agent.capabilities().pilots().iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                "memoryMb": agent.container().memory_mb(),
                "cpu": agent.container().cpu(),
                "workflows": agent.workflows().iter().map(|w| w.as_str()).collect::<Vec<_>>(),
            });
            let capabilities = serde_json::to_value(Capabilities::from(agent.capabilities()))
                .unwrap_or(Value::Null);
            conn.execute(
                "INSERT OR REPLACE INTO agents (id, name, description, model, image, capabilities,
                     definition, created_at, updated_at, source_path, projection_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    agent.id().as_str(),
                    agent.name(),
                    agent.description(),
                    agent.model().map(ToString::to_string),
                    agent.container().image(),
                    capabilities.to_string(),
                    definition.to_string(),
                    ms(agent.created_at()),
                    ms(agent.updated_at()),
                    source_path,
                    PROJECTION_VERSION,
                ],
            )?;
        }
        Entity::Workflow(workflow) => {
            let definition = serde_json::to_string(&WorkflowDefinition::from(workflow))
                .unwrap_or_else(|_| "{}".to_string());
            let agent_id = match workflow.scope() {
                crate::services::domain::workflow::WorkflowScope::Agent(agent) => {
                    Some(agent.as_str())
                }
                crate::services::domain::workflow::WorkflowScope::Global => None,
            };
            conn.execute(
                "INSERT OR REPLACE INTO workflows (id, name, definition, created_at, agent_id,
                     updated_at, source_path, projection_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    workflow.id().as_str(),
                    workflow.name(),
                    definition,
                    ms(workflow.created_at()),
                    agent_id,
                    ms(workflow.updated_at()),
                    source_path,
                    PROJECTION_VERSION,
                ],
            )?;
        }
        Entity::Settings(settings) => {
            conn.execute("DELETE FROM settings", [])?;
            let values: Value = serde_yaml::from_str(&settings.to_yaml()).unwrap_or(Value::Null);
            if let Value::Object(map) = values {
                for (key, value) in map {
                    conn.execute(
                        "INSERT INTO settings (key, value, source_path, projection_version)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![key, value.to_string(), source_path, PROJECTION_VERSION],
                    )?;
                }
            }
        }
        Entity::Connectors(manifest) => {
            conn.execute("DELETE FROM connectors", [])?;
            for connector in manifest.connectors() {
                let definition = json!({
                    "config": serde_json::to_value(&connector.config).unwrap_or(Value::Null),
                    "secretRefs": serde_json::to_value(&connector.secret_refs).unwrap_or(Value::Null),
                });
                conn.execute(
                    "INSERT INTO connectors (id, kind, definition, source_path, projection_version)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        connector.id.as_str(),
                        connector.kind,
                        definition.to_string(),
                        source_path,
                        PROJECTION_VERSION
                    ],
                )?;
            }
        }
        Entity::Trigger(trigger) => {
            let now = db::now_ms();
            conn.execute(
                "INSERT INTO triggers (id, name, enabled, source_type, source_config, target_type,
                     target_id, input_template, priority, rate_limit_seconds, created_at,
                     updated_at, source_path, projection_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12, ?13)
                 ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name, enabled = excluded.enabled,
                     source_type = excluded.source_type, source_config = excluded.source_config,
                     target_type = excluded.target_type, target_id = excluded.target_id,
                     input_template = excluded.input_template, priority = excluded.priority,
                     rate_limit_seconds = excluded.rate_limit_seconds,
                     updated_at = excluded.updated_at, source_path = excluded.source_path,
                     projection_version = excluded.projection_version",
                params![
                    trigger.id().as_str(),
                    trigger.name(),
                    trigger.enabled(),
                    trigger.source().type_name(),
                    trigger.source().config_json().to_string(),
                    trigger.target().type_name(),
                    trigger.target().id(),
                    trigger.input_template(),
                    trigger.priority(),
                    trigger.rate_limit_seconds(),
                    now,
                    source_path,
                    PROJECTION_VERSION,
                ],
            )?;
        }
        Entity::Manifest(_) => {}
    }
    Ok(())
}

fn delete_row(conn: &Connection, kind: EntityKind, id: &str) -> rusqlite::Result<()> {
    match kind {
        EntityKind::Agent => conn.execute("DELETE FROM agents WHERE id = ?1", [id])?,
        EntityKind::Workflow => conn.execute("DELETE FROM workflows WHERE id = ?1", [id])?,
        EntityKind::Settings => conn.execute("DELETE FROM settings", [])?,
        EntityKind::Connectors => conn.execute("DELETE FROM connectors", [])?,
        EntityKind::Trigger => conn.execute("DELETE FROM triggers WHERE id = ?1", [id])?,
        EntityKind::Manifest => 0,
    };
    Ok(())
}

fn hashes(conn: &Connection) -> rusqlite::Result<HashMap<(EntityKind, String), String>> {
    let mut stmt = conn.prepare("SELECT entity_kind, entity_id, file_hash FROM sync_state")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (kind, id, hash) = row?;
        if let Some(kind) = EntityKind::parse(&kind) {
            out.insert((kind, id), hash);
        }
    }
    Ok(out)
}

fn needs_rebuild(conn: &Connection) -> rusqlite::Result<bool> {
    let synced: bool = conn
        .prepare("SELECT 1 FROM sync_state WHERE entity_kind = 'settings'")?
        .exists([])?;
    if !synced {
        return Ok(true);
    }
    for table in ["agents", "workflows", "triggers", "settings", "connectors"] {
        let stale: bool = conn
            .prepare(&format!(
                "SELECT 1 FROM {table} WHERE projection_version IS NOT ?1 LIMIT 1"
            ))?
            .exists([PROJECTION_VERSION])?;
        if stale {
            return Ok(true);
        }
    }
    Ok(false)
}

/// An agent's projected egress capabilities (the Permissions panel).
pub fn agent_capabilities(conn: &Connection, agent_id: &str) -> rusqlite::Result<Option<Value>> {
    use rusqlite::OptionalExtension;
    let text: Option<String> = conn
        .query_row(
            "SELECT capabilities FROM agents WHERE id = ?1",
            [agent_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::repo::RepoSnapshot;
    use crate::services::domain::{Agent, AgentId, Settings, Workflow};

    fn repo() -> AgentRepo {
        AgentRepo::assemble(
            "/repo",
            RepoSnapshot {
                agents: vec![Agent::from_skill_md(
                    AgentId::parse("writer").unwrap(),
                    "---\nname: Writer\nnetwork:\n  allow:\n    - host: x.com\n---\nWrite.",
                )
                .unwrap()],
                workflows: vec![Workflow::from_yaml(
                    "id: daily\nname: Daily\nsteps:\n  - {id: a, agent: writer, input: hi}\n",
                )
                .unwrap()],
                settings: Some(Settings::from_yaml("theme: dark").unwrap()),
                ..Default::default()
            },
        )
    }

    fn fingerprints() -> Vec<Fingerprint> {
        [
            (EntityKind::Agent, "writer"),
            (EntityKind::Workflow, "daily"),
            (EntityKind::Settings, "settings"),
            (EntityKind::Connectors, "connectors"),
        ]
        .into_iter()
        .map(|(kind, id)| Fingerprint {
            kind,
            id: id.into(),
            source_path: format!("/repo/{id}"),
            hash: format!("h-{id}"),
        })
        .collect()
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn rebuilds_everything_in_one_transaction() {
        let db = Db::in_memory();
        let mut conn = db.conn().unwrap();
        assert!(needs_rebuild(&conn).unwrap());
        let stats = rebuild_all(&mut conn, &repo(), &fingerprints()).unwrap();
        assert_eq!((stats.agents, stats.workflows), (1, 1));
        assert!(!needs_rebuild(&conn).unwrap());
        assert_eq!(count(&conn, "agents"), 1);
        assert_eq!(db::list_workflows(&conn).unwrap()[0].id, "daily");
        let theme: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'theme'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(theme, "\"dark\"");
        let caps = agent_capabilities(&conn, "writer").unwrap().unwrap();
        assert_eq!(caps["network"]["allow"][0]["host"], "x.com");
        assert_eq!(
            hashes(&conn).unwrap()[&(EntityKind::Agent, "writer".into())],
            "h-writer"
        );
        // Runtime settings are untouched
        assert!(db::get_setting(&conn, crate::engine::CANNED_SCRIPT_KEY)
            .unwrap()
            .is_some());

        // Rebuilding again replaces, never duplicates
        rebuild_all(&mut conn, &repo(), &fingerprints()).unwrap();
        assert_eq!(count(&conn, "agents"), 1);
    }

    #[test]
    fn projects_and_removes_single_entities() {
        let db = Db::in_memory();
        let mut conn = db.conn().unwrap();
        let mut repo = repo();
        rebuild_all(&mut conn, &repo, &fingerprints()).unwrap();
        repo.remove(EntityKind::Agent, "writer");
        project_entity(&mut conn, &repo, EntityKind::Agent, "writer", None).unwrap();
        assert_eq!(count(&conn, "agents"), 0);
        assert!(!hashes(&conn)
            .unwrap()
            .contains_key(&(EntityKind::Agent, "writer".into())));
    }

    #[test]
    fn a_stale_projection_version_forces_a_rebuild() {
        let db = Db::in_memory();
        let mut conn = db.conn().unwrap();
        rebuild_all(&mut conn, &repo(), &fingerprints()).unwrap();
        conn.execute("UPDATE agents SET projection_version = 0", [])
            .unwrap();
        assert!(needs_rebuild(&conn).unwrap());
    }
}
