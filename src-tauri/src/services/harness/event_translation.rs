//! ACP updates → the run events a native run emits, so the chat, run
//! history, and evidence work the same whatever ran the turn:
//!
//! | `AcpSessionUpdate` | Nandi |
//! |---|---|
//! | `MessageDelta` | `agent://token` on the step's assistant message |
//! | `ThinkingDelta` | the step's `reasoning` |
//! | `ToolCallStarted` | `agent://tool_call` (closes the text step) |
//! | `ToolCallFinished` | `agent://tool_result` |
//! | `PermissionRequest` | `harness://permission_request` (answered already) |
//! | `TurnFinished` | the last step's metadata; the caller ends the run |
//! | `Error` | the caller fails the run |
//!
//! Every update is also `harness://turn_delta` for the session viewer.

use serde::Serialize;
use serde_json::Value;

use super::acp::{AcpSessionUpdate, AcpUsage};
use crate::db::{AssistantMeta, PlannedCall};
use crate::engine::ToolOutcome;
use crate::services::events::{self, EventBus, NandiEvent, RunContext};

/// Prefix for tool call IDs from a harness (they're only unique per session)
const CALL_ID_PREFIX: &str = "harness";

/// `harness://turn_delta`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessTurnDelta {
    pub session_id: String,
    pub run_id: String,
    pub update: AcpSessionUpdate,
}

impl NandiEvent for HarnessTurnDelta {
    const NAME: &'static str = super::HARNESS_TURN_DELTA;
}

/// `harness://permission_request`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessPermissionRequest {
    pub session_id: String,
    pub run_id: String,
    pub agent_id: String,
    pub tool_call_id: String,
    pub title: String,
    pub decision: String,
}

impl NandiEvent for HarnessPermissionRequest {
    const NAME: &'static str = super::HARNESS_PERMISSION_REQUEST;
}

/// What one update does to the run (pure; `TurnTranslator::apply` runs it)
#[derive(Debug, Clone, PartialEq)]
pub enum RunAction {
    Token(String),
    Reasoning(String),
    ToolCall(PlannedCall),
    ToolResult {
        call_id: String,
        outcome: (String, bool),
    },
    Permission {
        tool_call_id: String,
        title: String,
        decision: String,
    },
    Finished {
        stop_reason: String,
        usage: Option<AcpUsage>,
    },
    Failed(String),
}

pub fn call_id(session_id: &str, tool_call_id: &str) -> String {
    format!("{CALL_ID_PREFIX}-{session_id}-{tool_call_id}")
}

/// The run action for `update`
pub fn translate(session_id: &str, update: &AcpSessionUpdate) -> RunAction {
    match update {
        AcpSessionUpdate::MessageDelta { text } => RunAction::Token(text.clone()),
        AcpSessionUpdate::ThinkingDelta { text } => RunAction::Reasoning(text.clone()),
        AcpSessionUpdate::ToolCallStarted {
            tool_call_id,
            title,
            tool_kind,
            input,
        } => RunAction::ToolCall(PlannedCall {
            id: call_id(session_id, tool_call_id),
            // The harness's own title ("Read src/main.rs"); the kind when
            // it has none
            tool: if title.trim().is_empty() {
                tool_kind.clone().unwrap_or_else(|| "tool".to_string())
            } else {
                title.clone()
            },
            args: match input {
                Value::Null => serde_json::json!({}),
                other => other.clone(),
            },
            // The harness already asked (and Nandi answered) if it needed to
            requires_approval: false,
        }),
        AcpSessionUpdate::ToolCallFinished {
            tool_call_id,
            output,
            is_error,
        } => RunAction::ToolResult {
            call_id: call_id(session_id, tool_call_id),
            outcome: (output.clone(), *is_error),
        },
        AcpSessionUpdate::PermissionRequest {
            tool_call_id,
            title,
            decision,
        } => RunAction::Permission {
            tool_call_id: tool_call_id.clone(),
            title: title.clone(),
            decision: decision.clone(),
        },
        AcpSessionUpdate::TurnFinished { stop_reason, usage } => RunAction::Finished {
            stop_reason: stop_reason.clone(),
            usage: *usage,
        },
        AcpSessionUpdate::Error { message } => RunAction::Failed(message.clone()),
    }
}

/// How a turn ended
#[derive(Debug, Clone, PartialEq)]
pub enum TurnOutcome {
    Finished {
        stop_reason: String,
        usage: Option<AcpUsage>,
    },
    Failed(String),
}

