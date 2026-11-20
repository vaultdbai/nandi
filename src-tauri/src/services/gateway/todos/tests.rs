//! Tests for the user's to-dos (`gateway::todos`)

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Value};

use super::*;
use crate::model::RunStatus;

const NOW: i64 = 1_800_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1000;

fn new(text: &str, kind: TodoKind, due: Option<&str>, source_ref: Option<&str>) -> NewTodo {
    let refs = TodoRefs {
        source: source_ref,
        related: None,
    };
    NewTodo::new(text, kind, due, TodoSource::Email, refs, NOW, &Utc).unwrap()
}

fn conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::init_db(&conn).unwrap();
    conn
}

fn decisions(state: &AppState) -> String {
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    state
        .store()
        .repos()
        .decisions
        .read(&month)
        .unwrap()
        .unwrap_or_default()
}

async fn get(state: &AppState, task_id: &str) -> GatewayAssignment {
    let id = task_id.to_string();
    state
        .db()
        .call(move |conn| assignments::get(conn, &id))
        .await
        .unwrap()
        .unwrap()
}

#[test]
fn a_todo_is_a_local_assignment() {
    let todo = new(
        "Reply to the landlord",
        TodoKind::Check,
        Some("1 day"),
        Some("m1"),
    );
    let row = assignment_of(&todo, "todo-1".into(), "Nandi".into(), NOW);
    assert!(row.is_local());
    assert_eq!(row.caller_instance, "", "no remote caller");
    assert_eq!(row.caller_name, "Nandi");
    assert_eq!(row.request, "Reply to the landlord");
    assert_eq!(row.kind, TodoKind::Check);
    assert_eq!(
        row.due_at,
        crate::services::domain::todo::parse_todo_due("1 day", NOW, &Utc).map(|t| t as f64)
    );
    assert_eq!(row.source_ref.as_deref(), Some("m1"));
    assert_eq!(
        hand_off_text("Call the dentist"),
        "Please do this: Call the dentist"
    );
}

#[test]
fn to_dos_sort_with_assigned_work_and_one_per_source_ref() {
    let conn = conn();
    let open = |id: &str, due: Option<&str>, source_ref: Option<&str>| {
        let todo = new(id, TodoKind::Do, due, source_ref);
        assignments::open(&conn, &assignment_of(&todo, id.into(), "Nandi".into(), NOW)).unwrap()
    };
    open("later", Some("5 days"), Some("m1"));
    open("soon", Some("1 day"), Some("m2"));
    open("undated", None, None);
    // The same email read again: the to-do already there
    let again = open("again", Some("9 days"), Some("m1"));
    assert_eq!(again.task_id, "later");

    let ids: Vec<String> = assignments::open_list(&conn)
        .unwrap()
        .into_iter()
        .map(|a| a.task_id)
        .collect();
    assert_eq!(ids, ["soon", "later", "undated"]);
}

#[test]
fn a_handed_todo_follows_its_run_until_done_then_undo_clears_it() {
    let conn = conn();
    let todo = new("Book the venue", TodoKind::Do, None, None);
    assignments::open(
        &conn,
        &assignment_of(&todo, "todo-1".into(), "You".into(), NOW),
    )
    .unwrap();
    // Open, not handed
    let open = assignments::get(&conn, "todo-1").unwrap().unwrap();
    assert_eq!((open.run_id, open.run_status), (None, None));

    // Handed: the run goes
    crate::db::tests::start_run(&conn, "writer", "run-1");
    let handed = assignments::hand_off(&conn, "todo-1", "writer", "run-1")
        .unwrap()
        .unwrap();
    assert_eq!(handed.assigned_agent.as_deref(), Some("writer"));
    assert_eq!(handed.run_status, Some(RunStatus::Running));

    // The run ended: still open, for the user to check and mark done
    crate::db::set_run_status(&conn, "run-1", RunStatus::Completed, None).unwrap();
    let ended = assignments::get(&conn, "todo-1").unwrap().unwrap();
    assert_eq!(ended.status, AssignmentStatus::Open);
    assert_eq!(ended.run_status, Some(RunStatus::Completed));

    // Done, then Undo: open again, not handed
    assignments::close(&conn, "todo-1", AssignmentStatus::Done, Some("booked"), NOW).unwrap();
    let done = assignments::done_todos_since(&conn, NOW - DAY).unwrap();
    assert_eq!(done.len(), 1);
    assert!(assignments::hand_off(&conn, "todo-1", "writer", "run-2")
        .unwrap()
        .is_none());
    let reopened = assignments::reopen(&conn, "todo-1").unwrap().unwrap();
    assert_eq!(reopened.status, AssignmentStatus::Open);
    assert_eq!(
        (reopened.note, reopened.closed_at, reopened.run_id),
        (None, None, None)
    );
    assert!(assignments::reopen(&conn, "todo-1").unwrap().is_none());
}

