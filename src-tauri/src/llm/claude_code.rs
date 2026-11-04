//! `claude:` models through Claude Code (the Claude Agent SDK, with the
//! user's own Claude Code sign-in), for when no Anthropic key is set.
//!
//! Each call is one tool-less Claude Code turn
//! (`harness::claude_sdk::complete`): the request's system prompt is the
//! whole system prompt and the conversation is rendered as the user turn.
//! Claude Code has no way to hand Nandi's own tools to the model and give
//! the calls back unexecuted, so tools are described in the system prompt
//! and the model calls them with `<tool_call>` blocks, which are parsed
//! back into `ToolCall`s. Nandi runs them as it does for every provider.

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{LlmRequest, LlmResponse, Message, ToolCall, Usage};
use crate::services::harness::claude_sdk;

const CALL_OPEN: &str = "<tool_call>";
const CALL_CLOSE: &str = "</tool_call>";

/// Claude Code's alias for a Nandi model name: `claude-opus-5` → `opus`.
/// Aliases resolve to the newest model of that family the user's Claude
/// Code can use (on their plan, or on Bedrock/Vertex), where a dated name
/// might not exist there.
pub fn model_alias(model: &str) -> &str {
    ["opus", "sonnet", "haiku"]
        .into_iter()
        .find(|family| model.contains(family))
        .unwrap_or(model)
}

pub async fn stream(
    model: &str,
    request: &LlmRequest,
    text: Option<mpsc::Sender<String>>,
) -> Result<LlmResponse, String> {
    let system = system_prompt(request);
    let prompt = render_conversation(&request.messages);
    // With tools, the raw text may hold `<tool_call>` blocks: only the
    // parsed answer is shown
    let live = if request.tools.is_empty() {
        text.clone()
    } else {
        None
    };
    let completion = claude_sdk::complete(&system, &prompt, Some(model_alias(model)), live).await?;
    let (content, tool_calls) = if request.tools.is_empty() {
        (completion.text, Vec::new())
    } else {
        parse_tool_calls(&completion.text)
    };
    if let Some(text) = text.filter(|_| !request.tools.is_empty() && !content.is_empty()) {
        let _ = text.send(content.clone()).await;
    }
    Ok(LlmResponse {
        stop_reason: Some(if tool_calls.is_empty() {
            completion.stop_reason
        } else {
            "tool_use".to_string()
        }),
        content,
        tool_calls,
        usage: Usage {
            input_tokens: completion.usage.input_tokens,
            output_tokens: completion.usage.output_tokens,
            cache_hit_tokens: completion.usage.cached_input_tokens,
        },
        reasoning: None,
        // Billed by the user's Claude Code plan or provider, not per call
        cost_usd: None,
    })
}

/// The request's system prompt, plus how to call its tools
fn system_prompt(request: &LlmRequest) -> String {
    let system = request.system.as_deref().unwrap_or("").trim();
    if request.tools.is_empty() {
        return system.to_string();
    }
    let tools: Vec<Value> = request
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
    let tools = serde_json::to_string_pretty(&tools).unwrap_or_default();
    format!(
        "{system}\n\n# Tools\n\nYou can call these tools:\n\n{tools}\n\n\
         To call a tool, write a block like this, with the arguments as a JSON \
         object matching the tool's input_schema:\n\n\
         {CALL_OPEN}{{\"name\": \"tool_name\", \"arguments\": {{}}}}{CALL_CLOSE}\n\n\
         You may write several blocks. After your tool calls, stop: the results \
         come back in <tool_result> blocks in the next message. Never write a \
         <tool_result> yourself, and use only this block (no other tool-call \
         format). When you need no more tools, answer normally without any \
         {CALL_OPEN} block."
    )
    .trim_start()
    .to_string()
}

/// The conversation as one user turn. A single user message goes as it is.
fn render_conversation(messages: &[Message]) -> String {
    if let [Message::User { content }] = messages {
        return content.clone();
    }
    let mut out = String::from("<conversation>\n");
    for message in messages {
        match message {
            Message::User { content } => {
                out.push_str(&format!("<user>\n{content}\n</user>\n"));
            }
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                out.push_str("<assistant>\n");
                if !content.is_empty() {
                    out.push_str(content);
                    out.push('\n');
                }
                for call in tool_calls {
                    let block = json!({"name": call.name, "arguments": call.arguments});
                    out.push_str(&format!("{CALL_OPEN}{block}{CALL_CLOSE}\n"));
                }
                out.push_str("</assistant>\n");
            }
            Message::Tool {
                name,
                content,
                is_error,
                ..
            } => {
                let status = if *is_error { " error=\"true\"" } else { "" };
                out.push_str(&format!(
                    "<tool_result name=\"{name}\"{status}>\n{content}\n</tool_result>\n"
                ));
            }
        }
    }
    out.push_str(
        "</conversation>\n\nContinue the conversation as the assistant: write only your next message.",
    );
    out
}

