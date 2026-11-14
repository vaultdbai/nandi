//! LLM provider API keys in the OS keychain.
//!
//! Clients can set or clear a key and ask whether one is configured, but can
//! never read a key back. Keys are used only in Rust (see `llm::dispatch`)
//! and never enter containers.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::llm::probe::{self, ProbeOutcome};
use crate::llm::Provider;
use crate::services::llm_trial::{self, TrialStatus};
use crate::services::secrets::Secrets;
use crate::services::AppState;

/// Providers that need an API key (Ollama runs locally without one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum KeyedProvider {
    DeepSeek,
    Anthropic,
    OpenAi,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmKeyStatus {
    pub provider: KeyedProvider,
    pub configured: bool,
}

impl KeyedProvider {
    const ALL: [KeyedProvider; 3] = [
        KeyedProvider::DeepSeek,
        KeyedProvider::Anthropic,
        KeyedProvider::OpenAi,
    ];

    /// The keyring entry, also referenced from `settings.yaml` `secret_refs`.
    pub fn keyring_user(self) -> &'static str {
        match self {
            KeyedProvider::DeepSeek => "nandi.llm.deepseek",
            KeyedProvider::Anthropic => "nandi.llm.anthropic",
            KeyedProvider::OpenAi => "nandi.llm.openai",
        }
    }

    /// The `secret_refs` key in `settings.yaml`, e.g. `deepseek_api_key`.
    pub fn setting_key(self) -> &'static str {
        match self {
            KeyedProvider::DeepSeek => "deepseek_api_key",
            KeyedProvider::Anthropic => "anthropic_api_key",
            KeyedProvider::OpenAi => "openai_api_key",
        }
    }

    pub fn all() -> [KeyedProvider; 3] {
        Self::ALL
    }

    fn label(self) -> &'static str {
        match self {
            KeyedProvider::DeepSeek => "DeepSeek",
            KeyedProvider::Anthropic => "Anthropic",
            KeyedProvider::OpenAi => "OpenAI",
        }
    }

    fn llm_provider(self) -> Provider {
        match self {
            KeyedProvider::DeepSeek => Provider::DeepSeek,
            KeyedProvider::Anthropic => Provider::Anthropic,
            KeyedProvider::OpenAi => Provider::OpenAi,
        }
    }
}

/// Why a key check failed, in terms the UI can explain and fix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum KeyTestFailure {
    /// The provider refused the key
    InvalidKey,
    /// The account has no credit or quota left
    NoQuota,
    RateLimited,
    /// No answer from the provider (offline, DNS, timeout, provider down)
    Unreachable,
    /// No key was given and none is stored
    NoKey,
    Other,
}

/// The result of checking one provider's key
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmKeyTest {
    pub ok: bool,
    pub latency_ms: u32,
    pub failure: Option<KeyTestFailure>,
    /// The provider's raw answer, for Details only
    pub detail: Option<String>,
}

/// Whether Ollama runs on this computer, and which models it has
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub running: bool,
    pub models: Vec<String>,
}

/// What a probe's answer means. `None` means the key works.
pub fn key_test_failure(outcome: &ProbeOutcome) -> Option<KeyTestFailure> {
    match outcome {
        ProbeOutcome::Ok => None,
        ProbeOutcome::Unreachable(_) => Some(KeyTestFailure::Unreachable),
        ProbeOutcome::Status { status, body } => {
            let body = body.to_ascii_lowercase();
            let out_of_credit = body.contains("quota")
                || body.contains("credit balance")
                || body.contains("insufficient balance");
            Some(match status {
                401 | 403 => KeyTestFailure::InvalidKey,
                402 => KeyTestFailure::NoQuota,
                // OpenAI reports an empty account as 429 insufficient_quota
                429 if out_of_credit => KeyTestFailure::NoQuota,
                429 => KeyTestFailure::RateLimited,
                // Anthropic: 400 "Your credit balance is too low"
                400 if out_of_credit => KeyTestFailure::NoQuota,
                500..=599 => KeyTestFailure::Unreachable,
                _ => KeyTestFailure::Other,
            })
        }
    }
}

fn outcome_detail(outcome: &ProbeOutcome) -> Option<String> {
    match outcome {
        ProbeOutcome::Ok => None,
        ProbeOutcome::Unreachable(error) => Some(error.clone()),
        ProbeOutcome::Status { status, body } => Some(format!("HTTP {status}: {body}")),
    }
}

