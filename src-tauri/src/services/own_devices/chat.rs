//! Asking Nandi from a paired device: the text is the person's own
//! message (their device, behind their PIN), in a chat with the Nandi
//! agent that only this device reads from the phone. A run it starts is
//! like one started here: every tool call still goes through the same
//! approvals, and a phone may approve only what it always could.
//!
//! - A device's chats are `phone:<device ID>:<uuid>` conversations of the
//!   Nandi agent, among the person's chats on the computer (the Nandi
//!   home's Recent, "From your phone"), named by their first message; the
//!   run's record says "Asked from <device name>" (`RunSource::own_device`).
//!   What the run may do is the device's Phone Controls' (`controls`).
//! - Reading one gives its latest messages (the person's, Nandi's and a
//!   failed run's error) and whether a run is still going.

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::wire::{self, PhoneMessage};
use crate::db::{self, role};
use crate::model::RunStatus;
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::orchestrator::RunSource;
use crate::services::AppState;

/// How many of a chat's latest rows a read looks at
const READ_ROWS: u32 = 200;

/// A new chat is named by its first message, cut like the computer's
const TITLE_CHARS: usize = 60;

/// The device that asks
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asker {
    pub device_id: String,
    pub name: String,
}

/// An ask that went in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub chat: String,
    pub message_id: String,
    /// The run that answers it (empty when the ask stopped one instead)
    pub run_id: String,
    pub queued: bool,
}

/// One chat, as a phone reads it
#[derive(Debug, Clone, PartialEq)]
pub struct ChatView {
    pub messages: Vec<PhoneMessage>,
    pub working: bool,
    pub waiting_for_approval: bool,
}

/// The ID prefix of `device_id`'s chats
fn prefix(device_id: &str) -> String {
    format!("phone:{device_id}:")
}

/// A new chat ID for `device_id`
pub fn new_chat_id(device_id: &str) -> String {
    format!("{}{}", prefix(device_id), uuid::Uuid::new_v4())
}

/// Whether `chat` is one of `device_id`'s chats. A device reaches only
/// its own: never the person's other chats, nor another device's.
pub fn owns(device_id: &str, chat: &str) -> bool {
    chat.strip_prefix(&prefix(device_id))
        .is_some_and(|rest| uuid::Uuid::parse_str(rest).is_ok())
}

/// The chats with Nandi, as a device's commands use them. Callers check
/// [`owns`] first.
pub trait PhoneChat: Send + Sync {
    /// Asks Nandi `text` as the person, in `chat` (`None` when that chat
    /// isn't there)
    fn ask(
        &self,
        asker: Asker,
        chat: String,
        new: bool,
        text: String,
    ) -> BoxFuture<'_, Result<Option<Asked>, String>>;

    /// `chat`'s messages after message `after`, oldest first (`None` when
    /// that chat isn't there)
    fn read(
        &self,
        chat: String,
        after: Option<String>,
    ) -> BoxFuture<'_, Result<Option<ChatView>, String>>;
}

/// The messages after `after` (all when it isn't among them), the latest
/// [`wire::MAX_CHAT_MESSAGES`]. Pure.
pub fn since(rows: &[db::MessageRow], after: Option<&str>) -> Vec<PhoneMessage> {
    let shown: Vec<&db::MessageRow> = rows
        .iter()
        .filter(|row| {
            matches!(
                row.role.as_str(),
                role::USER | role::ASSISTANT | role::ERROR
            ) && !row.is_continuation()
                && !row.content.trim().is_empty()
        })
        .collect();
    let start = after
        .and_then(|id| shown.iter().position(|row| row.id == id))
        .map_or(0, |at| at + 1);
    let newer = &shown[start..];
    newer[newer.len().saturating_sub(wire::MAX_CHAT_MESSAGES)..]
        .iter()
        .map(|row| PhoneMessage {
            id: row.id.clone(),
            role: row.role.clone(),
            text: wire::cut(&row.content, wire::MAX_MESSAGE_CHARS),
            created_at: row.created_at as f64,
        })
        .collect()
}

/// The app's own chats with Nandi
pub struct AppChat {
    state: AppState,
}

impl AppChat {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Whether `chat` is a conversation of the Nandi agent's
    async fn is_nandis(&self, chat: &str) -> Result<bool, String> {
        let id = chat.to_string();
        let owner = self
            .state
            .db()
            .call(move |conn| db::conversation_agent(conn, &id))
            .await?;
        Ok(owner.as_deref() == Some(DEFAULT_AGENT))
    }
}