/// Writes one harness turn into a run as it streams. A step is the text
/// before a batch of tool calls, like a native step: text that follows a
/// tool call starts the next step.
pub struct TurnTranslator {
    bus: EventBus,
    ctx: RunContext,
    session_id: String,
    step: u32,
    message_id: String,
    /// The current step has text, reasoning, or calls not yet finished
    open: bool,
    reasoning: String,
    calls: Vec<PlannedCall>,
}

impl TurnTranslator {
    pub fn new(bus: EventBus, ctx: RunContext, session_id: &str, first_step: u32) -> Self {
        Self {
            bus,
            ctx,
            session_id: session_id.to_string(),
            step: first_step,
            message_id: uuid::Uuid::new_v4().to_string(),
            open: false,
            reasoning: String::new(),
            calls: Vec::new(),
        }
    }

    /// Applies `update`; `Some` when the turn is over
    pub async fn apply(
        &mut self,
        update: &AcpSessionUpdate,
    ) -> Result<Option<TurnOutcome>, String> {
        events::emit(
            &self.bus,
            &HarnessTurnDelta {
                session_id: self.session_id.clone(),
                run_id: self.ctx.run_id.clone(),
                update: update.clone(),
            },
        );
        match translate(&self.session_id, update) {
            RunAction::Token(text) => {
                if !self.calls.is_empty() {
                    self.finish_step().await?;
                }
                self.open = true;
                self.bus
                    .token(&self.ctx, &self.message_id, self.step, &text)
                    .await?;
            }
            RunAction::Reasoning(text) => {
                if !self.calls.is_empty() {
                    self.finish_step().await?;
                }
                self.open = true;
                self.reasoning.push_str(&text);
            }
            RunAction::ToolCall(call) => {
                self.open = true;
                self.calls.push(call.clone());
                self.bus.tool_call(&self.ctx, &call).await?;
            }
            RunAction::ToolResult { call_id, outcome } => {
                let call = self.calls.iter().find(|c| c.id == call_id).cloned();
                if let Some(call) = call {
                    let outcome = ToolOutcome {
                        result: outcome.0,
                        is_error: outcome.1,
                    };
                    self.bus.tool_result(&self.ctx, &call, &outcome).await?;
                }
            }
            // Kept in the run's log: Run Detail lists what the runtime
            // allowed or denied on its own
            RunAction::Permission {
                tool_call_id,
                title,
                decision,
            } => {
                self.bus
                    .record(
                        &self.ctx.run_id,
                        HarnessPermissionRequest {
                            session_id: self.session_id.clone(),
                            run_id: self.ctx.run_id.clone(),
                            agent_id: self.ctx.agent_id.clone(),
                            tool_call_id,
                            title,
                            decision,
                        },
                    )
                    .await?
            }
            RunAction::Finished { stop_reason, usage } => {
                self.finish_step().await?;
                return Ok(Some(TurnOutcome::Finished { stop_reason, usage }));
            }
            RunAction::Failed(message) => {
                self.finish_step().await?;
                return Ok(Some(TurnOutcome::Failed(message)));
            }
        }
        Ok(None)
    }

    /// Records the open step's metadata and starts the next one
    async fn finish_step(&mut self) -> Result<(), String> {
        if !self.open {
            return Ok(());
        }
        let meta = AssistantMeta {
            step: self.step,
            planned_calls: std::mem::take(&mut self.calls),
            reasoning: Some(std::mem::take(&mut self.reasoning)).filter(|r| !r.is_empty()),
        };
        self.bus
            .finish_step(&self.ctx, &self.message_id, meta)
            .await?;
        self.step += 1;
        self.message_id = uuid::Uuid::new_v4().to_string();
        self.open = false;
        Ok(())
    }

    /// The next step's index (for a follow-up turn on the same run)
    pub fn next_step(&self) -> u32 {
        self.step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::services::events::tests::{bus, started};

    fn all_variants() -> Vec<AcpSessionUpdate> {
        vec![
            AcpSessionUpdate::ThinkingDelta { text: "hmm".into() },
            AcpSessionUpdate::MessageDelta {
                text: "Reading".into(),
            },
            AcpSessionUpdate::PermissionRequest {
                tool_call_id: "t1".into(),
                title: "Read a.txt".into(),
                decision: "allow_once".into(),
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
                text: "It says hello".into(),
            },
            AcpSessionUpdate::TurnFinished {
                stop_reason: "end_turn".into(),
                usage: Some(AcpUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..AcpUsage::default()
                }),
            },
        ]
    }

