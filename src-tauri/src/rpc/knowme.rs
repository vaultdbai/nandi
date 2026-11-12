//! Getting to know you (`services::knowme`).

use crate::services::knowme::proposal::KnowMeProposal;
use crate::services::knowme::{self, KnowMeStatus, KnowMeTodoAction};

super::commands! {
    /// The know-me to-do (brought back when its time came), the groups a
    /// personal item may be shared with, and the latest confirmed changes
    fn get_know_me(state) -> KnowMeStatus => knowme::status(&state).await;

    /// The to-do's Later (back in a week) or Skip (back in a month)
    fn know_me_todo(state, action: KnowMeTodoAction) -> KnowMeStatus => knowme::todo(&state, action).await;

    /// Confirms a `knowme_propose` card, with the person's edits
    fn confirm_know_me_proposal(state, approval_id: String, proposal: KnowMeProposal) -> () => knowme::confirm(&state, approval_id, proposal).await;
}
