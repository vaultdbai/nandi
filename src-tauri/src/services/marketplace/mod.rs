//! The marketplace: other people's git repos, vendored into the gold copy.
//!
//! It is a distribution layer, not a new store. Registries (`registry`) list
//! what's available; the installer (`install`) fetches a source and writes
//! it under `.nandi/agents/marketplace/`, records it in `.nandi/installed.yaml`,
//! and re-scans. From there an installed agent is an agent like any other,
//! except read-only until the user forks it.
//!
//! Rules:
//! - Nothing updates on its own: "Check for updates" only reports.
//! - An entry whose `min_nandi_version` is newer than this app is never
//!   installed.
//! - Tokens for private registries live in the keyring; settings.yaml and
//!   installed.yaml hold references and repo locations only.
//!
//! See `docs/developer/marketplace.md`.

pub mod community;
pub mod connectors;
pub mod consent;
pub mod events;
pub mod github;
pub mod install;
pub mod local;
pub mod manifest;
pub mod publish;
pub mod registry;
// Wave 2B redesign (Track C)
#[allow(dead_code)]
pub mod search;
pub mod signature;
pub mod source;

use std::path::PathBuf;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use specta::Type;

use self::events::*;
use self::install::{
    AppliedUpdate, InstalledAgentInfo, InstalledItem, InstalledPackInfo, Installer, UpdateAvailable,
};
use self::local::LocalPackSource;
use self::manifest::{AgentManifest, ManifestCapabilities, PackManifest};
use self::registry::{Registry, RegistryClient, RegistryIndex, RegistryList};
use self::source::{FetchProgress, GitSource};
use crate::services::agents::{self, AgentInfo};
use crate::services::auth::github as github_sign_in;
use crate::services::domain::installed::{self, InstallKind};
use crate::services::domain::settings::{
    normalize_registry_url, RegistryConfig, RegistryKind, SecretRef, DEFAULT_REGISTRY,
};
use crate::services::domain::EntityKind;
use crate::services::events::emit;
use crate::services::repos::SingletonRepository;
use crate::services::AppState;

/// A registry with this `auth` uses the signed-in GitHub account.
const GITHUB_SIGN_IN_REF: &str = github_sign_in::TOKEN_ENTRY;

/// (registry, publisher) → (when fetched, their public key)
type PublisherKeys = std::collections::HashMap<(String, String), (Instant, Option<String>)>;

/// Marketplace state shared by every request.
pub struct Marketplace {
    client: RegistryClient,
    /// Scratch repos for fetching sources (outside the gold copy)
    scratch_dir: PathBuf,
    /// One install / uninstall / fork / update at a time: they read and
    /// rewrite installed.yaml
    writer: tokio::sync::Mutex<()>,
    /// (registry, publisher) → (when fetched, their public key)
    publisher_keys: std::sync::Mutex<PublisherKeys>,
}

impl Marketplace {
    /// `cache_dir` is `~/.nandi/local/cache`.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            client: RegistryClient::new(&cache_dir),
            scratch_dir: cache_dir.join("checkouts"),
            writer: tokio::sync::Mutex::new(()),
            publisher_keys: std::sync::Mutex::default(),
        }
    }
}

/// Filters for the Discover tab.
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct MarketplaceFilter {
    /// Matches name, description, publisher, and tags (case-insensitive)
    pub query: Option<String>,
    /// Entries with every one of these tags
    pub tags: Vec<String>,
    pub verified_only: bool,
    pub installed_only: bool,
    /// Ignore the cache TTL and ask the registries again
    pub refresh: bool,
}

/// A configured registry, for the Registries tab.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegistryInfo {
    pub url: String,
    /// public | community | personal
    pub kind: String,
    /// Uses a token (its own, or the GitHub sign-in)
    pub has_auth: bool,
    pub is_default: bool,
    pub agent_count: Option<u32>,
    pub pack_count: Option<u32>,
    /// RFC 3339, when the index was last fetched
    pub fetched_at: Option<String>,
    /// Why it couldn't be read, or a staleness warning
    pub warning: Option<String>,
}

/// Input for adding a registry.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddRegistryInput {
    pub url: String,
    /// public | community | personal
    pub kind: String,
    /// A GitHub token for a private registry (stored in the keyring)
    #[serde(default)]
    pub token: Option<String>,
    /// Use the signed-in GitHub account instead of a token
    #[serde(default)]
    pub use_github_sign_in: bool,
}

/// A release of an agent's source.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version: String,
    pub git_ref: String,
    pub commit: String,
}

/// Other agents by the same publisher.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublisherInfo {
    pub handle: String,
    /// `https://github.com/<handle>`
    pub profile_url: String,
    pub agents: Vec<AgentSummary>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
}

/// The detail modal: README, versions, capabilities, publisher.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentDetail {
    pub manifest: AgentManifest,
    /// Markdown; `None` if the source has no README
    pub readme: Option<String>,
    /// Newest first; empty if the source couldn't be reached
    pub versions: Vec<VersionInfo>,
    pub capabilities: ManifestCapabilities,
    pub publisher: PublisherInfo,
    /// `https://github.com/...` for the source
    pub source_url: Option<String>,
}

/// The detail modal for a pack.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackDetail {
    pub manifest: PackManifest,
    pub readme: Option<String>,
    pub versions: Vec<VersionInfo>,
    pub source_url: Option<String>,
}

