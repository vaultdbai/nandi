//! Capabilities as first-class entities (wave A).
//!
//! - `definition`: the file format (Track A)
//! - `projection`: files into `capabilities_projection` (Track A)
//! - `evidence`: one row per finished task, aggregates (Track A)
//! - `registry`, `git_sync`: subscribed git registries (Track B)
//! - `workflow_provider`, `aggregation`: workflows as providers, the
//!   hourly evidence job and ambiguity clusters (Track F)
//!
//! Files are the gold copy (`.nandi/capabilities/<id>.yaml`, written only
//! by `repos::capability_repo`); SQLite is rebuildable from them. An
//! agent provides a capability by naming it in skill.md (`serves:` or
//! `provides:`), a workflow by `provides:`.

pub mod aggregation;
pub mod definition;
pub mod evidence;
pub mod git_sync;
pub mod projection;
pub mod registry;
pub mod workflow_provider;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::a2a::task::TaskState;
use crate::services::auth;
use crate::services::domain::{Agent, ChangedBy, DomainEvent, EntityKind, Problem};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::network;
use crate::services::repo_store::{EventPublisher, RepoStore};
use crate::services::repos::capability_repo::CapabilityRepository;
use crate::services::tools::builtin::builtin_tools;
use crate::services::tools::definition::{normalize_id, ToolSource};
use crate::services::tools::{ToolCatalog, ToolService};
use crate::services::watcher;
use crate::services::AppState;
use definition::{OwnerKind, OwnershipChange};
use projection::{Projected, ProjectedCapability};
use registry::{CapabilityRegistryService, RegistryCapability};

pub use aggregation::AmbiguityCluster;
pub use definition::{
    CapabilityDefinition, CapabilityOwner, CapabilityRouteStrategy, ProviderKind,
};
pub use evidence::CapabilityEvidence;

// Events. Payloads are typed by the tracks that emit them.
pub const CAPABILITY_CREATED: &str = "capability://created";
pub const CAPABILITY_UPDATED: &str = "capability://updated";
pub const CAPABILITY_DELETED: &str = "capability://deleted";
pub const CAPABILITY_OWNERSHIP_TRANSFERRED: &str = "capability://ownership_transferred";
pub const PROVIDER_REGISTERED: &str = "provider://registered";
pub const PROVIDER_UNREGISTERED: &str = "provider://unregistered";
pub const REGISTRY_SYNCED: &str = "registry://synced";
pub const REGISTRY_SYNC_FAILED: &str = "registry://sync_failed";
pub const EVIDENCE_AGGREGATED: &str = "evidence://aggregated";

/// Where a listed capability comes from
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    /// `.nandi/capabilities/` in this instance's repo
    #[default]
    Local,
    /// A subscribed registry (`registry_origin` names it)
    Registry,
    /// Seen on the network (relay presence), no definition file here
    Network,
}

/// One entry of the Capabilities page: the definition and how this
/// instance relates to it (the page's You own / You provide / You use /
/// Discover sections)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityListing {
    pub definition: CapabilityDefinition,
    pub source: CapabilitySource,
    pub registry_origin: Option<String>,
    /// The owner is this instance's identity
    pub owned: bool,
    /// A local agent or workflow provides it
    pub provided: bool,
    /// This instance has requested it (requester evidence)
    pub used: bool,
    pub local_providers: u32,
    /// Local + remote providers online now
    pub online_providers: u32,
    pub success_rate: Option<f32>,
    /// Invocations in the last 30 days
    pub invocations: u32,
    /// Required tools the tool catalog doesn't have (yet)
    pub missing_tools: Vec<String>,
}

/// A `capability_providers` row, enriched with the local agent or workflow
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProvider {
    pub capability_id: String,
    pub capability_version: Option<String>,
    pub instance_id: String,
    pub provider_kind: ProviderKind,
    pub provider_id: String,
    /// The agent's or workflow's name, when local
    pub display_name: Option<String>,
    pub local: bool,
    pub enabled: bool,
    pub max_concurrent: Option<u32>,
    pub provided_tools: Vec<String>,
    pub registered_at: Option<Timestamp>,
}

/// One row of the provider breakdown table
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProviderStats {
    pub provider: CapabilityProvider,
    pub online: bool,
    pub load: u32,
    pub evidence: CapabilityEvidence,
}

/// Payload of `capability://created` and `capability://updated`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityChanged {
    pub id: String,
    pub definition: CapabilityDefinition,
    pub registry_origin: Option<String>,
}

/// Payload of `capability://deleted`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDeleted {
    pub id: String,
}

/// Payload of `capability://ownership_transferred`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityOwnershipTransferred {
    pub id: String,
    pub from: CapabilityOwner,
    pub to: CapabilityOwner,
    pub reason: Option<String>,
}

/// Payload of `provider://registered` and `provider://unregistered`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderChanged {
    pub capability_id: String,
    pub instance_id: String,
    pub provider_kind: ProviderKind,
    pub provider_id: String,
}

struct Created(CapabilityChanged);
struct Updated(CapabilityChanged);
struct Registered(ProviderChanged);
struct Unregistered(ProviderChanged);

macro_rules! wrapped_event {
    ($ty:ident, $name:expr) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(s)
            }
        }
        impl NandiEvent for $ty {
            const NAME: &'static str = $name;
        }
    };
}
wrapped_event!(Created, CAPABILITY_CREATED);
wrapped_event!(Updated, CAPABILITY_UPDATED);
wrapped_event!(Registered, PROVIDER_REGISTERED);
wrapped_event!(Unregistered, PROVIDER_UNREGISTERED);

impl NandiEvent for CapabilityDeleted {
    const NAME: &'static str = CAPABILITY_DELETED;
}

impl NandiEvent for CapabilityOwnershipTransferred {
    const NAME: &'static str = CAPABILITY_OWNERSHIP_TRANSFERRED;
}

/// Window of the listing's success rate and invocation count
const LISTING_WINDOW_DAYS: u32 = 30;
/// The default evidence window
pub const DEFAULT_WINDOW_DAYS: u32 = 30;

/// Capabilities, their providers, and their evidence (Track A; Track F
/// adds the ambiguity methods).
pub struct CapabilityService {
    db: Db,
    bus: EventBus,
    repo: CapabilityRepository,
    /// This instance (`capability_providers.instance_id` of local rows)
    instance_id: String,
    /// Local agents, for `serves:`/`provides:` and display names
    store: Option<RepoStore>,
    registries: Option<Arc<CapabilityRegistryService>>,
    tools: Option<Arc<ToolService>>,
    /// Owner IDs that mean "this instance's user" (the slug, the instance)
    owner_ids: RwLock<Vec<String>>,
}

