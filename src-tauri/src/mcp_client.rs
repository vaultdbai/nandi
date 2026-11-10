//! MCP client for an agent's containerized tool server, over stdio:
//! `docker exec -i <container> python /opt/nandi/app.py --stdio`. The
//! container has no published ports (it's on the internal `nandi-agents`
//! network), and only someone who can run `docker exec` reaches it, so
//! there's no token to guard.

use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

use crate::llm::ToolSpec;

/// The tool server inside the `nandi-runtime` image
const SERVER: &str = "/opt/nandi/app.py";
/// How long a fresh container's server gets to answer `initialize`
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// The runtime's shell timeout when skill.md doesn't set `shell_timeout_s`
const DEFAULT_SHELL_TIMEOUT_S: u32 = 30;
/// How much longer than the shell timeout a call may take, so the server
/// reports the timeout before the host gives up on it
const CALL_TIMEOUT_MARGIN_S: u32 = 30;

/// How long the host waits for one tool call of an agent whose `shell` calls
/// may run `shell_timeout_s` (skill.md; `None` is the runtime's 30s).
pub fn call_timeout(shell_timeout_s: Option<u32>) -> Duration {
    let shell = shell_timeout_s.unwrap_or(DEFAULT_SHELL_TIMEOUT_S);
    Duration::from_secs(u64::from(shell + CALL_TIMEOUT_MARGIN_S))
}
/// Tool output beyond this is truncated before it reaches the model.
const MAX_RESULT_CHARS: usize = 100_000;

pub struct McpSession {
    service: RunningService<RoleClient, ()>,
    /// [`call_timeout`] for the agent this session serves
    call_timeout: Duration,
}

/// The textual result of one tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
}

/// A failure talking to the server, as opposed to the tool itself failing
/// (which is a [`ToolOutcome`] with `is_error`).
#[derive(Debug)]
pub struct TransportError(pub String);

impl McpSession {
    /// Starts the tool server in `container_id` and opens a session on it.
    /// The server lives as long as the session (`close` ends it). Each tool
    /// call may take up to `call_timeout` ([`call_timeout`]).
    pub async fn connect(container_id: &str, call_timeout: Duration) -> Result<Self, String> {
        let mut command = tokio::process::Command::new("docker");
        command.args(["exec", "-i", container_id, "python", SERVER, "--stdio"]);
        #[cfg(windows)]
        {
            // No console window flashing up for docker.exe
            command.creation_flags(0x0800_0000);
        }
        let (transport, _stderr) = TokioChildProcess::builder(command)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "Docker is not installed".to_string(),
                _ => format!("Could not start the agent's tool server: {e}"),
            })?;
        let service = tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| "The agent's tool server did not answer".to_string())?
            .map_err(|e| format!("Could not connect to the agent runtime: {e}"))?;
        log::info!("Connected to the tool server in container {container_id}");
        Ok(Self {
            service,
            call_timeout,
        })
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolSpec>, String> {
        let tools = self
            .service
            .list_all_tools()
            .await
            .map_err(|e| format!("Failed to list runtime tools: {e}"))?;

        Ok(tools
            .into_iter()
            .map(|tool| ToolSpec {
                name: tool.name.to_string(),
                description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
                input_schema: Value::Object((*tool.input_schema).clone()),
            })
            .collect())
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolOutcome, TransportError> {
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = match arguments {
            Value::Object(map) => Some(map.clone()),
            Value::Null => None,
            other => {
                return Ok(ToolOutcome {
                    content: format!("Tool arguments must be a JSON object, got: {other}"),
                    is_error: true,
                })
            }
        };

        let result = tokio::time::timeout(self.call_timeout, self.service.call_tool(params))
            .await
            .map_err(|_| TransportError(format!("Tool '{name}' timed out")))?
            .map_err(|e| TransportError(format!("Tool '{name}' failed: {e}")))?;

        // Go through JSON so new content block variants don't break us
        let result = serde_json::to_value(&result).unwrap_or_default();
        Ok(outcome_from_result(&result))
    }

    pub async fn close(self) {
        if let Err(e) = self.service.cancel().await {
            log::debug!("MCP session close: {e}");
        }
    }
}

/// Prefers structured output (e.g. shell's `{stdout, stderr, exit_code}`),
/// falling back to the concatenated text blocks.
fn outcome_from_result(result: &Value) -> ToolOutcome {
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let content = match result.get("structuredContent") {
        Some(structured) if !structured.is_null() && !is_error => structured.to_string(),
        _ => result["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
    };
    ToolOutcome {
        content: truncate(content),
        is_error,
    }
}

fn truncate(mut text: String) -> String {
    if let Some((index, _)) = text.char_indices().nth(MAX_RESULT_CHARS) {
        let omitted = text[index..].chars().count();
        text.truncate(index);
        text.push_str(&format!("\n[... {omitted} characters truncated]"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calls_outlast_the_shell_timeout() {
        assert_eq!(call_timeout(None), Duration::from_secs(60));
        assert_eq!(call_timeout(Some(600)), Duration::from_secs(630));
    }

    #[test]
    fn prefers_structured_content() {
        let result = json!({
            "content": [{"type": "text", "text": "{\"exit_code\": 0}"}],
            "structuredContent": {"stdout": "a.txt\n", "stderr": "", "exit_code": 0},
            "isError": false
        });
        let outcome = outcome_from_result(&result);
        assert!(!outcome.is_error);
        assert!(outcome.content.contains("\"stdout\":\"a.txt\\n\""));
    }

    #[test]
    fn joins_text_blocks_for_errors() {
        let result = json!({
            "content": [{"type": "text", "text": "Path is outside /agent/workdir"}],
            "isError": true
        });
        let outcome = outcome_from_result(&result);
        assert!(outcome.is_error);
        assert_eq!(outcome.content, "Path is outside /agent/workdir");
    }

    #[test]
    fn truncates_long_output_on_char_boundary() {
        let text = "é".repeat(MAX_RESULT_CHARS + 5);
        let truncated = truncate(text);
        assert!(truncated.ends_with("[... 5 characters truncated]"));
    }
}
