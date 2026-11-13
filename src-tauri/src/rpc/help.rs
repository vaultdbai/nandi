//! The built-in help guide (Tauri adapter for `services::help`).

use crate::services::help::{self, HelpAnswer, HelpReason, HelpTopic};

super::commands! {
    /// Every topic of the built-in help guide, in order
    fn help_topics() -> Vec<HelpTopic> => Ok(help::topics());

    /// The help guide's answer to `text`, for when no model can answer (a chat
    /// sent with no model)
    fn help_answer(text: String) -> HelpAnswer => Ok(help::answer(&text, HelpReason::NoModel, None));
}