/// A fork, and anything that still names the original.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForkedAgent {
    pub agent: AgentInfo,
    pub warnings: Vec<String>,
}

// -- Helpers ------------------------------------------------------------------

fn installer(state: &AppState) -> Installer {
    Installer::new(
        state.store().clone(),
        state.marketplace().scratch_dir.clone(),
    )
}

fn registries(state: &AppState) -> RegistryList {
    let configs = state.store().read(|repo| repo.settings().registries());
    RegistryList::from_configs(&configs)
}

/// The token for a registry (its own keyring entry, or the GitHub
/// sign-in). Never leaves Rust.
fn token(state: &AppState, registry: &Registry) -> Option<String> {
    registry.auth.as_ref().and_then(|auth| secret(state, auth))
}

/// What a registry's `auth` points at: the GitHub sign-in, or the
/// registry's own keyring entry
fn secret(state: &AppState, auth: &SecretRef) -> Option<String> {
    if auth.as_str() == GITHUB_SIGN_IN_REF {
        github_sign_in::token(state.secrets())
    } else {
        state.secrets().get(auth.as_str()).ok().flatten()
    }
}

fn token_for_url(state: &AppState, registry_url: Option<&str>) -> Option<String> {
    let list = registries(state);
    registry_url
        .and_then(|url| list.find(url))
        .and_then(|registry| token(state, registry))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("Marketplace task failed: {e}"))?
}

fn report<T>(
    state: &AppState,
    action: &str,
    id: &str,
    result: Result<T, String>,
) -> Result<T, String> {
    if let Err(error) = &result {
        log::warn!("Marketplace {action} '{id}' failed: {error}");
        emit(
            state.bus(),
            &MarketplaceError {
                action: action.to_string(),
                id: id.to_string(),
                error: error.clone(),
            },
        );
    }
    result
}

/// Emits fetch progress at most every 200 ms (and the final count).
fn progress_emitter(
    state: &AppState,
    kind: InstallKind,
    id: &str,
) -> impl FnMut(FetchProgress) + Send + 'static {
    let bus = state.bus().clone();
    let (kind, id) = (kind.as_str().to_string(), id.to_string());
    let mut last = Instant::now() - Duration::from_secs(1);
    move |progress: FetchProgress| {
        let done =
            progress.total_objects > 0 && progress.received_objects == progress.total_objects;
        if !done && last.elapsed() < Duration::from_millis(200) {
            return;
        }
        last = Instant::now();
        emit(
            &bus,
            &MarketplaceProgress {
                kind: kind.clone(),
                id: id.clone(),
                received_objects: u32::try_from(progress.received_objects).unwrap_or(u32::MAX),
                total_objects: u32::try_from(progress.total_objects).unwrap_or(u32::MAX),
            },
        );
    }
}

/// Every registry's index (cached within its TTL). A registry that fails
/// is left out, with why.
async fn indexes(state: &AppState, force: bool) -> (Vec<RegistryIndex>, Vec<(String, String)>) {
    let list = registries(state);
    let client = &state.marketplace().client;
    let fetches = list.registries.iter().map(|registry| {
        let token = token(state, registry);
        async move {
            let result = client.fetch_index(registry, token.as_deref(), force).await;
            (registry.url.clone(), result)
        }
    });
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for (url, result) in futures_util::future::join_all(fetches).await {
        match result {
            Ok(index) => ok.push(index),
            Err(e) => failed.push((url, e)),
        }
    }
    (ok, failed)
}

/// First-party packs and agents carry this tag (onboarding ticks them)
const OFFICIAL_TAG: &str = "official";

/// Verified official entries first, the rest in registry order, so the
/// first-party pack is the first card in Library and in onboarding.
fn official_first<T>(
    items: &mut [T],
    tags: impl Fn(&T) -> &[String],
    verified: impl Fn(&T) -> bool,
) {
    items.sort_by_key(|item| {
        let official = verified(item)
            && tags(item)
                .iter()
                .any(|t| t.eq_ignore_ascii_case(OFFICIAL_TAG));
        !official
    });
}

fn matches_filter(
    filter: &MarketplaceFilter,
    fields: [&str; 3],
    tags: &[String],
    verified: bool,
    installed: bool,
) -> bool {
    if filter.verified_only && !verified || filter.installed_only && !installed {
        return false;
    }
    if !filter
        .tags
        .iter()
        .all(|wanted| tags.iter().any(|t| t.eq_ignore_ascii_case(wanted)))
    {
        return false;
    }
    match filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        None => true,
        Some(query) => {
            let query = query.to_lowercase();
            fields.iter().any(|f| f.to_lowercase().contains(&query))
                || tags.iter().any(|t| t.to_lowercase().contains(&query))
        }
    }
}

// -- Catalog --------------------------------------------------------------------

/// Every pack across the registries (deduplicated: latest version wins),
/// with what's installed. Used by onboarding's starter packs.
/// Every agent and pack across the registries: deduplicated (latest
/// version wins), signatures verified, and install state filled in.
struct Catalog {
    agents: Vec<AgentManifest>,
    packs: Vec<PackManifest>,
    /// Registries that couldn't be read: (url, why)
    failed: Vec<(String, String)>,
}