/// A `<tool_call>` body as JSON. Models sometimes stop one or two closing
/// braces short on nested arguments (`{"name": …, "arguments": {"input":
/// {…}}`); the brackets still open (outside strings) are closed before
/// giving up.
fn parse_call(body: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str::<Value>(body).or_else(|error| {
        let mut open = Vec::new();
        let (mut in_string, mut escaped) = (false, false);
        for c in body.chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' if in_string => escaped = true,
                '"' => in_string = !in_string,
                '{' | '[' if !in_string => open.push(c),
                '}' | ']' if !in_string => {
                    open.pop();
                }
                _ => {}
            }
        }
        if in_string || open.is_empty() {
            return Err(error);
        }
        let closers: String = open
            .iter()
            .rev()
            .map(|c| if *c == '{' { '}' } else { ']' })
            .collect();
        serde_json::from_str::<Value>(&format!("{body}{closers}"))
    })
}

/// Splits the model's text into its answer and its tool calls.
///
/// Calls are `<tool_call>{"name", "arguments"}</tool_call>` blocks (the
/// format the prompt asks for), or `<invoke name="…"><parameter
/// name="…">…</parameter></invoke>` blocks, which models also write
/// (optionally inside `<function_calls>`). A block that can't be read stays
/// as text. Tool results the model writes itself (`<tool_result>`,
/// `<function_results>`) are dropped: only Nandi sends results. So are
/// leftovers of the format itself, which the chat would otherwise show: a
/// closing tag with no opening one, and next to real calls, a fragment of
/// a few letters (the model's "hn" or "ext" before a block).
fn parse_tool_calls(text: &str) -> (String, Vec<ToolCall>) {
    let text = drop_invented_results(text);
    let mut content = String::new();
    let mut calls = Vec::new();
    let mut rest = text.as_str();
    loop {
        let json_at = rest.find(CALL_OPEN);
        let invoke_at = rest.find(INVOKE_OPEN);
        let (start, invoke) = match (json_at, invoke_at) {
            (Some(j), Some(i)) if i < j => (i, true),
            (Some(j), _) => (j, false),
            (None, Some(i)) => (i, true),
            (None, None) => break,
        };
        content.push_str(&rest[..start]);
        let block = &rest[start..];
        let (close, parsed) = if invoke {
            match block.find(INVOKE_CLOSE) {
                Some(end) => (
                    end + INVOKE_CLOSE.len(),
                    parse_invoke(&block[..end + INVOKE_CLOSE.len()]),
                ),
                None => {
                    content.push_str(block);
                    rest = "";
                    break;
                }
            }
        } else {
            let after = &block[CALL_OPEN.len()..];
            match after.find(CALL_CLOSE) {
                Some(end) => (
                    CALL_OPEN.len() + end + CALL_CLOSE.len(),
                    parse_call(after[..end].trim())
                        .ok()
                        .filter(|v| v.get("name").and_then(Value::as_str).is_some())
                        .map(|value| {
                            (
                                value["name"].as_str().unwrap_or_default().to_string(),
                                match value.get("arguments") {
                                    Some(arguments @ Value::Object(_)) => arguments.clone(),
                                    _ => json!({}),
                                },
                            )
                        }),
                ),
                None => {
                    content.push_str(block);
                    rest = "";
                    break;
                }
            }
        };
        match parsed {
            Some((name, arguments)) => calls.push(ToolCall {
                id: format!("cc_{}", uuid::Uuid::new_v4().simple()),
                name,
                arguments,
            }),
            None => content.push_str(&block[..close]),
        }
        rest = &block[close..];
    }
    content.push_str(rest);
    let content = content
        .replace("<function_calls>", "")
        .replace("</function_calls>", "");
    let content = drop_orphan_closers(&content);
    let content = content.trim();
    if !calls.is_empty() && is_fragment(content) {
        return (String::new(), calls);
    }
    (content.to_string(), calls)
}

