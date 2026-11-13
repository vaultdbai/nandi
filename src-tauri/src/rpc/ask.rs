//! Ask box commands (Tauri adapter for `services::ask`).

use crate::services::ask;
use crate::services::ask::{AskHistoryEntry, AskRequest, AskResponse};

super::commands! {
    /// Classifies the ask and answers it: an inline answer, a plan, an edit
    /// preview, or a clarifying question
    fn ask(state, request: AskRequest) -> AskResponse => ask::ask(&state, request).await;

    /// Starts answering; returns the request ID. `ask://streaming_delta` and
    /// `ask://completed` follow.
    fn ask_streaming(state, request: AskRequest) -> String => ask::ask_streaming(&state, request).await;

    /// Recent asks, newest first; `limit` `None` is 20
    fn list_ask_history(state, limit: Option<u32>) -> Vec<AskHistoryEntry> => ask::list_ask_history(&state, limit).await;
}
