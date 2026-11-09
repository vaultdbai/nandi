//! The Decisions page's reads: the months that have decisions, and one
//! month's entries (newest first, filtered), each with whether its Ask
//! chat is still on this computer. Purely local: the gold copy's
//! `.nandi/decisions/` files and `state.db`, no relay.

use crate::services::domain::decision::{select, DecisionFilter, DecisionRecord};
use crate::services::AppState;

/// The months with a decisions file (`2026-11`), newest first
pub fn months(state: &AppState) -> Result<Vec<String>, String> {
    Ok(state.store().repos().decisions.months()?)
}

/// One month's decisions that pass `filter`, newest first
pub async fn list(
    state: &AppState,
    month: String,
    filter: Option<DecisionFilter>,
) -> Result<Vec<DecisionRecord>, String> {
    let entries = state.store().repos().decisions.entries(&month)?;
    let mut out = select(&entries, &filter.unwrap_or_default());
    let chats: Vec<String> = out.iter().filter_map(|r| r.chat_id.clone()).collect();
    if chats.is_empty() {
        return Ok(out);
    }
    let existing = state
        .db()
        .call(move |conn| {
            let mut found = std::collections::HashSet::new();
            for chat in chats {
                if crate::db::conversation_agent(conn, &chat)?.is_some() {
                    found.insert(chat);
                }
            }
            Ok(found)
        })
        .await?;
    for record in &mut out {
        record.chat_exists = record
            .chat_id
            .as_ref()
            .is_some_and(|chat| existing.contains(chat));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::inbox::audit::{in_chat, record, Decided};

    #[tokio::test]
    async fn the_months_and_their_entries_come_back_newest_first_with_chat_state() {
        let state = AppState::for_tests(Vec::new());
        assert!(months(&state).unwrap().is_empty());
        let month = chrono::Utc::now().format("%Y-%m").to_string();
        assert!(list(&state, month.clone(), None).await.unwrap().is_empty());

        record(&state, Decided::new("failed", "f1", "dismissed"));
        in_chat(
            "inbox:approval:a1".into(),
            Some("only once".into()),
            async {
                record(
                    &state,
                    Decided::new("approval", "a1", "approved").subject("run_report"),
                );
            },
        )
        .await;
        // The chat of the second one exists; a third names one that doesn't
        state
            .db()
            .call(|conn| {
                crate::services::inbox::thread::ensure(conn, "inbox:approval:a1", "run_report")
            })
            .await
            .unwrap();
        in_chat("inbox:approval:gone".into(), None, async {
            record(&state, Decided::new("approval", "a2", "refused"));
        })
        .await;

        assert_eq!(months(&state).unwrap(), [month.clone()]);
        let all = list(&state, month.clone(), None).await.unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.windows(2).all(|w| w[0].at >= w[1].at));
        let approved = all.iter().find(|r| r.item_id == "a1").unwrap();
        assert_eq!(approved.item_label, "run_report");
        assert_eq!(approved.reason.as_deref(), Some("only once"));
        assert_eq!(approved.chat_id.as_deref(), Some("inbox:approval:a1"));
        assert!(approved.chat_exists);
        let gone = all.iter().find(|r| r.item_id == "a2").unwrap();
        assert!(!gone.chat_exists);
        assert!(!all.iter().find(|r| r.item_id == "f1").unwrap().chat_exists);

        let filter = DecisionFilter {
            kind: Some("approval".into()),
            decision: Some("refused".into()),
            search: None,
        };
        let some = list(&state, month, Some(filter)).await.unwrap();
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].item_id, "a2");
        assert!(list(&state, "nope".into(), None).await.is_err());
    }
}
