//! The unified tool catalog (wave A, Track C): built-in tools, connector
//! tools, and pilot tools as first-class entities in `tool_catalog`.
//! Capabilities declare `required_tools`; providers advertise the tools
//! they have; selection filters by coverage.
//!
//! On any source change (connector installed, pilot started) the catalog
//! is rebuilt and `tool://catalog_changed` emitted.
//!
//! - `tool_catalog` is a projection of the sources, except `enabled`,
//!   which is the user's and survives rebuilds.
//! - A rebuild drops the tools of sources that are gone (an uninstalled
//!   connector), but keeps those of sources that can't list their tools
//!   right now (a stopped pilot), so capabilities don't flap.
//! - The index is rebuilt from the table after every write.

pub mod builtin;
pub mod catalog;
pub mod connectors;
pub mod definition;
pub mod pilots;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::Serialize;

pub use catalog::ToolCatalog;
pub use definition::{ToolEntry, ToolUsageStats};

use crate::db::{now_ms, Db};
use crate::services::domain::{ChangedBy, DomainEvent, EntityKind, Problem};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::repo_store::EventPublisher;
use crate::services::{AppState, WeakAppState};
use definition::{ToolDefinition, ToolSource};

pub const TOOL_CATALOG_CHANGED: &str = "tool://catalog_changed";
pub const TOOL_USAGE_RECORDED: &str = "tool://usage_recorded";

/// How often the background task checks whether connectors or pilots
/// started or stopped (a cheap fingerprint; no keyring, no network)
const POLL_INTERVAL: Duration = Duration::from_secs(10);
/// The window `ToolEntry::usage_count` covers
const USAGE_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The catalog changed (`tool://catalog_changed`); refetch it
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogChanged {
    pub tool_count: u32,
}

impl NandiEvent for ToolCatalogChanged {
    const NAME: &'static str = TOOL_CATALOG_CHANGED;
}

pub struct ToolService {
    db: Db,
    bus: EventBus,
    index: RwLock<ToolCatalog>,
    /// For the connector and pilot sources; `None` in tests
    state: Option<WeakAppState>,
    /// One rebuild at a time
    rebuilding: tokio::sync::Mutex<()>,
}

impl ToolService {
    /// Loads the persisted catalog and brings the built-ins up to date.
    /// Connector and pilot rows stay as persisted until the first rebuild.
    pub fn new(db: Db, bus: EventBus, state: Option<WeakAppState>) -> Result<Self, String> {
        let entries = {
            let mut conn = db.conn()?;
            let tx = conn.transaction().map_err(db_error)?;
            apply(&tx, builtin::builtin_tools(), |source| {
                *source != ToolSource::Builtin
            })
            .map_err(db_error)?;
            let entries = load_entries(&tx).map_err(db_error)?;
            tx.commit().map_err(db_error)?;
            entries
        };
        Ok(Self {
            db,
            bus,
            index: RwLock::new(ToolCatalog::from_entries(entries)),
            state,
            rebuilding: tokio::sync::Mutex::new(()),
        })
    }

    /// Sorted by ID, with 30-day usage
    pub async fn list_all(&self) -> Result<Vec<ToolEntry>, String> {
        let usage = self.usage_by_id().await?;
        Ok(self
            .catalog()
            .all()
            .into_iter()
            .map(|entry| with_usage(entry.clone(), &usage))
            .collect())
    }

    /// By ID, bare built-in name, or runtime alias
    pub async fn get(&self, id: &str) -> Result<ToolEntry, String> {
        let entry = self.catalog().get(id).cloned().ok_or_else(|| unknown(id))?;
        let usage = self.usage_by_id().await?;
        Ok(with_usage(entry, &usage))
    }

