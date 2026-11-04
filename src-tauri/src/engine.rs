//! Turn engines decide what an agent says and which tools it calls.
//!
//! Only the canned engine exists for now: it replays a script stored in the
//! `settings` table (key `canned_script`), so the state machine and event
//! flow can be exercised without an LLM. Edit the setting to change the
//! script; it's re-read at the start of each step.
//!
//! An LLM engine would implement [`TurnEngine`] by mapping the run's
//! persisted messages to provider calls.

use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::{self, PlannedCall};
use crate::model::STUDIO_AGENT_ID;

pub const CANNED_SCRIPT_KEY: &str = "canned_script";
const DEFAULT_TOKEN_DELAY_MS: u64 = 35;

/// One assistant step: text, then zero or more tool calls.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineStep {
    pub text: String,
    pub calls: Vec<PlannedCall>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub result: String,
    pub is_error: bool,
}

pub trait TurnEngine: Send + Sync {
    /// Plans step `step_index` (0-based) of a run started with `input`.
    /// `None` means the run is complete.
    fn plan(&self, agent_id: &str, input: &str, step_index: u32) -> Option<EngineStep>;

    fn execute(&self, call: &PlannedCall) -> ToolOutcome;

    /// Pause between streamed text chunks
    fn token_delay(&self) -> Duration;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CannedScript {
    /// Steps for every agent run
    pub agent: Vec<CannedStep>,
    /// Steps for Studio conversations
    pub studio: Vec<CannedStep>,
    #[serde(default = "default_token_delay")]
    pub token_delay_ms: u64,
}

fn default_token_delay() -> u64 {
    DEFAULT_TOKEN_DELAY_MS
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CannedStep {
    /// `{{input}}` is replaced with the user's message
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<CannedToolCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CannedToolCall {
    pub tool: String,
    pub args: Value,
    pub result: String,
    #[serde(default)]
    pub requires_approval: bool,
    #[serde(default)]
    pub is_error: bool,
}

/// Seeded into `settings` on first run. Exercises streaming, an automatic
/// tool call, and one that needs approval.
pub fn default_script() -> Value {
    json!({
        "agent": [
            {
                "text": "Let me take a look at the workspace before answering.",
                "toolCalls": [
                    {
                        "tool": "list_dir",
                        "args": { "path": "." },
                        "result": "[\"notes/\", \"report.md\"]"
                    },
                    {
                        "tool": "shell",
                        "args": { "cmd": "wc -w report.md" },
                        "result": "{\"stdout\": \"1284 report.md\\n\", \"stderr\": \"\", \"exit_code\": 0}",
                        "requiresApproval": true
                    }
                ]
            },
            {
                "text": "Done. For **\"{{input}}\"**: the workspace has a `notes/` folder and a 1,284-word `report.md`.\n\n_This is a canned response from the local database — no model was called._"
            }
        ],
        "studio": [
            {
                "text": "Nandi isn't connected to a model yet. This canned reply comes from the local database.\n\nYou said: \"{{input}}\""
            }
        ],
        "tokenDelayMs": DEFAULT_TOKEN_DELAY_MS
    })
}

pub struct CannedEngine {
    script: CannedScript,
}

impl CannedEngine {
    /// Loads the script from settings, falling back to the default if it's
    /// missing or invalid.
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let script = db::get_setting(conn, CANNED_SCRIPT_KEY)?
            .and_then(|value| match serde_json::from_value(value) {
                Ok(script) => Some(script),
                Err(e) => {
                    log::warn!("Invalid {CANNED_SCRIPT_KEY} setting, using default: {e}");
                    None
                }
            })
            .unwrap_or_else(|| {
                serde_json::from_value(default_script()).expect("default script is valid")
            });
        Ok(Self { script })
    }

    fn steps(&self, agent_id: &str) -> &[CannedStep] {
        if agent_id == STUDIO_AGENT_ID {
            &self.script.studio
        } else {
            &self.script.agent
        }
    }
}

impl TurnEngine for CannedEngine {
    fn plan(&self, agent_id: &str, input: &str, step_index: u32) -> Option<EngineStep> {
        let step = self.steps(agent_id).get(step_index as usize)?;
        Some(EngineStep {
            text: step.text.replace("{{input}}", input),
            calls: step
                .tool_calls
                .iter()
                .map(|call| PlannedCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    tool: call.tool.clone(),
                    args: call.args.clone(),
                    requires_approval: call.requires_approval,
                })
                .collect(),
        })
    }

    fn execute(&self, call: &PlannedCall) -> ToolOutcome {
        let scripted = [&self.script.agent, &self.script.studio]
            .into_iter()
            .flatten()
            .flat_map(|step| &step.tool_calls)
            .find(|scripted| scripted.tool == call.tool && scripted.args == call.args);
        match scripted {
            Some(scripted) => ToolOutcome {
                result: scripted.result.clone(),
                is_error: scripted.is_error,
            },
            None => ToolOutcome {
                result: format!("Canned engine has no result for '{}'", call.tool),
                is_error: true,
            },
        }
    }

    fn token_delay(&self) -> Duration {
        Duration::from_millis(self.script.token_delay_ms)
    }
}

/// Splits text into word-sized chunks for streaming (whitespace kept).
pub fn chunk_text(text: &str) -> Vec<&str> {
    text.split_inclusive(char::is_whitespace).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> CannedEngine {
        CannedEngine {
            script: serde_json::from_value(default_script()).unwrap(),
        }
    }

    #[test]
    fn plans_agent_steps_then_finishes() {
        let engine = engine();
        let first = engine.plan("writer", "hello", 0).unwrap();
        assert_eq!(first.calls.len(), 2);
        assert!(!first.calls[0].requires_approval);
        assert!(first.calls[1].requires_approval);

        let last = engine.plan("writer", "hello", 1).unwrap();
        assert!(last.text.contains("\"hello\""));
        assert!(last.calls.is_empty());
        assert!(engine.plan("writer", "hello", 2).is_none());
    }

    #[test]
    fn studio_gets_its_own_script() {
        let step = engine().plan(STUDIO_AGENT_ID, "hi", 0).unwrap();
        assert!(step.text.contains("connected to a model"));
        assert!(step.calls.is_empty());
    }

    #[test]
    fn executes_scripted_calls() {
        let engine = engine();
        let call = engine.plan("a", "x", 0).unwrap().calls.remove(0);
        let outcome = engine.execute(&call);
        assert!(outcome.result.contains("report.md"));
        assert!(!outcome.is_error);

        let unknown = PlannedCall {
            id: "c".into(),
            tool: "rm".into(),
            args: json!({}),
            requires_approval: false,
        };
        assert!(engine.execute(&unknown).is_error);
    }

    #[test]
    fn falls_back_to_default_on_invalid_setting() {
        let db = crate::db::Db::in_memory();
        let conn = db.conn().unwrap();
        db::set_setting(&conn, CANNED_SCRIPT_KEY, &json!({"nope": true})).unwrap();
        let engine = CannedEngine::load(&conn).unwrap();
        assert_eq!(engine.script.agent.len(), 2);
    }

    #[test]
    fn chunks_keep_all_text() {
        let text = "Done. For **x**:\nnext";
        assert_eq!(chunk_text(text).concat(), text);
        assert_eq!(chunk_text(text).len(), 4);
    }
}
