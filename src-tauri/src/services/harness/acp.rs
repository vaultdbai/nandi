//! ACP (Agent Client Protocol) sessions over a child's stdio, on the
//! official `agent-client-protocol` SDK. Nandi is the client; the harness
//! CLI is the agent.
//!
//! The SDK runs a connection for the lifetime of one future, so a session
//! is a background task that owns the connection and takes commands
//! (prompt, cancel, shutdown) over a channel. The SDK's dispatch loop
//! delivers `session/update` notifications in order and before the
//! `session/prompt` response, so a turn's stream always ends with
//! `TurnFinished` after every update.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AuthMethodId, AuthenticateRequest, CancelNotification, ContentBlock, EnvVariable,
    InitializeRequest, McpServer, McpServerHttp, McpServerStdio, NewSessionRequest,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, SessionUpdate,
    StopReason, TextContent, ToolCallContent, ToolCallStatus, ToolKind,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use futures_util::{AsyncRead, AsyncWrite};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tokio::sync::{mpsc, oneshot};

use super::adapter::{EventStream, HarnessError, McpServerRef};

/// Per-request timeout unless the config says otherwise. `session/prompt`
/// has none: a turn takes as long as it takes (cancel stops it).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Buffered updates per turn before the session waits for the reader
const TURN_BUFFER: usize = 1024;
/// Longest tool output kept in an update (the CLI keeps the full output)
const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;

/// Token usage a harness reports for a turn
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AcpUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_input_tokens: u32,
    pub cost_usd: Option<f64>,
}

/// What a session tells the client during a turn (ACP `session/update`,
/// plus the turn's end and errors), normalized across harnesses
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AcpSessionUpdate {
    /// `agent_message_chunk`
    MessageDelta {
        text: String,
    },
    /// `agent_thought_chunk`
    ThinkingDelta {
        text: String,
    },
    /// `tool_call`
    #[serde(rename_all = "camelCase")]
    ToolCallStarted {
        tool_call_id: String,
        title: String,
        /// ACP tool kind: read, edit, execute, fetch, search, …
        tool_kind: Option<String>,
        input: Value,
    },
    /// `tool_call_update` with status completed or failed
    #[serde(rename_all = "camelCase")]
    ToolCallFinished {
        tool_call_id: String,
        output: String,
        is_error: bool,
    },
    /// The `session/prompt` response
    #[serde(rename_all = "camelCase")]
    TurnFinished {
        /// end_turn, max_tokens, max_turn_requests, refusal, cancelled
        stop_reason: String,
        usage: Option<AcpUsage>,
    },
    /// `session/request_permission`, already answered per policy
    #[serde(rename_all = "camelCase")]
    PermissionRequest {
        tool_call_id: String,
        title: String,
        /// The option Nandi picked (`allow_once`, `reject_once`, …), or
        /// `cancelled` when the harness offered nothing that fits
        decision: String,
    },
    Error {
        message: String,
    },
}

impl AcpSessionUpdate {
    /// The serde tag, e.g. `messageDelta` (the `harness_events.kind`)
    pub fn kind(&self) -> &'static str {
        match self {
            Self::MessageDelta { .. } => "messageDelta",
            Self::ThinkingDelta { .. } => "thinkingDelta",
            Self::ToolCallStarted { .. } => "toolCallStarted",
            Self::ToolCallFinished { .. } => "toolCallFinished",
            Self::TurnFinished { .. } => "turnFinished",
            Self::PermissionRequest { .. } => "permissionRequest",
            Self::Error { .. } => "error",
        }
    }
}

/// What the agent said about itself in `initialize`
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AcpAgentInfo {
    pub protocol_version: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub auth_methods: Vec<String>,
}

/// How Nandi answers `session/request_permission`. The container is the
/// sandbox, so the default allows; an agent's `allowed-tools` narrows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum PermissionPolicy {
    #[default]
    AllowAll,
    /// Allowed when the tool's title or ACP kind contains one of these
    /// (case-insensitive), e.g. `read`, `edit`, `Bash`
    Allow(Vec<String>),
    DenyAll,
    /// `allow` (`None`: everything) minus `deny`, which wins
    Except {
        allow: Option<Vec<String>>,
        deny: Vec<String>,
    },
}

