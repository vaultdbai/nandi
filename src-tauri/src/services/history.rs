//! Rebuilds the model's conversation history from persisted messages.
//!
//! Every assistant tool call gets a result message right after it (the API
//! rejects unanswered calls): the tool's output, or a note that the user
//! rejected it or that it never ran. DeepSeek's thinking is replayed via
//! `reasoning`, which it requires once tools are involved.

use std::collections::HashMap;

use crate::db::{role, AssistantMeta, MessageRow, ToolCallMeta, UserMeta};
use crate::llm::{Message, ToolCall};
use crate::model::ApprovalStatus;

/// Long tool output is cut before it goes back to the model. Connectors
/// that can return a lot (a PR's diff) size their answer to fit, so it
/// arrives whole.
pub const MAX_TOOL_RESULT_CHARS: usize = 40_000;

pub const REJECTED_RESULT: &str = "The user rejected this tool call.";
const EXPIRED_RESULT: &str = "Not run: the run ended before the user decided.";
const NOT_RUN_RESULT: &str = "Not run: the run was cancelled or interrupted.";

fn truncate(text: &str) -> String {
    match text.char_indices().nth(MAX_TOOL_RESULT_CHARS) {
        Some((index, _)) => format!("{}\n[… output truncated]", &text[..index]),
        None => text.to_string(),
    }
}

/// `rows` are a conversation's messages, oldest first; `approvals` maps tool
/// call IDs to their approval status.
pub fn build(rows: &[MessageRow], approvals: &HashMap<String, ApprovalStatus>) -> Vec<Message> {
    let results: HashMap<&str, ToolCallMeta> = rows
        .iter()
        .filter(|m| m.role == role::TOOL_CALL)
        .filter_map(|m| {
            Some((
                m.id.as_str(),
                serde_json::from_value(m.metadata.clone()).ok()?,
            ))
        })
        .collect();

    // A window may start mid-exchange; begin at its first user message
    let start = rows
        .iter()
        .position(|m| m.role == role::USER)
        .unwrap_or(rows.len());

    let mut history = Vec::new();
    for row in &rows[start..] {
        match row.role.as_str() {
            role::USER => {
                let attachments = serde_json::from_value::<UserMeta>(row.metadata.clone())
                    .unwrap_or_default()
                    .attachments;
                let mut content = row.content.clone();
                if !attachments.is_empty() {
                    let names: Vec<_> = attachments.iter().map(|a| a.name.as_str()).collect();
                    content.push_str(&format!(
                        "\n\n[The user attached: {}. Their contents aren't available to you yet.]",
                        names.join(", ")
                    ));
                }
                history.push(Message::User { content });
            }
            role::ASSISTANT => {
                let meta: AssistantMeta =
                    serde_json::from_value(row.metadata.clone()).unwrap_or_default();
                history.push(Message::Assistant {
                    content: row.content.clone(),
                    tool_calls: meta
                        .planned_calls
                        .iter()
                        .map(|call| ToolCall {
                            id: call.id.clone(),
                            name: call.tool.clone(),
                            arguments: call.args.clone(),
                        })
                        .collect(),
                    reasoning: meta.reasoning.clone(),
                });
                for call in &meta.planned_calls {
                    let (content, is_error) = match results.get(call.id.as_str()) {
                        Some(meta) => (
                            truncate(meta.result.as_deref().unwrap_or("(no output)")),
                            meta.is_error,
                        ),
                        None => match approvals.get(&call.id) {
                            Some(ApprovalStatus::Rejected) => (REJECTED_RESULT.to_string(), true),
                            Some(ApprovalStatus::Expired) => (EXPIRED_RESULT.to_string(), true),
                            _ => (NOT_RUN_RESULT.to_string(), true),
                        },
                    };
                    history.push(Message::Tool {
                        tool_call_id: call.id.clone(),
                        name: call.tool.clone(),
                        content,
                        is_error,
                    });
                }
            }
            // Tool results are attached to their call above; approvals and
            // errors are UI-only
            _ => {}
        }
    }
    history
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PlannedCall;
    use crate::model::{AttachmentInfo, AttachmentKind, ToolCallStatus};
    use serde_json::json;

    fn call(id: &str, tool: &str) -> PlannedCall {
        PlannedCall {
            id: id.into(),
            tool: tool.into(),
            args: json!({}),
            requires_approval: false,
        }
    }

    fn assistant(id: &str, text: &str, calls: Vec<PlannedCall>) -> MessageRow {
        MessageRow::new(
            id,
            "c",
            Some("r"),
            role::ASSISTANT,
            text,
            AssistantMeta {
                step: 0,
                planned_calls: calls,
                reasoning: Some(format!("thinking about {id}")),
            },
        )
    }

    fn tool_result(id: &str, result: &str) -> MessageRow {
        MessageRow::new(
            id,
            "c",
            Some("r"),
            role::TOOL_CALL,
            "",
            ToolCallMeta {
                tool: "list_dir".into(),
                args: json!({}),
                status: ToolCallStatus::Done,
                result: Some(result.into()),
                is_error: false,
            },
        )
    }

    #[test]
    fn answers_every_tool_call_in_order() {
        let rows = vec![
            // Left over from before the window: skipped
            tool_result("old", "x"),
            MessageRow::new(
                "u1",
                "c",
                Some("r"),
                role::USER,
                "check",
                UserMeta::default(),
            ),
            assistant(
                "a1",
                "Looking.",
                vec![
                    call("c1", "list_dir"),
                    call("c2", "shell"),
                    call("c3", "shell"),
                ],
            ),
            tool_result("c1", "a.txt"),
            MessageRow::new("ap", "c", Some("r"), role::APPROVAL, "", json!({})),
            assistant("a2", "Done.", vec![]),
        ];
        let approvals = HashMap::from([("c2".to_string(), ApprovalStatus::Rejected)]);

        let history = build(&rows, &approvals);

        assert_eq!(history.len(), 6, "{history:#?}");
        assert!(matches!(&history[0], Message::User { content } if content == "check"));
        assert!(
            matches!(&history[1], Message::Assistant { tool_calls, reasoning, .. }
            if tool_calls.len() == 3 && reasoning.as_deref() == Some("thinking about a1"))
        );
        assert!(
            matches!(&history[2], Message::Tool { content, is_error: false, .. } if content == "a.txt")
        );
        assert!(
            matches!(&history[3], Message::Tool { content, is_error: true, .. } if content == REJECTED_RESULT)
        );
        assert!(matches!(&history[4], Message::Tool { content, .. } if content == NOT_RUN_RESULT));
        assert!(matches!(&history[5], Message::Assistant { content, .. } if content == "Done."));
    }

    #[test]
    fn mentions_attachments_and_truncates_long_output() {
        let user = MessageRow::new(
            "u1",
            "c",
            None,
            role::USER,
            "see file",
            UserMeta {
                attachments: vec![AttachmentInfo {
                    id: "f".into(),
                    name: "brief.pdf".into(),
                    kind: AttachmentKind::File,
                    size_bytes: 10.0,
                }],
            },
        );
        let long = "x".repeat(MAX_TOOL_RESULT_CHARS + 10);
        let rows = vec![
            user,
            assistant("a1", "", vec![call("c1", "list_dir")]),
            tool_result("c1", &long),
        ];

        let history = build(&rows, &HashMap::new());

        assert!(matches!(&history[0], Message::User { content } if content.contains("brief.pdf")));
        assert!(
            matches!(&history[2], Message::Tool { content, .. } if content.ends_with("[… output truncated]"))
        );
    }
}
