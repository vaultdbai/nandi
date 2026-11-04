//! Anthropic Messages API adapter.

use serde_json::{json, Value};

use super::{send_json, LlmRequest, LlmResponse, Message, ToolCall, Usage};

const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
pub(super) const API_VERSION: &str = "2023-06-01";

pub async fn complete(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    request: &LlmRequest,
    max_tokens: u32,
) -> Result<LlmResponse, String> {
    let body = request_body(model, request, max_tokens);
    let http = client
        .post(MESSAGES_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", API_VERSION)
        .json(&body);
    parse_response(&send_json("Anthropic", http).await?)
}

fn request_body(model: &str, request: &LlmRequest, max_tokens: u32) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages(&request.messages),
    });
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        body["system"] = json!(system);
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect();
    }
    body
}

/// Converts to content-block messages. Tool results become `tool_result`
/// blocks in a user turn, and consecutive same-role turns are merged, as the
/// API requires all results for one assistant turn in the next user turn.
fn messages(history: &[Message]) -> Vec<Value> {
    let mut turns: Vec<(&str, Vec<Value>)> = Vec::new();

    for message in history {
        let (role, blocks) = match message {
            Message::User { content } => ("user", vec![json!({"type": "text", "text": content})]),
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                let mut blocks = Vec::new();
                if !content.is_empty() {
                    blocks.push(json!({"type": "text", "text": content}));
                }
                blocks.extend(tool_calls.iter().map(|call| {
                    json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    })
                }));
                ("assistant", blocks)
            }
            Message::Tool {
                tool_call_id,
                content,
                is_error,
                ..
            } => (
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                    "is_error": is_error,
                })],
            ),
        };
        if blocks.is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => last_blocks.extend(blocks),
            _ => turns.push((role, blocks)),
        }
    }

    turns
        .into_iter()
        .map(|(role, content)| json!({"role": role, "content": content}))
        .collect()
}

fn parse_response(body: &Value) -> Result<LlmResponse, String> {
    let blocks = body["content"]
        .as_array()
        .ok_or("Anthropic response has no content")?;

    let mut response = LlmResponse {
        stop_reason: body["stop_reason"].as_str().map(str::to_string),
        usage: Usage {
            input_tokens: body["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: body["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
            cache_hit_tokens: 0,
        },
        ..Default::default()
    };
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => response
                .content
                .push_str(block["text"].as_str().unwrap_or("")),
            Some("tool_use") => response.tool_calls.push(ToolCall {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                arguments: block["input"].clone(),
            }),
            _ => {}
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolSpec;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: "shell".to_string(),
            arguments: json!({"cmd": "ls"}),
        }
    }

    #[test]
    fn groups_tool_results_into_one_user_turn() {
        let history = vec![
            Message::User {
                content: "list".into(),
            },
            Message::Assistant {
                content: "Running".into(),
                tool_calls: vec![call("a"), call("b")],
                reasoning: None,
            },
            Message::Tool {
                tool_call_id: "a".into(),
                name: "shell".into(),
                content: "x".into(),
                is_error: false,
            },
            Message::Tool {
                tool_call_id: "b".into(),
                name: "shell".into(),
                content: "denied".into(),
                is_error: true,
            },
        ];

        let turns = messages(&history);

        assert_eq!(turns.len(), 3);
        assert_eq!(turns[1]["content"][1]["type"], "tool_use");
        assert_eq!(turns[2]["role"], "user");
        assert_eq!(turns[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(turns[2]["content"][1]["is_error"], true);
    }

    #[test]
    fn builds_body_with_system_and_tools() {
        let request = LlmRequest {
            model: "claude:claude-sonnet-5".into(),
            system: Some("Be brief".into()),
            messages: vec![Message::User {
                content: "hi".into(),
            }],
            tools: vec![ToolSpec {
                name: "shell".into(),
                description: "Run".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: None,
        };

        let body = request_body("claude-sonnet-5", &request, 1000);

        assert_eq!(body["model"], "claude-sonnet-5");
        assert_eq!(body["system"], "Be brief");
        assert_eq!(body["max_tokens"], 1000);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn parses_text_and_tool_use() {
        let body = json!({
            "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "t1", "name": "shell", "input": {"cmd": "ls"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 12, "output_tokens": 7}
        });

        let response = parse_response(&body).unwrap();

        assert_eq!(response.content, "Let me check.");
        assert_eq!(response.tool_calls, vec![call("t1")]);
        assert_eq!(response.usage.input_tokens, 12);
        assert_eq!(response.stop_reason.as_deref(), Some("tool_use"));
    }
}
