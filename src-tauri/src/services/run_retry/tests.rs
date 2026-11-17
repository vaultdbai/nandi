use rusqlite::params;
use serde_json::json;

use super::*;
use crate::db::Db;
use crate::model::RunStatus;
use crate::services::events::NandiEvent;
use crate::services::orchestrator::middleware::AgentTurnAborted;
use crate::services::run_waiter::newest_run;

const MINUTE: i64 = 60 * 1000;

/// A run of `agent` from `source` that failed with `error`; returns when
/// it ended
fn failed_run(conn: &Connection, id: &str, source: SourceType, error: &str) -> i64 {
    db::tests::start_run(conn, "writer", id);
    conn.execute(
        "UPDATE runs SET source_type = ?2, input = ?3 WHERE id = ?1",
        params![id, source.as_str(), format!("Do the {id} thing")],
    )
    .unwrap();
    db::set_run_status(conn, id, RunStatus::Failed, Some(error))
        .unwrap()
        .unwrap()
        .ended_at
        .unwrap() as i64
}

/// `retry` tries `of` again as `attempt`, and ends `status`
fn retried(conn: &Connection, of: &str, retry: &str, attempt: u32, status: RunStatus) {
    db::tests::start_run(conn, "writer", retry);
    conn.execute(
        "UPDATE runs SET source_type = 'cron', input = 'with a note' WHERE id = ?1",
        [retry],
    )
    .unwrap();
    store::link(conn, retry, of, attempt).unwrap();
    db::set_run_status(conn, retry, status, Some("boom")).unwrap();
}

#[test]
fn retries_wait_longer_each_time_and_stop_after_the_last() {
    assert_eq!(due_at(1_000, 1), Some(1_000 + 2 * MINUTE));
    assert_eq!(due_at(1_000, 2), Some(1_000 + 10 * MINUTE));
    assert_eq!(due_at(1_000, MAX_ATTEMPTS), None);
    assert_eq!(due_at(1_000, 0), None);
}

#[test]
fn a_retry_asks_the_first_input_again_and_says_what_went_wrong() {
    let text = retry_input(
        "Morning digest.",
        2,
        Some("  Interrupted: the app was closed "),
        false,
    );
    assert!(text.starts_with("Morning digest.\n\n(Nandi: Attempt 2 of 3."));
    assert!(text.contains("The last attempt failed: Interrupted: the app was closed."));
    assert!(text.contains("check what was already done before you repeat anything"));

    let asked = retry_input("Morning digest.", 4, None, true);
    assert!(asked.contains("The user asked to try this again"));
    assert!(asked.contains("no reason was recorded"));

    let long = "x".repeat(2_000);
    assert!(retry_input("Hi", 2, Some(&long), false).len() < 500);
}

#[test]
fn only_nandis_own_failures_nothing_retries_yet_are_open() {
    let db = Db::in_memory();
    let conn = db.conn().unwrap();
    failed_run(&conn, "digest", SourceType::Cron, "model down");
    failed_run(&conn, "mail", SourceType::Email, "model down");
    failed_run(&conn, "chat", SourceType::User, "model down");
    failed_run(&conn, "capped", SourceType::Cron, "Cost cap reached");
    conn.execute(
        "INSERT INTO run_events (run_id, kind, payload, created_at) VALUES ('capped', ?1, '{}', 0)",
        [AgentTurnAborted::NAME],
    )
    .unwrap();

    let open = |conn: &Connection| -> Vec<String> {
        store::open_failures(conn, 0, RETRIED)
            .unwrap()
            .into_iter()
            .map(|f| f.run.id)
            .collect()
    };
    // The user's chat is theirs; a cap is the user's limit
    assert_eq!(open(&conn), ["digest", "mail"]);
    // The Inbox still shows the capped one
    let latest: Vec<String> = store::latest_failures(&conn, 0, RETRIED)
        .unwrap()
        .into_iter()
        .map(|f| f.run.id)
        .collect();
    assert_eq!(latest, ["digest", "mail", "capped"]);

    retried(&conn, "digest", "digest-2", 2, RunStatus::Failed);
    let failures = store::open_failures(&conn, 0, RETRIED).unwrap();
    let ids: Vec<&str> = failures.iter().map(|f| f.run.id.as_str()).collect();
    assert_eq!(ids, ["mail", "digest-2"]);
    // The second attempt asks what the first one did, not its own note
    let second = &failures[1];
    assert_eq!(second.attempt, 2);
    assert_eq!(second.first_input, "Do the digest thing");
}

