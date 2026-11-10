//! Tests for getting to know you (`services::knowme`)

use std::time::Duration;

use nandi_protocol::GroupRole;
use serde_json::json;

use super::*;
use crate::db;
use crate::services::connectors::{self, folders};
use crate::services::domain::group::{CopyMember, GroupCopy, POLICY_TEMPLATE};
use crate::services::inbox::{thread, tool::PROPOSE};
use crate::services::notes::NOTES_FOLDER_ID;

const FAMILY: &str = "g_0123456789abcdef";

/// A state whose My notes is a temporary folder
async fn with_notes() -> (AppState, tempfile::TempDir) {
    let state = AppState::for_tests(Vec::new());
    let tmp = tempfile::tempdir().unwrap();
    connectors::share_folder(
        &state,
        folders::SharedFolder {
            id: NOTES_FOLDER_ID.into(),
            path: tmp.path().to_string_lossy().to_string(),
            access: folders::Access::Write,
        },
    )
    .await
    .unwrap();
    (state, tmp)
}

fn me(state: &AppState) -> String {
    crate::services::network::local_instance_id(state.db()).unwrap()
}

/// This Nandi is in the group "Family" with `others`
fn in_family(state: &AppState, others: &[&str]) {
    let mut members = vec![CopyMember {
        instance_id: me(state),
        role: GroupRole::Admin,
    }];
    members.extend(others.iter().map(|id| CopyMember {
        instance_id: id.to_string(),
        role: GroupRole::Member,
    }));
    state
        .store()
        .repos()
        .groups
        .save(&GroupCopy {
            group_id: FAMILY.into(),
            name: "Family".into(),
            version: 1,
            author: me(state),
            signature: "sig".into(),
            updated_at: 1.0,
            members,
            text: POLICY_TEMPLATE.to_string(),
        })
        .unwrap();
}

fn read(tmp: &tempfile::TempDir, name: &str) -> Option<String> {
    std::fs::read_to_string(tmp.path().join(name)).ok()
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

fn manifest(state: &AppState) -> crate::services::gateway::GatewayManifest {
    state.gateway().unwrap().manifest().unwrap()
}

/// The proposal the Chief of Staff makes in the chat
fn proposed() -> serde_json::Value {
    json!({
        "summary": "Scrum master for two teams; a parent",
        "responsibilities": [
            {"text": "Sprint planning", "kind": "work"},
            {"text": "Hiring interviews", "kind": "work", "onCard": false},
            {"text": "School runs", "kind": "personal", "onCard": true},
            {"text": "Mum's appointments", "kind": "personal", "shareWithGroup": FAMILY}
        ],
        "canHelpWith": "Sprint ceremonies and Jira questions.",
        "notes": [
            {"text": "Two teams of six, two-week sprints"},
            {"text": "Children aged 4 and 7", "private": true}
        ],
        "suggest": [{"name": "Sprint review prep", "why": "Prepares the review each sprint"}]
    })
}

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn chief_approvals(state: &AppState) -> Vec<crate::model::PendingApproval> {
    state
        .db()
        .call(|conn| db::pending_approvals(conn, Some(CHIEF_OF_STAFF)))
        .await
        .unwrap()
}

/// The Chief of Staff proposes in the know-me chat (never marked as
/// needing approval: the gate makes it wait anyway); returns its card
async fn chief_proposes(
    state: &AppState,
    args: serde_json::Value,
) -> crate::model::PendingApproval {
    crate::services::builtin_agents::seed(state.store().layout()).unwrap();
    state.store().load().unwrap();
    let script = json!({
        "agent": [
            { "text": "Here's what I'd change.", "toolCalls": [{
                "tool": PROPOSE, "args": args, "result": "", "requiresApproval": false
            }] },
            { "text": "Saved." }
        ],
        "studio": [],
        "tokenDelayMs": 0
    });
    state
        .db()
        .call(move |conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
        })
        .await
        .unwrap();
    thread::open(state, THREAD_ID.into()).await.unwrap();
    thread::send(
        state,
        THREAD_ID.into(),
        "I'm a scrum master, and I have two kids".into(),
        None,
    )
    .await
    .unwrap();
    wait_for("the proposal card", || async {
        !chief_approvals(state).await.is_empty()
    })
    .await;
    chief_approvals(state).await.remove(0)
}

