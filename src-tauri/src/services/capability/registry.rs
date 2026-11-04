//! Capability registries (wave A, Track B): git remotes whose
//! `capabilities/` folder (or `.nandi/capabilities/`) is merged into the
//! local capability index, read-only.
//!
//! - Clones live in `~/.nandi/local/cache/capability-registries/<url-hash>/`
//! - Local definitions override a registry's with the same ID; two
//!   registries may not define the same ID (the second is reported, not
//!   merged)
//! - Synced on add, on demand, and every `SYNC_INTERVAL_SECS`
//! - Never pushed to
//!
//! The index is in memory, rebuilt at startup from the clones already on
//! disk (no network), so a registry's capabilities are there before its
//! first sync of the session. Each sync also replaces the registry's rows
//! in `capabilities_projection` (`registry_origin` = the URL).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::definition::{self, CapabilityDefinition};
use super::{git_sync, projection, REGISTRY_SYNCED, REGISTRY_SYNC_FAILED};
use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::auth;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::secrets::Secrets;
use crate::services::AppState;

/// Every 6 hours
pub const SYNC_INTERVAL_SECS: u64 = 6 * 60 * 60;
/// Under `~/.nandi/local/cache/`
pub const REGISTRY_CACHE_DIR: &str = "capability-registries";
/// Folders inside a registry repo that hold capability files
const REGISTRY_FOLDERS: &[&str] = &[".nandi/capabilities", "capabilities"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RegistryKind {
    /// A path on disk (or `file://`)
    Local,
    Github,
    Gitlab,
    #[default]
    Generic,
}

impl RegistryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Generic => "generic",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "local" => Self::Local,
            "github" => Self::Github,
            "gitlab" => Self::Gitlab,
            _ => Self::Generic,
        }
    }

    /// From the URL's shape: a folder or `file://` is local, then by host
    pub fn detect(url: &str) -> Self {
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("file://") || is_local_path(url) {
            Self::Local
        } else if lower.contains("github.com") {
            Self::Github
        } else if lower.contains("gitlab") {
            Self::Gitlab
        } else {
            Self::Generic
        }
    }
}

/// A `capability_registries` row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRegistry {
    pub id: String,
    pub url: String,
    pub kind: RegistryKind,
    pub branch_prefix: Option<String>,
    pub user_slug: Option<String>,
    pub last_synced_at: Option<Timestamp>,
    pub last_commit: Option<String>,
    pub subscribed: bool,
    /// The last sync's error, if it failed (not persisted across restarts)
    pub last_error: Option<String>,
    /// Capabilities it currently contributes
    pub capability_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub url: String,
    pub commit: Option<String>,
    pub added: u32,
    pub updated: u32,
    pub removed: u32,
    /// Files that didn't parse, IDs another registry already defines, ...
    pub warnings: Vec<String>,
    /// Set when the sync failed (the previous clone stays in use)
    pub error: Option<String>,
}

/// A capability a registry contributes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryCapability {
    pub definition: CapabilityDefinition,
    /// The registry URL (`capabilities_projection.registry_origin`)
    pub origin: String,
    /// Path inside the clone
    pub source_path: String,
}

/// `registry://synced`: a sync (or a removal) changed what a registry
/// contributes
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistrySynced {
    #[serde(flatten)]
    result: SyncResult,
}

impl NandiEvent for RegistrySynced {
    const NAME: &'static str = REGISTRY_SYNCED;
}

/// `registry://sync_failed`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistrySyncFailed {
    url: String,
    error: String,
}

impl NandiEvent for RegistrySyncFailed {
    const NAME: &'static str = REGISTRY_SYNC_FAILED;
}

/// What a row holds
#[derive(Debug, Clone)]
struct Row {
    id: String,
    url: String,
    kind: RegistryKind,
    branch_prefix: Option<String>,
    user_slug: Option<String>,
    last_synced_at: Option<i64>,
    last_commit: Option<String>,
    subscribed: bool,
}

impl Row {
    /// `<prefix>/<slug>` when both are set, `<prefix>` alone, else the
    /// registry's default branch
    fn branch(&self) -> Option<String> {
        match (&self.branch_prefix, &self.user_slug) {
            (Some(prefix), Some(slug)) => Some(format!("{prefix}/{slug}")),
            (Some(prefix), None) => Some(prefix.clone()),
            _ => None,
        }
    }
}

