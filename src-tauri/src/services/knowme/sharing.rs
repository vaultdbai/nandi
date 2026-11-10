//! A personal item shared with one group (owner decision, 2026-11-09).
//!
//! The person picks one group they're in; the item is kept in My notes,
//! `private/groups/<group>.md` (`proposal::group_note`), and nowhere else:
//! not on the card, not on the relay. When a member of that group asks
//! this Nandi (`ask-me`, directly or through the gateway), the Chief of
//! Staff's turn on their request gets those items ([`prompt_for_task`]),
//! so it may answer that member with them. Anyone else's request doesn't.
//! The answer travels like any answer (sealed, through the relay); the
//! membership is this Nandi's own group copy (`.nandi/groups/`).
//!
//! What this doesn't do: the relay's people search (`find_people`) can't
//! find someone by a group-shared item. That would need the relay to
//! publish per-group fields (protocol work); see
//! `docs/developer/know-me.md`.

use rusqlite::OptionalExtension;

use super::proposal::group_note;
use crate::services::AppState;

/// The items shared with the groups the requester of `task_id` (the Chief
/// of Staff's task conversation `task:<id>`) is in with this Nandi, for
/// its system prompt; `None` for a local request or nothing shared
pub async fn prompt_for_task(state: &AppState, task_id: &str) -> Option<String> {
    // A gateway's hand-off to the Chief of Staff answers the gateway task
    let root = crate::services::a2a::executor::gateway_parent(task_id).to_string();
    let requester: String = state
        .db()
        .call(move |conn| {
            conn.query_row(
                "SELECT requester_instance FROM tasks WHERE id = ?1",
                [root],
                |row| row.get(0),
            )
            .optional()
        })
        .await
        .ok()
        .flatten()?;
    let me = crate::services::network::local_instance_id(state.db()).ok()?;
    if requester == me {
        return None;
    }
    let (copies, _) = state.store().repos().groups.load_all().ok()?;
    let folder = crate::services::notes::existing_folder(state)?;
    let mut sections = Vec::new();
    for copy in copies {
        let member = |id: &str| copy.members.iter().any(|m| m.instance_id == id);
        if !member(&requester) || !member(&me) {
            continue;
        }
        let Ok(text) = folder.read(&group_note(&copy.group_id)) else {
            continue;
        };
        let items: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("- "))
            .collect();
        if items.is_empty() {
            continue;
        }
        let name = copy.name.replace(['"', '\n'], "");
        sections.push(format!(
            "Shared with the group \"{name}\":\n{}",
            items.join("\n")
        ));
    }
    if sections.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n## Shared with this requester's group\n\nThe person asking is in a group with the \
user, and the user chose to share these personal items with that group's members. You may use \
them when answering this person. Everything else in `my-notes/private/` stays private.\n\n\
<group-shares>\n{}\n</group-shares>\n",
        sections.join("\n\n")
    ))
}