    /// Disabled tools stay in the catalog (and count as present for
    /// `required_tools`)
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<ToolEntry, String> {
        let canonical = self
            .catalog()
            .get(id)
            .map(|e| e.definition.id.clone())
            .ok_or_else(|| unknown(id))?;
        let key = canonical.clone();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE tool_catalog SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
                    params![key, enabled, now_ms()],
                )
            })
            .await?;
        self.reload().await?;
        self.emit_changed();
        self.get(&canonical).await
    }

    /// Rebuilds from every source, emits `tool://catalog_changed` if
    /// anything changed
    pub async fn rebuild(&self) -> Result<(), String> {
        let _guard = self.rebuilding.lock().await;
        let mut definitions = builtin::builtin_tools();
        // Sources whose current tools are unknown keep their rows
        let mut keep_connectors = false;
        let mut keep_pilots = HashSet::new();
        if let Some(state) = self.state.as_ref().and_then(WeakAppState::upgrade) {
            match crate::services::connectors::list_connectors(&state).await {
                Ok(infos) => definitions.extend(connectors::connector_definitions(&infos)),
                Err(e) => {
                    log::warn!("Tool catalog: connectors unavailable ({e})");
                    keep_connectors = true;
                }
            }
            let scan = pilots::scan(&state).await;
            definitions.extend(scan.tools);
            keep_pilots.extend(scan.unlisted);
        }
        self.rebuild_from(definitions, move |source| match source {
            ToolSource::Connector { .. } => keep_connectors,
            ToolSource::Pilot { pilot_id } => keep_pilots.contains(pilot_id),
            // Never listed by a source yet: user-managed
            ToolSource::Custom { .. } => true,
            ToolSource::Builtin => false,
        })
        .await
        .map(|_| ())
    }

    /// Replaces the catalog with `definitions`, keeping the rows of the
    /// sources `keep` accepts. Whether anything changed.
    pub(crate) async fn rebuild_from(
        &self,
        definitions: Vec<ToolDefinition>,
        keep: impl Fn(&ToolSource) -> bool + Send + 'static,
    ) -> Result<bool, String> {
        let changed = self
            .db
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let changed = apply(&tx, definitions, keep)?;
                tx.commit()?;
                Ok(changed)
            })
            .await?;
        if changed {
            self.reload().await?;
            self.emit_changed();
        }
        Ok(changed)
    }

    /// A snapshot of the current index
    pub fn catalog(&self) -> ToolCatalog {
        self.index.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The entries of `required` the catalog doesn't know (a capability
    /// loads anyway: the tool may be installed later)
    pub fn missing_tools(&self, required: &[String]) -> Vec<String> {
        self.catalog()
            .missing(required)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Calls per catalog tool in the last 30 days, from the runs' tool-call
    /// messages. Runtime names resolve to catalog IDs; names the catalog
    /// doesn't know are reported as they are.
    pub async fn usage_stats(&self) -> Result<Vec<ToolUsageStats>, String> {
        let since = now_ms() - USAGE_WINDOW_MS;
        let rows = self.db.call(move |conn| usage_rows(conn, since)).await?;
        let catalog = self.catalog();
        let mut by_id: BTreeMap<String, ToolUsageStats> = BTreeMap::new();
        for row in rows {
            let id = catalog
                .get(&row.tool)
                .map_or(row.tool.clone(), |e| e.definition.id.clone());
            let stats = by_id.entry(id.clone()).or_insert_with(|| ToolUsageStats {
                tool_id: id,
                calls: 0,
                failures: 0,
                last_used_at: None,
            });
            stats.calls += row.calls;
            stats.failures += row.failures;
            let last = row.last_used_at as f64;
            stats.last_used_at = Some(stats.last_used_at.map_or(last, |t| t.max(last)));
        }
        Ok(by_id.into_values().collect())
    }

    async fn usage_by_id(&self) -> Result<HashMap<String, u32>, String> {
        Ok(self
            .usage_stats()
            .await?
            .into_iter()
            .map(|s| (s.tool_id, s.calls))
            .collect())
    }

    async fn reload(&self) -> Result<(), String> {
        let entries = self.db.call(|conn| load_entries(conn)).await?;
        *self.index.write().unwrap_or_else(|e| e.into_inner()) = ToolCatalog::from_entries(entries);
        Ok(())
    }

    fn emit_changed(&self) {
        let tool_count = self.catalog().len() as u32;
        events::emit(&self.bus, &ToolCatalogChanged { tool_count });
    }
}

fn with_usage(mut entry: ToolEntry, usage: &HashMap<String, u32>) -> ToolEntry {
    entry.usage_count = usage.get(&entry.definition.id).copied().unwrap_or(0);
    entry
}

fn unknown(id: &str) -> String {
    format!("Tool '{id}' isn't in the catalog")
}

fn db_error(e: rusqlite::Error) -> String {
    format!("Tool catalog: {e}")
}

/// Upserts `definitions` (first of a duplicate ID wins), deletes the rows
/// they don't cover unless `keep` accepts their source. `enabled` is left
/// as the user set it. Whether anything changed.
fn apply(
    conn: &Connection,
    definitions: Vec<ToolDefinition>,
    keep: impl Fn(&ToolSource) -> bool,
) -> rusqlite::Result<bool> {
    let existing: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT id, definition_json FROM tool_catalog")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut changed = false;
    let mut seen = HashSet::new();
    let now = now_ms();
    for definition in definitions {
        if !seen.insert(definition.id.clone()) {
            continue;
        }
        let json = serde_json::to_string(&definition)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        if existing.get(&definition.id) == Some(&json) {
            continue;
        }
        conn.execute(
            "INSERT INTO tool_catalog (id, definition_json, source, source_id, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(id) DO UPDATE SET definition_json = excluded.definition_json,
                 source = excluded.source, source_id = excluded.source_id,
                 updated_at = excluded.updated_at",
            params![
                definition.id,
                json,
                definition.source.kind(),
                definition.source.source_id(),
                now
            ],
        )?;
        changed = true;
    }
    for (id, json) in &existing {
        if seen.contains(id) {
            continue;
        }
        let kept = serde_json::from_str::<ToolDefinition>(json).is_ok_and(|d| keep(&d.source));
        if !kept {
            conn.execute("DELETE FROM tool_catalog WHERE id = ?1", [id])?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Names of the tools the user switched off: agents are never offered them
pub fn disabled_tool_names(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    Ok(load_entries(conn)?
        .into_iter()
        .filter(|entry| !entry.enabled)
        .map(|entry| entry.definition.name)
        .collect())
}

fn load_entries(conn: &Connection) -> rusqlite::Result<Vec<ToolEntry>> {
    let mut stmt =
        conn.prepare("SELECT id, definition_json, COALESCE(enabled, 1) FROM tool_catalog")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, json, enabled) = row?;
        match serde_json::from_str::<ToolDefinition>(&json) {
            Ok(definition) => entries.push(ToolEntry {
                definition,
                enabled,
                usage_count: 0,
            }),
            Err(e) => log::warn!("Tool catalog: skipping unreadable row '{id}' ({e})"),
        }
    }
    Ok(entries)
}

struct UsageRow {
    tool: String,
    calls: u32,
    failures: u32,
    last_used_at: i64,
}

fn usage_rows(conn: &Connection, since: i64) -> rusqlite::Result<Vec<UsageRow>> {
    let mut stmt = conn.prepare(
        "SELECT json_extract(metadata, '$.tool') AS tool, COUNT(*),
                SUM(CASE WHEN json_extract(metadata, '$.isError') THEN 1 ELSE 0 END),
                MAX(created_at)
         FROM messages
         WHERE role = ?1 AND created_at >= ?2 AND json_valid(metadata)
           AND json_extract(metadata, '$.tool') IS NOT NULL
         GROUP BY tool",
    )?;
    let rows = stmt.query_map(params![crate::db::role::TOOL_CALL, since], |r| {
        Ok(UsageRow {
            tool: r.get(0)?,
            calls: r.get(1)?,
            failures: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
            last_used_at: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// Rebuilds when `.nandi/connectors.yaml` changes (install, uninstall)
struct ConnectorWatch(WeakAppState);

impl EventPublisher for ConnectorWatch {
    fn changed(&self, event: &DomainEvent) {
        if event.kind == EntityKind::Connectors {
            spawn_rebuild(&self.0);
        }
    }

    fn invalid(&self, _problem: &Problem, _changed_by: ChangedBy) {}
}

fn spawn_rebuild(weak: &WeakAppState) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    let runtime = state.runtime().clone();
    runtime.spawn(async move {
        if let Some(tools) = state.tools() {
            if let Err(e) = tools.rebuild().await {
                log::warn!("Tool catalog rebuild failed: {e}");
            }
        }
    });
}

/// Builds the catalog, subscribes to connector and pilot changes. Must not
/// block: the first full rebuild runs in the background.
pub fn init(state: &AppState) -> Option<Arc<ToolService>> {
    let service = match ToolService::new(
        state.db().clone(),
        state.bus().clone(),
        Some(state.downgrade()),
    ) {
        Ok(service) => Arc::new(service),
        Err(e) => {
            log::error!("Tool catalog disabled: {e}");
            return None;
        }
    };
    state
        .store()
        .subscribe(Arc::new(ConnectorWatch(state.downgrade())));
    let weak = state.downgrade();
    state.runtime().spawn(async move {
        let mut last = None;
        loop {
            let Some(state) = weak.upgrade() else {
                break;
            };
            let print = (connectors::fingerprint(&state), pilots::fingerprint(&state));
            // `tools()` is set right after `init` returns
            if let Some(tools) = state.tools() {
                if last.as_ref() != Some(&print) {
                    if let Err(e) = tools.rebuild().await {
                        log::warn!("Tool catalog rebuild failed: {e}");
                    }
                    last = Some(print);
                }
            }
            drop(state);
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
    Some(service)
}

fn service(state: &AppState) -> Result<&Arc<ToolService>, String> {
    state
        .tools()
        .ok_or_else(|| "The tool catalog is unavailable".to_string())
}

// Transport entry points (commands/tools.rs, http/tools.rs)

pub async fn list_tools(state: &AppState) -> Result<Vec<ToolEntry>, String> {
    service(state)?.list_all().await
}

pub async fn get_tool(state: &AppState, id: &str) -> Result<ToolEntry, String> {
    service(state)?.get(id).await
}

pub async fn set_tool_enabled(
    state: &AppState,
    id: &str,
    enabled: bool,
) -> Result<ToolEntry, String> {
    service(state)?.set_enabled(id, enabled).await
}

pub async fn get_tool_usage_stats(state: &AppState) -> Result<Vec<ToolUsageStats>, String> {
    service(state)?.usage_stats().await
}

#[cfg(test)]
mod tests;
