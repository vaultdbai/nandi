//! Connectors in registries: listing, the install dialog, install and
//! publish.
//!
//! ```text
//! index.json connectors[] ─▶ Library › Browse › Connectors
//! Install ─▶ fetch connector.md at the entry's ref (and commit) ─▶ it must
//!   say exactly what the entry says ─▶ the consent dialog (reads, writes,
//!   hosts, sign-in, image) ─▶ connectors.yaml + .nandi/connectors/<id>/connector.md
//!   + the key in the keyring. No grant: each agent asks first
//! Publish (Studio) ─▶ tag <id>-v<version> ─▶ signed entry ─▶ a pull request
//!   to a GitHub registry, or written straight into a local (file://) one
//! ```
//!
//! The public registry never installs a connector that runs on the host
//! (decision 2); a company (community) or personal registry may list one.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::registry::{self, Registry, RegistryEntry};
use super::source::GitSource;
use super::{blocking, indexes, publisher_key, registries, report, signature, token};
use crate::services::connectors::{self, PackageInfo};
use crate::services::domain::connector_package::{
    ConnectorPackage, PackageRelease, PackageSource, Placement,
};
use crate::services::domain::installed::{check_repo_path, check_source};
use crate::services::domain::settings::RegistryKind;
use crate::services::domain::{ConnectorId, PublisherHandle};
use crate::services::repos::layout::CONNECTOR_FILE;
use crate::services::AppState;

/// A `connectors` entry of an index, as the Library lists it
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub repo: String,
    pub path: String,
    pub version: String,
    pub git_ref: String,
    pub commit: Option<String>,
    pub signature: Option<String>,
    /// `mcp-stdio`, `mcp-http` or `builtin`
    pub kind: String,
    pub placement: Placement,
    pub image: Option<String>,
    pub tags: Vec<String>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub hosts: Vec<String>,
    /// `none`, `api_key`, `oauth`, `device_code`
    pub auth: String,
    /// The registry it came from
    pub registry: String,
    /// Its signature verifies against the publisher's key
    pub verified: bool,
    /// The installed version, if it's installed
    pub installed_version: Option<String>,
    /// The entry as published, for the install check
    #[serde(skip)]
    #[specta(skip)]
    pub raw: Value,
}

#[derive(Deserialize)]
struct Wire {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    publisher: String,
    repo: String,
    path: String,
    latest_version: String,
    #[serde(default)]
    r#ref: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    kind: String,
    #[serde(default)]
    placement: Placement,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    data: WireData,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    auth: Option<WireAuth>,
}

#[derive(Deserialize, Default)]
struct WireData {
    #[serde(default)]
    read: Vec<String>,
    #[serde(default)]
    write: Vec<String>,
}

#[derive(Deserialize)]
struct WireAuth {
    #[serde(rename = "type")]
    kind: String,
}

impl ConnectorEntry {
    /// Parses one entry leniently and validates it strictly: a bad entry is
    /// skipped with why
    pub fn parse(value: Value, registry: &str) -> Result<Self, String> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("(no id)")
            .to_string();
        let wire: Wire =
            serde_json::from_value(value.clone()).map_err(|e| format!("connector '{id}': {e}"))?;
        let bad = |e: String| format!("connector '{}': {e}", wire.id);
        ConnectorId::parse(wire.id.clone()).map_err(|e| bad(e.to_string()))?;
        PublisherHandle::parse(wire.publisher.clone()).map_err(|e| bad(e.to_string()))?;
        check_source(&wire.id, &wire.repo).map_err(|e| bad(e.to_string()))?;
        check_repo_path(&wire.id, &wire.path).map_err(|e| bad(e.to_string()))?;
        let version = super::manifest::parse_version(&wire.latest_version).map_err(bad)?;
        if !matches!(wire.kind.as_str(), "mcp-stdio" | "mcp-http" | "builtin") {
            return Err(bad(format!(
                "kind '{}' isn't mcp-stdio, mcp-http or builtin",
                wire.kind
            )));
        }
        Ok(Self {
            git_ref: wire.r#ref.clone().unwrap_or_else(|| format!("v{version}")),
            id: wire.id,
            name: wire.name,
            description: wire.description,
            publisher: wire.publisher.to_ascii_lowercase(),
            repo: wire.repo,
            path: wire.path,
            version: version.to_string(),
            commit: wire.commit,
            signature: wire.signature,
            kind: wire.kind,
            placement: wire.placement,
            image: wire.image,
            tags: wire.tags,
            reads: wire.data.read,
            writes: wire.data.write,
            hosts: wire.hosts,
            auth: wire.auth.map_or_else(|| "none".to_string(), |a| a.kind),
            registry: registry.to_string(),
            verified: false,
            installed_version: None,
            raw: value,
        })
    }
}