impl PermissionPolicy {
    /// From the agent's tools as ACP kinds: `allowed` (empty: everything)
    /// and what a policy `denied`
    pub fn from_tools(allowed: &[String], denied: &[String]) -> Self {
        match (allowed.is_empty(), denied.is_empty()) {
            (true, true) => Self::AllowAll,
            (false, true) => Self::Allow(allowed.to_vec()),
            (allow_all, false) => Self::Except {
                allow: (!allow_all).then(|| allowed.to_vec()),
                deny: denied.to_vec(),
            },
        }
    }

    fn allows(&self, title: &str, kind: Option<&str>) -> bool {
        match self {
            Self::AllowAll => true,
            Self::DenyAll => false,
            Self::Allow(tools) => matches_tool(tools, title, kind),
            Self::Except { allow, deny } => {
                !matches_tool(deny, title, kind)
                    && allow
                        .as_ref()
                        .is_none_or(|allow| matches_tool(allow, title, kind))
            }
        }
    }
}

/// Whether one of `tools` names the call: its ACP kind, or a word in its
/// title (case-insensitive)
fn matches_tool(tools: &[String], title: &str, kind: Option<&str>) -> bool {
    let title = title.to_lowercase();
    let kind = kind.map(str::to_lowercase);
    tools
        .iter()
        .map(|t| t.to_lowercase())
        .any(|tool| title.contains(&tool) || kind.as_deref().is_some_and(|k| k == tool))
}

/// A harness tool call waiting for a person's approval
#[derive(Debug, Clone)]
pub struct AskedCall {
    pub tool_call_id: String,
    /// Nandi's name for it (`shell`, `write_file`, …), else the CLI's title
    pub tool: String,
    /// `{title, input}`: what the Inbox shows
    pub args: Value,
}

/// Asks a person whether a harness tool call may run
#[async_trait::async_trait]
pub trait PermissionAsker: Send + Sync {
    /// `true` once approved; `false` when rejected, expired, or the run ended
    async fn ask(&self, call: AskedCall) -> bool;
}

/// The calls a person approves first (an agent's `requires_approval`, as
/// ACP kinds), and who asks them. No asker: those calls are refused.
#[derive(Clone, Default)]
pub struct Approvals {
    pub kinds: Vec<String>,
    pub asker: Option<Arc<dyn PermissionAsker>>,
}

impl std::fmt::Debug for Approvals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Approvals")
            .field("kinds", &self.kinds)
            .field("asker", &self.asker.is_some())
            .finish()
    }
}

/// How a permission request is answered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Allow,
    Reject,
    /// A person decides (`Approvals::asker`)
    Ask,
}

fn decide(
    policy: &PermissionPolicy,
    approvals: &Approvals,
    title: &str,
    kind: Option<&str>,
) -> Decision {
    if !policy.allows(title, kind) {
        Decision::Reject
    } else if matches_tool(&approvals.kinds, title, kind) {
        Decision::Ask
    } else {
        Decision::Allow
    }
}

/// The call as the Inbox shows it, named the way Nandi's own tools are
fn asked_call(request: &RequestPermissionRequest, title: &str, kind: Option<&str>) -> AskedCall {
    let tool = match kind {
        Some("execute") => "shell",
        Some("read" | "search") => "read_file",
        Some("edit" | "delete" | "move") => "write_file",
        Some("fetch") => "fetch",
        _ => title,
    };
    AskedCall {
        tool_call_id: request.tool_call.tool_call_id.0.to_string(),
        tool: tool.to_string(),
        args: serde_json::json!({
            "title": title,
            "input": request.tool_call.fields.raw_input.clone().unwrap_or(Value::Null),
        }),
    }
}

