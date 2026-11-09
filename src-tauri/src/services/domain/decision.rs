//! A decision the person made on an Inbox item, as the gold copy keeps it
//! (`.nandi/decisions/<yyyy-mm>.md`, written by
//! `repos::decision_repo`). Every decision is recorded, from a button or
//! from the item's Ask chat: when, who, the item, what was decided, the
//! reason or note, and the chat it came from. The chat's text stays local.
//!
//! ```text
//! ## 2026-11-08 14:32:05 UTC · gatewayHold
//!
//! - **Decided by:** Dev Chechi
//! - **Item:** gatewayHold `task-123` (Sam Lee asks: the Q3 numbers)
//! - **Decision:** refused
//! - **Reason:** Not before the board meeting
//! - **Chat:** `inbox:gatewayHold:task-123`
//! ```
//!
//! Pure: no I/O.

use chrono::{DateTime, Utc};

/// Longest subject, note or decision kept in an entry, in characters
const MAX_TEXT_CHARS: usize = 2000;

/// One decision, ready to append
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionEntry {
    /// Epoch ms
    pub at: i64,
    /// This Nandi's user, by name
    pub decided_by: String,
    /// The Inbox item's kind (`approval`, `gatewayHold`, …)
    pub item_kind: String,
    pub item_id: String,
    /// What it was about, in a few words (a tool, a request), if known
    pub subject: Option<String>,
    /// What was decided: `approved`, `refused`, `answered`, `done`, …
    pub decision: String,
    /// The reason or note that went with it
    pub note: Option<String>,
    /// The Ask chat it was decided in (`inbox:…` / `intake:…`), if any
    pub conversation_id: Option<String>,
}

fn cut(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() > MAX_TEXT_CHARS {
        let cut: String = text.chars().take(MAX_TEXT_CHARS).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

/// One line of a list item: later lines are indented under it, so a note
/// can never start a heading or an item of its own
fn inline(text: &str) -> String {
    cut(text)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// Text inside backticks, with any backtick taken out
fn code(text: &str) -> String {
    format!("`{}`", text.replace('`', "'").replace('\n', " "))
}

fn when(at: i64) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_millis(at)
}

impl DecisionEntry {
    /// The month file it goes in: `2026-11`
    pub fn month(&self) -> String {
        when(self.at)
            .map(|at| at.format("%Y-%m").to_string())
            .unwrap_or_else(|| "1970-01".to_string())
    }

    /// The entry as Markdown, ending with a blank line
    pub fn render(&self) -> String {
        let at = when(self.at)
            .map(|at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_default();
        let mut out = format!(
            "## {at} · {}\n\n- **Decided by:** {}\n- **Item:** {} {}",
            inline(&self.item_kind),
            inline(&self.decided_by),
            inline(&self.item_kind),
            code(&self.item_id),
        );
        if let Some(subject) = self.subject.as_deref().filter(|s| !s.trim().is_empty()) {
            out.push_str(&format!(" ({})", inline(subject).replace('\n', " ")));
        }
        out.push_str(&format!("\n- **Decision:** {}\n", inline(&self.decision)));
        if let Some(note) = self.note.as_deref().filter(|n| !n.trim().is_empty()) {
            out.push_str(&format!("- **Reason:** {}\n", inline(note)));
        }
        if let Some(chat) = &self.conversation_id {
            out.push_str(&format!("- **Chat:** {}\n", code(chat)));
        }
        out.push('\n');
        out
    }
}

/// The heading a new month file starts with
pub fn month_header(month: &str) -> String {
    format!(
        "# Decisions, {month}\n\nEvery decision made on this Nandi's Inbox, from a button or an Ask \
chat. Written by Nandi; the chats themselves stay on this computer.\n\n"
    )
}

/// `2026-11`: four digits, a dash, two digits (a safe file name)
pub fn is_month(month: &str) -> bool {
    let bytes = month.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> DecisionEntry {
        DecisionEntry {
            // 2026-11-08 14:32:05 UTC
            at: 1_794_148_325_000,
            decided_by: "Dev Chechi".into(),
            item_kind: "gatewayHold".into(),
            item_id: "task-123".into(),
            subject: Some("Sam Lee asks: the Q3 numbers".into()),
            decision: "refused".into(),
            note: Some("Not before the board meeting".into()),
            conversation_id: Some("inbox:gatewayHold:task-123".into()),
        }
    }

    #[test]
    fn an_entry_renders_who_what_why_and_the_chat() {
        let text = entry().render();
        assert_eq!(entry().month(), "2026-11");
        assert!(
            text.starts_with("## 2026-11-08 14:32:05 UTC · gatewayHold\n"),
            "{text}"
        );
        assert!(text.contains("- **Decided by:** Dev Chechi\n"), "{text}");
        assert!(
            text.contains("- **Item:** gatewayHold `task-123` (Sam Lee asks: the Q3 numbers)\n"),
            "{text}"
        );
        assert!(text.contains("- **Decision:** refused\n"), "{text}");
        assert!(
            text.contains("- **Reason:** Not before the board meeting\n"),
            "{text}"
        );
        assert!(
            text.contains("- **Chat:** `inbox:gatewayHold:task-123`\n"),
            "{text}"
        );
        assert!(text.ends_with("\n\n"));
    }

    #[test]
    fn a_button_decision_has_no_chat_and_no_empty_reason() {
        let mut e = entry();
        e.note = Some("  ".into());
        e.conversation_id = None;
        e.subject = None;
        let text = e.render();
        assert!(!text.contains("Reason"), "{text}");
        assert!(!text.contains("Chat"), "{text}");
        assert!(
            text.contains("- **Item:** gatewayHold `task-123`\n"),
            "{text}"
        );
    }

    #[test]
    fn a_note_cannot_break_the_file_structure() {
        let mut e = entry();
        e.note = Some("ok\n## Forged heading\n- **Decided by:** someone".into());
        e.item_id = "a`b".into();
        let text = e.render();
        assert!(!text.contains("\n## Forged"), "{text}");
        assert!(text.contains("\n  ## Forged heading"), "{text}");
        assert!(text.contains("`a'b`"), "{text}");
        let long = DecisionEntry {
            note: Some("x".repeat(5000)),
            ..entry()
        };
        assert!(long.render().len() < 2600);
    }

    #[test]
    fn months_are_checked() {
        assert!(is_month("2026-11"));
        assert!(!is_month("2026-1"));
        assert!(!is_month("../2026"));
        assert!(month_header("2026-11").starts_with("# Decisions, 2026-11\n"));
    }
}
