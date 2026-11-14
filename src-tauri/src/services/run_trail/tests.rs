//! The run trail through the bus: what a commit appends, and a `state.db`
//! rebuilt from it

use super::*;
use crate::model::{ApprovalDecision, ApprovalStatus};
use crate::services::events::tests::{started, RecordingSink};
use crate::services::events::{EventBus, RunContext};

/// A bus on an in-memory database, writing its trail in a fresh folder
fn bus_with_trail() -> (EventBus, RunTrail, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let trail = RunTrail::in_local_dir(dir.path());
    let bus = EventBus::new(db::Db::in_memory(), Arc::new(RecordingSink::default()))
        .with_trail(trail.clone());
    (bus, trail, dir)
}

fn entries(trail: &RunTrail) -> Vec<TrailEntry> {
    let (entries, unreadable) = read(trail.path().unwrap()).unwrap();
    assert!(unreadable.is_empty(), "unreadable lines {unreadable:?}");
    entries
}

/// Every `run_events` row, in ID order
async fn run_events(db: &db::Db) -> Vec<RunEventRow> {
    db.call(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, run_id, kind, payload, created_at FROM run_events ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(RunEventRow {
                id: row.get(0)?,
                run_id: row.get(1)?,
                kind: row.get(2)?,
                payload: db::json_col(row, 3)?,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect()
    })
    .await
    .unwrap()
}

async fn decided(db: &db::Db, approval_id: &str) -> DecidedApproval {
    let id = approval_id.to_string();
    db.call(move |conn| db::decided_approval(conn, &id))
        .await
        .unwrap()
        .unwrap()
}

/// A shell call that waits on an approval, decided by Ada. Returns the
/// approval's ID.
async fn approved_call(bus: &EventBus, ctx: &RunContext, call_id: &str) -> String {
    let call = db::PlannedCall {
        id: call_id.into(),
        tool: "shell".into(),
        args: serde_json::json!({ "cmd": "ls" }),
        requires_approval: true,
    };
    let approval_id = bus.approval_request(ctx, &call).await.unwrap();
    let (id, run_id) = (approval_id.clone(), ctx.run_id.clone());
    let approval = bus
        .db
        .call(move |conn| {
            db::set_approval_decider(conn, &id, "Ada")?;
            db::approval_for_call(conn, &run_id, &call.id)
        })
        .await
        .unwrap()
        .unwrap();
    bus.approval_resolved(ctx, approval, ApprovalDecision::Approved)
        .await
        .unwrap();
    approval_id
}

#[tokio::test]
async fn each_commit_is_in_the_trail_when_it_returns() {
    let (bus, trail, _dir) = bus_with_trail();
    let ctx = started(&bus, "r1").await;
    // The run's first events are there as soon as the run has started
    let written: Vec<TrailEntry> = run_events(&bus.db)
        .await
        .into_iter()
        .map(TrailEntry::RunEvent)
        .collect();
    assert!(!written.is_empty());
    assert_eq!(entries(&trail), written);

    let approval_id = approved_call(&bus, &ctx, "c1").await;
    let decision = decided(&bus.db, &approval_id).await;
    assert_eq!(decision.approval.status, ApprovalStatus::Approved);
    assert_eq!(decision.approval.decided_by.as_deref(), Some("Ada"));
    assert!(decision.resolved_at.is_some());

    // Every run event, each once, and the decision as the database has it
    let trail_entries = entries(&trail);
    let trail_events: Vec<RunEventRow> = trail_entries
        .iter()
        .filter_map(|entry| match entry {
            TrailEntry::RunEvent(row) => Some(row.clone()),
            TrailEntry::ApprovalDecision(_) => None,
        })
        .collect();
    assert_eq!(trail_events, run_events(&bus.db).await);
    let decisions: Vec<&TrailEntry> = trail_entries
        .iter()
        .filter(|entry| matches!(entry, TrailEntry::ApprovalDecision(_)))
        .collect();
    assert_eq!(decisions, [&TrailEntry::ApprovalDecision(decision)]);
    assert_eq!(
        audit::verify(trail.path().unwrap()).unwrap(),
        ChainCheck::Intact {
            lines: trail_entries.len() as u32
        }
    );
}

#[tokio::test]
async fn streamed_tokens_never_touch_the_trail() {
    let (bus, trail, _dir) = bus_with_trail();
    let ctx = started(&bus, "r1").await;
    let before = entries(&trail).len();
    for text in ["Hel", "lo", " there"] {
        bus.token(&ctx, "m1", 0, text).await.unwrap();
    }
    assert_eq!(entries(&trail).len(), before);
}

#[tokio::test]
async fn an_expired_approval_is_a_decision_in_the_trail() {
    let (bus, trail, _dir) = bus_with_trail();
    let ctx = started(&bus, "r1").await;
    let call = db::PlannedCall {
        id: "c1".into(),
        tool: "shell".into(),
        args: serde_json::json!({}),
        requires_approval: true,
    };
    let approval_id = bus.approval_request(&ctx, &call).await.unwrap();
    // Nobody decided: the run ends and its approval expires
    bus.turn_done(&ctx, crate::model::RunStatus::Cancelled, None)
        .await
        .unwrap();
    let expired = decided(&bus.db, &approval_id).await;
    assert_eq!(expired.approval.status, ApprovalStatus::Expired);
    assert!(entries(&trail).contains(&TrailEntry::ApprovalDecision(expired)));
}

#[tokio::test]
async fn a_new_state_db_gets_its_rows_back_from_the_trail() {
    let (bus, trail, _dir) = bus_with_trail();
    let ctx = started(&bus, "r1").await;
    let approval_id = approved_call(&bus, &ctx, "c1").await;
    let (events, decision) = (
        run_events(&bus.db).await,
        decided(&bus.db, &approval_id).await,
    );

    // `state.db` deleted: a new, empty one
    let fresh = db::Db::in_memory();
    let restored = trail
        .restore(&mut fresh.conn().unwrap())
        .unwrap()
        .expect("an empty database is restored");
    assert!(restored.is_clean(), "{restored:?}");
    assert_eq!(restored.run_events as usize, events.len());
    assert_eq!(restored.approval_decisions, 1);
    assert_eq!(run_events(&fresh).await, events);
    assert_eq!(decided(&fresh, &approval_id).await, decision);

    // A database that has its rows is never touched again
    assert_eq!(trail.restore(&mut fresh.conn().unwrap()).unwrap(), None);
    // New events continue after the restored IDs
    let new_bus =
        EventBus::new(fresh.clone(), Arc::new(RecordingSink::default())).with_trail(trail);
    started(&new_bus, "r2").await;
    let after = run_events(&fresh).await;
    assert!(after.len() > events.len());
    assert!(after[events.len()].id > events.last().unwrap().id);
}

#[tokio::test]
async fn a_broken_trail_is_reported_and_what_can_be_read_is_restored() {
    let (bus, trail, _dir) = bus_with_trail();
    let ctx = started(&bus, "r1").await;
    approved_call(&bus, &ctx, "c1").await;
    let path = trail.path().unwrap().to_path_buf();
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let count = lines.len() as u32;
    // The run's message edited: line 3 no longer follows it. The last
    // line garbled: it can't be read at all.
    assert!(lines[1].contains("draft"), "{}", lines[1]);
    lines[1] = lines[1].replace("draft", "edited");
    *lines.last_mut().unwrap() = "not json".to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    let fresh = db::Db::in_memory();
    let restored = trail.restore(&mut fresh.conn().unwrap()).unwrap().unwrap();
    assert!(!restored.is_clean());
    assert_eq!(restored.unreadable, [count]);
    assert!(
        matches!(restored.chain, ChainCheck::Broken { line: 3, .. }),
        "{:?}",
        restored.chain
    );
    assert_eq!(
        restored.run_events + restored.approval_decisions,
        count - 1,
        "every readable line is restored"
    );
}

#[test]
fn nothing_is_restored_without_a_trail() {
    let fresh = db::Db::in_memory();
    let dir = tempfile::tempdir().unwrap();
    let missing = RunTrail::in_local_dir(dir.path());
    assert_eq!(missing.restore(&mut fresh.conn().unwrap()).unwrap(), None);
    assert_eq!(
        RunTrail::off().restore(&mut fresh.conn().unwrap()).unwrap(),
        None
    );
    // Writing with no trail is a no-op
    RunTrail::off().append(vec![]);
}

#[tokio::test]
async fn the_trail_reports_its_health_like_an_audit_log() {
    /// What is done to the trail before its health is asked
    enum Change {
        Nothing,
        EditLine2,
        RemoveTheFile,
    }
    // (the change, the chain the health reports, given the trail's lines)
    let cases: [(Change, fn(u32) -> ChainCheck); 3] = [
        (Change::Nothing, |lines| ChainCheck::Intact { lines }),
        (Change::EditLine2, |_| ChainCheck::Broken {
            line: 3,
            reason: String::new(),
        }),
        (Change::RemoveTheFile, |_| ChainCheck::Intact { lines: 0 }),
    ];
    for (change, expected) in cases {
        let (bus, trail, _dir) = bus_with_trail();
        let ctx = started(&bus, "r1").await;
        approved_call(&bus, &ctx, "c1").await;
        let path = trail.path().unwrap().to_path_buf();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines = text.lines().count() as u32;
        match change {
            Change::Nothing => {}
            Change::EditLine2 => {
                let mut edited: Vec<String> = text.lines().map(str::to_string).collect();
                edited[1] = edited[1].replace("draft", "edited");
                std::fs::write(
                    &path,
                    format!(
                        "{}
",
                        edited.join(
                            "
"
                        )
                    ),
                )
                .unwrap();
            }
            Change::RemoveTheFile => std::fs::remove_file(&path).unwrap(),
        }

        let health = bus.trail().health().await.unwrap();
        assert_eq!(health.lost_entries, 0);
        match (health.chain, expected(lines)) {
            // The reason is the chain's own words; the line is what counts
            (ChainCheck::Broken { line, .. }, ChainCheck::Broken { line: want, .. }) => {
                assert_eq!(line, want)
            }
            (chain, want) => assert_eq!(chain, want),
        }
    }
    // No trail at all reads as intact and empty
    let health = RunTrail::off().health().await.unwrap();
    assert_eq!(health.chain, ChainCheck::Intact { lines: 0 });
}
