//! An image's download size, read from its registry before the pull, so
//! the app can say "about 450 MB" up front. Anonymous, as for any public
//! image: the registry's `WWW-Authenticate` challenge names where to get a
//! pull token (ghcr.io, Docker Hub and most registries work this way).
//!
//! Only a hint: offline, or for a registry that wants a login, the size is
//! `None` and the pull's own progress counts the layers it sees.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// The whole lookup gives up after this
const TIMEOUT: Duration = Duration::from_secs(6);

pub(crate) const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
    application/vnd.docker.distribution.manifest.list.v2+json, \
    application/vnd.oci.image.manifest.v1+json, \
    application/vnd.docker.distribution.manifest.v2+json";

/// A failed lookup isn't tried again for this long (status calls ask often)
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// What a lookup found this session, by image reference: the size, or
/// when it last failed
static SIZES: LazyLock<Mutex<HashMap<String, Result<u64, Instant>>>> =
    LazyLock::new(Mutex::default);

/// `ghcr.io/vaultdbai/nandi-harness:sandbox-6` as registry, repository, tag
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub registry: String,
    pub repository: String,
    pub tag: String,
}

impl Reference {
    pub fn parse(image: &str) -> Option<Self> {
        let (name, tag) = match image.rsplit_once(':') {
            Some((name, tag)) if !tag.contains('/') => (name, tag),
            _ => (image, "latest"),
        };
        let (registry, repository) = name.split_once('/')?;
        if !registry.contains('.') && !registry.contains(':') && registry != "localhost" {
            // Docker Hub's short names (`library/node`) aren't ours to size
            return None;
        }
        Some(Self {
            registry: registry.to_string(),
            repository: repository.to_string(),
            tag: tag.to_string(),
        })
    }
}

/// `1234567890` → `1.1 GB`; `456000000` → `435 MB` (binary units, as
/// the UI shows them)
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{} MB", mb.round().max(1.0) as u64)
    }
}

/// The compressed size of `image` for this computer's platform, when its
/// registry answers in time
pub async fn download_size(image: &str) -> Option<u64> {
    match SIZES.lock().ok()?.get(image).copied() {
        Some(Ok(size)) => return Some(size),
        Some(Err(failed)) if failed.elapsed() < RETRY_AFTER => return None,
        _ => {}
    }
    let reference = Reference::parse(image)?;
    let found = match tokio::time::timeout(TIMEOUT, fetch_size(&reference)).await {
        Ok(Ok(size)) => Ok(size),
        Ok(Err(e)) => {
            log::debug!("No download size for {image}: {e}");
            Err(Instant::now())
        }
        Err(_) => Err(Instant::now()),
    };
    SIZES.lock().ok()?.insert(image.to_string(), found);
    found.ok()
}

async fn fetch_size(reference: &Reference) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let url = |what: &str| {
        format!(
            "https://{}/v2/{}/manifests/{what}",
            reference.registry, reference.repository
        )
    };
    let mut token: Option<String> = None;
    let mut manifest = get(&client, &url(&reference.tag), None).await?;
    if let Manifest::Challenge(challenge) = &manifest {
        token = Some(pull_token(&client, challenge, &reference.repository).await?);
        manifest = get(&client, &url(&reference.tag), token.as_deref()).await?;
    }
    if let Manifest::Body(body) = &manifest {
        if let Some(digest) = platform_digest(body, host_arch()) {
            manifest = get(&client, &url(&digest), token.as_deref()).await?;
        }
    }
    match manifest {
        Manifest::Body(body) => layers_size(&body).ok_or_else(|| "no layers".to_string()),
        Manifest::Challenge(_) => Err("the registry wants a login".to_string()),
    }
}

enum Manifest {
    Body(String),
    /// 401 with this `WWW-Authenticate`
    Challenge(String),
}

async fn get(client: &reqwest::Client, url: &str, token: Option<&str>) -> Result<Manifest, String> {
    let mut request = client.get(url).header("Accept", ACCEPT);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        return Ok(Manifest::Challenge(challenge));
    }
    if !response.status().is_success() {
        return Err(format!("{url}: {}", response.status()));
    }
    response
        .text()
        .await
        .map(Manifest::Body)
        .map_err(|e| e.to_string())
}

