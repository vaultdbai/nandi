//! Ollama `/api/chat` adapter (local, no API key).

use serde_json::{json, Value};

use super::{send_json, LlmRequest, LlmResponse, Message, ToolCall, ToolSpec, Usage};

const CHAT_URL: &str = "http://localhost:11434/api/chat";

pub async fn complete(
    client: &reqwest::Client,
    model: &str,
    request: &LlmRequest,
    max_tokens: u32,
) -> Result<LlmResponse, String> {
    let body = request_body(model, request, max_tokens);
    parse_response(&send_json("Ollama", client.post(CHAT_URL).json(&body)).await?)
}

/// OpenAI-style function tool definitions (also used by Ollama).
pub(super) fn function_tools(tools: &[ToolSpec]) -> Value {
    tools
        .iter()
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

fn request_body(model: &str, request: &LlmRequest, max_tokens: u32) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.extend(request.messages.iter().map(|message| match message {
        Message::User { content } => json!({"role": "user", "content": content}),
        Message::Assistant {
            content,
            tool_calls,
            ..
        } => json!({
            "role": "assistant",
            "content": content,
            "tool_calls": tool_calls
                .iter()
                .map(|call| json!({"function": {"name": call.name, "arguments": call.arguments}}))
                .collect::<Vec<_>>(),
        }),
        Message::Tool { name, content, .. } => {
            json!({"role": "tool", "tool_name": name, "content": content})
        }
    }));

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "options": {"num_predict": max_tokens},
    });
    if !request.tools.is_empty() {
        body["tools"] = function_tools(&request.tools);
    }
    body
}

fn parse_response(body: &Value) -> Result<LlmResponse, String> {
    let message = body
        .get("message")
        .ok_or("Ollama response has no message")?;

    // Ollama doesn't assign tool call IDs, so generate them
    let tool_calls = message["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| ToolCall {
                    id: format!("call_{}", uuid::Uuid::new_v4().simple()),
                    name: call["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    arguments: call["function"]["arguments"].clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(LlmResponse {
        content: message["content"].as_str().unwrap_or_default().to_string(),
        tool_calls,
        stop_reason: body["done_reason"].as_str().map(str::to_string),
        usage: Usage {
            input_tokens: body["prompt_eval_count"].as_u64().unwrap_or(0) as u32,
            output_tokens: body["eval_count"].as_u64().unwrap_or(0) as u32,
            cache_hit_tokens: 0,
        },
        reasoning: None,
        cost_usd: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_calls_and_generates_ids() {
        let body = json!({
            "message": {"role": "assistant", "content": "", "tool_calls": [
                {"function": {"name": "list_dir", "arguments": {"path": "."}}}
            ]},
            "done_reason": "stop",
            "prompt_eval_count": 20,
            "eval_count": 4
        });

        let response = parse_response(&body).unwrap();

        assert_eq!(response.tool_calls[0].name, "list_dir");
        assert!(response.tool_calls[0].id.starts_with("call_"));
        assert_eq!(response.usage.input_tokens, 20);
    }

    #[test]
    fn tool_results_carry_tool_name() {
        let request = LlmRequest {
            model: "ollama:llama3.2".into(),
            system: None,
            messages: vec![Message::Tool {
                tool_call_id: "x".into(),
                name: "shell".into(),
                content: "out".into(),
                is_error: false,
            }],
            tools: vec![],
            max_tokens: None,
        };
        let body = request_body("llama3.2", &request, 64);
        assert_eq!(body["messages"][0]["tool_name"], "shell");
        assert_eq!(body["stream"], false);
    }
}