async fn catalog(state: &AppState, force: bool) -> Catalog {
    let (indexes, failed) = indexes(state, force).await;
    let mut agents = RegistryList::merge_agents(&indexes);
    let mut packs = RegistryList::merge_packs(&indexes);
    verify_signatures(state, &mut agents, &mut packs).await;
    let (installed_packs, installed_agents) = installer(state).installed_versions();
    let version_of = |list: &[(String, String)], id: &str| {
        list.iter().find(|(i, _)| i == id).map(|(_, v)| v.clone())
    };
    for agent in &mut agents {
        agent.installed_version = version_of(&installed_agents, &agent.id);
    }
    for pack in &mut packs {
        pack.installed_version = version_of(&installed_packs, &pack.id);
    }
    Catalog {
        agents,
        packs,
        failed,
    }
}

/// How long a fetched publisher key is trusted without asking again
const PUBLISHER_TTL: Duration = Duration::from_secs(60 * 60);

/// The publisher's public key from `publishers/<handle>.json` in
/// `registry`, or `None` if they don't sign or it couldn't be fetched. Only
/// a fetched file is cached: a fetch that failed is tried again next time,
/// so one timeout doesn't leave signed entries unverified for an hour.
async fn publisher_key(state: &AppState, registry: &Registry, handle: &str) -> Option<String> {
    let key = (registry.url.clone(), handle.to_string());
    if let Some((fetched, value)) = state
        .marketplace()
        .publisher_keys
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned()
    {
        if fetched.elapsed() < PUBLISHER_TTL {
            return value;
        }
    }
    let token = token(state, registry);
    let fetched = state
        .marketplace()
        .client
        .fetch_file(
            registry,
            &format!("publishers/{handle}.json"),
            token.as_deref(),
        )
        .await;
    let bytes = match fetched {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!(
                "Couldn't fetch publisher '{handle}' from {}: {e}",
                registry.url
            );
            return None;
        }
    };
    let value = publisher_file_key(&bytes, handle);
    state
        .marketplace()
        .publisher_keys
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, (Instant::now(), value.clone()));
    value
}

/// The public key in a fetched `publishers/<handle>.json`, if it's that
/// publisher's file and names one
fn publisher_file_key(bytes: &[u8], handle: &str) -> Option<String> {
    serde_json::from_slice::<manifest::PublisherFile>(bytes)
        .ok()
        .filter(|file| file.github_handle.eq_ignore_ascii_case(handle))
        .and_then(|file| file.public_key)
}

/// Marks entries whose signature verifies against their publisher's key
/// in the registry they came from.
async fn verify_signatures(
    state: &AppState,
    agents: &mut [AgentManifest],
    packs: &mut [PackManifest],
) {
    let list = registries(state);
    let mut keys: std::collections::HashMap<(String, String), Option<String>> =
        std::collections::HashMap::new();
    let wanted = agents
        .iter()
        .filter(|a| a.signature.is_some() && a.commit.is_some())
        .map(|a| (a.registry.clone(), a.publisher.clone()))
        .chain(
            packs
                .iter()
                .filter(|p| p.signature.is_some() && p.commit.is_some())
                .map(|p| (p.registry.clone(), p.publisher.clone())),
        )
        .collect::<std::collections::BTreeSet<_>>();
    for (registry_url, handle) in wanted {
        let Some(registry) = list.find(&registry_url) else {
            continue;
        };
        let key = publisher_key(state, registry, &handle).await;
        keys.insert((registry_url, handle), key);
    }
    let check = |registry: &str,
                 publisher: &str,
                 signature: &Option<String>,
                 commit: &Option<String>,
                 kind: &str,
                 id: &str,
                 version: &str| {
        match (
            keys.get(&(registry.to_string(), publisher.to_string())),
            signature,
            commit,
        ) {
            (Some(Some(key)), Some(signature), Some(commit)) => {
                signature::verify(key, signature, kind, id, version, commit)
            }
            _ => false,
        }
    };
    for agent in agents.iter_mut() {
        agent.verified = check(
            &agent.registry,
            &agent.publisher,
            &agent.signature,
            &agent.commit,
            "agent",
            &agent.id,
            &agent.latest_version,
        );
    }
    for pack in packs.iter_mut() {
        pack.verified = check(
            &pack.registry,
            &pack.publisher,
            &pack.signature,
            &pack.commit,
            "pack",
            &pack.id,
            &pack.latest_version,
        );
    }
}

fn first_failure(state: &AppState, failed: Vec<(String, String)>) -> Result<(), String> {
    match failed.into_iter().next() {
        Some((url, error)) => {
            Err(report(state, "registry", &url, Err::<(), _>(error)).unwrap_err())
        }
        None => Ok(()),
    }
}

/// Every pack across the registries, with what's installed. Used by
/// onboarding's starter packs.
pub async fn list_available_packs(state: &AppState) -> Result<Vec<PackManifest>, String> {
    list_packs(state, &MarketplaceFilter::default()).await
}

