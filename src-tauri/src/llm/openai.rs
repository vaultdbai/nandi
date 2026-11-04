//! OpenAI Chat Completions API adapter.

use serde_json::{json, Value};

use super::{send_json, LlmRequest, LlmResponse, Message, ToolCall, Usage};

const CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";

pub async fn complete(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    request: &LlmRequest,
    max_tokens: u32,
) -> Result<LlmResponse, String> {
    let body = request_body(model, request, max_tokens);
    let http = client.post(CHAT_URL).bearer_auth(api_key).json(&body);
    parse_response(&send_json("OpenAI", http).await?)
}

fn request_body(model: &str, request: &LlmRequest, max_tokens: u32) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.extend(request.messages.iter().map(message));

    let mut body = json!({
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_tokens,
    });
    if !request.tools.is_empty() {
        body["tools"] = super::ollama::function_tools(&request.tools);
    }
    body
}

fn message(message: &Message) -> Value {
    match message {
        Message::User { content } => json!({"role": "user", "content": content}),
        Message::Assistant {
            content,
            tool_calls,
            ..
        } => {
            let mut value = json!({
                "role": "assistant",
                "content": if content.is_empty() { Value::Null } else { json!(content) },
            });
            if !tool_calls.is_empty() {
                value["tool_calls"] = tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            // OpenAI encodes arguments as a JSON string
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

fn parse_response(body: &Value) -> Result<LlmResponse, String> {
    let choice = &body["choices"][0];
    let message = choice
        .get("message")
        .ok_or("OpenAI response has no choices")?;

    let tool_calls = message["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    let raw = call["function"]["arguments"].as_str().unwrap_or("{}");
                    ToolCall {
                        id: call["id"].as_str().unwrap_or_default().to_string(),
                        name: call["function"]["name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        // Malformed JSON is kept visible rather than dropped
                        arguments: serde_json::from_str(raw)
                            .unwrap_or_else(|_| json!({ "_raw": raw })),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(LlmResponse {
        content: message["content"].as_str().unwrap_or_default().to_string(),
        tool_calls,
        stop_reason: choice["finish_reason"].as_str().map(str::to_string),
        usage: Usage {
            input_tokens: body["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: body["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
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
    fn encodes_tool_calls_and_results() {
        let assistant = message(&Message::Assistant {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "shell".into(),
                arguments: json!({"cmd": "ls"}),
            }],
            reasoning: None,
        });
        assert!(assistant["content"].is_null());
        assert_eq!(
            assistant["tool_calls"][0]["function"]["arguments"],
            r#"{"cmd":"ls"}"#
        );

        let result = message(&Message::Tool {
            tool_call_id: "c1".into(),
            name: "shell".into(),
            content: "ok".into(),
            is_error: false,
        });
        assert_eq!(result["role"], "tool");
        assert_eq!(result["tool_call_id"], "c1");
    }

    #[test]
    fn prepends_system_message() {
        let request = LlmRequest {
            model: "gpt:gpt-5".into(),
            system: Some("sys".into()),
            messages: vec![Message::User {
                content: "hi".into(),
            }],
            tools: vec![],
            max_tokens: None,
        };
        let body = request_body("gpt-5", &request, 100);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hi");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn parses_tool_calls_with_string_arguments() {
        let body = json!({
            "choices": [{
                "message": {"content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "shell", "arguments": "{\"cmd\":\"ls\"}"}},
                    {"id": "c2", "type": "function", "function": {"name": "shell", "arguments": "not json"}}
                ]},
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 3}
        });

        let response = parse_response(&body).unwrap();

        assert_eq!(response.content, "");
        assert_eq!(response.tool_calls[0].arguments, json!({"cmd": "ls"}));
        assert_eq!(
            response.tool_calls[1].arguments,
            json!({"_raw": "not json"})
        );
        assert_eq!(response.usage.output_tokens, 3);
    }
}
