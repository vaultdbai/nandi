//! Registries: where the catalog comes from.
//!
//! A registry is a git repo (or any static file host) with `index.json` at
//! its root, plus per-entry files (`agents/<id>.json`, `packs/<id>.json`,
//! `publishers/<handle>.json`) and assets (READMEs, icons). GitHub is the
//! CDN: public registries are read from `raw.githubusercontent.com`;
//! private ones (with a token) through the Contents API.
//!
//! Every index fetch is cached in `~/.nandi/local/cache/registry-<hash>.json`
//! with its ETag. Within the TTL (1 hour public, 15 minutes community, 5
//! minutes personal) the cache is used as-is; after it, a conditional
//! request (`If-None-Match`) usually costs a `304`. If GitHub rate-limits
//! us or is unreachable, the cached index is used with a warning.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::manifest::{self, AgentManifest, PackManifest};
use crate::services::domain::settings::{
    normalize_registry_url, RegistryConfig, RegistryKind, SecretRef,
};
use crate::services::repos::fs_atomic::{content_hash, write_atomic};

const USER_AGENT: &str = concat!("nandi/", env!("CARGO_PKG_VERSION"));
/// Largest index or asset we'll download
const MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;

/// One configured registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    pub url: String,
    pub kind: RegistryKind,
    /// Keyring entry with a GitHub token, for private registries
    pub auth: Option<SecretRef>,
}

/// Where a registry's files are.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Location {
    GitHub { owner: String, repo: String },
    Http { base: String },
    Local { dir: PathBuf },
}

impl Registry {
    pub fn from_config(config: &RegistryConfig) -> Self {
        Self {
            url: normalize_registry_url(&config.url),
            kind: config.kind,
            auth: config.auth.clone(),
        }
    }

    fn location(&self) -> Location {
        if let Some(path) = self.url.strip_prefix("github.com/") {
            let (owner, repo) = path.split_once('/').unwrap_or((path, ""));
            return Location::GitHub {
                owner: owner.to_string(),
                repo: repo.to_string(),
            };
        }
        if let Some(path) = self.url.strip_prefix("file://") {
            // file:///C:/x on Windows, file:///home/x elsewhere
            let path = if path.len() > 2 && path.as_bytes()[2] == b':' {
                &path[1..]
            } else {
                path
            };
            return Location::Local {
                dir: PathBuf::from(path),
            };
        }
        Location::Http {
            base: self.url.clone(),
        }
    }

    /// How long a fetched index is used without asking again.
    pub fn ttl(&self) -> Duration {
        match self.kind {
            RegistryKind::Public => Duration::from_secs(60 * 60),
            RegistryKind::Community => Duration::from_secs(15 * 60),
            RegistryKind::Personal => Duration::from_secs(5 * 60),
        }
    }

    fn cache_path(&self, cache_dir: &Path) -> PathBuf {
        let hash = content_hash(self.url.as_bytes());
        cache_dir.join(format!("registry-{}.json", &hash[..16]))
    }

    /// The URL of a file in the registry, and whether it's an API URL
    /// (needs the raw media type).
    fn file_url(&self, relative: &str, authenticated: bool) -> Option<(String, bool)> {
        match self.location() {
            Location::GitHub { owner, repo } if authenticated => Some((
                format!("https://api.github.com/repos/{owner}/{repo}/contents/{relative}"),
                true,
            )),
            Location::GitHub { owner, repo } => Some((
                format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/{relative}"),
                false,
            )),
            Location::Http { base } => Some((format!("{base}/{relative}"), false)),
            Location::Local { .. } => None,
        }
    }
}