/// The anonymous pull token the challenge points to
pub(crate) async fn pull_token(
    client: &reqwest::Client,
    challenge: &str,
    repository: &str,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Token {
        token: Option<String>,
        access_token: Option<String>,
    }
    let params = parse_challenge(challenge).ok_or("no bearer challenge")?;
    let realm = params.get("realm").ok_or("no realm")?;
    let scope = params
        .get("scope")
        .cloned()
        .unwrap_or_else(|| format!("repository:{repository}:pull"));
    let mut query = vec![("scope", scope)];
    if let Some(service) = params.get("service") {
        query.push(("service", service.clone()));
    }
    let url = reqwest::Url::parse_with_params(realm, &query).map_err(|e| e.to_string())?;
    let token: Token = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    token
        .token
        .or(token.access_token)
        .ok_or_else(|| "no token".to_string())
}

/// `Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="…"` →
/// its parameters
fn parse_challenge(header: &str) -> Option<HashMap<String, String>> {
    let rest = header.trim().strip_prefix("Bearer ")?;
    let mut params = HashMap::new();
    for part in rest.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        params.insert(
            key.trim().to_string(),
            value.trim().trim_matches('"').to_string(),
        );
    }
    Some(params)
}

/// Docker's name for this computer's CPU (what a pull picks)
fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

#[derive(Deserialize)]
struct Index {
    manifests: Option<Vec<IndexEntry>>,
    layers: Option<Vec<Blob>>,
    config: Option<Blob>,
}

#[derive(Deserialize)]
struct IndexEntry {
    digest: String,
    platform: Option<Platform>,
}

#[derive(Deserialize)]
struct Platform {
    os: String,
    architecture: String,
}

#[derive(Deserialize)]
struct Blob {
    size: u64,
}

/// In an index (several platforms), the linux/`arch` manifest's digest
fn platform_digest(body: &str, arch: &str) -> Option<String> {
    let index: Index = serde_json::from_str(body).ok()?;
    index
        .manifests?
        .into_iter()
        .find(|m| {
            m.platform
                .as_ref()
                .is_some_and(|p| p.os == "linux" && p.architecture == arch)
        })
        .map(|m| m.digest)
}

/// In one platform's manifest, the config and layers together
fn layers_size(body: &str) -> Option<u64> {
    let manifest: Index = serde_json::from_str(body).ok()?;
    let layers = manifest.layers?;
    Some(layers.iter().map(|l| l.size).sum::<u64>() + manifest.config.map_or(0, |c| c.size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_split_into_registry_repository_and_tag() {
        assert_eq!(
            Reference::parse("ghcr.io/vaultdbai/nandi-harness:sandbox-6"),
            Some(Reference {
                registry: "ghcr.io".into(),
                repository: "vaultdbai/nandi-harness".into(),
                tag: "sandbox-6".into(),
            })
        );
        let local = Reference::parse("localhost:5000/team/img").unwrap();
        assert_eq!(
            (local.registry.as_str(), local.tag.as_str()),
            ("localhost:5000", "latest")
        );
        assert_eq!(Reference::parse("node:22"), None);
        assert_eq!(Reference::parse("library/node:22"), None);
    }

    #[test]
    fn a_bearer_challenge_names_the_token_service() {
        let params = parse_challenge(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:vaultdbai/nandi-harness:pull""#,
        )
        .unwrap();
        assert_eq!(params["realm"], "https://ghcr.io/token");
        assert_eq!(params["service"], "ghcr.io");
        assert_eq!(params["scope"], "repository:vaultdbai/nandi-harness:pull");
        assert!(parse_challenge("Basic realm=\"x\"").is_none());
    }

    #[test]
    fn an_index_picks_this_platform_and_a_manifest_sums_its_layers() {
        let index = r#"{"manifests":[
            {"digest":"sha256:arm","platform":{"os":"linux","architecture":"arm64"}},
            {"digest":"sha256:amd","platform":{"os":"linux","architecture":"amd64"}},
            {"digest":"sha256:att","platform":{"os":"unknown","architecture":"unknown"}}]}"#;
        assert_eq!(
            platform_digest(index, "amd64").as_deref(),
            Some("sha256:amd")
        );
        assert_eq!(
            platform_digest(index, "arm64").as_deref(),
            Some("sha256:arm")
        );
        assert_eq!(platform_digest(index, "s390x"), None);
        let manifest = r#"{"config":{"size":7},"layers":[{"size":100},{"size":23}]}"#;
        assert_eq!(platform_digest(manifest, "amd64"), None);
        assert_eq!(layers_size(manifest), Some(130));
        assert_eq!(layers_size(index), None);
    }

    #[test]
    fn sizes_read_as_megabytes_or_gigabytes() {
        assert_eq!(megabytes(456_000_000), "435 MB");
        assert_eq!(megabytes(1_234_567_890), "1.1 GB");
        assert_eq!(megabytes(10), "1 MB");
    }
}