/// How to open the session
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// The agent's cwd (inside the container for container placement)
    pub cwd: PathBuf,
    pub mcp_servers: Vec<McpServerRef>,
    pub timeout: Duration,
    pub permissions: PermissionPolicy,
    pub approvals: Approvals,
    /// `authenticate` with this method before `session/new`, when the agent
    /// offers it (e.g. Gemini CLI's `gemini-api-key`)
    pub auth_method: Option<String>,
}

enum Command {
    Prompt {
        text: String,
        updates: mpsc::Sender<AcpSessionUpdate>,
    },
    Cancel,
}

/// Where `session/update`s of the current turn go
type TurnSink = Arc<Mutex<Option<mpsc::Sender<AcpSessionUpdate>>>>;

/// An open ACP session. Dropping it (or `shutdown`) closes the connection;
/// the caller owns the child process.
pub struct AcpClient {
    commands: mpsc::UnboundedSender<Command>,
    task: tokio::task::JoinHandle<Result<(), HarnessError>>,
    info: AcpAgentInfo,
    session_id: String,
}

impl AcpClient {
    /// Runs `initialize` and `session/new` over the child's stdin
    /// (`outgoing`) and stdout (`incoming`), each within `options.timeout`.
    pub async fn connect<W, R>(
        outgoing: W,
        incoming: R,
        options: SessionOptions,
    ) -> Result<Self, HarnessError>
    where
        W: AsyncWrite + Send + 'static,
        R: AsyncRead + Send + 'static,
    {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let timeout = options.timeout;
        let task = tokio::spawn(run_connection(
            ByteStreams::new(outgoing, incoming),
            options,
            receiver,
            ready_tx,
        ));
        // The task also enforces `timeout` per request; this bound covers
        // a child that never answers anything
        let ready = tokio::time::timeout(timeout * 2 + Duration::from_secs(1), ready_rx).await;
        match ready {
            Ok(Ok(Ok((info, session_id)))) => Ok(Self {
                commands,
                task,
                info,
                session_id,
            }),
            Ok(Ok(Err(error))) => {
                task.abort();
                Err(error)
            }
            // The task ended without reporting: it failed or the child died
            Ok(Err(_)) => Err(match task.await {
                Ok(Err(error)) => error,
                _ => HarnessError::Exited(None),
            }),
            Err(_) => {
                task.abort();
                Err(HarnessError::Timeout(timeout))
            }
        }
    }