/// Checks `api_key` (or, when `None`, the stored key) against `provider`
/// without storing anything. Saving is a separate step, so a key that
/// fails is never kept.
pub async fn test_llm_key(
    state: &AppState,
    provider: KeyedProvider,
    api_key: Option<String>,
) -> Result<LlmKeyTest, String> {
    let key = match api_key.map(|key| key.trim().to_string()) {
        Some(key) if !key.is_empty() => key,
        _ => match state
            .secrets()
            .get(provider.keyring_user())?
            .filter(|key| !key.trim().is_empty())
        {
            Some(key) => key,
            None => {
                return Ok(LlmKeyTest {
                    ok: false,
                    latency_ms: 0,
                    failure: Some(KeyTestFailure::NoKey),
                    detail: None,
                })
            }
        },
    };
    let started = std::time::Instant::now();
    let outcome = probe::list_models(provider.llm_provider(), &key).await;
    let failure = key_test_failure(&outcome);
    log::info!(
        "Checked the {} key: {}",
        provider.label(),
        failure.map_or("ok".to_string(), |f| format!("{f:?}"))
    );
    Ok(LlmKeyTest {
        ok: failure.is_none(),
        latency_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
        failure,
        detail: outcome_detail(&outcome),
    })
}

/// `provider`'s current models, fetched with the stored key, newest first:
/// the default-model field suggests them, so they're never older than the
/// provider's own list. Without a key, or when the provider can't be
/// reached, the field falls back to its built-in suggestions.
pub async fn list_provider_models(
    state: &AppState,
    provider: KeyedProvider,
) -> Result<Vec<String>, String> {
    let key = state
        .secrets()
        .get(provider.keyring_user())?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| format!("No {} key yet", provider.label()))?;
    probe::model_ids(provider.llm_provider(), &key)
        .await
        .map_err(|outcome| match outcome {
            probe::ProbeOutcome::Status { status, .. } => {
                format!("{} answered {status} for its model list", provider.label())
            }
            probe::ProbeOutcome::Unreachable(e) => {
                format!("Couldn't reach {}: {e}", provider.label())
            }
            probe::ProbeOutcome::Ok => String::new(),
        })
}

/// Looks for a local Ollama (`localhost:11434`)
pub async fn detect_ollama() -> OllamaStatus {
    match probe::ollama_models().await {
        Some(models) => OllamaStatus {
            running: true,
            models,
        },
        None => OllamaStatus {
            running: false,
            models: Vec::new(),
        },
    }
}

/// The stored key for `provider`, or a message telling the user to add one.
pub fn require_api_key(secrets: &Secrets, provider: Provider) -> Result<String, String> {
    let keyed = match provider {
        Provider::DeepSeek => KeyedProvider::DeepSeek,
        Provider::Anthropic => KeyedProvider::Anthropic,
        Provider::OpenAi => KeyedProvider::OpenAi,
        Provider::Ollama => return Err("Ollama does not use an API key".to_string()),
    };
    secrets
        .get(keyed.keyring_user())?
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            format!(
                "{} API key not configured. Add it in Settings › Models & runtimes.",
                keyed.label()
            )
        })
}

/// Whether `provider`'s models can be called: a key, or for Anthropic the
/// user's own Claude Code sign-in (`llm::claude_code`)
pub fn has_access(secrets: &Secrets, provider: KeyedProvider) -> bool {
    has_key(secrets, provider)
        || (provider == KeyedProvider::Anthropic && crate::services::harness::claude_sdk::usable())
}

/// What every model call falls back to, in order, when the model it asked
/// for has no key: the first provider the user has set a key for (Claude
/// Code's own sign-in counts for Anthropic)
const FALLBACK_MODELS: [(KeyedProvider, &str); 3] = [
    (KeyedProvider::Anthropic, "claude:claude-sonnet-5"),
    (KeyedProvider::OpenAi, "gpt:gpt-5"),
    (KeyedProvider::DeepSeek, "deepseek:deepseek-flash"),
];

const NO_KEY: &str = "No LLM API key is set. Add one (Anthropic, OpenAI, or DeepSeek) in Settings › Models & runtimes, or sign in to Claude Code (`claude`) to use it for every model call.";