/// A registry's catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryIndex {
    pub registry: String,
    pub agents: Vec<AgentManifest>,
    pub packs: Vec<PackManifest>,
    /// `connectors` entries (`connector.schema.json`), checked on install
    pub connectors: Vec<super::connectors::ConnectorEntry>,
    pub fetched_at: SystemTime,
    /// Set when this is a stale cache (rate-limited, offline)
    pub warning: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CachedIndex {
    url: String,
    etag: Option<String>,
    fetched_at_ms: u64,
    body: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn from_ms(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

/// What a download returned.
enum Fetched {
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
    NotModified,
}

/// Fetches registry files over HTTP, with the index cache.
#[derive(Clone)]
pub struct RegistryClient {
    http: reqwest::Client,
    cache_dir: PathBuf,
}

impl RegistryClient {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            cache_dir: cache_dir.into(),
        }
    }

    async fn get(
        &self,
        url: &str,
        api: bool,
        token: Option<&str>,
        etag: Option<&str>,
    ) -> Result<Fetched, FetchError> {
        let mut request = self.http.get(url);
        if api {
            request = request
                .header(reqwest::header::ACCEPT, "application/vnd.github.raw+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
        }
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || (status == reqwest::StatusCode::FORBIDDEN
                && response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|v| v == "0"));
        if rate_limited {
            return Err(FetchError::RateLimited);
        }
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if response
            .content_length()
            .is_some_and(|len| len as usize > MAX_DOWNLOAD_BYTES)
        {
            return Err(FetchError::TooLarge);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?;
        if bytes.len() > MAX_DOWNLOAD_BYTES {
            return Err(FetchError::TooLarge);
        }
        Ok(Fetched::Body {
            bytes: bytes.to_vec(),
            etag,
        })
    }

    fn load_cache(&self, registry: &Registry) -> Option<CachedIndex> {
        let text = std::fs::read_to_string(registry.cache_path(&self.cache_dir)).ok()?;
        serde_json::from_str::<CachedIndex>(&text)
            .ok()
            .filter(|cached| cached.url == registry.url)
    }

    fn save_cache(&self, registry: &Registry, cached: &CachedIndex) {
        let path = registry.cache_path(&self.cache_dir);
        let result = serde_json::to_vec(cached)
            .map_err(|e| e.to_string())
            .and_then(|bytes| write_atomic(&path, &bytes).map_err(|e| e.to_string()));
        if let Err(e) = result {
            log::warn!("Failed to cache the {} index: {e}", registry.url);
        }
    }

    fn parse(
        registry: &Registry,
        body: &str,
        fetched_at: SystemTime,
        warning: Option<String>,
    ) -> Result<RegistryIndex, String> {
        let parsed = manifest::parse_index(body, &registry.url)?;
        Ok(RegistryIndex {
            registry: registry.url.clone(),
            agents: parsed.agents,
            packs: parsed.packs,
            connectors: parsed.connectors,
            fetched_at,
            warning,
        })
    }

    /// The registry's index: from the cache within the TTL (unless
    /// `force`), else fetched (conditionally, by ETag).
    pub async fn fetch_index(
        &self,
        registry: &Registry,
        token: Option<&str>,
        force: bool,
    ) -> Result<RegistryIndex, String> {
        if let Location::Local { dir } = registry.location() {
            let path = dir.join("index.json");
            let body = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
            return Self::parse(registry, &body, SystemTime::now(), None);
        }
        let cached = self.load_cache(registry);
        if let Some(cached) = &cached {
            let age = Duration::from_millis(now_ms().saturating_sub(cached.fetched_at_ms));
            if !force && age < registry.ttl() {
                return Self::parse(registry, &cached.body, from_ms(cached.fetched_at_ms), None);
            }
        }
        let (url, api) = registry
            .file_url("index.json", token.is_some())
            .expect("remote registries have URLs");
        let etag = cached.as_ref().and_then(|c| c.etag.as_deref());
        match self.get(&url, api, token, etag).await {
            Ok(Fetched::NotModified) => {
                // Only our cached ETag earns a 304; without one, a proxy or
                // captive portal answered
                let Some(mut cached) = cached else {
                    return Err(format!(
                        "{} answered 'not modified' with nothing cached here. A proxy may be in the way",
                        registry.url
                    ));
                };
                cached.fetched_at_ms = now_ms();
                self.save_cache(registry, &cached);
                Self::parse(registry, &cached.body, SystemTime::now(), None)
            }
            Ok(Fetched::Body { bytes, etag }) => {
                let body = String::from_utf8(bytes)
                    .map_err(|_| format!("The {} index isn't UTF-8 text", registry.url))?;
                // Parse before caching: never replace a good cache with junk
                let index = Self::parse(registry, &body, SystemTime::now(), None)?;
                self.save_cache(
                    registry,
                    &CachedIndex {
                        url: registry.url.clone(),
                        etag,
                        fetched_at_ms: now_ms(),
                        body,
                    },
                );
                Ok(index)
            }
            Err(error) => {
                let message = error.describe(&registry.url);
                match cached {
                    Some(cached) => {
                        log::warn!("{message}; using the cached index");
                        let warning = Some(format!(
                            "{message}. Showing the catalog as of {}.",
                            chrono::DateTime::<chrono::Utc>::from(from_ms(cached.fetched_at_ms))
                                .format("%Y-%m-%d %H:%M UTC")
                        ));
                        Self::parse(
                            registry,
                            &cached.body,
                            from_ms(cached.fetched_at_ms),
                            warning,
                        )
                    }
                    None => Err(message),
                }
            }
        }
    }

    /// A file from the registry repo (a README, an entry, an icon).
    pub async fn fetch_file(
        &self,
        registry: &Registry,
        relative: &str,
        token: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        if !crate::services::repos::vendor_repo::is_safe_relative(relative) {
            return Err(format!("'{relative}' is not a file in the registry"));
        }
        if let Location::Local { dir } = registry.location() {
            let path = dir.join(relative);
            return tokio::fs::read(&path)
                .await
                .map_err(|e| format!("Couldn't read {}: {e}", path.display()));
        }
        let (url, api) = registry
            .file_url(relative, token.is_some())
            .expect("remote registries have URLs");
        self.download(&url, api, token, &registry.url).await
    }

    /// An asset by its entry's URL: absolute `https://`, or relative to the
    /// registry.
    pub async fn fetch_asset(
        &self,
        registry: &Registry,
        url: &str,
        token: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        if url.starts_with("https://") {
            // Only send the token to GitHub itself
            let token = token.filter(|_| {
                url.starts_with("https://raw.githubusercontent.com/")
                    || url.starts_with("https://api.github.com/")
            });
            return self.download(url, false, token, url).await;
        }
        self.fetch_file(registry, url, token).await
    }

    async fn download(
        &self,
        url: &str,
        api: bool,
        token: Option<&str>,
        what: &str,
    ) -> Result<Vec<u8>, String> {
        match self.get(url, api, token, None).await {
            Ok(Fetched::Body { bytes, .. }) => Ok(bytes),
            Ok(Fetched::NotModified) => Err(format!("{what}: unexpected 304")),
            Err(e) => Err(e.describe(what)),
        }
    }
}

#[derive(Debug)]
enum FetchError {
    Network(String),
    RateLimited,
    Status(u16),
    TooLarge,
}

impl FetchError {
    fn describe(&self, what: &str) -> String {
        match self {
            Self::Network(e) => format!("Couldn't reach {what}: {e}"),
            Self::RateLimited => format!("GitHub is rate-limiting requests to {what}"),
            Self::Status(404) => {
                format!("{what} wasn't found (if it's private, add a GitHub token)")
            }
            Self::Status(401 | 403) => format!("{what} refused access; check its token"),
            Self::Status(code) => format!("{what} returned HTTP {code}"),
            Self::TooLarge => format!(
                "{what} is larger than {} MB",
                MAX_DOWNLOAD_BYTES / (1024 * 1024)
            ),
        }
    }
}

// -- Several registries ---------------------------------------------------------

/// The configured registries, in priority order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RegistryList {
    pub registries: Vec<Registry>,
}

