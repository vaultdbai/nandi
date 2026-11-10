//! Provider-neutral LLM calls.
//!
//! The orchestrator and the container-facing proxy (`services::egress`) both go
//! through [`stream`] (or [`complete`]), which picks a provider from the
//! model prefix, injects the API key from the OS keychain, and writes an
//! audit entry.
//!
//! Model strings look like `deepseek:deepseek-flash`, `claude:claude-sonnet-5`,
//! `gpt:gpt-5`, or `ollama:llama3.2`.
//!
//! `claude:` models go through Claude Code when it can sign in (the team's
//! Bedrock choice or Claude Code's own login: `claude_code`, through the
//! Claude Agent SDK), else the Anthropic key. Keys come last.
//!
//! With nothing of the user's own, `deepseek:` calls run on the free trial
//! (`services::llm_trial`): Nandi's gateway, which holds the key and counts
//! the tokens.

mod anthropic;
pub mod claude_code;
pub(crate) mod deepseek;
#[cfg(test)]
pub(crate) use deepseek::BASE_URL_OVERRIDE as DEEPSEEK_BASE_URL_OVERRIDE;
mod ollama;
mod openai;
pub mod probe;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::audit::{self, AuditLog};
use crate::services::secrets::Secrets;
use crate::services::{llm_keys, llm_trial};

/// Generous: tool-heavy turns on large models can take minutes.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_MAX_TOKENS: u32 = 4096;
/// Streams may run long (thinking); this bounds silence between chunks.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// One conversation entry. This is also the wire format of the LLM proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        content: String,
    },
    Assistant {
        #[serde(default)]
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
        /// The model's thinking for this turn. DeepSeek requires it to be
        /// sent back on later requests when tools are in play.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
    /// The result of executing an assistant tool call
    Tool {
        tool_call_id: String,
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON object of arguments
    pub arguments: serde_json::Value,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema for the arguments object
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    /// `<provider>:<model>`, e.g. `claude:claude-sonnet-5`
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmResponse {
    /// Text output (may be empty when the model only calls tools)
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Usage,
    /// Thinking text, for providers that expose it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Estimated cost, for providers with known pricing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Input tokens served from the provider's prompt cache
    #[serde(default)]
    pub cache_hit_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    DeepSeek,
    Anthropic,
    OpenAi,
    Ollama,
}

/// Who initiated the call, for the audit log.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallSource {
    #[allow(dead_code)] // For the LLM TurnEngine, not connected while chat uses the canned engine
    Orchestrator,
    Proxy,
}

/// Splits `claude:claude-sonnet-5` into its provider and model name.
pub fn parse_model(model: &str) -> Result<(Provider, &str), String> {
    let (prefix, name) = model
        .split_once(':')
        .filter(|(_, name)| !name.trim().is_empty())
        .ok_or_else(|| {
            format!("Invalid model '{model}': expected '<provider>:<model>', e.g. 'claude:claude-sonnet-5'")
        })?;
    let provider = match prefix {
        "deepseek" => Provider::DeepSeek,
        "claude" => Provider::Anthropic,
        "gpt" => Provider::OpenAi,
        "ollama" => Provider::Ollama,
        other => {
            return Err(format!(
                "Unknown model provider '{other}': use 'deepseek:', 'claude:', 'gpt:', or 'ollama:'"
            ))
        }
    };
    Ok((provider, name))
}

/// For streamed responses: no overall deadline, but a bound on silence.
fn streaming_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(STREAM_IDLE_TIMEOUT)
            .build()
            .expect("failed to build HTTP client")
    })
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("failed to build HTTP client")
    })
}

#[derive(Serialize)]
struct LlmAuditEntry<'a> {
    ts: u64,
    source: CallSource,
    agent_id: Option<&'a str>,
    model: &'a str,
    status: &'static str,
    input_tokens: u32,
    output_tokens: u32,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// Runs one completion and audits it, without streaming text to a caller.
pub async fn complete(
    secrets: &Secrets,
    request: &LlmRequest,
    agent_id: Option<&str>,
    source: CallSource,
) -> Result<LlmResponse, String> {
    stream(secrets, request, agent_id, source, None).await
}

/// Runs one completion and audits it. Text deltas are sent to `text` as
/// they arrive (providers without streaming send the whole text once). The
/// full response is returned at the end.
///
/// Dropping the returned future aborts the HTTP request.
pub async fn stream(
    secrets: &Secrets,
    request: &LlmRequest,
    agent_id: Option<&str>,
    source: CallSource,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    let started = Instant::now();
    // The requested model if its key is set, else one the user has a key
    // for (`llm_keys::usable_model`)
    let resolved;
    let request = match llm_keys::usable_model(secrets, &request.model) {
        Ok(model) if model != request.model => {
            resolved = LlmRequest {
                model,
                ..request.clone()
            };
            &resolved
        }
        Ok(_) => request,
        Err(e) => return Err(e),
    };
    let result = dispatch(secrets, request, text).await;

    let (status, usage, cost_usd, error) = match &result {
        Ok(response) => ("ok", response.usage, response.cost_usd, None),
        Err(e) => ("error", Usage::default(), None, Some(e.as_str())),
    };
    audit::record(
        AuditLog::Llm,
        &LlmAuditEntry {
            ts: audit::now_ms(),
            source,
            agent_id,
            model: &request.model,
            status,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            duration_ms: started.elapsed().as_millis() as u64,
            cost_usd,
            error,
        },
    )
    .await;

    match &result {
        Ok(r) => log::debug!(
            "LLM {} ok: {} in / {} out tokens, {} tool calls",
            request.model,
            r.usage.input_tokens,
            r.usage.output_tokens,
            r.tool_calls.len()
        ),
        Err(e) => log::warn!("LLM {} failed: {e}", request.model),
    }
    result
}

