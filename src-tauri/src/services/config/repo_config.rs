//! Enterprise LLM config: `.nandi/llm.yaml` in the repo,
//! so everyone who clones a team repo shares one Bedrock / Azure / LiteLLM
//! setup without configuring their machine.
//!
//! ```yaml
//! providers:
//!   - id: bedrock-primary
//!     kind: bedrock          # bedrock | azure | litellm | anthropic | openai | ollama
//!     region: us-east-1
//!     profile: acme-dev      # an AWS CLI profile; credentials stay in the AWS CLI
//!     model: us.anthropic.claude-sonnet-5
//!   - id: gateway
//!     kind: litellm
//!     endpoint: https://llm.acme.internal
//! routing:
//!   default: bedrock-primary
//!   fallbacks: [gateway]
//! ```
//!
//! Precedence, per value: environment variable > repo config > local
//! config (the agent's own `runtime_config` stays more specific than all
//! three). The file holds no secrets: credentials still come from the AWS
//! CLI, the keyring, or the environment.
//!
//! - Harness agents: the default provider picks the CLI's provider
//!   (`bedrock` with its profile and region, `azure` with its endpoint,
//!   `litellm` as an external Anthropic-compatible gateway) and its model.
//! - Native agents: an `anthropic` / `openai` / `ollama` default provider's
//!   model replaces the default model from settings (`default_model`).
//!
//! The file is re-read whenever it changes on disk (an mtime check per
//! read), so a `git pull` takes effect; "Sync" re-reads it now, records it
//! in `repo_config_cache`, and emits `config://repo_llm_changed`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, Db};
use crate::model::Timestamp;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::repo_store::RepoStore;
use crate::services::AppState;

/// `config://repo_llm_changed`
pub const REPO_LLM_CHANGED: &str = "config://repo_llm_changed";

/// The file, relative to the repo root
pub const REPO_LLM_FILE: &str = ".nandi/llm.yaml";

/// `repo_config_cache` key of the last synced file
const CACHE_KEY: &str = "llm";

/// Overrides the provider kind (environment beats the repo)
pub const PROVIDER_ENV: &str = "NANDI_LLM_PROVIDER";
/// Overrides the model (environment beats the repo)
pub const MODEL_ENV: &str = "NANDI_LLM_MODEL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderKind {
    Bedrock,
    Azure,
    Litellm,
    Anthropic,
    Openai,
    Ollama,
}

impl LlmProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bedrock => "bedrock",
            Self::Azure => "azure",
            Self::Litellm => "litellm",
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Ollama => "ollama",
        }
    }

    /// The `<prefix>:` of Nandi's native model refs, for providers native
    /// agents can call
    fn native_prefix(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("claude"),
            Self::Openai => Some("gpt"),
            Self::Ollama => Some("ollama"),
            Self::Bedrock | Self::Azure | Self::Litellm => None,
        }
    }
}

/// One entry of `providers:`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmProviderConfig {
    pub id: String,
    pub kind: LlmProviderKind,
    /// Bedrock / Azure region
    #[serde(default)]
    pub region: Option<String>,
    /// AWS CLI profile name (Bedrock)
    #[serde(default)]
    pub profile: Option<String>,
    /// Azure AI Foundry or LiteLLM base URL
    #[serde(default)]
    pub endpoint: Option<String>,
    /// The model this provider serves by default
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoLlmConfig {
    pub providers: Vec<LlmProviderConfig>,
    /// A provider ID from `providers`
    pub default_provider: String,
    /// Provider IDs to try, in order, when the default isn't usable
    pub fallbacks: Vec<String>,
}

impl RepoLlmConfig {
    pub fn provider(&self, id: &str) -> Option<&LlmProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn default(&self) -> Option<&LlmProviderConfig> {
        self.provider(&self.default_provider)
    }
}

/// Where an effective value came from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ConfigSource {
    Environment,
    Repo,
    Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveValue {
    pub value: String,
    pub source: ConfigSource,
}

