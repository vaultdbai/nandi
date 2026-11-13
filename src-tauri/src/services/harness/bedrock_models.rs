//! The Bedrock model catalog: foundation models and inference profiles for
//! a region, from Bedrock itself (`aws-sdk-bedrock`: ListFoundationModels +
//! ListInferenceProfiles, with the user's AWS identity), cached in
//! `bedrock_models_cache`, and annotated from the bundled
//! `bedrock_model_registry.json` (tool use, tested status, notes).
//!
//! Anthropic models run through Claude Code's native Bedrock support
//! (`CLAUDE_CODE_USE_BEDROCK`); third-party models need the LiteLLM
//! gateway (`requires_gateway`). Before the first refresh, the registry's
//! `known` models stand in.

use aws_sdk_bedrock::types::{InferenceType, ModelModality};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, ts, Db};
use crate::model::Timestamp;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;

const REGISTRY: &str = include_str!("bedrock_model_registry.json");
/// Cross-region inference profile prefixes (`us.anthropic.…`)
const PROFILE_PREFIXES: &[&str] = &[
    "us-gov.", "us.", "eu.", "apac.", "jp.", "au.", "ca.", "global.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TestedStatus {
    /// Verified end to end with a harness
    Tested,
    /// Should work; not verified
    Untested,
    /// Works with caveats (see `notes`)
    KnownIssues,
    /// Can't drive an agent (no tool use, embeddings, image-only)
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BedrockModelKind {
    Foundation,
    /// A cross-region (or application) inference profile, e.g. `us.anthropic…`
    InferenceProfile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BedrockModel {
    /// What the harness takes: a model ID or an inference profile ID
    pub id: String,
    pub name: String,
    /// `anthropic`, `meta`, `mistral`, `amazon`, `cohere`, `deepseek`, …
    pub provider: String,
    pub kind: BedrockModelKind,
    pub region: String,
    /// Claude: Claude Code's native Bedrock support
    pub is_anthropic: bool,
    /// Third-party: through the LiteLLM gateway
    pub requires_gateway: bool,
    pub streaming: bool,
    pub tool_use: bool,
    pub tested: TestedStatus,
    pub notes: Option<String>,
    pub fetched_at: Option<Timestamp>,
}

/// `bedrock://models_refreshed`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockModelsRefreshed {
    pub region: String,
    pub count: u32,
}

impl NandiEvent for BedrockModelsRefreshed {
    const NAME: &'static str = super::BEDROCK_MODELS_REFRESHED;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registry {
    rules: Vec<Rule>,
    known: Vec<Known>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rule {
    #[serde(rename = "match")]
    pattern: String,
    tested: TestedStatus,
    tool_use: bool,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Known {
    id: String,
    name: String,
}

fn registry() -> Registry {
    // Bundled and covered by a test: it always parses
    serde_json::from_str(REGISTRY).unwrap_or(Registry {
        rules: Vec::new(),
        known: Vec::new(),
    })
}

/// The provider in a model or profile ID: `us.meta.llama3…` → `meta`
pub fn provider_of(id: &str) -> String {
    let base = PROFILE_PREFIXES
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .unwrap_or(id);
    base.split('.').next().unwrap_or(base).to_lowercase()
}

/// A catalog entry, annotated from the registry
fn model(
    registry: &[Rule],
    id: &str,
    name: &str,
    kind: BedrockModelKind,
    region: &str,
    streaming: bool,
    fetched_at: Option<Timestamp>,
) -> BedrockModel {
    let provider = provider_of(id);
    let is_anthropic = provider == "anthropic";
    let rule = registry.iter().find(|r| id.contains(&r.pattern));
    BedrockModel {
        id: id.to_string(),
        name: name.to_string(),
        provider,
        kind,
        region: region.to_string(),
        is_anthropic,
        requires_gateway: !is_anthropic,
        streaming,
        tool_use: rule.is_none_or(|r| r.tool_use),
        tested: rule.map_or(TestedStatus::Untested, |r| r.tested),
        notes: rule.and_then(|r| r.notes.clone()),
        fetched_at,
    }
}

/// The offline catalog: the registry's known models
pub fn known_models(region: &str) -> Vec<BedrockModel> {
    let registry = registry();
    registry
        .known
        .iter()
        .map(|k| {
            let kind = if PROFILE_PREFIXES.iter().any(|p| k.id.starts_with(p)) {
                BedrockModelKind::InferenceProfile
            } else {
                BedrockModelKind::Foundation
            };
            model(&registry.rules, &k.id, &k.name, kind, region, true, None)
        })
        .collect()
}

pub struct BedrockModelService {
    db: Db,
    bus: EventBus,
}

impl BedrockModelService {
    pub fn new(db: Db, bus: EventBus) -> Self {
        Self { db, bus }
    }

    /// The cached catalog for `region`, else the known models
    pub async fn list(&self, region: &str) -> Result<Vec<BedrockModel>, String> {
        let key = region.to_string();
        let cached: Vec<BedrockModel> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT model_json FROM bedrock_models_cache WHERE region = ?1 ORDER BY name",
                )?;
                let rows = stmt
                    .query_map([&key], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?
            .iter()
            .filter_map(|json| serde_json::from_str(json).ok())
            .collect();
        Ok(if cached.is_empty() {
            known_models(region)
        } else {
            cached
        })
    }

    /// Asks Bedrock for `region`'s catalog with `sdk`'s identity and
    /// replaces the cache
    pub async fn refresh(
        &self,
        sdk: &aws_config::SdkConfig,
        region: &str,
    ) -> Result<Vec<BedrockModel>, String> {
        let client = aws_sdk_bedrock::Client::new(sdk);
        let registry = registry();
        let fetched_at = Some(ts(now_ms()));
        let mut models = Vec::new();

        let foundation = client
            .list_foundation_models()
            .by_output_modality(ModelModality::Text)
            .send()
            .await
            .map_err(|e| format!("Bedrock wouldn't list its models: {}", error_text(&e)))?;
        for summary in foundation.model_summaries() {
            // Models only reachable through an inference profile come below
            if !summary
                .inference_types_supported()
                .contains(&InferenceType::OnDemand)
            {
                continue;
            }
            models.push(model(
                &registry.rules,
                summary.model_id(),
                summary.model_name().unwrap_or(summary.model_id()),
                BedrockModelKind::Foundation,
                region,
                summary.response_streaming_supported().unwrap_or(false),
                fetched_at,
            ));
        }

        let mut next: Option<String> = None;
        loop {
            let page = client
                .list_inference_profiles()
                .set_next_token(next.take())
                .send()
                .await
                .map_err(|e| {
                    format!(
                        "Bedrock wouldn't list inference profiles: {}",
                        error_text(&e)
                    )
                })?;
            for profile in page.inference_profile_summaries() {
                models.push(model(
                    &registry.rules,
                    profile.inference_profile_id(),
                    profile.inference_profile_name(),
                    BedrockModelKind::InferenceProfile,
                    region,
                    true,
                    fetched_at,
                ));
            }
            match page.next_token() {
                Some(token) if !token.is_empty() => next = Some(token.to_string()),
                _ => break,
            }
        }
        models.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        models.dedup_by(|a, b| a.id == b.id);
        self.store(region, &models).await?;
        events::emit(
            &self.bus,
            &BedrockModelsRefreshed {
                region: region.to_string(),
                count: models.len() as u32,
            },
        );
        Ok(models)
    }

    async fn store(&self, region: &str, models: &[BedrockModel]) -> Result<(), String> {
        let region = region.to_string();
        let rows: Vec<(String, String, String, String, String)> = models
            .iter()
            .map(|m| {
                (
                    m.id.clone(),
                    format!("{:?}", m.kind).to_lowercase(),
                    m.provider.clone(),
                    m.name.clone(),
                    serde_json::to_string(m).unwrap_or_default(),
                )
            })
            .collect();
        self.db
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "DELETE FROM bedrock_models_cache WHERE region = ?1",
                    [&region],
                )?;
                for (id, kind, provider, name, json) in &rows {
                    tx.execute(
                        "INSERT INTO bedrock_models_cache
                             (region, model_id, kind, provider, name, model_json, fetched_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![region, id, kind, provider, name, json, now_ms()],
                    )?;
                }
                tx.commit()
            })
            .await
    }
}

fn error_text(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text = format!("{text}: {cause}");
        source = cause.source();
    }
    text
}