pub struct CapabilityRegistryService {
    db: Db,
    bus: EventBus,
    /// For the GitHub sign-in's token (github.com registries)
    secrets: Secrets,
    /// `~/.nandi/local/cache`
    cache_dir: PathBuf,
    /// `.nandi/capabilities/` in the gold copy, for override warnings
    local_dir: Option<PathBuf>,
    /// Per registry URL, in subscription order (the first to claim an ID
    /// keeps it)
    index: RwLock<Vec<(String, Vec<RegistryCapability>)>>,
    /// The last failed sync's error per URL
    errors: RwLock<HashMap<String, String>>,
    /// One git operation at a time
    sync_lock: tokio::sync::Mutex<()>,
}

impl CapabilityRegistryService {
    pub fn new(
        db: Db,
        bus: EventBus,
        secrets: Secrets,
        cache_dir: PathBuf,
        local_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            db,
            bus,
            secrets,
            cache_dir,
            local_dir,
            index: RwLock::new(Vec::new()),
            errors: RwLock::new(HashMap::new()),
            sync_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Every capability the subscribed registries contribute (Track A
    /// merges these under the local ones).
    pub fn indexed_capabilities(&self) -> Vec<RegistryCapability> {
        self.index
            .read()
            .map(|index| index.iter().flat_map(|(_, caps)| caps.clone()).collect())
            .unwrap_or_default()
    }

    /// `indexed_capabilities` without the IDs defined locally (local wins)
    pub fn visible_capabilities(&self, local_ids: &HashSet<String>) -> Vec<RegistryCapability> {
        self.indexed_capabilities()
            .into_iter()
            .filter(|cap| !local_ids.contains(&cap.definition.id))
            .collect()
    }

    /// The registry file that defines `id`, as it is in the clone: the
    /// starting point for a local override
    pub fn source_text(&self, id: &str) -> Option<String> {
        let cap = self
            .indexed_capabilities()
            .into_iter()
            .find(|cap| cap.definition.id == id)?;
        fs::read_to_string(git_sync::clone_dir(&self.cache_dir, &cap.origin).join(cap.source_path))
            .ok()
    }

    pub async fn list(&self) -> Result<Vec<CapabilityRegistry>, String> {
        let rows = self.rows().await?;
        Ok(rows.into_iter().map(|row| self.to_registry(row)).collect())
    }

    /// Subscribes and does the first sync. A failed first sync keeps the
    /// subscription (its `last_error` says why); `kind`: `None` detects it.
    pub async fn add_registry(
        &self,
        url: &str,
        kind: Option<RegistryKind>,
        branch_prefix: Option<String>,
        user_slug: Option<String>,
    ) -> Result<CapabilityRegistry, String> {
        let url = normalize_url(url);
        validate_url(&url)?;
        let row = Row {
            id: git_sync::url_hash(&url),
            kind: kind.unwrap_or_else(|| RegistryKind::detect(&url)),
            branch_prefix: clean(branch_prefix),
            user_slug: clean(user_slug),
            last_synced_at: None,
            last_commit: None,
            subscribed: true,
            url: url.clone(),
        };
        let inserted = self
            .db
            .call(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO capability_registries
                       (id, url, kind, branch_prefix, user_slug, subscribed)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                    params![
                        row.id,
                        row.url,
                        row.kind.as_str(),
                        row.branch_prefix,
                        row.user_slug
                    ],
                )
            })
            .await?;
        if inserted == 0 {
            return Err(format!("Already subscribed to {url}"));
        }
        self.sync_registry(&url).await?;
        let row = self
            .row(&url)
            .await?
            .ok_or_else(|| format!("Not subscribed to {url}"))?;
        Ok(self.to_registry(row))
    }

    /// Unsubscribes, drops its capabilities, and deletes its clone
    pub async fn remove_registry(&self, url: &str) -> Result<(), String> {
        let url = normalize_url(url);
        let _guard = self.sync_lock.lock().await;
        let key = url.clone();
        let deleted = self
            .db
            .call(move |conn| {
                conn.execute("DELETE FROM capability_registries WHERE url = ?1", [key])
            })
            .await?;
        if deleted == 0 {
            return Err(format!("Not subscribed to {url}"));
        }
        let removed = self.take_index(&url);
        if let Ok(mut errors) = self.errors.write() {
            errors.remove(&url);
        }
        self.project(&url, &[]).await;
        let dir = git_sync::clone_dir(&self.cache_dir, &url);
        match tokio::task::spawn_blocking(move || git_sync::remove_clone(&dir)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log::warn!("{e}"),
            Err(e) => log::warn!("Failed to remove the clone of {url}: {e}"),
        }
        events::emit(
            &self.bus,
            &RegistrySynced {
                result: SyncResult {
                    url,
                    removed,
                    ..SyncResult::default()
                },
            },
        );
        Ok(())
    }

