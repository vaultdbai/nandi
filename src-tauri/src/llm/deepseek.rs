//! DeepSeek Chat Completions adapter (OpenAI-compatible, streamed).
//!
//! `POST https://api.deepseek.com/chat/completions` with `stream: true` and
//! `stream_options.include_usage`. The response is server-sent events:
//! `data: {chunk}` lines (plus `: keep-alive` comments) ending in
//! `data: [DONE]`. Text arrives in `delta.content`, thinking in
//! `delta.reasoning_content`, and tool calls in `delta.tool_calls`
//! fragments keyed by `index` (id/name once, `arguments` as string pieces).
//!
//! Thinking mode is on by default; with tools, every earlier
//! `reasoning_content` must be sent back or the API returns 400, so it is
//! carried on [`Message::Assistant`].

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{LlmRequest, LlmResponse, Message, ToolCall, ToolSpec, Usage};

const BASE_URL: &str = "https://api.deepseek.com";
/// Retries for 429 / 500 / 503 before any output has streamed
const MAX_RETRIES: u32 = 2;
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// Longest `Retry-After` we'll honor; beyond this, report the rate limit
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// Lets integration tests point the provider at a mock server.
#[cfg(test)]
pub(crate) static BASE_URL_OVERRIDE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(super) fn base_url() -> String {
    #[cfg(test)]
    if let Some(url) = BASE_URL_OVERRIDE.lock().unwrap().clone() {
        return url;
    }
    BASE_URL.to_string()
}

pub async fn stream(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    request: &LlmRequest,
    max_tokens: u32,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    stream_from(
        &base_url(),
        client,
        api_key,
        model,
        request,
        max_tokens,
        text,
    )
    .await
}

async fn stream_from(
    base_url: &str,
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    request: &LlmRequest,
    max_tokens: u32,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    let body = request_body(model, request, max_tokens);
    let url = format!("{base_url}/chat/completions");

    let mut attempt = 0;
    let response = loop {
        let response = client
            .post(&url)
            .bearer_auth(api_key)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Could not reach DeepSeek: {e}"))?;

        let status = response.status().as_u16();
        if response.status().is_success() {
            break response;
        }
        let retryable = matches!(status, 429 | 500 | 503);
        if retryable && attempt < MAX_RETRIES {
            let delay = retry_after(&response).unwrap_or(RETRY_DELAYS[attempt as usize]);
            if delay <= MAX_RETRY_AFTER {
                log::warn!("DeepSeek returned {status}; retrying in {delay:?}");
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
        }
        let body = response.text().await.unwrap_or_default();
        return Err(error_message(status, &body));
    };

    read_stream(response, model, text).await
}

/// Reads a streamed answer (server-sent events) into a response, sending
/// text deltas to `text` as they arrive. Also reads the free trial's
/// gateway (`services::llm_trial`), which passes DeepSeek's stream through
/// unchanged.
pub(crate) async fn read_stream(
    response: reqwest::Response,
    model: &str,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    let mut state = StreamState::default();
    let mut lines = LineBuffer::default();
    let mut bytes = response.bytes_stream();
    'read: while let Some(chunk) = bytes.next().await {
        let chunk = chunk.map_err(|e| format!("DeepSeek stream interrupted: {e}"))?;
        for line in lines.push(&chunk) {
            match parse_line(&line)? {
                SseLine::Done => break 'read,
                SseLine::Ignore => {}
                SseLine::Chunk(chunk) => {
                    if let Some(delta) = state.apply(&chunk) {
                        if let Some(text) = &text {
                            // A closed receiver just means nobody is watching
                            let _ = text.send(delta).await;
                        }
                    }
                }
            }
        }
    }

    Ok(state.finish(model, SystemTime::now()))
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// User-facing messages for DeepSeek's documented error codes.
fn error_message(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| {
            json.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(200).collect());
    match status {
        401 => "DeepSeek rejected the API key (401). Check it in Preferences → Advanced.".to_string(),
        402 => "Your DeepSeek account is out of balance (402). Top up at platform.deepseek.com, then try again.".to_string(),
        429 => "DeepSeek is rate-limiting requests (429). Wait a moment and try again.".to_string(),
        400 | 422 => format!("DeepSeek rejected the request ({status}): {detail}"),
        500 | 503 => format!("DeepSeek is unavailable right now ({status}). Try again shortly."),
        _ => format!("DeepSeek API error ({status}): {detail}"),
    }
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// DeepSeek function names: ≤128 chars of `[A-Za-z0-9_-]`.
fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// MCP tool definitions as OpenAI-style function tools.
fn tools(specs: &[ToolSpec]) -> Vec<Value> {
    specs
        .iter()
        .filter(|tool| {
            let ok = valid_tool_name(&tool.name);
            if !ok {
                log::warn!(
                    "Skipping tool '{}': name not accepted by DeepSeek",
                    tool.name
                );
            }
            ok
        })
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                },
            })
        })
        .collect()
}