fn region_or_default(state: &AppState, region: Option<String>) -> String {
    region
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| state.aws_auth().default_region())
}

/// Cached models for `region` (`None`: the AWS default region); the
/// known models if nothing is cached
pub async fn list_bedrock_models(
    state: &AppState,
    region: Option<String>,
) -> Result<Vec<BedrockModel>, String> {
    let region = region_or_default(state, region);
    state.bedrock_models().list(&region).await
}

/// Asks Bedrock, replaces the cache, emits `bedrock://models_refreshed`
pub async fn refresh_bedrock_models(
    state: &AppState,
    region: Option<String>,
) -> Result<Vec<BedrockModel>, String> {
    let region = region_or_default(state, region);
    let aws = state.aws_auth();
    let sdk = aws.sdk_config(None, Some(region.clone())).await?;
    state.bedrock_models().refresh(&sdk, &region).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_registry_parses() {
        let registry: Registry = serde_json::from_str(REGISTRY).unwrap();
        assert!(!registry.rules.is_empty());
        assert!(!registry.known.is_empty());
        assert!(
            registry
                .rules
                .iter()
                .all(|r| r.tested != TestedStatus::Tested),
            "nothing is marked tested until it has been"
        );
    }

    #[test]
    fn models_are_normalized_by_provider_and_registry() {
        let rules = registry().rules;
        let claude = model(
            &rules,
            "us.anthropic.claude-sonnet-4-20250514-v1:0",
            "Claude Sonnet 4",
            BedrockModelKind::InferenceProfile,
            "us-east-1",
            true,
            None,
        );
        assert_eq!(claude.provider, "anthropic");
        assert!(claude.is_anthropic);
        assert!(!claude.requires_gateway);
        assert!(claude.tool_use);

        let llama = model(
            &rules,
            "meta.llama3-3-70b-instruct-v1:0",
            "Llama",
            BedrockModelKind::Foundation,
            "us-east-1",
            true,
            None,
        );
        assert_eq!(llama.provider, "meta");
        assert!(llama.requires_gateway);
        assert_eq!(llama.tested, TestedStatus::Untested);

        let r1 = model(
            &rules,
            "us.deepseek.r1-v1:0",
            "R1",
            BedrockModelKind::InferenceProfile,
            "us-east-1",
            true,
            None,
        );
        assert_eq!(r1.tested, TestedStatus::KnownIssues);
        assert!(!r1.tool_use);

        let embed = model(
            &rules,
            "amazon.titan-embed-text-v2:0",
            "Embed",
            BedrockModelKind::Foundation,
            "us-east-1",
            false,
            None,
        );
        assert_eq!(embed.tested, TestedStatus::Unsupported);

        assert_eq!(provider_of("global.anthropic.claude-x"), "anthropic");
        assert_eq!(provider_of("eu.mistral.pixtral"), "mistral");
        assert_eq!(provider_of("cohere.command-r-v1:0"), "cohere");
    }

    #[tokio::test]
    async fn the_known_models_stand_in_until_a_refresh_is_cached() {
        let (bus, _) = crate::services::events::tests::bus();
        let service = BedrockModelService::new(bus.db.clone(), bus);
        let offline = service.list("eu-west-1").await.unwrap();
        assert!(!offline.is_empty());
        assert!(offline
            .iter()
            .all(|m| m.region == "eu-west-1" && m.fetched_at.is_none()));

        let fetched = vec![model(
            &registry().rules,
            "amazon.nova-pro-v1:0",
            "Nova Pro",
            BedrockModelKind::Foundation,
            "eu-west-1",
            true,
            Some(1.0),
        )];
        service.store("eu-west-1", &fetched).await.unwrap();
        assert_eq!(service.list("eu-west-1").await.unwrap(), fetched);
        assert_ne!(
            service.list("us-east-1").await.unwrap(),
            fetched,
            "per region"
        );
    }
}