impl RegistryList {
    pub fn from_configs(configs: &[RegistryConfig]) -> Self {
        Self {
            registries: configs.iter().map(Registry::from_config).collect(),
        }
    }

    pub fn find(&self, url: &str) -> Option<&Registry> {
        let url = normalize_registry_url(url);
        self.registries.iter().find(|r| r.url == url)
    }

    /// Every agent across `indexes`, one per ID: the highest
    /// `latest_version` wins; a tie goes to the earlier registry.
    pub fn merge_agents(indexes: &[RegistryIndex]) -> Vec<AgentManifest> {
        merge(indexes.iter().flat_map(|i| i.agents.iter()), |a| {
            (a.id.clone(), a.latest_version.clone())
        })
    }

    /// Every pack across `indexes`, one per ID (as `merge_agents`).
    pub fn merge_packs(indexes: &[RegistryIndex]) -> Vec<PackManifest> {
        merge(indexes.iter().flat_map(|i| i.packs.iter()), |p| {
            (p.id.clone(), p.latest_version.clone())
        })
    }
}

fn merge<'a, T: Clone + 'a>(
    entries: impl Iterator<Item = &'a T>,
    key: impl Fn(&T) -> (String, String),
) -> Vec<T> {
    let mut merged: Vec<(String, semver::Version, T)> = Vec::new();
    for entry in entries {
        let (id, version) = key(entry);
        let version = manifest::parse_version(&version).unwrap_or(semver::Version::new(0, 0, 0));
        match merged.iter_mut().find(|(existing, ..)| *existing == id) {
            Some(slot) if version > slot.1 => *slot = (id, version, entry.clone()),
            Some(_) => {}
            None => merged.push((id, version, entry.clone())),
        }
    }
    merged.into_iter().map(|(.., entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::marketplace::manifest::tests::SAMPLE_INDEX;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn registry(url: &str, kind: RegistryKind) -> Registry {
        Registry {
            url: url.to_string(),
            kind,
            auth: None,
        }
    }

    #[test]
    fn locations_and_urls() {
        let github = registry("github.com/vaultdbai/nandi-registry", RegistryKind::Public);
        assert_eq!(
            github.file_url("index.json", false).unwrap().0,
            "https://raw.githubusercontent.com/vaultdbai/nandi-registry/HEAD/index.json"
        );
        assert_eq!(
            github.file_url("index.json", true).unwrap(),
            (
                "https://api.github.com/repos/vaultdbai/nandi-registry/contents/index.json"
                    .to_string(),
                true
            )
        );
        assert!(registry("file:///tmp/reg", RegistryKind::Personal)
            .file_url("index.json", false)
            .is_none());
        assert!(registry("x", RegistryKind::Personal).ttl() < github.ttl());
    }

    #[test]
    fn fetch_index_parses_a_local_registry() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("index.json"), SAMPLE_INDEX).unwrap();
        let url = format!(
            "file://{}",
            tmp.path().display().to_string().replace('\\', "/")
        );
        let url = if url.as_bytes().get(8) == Some(&b':') {
            url.replacen("file://", "file:///", 1)
        } else {
            url
        };
        let client = RegistryClient::new(tmp.path().join("cache"));
        let index = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(client.fetch_index(&registry(&url, RegistryKind::Personal), None, false))
            .unwrap();
        assert_eq!(index.agents[0].id, "arxiv-researcher");
        assert_eq!(index.packs[0].id, "research-pack");
    }

    fn index(registry: &str, agents: &[(&str, &str)]) -> RegistryIndex {
        let json = serde_json::json!({
            "agents": agents.iter().map(|(id, version)| serde_json::json!({
                "id": id, "name": id, "publisher": "p", "repo": "github.com/p/r",
                "path": id, "latest_version": version,
            })).collect::<Vec<_>>()
        });
        RegistryClient::parse(
            &self::registry(registry, RegistryKind::Public),
            &json.to_string(),
            SystemTime::now(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn dedup_keeps_the_latest_version() {
        let merged = RegistryList::merge_agents(&[
            index("a", &[("x", "1.2.0"), ("y", "1.0.0")]),
            index("b", &[("x", "1.10.0"), ("y", "1.0.0"), ("z", "0.1.0")]),
        ]);
        let found: Vec<(&str, &str, &str)> = merged
            .iter()
            .map(|a| {
                (
                    a.id.as_str(),
                    a.latest_version.as_str(),
                    a.registry.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("x", "1.10.0", "b"),
                ("y", "1.0.0", "a"),
                ("z", "0.1.0", "b")
            ]
        );
    }

    /// A tiny HTTP server: `index.json` with an ETag, a 304 on a match,
    /// and a rate limit when told to.
    async fn serve(hits: Arc<AtomicUsize>, limited: Arc<std::sync::atomic::AtomicBool>) -> String {
        use axum::http::{HeaderMap, StatusCode};
        use axum::response::IntoResponse;
        let app = axum::Router::new().route(
            "/index.json",
            axum::routing::get(move |headers: HeaderMap| {
                let hits = hits.clone();
                let limited = limited.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    if limited.load(Ordering::SeqCst) {
                        return (StatusCode::FORBIDDEN, [("x-ratelimit-remaining", "0")], "")
                            .into_response();
                    }
                    if headers.get("if-none-match").is_some_and(|v| v == "\"v1\"") {
                        return StatusCode::NOT_MODIFIED.into_response();
                    }
                    ([("etag", "\"v1\"")], SAMPLE_INDEX).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn caches_with_etags_and_falls_back_when_rate_limited() {
        let tmp = tempfile::tempdir().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let limited = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let base = serve(hits.clone(), limited.clone()).await;
        let client = RegistryClient::new(tmp.path());
        let reg = registry(&base, RegistryKind::Personal);

        let first = client.fetch_index(&reg, None, false).await.unwrap();
        assert_eq!(first.agents.len(), 2);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // Within the TTL: no request
        client.fetch_index(&reg, None, false).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // Forced: a conditional request answered with 304
        let again = client.fetch_index(&reg, None, true).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(again.agents, first.agents);
        assert!(again.warning.is_none());
        // Rate-limited: the cache, with a warning
        limited.store(true, Ordering::SeqCst);
        let stale = client.fetch_index(&reg, None, true).await.unwrap();
        assert!(stale.warning.unwrap().contains("rate-limiting"));
        assert_eq!(stale.agents, first.agents);
        // No cache at all: an error
        let fresh = RegistryClient::new(tmp.path().join("empty"));
        assert!(fresh.fetch_index(&reg, None, true).await.is_err());
    }

    #[tokio::test]
    async fn a_304_with_nothing_cached_is_an_error_not_a_panic() {
        // A proxy that answers "not modified" to everything
        let app = axum::Router::new().route(
            "/index.json",
            axum::routing::get(|| async { axum::http::StatusCode::NOT_MODIFIED }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry(&format!("http://{addr}"), RegistryKind::Personal);

        let error = RegistryClient::new(tmp.path())
            .fetch_index(&reg, None, false)
            .await
            .unwrap_err();
        assert!(error.contains("not modified"), "{error}");
    }
}
