//! A short-lived MCP session to a pilot's server on host loopback.
//!
//! Unlike the container client (`mcp_client`), it doesn't retry: a pilot
//! that doesn't answer quickly is treated as unhealthy and skipped.

use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

use crate::llm::ToolSpec;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RESULT_CHARS: usize = 100_000;

pub struct PilotSession {
    service: RunningService<RoleClient, ()>,
}

/// A tool the pilot offers, under its own (unprefixed) name.
pub struct PilotTool {
    pub spec: ToolSpec,
    /// The server's `readOnlyHint`
    pub read_only: bool,
}

/// `http://127.0.0.1:<port>/mcp`
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

impl PilotSession {
    pub async fn connect(port: u16) -> Result<Self, String> {
        let config = StreamableHttpClientTransportConfig::with_uri(url(port));
        let transport = StreamableHttpClientTransport::from_config(config);
        let service = tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| "the pilot didn't answer in time".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(Self { service })
    }

    pub async fn list_tools(&self) -> Result<Vec<PilotTool>, String> {
        let tools = tokio::time::timeout(LIST_TIMEOUT, self.service.list_all_tools())
            .await
            .map_err(|_| "listing tools timed out".to_string())?
            .map_err(|e| format!("listing tools failed: {e}"))?;
        Ok(tools
            .into_iter()
            .map(|tool| PilotTool {
                read_only: tool
                    .annotations
                    .as_ref()
                    .and_then(|a| a.read_only_hint)
                    .unwrap_or(false),
                spec: ToolSpec {
                    name: tool.name.to_string(),
                    description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
                    input_schema: Value::Object((*tool.input_schema).clone()),
                },
            })
            .collect())
    }

    /// `(content, is_error)`; `Err` if the server couldn't be asked.
    pub async fn call(&self, name: &str, arguments: &Value) -> Result<(String, bool), String> {
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = match arguments {
            Value::Object(map) => Some(map.clone()),
            Value::Null => None,
            other => {
                return Ok((
                    format!("Tool arguments must be a JSON object, got: {other}"),
                    true,
                ))
            }
        };
        let result = tokio::time::timeout(CALL_TIMEOUT, self.service.call_tool(params))
            .await
            .map_err(|_| format!("Tool '{name}' timed out"))?
            .map_err(|e| format!("Tool '{name}' failed: {e}"))?;
        let result = serde_json::to_value(&result).unwrap_or_default();
        Ok(outcome(&result))
    }

    pub async fn close(self) {
        if let Err(e) = self.service.cancel().await {
            log::debug!("Pilot MCP session close: {e}");
        }
    }
}

/// Structured output if present, else the text blocks, truncated.
fn outcome(result: &Value) -> (String, bool) {
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let mut content = match result.get("structuredContent").filter(|v| !v.is_null()) {
        Some(structured) => structured.to_string(),
        None => result["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
    };
    if content.chars().count() > MAX_RESULT_CHARS {
        content = content.chars().take(MAX_RESULT_CHARS).collect();
        content.push_str("\n[truncated]");
    }
    (content, is_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefers_structured_content() {
        let text =
            json!({"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]});
        assert_eq!(outcome(&text), ("a\nb".to_string(), false));
        let structured = json!({"content": [], "structuredContent": {"x": 1}, "isError": true});
        assert_eq!(outcome(&structured), ("{\"x\":1}".to_string(), true));
    }
}