#[test]
fn work_a_person_assigned_is_never_reopened_or_handed_as_a_todo() {
    let conn = conn();
    let theirs = GatewayAssignment {
        task_id: "t1".into(),
        caller_instance: "desktop-sam".into(),
        caller_name: "Sam Lee".into(),
        request: "Upload the evidence".into(),
        created_at: NOW as f64,
        ..GatewayAssignment::default()
    };
    assignments::open(&conn, &theirs).unwrap();
    assert!(!assignments::get(&conn, "t1").unwrap().unwrap().is_local());
    assert!(assignments::hand_off(&conn, "t1", "writer", "r1")
        .unwrap()
        .is_none());
    assignments::close(&conn, "t1", AssignmentStatus::Done, None, NOW).unwrap();
    assert!(assignments::reopen(&conn, "t1").unwrap().is_none());
    assert!(assignments::done_todos_since(&conn, 0).unwrap().is_empty());
}

#[tokio::test]
async fn adds_dones_and_reopens_are_recorded_and_close_without_a_relay() {
    let state = AppState::for_tests(Vec::new());
    let from_mail = new("Reply to the landlord", TodoKind::Do, None, Some("msg-9"));
    let added = add(
        &state,
        from_mail.clone(),
        Actor::Agent(DEFAULT_AGENT.into()),
    )
    .await
    .unwrap();
    assert!(!added.existing);
    assert!(added.todo.task_id.starts_with(ID_PREFIX));
    let again = add(&state, from_mail, Actor::Agent(DEFAULT_AGENT.into()))
        .await
        .unwrap();
    assert!(again.existing);
    assert_eq!(again.todo.task_id, added.todo.task_id);

    let id = added.todo.task_id.clone();
    // No relay, no gateway task: Done just closes it
    let done = done(&state, id.clone(), Actor::User, Some(" replied ".into()))
        .await
        .unwrap();
    assert_eq!(done.status, AssignmentStatus::Done);
    assert!(super::done(&state, id.clone(), Actor::User, None)
        .await
        .is_err());
    reopen(&state, id.clone()).await.unwrap();
    assert!(reopen(&state, id.clone()).await.is_err());

    let text = decisions(&state);
    let item = format!("- **Item:** assignment `{id}`");
    assert_eq!(text.matches(&item).count(), 3, "{text}");
    assert_eq!(text.matches("- **Decision:** added").count(), 1, "{text}");
    assert!(text.contains("- **Decision:** done"), "{text}");
    assert!(text.contains("- **Reason:** replied"), "{text}");
    assert!(text.contains("- **Decision:** reopened"), "{text}");
    // Nandi added it: recorded under its name, not the user's
    let by_nandi = format!("- **Decided by:** {}", agent_name(&state, DEFAULT_AGENT));
    assert_eq!(text.matches(&by_nandi).count(), 1, "{text}");
}

#[tokio::test]
async fn each_ask_in_one_email_is_its_own_todo_and_a_reread_adds_none() {
    let state = AppState::for_tests(Vec::new());
    let read = || async {
        let mut added = Vec::new();
        for (n, text) in [
            (1, "Send the signed lease"),
            (2, "Book the move-out inspection"),
        ] {
            let todo = new(text, TodoKind::Do, None, Some(&format!("msg-7#{n}")));
            added.push(
                add(&state, todo, Actor::Agent(DEFAULT_AGENT.into()))
                    .await
                    .unwrap(),
            );
        }
        added
    };
    let first = read().await;
    assert!(first.iter().all(|a| !a.existing));
    assert_ne!(first[0].todo.task_id, first[1].todo.task_id);
    let again = read().await;
    assert!(
        again.iter().all(|a| a.existing),
        "the same numbers: no new ones"
    );
    assert_eq!(list(&state).await.unwrap().len(), 2);
}