    /// Fetches (read-only) and re-indexes one registry. A failed fetch is
    /// reported in the result (and `registry://sync_failed`), not as `Err`;
    /// the previous clone's capabilities stay.
    pub async fn sync_registry(&self, url: &str) -> Result<SyncResult, String> {
        let url = normalize_url(url);
        let _guard = self.sync_lock.lock().await;
        let row = self
            .row(&url)
            .await?
            .ok_or_else(|| format!("Not subscribed to {url}"))?;
        let dir = git_sync::clone_dir(&self.cache_dir, &url);
        let (fetch_url, branch) = (url.clone(), row.branch());
        let token = auth::github::token_for_url(&self.secrets, &url);
        let fetched = tokio::task::spawn_blocking(move || {
            let commit =
                git_sync::clone_or_pull(&fetch_url, &dir, branch.as_deref(), token.as_deref())?;
            Ok::<_, String>((commit, scan(&dir, &fetch_url)))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);

        let (commit, (capabilities, mut warnings)) = match fetched {
            Ok(fetched) => fetched,
            Err(error) => {
                log::warn!("Capability registry {url} failed to sync: {error}");
                if let Ok(mut errors) = self.errors.write() {
                    errors.insert(url.clone(), error.clone());
                }
                events::emit(
                    &self.bus,
                    &RegistrySyncFailed {
                        url: url.clone(),
                        error: error.clone(),
                    },
                );
                return Ok(SyncResult {
                    url,
                    error: Some(error),
                    ..SyncResult::default()
                });
            }
        };

        let local_ids = self.local_ids();
        for cap in &capabilities {
            if local_ids.contains(&cap.definition.id) {
                warnings.push(format!(
                    "'{}' is also defined locally; the local definition wins",
                    cap.definition.id
                ));
            }
        }
        let (added, updated, removed) = self.put_index(&url, capabilities, &mut warnings);
        if let Ok(mut errors) = self.errors.write() {
            errors.remove(&url);
        }
        let (key, head) = (url.clone(), commit.clone());
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE capability_registries SET last_synced_at = ?1, last_commit = ?2
                     WHERE url = ?3",
                    params![now_ms(), head, key],
                )
            })
            .await?;
        let projected: Vec<_> = self.registry_capabilities(&url);
        self.project(&url, &projected).await;

        let result = SyncResult {
            url,
            commit: Some(commit),
            added,
            updated,
            removed,
            warnings,
            error: None,
        };
        events::emit(
            &self.bus,
            &RegistrySynced {
                result: result.clone(),
            },
        );
        Ok(result)
    }

    /// Syncs every subscribed registry, one after another
    pub async fn sync_all(&self) -> Vec<SyncResult> {
        let rows = match self.rows().await {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("Failed to list capability registries: {e}");
                return Vec::new();
            }
        };
        let mut results = Vec::new();
        for row in rows.into_iter().filter(|row| row.subscribed) {
            match self.sync_registry(&row.url).await {
                Ok(result) => results.push(result),
                Err(error) => results.push(SyncResult {
                    url: row.url,
                    error: Some(error),
                    ..SyncResult::default()
                }),
            }
        }
        results
    }

    /// Indexes the clones already on disk (no network), in subscription
    /// order. Blocking: called once at startup.
    pub fn load_from_disk(&self) -> Result<(), String> {
        let rows = {
            let conn = self.db.conn()?;
            read_rows(&conn).map_err(|e| e.to_string())?
        };
        for row in rows.into_iter().filter(|row| row.subscribed) {
            let dir = git_sync::clone_dir(&self.cache_dir, &row.url);
            if !dir.exists() {
                continue;
            }
            let (capabilities, mut warnings) = scan(&dir, &row.url);
            self.put_index(&row.url, capabilities, &mut warnings);
            for warning in warnings {
                log::warn!("Capability registry {}: {warning}", row.url);
            }
        }
        Ok(())
    }

    fn to_registry(&self, row: Row) -> CapabilityRegistry {
        let last_error = self
            .errors
            .read()
            .ok()
            .and_then(|errors| errors.get(&row.url).cloned());
        let capability_count = self.registry_capabilities(&row.url).len() as u32;
        CapabilityRegistry {
            id: row.id,
            url: row.url,
            kind: row.kind,
            branch_prefix: row.branch_prefix,
            user_slug: row.user_slug,
            last_synced_at: row.last_synced_at.map(|at| at as Timestamp),
            last_commit: row.last_commit,
            subscribed: row.subscribed,
            last_error,
            capability_count,
        }
    }

    fn registry_capabilities(&self, url: &str) -> Vec<RegistryCapability> {
        self.index
            .read()
            .ok()
            .and_then(|index| {
                index
                    .iter()
                    .find(|(origin, _)| origin == url)
                    .map(|(_, caps)| caps.clone())
            })
            .unwrap_or_default()
    }

    /// Replaces `url`'s entry, dropping IDs another registry already
    /// claims. Returns (added, updated, removed) against the old entry.
    fn put_index(
        &self,
        url: &str,
        capabilities: Vec<RegistryCapability>,
        warnings: &mut Vec<String>,
    ) -> (u32, u32, u32) {
        let Ok(mut index) = self.index.write() else {
            return (0, 0, 0);
        };
        let claimed: HashMap<String, String> = index
            .iter()
            .filter(|(origin, _)| origin != url)
            .flat_map(|(origin, caps)| {
                caps.iter()
                    .map(move |cap| (cap.definition.id.clone(), origin.clone()))
            })
            .collect();
        let mut kept = Vec::with_capacity(capabilities.len());
        for cap in capabilities {
            match claimed.get(&cap.definition.id) {
                Some(other) => warnings.push(format!(
                    "'{}' is already defined by the registry {other}; IDs must be unique across registries",
                    cap.definition.id
                )),
                None => kept.push(cap),
            }
        }

        let old: HashMap<String, CapabilityDefinition> = index
            .iter()
            .find(|(origin, _)| origin == url)
            .map(|(_, caps)| {
                caps.iter()
                    .map(|cap| (cap.definition.id.clone(), cap.definition.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let (mut added, mut updated) = (0, 0);
        for cap in &kept {
            match old.get(&cap.definition.id) {
                None => added += 1,
                Some(previous) if *previous != cap.definition => updated += 1,
                Some(_) => {}
            }
        }
        let new_ids: HashSet<&str> = kept.iter().map(|cap| cap.definition.id.as_str()).collect();
        let removed = old
            .keys()
            .filter(|id| !new_ids.contains(id.as_str()))
            .count() as u32;

        match index.iter_mut().find(|(origin, _)| origin == url) {
            Some(entry) => entry.1 = kept,
            None => index.push((url.to_string(), kept)),
        }
        (added, updated, removed)
    }

    /// Drops `url`'s entry; returns how many capabilities it had
    fn take_index(&self, url: &str) -> u32 {
        let Ok(mut index) = self.index.write() else {
            return 0;
        };
        let Some(position) = index.iter().position(|(origin, _)| origin == url) else {
            return 0;
        };
        index.remove(position).1.len() as u32
    }

    /// File stems in `.nandi/capabilities/` (a file is named by its ID)
    fn local_ids(&self) -> HashSet<String> {
        let Some(dir) = &self.local_dir else {
            return HashSet::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return HashSet::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_capability_file(path))
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
            .collect()
    }

    /// Replaces the registry's projection rows. Best-effort: the in-memory
    /// index is what `indexed_capabilities` serves.
    async fn project(&self, url: &str, capabilities: &[RegistryCapability]) {
        let rows: Vec<(CapabilityDefinition, String)> = capabilities
            .iter()
            .map(|cap| (cap.definition.clone(), cap.source_path.clone()))
            .collect();
        match projection::replace_origin(&self.db, Some(url), &rows).await {
            Ok(()) => {}
            Err(e) => log::warn!("Failed to project the capabilities of {url}: {e}"),
        }
    }

    async fn rows(&self) -> Result<Vec<Row>, String> {
        self.db.call(|conn| read_rows(conn)).await
    }

    async fn row(&self, url: &str) -> Result<Option<Row>, String> {
        let url = url.to_string();
        self.db
            .call(move |conn| {
                conn.query_row(&format!("{SELECT_ROW} WHERE url = ?1"), [url], row_from_sql)
                    .optional()
            })
            .await
    }
}

const SELECT_ROW: &str = "SELECT id, url, kind, branch_prefix, user_slug, last_synced_at,
        last_commit, COALESCE(subscribed, 1) FROM capability_registries";

fn read_rows(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<Row>> {
    let mut statement = conn.prepare(&format!("{SELECT_ROW} ORDER BY rowid"))?;
    let rows = statement.query_map([], row_from_sql)?;
    rows.collect()
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        url: row.get(1)?,
        kind: RegistryKind::parse(&row.get::<_, String>(2)?),
        branch_prefix: row.get(3)?,
        user_slug: row.get(4)?,
        last_synced_at: row.get(5)?,
        last_commit: row.get(6)?,
        subscribed: row.get::<_, i64>(7)? != 0,
    })
}

/// Every valid capability file in the clone's `.nandi/capabilities/` and
/// `capabilities/`, and a warning per file skipped. Blocking.
fn scan(dir: &Path, origin: &str) -> (Vec<RegistryCapability>, Vec<String>) {
    let mut capabilities: Vec<RegistryCapability> = Vec::new();
    let mut warnings = Vec::new();
    for folder in REGISTRY_FOLDERS {
        let Ok(entries) = fs::read_dir(dir.join(folder)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_capability_file(path))
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let source_path = format!("{folder}/{name}");
            let parsed = fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| definition::parse(&text))
                .and_then(|definition| definition::validate(&definition).map(|()| definition));
            match parsed {
                Ok(definition) => {
                    if capabilities
                        .iter()
                        .any(|cap| cap.definition.id == definition.id)
                    {
                        warnings.push(format!(
                            "{source_path}: '{}' is defined twice in this registry",
                            definition.id
                        ));
                        continue;
                    }
                    capabilities.push(RegistryCapability {
                        definition,
                        origin: origin.to_string(),
                        source_path,
                    });
                }
                Err(e) => warnings.push(format!("{source_path}: {e}")),
            }
        }
    }
    (capabilities, warnings)
}

