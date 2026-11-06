//! Hand-over: the agent prepares a page in the visible browser (a post
//! typed into a composer) and a person finishes it there.
//!
//! The agent calls `browser_hand_over` (a tool Nandi adds to the `browser`
//! pilot) once the page is ready. That call always waits for the person:
//!
//! 1. Asking: an Inbox item "Your turn: review and post" with the draft and
//!    the site, and Nandi brings the browser window to the front
//!    ([`bring_forward`]). It waits up to [`TTL_MS`] (a day), not an hour
//!    like other approvals.
//! 2. The person edits what they like in the browser and presses Post
//!    themselves, then answers in Nandi: **Done**, **Skip** or **Changed
//!    it** (with the final text). Done and Changed it approve the call;
//!    Skip rejects it.
//! 3. The call's result tells the agent what happened ([`result`]), and it's
//!    on the run like every tool result. The agent then notes it.
//!
//! The final text travels from the answer to the call's result in memory
//! ([`record`], [`take`]): the call runs a moment after the approval.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::browser::BROWSER;
use super::tools::tool_name;
use crate::llm::ToolSpec;

/// The tool's name as Playwright MCP's tools are named
pub const TOOL: &str = "browser_hand_over";
/// How long a hand-over waits for the person
pub const TTL_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_TEXT_CHARS: usize = 20_000;

/// What the person did
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum HandOverOutcome {
    /// They published it as prepared
    Done,
    /// They didn't publish it
    Skipped,
    /// They changed it, then published it
    Changed,
}

/// The person's answer to a hand-over
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HandOverAnswer {
    pub outcome: HandOverOutcome,
    /// What went out, for `changed`
    #[serde(default)]
    pub final_text: Option<String>,
}

/// Whether `name` (as the model calls it) is the hand-over
pub fn is_hand_over(name: &str) -> bool {
    name == tool_name(BROWSER, TOOL)
}

/// The tool the model sees, next to the browser's own
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: TOOL.to_string(),
        description: "Hand the page over to the person once it's ready: they review what you prepared in this \
            browser window, change anything, and press the final button (Post, Publish, Send) themselves. Call it \
            after you filled in a composer or form you may not submit. It waits for them. The result says what \
            they did: posted it as prepared, posted it changed (with the final text), or skipped it. If the call \
            is rejected, they skipped it: nothing went out."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "site": {
                    "type": "string",
                    "description": "Where the page is, e.g. LinkedIn or x.com"
                },
                "summary": {
                    "type": "string",
                    "description": "One line on what's ready, e.g. 'Launch post for the Acme campaign'"
                },
                "draft": {
                    "type": "string",
                    "description": "The text you typed, exactly as typed"
                }
            },
            "required": ["site", "summary"],
            "additionalProperties": false
        }),
    }
}

static ANSWERS: LazyLock<Mutex<HashMap<String, HandOverAnswer>>> = LazyLock::new(Default::default);

/// Keeps the person's answer for the call `tool_call_id` until it runs
pub fn record(tool_call_id: &str, answer: HandOverAnswer) {
    if let Ok(mut answers) = ANSWERS.lock() {
        answers.insert(tool_call_id.to_string(), answer);
    }
}

/// The answer for `tool_call_id`, once
pub fn take(tool_call_id: &str) -> Option<HandOverAnswer> {
    ANSWERS.lock().ok()?.remove(tool_call_id)
}

/// The answer, checked: a `changed` answer has its text
pub fn validate(answer: HandOverAnswer) -> Result<HandOverAnswer, String> {
    let final_text = answer
        .final_text
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    match (answer.outcome, &final_text) {
        (HandOverOutcome::Changed, None) => {
            Err("Paste the text that went out, or choose Done".to_string())
        }
        (_, Some(text)) if text.chars().count() > MAX_TEXT_CHARS => {
            Err(format!("The text is over {MAX_TEXT_CHARS} characters"))
        }
        (HandOverOutcome::Changed, _) => Ok(HandOverAnswer {
            outcome: answer.outcome,
            final_text,
        }),
        _ => Ok(HandOverAnswer {
            outcome: answer.outcome,
            final_text: None,
        }),
    }
}

/// What the agent reads once the person answered. An approval without an
/// answer (approved from a plain approval card) is Done.
pub fn result(answer: Option<HandOverAnswer>, args: &Value) -> String {
    let site = args["site"].as_str().unwrap_or("the site");
    match answer {
        Some(HandOverAnswer {
            outcome: HandOverOutcome::Changed,
            final_text: Some(text),
        }) => format!(
            "The person changed it and published it on {site} themselves. What went out:\n\n{text}"
        ),
        Some(HandOverAnswer {
            outcome: HandOverOutcome::Skipped,
            ..
        }) => format!("The person skipped it: nothing was published on {site}."),
        _ => format!("The person published it on {site} themselves, as you prepared it."),
    }
}

/// Brings `agent`'s visible browser to the front, so the person finds the
/// page ready. Nothing when its browser isn't open.
pub async fn bring_forward(agent: &str) {
    if let Err(e) = super::browser::bring_to_front(agent, BROWSER).await {
        log::info!("Couldn't bring the browser forward for '{agent}': {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(outcome: HandOverOutcome, text: Option<&str>) -> HandOverAnswer {
        HandOverAnswer {
            outcome,
            final_text: text.map(str::to_string),
        }
    }

    #[test]
    fn the_tool_is_the_visible_browser_s() {
        assert!(is_hand_over("pilot_browser__browser_hand_over"));
        assert!(!is_hand_over("pilot_browser-headless__browser_hand_over"));
        assert!(!is_hand_over("pilot_browser__browser_click"));
        assert_eq!(spec().name, TOOL);
    }

    #[test]
    fn answers_are_checked() {
        assert!(validate(answer(HandOverOutcome::Changed, Some("  "))).is_err());
        assert_eq!(
            validate(answer(HandOverOutcome::Changed, Some(" Final "))).unwrap(),
            answer(HandOverOutcome::Changed, Some("Final"))
        );
        assert_eq!(
            validate(answer(HandOverOutcome::Done, Some("ignored"))).unwrap(),
            answer(HandOverOutcome::Done, None)
        );
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(validate(answer(HandOverOutcome::Changed, Some(&long))).is_err());
    }

    #[test]
    fn the_agent_reads_what_the_person_did() {
        let args = json!({ "site": "LinkedIn", "summary": "Launch post" });
        let done = result(Some(answer(HandOverOutcome::Done, None)), &args);
        assert!(done.contains("published it on LinkedIn"), "{done}");
        let skipped = result(Some(answer(HandOverOutcome::Skipped, None)), &args);
        assert!(skipped.contains("nothing was published"), "{skipped}");
        let changed = result(
            Some(answer(HandOverOutcome::Changed, Some("Final words"))),
            &args,
        );
        assert!(changed.ends_with("Final words"), "{changed}");
        assert!(result(None, &args).contains("as you prepared it"));
    }

    #[test]
    fn an_answer_is_kept_for_its_call_once() {
        record("call-1", answer(HandOverOutcome::Skipped, None));
        assert_eq!(take("call-1"), Some(answer(HandOverOutcome::Skipped, None)));
        assert_eq!(take("call-1"), None);
    }
}
