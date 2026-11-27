//! The user's to-dos: assignments to the user that Nandi (from mail,
//! events and chats, `todo_add`) or the user (by hand) opens here. They are
//! `gateway_assignments` rows with a `source` and no remote caller
//! (`caller_instance` empty), so the Inbox, its Done and its Ask chat work
//! on them as on work a person assigned, and closing one sends nothing.
//!
//! | Action         | Who                    | Decisions entry (`assignment`) |
//! | -------------- | ---------------------- | ------------------------------ |
//! | [`add`]        | Nandi, or the user     | `added`                        |
//! | [`hand_off`]   | the user               | `handed to <agent>`            |
//! | [`done`]       | the user, or Nandi     | `done`                         |
//! | [`reopen`]     | the user (Undo), Nandi | `reopened`                     |
//!
//! Adding from the same place twice (`source` + `source_ref`, an email's
//! message ID) returns the to-do already there. None of this needs the
//! relay.

pub mod tool;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::assignments::{self, AssignmentStatus, GatewayAssignment, GatewayAssignmentChanged};
use crate::db::now_ms;
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::domain::todo::{todo_note, NewTodo, TodoKind, TodoRefs, TodoSource};
use crate::services::events;
use crate::services::inbox::audit::{self, Decided};
use crate::services::inbox::item::InboxItemRef;
use crate::services::AppState;

/// The start of a to-do's ID (a person's assignment is a gateway task ID)
pub const ID_PREFIX: &str = "todo-";

/// The Decisions record's kind for every assignment, to-dos included
const KIND: &str = "assignment";

/// How far back [`done_recently`] looks
const DONE_SHOWN_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Who acts on a to-do
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// The person, from Home, the Inbox or a chat's confirm
    User,
    /// An agent, by its ID
    Agent(String),
}

/// What [`add`] did
#[derive(Debug, Clone, PartialEq)]
pub struct Added {
    pub todo: GatewayAssignment,
    /// It was there already (the same `source` + `source_ref`)
    pub existing: bool,
}

/// A to-do handed to an agent: the chat its run is in
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HandedTodo {
    pub todo: GatewayAssignment,
    pub agent_id: String,
    pub conversation_id: String,
}

/// An agent's name, else its ID
fn agent_name(state: &AppState, agent_id: &str) -> String {
    state
        .store()
        .agent(agent_id)
        .map(|agent| agent.name().to_string())
        .unwrap_or_else(|_| agent_id.to_string())
}

/// The name a to-do is "from": the agent that added it, else the user
fn added_by(state: &AppState, by: &Actor) -> String {
    match by {
        Actor::User => crate::services::chat::decider(state),
        Actor::Agent(id) => agent_name(state, id),
    }
}

/// Records `decided`: the user's as theirs, an agent's under its name
fn record(state: &AppState, by: &Actor, decided: Decided) {
    let decided = match by {
        Actor::User => decided,
        Actor::Agent(id) => decided.by(agent_name(state, id)),
    };
    audit::record(state, decided);
}

fn changed(state: &AppState, todo: &GatewayAssignment) {
    events::emit(
        state.bus(),
        &GatewayAssignmentChanged {
            assignment: todo.clone(),
        },
    );
}

/// A to-do's row before it is opened. Pure, for tests.
pub fn assignment_of(todo: &NewTodo, task_id: String, from: String, now: i64) -> GatewayAssignment {
    GatewayAssignment {
        task_id,
        caller_instance: String::new(),
        caller_name: from,
        request: todo.text.clone(),
        status: AssignmentStatus::Open,
        created_at: now as f64,
        due_at: todo.due_at.map(|t| t as f64),
        kind: todo.kind,
        source: Some(todo.source),
        source_ref: todo.source_ref.clone(),
        related_ref: todo.related_ref.clone(),
        context: todo.context.clone(),
        ..GatewayAssignment::default()
    }
}

/// Opens a to-do for the user (or returns the one already there for its
/// source), records it and tells the Inbox
pub async fn add(state: &AppState, todo: NewTodo, by: Actor) -> Result<Added, String> {
    let task_id = format!("{ID_PREFIX}{}", uuid::Uuid::new_v4());
    let row = assignment_of(&todo, task_id.clone(), added_by(state, &by), now_ms());
    let opened = state
        .db()
        .call(move |conn| assignments::open(conn, &row))
        .await?;
    let existing = opened.task_id != task_id;
    if !existing {
        record(
            state,
            &by,
            Decided::new(KIND, &opened.task_id, "added").subject(&opened.request),
        );
        changed(state, &opened);
    }
    Ok(Added {
        todo: opened,
        existing,
    })
}