    pub fn info(&self) -> &AcpAgentInfo {
        &self.info
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Sends `session/prompt`; the stream carries the turn's updates and
    /// ends with `TurnFinished` (or `Error`).
    pub fn prompt(&self, text: &str) -> Result<EventStream, HarnessError> {
        let (updates, stream) = mpsc::channel(TURN_BUFFER);
        self.commands
            .send(Command::Prompt {
                text: text.to_string(),
                updates,
            })
            .map_err(|_| HarnessError::Exited(None))?;
        Ok(stream)
    }

    /// `session/cancel`: the turn ends with stop reason `cancelled`
    pub fn cancel(&self) -> Result<(), HarnessError> {
        self.commands
            .send(Command::Cancel)
            .map_err(|_| HarnessError::Exited(None))
    }

    /// Closes the connection (the child sees stdin close)
    pub async fn shutdown(self) {
        drop(self.commands);
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }

    /// Whether the connection task is still running
    pub fn is_alive(&self) -> bool {
        !self.task.is_finished()
    }
}

async fn run_connection<W, R>(
    transport: ByteStreams<W, R>,
    options: SessionOptions,
    mut commands: mpsc::UnboundedReceiver<Command>,
    ready: oneshot::Sender<Result<(AcpAgentInfo, String), HarnessError>>,
) -> Result<(), HarnessError>
where
    W: AsyncWrite + Send + 'static,
    R: AsyncRead + Send + 'static,
{
    let sink: TurnSink = Arc::default();
    let notifications = sink.clone();
    let permissions_sink = sink.clone();
    let policy = options.permissions.clone();
    let approvals = options.approvals.clone();
    let mut ready = Some(ready);

    let result = Client
        .builder()
        .name("nandi")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let current = notifications.lock().ok().and_then(|s| s.clone());
                if let (Some(updates), Some(update)) = (current, normalize(notification.update)) {
                    let _ = updates.send(update).await;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let title = request.tool_call.fields.title.clone().unwrap_or_default();
                let kind = request.tool_call.fields.kind.as_ref().and_then(tool_kind);
                let current = permissions_sink.lock().ok().and_then(|s| s.clone());
                let allowed = match decide(&policy, &approvals, &title, kind.as_deref()) {
                    Decision::Allow => true,
                    Decision::Reject => false,
                    Decision::Ask => {
                        // A person may take a while; this handler blocks
                        // the dispatch loop, so wait in a task of its own
                        let asker = approvals.asker.clone();
                        tokio::spawn(async move {
                            let allowed = match asker {
                                Some(asker) => {
                                    asker
                                        .ask(asked_call(&request, &title, kind.as_deref()))
                                        .await
                                }
                                None => false,
                            };
                            let (update, response) = answer_permission(&request, allowed);
                            if let Some(updates) = current {
                                let _ = updates.send(update).await;
                            }
                            let _ = responder.respond(response);
                        });
                        return Ok(());
                    }
                };
                let (update, response) = answer_permission(&request, allowed);
                if let Some(updates) = current {
                    let _ = updates.send(update).await;
                }
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async |cx: ConnectionTo<Agent>| {
            let opened = open_session(&cx, &options).await;
            let session_id = match opened {
                Ok((info, session_id)) => {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(Ok((info, session_id.clone())));
                    }
                    session_id
                }
                Err(error) => {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(Err(error));
                    }
                    return Ok(());
                }
            };
            while let Some(command) = commands.recv().await {
                let Command::Prompt { text, updates } = command else {
                    continue; // nothing to cancel between turns
                };
                if let Ok(mut current) = sink.lock() {
                    *current = Some(updates.clone());
                }
                let prompt = cx
                    .send_request(PromptRequest::new(
                        session_id.clone(),
                        vec![ContentBlock::Text(TextContent::new(text))],
                    ))
                    .block_task();
                tokio::pin!(prompt);
                let finished = loop {
                    tokio::select! {
                        response = &mut prompt => break response,
                        command = commands.recv() => match command {
                            Some(Command::Cancel) => {
                                let _ = cx.send_notification(CancelNotification::new(session_id.clone()));
                            }
                            // Only one turn at a time: a second prompt waits
                            Some(Command::Prompt { updates: queued, .. }) => {
                                let _ = queued
                                    .send(AcpSessionUpdate::Error {
                                        message: "A turn is already running".to_string(),
                                    })
                                    .await;
                            }
                            None => return Ok(()),
                        },
                    }
                };
                if let Ok(mut current) = sink.lock() {
                    *current = None;
                }
                let last = match finished {
                    Ok(response) => AcpSessionUpdate::TurnFinished {
                        stop_reason: stop_reason(&response.stop_reason).to_string(),
                        usage: response.usage.map(|u| AcpUsage {
                            input_tokens: clamp(u.input_tokens),
                            output_tokens: clamp(u.output_tokens),
                            cached_input_tokens: clamp(u.cached_read_tokens.unwrap_or(0)),
                            cost_usd: None,
                        }),
                    },
                    Err(error) => AcpSessionUpdate::Error {
                        message: error.message.clone(),
                    },
                };
                let _ = updates.send(last).await;
            }
            Ok(())
        })
        .await;
    // `ready` is still here only if the connection never got that far
    let error = result
        .err()
        .map(|e| HarnessError::Protocol(e.message.clone()));
    if let (Some(ready), Some(error)) = (ready.take(), error.clone()) {
        let _ = ready.send(Err(error));
    }
    error.map_or(Ok(()), Err)
}