/// What `get_repo_llm_config` reports
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RepoLlmConfigStatus {
    /// Whether `.nandi/llm.yaml` exists
    pub found: bool,
    pub path: String,
    /// `None` when missing or invalid
    pub config: Option<RepoLlmConfig>,
    /// Why the file couldn't be used
    pub error: Option<String>,
    /// The default provider's settings after precedence
    pub effective_provider: Option<EffectiveValue>,
    pub effective_region: Option<EffectiveValue>,
    pub effective_profile: Option<EffectiveValue>,
    pub effective_model: Option<EffectiveValue>,
    /// When the file was last synced (epoch ms)
    pub synced_at: Option<Timestamp>,
}

impl NandiEvent for RepoLlmConfigStatus {
    const NAME: &'static str = REPO_LLM_CHANGED;
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LlmFile {
    #[serde(default)]
    providers: Vec<LlmProviderConfig>,
    #[serde(default)]
    routing: Routing,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Routing {
    default: Option<String>,
    #[serde(default)]
    fallbacks: Vec<String>,
}

fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Parses and checks `.nandi/llm.yaml`: at least one provider, unique IDs,
/// a default (`routing.default`, else the first provider) and fallbacks
/// that name providers, and the fields each kind needs
pub fn parse(yaml: &str) -> Result<RepoLlmConfig, String> {
    let file: LlmFile =
        serde_yaml::from_str(yaml).map_err(|e| format!("{REPO_LLM_FILE} isn't valid: {e}"))?;
    let mut ids = HashSet::new();
    let mut providers = Vec::new();
    for provider in file.providers {
        let provider = LlmProviderConfig {
            id: provider.id.trim().to_string(),
            region: trimmed(provider.region),
            profile: trimmed(provider.profile),
            endpoint: trimmed(provider.endpoint),
            model: trimmed(provider.model),
            ..provider
        };
        if provider.id.is_empty() {
            return Err(format!("{REPO_LLM_FILE}: a provider has no id"));
        }
        if !ids.insert(provider.id.clone()) {
            return Err(format!(
                "{REPO_LLM_FILE}: provider '{}' is listed twice",
                provider.id
            ));
        }
        let needs_endpoint = matches!(
            provider.kind,
            LlmProviderKind::Azure | LlmProviderKind::Litellm
        );
        if needs_endpoint && provider.endpoint.is_none() {
            return Err(format!(
                "{REPO_LLM_FILE}: provider '{}' ({}) needs an endpoint",
                provider.id,
                provider.kind.as_str()
            ));
        }
        providers.push(provider);
    }
    let Some(first) = providers.first() else {
        return Err(format!("{REPO_LLM_FILE} lists no providers"));
    };
    let default_provider = trimmed(file.routing.default).unwrap_or_else(|| first.id.clone());
    if !ids.contains(&default_provider) {
        return Err(format!(
            "{REPO_LLM_FILE}: routing.default '{default_provider}' isn't a provider"
        ));
    }
    let mut fallbacks = Vec::new();
    for fallback in file.routing.fallbacks {
        let fallback = fallback.trim().to_string();
        if !ids.contains(&fallback) {
            return Err(format!(
                "{REPO_LLM_FILE}: fallback '{fallback}' isn't a provider"
            ));
        }
        if fallback != default_provider && !fallbacks.contains(&fallback) {
            fallbacks.push(fallback);
        }
    }
    Ok(RepoLlmConfig {
        providers,
        default_provider,
        fallbacks,
    })
}

/// The file as last read
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    pub found: bool,
    pub config: Option<RepoLlmConfig>,
    pub error: Option<String>,
}

pub fn path_in(store: &RepoStore) -> PathBuf {
    store.layout().meta_dir.join("llm.yaml")
}

/// A parsed file and the mtime it was read at
type Cached = (Option<SystemTime>, Loaded);

/// Parsed files by path
static FILES: LazyLock<Mutex<HashMap<PathBuf, Cached>>> = LazyLock::new(Mutex::default);

fn read_file(path: &Path) -> Loaded {
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(config) => Loaded {
                found: true,
                config: Some(config),
                error: None,
            },
            Err(error) => {
                log::warn!("{error}");
                Loaded {
                    found: true,
                    config: None,
                    error: Some(error),
                }
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::default(),
        Err(e) => Loaded {
            found: true,
            config: None,
            error: Some(format!("Couldn't read {}: {e}", path.display())),
        },
    }
}

/// The file at `path`, re-read when its mtime changed (or `force`)
pub fn load(path: &Path, force: bool) -> Loaded {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    if !force {
        if let Some((at, loaded)) = files.get(path) {
            if *at == mtime {
                return loaded.clone();
            }
        }
    }
    let loaded = read_file(path);
    files.insert(path.to_path_buf(), (mtime, loaded.clone()));
    loaded
}

// ---------------------------------------------------------------------------
// Precedence
// ---------------------------------------------------------------------------

fn from_env(env: &dyn Fn(&str) -> Option<String>, names: &[&str]) -> Option<EffectiveValue> {
    names
        .iter()
        .find_map(|name| env(name).filter(|v| !v.trim().is_empty()))
        .map(|value| EffectiveValue {
            value: value.trim().to_string(),
            source: ConfigSource::Environment,
        })
}

fn from(value: Option<&String>, source: ConfigSource) -> Option<EffectiveValue> {
    value.map(|value| EffectiveValue {
        value: value.clone(),
        source,
    })
}

/// The default provider's settings after precedence
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effective {
    pub provider: Option<EffectiveValue>,
    pub region: Option<EffectiveValue>,
    pub profile: Option<EffectiveValue>,
    pub endpoint: Option<EffectiveValue>,
    pub model: Option<EffectiveValue>,
}

/// Environment > `config`'s default provider > `local_model` (the default
/// model from settings)
pub fn effective(
    config: Option<&RepoLlmConfig>,
    env: &dyn Fn(&str) -> Option<String>,
    local_model: Option<&str>,
) -> Effective {
    let repo = config.and_then(RepoLlmConfig::default);
    let kind = repo.map(|p| p.kind.as_str().to_string());
    let endpoint_env: &[&str] = match repo.map(|p| p.kind) {
        Some(LlmProviderKind::Azure) => &["ANTHROPIC_FOUNDRY_BASE_URL"],
        Some(LlmProviderKind::Litellm) => &["ANTHROPIC_BASE_URL"],
        _ => &[],
    };
    let local_model = local_model.map(str::to_string);
    Effective {
        provider: from_env(env, &[PROVIDER_ENV])
            .or_else(|| from(kind.as_ref(), ConfigSource::Repo)),
        region: from_env(env, &["AWS_REGION", "AWS_DEFAULT_REGION"])
            .or_else(|| from(repo.and_then(|p| p.region.as_ref()), ConfigSource::Repo)),
        profile: from_env(env, &["AWS_PROFILE"])
            .or_else(|| from(repo.and_then(|p| p.profile.as_ref()), ConfigSource::Repo)),
        endpoint: from_env(env, endpoint_env)
            .or_else(|| from(repo.and_then(|p| p.endpoint.as_ref()), ConfigSource::Repo)),
        model: from_env(env, &[MODEL_ENV])
            .or_else(|| from(repo.and_then(|p| p.model.as_ref()), ConfigSource::Repo))
            .or_else(|| from(local_model.as_ref(), ConfigSource::Local)),
    }
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Only what the repo decides (the environment didn't): `None` leaves a
/// value to whatever reads the environment or local config downstream
fn repo_only(value: Option<EffectiveValue>) -> Option<String> {
    value
        .filter(|v| v.source == ConfigSource::Repo)
        .map(|v| v.value)
}

/// The repo's choices for a harness session, where the environment hasn't
/// set them; each is used only where the agent's own `runtime_config` is
/// silent
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarnessDefaults {
    /// `bedrock`, `azure`, `anthropic`, `openai`, or `litellm`
    pub provider: Option<String>,
    pub aws_profile: Option<String>,
    pub aws_region: Option<String>,
    /// Azure AI Foundry or LiteLLM base URL
    pub endpoint: Option<String>,
    pub model: Option<String>,
}

pub fn harness_defaults_with(
    config: Option<&RepoLlmConfig>,
    env: &dyn Fn(&str) -> Option<String>,
) -> HarnessDefaults {
    let effective = effective(config, env, None);
    let kind = config.and_then(RepoLlmConfig::default).map(|p| p.kind);
    HarnessDefaults {
        provider: repo_only(effective.provider),
        aws_profile: repo_only(effective.profile)
            .filter(|_| kind == Some(LlmProviderKind::Bedrock)),
        aws_region: repo_only(effective.region),
        endpoint: repo_only(effective.endpoint),
        // A native provider's model is a Nandi ref, not a CLI model ID
        model: repo_only(effective.model)
            .filter(|_| kind.is_some_and(|k| k.native_prefix().is_none())),
    }
}

pub fn harness_defaults(store: &RepoStore) -> HarnessDefaults {
    let loaded = load(&path_in(store), false);
    harness_defaults_with(loaded.config.as_ref(), &process_env)
}

/// `model` as a native model ref for `kind` (`claude:…`, `gpt:…`,
/// `ollama:…`); `None` for providers native agents can't call
fn native_ref(kind: LlmProviderKind, model: &str) -> Option<String> {
    let prefix = kind.native_prefix()?;
    if crate::llm::parse_model(model).is_ok() {
        Some(model.to_string())
    } else {
        Some(format!("{prefix}:{model}"))
    }
}

/// The model native calls use by default: `NANDI_LLM_MODEL`, else the repo
/// default provider's model (when native agents can call it), else the
/// default model from settings
pub fn default_model_with(
    config: Option<&RepoLlmConfig>,
    env: &dyn Fn(&str) -> Option<String>,
    settings_model: String,
) -> String {
    if let Some(model) = env(MODEL_ENV).filter(|m| crate::llm::parse_model(m.trim()).is_ok()) {
        return model.trim().to_string();
    }
    config
        .and_then(RepoLlmConfig::default)
        .and_then(|p| native_ref(p.kind, p.model.as_deref()?))
        .unwrap_or(settings_model)
}

pub fn default_model(store: &RepoStore) -> String {
    let settings_model = store.read(|repo| repo.settings().default_model().to_string());
    let loaded = load(&path_in(store), false);
    default_model_with(loaded.config.as_ref(), &process_env, settings_model)
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Syncs the file into `repo_config_cache` and reports what's in effect
pub struct RepoConfigService {
    store: RepoStore,
    db: Db,
    bus: EventBus,
}

impl RepoConfigService {
    /// Never blocks: the file is read on first use
    pub fn new(store: RepoStore, db: Db, bus: EventBus) -> Self {
        Self { store, db, bus }
    }

    fn path(&self) -> PathBuf {
        path_in(&self.store)
    }

    async fn synced_at(&self) -> Result<Option<Timestamp>, String> {
        self.db
            .call(|conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT synced_at FROM repo_config_cache WHERE key = ?1",
                    [CACHE_KEY],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
            })
            .await
            .map(|at| at.map(db::ts))
    }

    fn status(&self, loaded: Loaded, synced_at: Option<Timestamp>) -> RepoLlmConfigStatus {
        let settings_model = self
            .store
            .read(|repo| repo.settings().default_model().to_string());
        let effective = effective(loaded.config.as_ref(), &process_env, Some(&settings_model));
        RepoLlmConfigStatus {
            found: loaded.found,
            path: self.path().display().to_string(),
            config: loaded.config,
            error: loaded.error,
            effective_provider: effective.provider,
            effective_region: effective.region,
            effective_profile: effective.profile,
            effective_model: effective.model,
            synced_at,
        }
    }

    /// What the file holds now and what's in effect
    pub async fn get(&self) -> Result<RepoLlmConfigStatus, String> {
        let loaded = load(&self.path(), false);
        Ok(self.status(loaded, self.synced_at().await?))
    }

    /// Re-reads the file, records it, and emits `config://repo_llm_changed`
    pub async fn reload(&self) -> Result<RepoLlmConfigStatus, String> {
        let loaded = load(&self.path(), true);
        let value = serde_json::to_string(&loaded.config).map_err(|e| e.to_string())?;
        let now = db::now_ms();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO repo_config_cache (key, value, synced_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                       synced_at = excluded.synced_at",
                    params![CACHE_KEY, value, now],
                )?;
                Ok(())
            })
            .await?;
        let status = self.status(loaded, Some(db::ts(now)));
        events::emit(&self.bus, &status);
        Ok(status)
    }
}

pub async fn get_repo_llm_config(state: &AppState) -> Result<RepoLlmConfigStatus, String> {
    state.repo_config().get().await
}

pub async fn reload_repo_llm_config(state: &AppState) -> Result<RepoLlmConfigStatus, String> {
    state.repo_config().reload().await
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACME: &str = r#"
providers:
  - id: bedrock-primary
    kind: bedrock
    region: us-east-1
    profile: acme-dev
    model: us.anthropic.claude-sonnet-5
  - id: gateway
    kind: litellm
    endpoint: https://llm.acme.internal
routing:
  default: bedrock-primary
  fallbacks: [gateway]
"#;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn parses_providers_and_routing() {
        let config = parse(ACME).unwrap();
        assert_eq!(config.default_provider, "bedrock-primary");
        assert_eq!(config.fallbacks, ["gateway"]);
        let default = config.default().unwrap();
        assert_eq!(default.kind, LlmProviderKind::Bedrock);
        assert_eq!(default.profile.as_deref(), Some("acme-dev"));
        // No routing: the first provider
        let first = parse("providers:\n  - id: a\n    kind: anthropic\n").unwrap();
        assert_eq!(first.default_provider, "a");
    }