/// The user adds a to-do by hand. `due`: `YYYY-MM-DD` (local), `N days` or
/// an RFC 3339 time.
pub async fn add_manual(
    state: &AppState,
    text: String,
    kind: TodoKind,
    due: Option<String>,
) -> Result<GatewayAssignment, String> {
    let todo = NewTodo::new(
        &text,
        kind,
        due.as_deref(),
        TodoSource::Manual,
        TodoRefs::default(),
        now_ms(),
        &chrono::Local,
    )
    .map_err(|e| e.to_string())?;
    Ok(add(state, todo, Actor::User).await?.todo)
}

/// A to-do by its ID, if it is one
async fn get_todo(state: &AppState, task_id: &str) -> Result<GatewayAssignment, String> {
    let id = task_id.to_string();
    state
        .db()
        .call(move |conn| assignments::get(conn, &id))
        .await?
        .filter(GatewayAssignment::is_local)
        .ok_or_else(|| "That to-do wasn't found".to_string())
}

/// Open to-dos (as the Inbox lists them), then those done in the last 7
/// days, the latest first
pub async fn list(state: &AppState) -> Result<Vec<GatewayAssignment>, String> {
    let since = now_ms() - DONE_SHOWN_MS;
    state
        .db()
        .call(move |conn| {
            let mut todos: Vec<GatewayAssignment> = assignments::open_list(conn)?
                .into_iter()
                .filter(GatewayAssignment::is_local)
                .collect();
            todos.extend(assignments::done_todos_since(conn, since)?);
            Ok(todos)
        })
        .await
}

/// Marks a to-do done, with what was done in `note`
pub async fn done(
    state: &AppState,
    task_id: String,
    by: Actor,
    note: Option<String>,
) -> Result<GatewayAssignment, String> {
    get_todo(state, &task_id).await?;
    let by_name = match &by {
        Actor::User => None,
        Actor::Agent(id) => Some(agent_name(state, id)),
    };
    super::close_assignment_as(state, task_id, true, todo_note(note.as_deref()), by_name).await
}

/// Opens a done to-do again: the user's Undo, or Nandi when a finding it
/// closed as fixed is back (`github_security`)
pub async fn reopen(
    state: &AppState,
    task_id: String,
    by: Actor,
) -> Result<GatewayAssignment, String> {
    let todo = state
        .db()
        .call(move |conn| assignments::reopen(conn, &task_id))
        .await?
        .ok_or("That to-do isn't done, or it isn't there")?;
    record(
        state,
        &by,
        Decided::new(KIND, &todo.task_id, "reopened").subject(&todo.request),
    );
    changed(state, &todo);
    Ok(todo)
}

/// What an agent is asked when it is handed `text`. Pure, for tests.
pub fn hand_off_text(text: &str) -> String {
    format!("Please do this: {text}")
}

/// Hands an open to-do to `agent_id`: a turn in Nandi's chat about it
/// (its Ask chat, with Done), or in another agent's chat. The to-do
/// remembers the run, so it shows as that agent's work while it goes; the
/// agent doesn't close it (only Nandi may, with `todo_done`).
pub async fn hand_off(
    state: &AppState,
    task_id: String,
    agent_id: String,
) -> Result<HandedTodo, String> {
    let todo = get_todo(state, &task_id).await?;
    if todo.status != AssignmentStatus::Open {
        return Err("That to-do is done already".to_string());
    }
    let text = hand_off_text(&todo.request);
    let (conversation_id, sent) = if agent_id == DEFAULT_AGENT {
        let thread = InboxItemRef::Assignment {
            task_id: task_id.clone(),
        }
        .conversation_id();
        crate::services::inbox::thread::open(state, thread.clone()).await?;
        let sent = crate::services::inbox::thread::send(state, thread.clone(), text, None).await?;
        (thread, sent)
    } else {
        let sent =
            crate::services::chat::send_message(state, agent_id.clone(), text, Vec::new(), None)
                .await?;
        let id = agent_id.clone();
        let conversation = state
            .db()
            .call(move |conn| crate::db::ensure_agent_conversation(conn, &id))
            .await?;
        (conversation, sent)
    };
    let todo = handed(state, task_id, agent_id.clone(), sent.run_id).await?;
    Ok(HandedTodo {
        todo,
        agent_id,
        conversation_id,
    })
}

/// Records that the user handed an open to-do to `agent_id`, whose run
/// `run_id` works on it: [`hand_off`], or an ask from a paired phone
/// (`own_devices`), which starts the run in the phone's own chat
pub async fn handed(
    state: &AppState,
    task_id: String,
    agent_id: String,
    run_id: String,
) -> Result<GatewayAssignment, String> {
    let (id, agent) = (task_id, agent_id.clone());
    let todo = state
        .db()
        .call(move |conn| assignments::hand_off(conn, &id, &agent, &run_id))
        .await?
        .ok_or("That to-do is done already")?;
    record(
        state,
        &Actor::User,
        Decided::new(
            KIND,
            &todo.task_id,
            format!("handed to {}", agent_name(state, &agent_id)),
        )
        .subject(&todo.request),
    );
    changed(state, &todo);
    Ok(todo)
}