async fn open_session(
    cx: &ConnectionTo<Agent>,
    options: &SessionOptions,
) -> Result<(AcpAgentInfo, String), HarnessError> {
    let initialize = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task();
    let init = tokio::time::timeout(options.timeout, initialize)
        .await
        .map_err(|_| HarnessError::Timeout(options.timeout))?
        .map_err(rpc_error)?;
    let info = AcpAgentInfo {
        protocol_version: serde_json::to_value(init.protocol_version)
            .ok()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        name: init.agent_info.as_ref().map(|i| i.name.clone()),
        version: init.agent_info.as_ref().map(|i| i.version.clone()),
        auth_methods: init
            .auth_methods
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
            .collect(),
    };
    if let Some(method) = options
        .auth_method
        .as_ref()
        .filter(|m| info.auth_methods.contains(m))
    {
        let authenticate = cx
            .send_request(AuthenticateRequest::new(AuthMethodId::new(method.as_str())))
            .block_task();
        tokio::time::timeout(options.timeout, authenticate)
            .await
            .map_err(|_| HarnessError::Timeout(options.timeout))?
            .map_err(|e| HarnessError::Auth(e.message))?;
    }
    let request = NewSessionRequest::new(options.cwd.clone())
        .mcp_servers(options.mcp_servers.iter().map(mcp_server).collect());
    let session = tokio::time::timeout(options.timeout, cx.send_request(request).block_task())
        .await
        .map_err(|_| HarnessError::Timeout(options.timeout))?
        .map_err(rpc_error)?;
    Ok((info, session.session_id.0.to_string()))
}

fn rpc_error(error: agent_client_protocol::Error) -> HarnessError {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_i64())
        .unwrap_or(-32603);
    // -32000: the agent wants authentication first (ACP auth_required)
    if code == -32000 {
        return HarnessError::Auth(error.message);
    }
    HarnessError::Rpc {
        code,
        message: error.message,
    }
}

fn mcp_server(server: &McpServerRef) -> McpServer {
    match server {
        McpServerRef::Stdio {
            name,
            command,
            args,
            env,
        } => McpServer::Stdio(
            McpServerStdio::new(name.clone(), command.clone())
                .args(args.clone())
                .env(env.iter().map(|(k, v)| EnvVariable::new(k, v)).collect()),
        ),
        McpServerRef::Http { name, url, headers } => McpServer::Http(
            McpServerHttp::new(name.clone(), url.clone()).headers(
                headers
                    .iter()
                    .map(|(k, v)| agent_client_protocol::schema::v1::HttpHeader::new(k, v))
                    .collect(),
            ),
        ),
    }
}

fn clamp(tokens: u64) -> u32 {
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

fn stop_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        _ => "other",
    }
}

fn tool_kind(kind: &ToolKind) -> Option<String> {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

fn block_text(block: &ContentBlock) -> Option<&str> {
    match block {
        ContentBlock::Text(text) => Some(&text.text),
        _ => None,
    }
}

fn tool_output(content: &[ToolCallContent], raw_output: Option<&Value>) -> String {
    let text: Vec<String> = content
        .iter()
        .filter_map(|c| match c {
            ToolCallContent::Content(c) => block_text(&c.content).map(str::to_string),
            ToolCallContent::Diff(diff) => Some(format!("Edited {}", diff.path.display())),
            _ => None,
        })
        .collect();
    let output = if text.is_empty() {
        match raw_output {
            Some(Value::String(s)) => s.clone(),
            Some(value) => value.to_string(),
            None => String::new(),
        }
    } else {
        text.join("\n")
    };
    crate::services::events::truncate(&output, MAX_TOOL_OUTPUT_CHARS)
}

/// One `session/update`, as Nandi sees it; `None` for updates Nandi doesn't
/// surface (plans, modes, available commands, echoed user text)
pub fn normalize(update: SessionUpdate) -> Option<AcpSessionUpdate> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => block_text(&chunk.content)
            .filter(|t| !t.is_empty())
            .map(|text| AcpSessionUpdate::MessageDelta {
                text: text.to_string(),
            }),
        SessionUpdate::AgentThoughtChunk(chunk) => block_text(&chunk.content)
            .filter(|t| !t.is_empty())
            .map(|text| AcpSessionUpdate::ThinkingDelta {
                text: text.to_string(),
            }),
        SessionUpdate::ToolCall(call) => Some(AcpSessionUpdate::ToolCallStarted {
            tool_call_id: call.tool_call_id.0.to_string(),
            title: call.title,
            tool_kind: tool_kind(&call.kind),
            input: call.raw_input.unwrap_or(Value::Null),
        }),
        SessionUpdate::ToolCallUpdate(update) => {
            let status = update.fields.status?;
            let is_error = match status {
                ToolCallStatus::Completed => false,
                ToolCallStatus::Failed => true,
                _ => return None,
            };
            Some(AcpSessionUpdate::ToolCallFinished {
                tool_call_id: update.tool_call_id.0.to_string(),
                output: tool_output(
                    update.fields.content.as_deref().unwrap_or_default(),
                    update.fields.raw_output.as_ref(),
                ),
                is_error,
            })
        }
        _ => None,
    }
}