#[tokio::test]
async fn the_first_to_do_comes_after_onboarding_and_skip_puts_it_away() {
    let state = AppState::for_tests(Vec::new());
    assert!(!status(&state).await.unwrap().todo_open);
    crate::services::auth::set_onboarding_step(&state, crate::services::auth::OnboardingStep::Done)
        .await
        .unwrap();
    let open = status(&state).await.unwrap();
    assert!(open.todo_open);
    assert!(!open.recheck);
    assert_eq!(open.thread_id, "inbox:knowMe:you");

    let skipped = todo(&state, KnowMeTodoAction::Skip).await.unwrap();
    assert!(!skipped.todo_open);
    // Reading it again doesn't bring it back, nor does onboarding again
    crate::services::auth::set_onboarding_step(&state, crate::services::auth::OnboardingStep::Done)
        .await
        .unwrap();
    assert!(!status(&state).await.unwrap().todo_open);

    // A month on, it's back once
    let mut cadence = load(&state).await.unwrap();
    cadence.next_at = Some(db::now_ms() - 1);
    cadence.shown_at = Some(db::now_ms() - cadence::MONTH_MS - 1);
    save(&state, Cadence::default(), cadence).await.unwrap();
    assert!(status(&state).await.unwrap().todo_open);
}

/// The whole path: the proposal waits on its card and writes nothing; the
/// person edits it and confirms; then NANDI.md gets the work items on the
/// card, My notes the rest, the decision is recorded and the to-do closes
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proposal_writes_nothing_until_confirmed_then_writes_where_each_part_belongs() {
    let (state, tmp) = with_notes().await;
    in_family(&state, &["desktop-mum"]);
    start(&state).await.unwrap();
    let card = chief_proposes(&state, proposed()).await;
    assert_eq!(card.tool, PROPOSE);

    // Nothing written before the confirm, and the card isn't an Inbox line
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(manifest(&state).responsibilities.is_empty());
    assert!(read(&tmp, "about-me.md").is_none());
    assert!(read(&tmp, "private/about-me.md").is_none());
    assert!(!decisions(&state).contains("knowMe"));
    assert!(crate::services::chat::list_pending_approvals(&state)
        .await
        .unwrap()
        .is_empty());

    // Edited on the card: the interviews go on the card after all
    let mut edited: KnowMeProposal = serde_json::from_value(proposed()).unwrap();
    edited.responsibilities[1].on_card = true;
    confirm(&state, card.id.clone(), edited).await.unwrap();
    wait_for("the decision", || async {
        decisions(&state).contains("knowMe")
    })
    .await;

    let card_items = manifest(&state).responsibilities;
    assert_eq!(card_items, ["Sprint planning", "Hiring interviews"]);
    assert!(manifest(&state)
        .body
        .contains("## What I can help with\n\nSprint ceremonies and Jira questions."));
    let nandi_md = std::fs::read_to_string(state.store().layout().root.join("NANDI.md")).unwrap();
    for personal in ["School", "Mum", "Children"] {
        assert!(!nandi_md.contains(personal), "{nandi_md}");
    }

    let about = read(&tmp, "about-me.md").unwrap();
    assert!(
        about.contains("- Two teams of six, two-week sprints"),
        "{about}"
    );
    assert!(
        !about.contains("School") && !about.contains("Children"),
        "{about}"
    );
    let private = read(&tmp, "private/about-me.md").unwrap();
    assert!(private.contains("- School runs"), "{private}");
    assert!(
        private.contains("- Mum's appointments (shared with Family)"),
        "{private}"
    );
    assert!(private.contains("- Children aged 4 and 7"), "{private}");
    let group = read(&tmp, &format!("private/groups/{FAMILY}.md")).unwrap();
    assert!(group.contains("- Mum's appointments"), "{group}");
    assert!(!group.contains("School"), "{group}");

    let text = decisions(&state);
    assert!(
        text.contains("- **Item:** knowMe `you` (Getting to know you)"),
        "{text}"
    );
    assert!(text.contains("- **Chat:** `inbox:knowMe:you`"), "{text}");
    assert!(
        text.contains("on the card: Sprint planning, Hiring interviews"),
        "{text}"
    );
    assert!(text.contains("shared with Family: 1"), "{text}");
    for personal in ["School", "Mum", "Children"] {
        assert!(!text.contains(personal), "{text}");
    }

    let after = status(&state).await.unwrap();
    assert!(!after.todo_open);
    assert!(after.recheck);
    assert_eq!(after.recent.len(), 1);
    assert_eq!(after.recent[0].kind, "knowMe");
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_personal_item_is_shared_only_with_a_group_i_am_in() {
    let (state, tmp) = with_notes().await;
    let mut args = proposed();
    args["responsibilities"][3]["shareWithGroup"] = json!("g_somebody_elses");
    let card = chief_proposes(&state, args).await;
    let mut edited: KnowMeProposal = serde_json::from_value(proposed()).unwrap();
    edited.responsibilities[3].share_with_group = Some("g_somebody_elses".into());
    let refused = confirm(&state, card.id.clone(), edited).await.unwrap_err();
    assert!(refused.contains("isn't a group you're in"), "{refused}");
    // Still waiting, nothing written
    assert_eq!(chief_approvals(&state).await.len(), 1);
    assert!(read(&tmp, "private/about-me.md").is_none());
    assert!(manifest(&state).responsibilities.is_empty());
    // The model's own arguments are checked the same way on approve
    crate::services::chat::approve_action(
        &state,
        None,
        card.id,
        crate::model::ApprovalDecision::Approved,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(read(&tmp, "private/about-me.md").is_none());
    assert!(!decisions(&state).contains("knowMe"));
    state.shutdown();
}

#[tokio::test]
async fn only_a_member_of_the_group_gets_what_is_shared_with_it() {
    let (state, tmp) = with_notes().await;
    in_family(&state, &["desktop-mum"]);
    std::fs::create_dir_all(tmp.path().join("private/groups")).unwrap();
    std::fs::write(
        tmp.path().join(format!("private/groups/{FAMILY}.md")),
        "# Shared with Family\n\n## Personal\n\n- Mum's appointments\n",
    )
    .unwrap();
    let mine = me(&state);
    for (id, requester) in [("t-mum", "desktop-mum"), ("t-bob", "desktop-bob")] {
        let mine = mine.clone();
        state
            .db()
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO tasks (id, requester_instance, requester_agent,                      provider_instance, provider_agent, capability, state, created_at,                      updated_at) VALUES (?1, ?2, 'gateway', ?3, 'gateway', 'ask-me',                      'working', 1, 1)",
                    rusqlite::params![id, requester, mine],
                )
            })
            .await
            .unwrap();
    }

    // Through the gateway: the hand-off answers the member's task
    let shared = sharing::prompt_for_task(&state, "t-mum-gw").await.unwrap();
    assert!(
        shared.contains("Shared with the group \"Family\""),
        "{shared}"
    );
    assert!(shared.contains("- Mum's appointments"), "{shared}");
    assert!(sharing::prompt_for_task(&state, "t-mum").await.is_some());
    // Someone outside the group, or no such task: nothing
    assert!(sharing::prompt_for_task(&state, "t-bob").await.is_none());
    assert!(sharing::prompt_for_task(&state, "nope").await.is_none());
}
