//! The mascot: its name and the hidden phrase.

use crate::services::mascot::{self, MascotInfo, PhraseResponse};

super::commands! {
    /// The bull's name
    fn mascot_get(state) -> MascotInfo => Ok(mascot::read(state.nandi_dir()));

    /// Renames the bull (`null`: back to the default)
    fn mascot_set(state, name: Option<String>) -> MascotInfo => mascot::write(state.nandi_dir(), name.as_deref());

    /// The bull's reply to a typed phrase, if it's the hidden one. Only words
    /// come back: nothing is approved, run or routed.
    fn mascot_phrase(state, input: String) -> Option<PhraseResponse> => Ok(mascot::handle_phrase(&input, &mascot::read(state.nandi_dir()).name));
}