/// `</tool_call>` and `</invoke>` that close nothing before them
fn drop_orphan_closers(text: &str) -> String {
    let mut out = text.to_string();
    for (open, close) in [(CALL_OPEN, CALL_CLOSE), (INVOKE_OPEN, INVOKE_CLOSE)] {
        let mut kept = String::new();
        let mut depth = 0usize;
        let mut rest = out.as_str();
        loop {
            let next_open = rest.find(open);
            let next_close = rest.find(close);
            match (next_open, next_close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    kept.push_str(&rest[..o + open.len()]);
                    rest = &rest[o + open.len()..];
                }
                (_, Some(c)) => {
                    kept.push_str(&rest[..c]);
                    if depth > 0 {
                        depth -= 1;
                        kept.push_str(close);
                    }
                    rest = &rest[c + close.len()..];
                }
                (Some(o), None) => {
                    depth += 1;
                    kept.push_str(&rest[..o + open.len()]);
                    rest = &rest[o + open.len()..];
                }
                (None, None) => break,
            }
        }
        kept.push_str(rest);
        out = kept;
    }
    out
}

/// A few letters with no spaces: a scrap of the format, not an answer
fn is_fragment(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= 4 && !text.contains(char::is_whitespace)
}

const INVOKE_OPEN: &str = "<invoke name=\"";
const INVOKE_CLOSE: &str = "</invoke>";
const PARAM_OPEN: &str = "<parameter name=\"";
const PARAM_CLOSE: &str = "</parameter>";

/// `<invoke name="tool"><parameter name="k">v</parameter>…</invoke>` →
/// the tool and its arguments. A value that reads as JSON (an object, a
/// list, a number, true/false) is that JSON; anything else is a string.
fn parse_invoke(block: &str) -> Option<(String, Value)> {
    let rest = block.strip_prefix(INVOKE_OPEN)?;
    let (name, mut rest) = rest.split_once("\">")?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut arguments = serde_json::Map::new();
    while let Some(at) = rest.find(PARAM_OPEN) {
        let after = &rest[at + PARAM_OPEN.len()..];
        let (key, after) = after.split_once("\">")?;
        let end = after.find(PARAM_CLOSE)?;
        let raw = after[..end].trim();
        let value = match serde_json::from_str::<Value>(raw) {
            Ok(
                value @ (Value::Object(_) | Value::Array(_) | Value::Number(_) | Value::Bool(_)),
            ) => value,
            _ => Value::String(raw.to_string()),
        };
        arguments.insert(key.trim().to_string(), value);
        rest = &after[end + PARAM_CLOSE.len()..];
    }
    Some((name.to_string(), Value::Object(arguments)))
}