pub async fn list_packs(
    state: &AppState,
    filter: &MarketplaceFilter,
) -> Result<Vec<PackManifest>, String> {
    let Catalog {
        agents,
        mut packs,
        failed,
    } = catalog(state, filter.refresh).await;
    // Packs installed from a folder or `.zip`: no registry lists them
    let installer = installer(state);
    let local = blocking(move || installer.installed_local_packs()).await?;
    let listed: Vec<String> = packs.iter().map(|p| p.id.clone()).collect();
    packs.extend(local.into_iter().filter(|p| !listed.contains(&p.id)));
    if agents.is_empty() && packs.is_empty() {
        first_failure(state, failed)?;
    }
    packs.retain(|p| {
        matches_filter(
            filter,
            [&p.name, &p.description, &p.publisher],
            &p.tags,
            p.verified,
            p.installed_version.is_some(),
        )
    });
    official_first(&mut packs, |p| &p.tags, |p| p.verified);
    Ok(packs)
}

/// Every agent across the registries, filtered.
pub async fn list_available_agents(
    state: &AppState,
    filter: &MarketplaceFilter,
) -> Result<Vec<AgentManifest>, String> {
    let Catalog {
        mut agents,
        packs,
        failed,
    } = catalog(state, filter.refresh).await;
    if agents.is_empty() && packs.is_empty() {
        first_failure(state, failed)?;
    }
    agents.retain(|a| {
        matches_filter(
            filter,
            [&a.name, &a.description, &a.publisher],
            &a.tags,
            a.verified,
            a.installed_version.is_some(),
        )
    });
    official_first(&mut agents, |a| &a.tags, |a| a.verified);
    Ok(agents)
}

async fn find_pack(state: &AppState, pack_id: &str) -> Result<PackManifest, String> {
    let Catalog { packs, failed, .. } = catalog(state, false).await;
    packs
        .into_iter()
        .find(|p| p.id == pack_id)
        .ok_or_else(|| match failed.first() {
            Some((url, e)) => format!("Pack '{pack_id}' not found ({url}: {e})"),
            None => format!("Pack '{pack_id}' isn't in any registry"),
        })
}

async fn find_agent(state: &AppState, agent_id: &str) -> Result<AgentManifest, String> {
    let Catalog { agents, failed, .. } = catalog(state, false).await;
    agents
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| match failed.first() {
            Some((url, e)) => format!("Agent '{agent_id}' not found ({url}: {e})"),
            None => format!("Agent '{agent_id}' isn't in any registry"),
        })
}

/// README, versions, capability breakdown, and publisher of an agent.
/// The registry an entry came from (or a public stand-in if it was removed).
fn registry_of(state: &AppState, url: &str) -> Registry {
    registries(state).find(url).cloned().unwrap_or(Registry {
        url: url.to_string(),
        kind: RegistryKind::Public,
        auth: None,
    })
}

