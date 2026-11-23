//! The weekly retro's proposals in the Inbox (`services::retro`).

use crate::services::retro::{self, RetroAction, RetroItem};

super::commands! {
    /// Proposals waiting, changes applied this week, and changes the
    /// check says didn't help (after running the check)
    fn list_retro_items(state) -> Vec<RetroItem> => retro::list(&state).await;

    /// Apply (one gold copy commit), refuse, undo or keep a proposal
    fn act_on_retro_proposal(state, id: String, action: RetroAction) -> () => retro::act(&state, id, action).await;
}
