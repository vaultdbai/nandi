//! What a paired device and its Nandi say to each other, inside the seal
//! (`SealPurpose::OwnDevice`, see `docs/developer/own-devices.md`). The
//! relay never sees any of it. JSON, camelCase fields:
//!
//! ```json
//! {"id": "c1", "op": "inbox"}
//! {"id": "c2", "op": "approve", "itemId": "<approval ID>", "pin": "482916"}
//! {"id": "c3", "op": "decline", "itemId": "<approval ID>", "pin": "482916"}
//! {"id": "c4", "op": "ask", "text": "What's on today?", "chat": "<chat ID>", "pin": "482916"}
//! {"id": "c5", "op": "chat", "chat": "<chat ID>", "after": "<message ID>"}
//! ```
//!
//! `ask` without `chat` starts a new chat with Nandi; its `pin` is needed
//! only when the device's session is closed ([`super::session`]). `chat`
//! reads a chat's messages after `after` (all of them without it): the
//! phone polls it while `working`.
//!
//! Each call is answered once, with its `id` as `re`: `outcome` is
//! `inbox`, `decided`, `asked`, `chat` or `refused` (a [`codes`] code and
//! a sentence).

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
    /// The session lapsed (or never opened): ask again with the PIN
    pub const PIN_NEEDED: &str = "pin_needed";
    /// No chat of this device's has that ID
    pub const CHAT_NOT_FOUND: &str = "chat_not_found";
    /// The ask is empty or longer than [`super::MAX_ASK_CHARS`]
    pub const ASK_INVALID: &str = "ask_invalid";
    /// Asking from this phone is off (Phone Controls): it only approves
    pub const ASKS_OFF: &str = "asks_off";
}

/// The most of each list one answer carries (oldest first)
pub const MAX_ITEMS: usize = 50;
/// Longest title, in characters
pub const MAX_TITLE_CHARS: usize = 120;
/// Longest short text, in characters
pub const MAX_DETAIL_CHARS: usize = 280;
/// Longest ask, in characters
pub const MAX_ASK_CHARS: usize = 4000;
/// The most messages one `chat` answer carries (the latest)
pub const MAX_CHAT_MESSAGES: usize = 20;
/// Longest message text in a `chat` answer, in characters
pub const MAX_MESSAGE_CHARS: usize = 4000;

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
    /// Asks Nandi as the person: in this device's chat `chat`, or a new
    /// one. `pin` opens the session; within it, none is needed.
    Ask {
        text: String,
        chat: Option<String>,
        pin: Option<String>,
    },
    /// One of this device's chats: its messages after message `after`
    Chat { chat: String, after: Option<String> },
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
    /// The ask is in: the chat it went to, and the person's message
    Asked {
        chat: String,
        message_id: String,
        /// Nandi was busy: the ask waits its turn
        queued: bool,
    },
    /// A chat's messages after the point asked for, oldest first
    Chat {
        chat: String,
        messages: Vec<PhoneMessage>,
        /// A run of this chat is queued, running or waiting for an approval
        working: bool,
        /// That run waits for an approval (in the Inbox)
        waiting_for_approval: bool,
    },
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

/// One message of a chat, as a phone shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneMessage {
    pub id: String,
    /// `user` (the person), `assistant` (Nandi) or `error` (the run failed)
    pub role: String,
    pub text: String,
    pub created_at: f64,
}

/// `text` at most `max` characters, its lines kept
pub fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() > max {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
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
    fn ask_and_chat_read_with_their_optional_parts() {
        let read_json = |value: Value| read(value.to_string().as_bytes());
        assert_eq!(
            read_json(json!({"id": "c1", "op": "ask", "text": "What's on today?"})),
            Read::Call(Call {
                id: "c1".into(),
                command: Command::Ask {
                    text: "What's on today?".into(),
                    chat: None,
                    pin: None
                }
            })
        );
        assert_eq!(
            read_json(json!({"id": "c2", "op": "ask", "text": "And then?",
                             "chat": "phone:x", "pin": "482916"})),
            Read::Call(Call {
                id: "c2".into(),
                command: Command::Ask {
                    text: "And then?".into(),
                    chat: Some("phone:x".into()),
                    pin: Some("482916".into())
                }
            })
        );
        assert_eq!(
            read_json(json!({"id": "c3", "op": "chat", "chat": "phone:x", "after": "m1"})),
            Read::Call(Call {
                id: "c3".into(),
                command: Command::Chat {
                    chat: "phone:x".into(),
                    after: Some("m1".into())
                }
            })
        );
        // An ask with no text, or a chat read with no chat, isn't a call
        for unknown in [
            json!({"id": "c4", "op": "ask"}),
            json!({"id": "c5", "op": "chat"}),
        ] {
            assert!(matches!(read_json(unknown), Read::Unknown { .. }));
        }
    }

    #[test]
    fn a_chat_answer_is_tagged_and_camel_case() {
        let reply = Reply {
            re: "c3".into(),
            answer: Answer::Chat {
                chat: "phone:x".into(),
                messages: vec![PhoneMessage {
                    id: "m2".into(),
                    role: "assistant".into(),
                    text: "Two meetings.".into(),
                    created_at: 2.0,
                }],
                working: false,
                waiting_for_approval: false,
            },
        };
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            json!({"re": "c3", "outcome": "chat", "chat": "phone:x", "working": false,
                   "waitingForApproval": false,
                   "messages": [{"id": "m2", "role": "assistant", "text": "Two meetings.",
                                 "createdAt": 2.0}]})
        );
        let asked = Reply {
            re: "c4".into(),
            answer: Answer::Asked {
                chat: "phone:x".into(),
                message_id: "m1".into(),
                queued: true,
            },
        };
        assert_eq!(
            serde_json::to_value(&asked).unwrap(),
            json!({"re": "c4", "outcome": "asked", "chat": "phone:x", "messageId": "m1",
                   "queued": true})
        );
    }

    #[test]
    fn short_text_is_one_line_and_cut() {
        assert_eq!(short("a\n  b", 10), "a b");
        assert_eq!(short("abcdefghij", 5), "abcd…");
        assert_eq!(cut(" a\nb ", 10), "a\nb");
        assert_eq!(cut("abcdefghij", 5), "abcd…");
    }
}
