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
//! The files are read back by [`parse_month`], which skips (and reports)
//! anything a hand edit left unreadable. Pure: no I/O.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;

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

/// One decision as the Decisions page shows it, read back from a month file
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRecord {
    /// Epoch ms (a JavaScript number)
    #[specta(type = f64)]
    pub at: i64,
    /// The Inbox item's kind (`approval`, `gatewayHold`, ...)
    pub kind: String,
    pub item_id: String,
    /// What the item was about; the item's id when the file names nothing
    pub item_label: String,
    /// `approved`, `refused`, `answered`, `done`, ...
    pub decision: String,
    pub decided_by: String,
    pub reason: Option<String>,
    /// The Ask chat it was decided in (`inbox:...` / `intake:...`), if any
    pub chat_id: Option<String>,
    /// That chat is still on this computer. Not in the file: the service
    /// fills it in; reading the file leaves it `false`.
    pub chat_exists: bool,
}

/// What [`parse_month`] found in one month file
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedMonth {
    /// In file order (oldest first, as written)
    pub entries: Vec<DecisionRecord>,
    /// The headings of the entries that couldn't be read
    pub skipped: Vec<String>,
}

/// Which decisions to show: every set field must match
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DecisionFilter {
    /// The item's kind
    pub kind: Option<String>,
    pub decision: Option<String>,
    /// Text in the item or the reason, any case
    pub search: Option<String>,
}

impl DecisionFilter {
    pub fn matches(&self, record: &DecisionRecord) -> bool {
        let is = |want: &Option<String>, have: &str| {
            want.as_deref()
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .is_none_or(|w| w.eq_ignore_ascii_case(have))
        };
        if !is(&self.kind, &record.kind) {
            return false;
        }
        // `approved` also matches `approved (once)`, `reassigned` matches
        // `reassigned to Sam`: a decision's word, then its detail
        if let Some(want) = self
            .decision
            .as_deref()
            .map(|d| d.trim().to_lowercase())
            .filter(|d| !d.is_empty())
        {
            let have = record.decision.to_lowercase();
            let detail = have
                .strip_prefix(&want)
                .is_some_and(|rest| rest.starts_with([' ', '(']));
            if have != want && !detail {
                return false;
            }
        }
        let Some(needle) = self
            .search
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
        else {
            return true;
        };
        [
            Some(record.item_label.as_str()),
            Some(record.item_id.as_str()),
            record.reason.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|text| text.to_lowercase().contains(&needle))
    }
}

/// The records that pass `filter`, newest first
pub fn select(entries: &[DecisionRecord], filter: &DecisionFilter) -> Vec<DecisionRecord> {
    let mut out: Vec<DecisionRecord> = entries
        .iter()
        .rev()
        .filter(|r| filter.matches(r))
        .cloned()
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.at));
    out
}

/// A list line: `- **Label:** value`
fn field(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("- **")?;
    let (label, value) = rest.split_once(":**")?;
    Some((label.trim(), value.trim()))
}

/// `2026-11-08 14:32:05 UTC · gatewayHold` -> (epoch ms, kind)
fn heading(text: &str) -> Option<(i64, String)> {
    let (at, kind) = text.split_once('\u{b7}')?;
    let at = at.trim().strip_suffix("UTC")?.trim();
    let at = NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S").ok()?;
    Some((at.and_utc().timestamp_millis(), kind.trim().to_string()))
}

/// ``gatewayHold `task-123` (Sam asks)`` -> (kind, id, subject)
fn item(text: &str) -> Option<(String, String, Option<String>)> {
    let (kind, rest) = text.split_once('`')?;
    let (id, rest) = rest.split_once('`')?;
    if id.trim().is_empty() {
        return None;
    }
    let rest = rest.trim();
    let subject = rest
        .strip_prefix('(')
        .and_then(|r| r.strip_suffix(')'))
        .unwrap_or(rest)
        .trim();
    Some((
        kind.trim().to_string(),
        id.trim().to_string(),
        (!subject.is_empty()).then(|| subject.to_string()),
    ))
}