    #[test]
    fn rejects_what_a_team_would_regret() {
        for (yaml, error) in [
            ("providers: []\n", "no providers"),
            ("providers:\n  - id: a\n    kind: vertex\n", "isn't valid"),
            (
                "providers:\n  - id: a\n    kind: bedrock\n    regoin: x\n",
                "isn't valid",
            ),
            (
                "providers:\n  - id: a\n    kind: bedrock\n  - id: a\n    kind: openai\n",
                "twice",
            ),
            (
                "providers:\n  - id: a\n    kind: litellm\n",
                "needs an endpoint",
            ),
            (
                "providers:\n  - id: a\n    kind: bedrock\nrouting:\n  default: b\n",
                "isn't a provider",
            ),
            (
                "providers:\n  - id: a\n    kind: bedrock\nrouting:\n  fallbacks: [z]\n",
                "fallback 'z'",
            ),
        ] {
            let got = parse(yaml).unwrap_err();
            assert!(got.contains(error), "{yaml}: {got}");
        }
    }

    #[test]
    fn the_environment_beats_the_repo_which_beats_local() {
        let config = parse(ACME).unwrap();
        let none = env(&[]);
        let repo = effective(Some(&config), &none, Some("claude:claude-sonnet-5"));
        assert_eq!(repo.provider.as_ref().unwrap().value, "bedrock");
        assert_eq!(repo.region.as_ref().unwrap().source, ConfigSource::Repo);
        assert_eq!(repo.profile.as_ref().unwrap().value, "acme-dev");
        assert_eq!(repo.model.as_ref().unwrap().source, ConfigSource::Repo);

        let set = env(&[("AWS_PROFILE", "mine"), ("AWS_REGION", "eu-west-1")]);
        let mine = effective(Some(&config), &set, None);
        assert_eq!(
            mine.profile,
            Some(EffectiveValue {
                value: "mine".into(),
                source: ConfigSource::Environment
            })
        );
        assert_eq!(mine.region.as_ref().unwrap().value, "eu-west-1");

        // No repo file: local settings
        let local = effective(None, &none, Some("claude:claude-sonnet-5"));
        assert_eq!(local.provider, None);
        assert_eq!(local.model.as_ref().unwrap().source, ConfigSource::Local);
    }