impl PhoneChat for AppChat {
    fn ask(
        &self,
        asker: Asker,
        chat: String,
        new: bool,
        text: String,
    ) -> BoxFuture<'_, Result<Option<Asked>, String>> {
        async move {
            self.state.store().require_agent(DEFAULT_AGENT)?;
            if new {
                let title = crate::services::events::truncate(
                    text.trim().lines().next().unwrap_or_default(),
                    TITLE_CHARS,
                );
                let id = chat.clone();
                self.state
                    .db()
                    .call(move |conn| {
                        conn.execute(
                            "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title)
                             VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![id, DEFAULT_AGENT, db::now_ms(), title],
                        )?;
                        Ok(())
                    })
                    .await?;
            } else if !self.is_nandis(&chat).await? {
                return Ok(None);
            } else if crate::services::chat::stop_instead(&self.state, &chat, &text)
                .await?
                .is_some()
            {
                // "stop" from the phone stops the chat's run, like the button
                return Ok(Some(Asked {
                    chat,
                    message_id: String::new(),
                    run_id: String::new(),
                    queued: false,
                }));
            }
            let sent = crate::services::chat::submit(
                self.state.orchestrators(),
                DEFAULT_AGENT,
                chat.clone(),
                text,
                Vec::new(),
                None,
                RunSource::own_device(&asker.device_id, &asker.name),
            )
            .await?;
            Ok(Some(Asked {
                chat,
                message_id: sent.message_id,
                run_id: sent.run_id,
                queued: sent.queued,
            }))
        }
        .boxed()
    }

    fn read(
        &self,
        chat: String,
        after: Option<String>,
    ) -> BoxFuture<'_, Result<Option<ChatView>, String>> {
        async move {
            if !self.is_nandis(&chat).await? {
                return Ok(None);
            }
            let (rows, runs) = self
                .state
                .db()
                .call(move |conn| {
                    Ok((
                        db::list_messages(conn, &chat, READ_ROWS)?,
                        db::unfinished_runs(conn, &chat)?,
                    ))
                })
                .await?;
            Ok(Some(ChatView {
                messages: since(&rows, after.as_deref()),
                working: !runs.is_empty(),
                waiting_for_approval: runs
                    .iter()
                    .any(|run| run.status == RunStatus::AwaitingApproval),
            }))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const DEVICE: &str = "device.0123456789abcdef0123456789abcdef";

    #[test]
    fn a_device_reaches_only_its_own_chats() {
        let chat = new_chat_id(DEVICE);
        assert!(owns(DEVICE, &chat));
        assert!(!owns("device.ffffffffffffffffffffffffffffffff", &chat));
        for other in [
            "agent:x",
            "inbox:approval:a1",
            &format!("phone:{DEVICE}:"),
            &format!("phone:{DEVICE}:not-a-uuid"),
            &format!("phone:{DEVICE}"),
        ] {
            assert!(!owns(DEVICE, other), "{other}");
        }
    }

    fn row(id: &str, role: &str, text: &str, meta: serde_json::Value) -> db::MessageRow {
        db::MessageRow::new(id, "phone:x", None, role, text, meta)
    }

    #[test]
    fn a_read_gives_what_came_after_and_leaves_out_what_the_person_never_wrote() {
        let rows = vec![
            row("m1", role::USER, "What's on today?", json!({})),
            row("t1", role::TOOL_CALL, "calendar", json!({})),
            row("m2", role::ASSISTANT, "Two meetings.", json!({})),
            row(
                "m3",
                role::USER,
                "Check again",
                json!({"continuation": true}),
            ),
            row("m4", role::ASSISTANT, "  ", json!({})),
            row("m5", role::ERROR, "The model is unreachable", json!({})),
        ];
        let ids = |messages: Vec<PhoneMessage>| -> Vec<String> {
            messages.into_iter().map(|m| m.id).collect()
        };
        assert_eq!(ids(since(&rows, None)), ["m1", "m2", "m5"]);
        assert_eq!(ids(since(&rows, Some("m1"))), ["m2", "m5"]);
        assert_eq!(ids(since(&rows, Some("m5"))), Vec::<String>::new());
        // A point that isn't there (or isn't shown) gives everything
        assert_eq!(ids(since(&rows, Some("gone"))), ["m1", "m2", "m5"]);
    }

    #[test]
    fn a_read_carries_at_most_the_latest_messages_each_cut() {
        let rows: Vec<db::MessageRow> = (0..wire::MAX_CHAT_MESSAGES + 5)
            .map(|i| {
                row(
                    &format!("m{i}"),
                    role::ASSISTANT,
                    &"x".repeat(wire::MAX_MESSAGE_CHARS + 10),
                    json!({}),
                )
            })
            .collect();
        let read = since(&rows, None);
        assert_eq!(read.len(), wire::MAX_CHAT_MESSAGES);
        assert_eq!(read[0].id, "m5");
        assert_eq!(read[0].text.chars().count(), wire::MAX_MESSAGE_CHARS);
    }
}