    #[test]
    fn every_variant_translates() {
        let actions: Vec<RunAction> = all_variants()
            .iter()
            .chain([&AcpSessionUpdate::Error {
                message: "boom".into(),
            }])
            .map(|u| translate("s1", u))
            .collect();
        assert_eq!(actions[0], RunAction::Reasoning("hmm".into()));
        assert_eq!(actions[1], RunAction::Token("Reading".into()));
        assert!(
            matches!(&actions[2], RunAction::Permission { decision, .. } if decision == "allow_once")
        );
        let RunAction::ToolCall(call) = &actions[3] else {
            panic!("{:?}", actions[3]);
        };
        assert_eq!(call.id, "harness-s1-t1");
        assert_eq!(call.tool, "Read a.txt");
        assert!(!call.requires_approval);
        assert_eq!(
            actions[4],
            RunAction::ToolResult {
                call_id: "harness-s1-t1".into(),
                outcome: ("hello".into(), false)
            }
        );
        assert!(
            matches!(&actions[6], RunAction::Finished { stop_reason, usage: Some(u) }
            if stop_reason == "end_turn" && u.input_tokens == 10)
        );
        assert_eq!(actions[7], RunAction::Failed("boom".into()));
    }

    #[test]
    fn a_tool_without_a_title_uses_its_kind() {
        let action = translate(
            "s1",
            &AcpSessionUpdate::ToolCallStarted {
                tool_call_id: "t".into(),
                title: " ".into(),
                tool_kind: Some("execute".into()),
                input: Value::Null,
            },
        );
        let RunAction::ToolCall(call) = action else {
            panic!()
        };
        assert_eq!(call.tool, "execute");
        assert_eq!(call.args, serde_json::json!({}));
    }

    #[tokio::test]
    async fn a_turn_becomes_steps_tool_calls_and_tokens() {
        let (bus, sink) = bus();
        let ctx = started(&bus, "r1").await;
        sink.take();
        let mut turn = TurnTranslator::new(bus.clone(), ctx.clone(), "s1", 0);
        let mut outcome = None;
        for update in all_variants() {
            outcome = turn.apply(&update).await.unwrap();
        }
        assert!(matches!(outcome, Some(TurnOutcome::Finished { .. })));
        let names = sink.names();
        for name in [
            "harness://turn_delta",
            "agent://token",
            "agent://tool_call",
            "agent://tool_result",
            "harness://permission_request",
        ] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }
        let messages = bus
            .db
            .call(|conn| db::list_run_messages(conn, "r1"))
            .await
            .unwrap();
        let assistant: Vec<_> = messages
            .iter()
            .filter(|m| m.role == db::role::ASSISTANT)
            .collect();
        // Text + call, then the answer after the call
        assert_eq!(assistant.len(), 2);
        assert_eq!(assistant[0].content, "Reading");
        assert_eq!(
            assistant[0].metadata["plannedCalls"][0]["id"],
            "harness-s1-t1"
        );
        assert_eq!(assistant[0].metadata["reasoning"], "hmm");
        assert_eq!(assistant[1].content, "It says hello");
        assert_eq!(turn.next_step(), 2);
        let tool = messages
            .iter()
            .find(|m| m.role == db::role::TOOL_CALL)
            .unwrap();
        assert_eq!(tool.metadata["result"], "hello");
        // The runtime's own decision is kept in the run's log for Run Detail
        let permissions: Vec<String> = bus
            .db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT payload FROM run_events WHERE run_id = 'r1' AND kind = ?1")?;
                let rows = stmt
                    .query_map(["harness://permission_request"], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(permissions.len(), 1);
        assert!(permissions[0].contains("allow_once"), "{permissions:?}");
    }

    #[tokio::test]
    async fn an_error_ends_the_turn() {
        let (bus, _) = bus();
        let ctx = started(&bus, "r2").await;
        let mut turn = TurnTranslator::new(bus, ctx, "s1", 0);
        let outcome = turn
            .apply(&AcpSessionUpdate::Error {
                message: "crashed".into(),
            })
            .await
            .unwrap();
        assert_eq!(outcome, Some(TurnOutcome::Failed("crashed".into())));
    }
}