/// A README by its entry's URL, or `<repo>/<folder>/README.md` on GitHub.
async fn fetch_readme(
    state: &AppState,
    registry: &Registry,
    readme_url: Option<String>,
    repo: &str,
    folder: Option<&str>,
) -> Option<String> {
    let url = readme_url.or_else(|| {
        repo.strip_prefix("github.com/").map(|path| match folder {
            Some(folder) => {
                format!("https://raw.githubusercontent.com/{path}/HEAD/{folder}/README.md")
            }
            None => format!("https://raw.githubusercontent.com/{path}/HEAD/README.md"),
        })
    })?;
    let token = token(state, registry);
    state
        .marketplace()
        .client
        .fetch_asset(registry, &url, token.as_deref())
        .await
        .inspect_err(|e| log::debug!("No README at {url}: {e}"))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// A source's release tags with the same prefix as `git_ref`, newest first
/// (empty if unreachable).
async fn fetch_versions(repo: &str, git_ref: &str, token: Option<String>) -> Vec<VersionInfo> {
    let source = GitSource::new(repo, token);
    let prefix = manifest::split_tag(git_ref)
        .map(|(p, _)| p)
        .unwrap_or_default();
    blocking(move || source.release_tags(&prefix))
        .await
        .inspect_err(|e| {
            log::debug!(
                "No versions for {}: {e}",
                source::web_url(repo).unwrap_or_default()
            )
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(version, r)| VersionInfo {
            version: version.to_string(),
            git_ref: r.name.trim_start_matches("refs/tags/").to_string(),
            commit: r.commit,
        })
        .collect()
}

/// README, versions, capability breakdown, and publisher of an agent.
pub async fn get_agent_detail(state: &AppState, agent_id: &str) -> Result<AgentDetail, String> {
    let manifest = find_agent(state, agent_id).await?;
    let registry = registry_of(state, &manifest.registry);
    let readme = fetch_readme(
        state,
        &registry,
        manifest.readme_url.clone(),
        &manifest.repo,
        Some(&manifest.path),
    )
    .await;
    let versions = fetch_versions(&manifest.repo, &manifest.git_ref, token(state, &registry)).await;
    let others = catalog(state, false)
        .await
        .agents
        .into_iter()
        .filter(|a| a.publisher == manifest.publisher && a.id != manifest.id)
        .map(|a| AgentSummary {
            id: a.id,
            name: a.name,
        })
        .collect();
    Ok(AgentDetail {
        capabilities: manifest.capabilities.clone(),
        publisher: PublisherInfo {
            profile_url: format!("https://github.com/{}", manifest.publisher),
            handle: manifest.publisher.clone(),
            agents: others,
        },
        source_url: source::web_url(&manifest.repo),
        readme,
        versions,
        manifest,
    })
}

/// README and versions of a pack.
pub async fn get_pack_detail(state: &AppState, pack_id: &str) -> Result<PackDetail, String> {
    let manifest = find_pack(state, pack_id).await?;
    let registry = registry_of(state, &manifest.registry);
    let readme = fetch_readme(
        state,
        &registry,
        manifest.readme_url.clone(),
        &manifest.repo,
        None,
    )
    .await;
    let versions = fetch_versions(&manifest.repo, &manifest.git_ref, token(state, &registry)).await;
    Ok(PackDetail {
        source_url: source::web_url(&manifest.repo),
        readme,
        versions,
        manifest,
    })
}

// -- Install, uninstall, fork, update ---------------------------------------------

pub async fn install_pack(state: &AppState, pack_id: &str) -> Result<InstalledPackInfo, String> {
    let result = async {
        let manifest = find_pack(state, pack_id).await?;
        let token = token_for_url(state, Some(&manifest.registry));
        let _writer = state.marketplace().writer.lock().await;
        let installer = installer(state);
        let mut progress = progress_emitter(state, InstallKind::Pack, pack_id);
        let source = manifest.repo.clone();
        let info =
            blocking(move || installer.install_pack(&manifest, token, &mut progress)).await?;
        emit(
            state.bus(),
            &MarketplaceInstalled {
                kind: InstallKind::Pack.as_str().to_string(),
                id: info.id.clone(),
                source,
                version: info.version.clone(),
            },
        );
        for agent in &info.agents {
            state.orchestrators().ensure(agent);
        }
        // S3 Backup saved before its agent arrived has no triggers yet
        if let Err(e) = crate::services::connectors::resync_backup_triggers(state).await {
            log::warn!("S3 Backup triggers after installing {}: {e}", info.id);
        }
        Ok(info)
    }
    .await;
    report(state, "install", pack_id, result)
}

/// What the pack in a folder or `.zip` on this computer would install and
/// request, for the consent dialog before installing it. No network.
pub async fn inspect_local_pack(state: &AppState, path: &str) -> Result<PackManifest, String> {
    let installer = installer(state);
    let path = PathBuf::from(path.trim());
    blocking(move || installer.inspect_local_pack(&LocalPackSource::open(&path)?)).await
}

/// Installs the pack in a folder or `.zip` on this computer, through the
/// same installer as a registry pack (validation, read-only vendoring,
/// consent before its agents run). Unverified: nothing signs a local pack.
/// No network.
pub async fn install_local_pack(state: &AppState, path: &str) -> Result<InstalledPackInfo, String> {
    let path = PathBuf::from(path.trim());
    let label = path.display().to_string();
    let result = async {
        let source = blocking(move || LocalPackSource::open(&path)).await?;
        let _writer = state.marketplace().writer.lock().await;
        let installer = installer(state);
        let from = installed::local_source(source.path());
        let info = blocking(move || installer.install_local_pack(&source)).await?;
        emit(
            state.bus(),
            &MarketplaceInstalled {
                kind: InstallKind::Pack.as_str().to_string(),
                id: info.id.clone(),
                source: from,
                version: info.version.clone(),
            },
        );
        for agent in &info.agents {
            state.orchestrators().ensure(agent);
        }
        if let Err(e) = crate::services::connectors::resync_backup_triggers(state).await {
            log::warn!("S3 Backup triggers after installing {}: {e}", info.id);
        }
        Ok(info)
    }
    .await;
    report(state, "install", &label, result)
}

pub async fn install_agent(state: &AppState, agent_id: &str) -> Result<InstalledAgentInfo, String> {
    let result = async {
        let manifest = find_agent(state, agent_id).await?;
        let token = token_for_url(state, Some(&manifest.registry));
        let _writer = state.marketplace().writer.lock().await;
        let installer = installer(state);
        let mut progress = progress_emitter(state, InstallKind::Agent, agent_id);
        let source = manifest.repo.clone();
        let info =
            blocking(move || installer.install_agent(&manifest, token, &mut progress)).await?;
        emit(
            state.bus(),
            &MarketplaceInstalled {
                kind: InstallKind::Agent.as_str().to_string(),
                id: info.id.clone(),
                source,
                version: info.version.clone(),
            },
        );
        state.orchestrators().ensure(&info.id);
        Ok(info)
    }
    .await;
    report(state, "install", agent_id, result)
}

/// Uninstalls a pack or agent (`kind`: `pack` | `agent`). Refused for
/// forked agents, a pack's single agent, anything in use, or while one of
/// its agents is busy.
pub async fn uninstall(state: &AppState, kind: &str, id: &str) -> Result<(), String> {
    let result = async {
        let kind = InstallKind::parse(kind).map_err(|e| e.to_string())?;
        let _writer = state.marketplace().writer.lock().await;
        let plan_installer = installer(state);
        let owned_id = id.to_string();
        let agents = blocking(move || plan_installer.uninstall_plan(kind, &owned_id)).await?;
        for agent in &agents {
            agents::retire_agent(state, agent).await?;
        }
        let installer = installer(state);
        let owned_id = id.to_string();
        blocking(move || installer.uninstall(kind, &owned_id)).await?;
        let store = state.store().clone();
        blocking(move || {
            for agent in &agents {
                consent::forget(&store, agent)?;
            }
            Ok(())
        })
        .await?;
        emit(
            state.bus(),
            &MarketplaceUninstalled {
                kind: kind.as_str().to_string(),
                id: id.to_string(),
            },
        );
        Ok(())
    }
    .await;
    report(state, "uninstall", id, result)
}

/// Forks an installed agent into the user's own agents as `new_id`.
pub async fn fork_agent(
    state: &AppState,
    installed_id: &str,
    new_id: &str,
    new_name: &str,
) -> Result<ForkedAgent, String> {
    let result = async {
        let _writer = state.marketplace().writer.lock().await;
        agents::retire_agent(state, installed_id).await?;
        let installer = installer(state);
        let (source, target, name) = (
            installed_id.to_string(),
            new_id.to_string(),
            new_name.to_string(),
        );
        let forked = blocking(move || installer.fork_agent(&source, &target, &name)).await?;
        let id = forked.agent.id().to_string();
        state.orchestrators().ensure(&id);
        let store = state.store().clone();
        let info = blocking(move || {
            store
                .agent_infos()
                .into_iter()
                .find(|a| a.id == id)
                .ok_or_else(|| "The fork was created but couldn't be read back".to_string())
        })
        .await?;
        emit(
            state.bus(),
            &MarketplaceForked {
                source_id: installed_id.to_string(),
                new_id: info.id.clone(),
            },
        );
        Ok(ForkedAgent {
            agent: info,
            warnings: forked.warnings,
        })
    }
    .await;
    report(state, "fork", installed_id, result)
}

pub async fn list_installed(state: &AppState) -> Result<Vec<InstalledItem>, String> {
    let installer = installer(state);
    blocking(move || installer.list_installed()).await
}

/// Asks every installed source for newer versions. Reports only.
pub async fn check_for_updates(state: &AppState) -> Result<Vec<UpdateAvailable>, String> {
    let installer = installer(state);
    let tokens: Vec<(String, Option<String>)> = registries(state)
        .registries
        .iter()
        .map(|r| (r.url.clone(), token(state, r)))
        .collect();
    let (updates, failures) = blocking(move || {
        installer.check_for_updates(&|registry| {
            let registry = normalize_registry_url(registry?);
            tokens
                .iter()
                .find(|(url, _)| *url == registry)
                .and_then(|(_, token)| token.clone())
        })
    })
    .await?;
    for (id, error) in failures {
        let _ = report(state, "check_updates", &id, Err::<(), _>(error));
    }
    for update in &updates {
        emit(
            state.bus(),
            &MarketplaceUpdateAvailable {
                kind: update.kind.clone(),
                id: update.id.clone(),
                current_version: update.current_version.clone(),
                latest_version: update.latest_version.clone(),
            },
        );
    }
    Ok(updates)
}

/// Updates an install to `new_ref` (from `check_for_updates`).
pub async fn apply_update(
    state: &AppState,
    installed_id: &str,
    new_ref: &str,
) -> Result<AppliedUpdate, String> {
    let result = async {
        let _writer = state.marketplace().writer.lock().await;
        let installed = state
            .store()
            .repos()
            .vendor
            .load_installed()
            .map_err(String::from)?;
        let registry = installed
            .agent(installed_id)
            .and_then(|a| a.registry.clone())
            .or_else(|| {
                installed
                    .pack(installed_id)
                    .and_then(|p| p.registry.clone())
            });
        let kind = if installed.agent(installed_id).is_some() {
            InstallKind::Agent
        } else {
            InstallKind::Pack
        };
        let token = token_for_url(state, registry.as_deref());
        // The registry's entry for this release, if it's signed and verifies.
        // Updates are found from the source's tags, so a cached index can
        // predate the release: then ask the registry again
        let target = manifest::split_tag(new_ref).map(|(_, v)| v.to_string());
        let signed_commit = |listed: &Catalog| match kind {
            InstallKind::Agent => listed
                .agents
                .iter()
                .find(|a| {
                    a.id == installed_id && a.verified && Some(&a.latest_version) == target.as_ref()
                })
                .and_then(|a| a.commit.clone()),
            InstallKind::Pack => listed
                .packs
                .iter()
                .find(|p| {
                    p.id == installed_id && p.verified && Some(&p.latest_version) == target.as_ref()
                })
                .and_then(|p| p.commit.clone()),
        };
        let verified_commit = match signed_commit(&catalog(state, false).await) {
            Some(commit) => Some(commit),
            None => signed_commit(&catalog(state, true).await),
        };
        let installer = installer(state);
        let mut progress = progress_emitter(state, kind, installed_id);
        let (id, git_ref) = (installed_id.to_string(), new_ref.to_string());
        let applied = blocking(move || {
            installer.apply_update(
                &id,
                &git_ref,
                token,
                verified_commit.as_deref(),
                &mut progress,
            )
        })
        .await?;
        emit(
            state.bus(),
            &MarketplaceUpdateApplied {
                kind: applied.kind.clone(),
                id: applied.id.clone(),
                from_version: applied.from_version.clone(),
                to_version: applied.to_version.clone(),
            },
        );
        Ok(applied)
    }
    .await;
    report(state, "update", installed_id, result)
}

// -- Consent ------------------------------------------------------------------

/// What a marketplace agent requests and what the user has approved.
pub async fn get_agent_consent(
    state: &AppState,
    agent_id: &str,
) -> Result<consent::ConsentStatus, String> {
    let (store, id) = (state.store().clone(), agent_id.to_string());
    blocking(move || consent::status(&store, &id)).await
}

/// Approves the given capability categories for a marketplace agent.
pub async fn grant_agent_consent(
    state: &AppState,
    agent_id: &str,
    categories: Vec<String>,
) -> Result<consent::ConsentStatus, String> {
    let (store, id) = (state.store().clone(), agent_id.to_string());
    blocking(move || consent::grant(&store, &id, &categories)).await
}

// -- Registries -----------------------------------------------------------------

fn kind_of(value: &str) -> Result<RegistryKind, String> {
    RegistryKind::parse(value).map_err(|e| e.to_string())
}

/// The keyring entry for a registry's own token.
fn registry_secret_ref(url: &str) -> Result<SecretRef, String> {
    let hash = crate::services::repos::fs_atomic::content_hash(url.as_bytes());
    SecretRef::parse(format!("nandi.marketplace.{}", &hash[..12])).map_err(|e| e.to_string())
}

async fn save_registries(state: &AppState, configs: Vec<RegistryConfig>) -> Result<(), String> {
    let mut settings = state.store().read(|repo| repo.settings().clone());
    settings
        .set_registries(configs)
        .map_err(|e| e.to_string())?;
    let store = state.store().clone();
    blocking(move || {
        store.repos().settings.save(&settings)?;
        store.commit(EntityKind::Settings, "settings")?;
        Ok(())
    })
    .await
}

pub async fn list_registries(state: &AppState) -> Result<Vec<RegistryInfo>, String> {
    let list = registries(state);
    let client = &state.marketplace().client;
    let fetches = list.registries.iter().map(|registry| {
        let token = token(state, registry);
        async move {
            let result = client.fetch_index(registry, token.as_deref(), false).await;
            (registry, result)
        }
    });
    Ok(futures_util::future::join_all(fetches)
        .await
        .into_iter()
        .map(|(registry, result)| {
            let (agents, packs, fetched_at, warning) = match result {
                Ok(index) => (
                    Some(u32::try_from(index.agents.len()).unwrap_or(u32::MAX)),
                    Some(u32::try_from(index.packs.len()).unwrap_or(u32::MAX)),
                    index
                        .fetched_at
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
                        .map(|t| t.to_rfc3339()),
                    index.warning,
                ),
                Err(e) => (None, None, None, Some(e)),
            };
            RegistryInfo {
                url: registry.url.clone(),
                kind: registry.kind.as_str().to_string(),
                has_auth: registry.auth.is_some(),
                is_default: registry.url == DEFAULT_REGISTRY,
                agent_count: agents,
                pack_count: packs,
                fetched_at,
                warning,
            }
        })
        .collect())
}

/// Adds a registry (and its token, to the keyring). The index is fetched
/// once to check the URL works before it's saved.
pub async fn add_registry(
    state: &AppState,
    input: AddRegistryInput,
) -> Result<RegistryInfo, String> {
    let result = async {
        let url = normalize_registry_url(&input.url);
        let kind = kind_of(&input.kind)?;
        let mut configs = state.store().read(|repo| repo.settings().registries());
        if configs
            .iter()
            .any(|c| normalize_registry_url(&c.url) == url)
        {
            return Err(format!("{url} is already in your registries"));
        }
        let token = input
            .token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let auth = match (&token, input.use_github_sign_in) {
            (Some(_), _) => Some(registry_secret_ref(&url)?),
            (None, true) => {
                if github_sign_in::token(state.secrets()).is_none() {
                    return Err("Sign in to GitHub first (Settings › Account)".to_string());
                }
                Some(SecretRef::parse(GITHUB_SIGN_IN_REF).map_err(|e| e.to_string())?)
            }
            (None, false) => None,
        };
        let config = RegistryConfig::new(&url, kind, auth.clone()).map_err(|e| e.to_string())?;
        // Try it before saving anything
        let probe_token = match (&token, &auth) {
            (Some(token), _) => Some(token.clone()),
            (None, Some(auth)) => secret(state, auth),
            (None, None) => None,
        };
        let registry = Registry::from_config(&config);
        let index = state
            .marketplace()
            .client
            .fetch_index(&registry, probe_token.as_deref(), true)
            .await?;
        if let (Some(token), Some(auth)) = (&token, &auth) {
            state.secrets().set(auth.as_str(), token)?;
        }
        configs.push(config.clone());
        save_registries(state, configs).await?;
        Ok(RegistryInfo {
            url: config.url,
            kind: kind.as_str().to_string(),
            has_auth: auth.is_some(),
            is_default: false,
            agent_count: Some(u32::try_from(index.agents.len()).unwrap_or(u32::MAX)),
            pack_count: Some(u32::try_from(index.packs.len()).unwrap_or(u32::MAX)),
            fetched_at: Some(chrono::Utc::now().to_rfc3339()),
            warning: index.warning,
        })
    }
    .await;
    report(state, "registry", &input.url, result)
}

/// The user's community registry, just created (or found again).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommunityRegistry {
    pub registry: RegistryInfo,
    /// Where the owner adds collaborators on GitHub
    pub invite_url: String,
    /// `false`: it already existed (from an earlier run)
    pub created: bool,
}

/// Creates a private `nandi-community-<login>` repo with the registry
/// layout (or reuses it), and adds it to the registries, read with the
/// signed-in account.
pub async fn create_community_registry(state: &AppState) -> Result<CommunityRegistry, String> {
    let result = async {
        let token = crate::services::auth::require_token(state)?;
        let setup = community::GitHubApi::new("https://api.github.com", token)
            .setup_community_registry()
            .await?;
        let auth = SecretRef::parse(GITHUB_SIGN_IN_REF).map_err(|e| e.to_string())?;
        let mut configs = state.store().read(|repo| repo.settings().registries());
        if !configs
            .iter()
            .any(|c| normalize_registry_url(&c.url) == setup.url)
        {
            configs.push(
                RegistryConfig::new(&setup.url, RegistryKind::Community, Some(auth))
                    .map_err(|e| e.to_string())?,
            );
            save_registries(state, configs).await?;
        }
        Ok(CommunityRegistry {
            registry: RegistryInfo {
                url: setup.url,
                kind: RegistryKind::Community.as_str().to_string(),
                has_auth: true,
                is_default: false,
                agent_count: None,
                pack_count: None,
                fetched_at: None,
                warning: None,
            },
            invite_url: setup.invite_url,
            created: setup.created,
        })
    }
    .await;
    report(state, "registry", "community", result)
}

/// Publishing one of the user's own agents to a registry.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishAgentInput {
    pub agent_id: String,
    /// One of the configured registries (on GitHub)
    pub registry_url: String,
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    pub pr_url: String,
    /// Merged right away (a community registry the user can push to)
    pub merged: bool,
    /// The release tag created in the agents repo
    pub tag: String,
    pub version: String,
    /// Signed with the local publisher key
    pub signed: bool,
}