#[test]
fn a_run_for_someone_else_gets_no_todo_tools() {
    use crate::model::SourceType;
    let store = crate::services::repo_store::RepoStore::for_tests(crate::db::Db::in_memory());
    crate::services::builtin_agents::seed(store.layout()).unwrap();
    let names = |source: Option<SourceType>| -> Vec<String> {
        tool::offered(&store, DEFAULT_AGENT, source)
            .into_iter()
            .map(|spec| spec.name)
            .collect()
    };
    // The user's own chat and the mail check keep the list
    for source in [Some(SourceType::User), Some(SourceType::Email), None] {
        assert_eq!(
            names(source),
            ["todo_add", "todo_list", "todo_done"],
            "{source:?}"
        );
    }
    // A task from a teammate or another Nandi (the gateway's ask-me and
    // assign-me run as delegations) never sees it
    assert!(names(Some(SourceType::Agent)).is_empty());
    assert!(!tool::allowed_in(Some(SourceType::Agent)));
    // Nor does any other agent
    assert!(tool::offered(&store, "writer", Some(SourceType::User)).is_empty());
}

#[tokio::test]
async fn the_user_adds_with_a_date_or_not_at_all() {
    let state = AppState::for_tests(Vec::new());
    let todo = add_manual(
        &state,
        " Renew the passport ".into(),
        TodoKind::Reminder,
        Some("2030-01-15".into()),
    )
    .await
    .unwrap();
    assert_eq!(todo.request, "Renew the passport");
    assert_eq!(todo.kind, TodoKind::Reminder);
    assert_eq!(todo.source, Some(TodoSource::Manual));
    assert!(todo.due_at.is_some());
    assert!(add_manual(&state, "  ".into(), TodoKind::Do, None)
        .await
        .is_err());
    assert!(
        add_manual(&state, "Pay".into(), TodoKind::Do, Some("whenever".into()))
            .await
            .is_err()
    );
    assert_eq!(list(&state).await.unwrap().len(), 1);
    // Not one of the to-dos: refused
    assert!(hand_off(&state, "t-nope".into(), DEFAULT_AGENT.into())
        .await
        .is_err());
}

#[tokio::test]
async fn nandi_keeps_the_list_with_its_tools() {
    let state = AppState::for_tests(Vec::new());
    let added = tool::call(
        &state,
        DEFAULT_AGENT,
        tool::ADD,
        &json!({"text": "Pay the power bill", "kind": "do", "due": "3 days",
                "source": "email", "source_ref": "m1", "related_ref": "inbox:approval:a1"}),
    )
    .await;
    assert!(!added.is_error, "{}", added.result);
    let added: Value = serde_json::from_str(&added.result).unwrap();
    assert_eq!(added["existing"], json!(false));
    assert_eq!(added["overdue"], json!(false));
    assert_eq!(added["related_ref"], json!("inbox:approval:a1"));
    let id = added["id"].as_str().unwrap().to_string();

    let listed = tool::call(&state, DEFAULT_AGENT, tool::LIST, &json!({})).await;
    let listed: Value = serde_json::from_str(&listed.result).unwrap();
    assert_eq!(listed["todos"][0]["id"], json!(id));
    assert_eq!(listed["todos"][0]["kind"], json!("do"));

    let done = tool::call(
        &state,
        DEFAULT_AGENT,
        tool::DONE,
        &json!({"id": id, "note": "paid"}),
    )
    .await;
    assert!(!done.is_error, "{}", done.result);
    assert_eq!(get(&state, &id).await.status, AssignmentStatus::Done);

    for bad in [
        json!({"text": "x", "kind": "do", "source": "manual"}),
        json!({"text": "x", "kind": "someday", "source": "chat"}),
        json!({"kind": "do", "source": "chat"}),
    ] {
        assert!(
            tool::call(&state, DEFAULT_AGENT, tool::ADD, &bad)
                .await
                .is_error,
            "{bad}"
        );
    }
    assert!(
        tool::call(&state, DEFAULT_AGENT, tool::DONE, &json!({}))
            .await
            .is_error
    );
}

#[test]
fn each_tool_has_a_spec_and_agents_name_only_their_sources() {
    for name in ["todo_add", "todo_list", "todo_done"] {
        assert!(tool::is_todo_tool(name));
        assert_eq!(tool::spec(name).unwrap().name, name);
    }
    assert!(!tool::is_todo_tool("work_open"));
    assert_eq!(tool::agent_source("email"), Some(TodoSource::Email));
    assert_eq!(tool::agent_source("manual"), None);
    assert_eq!(tool::agent_source("fax"), None);
}