fn parse_entry(head: &str, body: &[&str]) -> Option<DecisionRecord> {
    let (at, heading_kind) = heading(head)?;
    // `- **Label:** value`, later lines indented under it
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in body {
        if let Some((label, value)) = field(line) {
            fields.push((label.to_string(), value.to_string()));
        } else if let Some(more) = line.strip_prefix("  ") {
            if let Some((_, value)) = fields.last_mut() {
                value.push('\n');
                value.push_str(more.trim_end());
            }
        }
    }
    let get = |label: &str| {
        fields
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case(label))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let (kind, item_id, subject) = item(&get("Item")?)?;
    Some(DecisionRecord {
        at,
        kind: if kind.is_empty() { heading_kind } else { kind },
        item_label: subject.unwrap_or_else(|| item_id.clone()),
        item_id,
        decision: get("Decision")?,
        decided_by: get("Decided by").unwrap_or_default(),
        reason: get("Reason"),
        chat_id: get("Chat").map(|c| c.trim_matches('`').trim().to_string()),
        chat_exists: false,
    })
}

/// Every entry in one month file's text. An entry that can't be read
/// (a hand edit took out its time, item or decision) is skipped and its
/// heading reported; the rest still come back.
pub fn parse_month(text: &str) -> ParsedMonth {
    fn finish(entry: Option<(&str, Vec<&str>)>, parsed: &mut ParsedMonth) {
        if let Some((head, body)) = entry {
            match parse_entry(head, &body) {
                Some(record) => parsed.entries.push(record),
                None => parsed.skipped.push(head.to_string()),
            }
        }
    }
    let mut parsed = ParsedMonth::default();
    let mut current: Option<(&str, Vec<&str>)> = None;
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            finish(current.take(), &mut parsed);
            current = Some((head.trim(), Vec::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }
    finish(current.take(), &mut parsed);
    parsed
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

    fn record_of(e: &DecisionEntry) -> DecisionRecord {
        let parsed = parse_month(&e.render());
        assert!(parsed.skipped.is_empty(), "{:?}", parsed.skipped);
        assert_eq!(parsed.entries.len(), 1);
        parsed.entries[0].clone()
    }

    #[test]
    fn what_was_written_reads_back() {
        let r = record_of(&entry());
        assert_eq!(r.at, 1_794_148_325_000);
        assert_eq!(r.kind, "gatewayHold");
        assert_eq!(r.item_id, "task-123");
        assert_eq!(r.item_label, "Sam Lee asks: the Q3 numbers");
        assert_eq!(r.decision, "refused");
        assert_eq!(r.decided_by, "Dev Chechi");
        assert_eq!(r.reason.as_deref(), Some("Not before the board meeting"));
        assert_eq!(r.chat_id.as_deref(), Some("inbox:gatewayHold:task-123"));
    }

    #[test]
    fn a_button_decision_reads_back_without_label_reason_or_chat() {
        let mut e = entry();
        e.subject = None;
        e.note = None;
        e.conversation_id = None;
        let r = record_of(&e);
        assert_eq!(r.item_label, "task-123");
        assert_eq!(r.reason, None);
        assert_eq!(r.chat_id, None);
    }

    #[test]
    fn multi_line_notes_and_parentheses_round_trip() {
        let mut e = entry();
        e.subject = Some("Run the report (Q3) now".into());
        e.note = Some("ok\n\n## Forged heading\n- **Decided by:** someone".into());
        let r = record_of(&e);
        assert_eq!(r.item_label, "Run the report (Q3) now");
        assert_eq!(
            r.reason.as_deref(),
            Some("ok\n\n## Forged heading\n- **Decided by:** someone")
        );
        assert_eq!(r.decided_by, "Dev Chechi");
    }

    #[test]
    fn a_whole_file_reads_back_in_order_and_hand_edits_are_skipped() {
        let mut text = month_header("2026-11");
        let mut a = entry();
        a.decision = "approved".into();
        text.push_str(&a.render());
        // Hand edits: no time, no item, no decision
        text.push_str(
            "## yesterday \u{b7} approval\n\n- **Item:** approval `x`\n- **Decision:** done\n\n",
        );
        text.push_str("## 2026-11-09 10:00:00 UTC \u{b7} approval\n\n- **Decision:** done\n\n");
        text.push_str("## 2026-11-09 10:00:00 UTC \u{b7} approval\n\n- **Item:** approval `x`\n\n");
        text.push_str("Some stray note the owner typed.\n\n");
        let mut b = entry();
        b.at += 60_000;
        b.decision = "done".into();
        // Saved by an editor that uses Windows line endings
        text.push_str(&b.render().replace('\n', "\r\n"));
        let parsed = parse_month(&text);
        let decisions: Vec<_> = parsed.entries.iter().map(|r| r.decision.as_str()).collect();
        assert_eq!(decisions, ["approved", "done"]);
        assert_eq!(parsed.skipped.len(), 3, "{:?}", parsed.skipped);
        assert!(parse_month("").entries.is_empty());
        assert!(parse_month("# Decisions, 2026-11\n").skipped.is_empty());
    }

    #[test]
    fn filters_narrow_by_kind_decision_and_text_newest_first() {
        let mk = |at: i64, kind: &str, decision: &str, label: &str, reason: Option<&str>| {
            DecisionRecord {
                at,
                kind: kind.into(),
                item_id: format!("id{at}"),
                item_label: label.into(),
                decision: decision.into(),
                decided_by: "Dev".into(),
                reason: reason.map(Into::into),
                chat_id: None,
                chat_exists: false,
            }
        };
        let all = vec![
            mk(1, "approval", "approved", "Send the report", None),
            mk(
                2,
                "gatewayHold",
                "refused",
                "Q3 numbers",
                Some("Board meeting"),
            ),
            mk(3, "approval", "refused", "Delete files", Some("too risky")),
        ];
        let f = |kind: Option<&str>, decision: Option<&str>, search: Option<&str>| {
            select(
                &all,
                &DecisionFilter {
                    kind: kind.map(Into::into),
                    decision: decision.map(Into::into),
                    search: search.map(Into::into),
                },
            )
            .iter()
            .map(|r| r.at)
            .collect::<Vec<_>>()
        };
        assert_eq!(f(None, None, None), [3, 2, 1]);
        assert_eq!(f(Some("approval"), None, None), [3, 1]);
        assert_eq!(f(None, Some("refused"), None), [3, 2]);
        assert_eq!(f(Some("approval"), Some("refused"), None), [3]);
        assert_eq!(f(None, None, Some("BOARD")), [2]);
        assert_eq!(f(None, None, Some("report")), [1]);
        assert_eq!(f(Some(" "), None, Some(" ")), [3, 2, 1]);
        let detailed = DecisionRecord {
            decision: "approved (once)".into(),
            ..all[0].clone()
        };
        let one = [detailed];
        let by = |d: &str| {
            select(
                &one,
                &DecisionFilter {
                    decision: Some(d.into()),
                    ..DecisionFilter::default()
                },
            )
            .len()
        };
        assert_eq!(
            (by("approved"), by("Approved"), by("app"), by("refused")),
            (1, 1, 0, 0)
        );
        assert!(f(None, None, Some("nothing like it")).is_empty());
    }

    #[test]
    fn months_are_checked() {
        assert!(is_month("2026-11"));
        assert!(!is_month("2026-1"));
        assert!(!is_month("../2026"));
        assert!(month_header("2026-11").starts_with("# Decisions, 2026-11\n"));
    }
}