#[test]
fn the_newest_run_of_a_line_is_its_latest_attempt() {
    let db = Db::in_memory();
    let conn = db.conn().unwrap();
    failed_run(&conn, "digest", SourceType::Cron, "model down");
    assert_eq!(newest_run(&conn, "digest").unwrap().unwrap().id, "digest");
    retried(&conn, "digest", "digest-2", 2, RunStatus::Failed);
    retried(&conn, "digest-2", "digest-3", 3, RunStatus::Completed);
    let newest = newest_run(&conn, "digest").unwrap().unwrap();
    assert_eq!(
        (newest.id.as_str(), newest.status),
        ("digest-3", RunStatus::Completed)
    );
}

#[test]
fn work_waits_on_a_retry_only_while_nandi_will_make_one() {
    let db = Db::in_memory();
    let conn = db.conn().unwrap();
    let ended = failed_run(&conn, "digest", SourceType::Cron, "model down");
    failed_run(&conn, "chat", SourceType::User, "model down");

    let now = ended + MINUTE;
    assert_eq!(
        next_try(&conn, "digest", now).unwrap(),
        Some(ended + 2 * MINUTE)
    );
    // Overdue (the app was closed): as soon as the sweep runs
    let later = ended + 30 * MINUTE;
    assert_eq!(next_try(&conn, "digest", later).unwrap(), Some(later));
    // Too old, someone else's, already tried again, or the last attempt
    let old = ended + ms(RETRY_WINDOW) + MINUTE;
    assert_eq!(next_try(&conn, "digest", old).unwrap(), None);
    assert_eq!(next_try(&conn, "chat", now).unwrap(), None);
    retried(&conn, "digest", "digest-2", 2, RunStatus::Failed);
    assert_eq!(next_try(&conn, "digest", now).unwrap(), None);
    retried(&conn, "digest-2", "digest-3", 3, RunStatus::Failed);
    assert_eq!(next_try(&conn, "digest-3", now).unwrap(), None);
}

async fn app() -> AppState {
    let state = AppState::for_tests(vec![]);
    state
        .db()
        .call(|conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            let mut script = crate::engine::default_script();
            script["tokenDelayMs"] = json!(0);
            db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
        })
        .await
        .unwrap();
    state
}

async fn run(state: &AppState, id: &str) -> crate::model::RunSummary {
    let id = id.to_string();
    state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn the_sweep_tries_a_failed_digest_again_once_it_is_due() {
    let state = app().await;
    let ended = state
        .db()
        .call(|conn| {
            let ended = failed_run(conn, "digest", SourceType::Cron, "Interrupted: the app was closed");
            conn.execute(
                "UPDATE runs SET source_id = 'morning-digest', source_label = 'Morning digest' WHERE id = 'digest'",
                [],
            )?;
            Ok(ended)
        })
        .await
        .unwrap();

    // Not yet due
    assert!(sweep(&state, ended + MINUTE).await.is_empty());

    let started = sweep(&state, ended + 3 * MINUTE).await;
    assert_eq!(started.len(), 1);
    let retry = run(&state, &started[0]).await;
    let original = run(&state, "digest").await;
    // Same conversation and source, the first input and Nandi's note
    assert_eq!(retry.conversation_id, original.conversation_id);
    assert_eq!(retry.source_type, SourceType::Cron);
    assert_eq!(retry.source_id.as_deref(), Some("morning-digest"));
    assert!(retry.input.starts_with("Do the digest thing"));
    assert!(retry.input.contains("Attempt 2 of 3"));
    let id = started[0].clone();
    let (link, newest) = state
        .db()
        .call(move |conn| {
            let link: (Option<String>, u32) = conn.query_row(
                "SELECT retry_of, attempt FROM runs WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((link, newest_run(conn, "digest")?.unwrap().id))
        })
        .await
        .unwrap();
    assert_eq!(link, (Some("digest".to_string()), 2));
    assert_eq!(newest, started[0]);

    // Tried once: never again for the same failure, by the sweep or by hand
    assert!(sweep(&state, ended + 30 * MINUTE).await.is_empty());
    let again = retry_run(&state, "digest".into()).await.unwrap_err();
    assert!(again.contains("already tried again"), "{again}");
    state.shutdown();
}

#[tokio::test]
async fn retry_tries_any_failed_run_again_but_nothing_else() {
    let state = app().await;
    state
        .db()
        .call(|conn| {
            failed_run(conn, "chat", SourceType::User, "model down");
            db::tests::start_run(conn, "writer", "fine");
            db::set_run_status(conn, "fine", RunStatus::Completed, None)?;
            Ok(())
        })
        .await
        .unwrap();

    let id = retry_run(&state, "chat".into()).await.unwrap();
    let retry = run(&state, &id).await;
    assert!(retry.input.contains("The user asked to try this again"));
    assert_eq!(retry.source_type, SourceType::User);

    let error = retry_run(&state, "fine".into()).await.unwrap_err();
    assert!(error.contains("didn't fail"), "{error}");
    assert!(retry_run(&state, "missing".into()).await.is_err());
    state.shutdown();
}
