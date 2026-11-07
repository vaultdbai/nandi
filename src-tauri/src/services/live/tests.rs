//! The change feed with real folders: who made each change, and Refresh
//! with no watcher at all.

use std::fs;
use std::sync::Arc;

use super::testing::wait_until;
use super::*;
use crate::services::connectors::{self, folders as shared};
use crate::services::events::tests::RecordingSink;
use crate::services::notes::{self, NOTES_FOLDER_ID};

fn changes_of(state: &AppState, area: ChangeArea) -> Vec<LiveChange> {
    state
        .live()
        .feed()
        .recent()
        .into_iter()
        .filter(|c| c.area == area)
        .collect()
}

fn agent(name: &str) -> String {
    format!("---\nname: {name}\n---\nWrite.")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_works_with_the_watcher_off() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    let layout = state.store().layout().clone();
    fs::create_dir_all(&layout.agents_dir).unwrap();
    state.store().load().unwrap();
    assert!(!state.store().is_watching());

    // Written by an editor while nothing watches
    let skill = layout.skill_path("writer");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(&skill, agent("Writer")).unwrap();

    let report = refresh_from_disk(&state).await.unwrap();
    assert!(report.changed);
    let changes = changes_of(&state, ChangeArea::Agent);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].target_id.as_deref(), Some("writer"));
    assert_eq!(changes[0].label, "Writer");
    assert_eq!(changes[0].action, FileAction::Created);
    assert_eq!(changes[0].author, ChangeAuthor::Outside);
    assert!(recorder.names().contains(&"live://changed"));
    // Nothing more to find the second time
    assert!(!refresh_from_disk(&state).await.unwrap().changed);
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_apps_own_definition_writes_are_by_you() {
    let state = AppState::for_tests(vec![]);
    let layout = state.store().layout().clone();
    fs::create_dir_all(&layout.agents_dir).unwrap();
    state.store().load().unwrap();
    let skill = layout.skill_path("writer");
    crate::services::repos::fs_atomic::write_atomic(&skill, agent("Writer").as_bytes()).unwrap();
    state.store().commit(EntityKind::Agent, "writer").unwrap();
    let changes = changes_of(&state, ChangeArea::Agent);
    assert_eq!(changes[0].author, ChangeAuthor::You);
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_repo_watcher_reports_profile_and_pack_files() {
    let state = AppState::for_tests(vec![]);
    let layout = state.store().layout().clone();
    fs::create_dir_all(&layout.agents_dir).unwrap();
    state.store().load().unwrap();
    state.watch_repo();

    fs::write(layout.root.join("NANDI.md"), "---\nname: Me\n---\n").unwrap();
    fs::write(layout.installed_path(), "agents: []\n").unwrap();
    wait_until("the profile and packs changes", || {
        !changes_of(&state, ChangeArea::Profile).is_empty()
            && !changes_of(&state, ChangeArea::Packs).is_empty()
    })
    .await;
    let profile = &changes_of(&state, ChangeArea::Profile)[0];
    assert_eq!(profile.label, "NANDI.md");
    assert_eq!(profile.author, ChangeAuthor::Outside);
    state.shutdown();
}

/// A state whose My notes is a temporary folder agents may write, being
/// watched
async fn watched_notes() -> (AppState, tempfile::TempDir) {
    let state = AppState::for_tests(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    connectors::share_folder(
        &state,
        shared::SharedFolder {
            id: NOTES_FOLDER_ID.into(),
            path: tmp.path().to_string_lossy().to_string(),
            access: shared::Access::Write,
        },
    )
    .await
    .unwrap();
    // Opening My notes starts the watch
    notes::my_notes(&state).await.unwrap();
    (state, tmp)
}

fn note_change(state: &AppState, name: &str) -> Option<LiveChange> {
    changes_of(state, ChangeArea::Notes)
        .into_iter()
        .find(|c| c.target_id.as_deref() == Some(name))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notes_say_who_changed_them() {
    let (state, tmp) = watched_notes().await;

    // An editor outside Nandi
    fs::write(tmp.path().join("outside.md"), "# Outside").unwrap();
    wait_until("the outside edit", || {
        note_change(&state, "outside.md").is_some()
    })
    .await;
    assert_eq!(
        note_change(&state, "outside.md").unwrap().author,
        ChangeAuthor::Outside
    );

    // The My notes page
    notes::save_note(&state, "mine.md".into(), "# Mine".into())
        .await
        .unwrap();
    wait_until("the page's save", || {
        note_change(&state, "mine.md").is_some()
    })
    .await;
    assert_eq!(
        note_change(&state, "mine.md").unwrap().author,
        ChangeAuthor::You
    );

    // Chief of Staff, during its run
    {
        let conn = state.db().conn().unwrap();
        crate::db::tests::start_run(&conn, crate::services::builtin_agents::CHIEF_OF_STAFF, "r1");
    }
    fs::write(tmp.path().join("by-agent.md"), "# Kept").unwrap();
    wait_until("the agent's write", || {
        note_change(&state, "by-agent.md").is_some()
    })
    .await;
    assert!(matches!(
        note_change(&state, "by-agent.md").unwrap().author,
        ChangeAuthor::Agent { agent_id, .. } if agent_id == "chief-of-staff"
    ));
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_in_a_work_folder_name_the_agent_or_workflow_running() {
    let (state, _notes) = watched_notes().await;
    let work = state.nandi_dir().join("local").join("work");
    fs::create_dir_all(work.join("writer")).unwrap();
    fs::create_dir_all(work.join("analyst")).unwrap();
    {
        let conn = state.db().conn().unwrap();
        crate::db::tests::start_run(&conn, "analyst", "r-analyst");
        conn.execute(
            "INSERT INTO workflow_runs (id, workflow_id, status, started_at) VALUES ('wr1', 'weekly', 'running', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE runs SET source_type = 'workflow', source_id = 'wr1' WHERE id = 'r-analyst'",
            [],
        )
        .unwrap();
    }
    fs::write(work.join("writer").join("draft.md"), "x").unwrap();
    fs::write(work.join("analyst").join("report.csv"), "x").unwrap();
    let work_change = |agent: &str| {
        changes_of(&state, ChangeArea::Work)
            .into_iter()
            .find(|c| c.target_id.as_deref() == Some(agent))
    };
    wait_until("both work folders", || {
        work_change("writer").is_some() && work_change("analyst").is_some()
    })
    .await;
    let writer = work_change("writer").unwrap();
    assert_eq!(writer.paths, ["draft.md"]);
    assert_eq!(writer.author, ChangeAuthor::Outside);
    assert_eq!(
        work_change("analyst").unwrap().author,
        ChangeAuthor::Workflow {
            workflow_id: "weekly".into(),
            workflow_name: "weekly".into(),
        }
    );
    state.shutdown();
}

#[test]
fn active_runs_are_running_or_just_ended() {
    let db = crate::db::Db::in_memory();
    let conn = db.conn().unwrap();
    crate::db::tests::start_run(&conn, "writer", "going");
    crate::db::tests::start_run(&conn, "analyst", "ended-long-ago");
    conn.execute(
        "UPDATE runs SET status = 'completed', ended_at = 1 WHERE id = 'ended-long-ago'",
        [],
    )
    .unwrap();
    let runs = author::active_runs(&conn, crate::db::now_ms() - author::RECENT_RUN_MS).unwrap();
    assert_eq!(
        runs,
        [author::ActiveRun {
            agent_id: "writer".into(),
            workflow_id: None,
        }]
    );
}

#[test]
fn the_recent_list_keeps_the_latest_changes_newest_first() {
    let (bus, recorder) = crate::services::events::tests::bus();
    let feed = ChangeFeed::new(bus);
    for n in 0..RECENT_CHANGES + 5 {
        feed.file_changed(
            ChangeArea::Policy,
            Some(format!("p{n}")),
            Path::new("/nowhere/p.md"),
        );
    }
    let recent = feed.recent();
    assert_eq!(recent.len(), RECENT_CHANGES);
    assert_eq!(
        recent[0].target_id.as_deref(),
        Some(format!("p{}", RECENT_CHANGES + 4).as_str())
    );
    assert_eq!(recent[0].action, FileAction::Deleted);
    assert_eq!(recorder.names().len(), RECENT_CHANGES + 5);
}
