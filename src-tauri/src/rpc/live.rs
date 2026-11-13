//! Live UI commands (Tauri adapter for `services::live`).

use crate::services::live::{self, LiveChange, RefreshReport};

super::commands! {
    /// The latest file changes, newest first: who changed what
    fn list_recent_changes(state) -> Vec<LiveChange> => Ok(state.live().feed().recent());

    /// Re-reads every definition from disk (the Refresh button, F5)
    fn refresh_from_disk(state) -> RefreshReport => live::refresh_from_disk(&state).await;
}
