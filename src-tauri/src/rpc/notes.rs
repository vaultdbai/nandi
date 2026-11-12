//! My notes and the Chief of Staff's suggestions.

use crate::services::notes::corrections::Suggestion;
use crate::services::notes::{self, MyNotes, NoteFile};

super::commands! {
    /// The notes folder and its notes; creates and shares it on first use
    fn get_my_notes(state) -> MyNotes => notes::my_notes(&state).await;

    /// One note's text
    fn read_note(state, name: String) -> String => notes::read_note(&state, name).await;

    /// Creates or replaces a note
    fn save_note(state, name: String, content: String) -> NoteFile => notes::save_note(&state, name, content).await;

    /// Deletes a note
    fn delete_note(state, name: String) -> () => notes::delete_note(&state, name).await;

    /// Lets agents update the notes (without asking each time), or not
    fn set_notes_agents_can_write(state, allow: bool) -> MyNotes => notes::set_agents_can_write(&state, allow).await;

    /// The Chief of Staff's open capability suggestions
    fn list_note_suggestions(state) -> Vec<Suggestion> => notes::suggestions(&state).await;

    /// Accepts or dismisses a suggestion (remembered in corrections.md)
    fn answer_note_suggestion(state, id: String, accept: bool) -> String => notes::answer_suggestion(&state, id, accept).await;
}