fn has_key(secrets: &Secrets, provider: KeyedProvider) -> bool {
    secrets
        .get(provider.keyring_user())
        .ok()
        .flatten()
        .is_some_and(|key| !key.trim().is_empty())
}

/// The model to call: `preferred` (an agent's model, or the default
/// model from settings) when its provider has a key (Ollama needs none),
/// else a model of the first provider the user has set a key for, else the
/// free trial (`llm_trial`) while the gateway offers it. A signed-in Claude
/// Code stands in for an Anthropic key, so with nothing else set every
/// call runs on Claude Code.
///
/// The trial is the last resort: the user's own key or sign-in always
/// wins, even with the trial switched on. It serves only a call nothing of
/// the user's can (owner decision 8, 2026-11-14).
pub fn usable_model(secrets: &Secrets, preferred: &str) -> Result<String, String> {
    resolve_model(secrets, preferred).map(|(model, _)| model)
}

/// [`usable_model`], and whether the model it picked runs on the trial.
/// It only reads: the trial turns off once a call on the user's own model
/// works (`llm_trial::own_model_worked`, from `llm::dispatch`).
fn resolve_model(secrets: &Secrets, preferred: &str) -> Result<(String, bool), String> {
    let provider = crate::llm::parse_model(preferred)?.0;
    let keyed = match provider {
        Provider::DeepSeek => KeyedProvider::DeepSeek,
        Provider::Anthropic => KeyedProvider::Anthropic,
        Provider::OpenAi => KeyedProvider::OpenAi,
        Provider::Ollama => return Ok((preferred.to_string(), false)),
    };
    if has_access(secrets, keyed) {
        return Ok((preferred.to_string(), false));
    }
    if let Some((provider, model)) = FALLBACK_MODELS
        .iter()
        .find(|(provider, _)| has_access(secrets, *provider))
    {
        log::info!(
            "No {} key for {preferred}; using {model} ({} key)",
            keyed.label(),
            provider.label()
        );
        return Ok((model.to_string(), false));
    }
    if llm_trial::available(secrets) {
        return Ok((llm_trial::TRIAL_MODEL.to_string(), true));
    }
    Err(llm_trial::why_unavailable(secrets)
        .unwrap_or(NO_KEY)
        .to_string())
}

/// Stores `api_key` for `provider` in the keyring (never in the repo;
/// `settings.yaml` only names the entry). An empty key removes it.
pub async fn set_llm_api_key(
    state: &AppState,
    provider: KeyedProvider,
    api_key: String,
) -> Result<(), String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        state.secrets().delete(provider.keyring_user())?;
        log::info!("Removed {} API key", provider.label());
        return Ok(());
    }
    state.secrets().set(provider.keyring_user(), api_key)?;
    log::info!("Stored {} API key", provider.label());
    llm_trial::own_model_set_up(state.secrets());
    Ok(())
}

/// The default model from settings (`<provider>:<model>`) and the model
/// calls actually use with the keys set now
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DefaultModelStatus {
    pub default_model: String,
    /// `None` when no usable key is set
    pub effective_model: Option<String>,
    /// How `claude:` calls sign in when they go through Claude Code (the
    /// team's Bedrock profile, or Claude Code's own login): an Anthropic
    /// key isn't needed then. `None`: they need the key.
    pub claude_sign_in: Option<crate::services::harness::HarnessAuth>,
    /// The free trial's state; `None` when it isn't set up (tests)
    pub trial: Option<TrialStatus>,
}

pub async fn get_default_model(state: &AppState) -> Result<DefaultModelStatus, String> {
    let default_model = state
        .store()
        .read(|repo| repo.settings().default_model().to_string());
    // `.nandi/llm.yaml` (or `NANDI_LLM_MODEL`) may override settings
    let chosen = crate::services::orchestrator::default_model(state.store());
    let resolved = resolve_model(state.secrets(), &chosen).ok();
    let on_trial = resolved.as_ref().is_some_and(|(_, trial)| *trial);
    Ok(DefaultModelStatus {
        default_model,
        effective_model: resolved.map(|(model, _)| model),
        claude_sign_in: claude_sign_in(),
        trial: llm_trial::status(state.secrets(), on_trial),
    })
}