/// The text without tool results the model wrote itself: from
/// `<tool_result` or `<function_results>` to its closing tag (any of
/// `</tool_result>`, `</tool_response>`, `</function_results>`), or to the
/// end when it isn't closed
fn drop_invented_results(text: &str) -> String {
    const OPENS: [&str; 2] = ["<tool_result", "<function_results>"];
    const CLOSES: [&str; 3] = ["</tool_result>", "</tool_response>", "</function_results>"];
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = OPENS.iter().filter_map(|open| rest.find(open)).min() {
        out.push_str(&rest[..start]);
        let block = &rest[start..];
        match CLOSES
            .iter()
            .filter_map(|close| block.find(close).map(|at| at + close.len()))
            .min()
        {
            Some(end) => rest = &block[end..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolSpec;

    #[test]
    fn nandi_models_map_to_claude_code_aliases() {
        assert_eq!(model_alias("claude-sonnet-5"), "sonnet");
        assert_eq!(model_alias("claude-opus-5-5"), "opus");
        assert_eq!(model_alias("claude-haiku-5"), "haiku");
        assert_eq!(model_alias("us.anthropic.custom"), "us.anthropic.custom");
    }

    #[test]
    fn tool_calls_are_parsed_out_of_the_text() {
        let text = "Let me look.\n<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.txt\"}}</tool_call>\n<tool_call>{\"name\": \"shell\"}</tool_call>";
        let (content, calls) = parse_tool_calls(text);
        assert_eq!(content, "Let me look.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, json!({"path": "a.txt"}));
        assert_eq!(calls[1].arguments, json!({}), "missing arguments are empty");
        assert_ne!(calls[0].id, calls[1].id);

        // One closing brace short (as models sometimes write): repaired
        let short = r#"<tool_call>{"name":"delegate_to_agent","arguments":{"target":"i/gateway","input":{"text":"Hi {Lena}","due":"2026-10-28"}}</tool_call>"#;
        let (content, calls) = parse_tool_calls(short);
        assert_eq!(content, "");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "delegate_to_agent");
        assert_eq!(calls[0].arguments["input"]["due"], "2026-10-28");
        assert_eq!(calls[0].arguments["input"]["text"], "Hi {Lena}");

        // The <invoke> format, inside <function_calls>
        let invoke = "Checking.\n<function_calls>\n<invoke name=\"filesystem__list_dir\">\n<parameter name=\"path\">my-notes</parameter>\n</invoke>\n<invoke name=\"delegate_to_agent\">\n<parameter name=\"capability\">ask-me</parameter>\n<parameter name=\"input\">{\"text\": \"Hi\"}</parameter>\n<parameter name=\"limit\">5</parameter>\n</invoke>\n</function_calls>";
        let (content, calls) = parse_tool_calls(invoke);
        assert_eq!(content, "Checking.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "filesystem__list_dir");
        assert_eq!(calls[0].arguments, json!({"path": "my-notes"}));
        assert_eq!(
            calls[1].arguments,
            json!({"capability": "ask-me", "input": {"text": "Hi"}, "limit": 5})
        );
        let (_, calls) = parse_tool_calls("<invoke name=\"my_requests\">\n</invoke>");
        assert_eq!(
            (calls[0].name.as_str(), &calls[0].arguments),
            ("my_requests", &json!({}))
        );

        // Results the model invents are dropped, closed or not
        let invented = "<tool_result name=\"my_requests\"> {\"requests\": []} </tool_response>\n<tool_call>{\"name\": \"my_requests\"}</tool_call>";
        let (content, calls) = parse_tool_calls(invented);
        assert_eq!(content, "");
        assert_eq!(calls.len(), 1);
        let (content, _) = parse_tool_calls("Done.\n<function_results>made up");
        assert_eq!(content, "Done.");

        // Not JSON, or unclosed: kept as text
        let (content, calls) = parse_tool_calls("<tool_call>oops</tool_call> and <tool_call>{");
        assert!(calls.is_empty());
        assert_eq!(content, "<tool_call>oops</tool_call> and <tool_call>{");
        assert_eq!(parse_tool_calls("plain").0, "plain");
    }

    #[test]
    fn leftovers_of_the_format_are_dropped() {
        let call = r#"<tool_call>{"name": "find_people"}</tool_call>"#;
        // Seen live: a scrap before a call, and a closer with no opener
        for scrap in ["hn", "ext", "ok."] {
            let (content, calls) = parse_tool_calls(&format!(
                "{scrap}
{call}"
            ));
            assert_eq!((content.as_str(), calls.len()), ("", 1), "{scrap}");
        }
        let (content, _) = parse_tool_calls(&format!(
            "</tool_call>
Let me redo that.
{call}"
        ));
        assert_eq!(content, "Let me redo that.");
        // Without a call, a short answer is the answer
        assert_eq!(parse_tool_calls("Yes").0, "Yes");
        // A real sentence next to a call stays
        assert_eq!(
            parse_tool_calls(&format!(
                "I'll check.
{call}"
            ))
            .0,
            "I'll check."
        );
    }

    #[test]
    fn a_conversation_with_tool_results_is_rendered_as_one_turn() {
        assert_eq!(
            render_conversation(&[Message::User {
                content: "hi".into()
            }]),
            "hi"
        );
        let rendered = render_conversation(&[
            Message::User {
                content: "list files".into(),
            },
            Message::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "shell".into(),
                    arguments: json!({"cmd": "ls"}),
                }],
                reasoning: None,
            },
            Message::Tool {
                tool_call_id: "c1".into(),
                name: "shell".into(),
                content: "a.txt".into(),
                is_error: false,
            },
        ]);
        assert!(rendered.contains("<user>\nlist files\n</user>"));
        assert!(rendered.contains("<tool_call>{") && rendered.contains(r#""name":"shell""#));
        assert!(rendered.contains(r#""arguments":{"cmd":"ls"}"#));
        assert!(rendered.contains("<tool_result name=\"shell\">\na.txt\n</tool_result>"));
        assert!(rendered.ends_with("write only your next message."));
    }

    #[test]
    fn tools_are_described_in_the_system_prompt() {
        let request = LlmRequest {
            model: "claude:claude-sonnet-5".into(),
            system: Some("You help.".into()),
            messages: vec![],
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Reads a file".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: None,
        };
        let system = system_prompt(&request);
        assert!(system.starts_with("You help."));
        assert!(system.contains("\"read_file\"") && system.contains(CALL_OPEN));
        let plain = LlmRequest {
            tools: vec![],
            ..request
        };
        assert_eq!(system_prompt(&plain), "You help.");
    }
}