async fn dispatch(
    secrets: &Secrets,
    request: &LlmRequest,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    let (provider, model) = parse_model(&request.model)?;
    let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    if provider == Provider::DeepSeek {
        // The user's own key, unless they turned the free trial back on
        if !llm_trial::chosen(secrets) {
            if let Ok(key) = llm_keys::require_api_key(secrets, provider) {
                let result =
                    deepseek::stream(streaming_client(), &key, model, request, max_tokens, text)
                        .await;
                return own_model(secrets, result);
            }
        }
        // The free trial, on its one model, through Nandi's gateway
        return llm_trial::chat(secrets, streaming_client(), request, max_tokens, text).await;
    }

    // The team's or Claude Code's own sign-in before a pasted key
    if provider == Provider::Anthropic && crate::services::harness::claude_sdk::usable() {
        return own_model(secrets, claude_code::stream(model, request, text).await);
    }

    let response = own_model(
        secrets,
        match provider {
            Provider::Anthropic => {
                let key = llm_keys::require_api_key(secrets, provider)?;
                anthropic::complete(http_client(), &key, model, request, max_tokens).await
            }
            Provider::OpenAi => {
                let key = llm_keys::require_api_key(secrets, provider)?;
                openai::complete(http_client(), &key, model, request, max_tokens).await
            }
            Provider::Ollama => ollama::complete(http_client(), model, request, max_tokens).await,
            Provider::DeepSeek => unreachable!("handled above"),
        },
    )?;
    // Non-streaming providers deliver their text in one piece
    if let Some(text) = text.filter(|_| !response.content.is_empty()) {
        let _ = text.send(response.content.clone()).await;
    }
    Ok(response)
}

/// A call on the user's own model (not the trial): once one works, an
/// untouched free trial turns off
fn own_model(
    secrets: &Secrets,
    result: Result<LlmResponse, String>,
) -> Result<LlmResponse, String> {
    if result.is_ok() {
        llm_trial::own_model_worked(secrets);
    }
    result
}

/// Reads an error body from a provider without echoing huge payloads.
async fn provider_error(provider: &str, response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|json| {
            // Anthropic/OpenAI: {"error": {"message": ...}}; Ollama: {"error": "..."}
            json.pointer("/error/message")
                .or_else(|| json.get("error"))
                .and_then(|v| v.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    format!("{provider} API error ({status}): {message}")
}

async fn send_json(
    provider: &str,
    request: reqwest::RequestBuilder,
) -> Result<serde_json::Value, String> {
    let response = request
        .send()
        .await
        .map_err(|e| format!("Could not reach {provider}: {e}"))?;
    if !response.status().is_success() {
        return Err(provider_error(provider, response).await);
    }
    response
        .json()
        .await
        .map_err(|e| format!("Unexpected response from {provider}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_prefixes() {
        assert_eq!(
            parse_model("claude:claude-sonnet-5").unwrap(),
            (Provider::Anthropic, "claude-sonnet-5")
        );
        assert_eq!(
            parse_model("gpt:gpt-5").unwrap(),
            (Provider::OpenAi, "gpt-5")
        );
        // Ollama tags contain a second colon
        assert_eq!(
            parse_model("ollama:llama3.2:3b").unwrap(),
            (Provider::Ollama, "llama3.2:3b")
        );
    }

    #[test]
    fn rejects_bad_models() {
        assert!(parse_model("claude-sonnet-5").is_err());
        assert!(parse_model("claude:").is_err());
        assert!(parse_model("gemini:pro").is_err());
    }

    #[test]
    fn message_wire_format_is_role_tagged() {
        let json = serde_json::json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "", "tool_calls": [
                {"id": "c1", "name": "shell", "arguments": {"cmd": "ls"}}
            ]},
            {"role": "tool", "tool_call_id": "c1", "name": "shell", "content": "a.txt"}
        ]);
        let messages: Vec<Message> = serde_json::from_value(json).unwrap();

        assert_eq!(messages.len(), 3);
        assert!(
            matches!(&messages[1], Message::Assistant { tool_calls, .. } if tool_calls[0].name == "shell")
        );
        assert!(matches!(
            &messages[2],
            Message::Tool {
                is_error: false,
                ..
            }
        ));
    }
}