/// The sign-in `claude:` calls use through Claude Code, if they can
fn claude_sign_in() -> Option<crate::services::harness::HarnessAuth> {
    use crate::services::harness::{claude_sdk, detection, HarnessAuth, HarnessAuthKind};
    if !claude_sdk::usable() {
        return None;
    }
    match claude_sdk::bedrock_route() {
        Some(route) => Some(HarnessAuth {
            kind: HarnessAuthKind::Bedrock,
            aws_profile: route.aws_profile,
            aws_region: route.aws_region,
            ..HarnessAuth::default()
        }),
        None => detection::auth_of("claude-code", &detection::DetectEnv::current()),
    }
}

/// Sets the default model in `settings.yaml` (portable; committed with
/// the repo like any other setting)
pub async fn set_default_model(
    state: &AppState,
    model: String,
) -> Result<DefaultModelStatus, String> {
    let model_ref =
        crate::services::domain::ModelRef::parse(model.trim()).map_err(|e| e.to_string())?;
    // Choosing a local model is setting up a model of one's own
    if model.trim().starts_with("ollama:") {
        llm_trial::own_model_set_up(state.secrets());
    }
    let store = state.store().clone();
    tokio::task::spawn_blocking(move || {
        use crate::services::repos::SingletonRepository;
        let mut settings = store.read(|repo| repo.settings().clone());
        settings.set_default_model(model_ref);
        store.repos().settings.save(&settings)?;
        store.commit(crate::services::domain::EntityKind::Settings, "settings")?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Saving the default model failed: {e}"))??;
    get_default_model(state).await
}

/// Turns the free trial on or off. On works only while tokens are left.
/// The default model stays as it is: the trial serves only the calls that
/// nothing of the user's can ([`usable_model`]).
pub async fn set_free_trial(state: &AppState, enabled: bool) -> Result<DefaultModelStatus, String> {
    llm_trial::set_enabled(state.secrets(), enabled)?;
    get_default_model(state).await
}

/// The free trial's allowance as the gateway counts it (`GET /v1/trial`)
pub async fn get_free_trial_account(state: &AppState) -> Result<llm_trial::TrialAccount, String> {
    llm_trial::account(state).await
}

/// Which providers have a key configured. Never returns the keys.
pub async fn get_llm_key_status(state: &AppState) -> Result<Vec<LlmKeyStatus>, String> {
    KeyedProvider::ALL
        .into_iter()
        .map(|provider| {
            let configured = state
                .secrets()
                .get(provider.keyring_user())?
                .is_some_and(|key| !key.is_empty());
            Ok(LlmKeyStatus {
                provider,
                configured,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keys_are_write_only_and_clearable() {
        let state = AppState::for_tests(vec![]);

        set_llm_api_key(&state, KeyedProvider::OpenAi, "  sk-test  ".into())
            .await
            .unwrap();
        let status = get_llm_key_status(&state).await.unwrap();
        assert!(status
            .iter()
            .any(|s| s.provider == KeyedProvider::OpenAi && s.configured));
        assert!(status
            .iter()
            .any(|s| s.provider == KeyedProvider::Anthropic && !s.configured));
        assert_eq!(
            require_api_key(state.secrets(), Provider::OpenAi).unwrap(),
            "sk-test"
        );

        set_llm_api_key(&state, KeyedProvider::OpenAi, String::new())
            .await
            .unwrap();
        assert!(require_api_key(state.secrets(), Provider::OpenAi)
            .unwrap_err()
            .contains("OpenAI API key not configured"));
    }

    #[test]
    fn the_preferred_model_is_used_first_then_any_provider_with_a_key() {
        let secrets = Secrets::in_memory();
        // No key at all: a clear error
        assert!(usable_model(&secrets, "deepseek:deepseek-flash")
            .unwrap_err()
            .contains("No LLM API key is set"));
        // Ollama needs none
        assert_eq!(
            usable_model(&secrets, "ollama:llama3.2").unwrap(),
            "ollama:llama3.2"
        );

        // Only an OpenAI key: a DeepSeek default falls back to it
        secrets
            .set(KeyedProvider::OpenAi.keyring_user(), "sk-o")
            .unwrap();
        assert_eq!(
            usable_model(&secrets, "deepseek:deepseek-flash").unwrap(),
            "gpt:gpt-5"
        );
        assert_eq!(usable_model(&secrets, "gpt:gpt-4o").unwrap(), "gpt:gpt-4o");

        // The preferred model wins once its own key is set
        secrets
            .set(KeyedProvider::DeepSeek.keyring_user(), "sk-d")
            .unwrap();
        assert_eq!(
            usable_model(&secrets, "deepseek:deepseek-chat").unwrap(),
            "deepseek:deepseek-chat"
        );
        // Fallback order: Anthropic, OpenAI, DeepSeek
        secrets
            .set(KeyedProvider::Anthropic.keyring_user(), "sk-a")
            .unwrap();
        secrets
            .delete(KeyedProvider::DeepSeek.keyring_user())
            .unwrap();
        assert_eq!(
            usable_model(&secrets, "deepseek:deepseek-chat").unwrap(),
            "claude:claude-sonnet-5"
        );
    }

    #[test]
    fn a_signed_in_claude_code_stands_in_for_an_anthropic_key() {
        let secrets = Secrets::in_memory();
        crate::services::harness::claude_sdk::set_usable_for_tests(true);
        // Claude models run on it as they are
        assert_eq!(
            usable_model(&secrets, "claude:claude-opus-5").unwrap(),
            "claude:claude-opus-5"
        );
        // With nothing else set, every call runs on it
        assert_eq!(
            usable_model(&secrets, "deepseek:deepseek-flash").unwrap(),
            "claude:claude-sonnet-5"
        );
        // A key for the preferred provider still wins
        secrets
            .set(KeyedProvider::DeepSeek.keyring_user(), "sk-d")
            .unwrap();
        assert_eq!(
            usable_model(&secrets, "deepseek:deepseek-flash").unwrap(),
            "deepseek:deepseek-flash"
        );
        crate::services::harness::claude_sdk::set_usable_for_tests(false);
    }

    /// Where a model call is served from
    #[derive(Debug, PartialEq)]
    enum Source {
        /// The user's own key or sign-in (or Ollama), on this model
        Own(String),
        /// Nandi's free trial
        FreeTrial,
        /// Nothing: the call fails with a message
        Nothing,
    }

    fn own(model: &str) -> Source {
        Source::Own(model.to_string())
    }

    /// Where [`resolve_model`] sends a call for `preferred`
    fn source_for(secrets: &Secrets, preferred: &str) -> Source {
        match resolve_model(secrets, preferred) {
            Ok((model, false)) => Source::Own(model),
            Ok((model, true)) => {
                assert_eq!(model, llm_trial::TRIAL_MODEL, "the trial runs one model");
                Source::FreeTrial
            }
            Err(_) => Source::Nothing,
        }
    }

    const ON: bool = true;
    const OFF: bool = false;
    const SIGNED_IN: bool = true;
    const NO_SIGN_IN: bool = false;

    /// Owner decision 8: the user's own key or sign-in always wins, even
    /// with the trial switched on. The trial serves only what nothing of
    /// theirs can.
    #[test]
    fn the_free_trial_is_the_last_resort() {
        use KeyedProvider::{Anthropic, DeepSeek, OpenAi};
        use Source::{FreeTrial, Nothing};
        let flash = llm_trial::TRIAL_MODEL;

        // (model asked for, own keys, Claude Code sign-in, trial switch, expected source)
        #[rustfmt::skip]
        let cases: [(&str, &[KeyedProvider], bool, bool, Source); 13] = [
            // An own key for the model asked for wins, trial on or off
            (flash,                  &[DeepSeek],          NO_SIGN_IN, ON,  own(flash)),
            (flash,                  &[DeepSeek],          NO_SIGN_IN, OFF, own(flash)),
            ("gpt:gpt-4o",           &[OpenAi],            NO_SIGN_IN, ON,  own("gpt:gpt-4o")),
            // Another provider's own key wins over the trial
            (flash,                  &[OpenAi],            NO_SIGN_IN, ON,  own("gpt:gpt-5")),
            (flash,                  &[Anthropic, OpenAi], NO_SIGN_IN, ON,  own("claude:claude-sonnet-5")),
            // An own sign-in wins over the trial
            ("claude:claude-opus-5", &[],                  SIGNED_IN,  ON,  own("claude:claude-opus-5")),
            (flash,                  &[],                  SIGNED_IN,  ON,  own("claude:claude-sonnet-5")),
            (flash,                  &[],                  SIGNED_IN,  OFF, own("claude:claude-sonnet-5")),
            // A key for the model asked for wins over a sign-in too
            (flash,                  &[DeepSeek],          SIGNED_IN,  ON,  own(flash)),
            // A local model needs nothing, so the trial never serves it
            ("ollama:llama3.2",      &[],                  NO_SIGN_IN, ON,  own("ollama:llama3.2")),
            // Nothing of the user's: the trial serves the call while it's on
            ("claude:claude-opus-5", &[],                  NO_SIGN_IN, ON,  FreeTrial),
            (flash,                  &[],                  NO_SIGN_IN, ON,  FreeTrial),
            (flash,                  &[],                  NO_SIGN_IN, OFF, Nothing),
        ];

        // A trial nobody calls here: resolving a model never reaches it
        llm_trial::use_test_trial(Some(std::sync::Arc::new(llm_trial::Trial::new(
            "http://127.0.0.1:9".into(),
            "test-instance".into(),
        ))));
        for (preferred, keys, sign_in, trial_on, expected) in cases {
            let secrets = Secrets::in_memory();
            for provider in keys {
                secrets.set(provider.keyring_user(), "sk-own").unwrap();
            }
            crate::services::harness::claude_sdk::set_usable_for_tests(sign_in);
            llm_trial::set_enabled(&secrets, trial_on).unwrap();
            assert_eq!(
                source_for(&secrets, preferred),
                expected,
                "{preferred} with keys {keys:?}, sign-in {sign_in}, trial on {trial_on}"
            );
        }
        crate::services::harness::claude_sdk::set_usable_for_tests(false);
        llm_trial::use_test_trial(None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_default_model_is_set_in_settings() {
        let state = AppState::for_tests(vec![]);
        set_llm_api_key(&state, KeyedProvider::OpenAi, "sk-o".into())
            .await
            .unwrap();
        let status = set_default_model(&state, "gpt:gpt-4o".into())
            .await
            .unwrap();
        assert_eq!(status.default_model, "gpt:gpt-4o");
        assert_eq!(status.effective_model.as_deref(), Some("gpt:gpt-4o"));
        // Without its key, calls use another provider's
        let status = set_default_model(&state, "claude:claude-opus-5".into())
            .await
            .unwrap();
        assert_eq!(status.effective_model.as_deref(), Some("gpt:gpt-5"));
        assert!(set_default_model(&state, "nonsense".into()).await.is_err());
        state.shutdown();
    }

    fn status(status: u16, body: &str) -> ProbeOutcome {
        ProbeOutcome::Status {
            status,
            body: body.to_string(),
        }
    }

    #[test]
    fn key_test_failures_map_every_provider_answer() {
        use KeyTestFailure::*;
        let rows: [(ProbeOutcome, Option<KeyTestFailure>); 11] = [
            (ProbeOutcome::Ok, None),
            (
                ProbeOutcome::Unreachable("dns error".into()),
                Some(Unreachable),
            ),
            (status(401, "invalid x-api-key"), Some(InvalidKey)),
            (status(403, "forbidden"), Some(InvalidKey)),
            (status(402, "Insufficient Balance"), Some(NoQuota)),
            (
                status(429, r#"{"error":{"code":"insufficient_quota"}}"#),
                Some(NoQuota),
            ),
            (status(429, "rate limit exceeded"), Some(RateLimited)),
            (status(400, "Your credit balance is too low"), Some(NoQuota)),
            (status(400, "bad request"), Some(Other)),
            (status(503, "overloaded"), Some(Unreachable)),
            (status(404, "not found"), Some(Other)),
        ];
        for (outcome, expected) in rows {
            assert_eq!(key_test_failure(&outcome), expected, "{outcome:?}");
        }
    }

    #[tokio::test]
    async fn testing_without_a_key_reports_no_key_and_stores_nothing() {
        let state = AppState::for_tests(vec![]);
        let result = test_llm_key(&state, KeyedProvider::Anthropic, Some("  ".into()))
            .await
            .unwrap();
        assert_eq!(
            result,
            LlmKeyTest {
                ok: false,
                latency_ms: 0,
                failure: Some(KeyTestFailure::NoKey),
                detail: None,
            }
        );
        let status = get_llm_key_status(&state).await.unwrap();
        assert!(status.iter().all(|s| !s.configured));
    }

    #[test]
    fn keyring_entries_are_valid_secret_refs() {
        for provider in KeyedProvider::all() {
            let entry = provider.keyring_user();
            assert!(
                crate::services::domain::settings::SecretRef::parse(entry).is_ok(),
                "{entry} must be a valid secret ref"
            );
        }
    }
}
