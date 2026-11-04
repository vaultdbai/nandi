//! Cheap checks of a model provider: does a key work, and is Ollama
//! running. Keys are checked by listing models, so no tokens are spent.
//! What a failure means for the user is decided in `services::llm_keys`.

use std::sync::OnceLock;
use std::time::Duration;

use super::Provider;

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// Ollama is on this computer: it answers at once or isn't running
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(2);
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models?limit=1";
/// The whole list, for suggestions (`model_ids`)
const ANTHROPIC_ALL_MODELS_URL: &str = "https://api.anthropic.com/v1/models?limit=1000";
const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";
/// Same host as `ollama::CHAT_URL`
const OLLAMA_TAGS_URL: &str = "http://localhost:11434/api/tags";
/// Error bodies are kept for Details only
const MAX_BODY_CHARS: usize = 300;

/// What a provider said to a probe
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Ok,
    /// A non-success HTTP status, with (the start of) its body
    Status {
        status: u16,
        body: String,
    },
    /// No answer: offline, DNS, TLS, or a timeout
    Unreachable(String),
}

fn probe_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .expect("failed to build HTTP client")
    })
}

/// Lists `provider`'s models with `api_key`. Ollama has no key and is
/// probed with [`ollama_models`] instead.
pub async fn list_models(provider: Provider, api_key: &str) -> ProbeOutcome {
    let client = probe_client();
    let request = match provider {
        Provider::Anthropic => client
            .get(ANTHROPIC_MODELS_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", super::anthropic::API_VERSION),
        Provider::OpenAi => client.get(OPENAI_MODELS_URL).bearer_auth(api_key),
        Provider::DeepSeek => client
            .get(format!("{}/models", super::deepseek::base_url()))
            .bearer_auth(api_key),
        Provider::Ollama => return ProbeOutcome::Ok,
    };
    match request.send().await {
        Ok(response) if response.status().is_success() => ProbeOutcome::Ok,
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            ProbeOutcome::Status {
                status,
                body: body.chars().take(MAX_BODY_CHARS).collect(),
            }
        }
        Err(e) => ProbeOutcome::Unreachable(e.to_string()),
    }
}

/// `provider`'s current models with `api_key`, newest first, chat models
/// only (`chat_model_ids`); a failure says what the provider answered
pub async fn model_ids(provider: Provider, api_key: &str) -> Result<Vec<String>, ProbeOutcome> {
    let client = probe_client();
    let request = match provider {
        Provider::Anthropic => client
            .get(ANTHROPIC_ALL_MODELS_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", super::anthropic::API_VERSION),
        Provider::OpenAi => client.get(OPENAI_MODELS_URL).bearer_auth(api_key),
        Provider::DeepSeek => client
            .get(format!("{}/models", super::deepseek::base_url()))
            .bearer_auth(api_key),
        Provider::Ollama => {
            return ollama_models()
                .await
                .ok_or_else(|| ProbeOutcome::Unreachable("Ollama isn't running".to_string()))
        }
    };
    let response = request
        .send()
        .await
        .map_err(|e| ProbeOutcome::Unreachable(e.to_string()))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(ProbeOutcome::Status {
            status,
            body: body.chars().take(MAX_BODY_CHARS).collect(),
        });
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ProbeOutcome::Unreachable(format!("Unreadable model list: {e}")))?;
    Ok(chat_model_ids(provider, &body))
}

/// Model IDs a chat can use, from a provider's `{"data": [{"id", …}]}`:
/// newest first (`created` / `created_at`, when given, else as listed),
/// and for OpenAI without embeddings, audio, images and moderation
fn chat_model_ids(provider: Provider, body: &serde_json::Value) -> Vec<String> {
    /// Kinds of OpenAI model that aren't for chat
    const NOT_CHAT: &[&str] = &[
        "embedding",
        "whisper",
        "tts",
        "audio",
        "realtime",
        "transcribe",
        "dall-e",
        "image",
        "moderation",
        "davinci",
        "babbage",
        "search",
        "computer-use",
    ];
    let mut models: Vec<(i64, String)> = body["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?.to_string();
                    let created = m["created"]
                        .as_i64()
                        .or_else(|| {
                            m["created_at"].as_str().and_then(|at| {
                                chrono::DateTime::parse_from_rfc3339(at)
                                    .ok()
                                    .map(|t| t.timestamp())
                            })
                        })
                        .unwrap_or(0);
                    Some((created, id))
                })
                .collect()
        })
        .unwrap_or_default();
    if provider == Provider::OpenAi {
        models.retain(|(_, id)| !NOT_CHAT.iter().any(|word| id.contains(word)));
    }
    // Stable: equal (unknown) dates keep the provider's order
    models.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
    models.into_iter().map(|(_, id)| id).collect()
}

/// The models installed in a local Ollama, or `None` when it isn't running.
pub async fn ollama_models() -> Option<Vec<String>> {
    let response = probe_client()
        .get(OLLAMA_TAGS_URL)
        .timeout(OLLAMA_TIMEOUT)
        .send()
        .await
        .ok()
        .filter(|r| r.status().is_success())?;
    let body: serde_json::Value = response.json().await.ok()?;
    Some(tag_names(&body))
}

/// `{"models": [{"name": "llama3.2:latest"}, …]}` → the names
fn tag_names(body: &serde_json::Value) -> Vec<String> {
    body["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_chat_models_newest_first() {
        let openai = serde_json::json!({ "data": [
            {"id": "gpt-4o", "created": 100},
            {"id": "text-embedding-3-large", "created": 300},
            {"id": "gpt-6-preview", "created": 400},
            {"id": "whisper-1", "created": 50},
            {"id": "gpt-4o-realtime-preview", "created": 350},
            {"id": "o4-mini", "created": 200},
        ]});
        assert_eq!(
            chat_model_ids(Provider::OpenAi, &openai),
            ["gpt-6-preview", "o4-mini", "gpt-4o"]
        );
        // Anthropic dates are RFC 3339; nothing is filtered out
        let anthropic = serde_json::json!({ "data": [
            {"id": "claude-older", "created_at": "2025-01-01T00:00:00Z"},
            {"id": "claude-newer", "created_at": "2026-06-01T00:00:00Z"},
        ]});
        assert_eq!(
            chat_model_ids(Provider::Anthropic, &anthropic),
            ["claude-newer", "claude-older"]
        );
        // No dates: the provider's order
        let deepseek =
            serde_json::json!({ "data": [{"id": "deepseek-chat"}, {"id": "deepseek-reasoner"}]});
        assert_eq!(
            chat_model_ids(Provider::DeepSeek, &deepseek),
            ["deepseek-chat", "deepseek-reasoner"]
        );
        assert!(chat_model_ids(Provider::OpenAi, &serde_json::json!({})).is_empty());
    }

    #[test]
    fn reads_ollama_tag_names() {
        let body = serde_json::json!({
            "models": [{"name": "llama3.2:latest", "size": 1}, {"name": "qwen3:8b"}, {"size": 2}]
        });
        assert_eq!(tag_names(&body), ["llama3.2:latest", "qwen3:8b"]);
        assert!(tag_names(&serde_json::json!({})).is_empty());
    }
}