fn is_capability_file(path: &Path) -> bool {
    path.is_file()
        && matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yaml" | "yml")
        )
}

/// Trimmed, without a trailing slash
fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let trimmed = url.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        url.to_string()
    } else {
        trimmed.to_string()
    }
}

/// A git URL (https, http, ssh, git, file, or scp-like `git@host:path`) or
/// a folder on this machine
fn validate_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("Enter the registry's git URL".to_string());
    }
    if url.chars().any(char::is_whitespace) {
        return Err(format!("'{url}' isn't a git URL"));
    }
    let lower = url.to_ascii_lowercase();
    let remote = ["https://", "http://", "ssh://", "git://", "file://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len());
    let scp = url
        .split_once(':')
        .is_some_and(|(user_host, path)| user_host.contains('@') && !path.is_empty());
    if remote || scp || is_local_path(url) {
        Ok(())
    } else {
        Err(format!(
            "'{url}' isn't a git URL or a folder on this machine"
        ))
    }
}

fn is_local_path(url: &str) -> bool {
    !url.contains("://") && Path::new(url).is_dir()
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().trim_matches('/').to_string())
        .filter(|v| !v.is_empty())
}

/// Loads the rows, indexes the clones on disk, and starts the 6-hourly
/// sync (whose first tick is right away, in the background).
pub fn init(state: &AppState) -> Option<Arc<CapabilityRegistryService>> {
    let cache_dir = state.nandi_dir().join("local").join("cache");
    let local_dir = state.store().repos().layout().capabilities_dir();
    let service = Arc::new(CapabilityRegistryService::new(
        state.db().clone(),
        state.bus().clone(),
        state.secrets().clone(),
        cache_dir,
        Some(local_dir),
    ));
    if let Err(e) = service.load_from_disk() {
        log::warn!("Failed to load capability registries: {e}");
    }
    let background = service.clone();
    state.runtime().spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(SYNC_INTERVAL_SECS));
        loop {
            tick.tick().await;
            for result in background.sync_all().await {
                if let Some(error) = result.error {
                    log::warn!("Capability registry {} failed to sync: {error}", result.url);
                }
            }
        }
    });
    Some(service)
}

