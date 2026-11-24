//! Nandi's to-do tools: thin wrappers over [`super`], so Nandi keeps the
//! user's to-do list from what it reads. Run on the host, offered only to
//! Nandi when its `agent.md` lists them, in chats, triggers and the email
//! and calendar checks' runs. They need no approval: like the notes, the
//! list is Nandi's to keep, and every add and done is in Decisions.
//!
//! Never in a run for someone else: a task delegated by a teammate or
//! another Nandi (`SourceType::Agent`, the gateway's `ask-me` and
//! `assign-me` included) gets none of them ([`allowed_in`]), and a call
//! from one is refused, so nobody else reads or changes the user's list.
//!
//! | Tool        | Arguments                                         | Does                                   |
//! | ----------- | ------------------------------------------------- | -------------------------------------- |
//! | `todo_add`  | `{text, kind, due?, source, source_ref?, related_ref?}` | adds one (same source_ref: no new) |
//! | `todo_list` | `{}`                                              | open ones, then done in the last week  |
//! | `todo_done` | `{id, note?}`                                     | marks one Nandi finished itself done   |

use serde_json::{json, Value};

use super::Actor;
use crate::db::now_ms;
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::model::SourceType;
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::domain::todo::{NewTodo, TodoKind, TodoRefs, TodoSource, MAX_TEXT_CHARS};
use crate::services::gateway::assignments::{AssignmentStatus, GatewayAssignment};
use crate::services::repo_store::RepoStore;
use crate::services::AppState;

pub const ADD: &str = "todo_add";
pub const LIST: &str = "todo_list";
pub const DONE: &str = "todo_done";
const TOOLS: [&str; 3] = [ADD, LIST, DONE];

pub fn is_todo_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// Whether a run started from `source` may keep the user's to-dos: never
/// one delegated by someone else. Pure, for tests.
pub fn allowed_in(source: Option<SourceType>) -> bool {
    source != Some(SourceType::Agent)
}

/// What a run for someone else is told when it calls a to-do tool
pub const REFUSED_FOR_OTHERS: &str =
    "The user's to-dos aren't available while you answer for someone else";

/// The to-do tools `agent_id` may call in a run started from `source`:
/// Nandi's, those its `agent.md` lists in `allowed-tools`, and none in a
/// run for someone else
pub fn offered(store: &RepoStore, agent_id: &str, source: Option<SourceType>) -> Vec<ToolSpec> {
    if agent_id != DEFAULT_AGENT || !allowed_in(source) {
        return Vec::new();
    }
    let Ok(agent) = store.agent(agent_id) else {
        return Vec::new();
    };
    let allowed = agent.capabilities().tools().allowed().unwrap_or_default();
    TOOLS
        .into_iter()
        .filter(|tool| allowed.iter().any(|t| t == tool))
        .filter_map(spec)
        .collect()
}

pub fn spec(name: &str) -> Option<ToolSpec> {
    let (description, schema) = match name {
        ADD => (
            "Add a to-do for the user: one thing they must do, check or remember, as one plain \
sentence of what to do with its deadline (\"Reply to the landlord about the deposit: ask for the \
inspection result. Deadline 2026-12-07.\"). `kind`: `do` (an action), `check` (verify and close) \
or `reminder` (a date to remember; give the date as `due`). `due`: YYYY-MM-DD, \"N days\" or an \
RFC 3339 time. `source` is where you read it. `source_ref` names the thing within it: the \
email's message id (an event's id) and its number in the order the asks appear, \
`<message-id>#1`, `<message-id>#2`. Read the same email again and use the same numbers: a \
source_ref already used returns the to-do already there (`existing: true`). \
`related_ref`: the Inbox thread of what you prepared for it (`inbox:approval:<id>`, a Request's \
id). When the user has several mailboxes, say in `text` which one an email came to \
(\"… (to me@work.com)\"). Don't add what is already done.",
            json!({"type": "object", "required": ["text", "kind", "source"], "properties": {
                "text": {"type": "string", "maxLength": MAX_TEXT_CHARS},
                "kind": {"type": "string", "enum": ["do", "check", "reminder"]},
                "due": {"type": "string"},
                "source": {"type": "string", "enum": ["email", "event", "chat"]},
                "source_ref": {"type": "string"},
                "related_ref": {"type": "string"},
            }}),
        ),
        LIST => (
            "List the user's to-dos: open ones soonest due first (with `overdue`, and the agent \
working on one), then those done in the last 7 days. Times are local.",
            json!({"type": "object", "properties": {}}),
        ),
        DONE => (
            "Mark a to-do done (an id from todo_list). Only for one you finished yourself, with \
what you did in `note`. When the user did it, they mark it in the Inbox.",
            json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"},
                "note": {"type": "string"},
            }}),
        ),
        _ => return None,
    };
    Some(ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schema,
    })
}