/// The answer to a permission request, and the update recording it
fn answer_permission(
    request: &RequestPermissionRequest,
    allowed: bool,
) -> (AcpSessionUpdate, RequestPermissionResponse) {
    let title = request.tool_call.fields.title.clone().unwrap_or_default();
    let wanted: &[PermissionOptionKind] = if allowed {
        &[
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ]
    } else {
        &[
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ]
    };
    let option = wanted
        .iter()
        .find_map(|kind| request.options.iter().find(|o| o.kind == *kind));
    let (decision, outcome) = match option {
        Some(option) => (
            serde_json::to_value(option.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
        ),
        None => ("cancelled".to_string(), RequestPermissionOutcome::Cancelled),
    };
    (
        AcpSessionUpdate::PermissionRequest {
            tool_call_id: request.tool_call.tool_call_id.0.to_string(),
            title,
            decision,
        },
        RequestPermissionResponse::new(outcome),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        AgentCapabilities, ContentChunk, InitializeResponse, NewSessionResponse, PermissionOption,
        PromptResponse, ToolCall, ToolCallUpdate, ToolCallUpdateFields,
    };
    use agent_client_protocol::Responder;
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    fn text(t: &str) -> ContentBlock {
        ContentBlock::Text(TextContent::new(t))
    }

    /// A scripted ACP agent on the other end of an in-memory pipe, built
    /// with the SDK's agent role: each prompt streams a thought, a tool
    /// call, and text, then ends the turn. A prompt containing "wait" ends
    /// only when cancelled.
    pub(crate) fn mock_agent() -> ClientEnds {
        let (client_side, agent_side) = tokio::io::duplex(64 * 1024);
        let (agent_read, agent_write) = tokio::io::split(agent_side);
        let waiting: Arc<Mutex<Option<Responder<PromptResponse>>>> = Arc::default();
        let on_cancel = waiting.clone();
        tokio::spawn(async move {
            let _ = Agent
                .builder()
                .name("mock-agent")
                .on_receive_request(
                    async move |init: InitializeRequest, responder, _cx| {
                        responder.respond(
                            InitializeResponse::new(init.protocol_version)
                                .agent_capabilities(AgentCapabilities::new()),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_: NewSessionRequest, responder, _cx| {
                        responder.respond(NewSessionResponse::new("acp-1"))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |prompt: PromptRequest, responder, cx| {
                        let said = match prompt.prompt.first() {
                            Some(ContentBlock::Text(t)) => t.text.clone(),
                            _ => String::new(),
                        };
                        let update = |u| SessionNotification::new(prompt.session_id.clone(), u);
                        if said.contains("wait") {
                            *waiting.lock().unwrap() = Some(responder);
                            return Ok(());
                        }
                        cx.send_notification(update(SessionUpdate::AgentThoughtChunk(
                            ContentChunk::new(text("thinking")),
                        )))?;
                        cx.send_notification(update(SessionUpdate::ToolCall(
                            ToolCall::new("t1", "Read a.txt")
                                .kind(ToolKind::Read)
                                .raw_input(serde_json::json!({"path": "a.txt"})),
                        )))?;
                        cx.send_notification(update(SessionUpdate::ToolCallUpdate(
                            ToolCallUpdate::new(
                                "t1",
                                ToolCallUpdateFields::new()
                                    .status(ToolCallStatus::Completed)
                                    .raw_output(serde_json::json!("hello")),
                            ),
                        )))?;
                        cx.send_notification(update(SessionUpdate::AgentMessageChunk(
                            ContentChunk::new(text(&format!("You said: {said}"))),
                        )))?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |_: CancelNotification, _cx| {
                        if let Some(responder) = on_cancel.lock().unwrap().take() {
                            let _ = responder.respond(PromptResponse::new(StopReason::Cancelled));
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .connect_to(ByteStreams::new(
                    agent_write.compat_write(),
                    agent_read.compat(),
                ))
                .await;
        });
        let (read, write) = tokio::io::split(client_side);
        ClientEnds { read, write }
    }

    pub(crate) struct ClientEnds {
        pub read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        pub write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    }

    pub(crate) fn options(timeout: Duration) -> SessionOptions {
        SessionOptions {
            cwd: PathBuf::from("/agent/workdir"),
            mcp_servers: vec![],
            timeout,
            permissions: PermissionPolicy::AllowAll,
            approvals: Approvals::default(),
            auth_method: None,
        }
    }

    async fn collect(mut stream: EventStream) -> Vec<AcpSessionUpdate> {
        let mut updates = Vec::new();
        while let Some(update) = stream.recv().await {
            let last = matches!(
                update,
                AcpSessionUpdate::TurnFinished { .. } | AcpSessionUpdate::Error { .. }
            );
            updates.push(update);
            if last {
                break;
            }
        }
        updates
    }

    #[tokio::test]
    async fn a_prompt_round_trips_with_every_update_before_the_end() {
        let ends = mock_agent();
        let client = AcpClient::connect(
            ends.write.compat_write(),
            ends.read.compat(),
            options(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(client.session_id(), "acp-1");

        let updates = collect(client.prompt("hi").unwrap()).await;
        assert_eq!(
            updates,
            vec![
                AcpSessionUpdate::ThinkingDelta {
                    text: "thinking".into()
                },
                AcpSessionUpdate::ToolCallStarted {
                    tool_call_id: "t1".into(),
                    title: "Read a.txt".into(),
                    tool_kind: Some("read".into()),
                    input: serde_json::json!({"path": "a.txt"}),
                },
                AcpSessionUpdate::ToolCallFinished {
                    tool_call_id: "t1".into(),
                    output: "hello".into(),
                    is_error: false,
                },
                AcpSessionUpdate::MessageDelta {
                    text: "You said: hi".into()
                },
                AcpSessionUpdate::TurnFinished {
                    stop_reason: "end_turn".into(),
                    usage: None
                },
            ]
        );
        // The session stays open for the next turn
        let second = collect(client.prompt("again").unwrap()).await;
        assert!(matches!(
            second.last(),
            Some(AcpSessionUpdate::TurnFinished { .. })
        ));
        client.shutdown().await;
    }

    #[tokio::test]
    async fn cancel_ends_the_turn_as_cancelled() {
        let ends = mock_agent();
        let client = AcpClient::connect(
            ends.write.compat_write(),
            ends.read.compat(),
            options(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        let stream = client.prompt("wait for me").unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        client.cancel().unwrap();
        let updates = collect(stream).await;
        assert_eq!(
            updates.last(),
            Some(&AcpSessionUpdate::TurnFinished {
                stop_reason: "cancelled".into(),
                usage: None
            })
        );
    }

    #[tokio::test]
    async fn a_silent_agent_times_out() {
        // Nobody answers on the other end
        let (client_side, _agent_side) = tokio::io::duplex(1024);
        let (read, write) = tokio::io::split(client_side);
        let started = std::time::Instant::now();
        let result = AcpClient::connect(
            write.compat_write(),
            read.compat(),
            options(Duration::from_millis(200)),
        )
        .await;
        assert!(
            matches!(result, Err(HarnessError::Timeout(_))),
            "{:?}",
            result.err()
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn a_child_that_exits_fails_the_connect() {
        let (client_side, agent_side) = tokio::io::duplex(1024);
        drop(agent_side);
        let (read, write) = tokio::io::split(client_side);
        let result = AcpClient::connect(
            write.compat_write(),
            read.compat(),
            options(Duration::from_secs(2)),
        )
        .await;
        assert!(result.is_err());
    }

    fn permission(title: &str, kind: ToolKind) -> RequestPermissionRequest {
        RequestPermissionRequest::new(
            "acp-1",
            ToolCallUpdate::new(
                "t1",
                ToolCallUpdateFields::new()
                    .title(title.to_string())
                    .kind(kind),
            ),
            vec![
                PermissionOption::new("yes", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new("no", "Reject", PermissionOptionKind::RejectOnce),
            ],
        )
    }

    #[test]
    fn permissions_follow_the_policy_and_the_approvals() {
        let none = Approvals::default();
        let decision = |policy: &PermissionPolicy, approvals: &Approvals| {
            decide(policy, approvals, "Run git status", Some("execute"))
        };
        assert_eq!(
            decision(&PermissionPolicy::AllowAll, &none),
            Decision::Allow
        );
        assert_eq!(
            decision(&PermissionPolicy::DenyAll, &none),
            Decision::Reject
        );
        let execute = PermissionPolicy::Allow(vec!["execute".into()]);
        assert_eq!(decision(&execute, &none), Decision::Allow);
        let read = PermissionPolicy::Allow(vec!["read".into()]);
        assert_eq!(decision(&read, &none), Decision::Reject);
        let denied = PermissionPolicy::from_tools(&[], &["execute".into()]);
        assert_eq!(decision(&denied, &none), Decision::Reject, "a deny wins");
        assert_eq!(
            decide(&denied, &none, "Edit a.txt", Some("edit")),
            Decision::Allow
        );

        let ask = Approvals {
            kinds: vec!["execute".into(), "edit".into()],
            asker: None,
        };
        assert_eq!(decision(&PermissionPolicy::AllowAll, &ask), Decision::Ask);
        // Not allowed at all beats asking
        assert_eq!(decision(&read, &ask), Decision::Reject);
        assert_eq!(
            decide(
                &PermissionPolicy::AllowAll,
                &ask,
                "Read a.txt",
                Some("read")
            ),
            Decision::Allow
        );
    }

    #[test]
    fn answers_pick_the_matching_option() {
        let request = permission("Run git status", ToolKind::Execute);
        let decision = |allowed| match answer_permission(&request, allowed).0 {
            AcpSessionUpdate::PermissionRequest { decision, .. } => decision,
            other => panic!("{other:?}"),
        };
        assert_eq!(decision(true), "allow_once");
        assert_eq!(decision(false), "reject_once");
        let none = RequestPermissionRequest::new(
            "acp-1",
            ToolCallUpdate::new("t1", ToolCallUpdateFields::new()),
            vec![],
        );
        assert!(matches!(
            answer_permission(&none, true),
            (AcpSessionUpdate::PermissionRequest { ref decision, .. }, _) if decision == "cancelled"
        ));
    }

    #[test]
    fn asked_calls_use_nandi_tool_names() {
        let request = permission("Run git push", ToolKind::Execute);
        let call = asked_call(&request, "Run git push", Some("execute"));
        assert_eq!(call.tool, "shell");
        assert_eq!(call.tool_call_id, "t1");
        assert_eq!(call.args["title"], "Run git push");
        assert_eq!(asked_call(&request, "Deploy", Some("other")).tool, "Deploy");
    }
}
