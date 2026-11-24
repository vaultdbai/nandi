//! What a paired device and its Nandi say to each other, inside the seal
//! (`SealPurpose::OwnDevice`, see `docs/developer/own-devices.md`). The
//! relay never sees any of it. JSON, camelCase fields:
//!
//! ```json
//! {"id": "c1", "op": "inbox"}
//! {"id": "c2", "op": "approve", "itemId": "<approval ID>", "pin": "482916"}
//! {"id": "c3", "op": "decline", "itemId": "<approval ID>", "pin": "482916"}
//! ```
//!
//! Each call is answered once, with its `id` as `re`: `outcome` is
//! `inbox`, `decided` or `refused` (a [`codes`] code and a sentence).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The codes of a refused call
pub mod codes {
    /// The PIN was wrong; `triesLeft` says how many before the lock
    pub const PIN_WRONG: &str = "pin_wrong";
    /// Too many wrong PINs: only the desktop unlocks it
    pub const DEVICE_LOCKED: &str = "device_locked";
    /// A risky approval: only on the computer
    pub const DESKTOP_ONLY: &str = "desktop_only";
    /// No such item waits (decided, expired, or never there)
    pub const ITEM_NOT_FOUND: &str = "item_not_found";
    /// A call this Nandi doesn't take from a device
    pub const UNKNOWN_COMMAND: &str = "unknown_command";
    /// It was allowed but didn't go through
    pub const FAILED: &str = "failed";
}

/// The most of each list one answer carries (oldest first)
pub const MAX_ITEMS: usize = 50;
/// Longest title, in characters
pub const MAX_TITLE_CHARS: usize = 120;
/// Longest short text, in characters
pub const MAX_DETAIL_CHARS: usize = 280;

/// One call from a device
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Call {
    /// The device's own ID for the call, echoed as the answer's `re`
    pub id: String,
    #[serde(flatten)]
    pub command: Command,
}

/// What a device may ask: nothing else is taken
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Command {
    /// The approvals waiting and the open to-dos
    Inbox,
    /// Approves one approval item, with the PIN
    Approve { item_id: String, pin: String },
    /// Declines one approval item, with the PIN
    Decline { item_id: String, pin: String },
}

/// What came out of the seal: a call, or what to answer instead
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    Call(Call),
    /// Not a call we take; answered `unknown_command` under its `id`
    Unknown {
        id: String,
    },
    /// Not even an `id`: dropped
    Unreadable,
}

/// Reads the opened content of a device's envelope
pub fn read(content: &[u8]) -> Read {
    if let Ok(call) = serde_json::from_slice::<Call>(content) {
        return Read::Call(call);
    }
    match serde_json::from_slice::<Value>(content)
        .ok()
        .and_then(|value| value.get("id")?.as_str().map(str::to_string))
    {
        Some(id) => Read::Unknown { id },
        None => Read::Unreadable,
    }
}

/// One answer, to the call `re`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub re: String,
    #[serde(flatten)]
    pub answer: Answer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Answer {
    Inbox {
        approvals: Vec<PhoneApproval>,
        todos: Vec<PhoneTodo>,
    },
    /// `decision`: `approved` or `declined`
    Decided { item_id: String, decision: String },
    Refused {
        code: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tries_left: Option<u32>,
    },
}

impl Answer {
    pub fn refused(code: &str, message: impl Into<String>) -> Self {
        Self::Refused {
            code: code.to_string(),
            message: message.into(),
            tries_left: None,
        }
    }
}

/// One approval waiting, as a phone shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneApproval {
    pub id: String,
    pub agent: String,
    /// What it asks to do, in a few words
    pub title: String,
    pub detail: String,
    pub created_at: f64,
    pub expires_at: Option<f64>,
    /// Why it can only be approved on the computer (it can still be
    /// declined); `None`: the phone may approve it
    pub desktop_only: Option<String>,
}

/// One open to-do, as a phone shows it (read only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneTodo {
    pub id: String,
    pub title: String,
    /// `do`, `check` or `remember`
    pub kind: String,
    pub from: String,
    pub due_at: Option<f64>,
}

/// `text` on one line, at most `max` characters
pub fn short(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        let cut: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_three_calls_read_and_anything_else_is_unknown_or_dropped() {
        let read_json = |value: Value| read(value.to_string().as_bytes());
        assert_eq!(
            read_json(json!({"id": "c1", "op": "inbox"})),
            Read::Call(Call {
                id: "c1".into(),
                command: Command::Inbox
            })
        );
        assert_eq!(
            read_json(json!({"id": "c2", "op": "approve", "itemId": "a1", "pin": "482916"})),
            Read::Call(Call {
                id: "c2".into(),
                command: Command::Approve {
                    item_id: "a1".into(),
                    pin: "482916".into()
                }
            })
        );
        // A device can't pair, revoke, change settings or skip the PIN
        for unknown in [
            json!({"id": "c3", "op": "pair"}),
            json!({"id": "c4", "op": "revoke_all"}),
            json!({"id": "c5", "op": "approve", "itemId": "a1"}),
        ] {
            assert!(matches!(read_json(unknown), Read::Unknown { .. }));
        }
        assert_eq!(read(b"not json"), Read::Unreadable);
        assert_eq!(read_json(json!({"op": "inbox"})), Read::Unreadable);
    }

    #[test]
    fn answers_carry_the_call_id_and_a_tagged_outcome() {
        let reply = Reply {
            re: "c2".into(),
            answer: Answer::Refused {
                code: codes::PIN_WRONG.into(),
                message: "Wrong PIN".into(),
                tries_left: Some(3),
            },
        };
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            json!({"re": "c2", "outcome": "refused", "code": "pin_wrong",
                   "message": "Wrong PIN", "triesLeft": 3})
        );
        let decided = Reply {
            re: "c3".into(),
            answer: Answer::Decided {
                item_id: "a1".into(),
                decision: "approved".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&decided).unwrap(),
            json!({"re": "c3", "outcome": "decided", "itemId": "a1", "decision": "approved"})
        );
    }

    #[test]
    fn short_text_is_one_line_and_cut() {
        assert_eq!(short("a\n  b", 10), "a b");
        assert_eq!(short("abcdefghij", 5), "abcd…");
    }
}