fn error(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: message.into(),
        is_error: true,
    }
}

fn ok(value: Value) -> ToolOutcome {
    ToolOutcome {
        result: value.to_string(),
        is_error: false,
    }
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Where an agent may say it read a to-do (`manual` is the user's own,
/// `security_check` the GitHub check's, which closes its own findings)
pub fn agent_source(source: &str) -> Option<TodoSource> {
    match source.parse::<TodoSource>() {
        Ok(TodoSource::Manual | TodoSource::SecurityCheck) | Err(_) => None,
        Ok(source) => Some(source),
    }
}

fn local_time(ms: f64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|at| {
        at.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
}

/// A to-do as the model sees it. Pure, for tests (`now` in epoch ms).
pub fn for_model(todo: &GatewayAssignment, now: i64) -> Value {
    let open = todo.status == AssignmentStatus::Open;
    json!({
        "id": todo.task_id,
        "text": todo.request,
        "kind": todo.kind,
        "done": !open,
        "due": todo.due_at.and_then(local_time),
        "overdue": open && todo.due_at.is_some_and(|d| (d as i64) < now),
        "source": todo.source,
        "source_ref": todo.source_ref,
        "related_ref": todo.related_ref,
        "handed_to": todo.assigned_agent,
        "done_note": todo.note,
    })
}

/// Runs a to-do tool for `agent_id`
pub async fn call(state: &AppState, agent_id: &str, tool: &str, args: &Value) -> ToolOutcome {
    let by = Actor::Agent(agent_id.to_string());
    match tool {
        ADD => {
            let Some(source) = text_arg(args, "source").and_then(agent_source) else {
                return error("'source' is one of email, event or chat");
            };
            let kind = match text_arg(args, "kind").map(str::parse::<TodoKind>) {
                None => TodoKind::Do,
                Some(Ok(kind)) => kind,
                Some(Err(_)) => return error("'kind' is one of do, check or reminder"),
            };
            let refs = TodoRefs {
                source: text_arg(args, "source_ref"),
                related: text_arg(args, "related_ref"),
            };
            let todo = match NewTodo::new(
                text_arg(args, "text").unwrap_or_default(),
                kind,
                text_arg(args, "due"),
                source,
                refs,
                now_ms(),
                &chrono::Local,
            ) {
                Ok(todo) => todo,
                Err(e) => return error(e.to_string()),
            };
            match super::add(state, todo, by).await {
                Ok(added) => {
                    let mut value = for_model(&added.todo, now_ms());
                    value["existing"] = json!(added.existing);
                    ok(value)
                }
                Err(e) => error(e),
            }
        }
        LIST => match super::list(state).await {
            Ok(todos) => {
                let now = now_ms();
                let todos: Vec<Value> = todos.iter().map(|t| for_model(t, now)).collect();
                ok(json!({ "todos": todos }))
            }
            Err(e) => error(e),
        },
        DONE => {
            let Some(id) = text_arg(args, "id") else {
                return error("'id' is required");
            };
            let note = text_arg(args, "note").map(str::to_string);
            match super::done(state, id.to_string(), by, note).await {
                Ok(todo) => ok(for_model(&todo, now_ms())),
                Err(e) => error(e),
            }
        }
        _ => error(format!("Unknown to-do tool '{tool}'")),
    }
}