impl CapabilityService {
    pub fn new(db: Db, bus: EventBus, repo: CapabilityRepository) -> Self {
        Self {
            db,
            bus,
            repo,
            instance_id: String::new(),
            store: None,
            registries: None,
            tools: None,
            owner_ids: RwLock::new(Vec::new()),
        }
    }

    pub fn with_instance_id(mut self, instance_id: impl Into<String>) -> Self {
        self.instance_id = instance_id.into();
        self.set_owner_ids(Vec::new());
        self
    }

    pub fn with_store(mut self, store: RepoStore) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_registries(mut self, registries: Option<Arc<CapabilityRegistryService>>) -> Self {
        self.registries = registries;
        self
    }

    pub fn with_tools(mut self, tools: Option<Arc<ToolService>>) -> Self {
        self.tools = tools;
        self
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    pub fn repo(&self) -> &CapabilityRepository {
        &self.repo
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Who "you" are for `owned` and new definitions: the user's slug (and
    /// always this instance's ID)
    pub fn set_owner_ids(&self, ids: Vec<String>) {
        let mut all: Vec<String> = ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
        if !self.instance_id.is_empty() && !all.contains(&self.instance_id) {
            all.push(self.instance_id.clone());
        }
        *self.owner_ids.write().unwrap_or_else(|e| e.into_inner()) = all;
    }

    fn owner_ids(&self) -> Vec<String> {
        self.owner_ids
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Owned by this instance's user
    pub fn is_owned(&self, definition: &CapabilityDefinition) -> bool {
        definition.owner.kind == OwnerKind::User && self.owner_ids().contains(&definition.owner.id)
    }

    /// Local definitions plus the registries' (local wins on the same ID)
    pub async fn list_all(&self) -> Result<Vec<CapabilityDefinition>, String> {
        Ok(self
            .merged()
            .await?
            .into_iter()
            .map(|p| p.definition)
            .collect())
    }

    /// Projected rows plus registry definitions not projected yet, by ID
    async fn merged(&self) -> Result<Vec<ProjectedCapability>, String> {
        let mut by_id: BTreeMap<String, ProjectedCapability> = projection::list(&self.db)
            .await?
            .into_iter()
            .map(|p| (p.definition.id.clone(), p))
            .collect();
        if let Some(registries) = &self.registries {
            for indexed in registries.indexed_capabilities() {
                by_id
                    .entry(indexed.definition.id.clone())
                    .or_insert(ProjectedCapability {
                        definition: indexed.definition,
                        source_path: Some(indexed.source_path),
                        registry_origin: Some(indexed.origin),
                    });
            }
        }
        Ok(by_id.into_values().collect())
    }

    /// `list_all` with relations, for the Capabilities page. Capabilities
    /// that only have providers (an agent serving a name with no file
    /// here, or remote presence) are listed with a placeholder definition.
    pub async fn list_listings(&self) -> Result<Vec<CapabilityListing>, String> {
        let known = self.merged().await?;
        let providers = self.all_provider_rows().await?;
        let mut listings = Vec::with_capacity(known.len());
        let mut seen = HashSet::new();
        for projected in known {
            seen.insert(projected.definition.id.clone());
            let source = if projected.registry_origin.is_some() {
                CapabilitySource::Registry
            } else {
                CapabilitySource::Local
            };
            listings.push(
                self.listing(
                    projected.definition,
                    source,
                    projected.registry_origin,
                    &providers,
                )
                .await?,
            );
        }
        let mut orphans: BTreeMap<String, Vec<&ProviderRow>> = BTreeMap::new();
        for row in &providers {
            if !seen.contains(&row.capability_id) {
                orphans
                    .entry(row.capability_id.clone())
                    .or_default()
                    .push(row);
            }
        }
        for (id, rows) in orphans {
            let local = rows.iter().any(|r| r.instance_id == self.instance_id);
            let definition = CapabilityDefinition {
                name: id.clone(),
                id,
                version: rows
                    .iter()
                    .find_map(|r| r.capability_version.clone())
                    .unwrap_or_default(),
                ..CapabilityDefinition::default()
            };
            let source = if local {
                CapabilitySource::Local
            } else {
                CapabilitySource::Network
            };
            listings.push(self.listing(definition, source, None, &providers).await?);
        }
        listings.sort_by(|a, b| a.definition.id.cmp(&b.definition.id));
        Ok(listings)
    }

    async fn listing(
        &self,
        definition: CapabilityDefinition,
        source: CapabilitySource,
        registry_origin: Option<String>,
        providers: &[ProviderRow],
    ) -> Result<CapabilityListing, String> {
        let id = definition.id.clone();
        let rows: Vec<&ProviderRow> = providers.iter().filter(|r| r.capability_id == id).collect();
        let local_providers = rows
            .iter()
            .filter(|r| r.instance_id == self.instance_id)
            .count() as u32;
        let online_providers = rows.iter().filter(|r| r.enabled).count() as u32;
        let evidence = evidence::aggregate(&self.db, &id, None, LISTING_WINDOW_DAYS).await?;
        let invocations: u32 = evidence.iter().map(|e| e.count).sum();
        let successes: u32 = evidence.iter().map(|e| e.success_count).sum();
        let used = evidence::has_requested(&self.db, &id).await?;
        Ok(CapabilityListing {
            owned: source != CapabilitySource::Network && self.is_owned(&definition),
            missing_tools: self.missing_tools(&definition),
            definition,
            source,
            registry_origin,
            provided: local_providers > 0,
            used,
            local_providers,
            online_providers,
            success_rate: (invocations > 0).then(|| successes as f32 / invocations as f32),
            invocations,
        })
    }

    /// Required tools (and routing's hard requirements) the catalog doesn't
    /// have. Empty without a tool catalog: nothing is known to be missing.
    pub fn missing_tools(&self, definition: &CapabilityDefinition) -> Vec<String> {
        let Some(tools) = &self.tools else {
            return Vec::new();
        };
        let mut required: Vec<String> = Vec::new();
        for tool in definition
            .required_tools
            .iter()
            .chain(&definition.routing.require_tools)
        {
            let id = normalize_id(tool);
            if !tool.trim().is_empty() && !required.contains(&id) {
                required.push(id);
            }
        }
        let catalog = tools.catalog();
        catalog
            .missing(&required)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    pub async fn get(&self, id: &str) -> Result<CapabilityDefinition, String> {
        if let Some(found) = projection::get(&self.db, id).await? {
            return Ok(found.definition);
        }
        self.registry_definition(id)
            .map(|indexed| indexed.definition)
            .ok_or_else(|| format!("Capability '{id}' not found"))
    }

    fn registry_definition(&self, id: &str) -> Option<RegistryCapability> {
        self.registries.as_ref().and_then(|registries| {
            registries
                .indexed_capabilities()
                .into_iter()
                .find(|c| c.definition.id == id)
        })
    }

    /// Validates, writes the file (staged), projects it, emits
    /// `capability://created|updated`. Returns it as saved. An unset owner
    /// becomes this instance's user; `created_at` is kept from the file.
    pub async fn save(&self, def: &CapabilityDefinition) -> Result<CapabilityDefinition, String> {
        let mut definition = def.clone();
        definition.id = definition.id.trim().to_string();
        definition.version = definition.version.trim().to_string();
        if definition.owner.id.trim().is_empty() {
            let me = self.owner_ids().into_iter().next().unwrap_or_default();
            definition.owner = CapabilityOwner {
                kind: OwnerKind::User,
                display_name: if definition.owner.display_name.trim().is_empty() {
                    me.clone()
                } else {
                    definition.owner.display_name.clone()
                },
                id: me,
                ..definition.owner
            };
        }
        let previous = self.local(&definition.id).await;
        let now = now_ms() as f64;
        definition.created_at = previous
            .as_ref()
            .and_then(|p| p.created_at)
            .or(definition.created_at)
            .or(Some(now));
        definition.updated_at = Some(now);
        self.write(&definition).await?;
        Ok(definition)
    }

    /// The local file's definition, if there is one
    async fn local(&self, id: &str) -> Option<CapabilityDefinition> {
        let repo = self.repo.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || repo.load(&id).ok())
            .await
            .ok()
            .flatten()
    }

    /// Writes the file, projects it, emits created/updated
    async fn write(&self, definition: &CapabilityDefinition) -> Result<(), String> {
        let repo = self.repo.clone();
        let to_write = definition.clone();
        tokio::task::spawn_blocking(move || repo.save(&to_write))
            .await
            .map_err(|e| format!("Capability write failed: {e}"))??;
        self.warn_missing_tools(definition);
        let path = self.repo.path(&definition.id).display().to_string();
        let projected = projection::upsert(&self.db, definition, &path, None).await?;
        self.announce(projected, definition, None);
        Ok(())
    }

    fn announce(
        &self,
        projected: Projected,
        definition: &CapabilityDefinition,
        registry_origin: Option<String>,
    ) {
        let payload = CapabilityChanged {
            id: definition.id.clone(),
            definition: definition.clone(),
            registry_origin,
        };
        match projected {
            Projected::Created => events::emit(&self.bus, &Created(payload)),
            Projected::Updated => events::emit(&self.bus, &Updated(payload)),
            Projected::Unchanged => {}
        }
    }

    fn warn_missing_tools(&self, definition: &CapabilityDefinition) {
        let missing = self.missing_tools(definition);
        if !missing.is_empty() {
            log::warn!(
                "Capability '{}' requires tools that aren't installed: {}",
                definition.id,
                missing.join(", ")
            );
        }
    }

    /// Removes the file and the row, emits `capability://deleted`. A
    /// registry's definition of the same ID takes its place.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let repo = self.repo.clone();
        let target = id.to_string();
        let removed = tokio::task::spawn_blocking(move || repo.delete(&target))
            .await
            .map_err(|e| format!("Capability delete failed: {e}"))??;
        if !removed {
            return Err(if self.registry_definition(id).is_some() {
                format!("Capability '{id}' comes from a registry; unsubscribe to remove it")
            } else {
                format!("Capability '{id}' not found")
            });
        }
        self.forget_local(id).await
    }

    /// The local file is gone: drop its row, fall back to a registry's
    async fn forget_local(&self, id: &str) -> Result<(), String> {
        let was_local = projection::get(&self.db, id)
            .await?
            .is_some_and(|p| p.registry_origin.is_none());
        if !was_local {
            return Ok(());
        }
        projection::remove(&self.db, id).await?;
        events::emit(&self.bus, &CapabilityDeleted { id: id.to_string() });
        if let Some(indexed) = self.registry_definition(id) {
            let projected = projection::upsert(
                &self.db,
                &indexed.definition,
                &indexed.source_path,
                Some(&indexed.origin),
            )
            .await?;
            self.announce(projected, &indexed.definition, Some(indexed.origin));
        }
        Ok(())
    }

    /// Appends to the owner history, writes the file, emits
    /// `capability://ownership_transferred`. Only local definitions whose
    /// owner marked them `transferable`.
    pub async fn transfer_ownership(
        &self,
        id: &str,
        new_owner: CapabilityOwner,
        reason: Option<String>,
    ) -> Result<CapabilityDefinition, String> {
        let Some(mut definition) = self.local(id).await else {
            return Err(if self.registry_definition(id).is_some() {
                format!("Capability '{id}' comes from a registry; its owner transfers it there")
            } else {
                format!("Capability '{id}' not found")
            });
        };
        if !definition.owner.transferable {
            return Err(format!(
                "Capability '{id}' isn't transferable (its owner set transferable: false)"
            ));
        }
        if new_owner.id.trim().is_empty() {
            return Err("The new owner needs an ID".to_string());
        }
        if new_owner.id == definition.owner.id && new_owner.kind == definition.owner.kind {
            return Err(format!(
                "Capability '{id}' already belongs to {}",
                new_owner.id
            ));
        }
        let from = definition.owner.clone();
        let reason = reason
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty());
        let mut history = from.history.clone();
        history.push(OwnershipChange {
            from: from.id.clone(),
            to: new_owner.id.trim().to_string(),
            at: now_ms() as f64,
            reason: reason.clone(),
        });
        definition.owner = CapabilityOwner {
            id: new_owner.id.trim().to_string(),
            display_name: if new_owner.display_name.trim().is_empty() {
                new_owner.id.trim().to_string()
            } else {
                new_owner.display_name.trim().to_string()
            },
            history,
            ..new_owner
        };
        definition.updated_at = Some(now_ms() as f64);
        self.write(&definition).await?;
        events::emit(
            &self.bus,
            &CapabilityOwnershipTransferred {
                id: id.to_string(),
                from: CapabilityOwner {
                    history: Vec::new(),
                    ..from
                },
                to: CapabilityOwner {
                    history: Vec::new(),
                    ..definition.owner.clone()
                },
                reason,
            },
        );
        Ok(definition)
    }

    /// Every `capability_providers` row
    async fn all_provider_rows(&self) -> Result<Vec<ProviderRow>, String> {
        self.db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT capability_id, capability_version, instance_id, provider_kind,
                            provider_id, enabled, max_concurrent, provided_tools, registered_at
                       FROM capability_providers
                      ORDER BY capability_id, instance_id, provider_id",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(ProviderRow {
                            capability_id: row.get(0)?,
                            capability_version: row.get(1)?,
                            instance_id: row.get(2)?,
                            provider_kind: row.get(3)?,
                            provider_id: row.get(4)?,
                            enabled: row.get::<_, Option<i64>>(5)?.unwrap_or(1) != 0,
                            max_concurrent: row.get(6)?,
                            provided_tools: row.get(7)?,
                            registered_at: row.get(8)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Agents and workflows providing `id`, local first
    pub async fn list_providers(&self, id: &str) -> Result<Vec<CapabilityProvider>, String> {
        let mut providers: Vec<CapabilityProvider> = self
            .all_provider_rows()
            .await?
            .into_iter()
            .filter(|row| row.capability_id == id)
            .map(|row| self.enrich(row))
            .collect();
        providers.sort_by(|a, b| {
            b.local
                .cmp(&a.local)
                .then_with(|| a.instance_id.cmp(&b.instance_id))
                .then_with(|| a.provider_id.cmp(&b.provider_id))
        });
        Ok(providers)
    }

    fn enrich(&self, row: ProviderRow) -> CapabilityProvider {
        let local = row.instance_id == self.instance_id;
        let kind = ProviderKind::parse(&row.provider_kind).unwrap_or_default();
        let display_name = match (&self.store, local) {
            (Some(store), true) => store.read(|repo| match kind {
                ProviderKind::Agent => repo.agent(&row.provider_id).map(|a| a.name().to_string()),
                ProviderKind::Workflow => repo
                    .workflow(&row.provider_id)
                    .map(|w| w.name().to_string()),
            }),
            _ => None,
        };
        CapabilityProvider {
            capability_id: row.capability_id,
            capability_version: row.capability_version,
            instance_id: row.instance_id,
            provider_kind: kind,
            provider_id: row.provider_id,
            display_name,
            local,
            enabled: row.enabled,
            max_concurrent: row.max_concurrent,
            provided_tools: row
                .provided_tools
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
            registered_at: row.registered_at.map(|ms| ms as f64),
        }
    }

    /// Per provider over `window_days` (0: all time)
    pub async fn evidence(
        &self,
        id: &str,
        window_days: u32,
    ) -> Result<Vec<CapabilityEvidence>, String> {
        evidence::aggregate(&self.db, id, None, window_days).await
    }

    /// One row per provider: registered ones (with or without evidence) and
    /// any that only have evidence (e.g. a provider since removed)
    pub async fn provider_breakdown(
        &self,
        id: &str,
    ) -> Result<Vec<CapabilityProviderStats>, String> {
        let providers = self.list_providers(id).await?;
        let mut evidence: HashMap<(String, String), CapabilityEvidence> =
            evidence::aggregate(&self.db, id, None, DEFAULT_WINDOW_DAYS)
                .await?
                .into_iter()
                .map(|e| ((e.instance_id.clone(), e.provider_id.clone()), e))
                .collect();
        let loads = self.open_task_counts(id).await?;
        let mut stats = Vec::new();
        for provider in providers {
            let key = (provider.instance_id.clone(), provider.provider_id.clone());
            let evidence = evidence.remove(&key).unwrap_or_else(|| CapabilityEvidence {
                capability_id: id.to_string(),
                capability_version: provider.capability_version.clone(),
                instance_id: provider.instance_id.clone(),
                provider_kind: provider.provider_kind,
                provider_id: provider.provider_id.clone(),
                window_days: DEFAULT_WINDOW_DAYS,
                ..CapabilityEvidence::default()
            });
            stats.push(CapabilityProviderStats {
                online: provider.enabled,
                load: loads.get(&key).copied().unwrap_or(0),
                provider,
                evidence,
            });
        }
        let mut gone: Vec<_> = evidence.into_values().collect();
        gone.sort_by_key(|e| std::cmp::Reverse(e.count));
        for evidence in gone {
            let key = (evidence.instance_id.clone(), evidence.provider_id.clone());
            stats.push(CapabilityProviderStats {
                provider: CapabilityProvider {
                    capability_id: id.to_string(),
                    capability_version: evidence.capability_version.clone(),
                    local: evidence.instance_id == self.instance_id,
                    instance_id: evidence.instance_id.clone(),
                    provider_kind: evidence.provider_kind,
                    provider_id: evidence.provider_id.clone(),
                    display_name: None,
                    enabled: false,
                    max_concurrent: None,
                    provided_tools: Vec::new(),
                    registered_at: None,
                },
                online: false,
                load: loads.get(&key).copied().unwrap_or(0),
                evidence,
            });
        }
        Ok(stats)
    }

    /// Unfinished tasks per (provider instance, provider) for `id`
    async fn open_task_counts(&self, id: &str) -> Result<HashMap<(String, String), u32>, String> {
        let capability = id.to_string();
        let terminal: Vec<&'static str> = TaskState::ALL
            .iter()
            .filter(|s| s.is_terminal())
            .map(|s| s.as_str())
            .collect();
        self.db
            .call(move |conn| {
                let placeholders = vec!["?"; terminal.len()].join(", ");
                let sql = format!(
                    "SELECT provider_instance, provider_agent, COUNT(*) FROM tasks
                      WHERE capability = ? AND state NOT IN ({placeholders})
                      GROUP BY provider_instance, provider_agent"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut args: Vec<&dyn rusqlite::ToSql> = vec![&capability];
                for state in &terminal {
                    args.push(state);
                }
                let rows = stmt
                    .query_map(args.as_slice(), |row| {
                        Ok(((row.get(0)?, row.get(1)?), row.get(2)?))
                    })?
                    .collect::<Result<HashMap<_, _>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Track F: the capability's clarification clusters, most frequent
    /// first (filled by the hourly aggregation job)
    pub async fn list_ambiguities(
        &self,
        capability_id: &str,
    ) -> Result<Vec<AmbiguityCluster>, String> {
        let id = capability_id.to_string();
        self.db
            .call(move |conn| aggregation::list_ambiguities(conn, &id))
            .await
    }

    /// Track F: adds an input-schema field from the cluster, writes the
    /// file, marks the cluster promoted. Returns the updated definition.
    pub async fn promote_ambiguity_to_schema(
        &self,
        capability_id: &str,
        ambiguity_cluster_id: &str,
    ) -> Result<CapabilityDefinition, String> {
        let definition = self.get(capability_id).await?;
        let cluster_id = ambiguity_cluster_id.to_string();
        let promoted = self
            .db
            .call({
                let (definition, cluster_id) = (definition.clone(), cluster_id.clone());
                move |conn| Ok(aggregation::promotion(conn, &definition, &cluster_id))
            })
            .await??;
        let saved = match promoted {
            Some(promoted) => self.save(&promoted).await?,
            // The field is already there: only the mark is missing
            None => definition,
        };
        self.db
            .call(move |conn| aggregation::mark_promoted(conn, &cluster_id))
            .await?;
        Ok(saved)
    }

    /// The watcher's entry point for a changed path under
    /// `.nandi/capabilities/` (created, edited, or removed). The folder
    /// itself (or anything that isn't a capability file) re-projects
    /// everything. An invalid edit keeps the last good version.
    pub async fn reload_entity(&self, path: &Path) -> Result<(), String> {
        let Some(id) = self.repo.id_of(path) else {
            return self.rebuild().await;
        };
        let repo = self.repo.clone();
        let read_id = id.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            let yaml = repo.path(&read_id);
            let yml = yaml.with_extension("yml");
            match [yaml, yml].into_iter().find(|p| p.is_file()) {
                Some(found) => repo.load_path(&found).map(|d| Some((d, found))),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| format!("Capability reload failed: {e}"))?;
        match loaded {
            Ok(Some((definition, found))) => {
                self.warn_missing_tools(&definition);
                let projected =
                    projection::upsert(&self.db, &definition, &found.display().to_string(), None)
                        .await?;
                self.announce(projected, &definition, None);
                Ok(())
            }
            Ok(None) => self.forget_local(&id).await,
            Err(e) => {
                log::warn!("Capability '{id}' file is invalid; keeping the last good version: {e}");
                Err(e)
            }
        }
    }

    /// Re-projects every local file (startup, `nandi rebuild`, the folder
    /// replaced), emitting created/updated/deleted for what changed.
    pub async fn rebuild(&self) -> Result<(), String> {
        let repo = self.repo.clone();
        let definitions = tokio::task::spawn_blocking(move || repo.load_all())
            .await
            .map_err(|e| format!("Capability rebuild failed: {e}"))??;
        let before: HashMap<String, CapabilityDefinition> = projection::list(&self.db)
            .await?
            .into_iter()
            .filter(|p| p.registry_origin.is_none())
            .map(|p| (p.definition.id.clone(), p.definition))
            .collect();
        let rows: Vec<(CapabilityDefinition, String)> = definitions
            .iter()
            .map(|d| (d.clone(), self.repo.path(&d.id).display().to_string()))
            .collect();
        projection::replace_origin(&self.db, None, &rows).await?;
        let mut kept = HashSet::new();
        for definition in &definitions {
            self.warn_missing_tools(definition);
            kept.insert(definition.id.clone());
            let projected = match before.get(&definition.id) {
                None => Projected::Created,
                Some(old) if old == definition => Projected::Unchanged,
                Some(_) => Projected::Updated,
            };
            self.announce(projected, definition, None);
        }
        for id in before.keys().filter(|id| !kept.contains(*id)) {
            events::emit(&self.bus, &CapabilityDeleted { id: id.clone() });
            if let Some(indexed) = self.registry_definition(id) {
                let projected = projection::upsert(
                    &self.db,
                    &indexed.definition,
                    &indexed.source_path,
                    Some(&indexed.origin),
                )
                .await?;
                self.announce(projected, &indexed.definition, Some(indexed.origin));
            }
        }
        Ok(())
    }

    /// Re-derives this instance's agent rows in `capability_providers`
    /// from the local agents' `serves:`/`provides:`, emitting
    /// `provider://registered|unregistered` for what changed. Workflow rows
    /// (Track F) are left alone.
    pub async fn refresh_local_providers(&self) -> Result<(), String> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let agents: Vec<Agent> = store.read(|repo| repo.agents().cloned().collect());
        let versions: HashMap<String, String> = projection::list(&self.db)
            .await?
            .into_iter()
            .map(|p| (p.definition.id, p.definition.version))
            .collect();
        let catalog = self.tools.as_ref().map(|tools| tools.catalog());
        let mut rows = Vec::new();
        for agent in &agents {
            let serves = agent.capabilities().serves();
            if serves.is_empty() {
                continue;
            }
            let tools = serde_json::to_string(&agent_tools(agent, catalog.as_ref()))
                .unwrap_or_else(|_| "[]".to_string());
            for capability in serves {
                rows.push(ProviderRow {
                    capability_version: versions.get(&capability.name).cloned(),
                    capability_id: capability.name.clone(),
                    instance_id: self.instance_id.clone(),
                    provider_kind: ProviderKind::Agent.as_str().to_string(),
                    provider_id: agent.id().to_string(),
                    enabled: true,
                    max_concurrent: Some(agent.capabilities().delegation_limit()),
                    provided_tools: Some(tools.clone()),
                    registered_at: Some(now_ms()),
                });
            }
        }
        let instance = self.instance_id.clone();
        let (added, removed) = self
            .db
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let before: HashSet<(String, String)> = {
                    let mut stmt = tx.prepare(
                        "SELECT capability_id, provider_id FROM capability_providers
                          WHERE instance_id = ?1 AND provider_kind = 'agent'",
                    )?;
                    let rows = stmt
                        .query_map([&instance], |row| Ok((row.get(0)?, row.get(1)?)))?
                        .collect::<Result<HashSet<_>, _>>()?;
                    rows
                };
                tx.execute(
                    "DELETE FROM capability_providers
                      WHERE instance_id = ?1 AND provider_kind = 'agent'",
                    [&instance],
                )?;
                let mut after = HashSet::new();
                for row in &rows {
                    // Keep the first registration time of a kept row
                    tx.execute(
                        "INSERT OR REPLACE INTO capability_providers (capability_id,
                            capability_version, instance_id, provider_kind, provider_id, enabled,
                            max_concurrent, provided_tools, registered_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8)",
                        params![
                            row.capability_id,
                            row.capability_version,
                            row.instance_id,
                            row.provider_kind,
                            row.provider_id,
                            row.max_concurrent,
                            row.provided_tools,
                            row.registered_at,
                        ],
                    )?;
                    after.insert((row.capability_id.clone(), row.provider_id.clone()));
                }
                tx.commit()?;
                let added: Vec<_> = after.difference(&before).cloned().collect();
                let removed: Vec<_> = before.difference(&after).cloned().collect();
                Ok((added, removed))
            })
            .await?;
        let change = |(capability_id, provider_id): (String, String)| ProviderChanged {
            capability_id,
            instance_id: self.instance_id.clone(),
            provider_kind: ProviderKind::Agent,
            provider_id,
        };
        for key in added {
            events::emit(&self.bus, &Registered(change(key)));
        }
        for key in removed {
            events::emit(&self.bus, &Unregistered(change(key)));
        }
        Ok(())
    }
}

/// A `capability_providers` row
#[derive(Debug, Clone)]
struct ProviderRow {
    capability_id: String,
    capability_version: Option<String>,
    instance_id: String,
    provider_kind: String,
    provider_id: String,
    enabled: bool,
    max_concurrent: Option<u32>,
    provided_tools: Option<String>,
    registered_at: Option<i64>,
}

/// The tool IDs an agent brings: its allowed built-ins (every built-in the
/// catalog knows when `allowed-tools` is omitted), and the catalog's tools
/// of its declared connectors and pilots. Sorted, distinct.
fn agent_tools(agent: &Agent, catalog: Option<&ToolCatalog>) -> Vec<String> {
    let connectors = crate::services::connectors::declared_connectors(agent);
    let pilots = crate::services::pilots::tools::declared(agent);
    let mut ids: Vec<String> = match agent.capabilities().tools().allowed() {
        Some(allowed) => allowed.iter().map(|t| normalize_id(t)).collect(),
        None => match catalog {
            Some(catalog) => catalog
                .all()
                .into_iter()
                .filter(|e| matches!(e.definition.source, ToolSource::Builtin))
                .map(|e| e.definition.id.clone())
                .collect(),
            None => builtin_tools().into_iter().map(|t| t.id).collect(),
        },
    };
    if let Some(catalog) = catalog {
        for entry in catalog.all() {
            let from_agent = match &entry.definition.source {
                ToolSource::Connector { connector_id } => connectors.contains(connector_id),
                ToolSource::Pilot { pilot_id } => pilots.contains(pilot_id),
                _ => false,
            };
            if from_agent {
                ids.push(entry.definition.id.clone());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Creates `.nandi/capabilities/` (with a staged `.gitkeep`) when missing.
/// Implemented by the freeze; runs at startup.
pub fn ensure_capabilities_dir(state: &AppState) {
    let repo = CapabilityRepository::new(state.store().repos().layout().clone());
    match repo.ensure_dir() {
        Ok(true) => log::info!("Created {}", repo.dir().display()),
        Ok(false) => {}
        Err(e) => log::warn!("Failed to create {}: {e}", repo.dir().display()),
    }
}

/// Keeps `capability_providers` in step with agent changes
struct ProviderRefresher {
    service: std::sync::Weak<CapabilityService>,
    runtime: tokio::runtime::Handle,
}

impl EventPublisher for ProviderRefresher {
    fn changed(&self, event: &DomainEvent) {
        if event.kind != EntityKind::Agent {
            return;
        }
        let Some(service) = self.service.upgrade() else {
            return;
        };
        self.runtime.spawn(async move {
            if let Err(e) = service.refresh_local_providers().await {
                log::warn!("Failed to refresh capability providers: {e}");
            }
        });
    }

    fn invalid(&self, _problem: &Problem, _changed_by: ChangedBy) {}
}

/// Builds the service, projects the files and the local providers in the
/// background, subscribes to agent changes, and takes the watcher's
/// `.nandi/capabilities/` changes. Doesn't block.
pub fn init(state: &AppState) -> Option<Arc<CapabilityService>> {
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Capabilities disabled: no instance ID ({e})");
            return None;
        }
    };
    let repo = CapabilityRepository::new(state.store().repos().layout().clone());
    let service = Arc::new(
        CapabilityService::new(state.db().clone(), state.bus().clone(), repo)
            .with_instance_id(instance_id)
            .with_store(state.store().clone())
            .with_registries(state.capability_registries().cloned())
            .with_tools(state.tools().cloned()),
    );
    state.store().subscribe(Arc::new(ProviderRefresher {
        service: Arc::downgrade(&service),
        runtime: state.runtime().clone(),
    }));
    let weak = Arc::downgrade(&service);
    let runtime = state.runtime().clone();
    watcher::set_capability_handler(
        &state.store().layout().root,
        Arc::new(move |paths: Vec<PathBuf>| {
            let Some(service) = weak.upgrade() else {
                return;
            };
            runtime.spawn(async move {
                for path in paths {
                    if let Err(e) = service.reload_entity(&path).await {
                        log::debug!("Capability reload of {}: {e}", path.display());
                    }
                }
            });
        }),
    );
    let started = service.clone();
    let identity = state.clone();
    state.runtime().spawn(async move {
        refresh_owner(&identity, &started).await;
        if let Err(e) = started.rebuild().await {
            log::error!("Failed to project the capability files: {e}");
        }
        if let Err(e) = started.refresh_local_providers().await {
            log::error!("Failed to register the local capability providers: {e}");
        }
    });
    Some(service)
}

/// Picks up the user's slug (it can change after onboarding)
async fn refresh_owner(state: &AppState, service: &CapabilityService) {
    let slug = auth::get_identity_status(state)
        .await
        .ok()
        .and_then(|status| status.user_slug);
    service.set_owner_ids(slug.into_iter().collect());
}

fn service(state: &AppState) -> Result<&Arc<CapabilityService>, String> {
    state
        .capabilities()
        .ok_or_else(|| "Capabilities aren't available".to_string())
}

// Transport entry points (commands/capabilities.rs, http/capabilities.rs)

pub async fn list_capabilities(state: &AppState) -> Result<Vec<CapabilityListing>, String> {
    let service = service(state)?;
    refresh_owner(state, service).await;
    service.list_listings().await
}

pub async fn get_capability(state: &AppState, id: &str) -> Result<CapabilityDefinition, String> {
    service(state)?.get(id).await
}

pub async fn save_capability(
    state: &AppState,
    definition: CapabilityDefinition,
) -> Result<CapabilityDefinition, String> {
    let service = service(state)?;
    refresh_owner(state, service).await;
    service.save(&definition).await
}

pub async fn delete_capability(state: &AppState, id: &str) -> Result<(), String> {
    service(state)?.delete(id).await
}

pub async fn transfer_capability_ownership(
    state: &AppState,
    id: &str,
    new_owner: CapabilityOwner,
    reason: Option<String>,
) -> Result<CapabilityDefinition, String> {
    service(state)?
        .transfer_ownership(id, new_owner, reason)
        .await
}

pub async fn list_providers_for_capability(
    state: &AppState,
    id: &str,
) -> Result<Vec<CapabilityProvider>, String> {
    service(state)?.list_providers(id).await
}

/// `window_days`: `None` is 30
pub async fn get_capability_evidence(
    state: &AppState,
    id: &str,
    window_days: Option<u32>,
) -> Result<Vec<CapabilityEvidence>, String> {
    service(state)?
        .evidence(id, window_days.unwrap_or(DEFAULT_WINDOW_DAYS))
        .await
}

pub async fn get_capability_provider_breakdown(
    state: &AppState,
    id: &str,
) -> Result<Vec<CapabilityProviderStats>, String> {
    service(state)?.provider_breakdown(id).await
}

/// Track F. Reads the clusters straight from SQLite, so it works before
/// the capability service is up.
pub async fn list_capability_ambiguities(
    state: &AppState,
    id: &str,
) -> Result<Vec<AmbiguityCluster>, String> {
    let id = id.to_string();
    state
        .db()
        .call(move |conn| aggregation::list_ambiguities(conn, &id))
        .await
}

/// Track F
pub async fn promote_ambiguity_to_schema(
    state: &AppState,
    id: &str,
    cluster_id: &str,
) -> Result<CapabilityDefinition, String> {
    service(state)?
        .promote_ambiguity_to_schema(id, cluster_id)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;
    use crate::services::repos::RepoLayout;
    use std::time::Duration;

    const TEST_CAP: &str = "id: test-cap
name: Test capability
description: For tests
version: 1.0.0
owner: { kind: user, id: me, display_name: Me, transferable: true }
required_tools: [shell]
";

    fn service() -> (CapabilityService, Arc<RecordingSink>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepoLayout::new(tmp.path().join("repo"), tmp.path().join("trash"));
        let sink = Arc::new(RecordingSink::default());
        let db = Db::in_memory();
        let bus = EventBus::new(db.clone(), sink.clone());
        let service = CapabilityService::new(db, bus, CapabilityRepository::new(layout))
            .with_instance_id("local-instance");
        service.set_owner_ids(vec!["me".into()]);
        (service, sink, tmp)
    }

    fn names(sink: &RecordingSink) -> Vec<&'static str> {
        sink.take().into_iter().map(|(name, _)| name).collect()
    }

    async fn wait_until<F, Fut>(what: &str, mut check: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..400 {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn a_file_written_by_hand_is_projected_and_announced() {
        let (service, sink, _tmp) = service();
        let path = service.repo().path("test-cap");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, TEST_CAP).unwrap();
        service.reload_entity(&path).await.unwrap();
        assert_eq!(
            service.get("test-cap").await.unwrap().name,
            "Test capability"
        );
        assert_eq!(names(&sink), [CAPABILITY_CREATED]);

        // Same file: nothing to announce
        service.reload_entity(&path).await.unwrap();
        assert!(names(&sink).is_empty());

        std::fs::write(&path, TEST_CAP.replace("For tests", "Edited")).unwrap();
        service.reload_entity(&path).await.unwrap();
        assert_eq!(service.get("test-cap").await.unwrap().description, "Edited");
        assert_eq!(names(&sink), [CAPABILITY_UPDATED]);

        // An invalid edit keeps the last good version
        std::fs::write(&path, "id: test-cap\nversion: nope\n").unwrap();
        assert!(service.reload_entity(&path).await.is_err());
        assert_eq!(service.get("test-cap").await.unwrap().description, "Edited");

        std::fs::remove_file(&path).unwrap();
        service.reload_entity(&path).await.unwrap();
        assert!(service.get("test-cap").await.is_err());
        assert_eq!(names(&sink), [CAPABILITY_DELETED]);
    }

    #[tokio::test]
    async fn save_fills_owner_and_times_and_delete_clears_everything() {
        let (service, sink, _tmp) = service();
        let mut draft = definition::parse(TEST_CAP).unwrap();
        draft.owner = CapabilityOwner::default();
        let saved = service.save(&draft).await.unwrap();
        assert_eq!(saved.owner.id, "me");
        assert_eq!(saved.owner.kind, OwnerKind::User);
        let created = saved.created_at.unwrap();
        assert!(service.repo().path("test-cap").is_file());
        assert_eq!(names(&sink), [CAPABILITY_CREATED]);

        let again = service
            .save(&CapabilityDefinition {
                description: "v2".into(),
                created_at: None,
                ..saved
            })
            .await
            .unwrap();
        assert_eq!(again.created_at, Some(created), "created_at is kept");
        assert_eq!(names(&sink), [CAPABILITY_UPDATED]);

        let listings = service.list_listings().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert!(listings[0].owned);
        assert_eq!(listings[0].source, CapabilitySource::Local);

        let mut bad = again.clone();
        bad.id = "Bad ID".into();
        assert!(service.save(&bad).await.is_err());

        service.delete("test-cap").await.unwrap();
        assert!(!service.repo().path("test-cap").exists());
        assert!(projection::list(service.db()).await.unwrap().is_empty());
        assert_eq!(names(&sink), [CAPABILITY_DELETED]);
        assert!(service.delete("test-cap").await.is_err());
    }

    #[tokio::test]
    async fn transfer_appends_history_writes_the_file_and_announces() {
        let (service, sink, _tmp) = service();
        service
            .save(&definition::parse(TEST_CAP).unwrap())
            .await
            .unwrap();
        sink.take();
        let team = CapabilityOwner {
            kind: OwnerKind::Team,
            id: "research".into(),
            display_name: "Research".into(),
            transferable: true,
            history: Vec::new(),
        };
        let moved = service
            .transfer_ownership("test-cap", team.clone(), Some("Reorg".into()))
            .await
            .unwrap();
        assert_eq!(moved.owner.id, "research");
        assert_eq!(moved.owner.history.len(), 1);
        assert_eq!(moved.owner.history[0].from, "me");
        assert_eq!(moved.owner.history[0].to, "research");
        assert_eq!(moved.owner.history[0].reason.as_deref(), Some("Reorg"));
        // On disk too
        let on_disk = service.repo().load("test-cap").unwrap();
        assert_eq!(on_disk.owner, moved.owner);
        let events = sink.take();
        let transferred: Vec<_> = events
            .iter()
            .filter(|(name, _)| *name == CAPABILITY_OWNERSHIP_TRANSFERRED)
            .collect();
        assert_eq!(transferred.len(), 1);
        assert_eq!(transferred[0].1["to"]["id"], "research");
        assert_eq!(transferred[0].1["reason"], "Reorg");
        assert!(!service.list_listings().await.unwrap()[0].owned);

        // Again to the same owner, to nobody, or of something missing
        assert!(service
            .transfer_ownership("test-cap", team, None)
            .await
            .is_err());
        assert!(service
            .transfer_ownership("test-cap", CapabilityOwner::default(), None)
            .await
            .is_err());
        assert!(service
            .transfer_ownership("missing", CapabilityOwner::default(), None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn untransferable_capabilities_stay_put() {
        let (service, _sink, _tmp) = service();
        let def = definition::parse(&TEST_CAP.replace(", transferable: true", "")).unwrap();
        service.save(&def).await.unwrap();
        let err = service
            .transfer_ownership(
                "test-cap",
                CapabilityOwner {
                    id: "bob".into(),
                    ..CapabilityOwner::default()
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(err.contains("transferable"), "{err}");
    }

    #[tokio::test]
    async fn rebuild_projects_every_file_and_drops_removed_ones() {
        let (service, sink, _tmp) = service();
        let dir = service.repo().dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test-cap.yaml"), TEST_CAP).unwrap();
        std::fs::write(
            dir.join("other.yaml"),
            TEST_CAP.replace("id: test-cap", "id: other"),
        )
        .unwrap();
        service.rebuild().await.unwrap();
        assert_eq!(service.list_all().await.unwrap().len(), 2);
        assert_eq!(names(&sink), [CAPABILITY_CREATED, CAPABILITY_CREATED]);
        std::fs::remove_file(dir.join("other.yaml")).unwrap();
        // The folder path itself means "re-scan"
        service.reload_entity(&dir).await.unwrap();
        let ids: Vec<String> = service
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, ["test-cap"]);
        assert_eq!(names(&sink), [CAPABILITY_DELETED]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agents_that_provide_a_capability_are_its_providers() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let layout = state.store().layout().clone();
        let skill = layout.skill_path("reviewer");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            &skill,
            "---\nname: Reviewer\nallowed-tools: [shell, read_file]\nprovides: [test-cap]\n---\nReview.",
        )
        .unwrap();
        std::fs::create_dir_all(layout.capabilities_dir()).unwrap();
        std::fs::write(layout.capability_path("test-cap"), TEST_CAP).unwrap();
        state.store().load().unwrap();
        let service = state.capabilities().expect("capability service").clone();
        service.rebuild().await.unwrap();
        service.refresh_local_providers().await.unwrap();

        let providers = service.list_providers("test-cap").await.unwrap();
        assert_eq!(providers.len(), 1);
        let reviewer = &providers[0];
        assert_eq!(reviewer.provider_id, "reviewer");
        assert_eq!(reviewer.provider_kind, ProviderKind::Agent);
        assert_eq!(reviewer.display_name.as_deref(), Some("Reviewer"));
        assert!(reviewer.local);
        assert_eq!(reviewer.capability_version.as_deref(), Some("1.0.0"));
        assert_eq!(
            reviewer.provided_tools,
            ["builtin.read_file", "builtin.shell"]
        );
        // The event bus delivers in the background; a slow CI runner can
        // check before it arrives
        wait_until("the provider registered event", || {
            let recorder = recorder.clone();
            async move { recorder.names().contains(&PROVIDER_REGISTERED) }
        })
        .await;

        let listings = list_capabilities(&state).await.unwrap();
        let listing = listings
            .iter()
            .find(|l| l.definition.id == "test-cap")
            .unwrap();
        assert!(listing.provided);
        assert_eq!(listing.local_providers, 1);
        let breakdown = service.provider_breakdown("test-cap").await.unwrap();
        assert_eq!(breakdown.len(), 1);
        assert!(breakdown[0].online);
        assert_eq!(breakdown[0].evidence.count, 0);

        // The agent stops providing it: its row goes
        std::fs::write(&skill, "---\nname: Reviewer\n---\nReview.").unwrap();
        state.store().load().unwrap();
        service.refresh_local_providers().await.unwrap();
        assert!(service.list_providers("test-cap").await.unwrap().is_empty());
        assert!(recorder.names().contains(&PROVIDER_UNREGISTERED));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_watcher_routes_capability_files_to_the_service() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let layout = state.store().layout().clone();
        std::fs::create_dir_all(layout.capabilities_dir()).unwrap();
        state.store().load().unwrap();
        let _watcher = watcher::spawn(
            state.store().clone(),
            state.bus().clone(),
            state.live().feed().clone(),
            state.runtime(),
        )
        .unwrap();
        std::fs::write(layout.capability_path("test-cap"), TEST_CAP).unwrap();
        let service = state.capabilities().unwrap().clone();
        wait_until("the projected capability", || {
            let service = service.clone();
            async move { service.get("test-cap").await.is_ok() }
        })
        .await;
        wait_until("capability://created", || {
            let seen = recorder.names().contains(&CAPABILITY_CREATED);
            async move { seen }
        })
        .await;
    }

    #[tokio::test]
    async fn listings_include_provider_only_capabilities() {
        let (service, _sink, _tmp) = service();
        service
            .db()
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO capability_providers (capability_id, instance_id, provider_kind,
                        provider_id) VALUES
                        ('remote-only', 'other-instance', 'agent', 'helper'),
                        ('served-here', 'local-instance', 'agent', 'writer')",
                )
            })
            .await
            .unwrap();
        let listings = service.list_listings().await.unwrap();
        let by_id: HashMap<&str, &CapabilityListing> = listings
            .iter()
            .map(|l| (l.definition.id.as_str(), l))
            .collect();
        assert_eq!(by_id["remote-only"].source, CapabilitySource::Network);
        assert!(!by_id["remote-only"].provided);
        assert_eq!(by_id["remote-only"].online_providers, 1);
        assert_eq!(by_id["served-here"].source, CapabilitySource::Local);
        assert!(by_id["served-here"].provided);
        let providers = service.list_providers("remote-only").await.unwrap();
        assert!(!providers[0].local);
        assert_eq!(providers[0].display_name, None);
    }
}