fn service(state: &AppState) -> Result<&Arc<CapabilityRegistryService>, String> {
    state
        .capability_registries()
        .ok_or_else(|| "Capability registries aren't available".to_string())
}

pub async fn list_capability_registries(
    state: &AppState,
) -> Result<Vec<CapabilityRegistry>, String> {
    service(state)?.list().await
}

/// `kind`: `None` detects it from the URL
pub async fn add_capability_registry(
    state: &AppState,
    url: &str,
    kind: Option<RegistryKind>,
    branch_prefix: Option<String>,
    user_slug: Option<String>,
) -> Result<CapabilityRegistry, String> {
    service(state)?
        .add_registry(url, kind, branch_prefix, user_slug)
        .await
}

pub async fn remove_capability_registry(state: &AppState, url: &str) -> Result<(), String> {
    service(state)?.remove_registry(url).await
}

pub async fn sync_capability_registry(state: &AppState, url: &str) -> Result<SyncResult, String> {
    service(state)?.sync_registry(url).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;
    use git2::{IndexAddOption, Repository, Signature};

    /// A bare repo (the registry) and a work repo that pushes to it
    struct Fixture {
        _temp: tempfile::TempDir,
        bare: PathBuf,
        work: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let bare = temp.path().join("registry.git");
            let work = temp.path().join("work");
            let mut options = git2::RepositoryInitOptions::new();
            options.bare(true).initial_head("main");
            Repository::init_opts(&bare, &options).unwrap();
            let mut options = git2::RepositoryInitOptions::new();
            options.initial_head("main");
            let repo = Repository::init_opts(&work, &options).unwrap();
            repo.remote("origin", bare.to_str().unwrap()).unwrap();
            Self {
                _temp: temp,
                bare,
                work,
            }
        }

        fn url(&self) -> String {
            self.bare.to_str().unwrap().to_string()
        }

        fn write(&self, path: &str, text: &str) -> &Self {
            let path = self.work.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
            self
        }

        fn delete(&self, path: &str) -> &Self {
            fs::remove_file(self.work.join(path)).unwrap();
            self
        }

        /// Commits everything and pushes `main`
        fn publish(&self) {
            let repo = Repository::open(&self.work).unwrap();
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
                .unwrap();
            index.update_all(["*"].iter(), None).unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let signature = Signature::now("Test", "test@example.com").unwrap();
            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                "update",
                &tree,
                &parents,
            )
            .unwrap();
            repo.find_remote("origin")
                .unwrap()
                .push(&["+refs/heads/main:refs/heads/main"], None)
                .unwrap();
        }
    }

    fn capability(id: &str, version: &str) -> String {
        format!(
            "---\nid: {id}\nname: {id}\ndescription: Does {id}\nversion: {version}\n\
             owner: {{ kind: user, id: alice, display_name: Alice }}\n\
             required_tools: [shell]\n---\nWhat {id} does.\n"
        )
    }

    struct Harness {
        _temp: tempfile::TempDir,
        service: CapabilityRegistryService,
        sink: Arc<RecordingSink>,
        cache: PathBuf,
        local: PathBuf,
        db: Db,
    }

    fn harness() -> Harness {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let local = temp.path().join("local-capabilities");
        fs::create_dir_all(&local).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let db = Db::in_memory();
        let bus = EventBus::new(db.clone(), sink.clone());
        let service = CapabilityRegistryService::new(
            db.clone(),
            bus,
            Secrets::in_memory(),
            cache.clone(),
            Some(local.clone()),
        );
        Harness {
            _temp: temp,
            service,
            sink,
            cache,
            local,
            db,
        }
    }

    fn ids(caps: &[RegistryCapability]) -> Vec<String> {
        let mut ids: Vec<String> = caps.iter().map(|c| c.definition.id.clone()).collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn adds_a_registry_from_a_bare_repo() {
        let fixture = Fixture::new();
        fixture
            .write(
                "capabilities/review.python.yaml",
                &capability("review.python", "1.0.0"),
            )
            .write(
                ".nandi/capabilities/summarize.yaml",
                &capability("summarize", "0.2.0"),
            )
            .write("README.md", "not a capability")
            .publish();
        let h = harness();

        let registry = h
            .service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();
        assert_eq!(registry.kind, RegistryKind::Local);
        assert_eq!(registry.capability_count, 2);
        assert!(registry.last_commit.is_some());
        assert!(registry.last_synced_at.is_some());
        assert_eq!(registry.last_error, None);

        let caps = h.service.indexed_capabilities();
        assert_eq!(ids(&caps), vec!["review.python", "summarize"]);
        assert!(caps.iter().all(|c| c.origin == fixture.url()));
        let review = caps
            .iter()
            .find(|c| c.definition.id == "review.python")
            .unwrap();
        assert_eq!(review.source_path, "capabilities/review.python.yaml");
        assert_eq!(review.definition.body.trim(), "What review.python does.");
        assert!(git_sync::clone_dir(&h.cache, &fixture.url())
            .join(".git")
            .exists());
        assert!(h.sink.names().contains(&REGISTRY_SYNCED));

        let listed = h.service.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, git_sync::url_hash(&fixture.url()));
    }

    #[tokio::test]
    async fn source_text_reads_the_registry_file_by_id() {
        let fixture = Fixture::new();
        let text = capability("named.differently", "1.0.0");
        fixture.write("capabilities/other.yaml", &text).publish();
        let h = harness();
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();

        assert_eq!(h.service.source_text("named.differently"), Some(text));
        assert_eq!(h.service.source_text("missing"), None);
    }

    #[tokio::test]
    async fn sync_picks_up_added_updated_and_removed_capabilities() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/a.yaml", &capability("a", "1.0.0"))
            .write("capabilities/b.yaml", &capability("b", "1.0.0"))
            .publish();
        let h = harness();
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();

        fixture
            .write("capabilities/a.yaml", &capability("a", "1.1.0"))
            .delete("capabilities/b.yaml")
            .write("capabilities/c.yaml", &capability("c", "1.0.0"))
            .publish();
        let result = h.service.sync_registry(&fixture.url()).await.unwrap();
        assert_eq!(result.error, None);
        assert_eq!((result.added, result.updated, result.removed), (1, 1, 1));
        let caps = h.service.indexed_capabilities();
        assert_eq!(ids(&caps), vec!["a", "c"]);
        let a = caps.iter().find(|c| c.definition.id == "a").unwrap();
        assert_eq!(a.definition.version, "1.1.0");

        // Nothing changed: an empty diff
        let again = h.service.sync_registry(&fixture.url()).await.unwrap();
        assert_eq!((again.added, again.updated, again.removed), (0, 0, 0));
        assert_eq!(again.commit, result.commit);
    }

    #[tokio::test]
    async fn a_local_capability_overrides_the_registry_one() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/shared.yaml", &capability("shared", "1.0.0"))
            .write(
                "capabilities/only-remote.yaml",
                &capability("only-remote", "1.0.0"),
            )
            .publish();
        let h = harness();
        fs::write(h.local.join("shared.yaml"), capability("shared", "9.0.0")).unwrap();

        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();
        let result = h.service.sync_registry(&fixture.url()).await.unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("'shared'") && w.contains("locally")));

        let local_ids = HashSet::from(["shared".to_string()]);
        assert_eq!(
            ids(&h.service.visible_capabilities(&local_ids)),
            vec!["only-remote"]
        );
    }

    #[tokio::test]
    async fn two_registries_may_not_define_the_same_id() {
        let first = Fixture::new();
        first
            .write("capabilities/dup.yaml", &capability("dup", "1.0.0"))
            .publish();
        let second = Fixture::new();
        second
            .write("capabilities/dup.yaml", &capability("dup", "2.0.0"))
            .write("capabilities/unique.yaml", &capability("unique", "1.0.0"))
            .publish();
        let h = harness();
        h.service
            .add_registry(&first.url(), None, None, None)
            .await
            .unwrap();
        h.service
            .add_registry(&second.url(), None, None, None)
            .await
            .unwrap();

        let result = h.service.sync_registry(&second.url()).await.unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("'dup'")));
        let caps = h.service.indexed_capabilities();
        assert_eq!(ids(&caps), vec!["dup", "unique"]);
        let dup = caps.iter().find(|c| c.definition.id == "dup").unwrap();
        assert_eq!(dup.origin, first.url());
        assert_eq!(dup.definition.version, "1.0.0");
    }

    #[tokio::test]
    async fn removing_a_registry_clears_its_capabilities() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/a.yaml", &capability("a", "1.0.0"))
            .publish();
        let h = harness();
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();
        let clone = git_sync::clone_dir(&h.cache, &fixture.url());
        assert!(clone.exists());

        h.service.remove_registry(&fixture.url()).await.unwrap();
        assert!(h.service.indexed_capabilities().is_empty());
        assert!(h.service.list().await.unwrap().is_empty());
        assert!(!clone.exists());
        assert!(h.service.remove_registry(&fixture.url()).await.is_err());
    }

    #[tokio::test]
    async fn a_failed_sync_is_reported_and_keeps_the_previous_clone() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/a.yaml", &capability("a", "1.0.0"))
            .publish();
        let h = harness();
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();

        git_sync::remove_clone(&fixture.bare).unwrap();
        let result = h.service.sync_registry(&fixture.url()).await.unwrap();
        assert!(result.error.is_some());
        assert!(h.sink.names().contains(&REGISTRY_SYNC_FAILED));
        assert_eq!(ids(&h.service.indexed_capabilities()), vec!["a"]);
        let listed = h.service.list().await.unwrap();
        assert!(listed[0].last_error.is_some());
        assert_eq!(listed[0].capability_count, 1);
    }

    #[tokio::test]
    async fn a_registry_that_never_synced_keeps_its_subscription() {
        let h = harness();
        let registry = h
            .service
            .add_registry(
                "https://example.invalid/nobody/registry.git",
                None,
                None,
                None,
            )
            .await;
        // An unreachable host fails the fetch, not the subscription
        let registry = registry.unwrap();
        assert_eq!(registry.kind, RegistryKind::Generic);
        assert!(registry.last_error.is_some());
        assert_eq!(registry.capability_count, 0);
        assert!(h.service.indexed_capabilities().is_empty());
    }

    #[tokio::test]
    async fn rejects_bad_and_duplicate_urls() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/a.yaml", &capability("a", "1.0.0"))
            .publish();
        let h = harness();
        for bad in ["", "not a url", "nowhere-at-all", "https://"] {
            assert!(
                h.service.add_registry(bad, None, None, None).await.is_err(),
                "{bad:?}"
            );
        }
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();
        let again = h
            .service
            .add_registry(&format!("{}/", fixture.url()), None, None, None)
            .await;
        assert!(again.unwrap_err().contains("Already subscribed"));
        assert!(h
            .service
            .sync_registry("https://example.com/x.git")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn invalid_files_are_skipped_with_a_warning() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/good.yaml", &capability("good", "1.0.0"))
            .write(
                "capabilities/bad-version.yaml",
                &capability("bad-version", "one"),
            )
            .write("capabilities/Bad_ID.yaml", &capability("Bad_ID", "1.0.0"))
            .write("capabilities/broken.yaml", "---\nid: [unclosed\n---\n")
            .publish();
        let h = harness();
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();
        let result = h.service.sync_registry(&fixture.url()).await.unwrap();
        assert_eq!(result.warnings.len(), 3, "{:?}", result.warnings);
        assert_eq!(ids(&h.service.indexed_capabilities()), vec!["good"]);
    }

    #[tokio::test]
    async fn follows_the_prefix_and_slug_branch() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/on-main.yaml", &capability("on-main", "1.0.0"))
            .publish();
        let repo = Repository::open(&fixture.work).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("caps/alice", &head, false).unwrap();
        repo.set_head("refs/heads/caps/alice").unwrap();
        fixture.write(
            "capabilities/on-branch.yaml",
            &capability("on-branch", "1.0.0"),
        );
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Test", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "branch",
            &tree,
            &[&head],
        )
        .unwrap();
        repo.find_remote("origin")
            .unwrap()
            .push(&["refs/heads/caps/alice:refs/heads/caps/alice"], None)
            .unwrap();

        let h = harness();
        h.service
            .add_registry(
                &fixture.url(),
                None,
                Some("caps".into()),
                Some("alice".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            ids(&h.service.indexed_capabilities()),
            vec!["on-branch", "on-main"]
        );
    }

    #[tokio::test]
    async fn the_index_is_rebuilt_from_clones_on_disk() {
        let fixture = Fixture::new();
        fixture
            .write("capabilities/a.yaml", &capability("a", "1.0.0"))
            .publish();
        let h = harness();
        h.service
            .add_registry(&fixture.url(), None, None, None)
            .await
            .unwrap();

        // A restart: same database and cache, empty index, no network
        let restarted = CapabilityRegistryService::new(
            h.db.clone(),
            EventBus::new(h.db.clone(), h.sink.clone()),
            Secrets::in_memory(),
            h.cache.clone(),
            None,
        );
        assert!(restarted.indexed_capabilities().is_empty());
        restarted.load_from_disk().unwrap();
        assert_eq!(ids(&restarted.indexed_capabilities()), vec!["a"]);
    }

    #[test]
    fn detects_the_kind_from_the_url() {
        assert_eq!(
            RegistryKind::detect("https://github.com/acme/caps.git"),
            RegistryKind::Github
        );
        assert_eq!(
            RegistryKind::detect("git@github.com:acme/caps.git"),
            RegistryKind::Github
        );
        assert_eq!(
            RegistryKind::detect("https://gitlab.example.com/acme/caps"),
            RegistryKind::Gitlab
        );
        assert_eq!(
            RegistryKind::detect("file:///srv/caps.git"),
            RegistryKind::Local
        );
        assert_eq!(
            RegistryKind::detect("https://git.example.com/caps"),
            RegistryKind::Generic
        );
    }
}