fn message(message: &Message) -> Value {
    match message {
        Message::User { content } => json!({"role": "user", "content": content}),
        Message::Assistant {
            content,
            tool_calls,
            reasoning,
        } => {
            let mut value = json!({
                "role": "assistant",
                "content": if content.is_empty() && !tool_calls.is_empty() { Value::Null } else { json!(content) },
            });
            if let Some(reasoning) = reasoning {
                value["reasoning_content"] = json!(reasoning);
            }
            if !tool_calls.is_empty() {
                value["tool_calls"] = tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {"name": call.name, "arguments": call.arguments.to_string()},
                        })
                    })
                    .collect();
            }
            value
        }
        Message::Tool {
            tool_call_id,
            content,
            ..
        } => json!({"role": "tool", "tool_call_id": tool_call_id, "content": content}),
    }
}

/// The Chat Completions body for `model` (also what the free trial's
/// gateway takes, with its one model)
pub(crate) fn request_body(model: &str, request: &LlmRequest, max_tokens: u32) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.extend(request.messages.iter().map(message));

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        "max_tokens": max_tokens,
    });
    let tools = tools(&request.tools);
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    body
}

// ---------------------------------------------------------------------------
// Response stream
// ---------------------------------------------------------------------------

/// Splits a byte stream into lines, holding partial lines between chunks.
#[derive(Default)]
struct LineBuffer(Vec<u8>);

impl LineBuffer {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.0.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(end) = self.0.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.0.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            lines.push(line.trim_end_matches(['\r', '\n']).to_string());
        }
        lines
    }
}

#[derive(Debug)]
enum SseLine {
    Chunk(Value),
    Done,
    Ignore,
}

