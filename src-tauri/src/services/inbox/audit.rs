//! Every Inbox decision goes into the gold copy
//! (`.nandi/decisions/<yyyy-mm>.md`, `repos::decision_repo`), from a
//! button or from the item's Ask chat. Each decision service calls
//! [`record`] once, after the decision took effect, so both paths write
//! the same entry.
//!
//! A decision from a chat runs inside [`in_chat`]: the entry then names the
//! conversation, and carries the note the person confirmed when the
//! service itself has no place for one (a plain tool approval). The chat's
//! text stays in `state.db`.

use std::future::Future;

use crate::services::domain::decision::DecisionEntry;
use crate::services::AppState;

/// The chat a decision is being made from
#[derive(Debug, Clone)]
struct ChatDecision {
    conversation_id: String,
    note: Option<String>,
}

tokio::task_local! {
    static CHAT: ChatDecision;
}

/// Runs `decide` as a decision made in the chat `conversation_id`, with
/// the person's `note`
pub async fn in_chat<F: Future>(
    conversation_id: String,
    note: Option<String>,
    decide: F,
) -> F::Output {
    CHAT.scope(
        ChatDecision {
            conversation_id,
            note: note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        },
        decide,
    )
    .await
}

/// What a decision service records
#[derive(Debug, Clone, Default)]
pub struct Decided {
    /// The Inbox kind (`approval`, `gatewayHold`, …)
    pub kind: &'static str,
    pub id: String,
    /// What it was about, in a few words
    pub subject: Option<String>,
    /// `approved`, `refused`, `answered`, `done`, …
    pub decision: String,
    /// The reason or note that went with it
    pub note: Option<String>,
}

impl Decided {
    pub fn new(kind: &'static str, id: impl Into<String>, decision: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            decision: decision.into(),
            ..Self::default()
        }
    }

    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        let subject = subject.into();
        self.subject = (!subject.trim().is_empty()).then_some(subject);
        self
    }

    pub fn note(mut self, note: Option<&str>) -> Self {
        self.note = note
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        self
    }
}

/// The entry for `decided`, by `who` at `at`, in the chat (if any)
fn entry(decided: Decided, who: String, at: i64) -> DecisionEntry {
    let chat = CHAT.try_with(Clone::clone).ok();
    DecisionEntry {
        at,
        decided_by: who,
        item_kind: decided.kind.to_string(),
        item_id: decided.id,
        subject: decided.subject,
        decision: decided.decision,
        note: decided
            .note
            .or_else(|| chat.as_ref().and_then(|c| c.note.clone())),
        conversation_id: chat.map(|c| c.conversation_id),
    }
}

/// Appends the decision to this month's file. The decision already took
/// effect, so a failure to write is logged, never returned.
pub fn record(state: &AppState, decided: Decided) {
    let who = crate::services::chat::decider(state);
    let entry = entry(decided, who, crate::db::now_ms());
    if let Err(e) = state.store().repos().decisions.append(&entry) {
        log::warn!(
            "Couldn't record the decision on {} {}: {e}",
            entry.item_kind,
            entry.item_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn this_month(state: &AppState) -> String {
        let month = chrono::Utc::now().format("%Y-%m").to_string();
        state
            .store()
            .repos()
            .decisions
            .read(&month)
            .unwrap()
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn a_button_decision_is_recorded_without_a_chat() {
        let state = AppState::for_tests(Vec::new());
        record(
            &state,
            Decided::new("gatewayHold", "t1", "refused")
                .subject("Sam asks")
                .note(Some(" too early ")),
        );
        let text = this_month(&state);
        assert!(
            text.contains("- **Item:** gatewayHold `t1` (Sam asks)"),
            "{text}"
        );
        assert!(text.contains("- **Decision:** refused"), "{text}");
        assert!(text.contains("- **Reason:** too early"), "{text}");
        assert!(!text.contains("**Chat:**"), "{text}");
    }

    #[tokio::test]
    async fn a_chat_decision_names_the_chat_and_carries_its_note() {
        let state = AppState::for_tests(Vec::new());
        in_chat(
            "inbox:approval:a1".into(),
            Some("only this once".into()),
            async {
                record(&state, Decided::new("approval", "a1", "approved"));
            },
        )
        .await;
        let text = this_month(&state);
        assert!(text.contains("- **Chat:** `inbox:approval:a1`"), "{text}");
        assert!(text.contains("- **Reason:** only this once"), "{text}");
        // The service's own note wins over the chat's
        in_chat("inbox:assignment:t2".into(), Some("chat".into()), async {
            record(
                &state,
                Decided::new("assignment", "t2", "done").note(Some("sent")),
            );
        })
        .await;
        let text = this_month(&state);
        assert!(text.contains("- **Reason:** sent"), "{text}");
    }
}