    #[test]
    fn harness_sessions_get_only_what_the_environment_left_open() {
        let config = parse(ACME).unwrap();
        let defaults = harness_defaults_with(Some(&config), &env(&[]));
        assert_eq!(defaults.provider.as_deref(), Some("bedrock"));
        assert_eq!(defaults.aws_profile.as_deref(), Some("acme-dev"));
        assert_eq!(defaults.aws_region.as_deref(), Some("us-east-1"));
        assert_eq!(
            defaults.model.as_deref(),
            Some("us.anthropic.claude-sonnet-5")
        );

        let defaults = harness_defaults_with(Some(&config), &env(&[("AWS_PROFILE", "mine")]));
        assert_eq!(
            defaults.aws_profile, None,
            "the AWS CLI reads AWS_PROFILE itself"
        );
        assert_eq!(defaults.provider.as_deref(), Some("bedrock"));

        let gateway = parse(&ACME.replace("default: bedrock-primary", "default: gateway")).unwrap();
        let defaults = harness_defaults_with(Some(&gateway), &env(&[]));
        assert_eq!(defaults.provider.as_deref(), Some("litellm"));
        assert_eq!(
            defaults.endpoint.as_deref(),
            Some("https://llm.acme.internal")
        );
        assert_eq!(
            harness_defaults_with(None, &env(&[])),
            HarnessDefaults::default()
        );
    }