/// What the install dialog shows: what the connector.md at the release
/// says, checked against the entry
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorConsent {
    pub id: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub verified: bool,
    pub registry: String,
    pub package: PackageInfo,
    /// The instructions agents get once granted
    pub instructions: String,
    pub installed_version: Option<String>,
}

/// Every connector in every registry, one per ID (merged like agents and
/// packs: the default registry wins its IDs). The public registry's host
/// connectors are left out
pub async fn list(state: &AppState, refresh: bool) -> Result<Vec<ConnectorEntry>, String> {
    let (indexes, _failed) = indexes(state, refresh).await;
    let list = registries(state);
    let mut listed: Vec<&ConnectorEntry> = Vec::new();
    for index in &indexes {
        let Some(registry) = list.find(&index.registry) else {
            continue;
        };
        for entry in &index.connectors {
            if registry.kind == RegistryKind::Public && entry.placement == Placement::Host {
                log::warn!(
                    "Registry {}: left out host connector '{}'",
                    registry.url,
                    entry.id
                );
                continue;
            }
            listed.push(entry);
        }
    }
    let mut out = registry::merge(listed);
    for entry in &mut out {
        if let Some(registry) = list.find(&entry.registry) {
            entry.verified = verified(state, registry, entry).await;
        }
    }
    let manifest = state.store().read(|repo| repo.connectors().clone());
    for entry in &mut out {
        entry.installed_version = manifest
            .get(&entry.id)
            .and_then(|d| d.config.get("package"))
            .and_then(|p| p.get("version"))
            .and_then(serde_yaml::Value::as_str)
            .map(str::to_string);
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    Ok(out)
}

impl RegistryEntry for ConnectorEntry {
    fn id(&self) -> &str {
        &self.id
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn registry(&self) -> &str {
        &self.registry
    }
    fn tags_mut(&mut self) -> &mut Vec<String> {
        &mut self.tags
    }
}

async fn verified(state: &AppState, registry: &Registry, entry: &ConnectorEntry) -> bool {
    let (Some(signature), Some(commit)) = (&entry.signature, &entry.commit) else {
        return false;
    };
    match publisher_key(state, registry, &entry.publisher).await {
        Some(key) => signature::verify(
            &key,
            signature,
            "connector",
            &entry.id,
            &entry.version,
            commit,
        ),
        None => false,
    }
}

async fn find(
    state: &AppState,
    registry_url: &str,
    id: &str,
) -> Result<(Registry, ConnectorEntry), String> {
    let list = registries(state);
    let registry = list
        .find(registry_url)
        .cloned()
        .ok_or_else(|| format!("{registry_url} isn't one of your registries"))?;
    let entry = list_from(state, &registry)
        .await?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("{registry_url} has no connector '{id}'"))?;
    if registry.kind == RegistryKind::Public && entry.placement == Placement::Host {
        return Err(format!(
            "'{id}' runs on the host: connectors from the public registry always run in a container"
        ));
    }
    Ok((registry, entry))
}

async fn list_from(state: &AppState, registry: &Registry) -> Result<Vec<ConnectorEntry>, String> {
    let token = token(state, registry);
    let index = state
        .marketplace()
        .client
        .fetch_index(registry, token.as_deref(), false)
        .await?;
    Ok(index.connectors)
}

/// The connector.md at the entry's release, parsed and checked against the
/// entry (a moved tag or a file that promises something else is refused)
async fn fetch_package(
    state: &AppState,
    registry: &Registry,
    entry: &ConnectorEntry,
) -> Result<(ConnectorPackage, String), String> {
    let source = GitSource::new(&entry.repo, token(state, registry));
    let scratch = state.marketplace().scratch_dir.clone();
    let (git_ref, path, id) = (entry.git_ref.clone(), entry.path.clone(), entry.id.clone());
    let expected = entry.commit.clone();
    let (text, commit) = blocking(move || {
        let remote = source.resolve(&git_ref)?;
        if let Some(expected) = &expected {
            if !remote.commit.eq_ignore_ascii_case(expected) {
                return Err(format!(
                    "The release tag {git_ref} moved to another commit since it was published; refusing to install"
                ));
            }
        }
        let checkout = source.fetch(&[remote.name.as_str()], &scratch, |_| {})?;
        let files = checkout.files(&remote.commit, Some(&path))?;
        let bytes = files
            .get(CONNECTOR_FILE)
            .ok_or_else(|| format!("The release has no {path}/{CONNECTOR_FILE}"))?;
        let text = String::from_utf8(bytes.clone()).map_err(|_| format!("{CONNECTOR_FILE} isn't UTF-8"))?;
        Ok((text, remote.commit))
    })
    .await?;
    let package = ConnectorPackage::parse(&id, &text).map_err(|e| e.to_string())?;
    package.matches_entry(&entry.raw)?;
    if registry.kind == RegistryKind::Public {
        package.check_public()?;
    }
    Ok((package, commit))
}

/// What installing `id` from `registry_url` would give, for the dialog
pub async fn consent(
    state: &AppState,
    registry_url: &str,
    id: &str,
) -> Result<ConnectorConsent, String> {
    let result = async {
        let (registry, mut entry) = find(state, registry_url, id).await?;
        entry.verified = verified(state, &registry, &entry).await;
        let (package, _) = fetch_package(state, &registry, &entry).await?;
        let manifest = state.store().read(|repo| repo.connectors().clone());
        let installed_version = manifest
            .get(id)
            .and_then(|d| d.config.get("package"))
            .and_then(|p| p.get("version"))
            .and_then(serde_yaml::Value::as_str)
            .map(str::to_string);
        let source = PackageSource {
            registry: registry.url.clone(),
            publisher: entry.publisher.clone(),
            commit: entry.commit.clone().unwrap_or_default(),
        };
        let decl = package.declaration(Some(&source));
        Ok(ConnectorConsent {
            id: entry.id,
            name: package.name.clone(),
            description: package.description.clone(),
            publisher: entry.publisher,
            verified: entry.verified,
            registry: registry.url,
            package: PackageInfo::of(&package, Some(&decl)),
            instructions: package.instructions.clone(),
            installed_version,
        })
    }
    .await;
    report(state, "connector consent", id, result)
}

/// Installs (or updates to) the registry's release of `id`, after the
/// person saw the dialog. `api_key` goes to the keyring
pub async fn install(
    state: &AppState,
    registry_url: &str,
    id: &str,
    api_key: Option<String>,
) -> Result<(), String> {
    let result = async {
        let _writer = state.marketplace().writer.lock().await;
        let (registry, entry) = find(state, registry_url, id).await?;
        let (package, commit) = fetch_package(state, &registry, &entry).await?;
        let source = PackageSource {
            registry: registry.url.clone(),
            publisher: entry.publisher.clone(),
            commit,
        };
        connectors::install_package(state, &package, Some(&source), api_key).await
    }
    .await;
    report(state, "connector install", id, result)
}

// ---------------------------------------------------------------------------
// Publish
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishConnectorInput {
    pub connector_id: String,
    pub registry_url: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishConnectorResult {
    /// The pull request (GitHub registries); `None` for a local registry
    pub pr_url: Option<String>,
    /// Listed right away (a local registry, or a community one you can push to)
    pub merged: bool,
    pub tag: String,
    pub version: String,
    pub signed: bool,
}

/// `<id>-v<version>`: the agents repo holds many connectors and agents
pub fn release_tag(id: &str, version: &str) -> String {
    super::publish::release_tag(id, version)
}

/// Commits the package folder if it changed and tags the release, in the
/// person's agents repo. Returns the commit
fn tag_locally(root: &std::path::Path, dir: &std::path::Path, tag: &str) -> Result<String, String> {
    let repo = git2::Repository::open(root)
        .map_err(|e| format!("Your agents repo can't be opened: {}", e.message()))?;
    let relative = dir
        .strip_prefix(root)
        .map_err(|_| "The connector isn't in your agents repo".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let mut index = repo.index().map_err(|e| e.message().to_string())?;
    index
        .add_all(
            [format!("{relative}/*")],
            git2::IndexAddOption::DEFAULT,
            None,
        )
        .map_err(|e| e.message().to_string())?;
    index.write().map_err(|e| e.message().to_string())?;
    let tree_id = index.write_tree().map_err(|e| e.message().to_string())?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(|e| e.message().to_string())?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let signature = repo
        .signature()
        .or_else(|_| git2::Signature::now("Nandi", "nandi@localhost"))
        .map_err(|e| e.message().to_string())?;
    let commit = match &parent {
        Some(parent) if parent.tree_id() == tree_id => parent.id(),
        _ => {
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                &format!("Publish {tag}"),
                &tree,
                &parents,
            )
            .map_err(|e| e.message().to_string())?
        }
    };
    let object = repo
        .find_object(commit, None)
        .map_err(|e| e.message().to_string())?;
    match repo.tag_lightweight(tag, &object, false) {
        Ok(_) => {}
        Err(e) if e.code() == git2::ErrorCode::Exists => {
            let existing = repo
                .revparse_single(&format!("refs/tags/{tag}"))
                .and_then(|o| o.peel_to_commit())
                .map_err(|e| e.message().to_string())?;
            if existing.id() != commit {
                return Err(format!(
                    "{tag} is already released from other content: raise the version"
                ));
            }
        }
        Err(e) => return Err(e.message().to_string()),
    }
    Ok(commit.to_string())
}

/// Writes the entry into a local registry folder (a company's shared
/// folder, or a test registry): `connectors/<id>.json` and `index.json`
fn write_local(
    dir: &std::path::Path,
    entry: &Value,
    publisher: &str,
    public_key: Option<&str>,
) -> Result<(), String> {
    let id = entry["id"].as_str().unwrap_or_default();
    let write = |path: std::path::PathBuf, text: String| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write(
        dir.join("connectors").join(format!("{id}.json")),
        format!("{entry:#}\n"),
    )?;
    let index_path = dir.join("index.json");
    let index = std::fs::read_to_string(&index_path).unwrap_or_default();
    let mut listed = entry.clone();
    if let Value::Object(map) = &mut listed {
        map.remove("icon_url");
        map.remove("readme_url");
    }
    write(
        index_path,
        super::publish::upsert_index(&index, "connectors", &listed)?,
    )?;
    let publisher_path = dir.join("publishers").join(format!("{publisher}.json"));
    if !publisher_path.is_file() {
        let file = json!({
            "github_handle": publisher,
            "public_key": public_key,
            "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        });
        write(publisher_path, format!("{file:#}\n"))?;
    }
    Ok(())
}

/// Publishes a connector made here (Studio): tags the release, signs the
/// entry with this computer's publisher key if there is one, and opens the
/// registry pull request (or writes a local registry)
pub async fn publish(
    state: &AppState,
    input: PublishConnectorInput,
) -> Result<PublishConnectorResult, String> {
    let id = input.connector_id.clone();
    let result = async {
        let store = state.store().clone();
        let package = store
            .repos()
            .connectors
            .load_package(&input.connector_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("'{}' isn't a connector made here", input.connector_id))?;
        let list = registries(state);
        let registry = list
            .find(&input.registry_url)
            .cloned()
            .ok_or_else(|| format!("{} isn't one of your registries", input.registry_url))?;
        if registry.kind == RegistryKind::Public {
            package.check_public()?;
        }
        let version = package.version.to_string();
        let tag = release_tag(package.id.as_str(), &version);
        let root = store.layout().root.clone();
        let dir = store.repos().connectors.package_dir(package.id.as_str());
        let path = dir
            .strip_prefix(&root)
            .map_err(|_| "The connector isn't in your agents repo".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let local = registry.url.starts_with("file://");
        let (repo, commit, publisher, api) = if local {
            let (root_c, dir_c, tag_c) = (root.clone(), dir.clone(), tag.clone());
            let commit = blocking(move || tag_locally(&root_c, &dir_c, &tag_c)).await?;
            let publisher = crate::services::auth::github::status(state)
                .login
                .unwrap_or_else(|| "local".to_string())
                .to_ascii_lowercase();
            (
                root.to_string_lossy().replace('\\', "/"),
                commit,
                publisher,
                None,
            )
        } else {
            let github_token = crate::services::auth::require_token(state)?;
            let api = super::community::GitHubApi::new("https://api.github.com", github_token);
            let publisher = api.login().await?.to_ascii_lowercase();
            let (root_c, dir_c) = (root.clone(), dir.clone());
            let (repo, commit) =
                blocking(move || super::publish::synced_release(&root_c, &dir_c)).await?;
            (repo, commit, publisher, Some(api))
        };
        let key = signature::default_key_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let signed = key
            .as_deref()
            .map(|secret| {
                signature::sign(secret, "connector", package.id.as_str(), &version, &commit)
            })
            .transpose()?;
        let public_key = key.as_deref().map(signature::public_key_of).transpose()?;
        let release = PackageRelease {
            publisher: publisher.clone(),
            repo: repo.clone(),
            path: path.clone(),
            tag: tag.clone(),
            commit: commit.clone(),
            min_nandi_version: super::manifest::NANDI_VERSION.to_string(),
            signature: signed.clone(),
        };
        let entry = package.entry(&release);
        let (pr_url, merged) = match api {
            None => {
                let dir = match registry.url.strip_prefix("file://") {
                    Some(p) if p.len() > 2 && p.as_bytes()[2] == b':' => {
                        std::path::PathBuf::from(&p[1..])
                    }
                    Some(p) => std::path::PathBuf::from(p),
                    None => unreachable!("local registries are file://"),
                };
                write_local(&dir, &entry, &publisher, public_key.as_deref())?;
                (None, true)
            }
            Some(api) => {
                let agent_release = super::publish::Release {
                    agent_id: package.id.to_string(),
                    version: version.clone(),
                    tag: tag.clone(),
                    commit: commit.clone(),
                    repo,
                    path,
                    publisher,
                };
                let published = api
                    .publish(
                        &agent_release,
                        &entry,
                        &registry.url,
                        registry.kind == RegistryKind::Community,
                        public_key.as_deref(),
                        super::publish::EntryList::Connectors,
                    )
                    .await?;
                (Some(published.pr_url), published.merged)
            }
        };
        if merged {
            let token = token(state, &registry);
            let _ = state
                .marketplace()
                .client
                .fetch_index(&registry, token.as_deref(), true)
                .await;
        }
        Ok(PublishConnectorResult {
            pr_url,
            merged,
            tag,
            version,
            signed: signed.is_some(),
        })
    }
    .await;
    report(state, "connector publish", &id, result)
}