fn parse_line(line: &str) -> Result<SseLine, String> {
    let Some(data) = line.strip_prefix("data:") else {
        // Blank separators and `: keep-alive` comments
        return Ok(SseLine::Ignore);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(SseLine::Done);
    }
    let value: Value =
        serde_json::from_str(data).map_err(|e| format!("Malformed DeepSeek stream chunk: {e}"))?;
    if let Some(error) = value.get("error") {
        let message = error["message"].as_str().unwrap_or("unknown error");
        return Err(format!("DeepSeek stream error: {message}"));
    }
    Ok(SseLine::Chunk(value))
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct StreamState {
    content: String,
    reasoning: String,
    calls: BTreeMap<u64, PartialCall>,
    finish_reason: Option<String>,
    usage: Usage,
}

impl StreamState {
    /// Folds one chunk in; returns its text delta, if any.
    fn apply(&mut self, chunk: &Value) -> Option<String> {
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            let hit = usage
                .pointer("/prompt_tokens_details/prompt_cache_hit_tokens")
                .or_else(|| usage.get("prompt_cache_hit_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            self.usage = Usage {
                input_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                output_tokens: usage["completion_tokens"].as_u64().unwrap_or(0) as u32,
                cache_hit_tokens: hit as u32,
            };
        }

        let choice = &chunk["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = &choice["delta"];
        if let Some(reasoning) = delta["reasoning_content"].as_str() {
            self.reasoning.push_str(reasoning);
        }
        for fragment in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = fragment["index"].as_u64().unwrap_or(0);
            let call = self.calls.entry(index).or_default();
            if let Some(id) = fragment["id"].as_str() {
                call.id = id.to_string();
            }
            if let Some(name) = fragment["function"]["name"].as_str() {
                call.name.push_str(name);
            }
            if let Some(arguments) = fragment["function"]["arguments"].as_str() {
                call.arguments.push_str(arguments);
            }
        }

        let text = delta["content"].as_str().filter(|t| !t.is_empty())?;
        self.content.push_str(text);
        Some(text.to_string())
    }

    fn finish(self, model: &str, at: SystemTime) -> LlmResponse {
        let tool_calls = self
            .calls
            .into_values()
            .filter(|call| !call.name.is_empty())
            .map(|call| {
                let raw = if call.arguments.trim().is_empty() {
                    "{}"
                } else {
                    &call.arguments
                };
                ToolCall {
                    id: if call.id.is_empty() {
                        format!("call_{}", uuid::Uuid::new_v4().simple())
                    } else {
                        call.id
                    },
                    name: call.name,
                    // Malformed JSON stays visible rather than being dropped
                    arguments: serde_json::from_str(raw).unwrap_or_else(|_| json!({ "_raw": raw })),
                }
            })
            .collect();
        LlmResponse {
            content: self.content,
            tool_calls,
            stop_reason: self.finish_reason,
            cost_usd: cost_usd(model, &self.usage, at),
            usage: self.usage,
            reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
        }
    }
}

// ---------------------------------------------------------------------------
// Pricing
// ---------------------------------------------------------------------------

/// Peak USD per 1M tokens: (input cache hit, input cache miss, output).
/// Off-peak is half. Source: api-docs.deepseek.com/quick_start/pricing.
fn peak_prices(model: &str) -> Option<(f64, f64, f64)> {
    match model {
        "deepseek-flash" | "deepseek-v4-flash" => Some((0.006, 0.30, 1.20)),
        "deepseek-v4-pro" => Some((0.044, 1.32, 3.96)),
        _ => None,
    }
}

/// Peak hours: weekdays 01:00–04:00 and 06:00–10:00 UTC. (DeepSeek also
/// exempts Chinese public holidays; not modeled.)
fn is_peak(at: SystemTime) -> bool {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let days = secs / 86_400;
    // 1970-01-01 was a Thursday; 0 = Sunday
    let weekday = (days + 4) % 7;
    let hour = (secs % 86_400) / 3_600;
    (1..=5).contains(&weekday) && ((1..4).contains(&hour) || (6..10).contains(&hour))
}

fn cost_usd(model: &str, usage: &Usage, at: SystemTime) -> Option<f64> {
    let (hit, miss, out) = peak_prices(model)?;
    let factor = if is_peak(at) { 1.0 } else { 0.5 };
    let hit_tokens = f64::from(usage.cache_hit_tokens.min(usage.input_tokens));
    let miss_tokens = f64::from(usage.input_tokens) - hit_tokens;
    let output_tokens = f64::from(usage.output_tokens);
    Some(factor * (hit_tokens * hit + miss_tokens * miss + output_tokens * out) / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    const SSE: &str = concat!(
        ": keep-alive\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"Need to list.\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Let me \"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"check.\"},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":\"}}]},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"ls\\\"}\"}}]},\"finish_reason\":null}],\"usage\":null}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":80,\"prompt_tokens_details\":{\"prompt_cache_hit_tokens\":1024,\"prompt_cache_miss_tokens\":176}}}\n\n",
        "data: [DONE]\n\n",
    );

    fn request() -> LlmRequest {
        LlmRequest {
            model: "deepseek:deepseek-flash".into(),
            system: Some("Be brief".into()),
            messages: vec![
                Message::User {
                    content: "list".into(),
                },
                Message::Assistant {
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "list_dir".into(),
                        arguments: json!({"path": "."}),
                    }],
                    reasoning: Some("thinking".into()),
                },
                Message::Tool {
                    tool_call_id: "c1".into(),
                    name: "list_dir".into(),
                    content: "a.txt".into(),
                    is_error: false,
                },
            ],
            tools: vec![
                ToolSpec {
                    name: "shell".into(),
                    description: "Run".into(),
                    input_schema: json!({"type": "object"}),
                },
                ToolSpec {
                    name: "bad name!".into(),
                    description: String::new(),
                    input_schema: json!({}),
                },
            ],
            max_tokens: None,
        }
    }

    #[test]
    fn builds_a_streaming_request_that_replays_reasoning() {
        let body = request_body("deepseek-flash", &request(), 4096);

        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        let assistant = &body["messages"][2];
        assert!(assistant["content"].is_null());
        assert_eq!(assistant["reasoning_content"], "thinking");
        assert_eq!(
            assistant["tool_calls"][0]["function"]["arguments"],
            r#"{"path":"."}"#
        );
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
        // Invalid tool names are dropped
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn accumulates_text_reasoning_tool_calls_and_usage() {
        let mut state = StreamState::default();
        let mut lines = LineBuffer::default();
        let mut deltas = Vec::new();
        // Feed in awkward slices to exercise partial lines
        for piece in SSE.as_bytes().chunks(7) {
            for line in lines.push(piece) {
                match parse_line(&line).unwrap() {
                    SseLine::Chunk(chunk) => deltas.extend(state.apply(&chunk)),
                    SseLine::Done => break,
                    SseLine::Ignore => {}
                }
            }
        }
        let response = state.finish("deepseek-flash", UNIX_EPOCH);

        assert_eq!(deltas, ["Let me ", "check."]);
        assert_eq!(response.content, "Let me check.");
        assert_eq!(response.reasoning.as_deref(), Some("Need to list."));
        assert_eq!(response.stop_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            response.tool_calls,
            vec![ToolCall {
                id: "call_0".into(),
                name: "shell".into(),
                arguments: json!({"cmd": "ls"})
            }]
        );
        assert_eq!(
            response.usage,
            Usage {
                input_tokens: 1200,
                output_tokens: 80,
                cache_hit_tokens: 1024
            }
        );
        assert!(response.cost_usd.unwrap() > 0.0);
    }

    #[test]
    fn maps_errors_to_clear_messages() {
        assert!(error_message(401, "").contains("API key"));
        assert!(error_message(402, "").contains("out of balance"));
        assert!(error_message(429, "").contains("rate-limiting"));
        assert!(error_message(400, r#"{"error":{"message":"bad tools"}}"#).contains("bad tools"));
        assert!(parse_line(r#"data: {"error":{"message":"boom"}}"#)
            .unwrap_err()
            .contains("boom"));
    }

    #[test]
    fn prices_peak_and_off_peak() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_hit_tokens: 0,
        };
        // Monday 2026-10-05 02:00 UTC (peak) and 12:00 UTC (off-peak)
        let monday = UNIX_EPOCH + Duration::from_secs(1_791_158_400);
        let peak = cost_usd(
            "deepseek-flash",
            &usage,
            monday + Duration::from_secs(2 * 3600),
        )
        .unwrap();
        let off = cost_usd(
            "deepseek-flash",
            &usage,
            monday + Duration::from_secs(12 * 3600),
        )
        .unwrap();
        assert!((peak - 1.50).abs() < 1e-9, "{peak}");
        assert!((off - 0.75).abs() < 1e-9, "{off}");
        assert!(cost_usd("deepseek-unknown", &usage, monday).is_none());
    }

    async fn mock_server(first_status: Option<StatusCode>) -> (String, Arc<AtomicU32>) {
        let calls = Arc::new(AtomicU32::new(0));
        let counter = calls.clone();
        let app = axum::Router::new().route(
            "/chat/completions",
            post(
                move |axum::extract::Json(body): axum::extract::Json<Value>| {
                    let counter = counter.clone();
                    async move {
                        assert_eq!(body["stream"], true);
                        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                            if let Some(status) = first_status {
                                return (status, [("retry-after", "0")], "slow down")
                                    .into_response();
                            }
                        }
                        ([("content-type", "text/event-stream")], SSE).into_response()
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, calls)
    }

    #[tokio::test]
    async fn streams_deltas_end_to_end_and_retries_rate_limits() {
        let (url, calls) = mock_server(Some(StatusCode::TOO_MANY_REQUESTS)).await;
        let (tx, mut rx) = mpsc::channel(16);

        let response = stream_from(
            &url,
            &reqwest::Client::new(),
            "key",
            "deepseek-flash",
            &request(),
            64,
            Some(tx),
        )
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2, "retried once after 429");
        assert_eq!(response.tool_calls[0].name, "shell");
        let mut streamed = String::new();
        while let Ok(delta) = rx.try_recv() {
            streamed.push_str(&delta);
        }
        assert_eq!(streamed, "Let me check.");
    }

    #[tokio::test]
    async fn auth_errors_are_not_retried() {
        let (url, calls) = mock_server(Some(StatusCode::UNAUTHORIZED)).await;
        let error = stream_from(
            &url,
            &reqwest::Client::new(),
            "bad",
            "deepseek-flash",
            &request(),
            64,
            None,
        )
        .await
        .unwrap_err();
        assert!(error.contains("API key"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