    #[test]
    fn native_agents_use_a_native_default_providers_model() {
        let settings = || "claude:claude-sonnet-5".to_string();
        let openai =
            parse("providers:\n  - id: o\n    kind: openai\n    model: gpt-5-mini\n").unwrap();
        assert_eq!(
            default_model_with(Some(&openai), &env(&[]), settings()),
            "gpt:gpt-5-mini"
        );
        // Bedrock isn't callable natively: settings stay
        let bedrock = parse(ACME).unwrap();
        assert_eq!(
            default_model_with(Some(&bedrock), &env(&[]), settings()),
            settings()
        );
        // The environment wins (when it names a valid model)
        let set = env(&[(MODEL_ENV, "ollama:llama3")]);
        assert_eq!(
            default_model_with(Some(&openai), &set, settings()),
            "ollama:llama3"
        );
        let bad = env(&[(MODEL_ENV, "nonsense")]);
        assert_eq!(
            default_model_with(Some(&openai), &bad, settings()),
            "gpt:gpt-5-mini"
        );
    }

    #[tokio::test]
    async fn the_service_reads_syncs_and_reports_the_file() {
        use crate::services::events::tests::RecordingSink;
        let recorder = std::sync::Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let missing = get_repo_llm_config(&state).await.unwrap();
        assert!(!missing.found && missing.config.is_none() && missing.error.is_none());
        assert!(missing.path.ends_with("llm.yaml"));

        let path = path_in(state.store());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, ACME).unwrap();
        let synced = reload_repo_llm_config(&state).await.unwrap();
        assert!(synced.found && synced.error.is_none());
        assert_eq!(synced.config.unwrap().default_provider, "bedrock-primary");
        assert!(synced.synced_at.is_some());
        assert!(recorder.names().contains(&REPO_LLM_CHANGED));

        // An invalid edit is reported, not fatal
        std::fs::write(&path, "providers: [").unwrap();
        let broken = reload_repo_llm_config(&state).await.unwrap();
        assert!(broken.found && broken.config.is_none());
        assert!(broken.error.unwrap().contains("isn't valid"));
        state.shutdown();
    }
}
