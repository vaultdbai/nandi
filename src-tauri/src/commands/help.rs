//! The built-in help guide (Tauri adapter for `services::help`).

use crate::services::help::{self, HelpAnswer, HelpReason, HelpTopic};

/// Every topic of the built-in help guide, in order
#[tauri::command]
#[specta::specta]
pub async fn help_topics() -> Result<Vec<HelpTopic>, String> {
    Ok(help::topics())
}

/// The help guide's answer to `text`, for when no model can answer (a chat
/// sent with no model)
#[tauri::command]
#[specta::specta]
pub async fn help_answer(text: String) -> Result<HelpAnswer, String> {
    Ok(help::answer(&text, HelpReason::NoModel, None))
}