/// Publishes a local agent: tags the synced commit in the agents repo and
/// opens a registry pull request (merged right away for a community
/// registry the user can push to).
pub async fn publish_agent(
    state: &AppState,
    input: PublishAgentInput,
) -> Result<PublishResult, String> {
    let result = async {
        let version = manifest::parse_version(&input.version)?.to_string();
        let store = state.store().clone();
        let agent = store.agent(&input.agent_id)?;
        let place = store
            .repos()
            .agents
            .locate(&input.agent_id)
            .ok_or_else(|| format!("Agent '{}' not found", input.agent_id))?;
        if place.origin.is_vendored() {
            return Err("Only your own agents can be published. Fork this one first.".to_string());
        }
        let list = registries(state);
        let registry = list
            .find(&input.registry_url)
            .cloned()
            .ok_or_else(|| format!("{} isn't one of your registries", input.registry_url))?;
        let github_token = crate::services::auth::require_token(state)?;
        let api = community::GitHubApi::new("https://api.github.com", github_token);
        let publisher = api.login().await?.to_ascii_lowercase();

        let (root, dir) = (store.layout().root.clone(), place.dir.clone());
        let (repo, commit) = blocking(move || publish::synced_release(&root, &dir)).await?;
        let relative = place
            .dir
            .strip_prefix(&store.layout().root)
            .map_err(|_| "The agent isn't in your agents repo".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let release = publish::Release {
            tag: publish::release_tag(&input.agent_id, &version),
            agent_id: input.agent_id.clone(),
            version: version.clone(),
            commit,
            repo,
            path: relative,
            publisher,
        };
        // Signed if this machine has a publisher key
        let key = signature::default_key_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let signed = key
            .as_deref()
            .map(|secret| {
                signature::sign(
                    secret,
                    "agent",
                    &release.agent_id,
                    &version,
                    &release.commit,
                )
            })
            .transpose()?;
        let public_key = key.as_deref().map(signature::public_key_of).transpose()?;
        let tags: Vec<String> = input
            .tags
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let entry = publish::entry(
            &agent,
            &release,
            &tags,
            place.dir.join("README.md").is_file(),
            signed.as_deref(),
        );
        let published = api
            .publish(
                &release,
                &entry,
                &registry.url,
                registry.kind == RegistryKind::Community,
                public_key.as_deref(),
                publish::EntryList::Agents,
            )
            .await?;
        if published.merged {
            // Show it in Discover now, not after the cache TTL
            let token = token(state, &registry);
            let _ = state
                .marketplace()
                .client
                .fetch_index(&registry, token.as_deref(), true)
                .await;
        }
        Ok(PublishResult {
            pr_url: published.pr_url,
            merged: published.merged,
            tag: release.tag,
            version,
            signed: signed.is_some(),
        })
    }
    .await;
    report(state, "publish", &input.agent_id, result)
}

/// Removes a registry (and its own token from the keyring). Installed
/// agents from it stay installed.
pub async fn remove_registry(state: &AppState, url: &str) -> Result<(), String> {
    let url = normalize_registry_url(url);
    let mut configs = state.store().read(|repo| repo.settings().registries());
    let Some(index) = configs
        .iter()
        .position(|c| normalize_registry_url(&c.url) == url)
    else {
        return Err(format!("{url} isn't in your registries"));
    };
    let removed = configs.remove(index);
    save_registries(state, configs).await?;
    if let Some(auth) = removed.auth.filter(|a| a.as_str() != GITHUB_SIGN_IN_REF) {
        if let Err(e) = state.secrets().delete(auth.as_str()) {
            log::warn!("Failed to remove the token for {url}: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
