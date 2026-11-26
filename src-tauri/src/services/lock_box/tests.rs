//! The Lock box's rules, each at release time: the agent, the site, the
//! lock level, and the record. Shared helpers for the proxy's tests too.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Url;

use super::unlock::{SystemUnlock, UnlockOutcome};
use super::*;
use crate::db::ApprovalRow;
use crate::model::{ApprovalDecision, ApprovalStatus};
use crate::services::domain::lock_box::{KeyHeader, KeyKind};
use crate::services::domain::{Agent, AgentId, EntityKind};
use crate::services::events::RunContext;
use crate::services::repos::Repository;

/// The agent the helpers set up, and the one `events::tests::started` runs
pub(crate) const AGENT: &str = "writer";
pub(crate) const VALUE: &str = "ghp_lockbox_test_value_5f3a9c1e7b";

/// A core with the agent `writer` (allowed to reach api.github.com)
pub(crate) fn state_with_agent() -> AppState {
    let state = AppState::for_tests(Vec::new());
    let layout = state.store().layout().clone();
    std::fs::create_dir_all(&layout.root).unwrap();
    git2::Repository::init(&layout.root).unwrap();
    state.store().load().unwrap();
    for (id, name) in [(AGENT, "Writer"), ("reviewer", "Reviewer")] {
        let agent = Agent::from_agent_md(
            AgentId::parse(id).unwrap(),
            &format!(
                "---\nname: {name}\nnetwork:\n  allow:\n    - host: api.github.com\n---\nWrite.\n"
            ),
        )
        .unwrap();
        state.store().repos().agents.save(&agent).unwrap();
        state.store().commit(EntityKind::Agent, id).unwrap();
    }
    state
}

pub(crate) fn rules(hosts: &[&str], lock: LockLevel) -> KeyRules {
    KeyRules {
        kind: KeyKind::Token,
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
        agents: vec![AGENT.to_string()],
        lock,
        unlock_minutes: 15,
        header: Some(KeyHeader {
            name: "authorization".into(),
            prefix: "Bearer ".into(),
        }),
    }
}

pub(crate) async fn add_key(state: &AppState, name: &str, rules: KeyRules) {
    add(
        state,
        NewKey {
            name: name.into(),
            value: VALUE.into(),
            rules,
        },
    )
    .await
    .unwrap();
}

pub(crate) async fn start_run(state: &AppState) -> RunContext {
    crate::services::events::tests::started(state.bus(), &uuid::Uuid::new_v4().to_string()).await
}

/// The key approval the run is waiting on
pub(crate) async fn pending_key_approval(state: &AppState, run_id: &str) -> ApprovalRow {
    for _ in 0..500 {
        let id = run_id.to_string();
        let rows = state
            .db()
            .call(move |conn| db::pending_approvals_for_run(conn, &id))
            .await
            .unwrap();
        if let Some(row) = rows.into_iter().find(approvals::is_key_approval) {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no key approval was asked");
}

/// This month's decisions file
pub(crate) fn decisions(state: &AppState) -> String {
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    state
        .store()
        .repos()
        .decisions
        .read(&month)
        .unwrap()
        .unwrap_or_default()
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// The OS prompt, answered by the test
struct FakeUnlock {
    name: Option<&'static str>,
    outcome: Mutex<UnlockOutcome>,
    asked: AtomicUsize,
    messages: Mutex<Vec<String>>,
}

impl FakeUnlock {
    fn new(name: Option<&'static str>, outcome: UnlockOutcome) -> Arc<Self> {
        Arc::new(Self {
            name,
            outcome: Mutex::new(outcome),
            asked: AtomicUsize::new(0),
            messages: Mutex::default(),
        })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SystemUnlock for FakeUnlock {
    async fn available(&self) -> Option<&'static str> {
        self.name
    }

    async fn verify(&self, message: &str) -> UnlockOutcome {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.messages.lock().unwrap().push(message.to_string());
        *self.outcome.lock().unwrap()
    }
}

#[tokio::test]
async fn a_key_s_value_goes_into_the_keychain_and_never_comes_back_out() {
    let state = state_with_agent();
    add_key(
        &state,
        "github",
        rules(&["api.github.com"], LockLevel::Open),
    )
    .await;
    assert_eq!(
        state
            .secrets()
            .get("nandi.lockbox.github")
            .unwrap()
            .as_deref(),
        Some(VALUE)
    );
    let shown = serde_json::to_string(&overview(&state).await.unwrap()).unwrap();
    assert!(shown.contains("\"github\""), "{shown}");
    assert!(shown.contains("\"stored\":true"), "{shown}");
    assert!(!shown.contains(VALUE), "the page never gets the value");
    // A key of the same name isn't replaced
    let again = add(
        &state,
        NewKey {
            name: "GitHub".into(),
            value: "other".into(),
            rules: rules(&["api.github.com"], LockLevel::Open),
        },
    )
    .await;
    assert!(again.unwrap_err().contains("already a key"));
    assert_eq!(
        state
            .secrets()
            .get("nandi.lockbox.github")
            .unwrap()
            .as_deref(),
        Some(VALUE)
    );
    // Debug output of what came in hides it too
    let debug = format!(
        "{:?}",
        NewKey {
            name: "x".into(),
            value: VALUE.into(),
            rules: rules(&[], LockLevel::Ask),
        }
    );
    assert!(!debug.contains(VALUE), "{debug}");
    state.shutdown();
}

#[tokio::test]
async fn a_key_needs_a_value_known_agents_and_valid_rules() {
    let state = state_with_agent();
    let new = |value: &str, rules: KeyRules| NewKey {
        name: "k".into(),
        value: value.into(),
        rules,
    };
    let bad = [
        new("", rules(&["api.github.com"], LockLevel::Ask)),
        new("a\nb", rules(&["api.github.com"], LockLevel::Ask)),
        new("v", rules(&["*.com"], LockLevel::Ask)),
        new(
            "v",
            KeyRules {
                agents: vec!["nobody-here".into()],
                ..rules(&["api.github.com"], LockLevel::Ask)
            },
        ),
    ];
    for key in bad {
        assert!(add(&state, key).await.is_err());
    }
    assert_eq!(state.secrets().get("nandi.lockbox.k").unwrap(), None);
    assert!(overview(&state).await.unwrap().keys.is_empty());
    state.shutdown();
}

#[tokio::test]
async fn deleting_a_key_removes_its_keychain_entry() {
    let state = state_with_agent();
    add_key(
        &state,
        "github",
        rules(&["api.github.com"], LockLevel::Open),
    )
    .await;
    delete(&state, "github".into()).await.unwrap();
    assert_eq!(state.secrets().get("nandi.lockbox.github").unwrap(), None);
    assert!(overview(&state).await.unwrap().keys.is_empty());
    let gone = release(&state, AGENT, "github", &url("https://api.github.com/")).await;
    assert!(gone.unwrap_err().contains("no key of that name"));
    state.shutdown();
}

#[tokio::test]
async fn an_open_key_is_released_only_on_its_sites() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    add_key(
        &state,
        "github",
        rules(&["api.github.com"], LockLevel::Open),
    )
    .await;

    let released = release(&state, AGENT, "github", &url("https://api.github.com/user"))
        .await
        .unwrap();
    assert_eq!(released.header, "authorization");
    assert_eq!(released.header_value, format!("Bearer {VALUE}"));
    assert!(!format!("{released:?}").contains(VALUE));

    for wrong in [
        "https://api.github.com.evil.com/",
        "https://api-github.com/",
        "https://evil.com/api.github.com",
        "https://xn--pi-github-6ve.com/",
        "https://аpi.github.com/",
        "https://api.github.com:8443/",
        "http://api.github.com/",
        "https://user:pw@api.github.com/",
        "https://140.82.112.6/",
        "https://uploads.github.com/",
    ] {
        let Ok(target) = Url::parse(wrong) else {
            continue;
        };
        let refused = release(&state, AGENT, "github", &target).await;
        let message = refused.unwrap_err();
        assert!(message.contains("wasn't released"), "{wrong}: {message}");
        assert!(!message.contains(VALUE));
    }

    let record = decisions(&state);
    assert!(
        record.contains("key 'github' for agent 'writer' to api.github.com"),
        "{record}"
    );
    assert!(record.contains("- **Decision:** released"), "{record}");
    assert!(record.contains(&format!("`{}`", run.run_id)), "{record}");
    assert!(record.contains("lock: open"), "{record}");
    assert!(record.contains("Lock box rule (open)"), "{record}");
    assert!(
        record.contains("to api.github.com.evil.com") && record.contains("site not bound"),
        "a refusal is on the record too: {record}"
    );
    assert!(!record.contains(VALUE), "the record never holds the value");

    let shown = overview(&state).await.unwrap();
    assert_eq!(shown.keys[0].last_used_by.as_deref(), Some(AGENT));
    assert_eq!(
        shown.keys[0].last_used_host.as_deref(),
        Some("api.github.com")
    );
    state.shutdown();
}

#[tokio::test]
async fn a_subdomain_rule_covers_its_subdomains_only() {
    let state = state_with_agent();
    add_key(
        &state,
        "corp",
        rules(&["*.corp.example.com"], LockLevel::Open),
    )
    .await;
    assert!(
        release(&state, AGENT, "corp", &url("https://api.corp.example.com/"))
            .await
            .is_ok()
    );
    for wrong in [
        "https://corp.example.com/",
        "https://evilcorp.example.com/",
        "https://api.corp.example.com.evil.com/",
    ] {
        assert!(
            release(&state, AGENT, "corp", &url(wrong)).await.is_err(),
            "{wrong}"
        );
    }
    state.shutdown();
}

#[tokio::test]
async fn a_key_with_no_site_or_no_header_is_never_released() {
    let state = state_with_agent();
    add_key(&state, "nowhere", rules(&[], LockLevel::Open)).await;
    let refused = release(&state, AGENT, "nowhere", &url("https://api.github.com/")).await;
    assert!(refused.unwrap_err().contains("isn't one of them"));

    add_key(
        &state,
        "headless",
        KeyRules {
            header: None,
            ..rules(&["api.github.com"], LockLevel::Open)
        },
    )
    .await;
    let refused = release(&state, AGENT, "headless", &url("https://api.github.com/")).await;
    assert!(refused.unwrap_err().contains("no header"));
    state.shutdown();
}

#[tokio::test]
async fn an_agent_not_allowed_gets_nothing() {
    let state = state_with_agent();
    add_key(
        &state,
        "github",
        rules(&["api.github.com"], LockLevel::Open),
    )
    .await;
    for agent in ["reviewer", "browser:writer", "connector:github", "Writer"] {
        let refused = release(&state, agent, "github", &url("https://api.github.com/")).await;
        assert!(
            refused.unwrap_err().contains("isn't allowed"),
            "{agent} got the key"
        );
    }
    // Revoked: the next release is refused
    let mut revoked = rules(&["api.github.com"], LockLevel::Open);
    revoked.agents.clear();
    update(&state, "github".into(), revoked).await.unwrap();
    assert!(
        release(&state, AGENT, "github", &url("https://api.github.com/"))
            .await
            .is_err()
    );
    assert!(decisions(&state).contains("agent not allowed"));
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_each_time_waits_for_the_card_and_a_no_releases_nothing() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    add_key(&state, "github", rules(&["api.github.com"], LockLevel::Ask)).await;
    let asking = || {
        let state = state.clone();
        tokio::spawn(async move {
            release(
                &state,
                AGENT,
                "github",
                &url("https://api.github.com/repos"),
            )
            .await
        })
    };

    // Approved
    let waiting = asking();
    let card = pending_key_approval(&state, &run.run_id).await;
    assert_eq!(card.tool, KEY_TOOL);
    assert_eq!(card.args["key"], "github");
    assert_eq!(card.args["agent"], AGENT);
    assert_eq!(card.args["host"], "api.github.com");
    assert!(
        !card.args.to_string().contains(VALUE),
        "the card never has the value"
    );
    assert!(!waiting.is_finished(), "it waits for the person");
    crate::services::chat::approve_action(
        &state,
        None,
        card.id.clone(),
        ApprovalDecision::Approved,
    )
    .await
    .unwrap();
    let released = waiting.await.unwrap().unwrap();
    assert_eq!(released.header_value, format!("Bearer {VALUE}"));
    let id = card.id.clone();
    let closed = state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed.status, ApprovalStatus::Approved);

    // Asked again for the next use, and declined
    let waiting = asking();
    let card = pending_key_approval(&state, &run.run_id).await;
    crate::services::chat::approve_action(
        &state,
        None,
        card.id.clone(),
        ApprovalDecision::Rejected,
    )
    .await
    .unwrap();
    let refused = waiting.await.unwrap().unwrap_err();
    assert!(refused.contains("didn't allow"), "{refused}");
    assert!(!refused.contains(VALUE));

    // Declined in this run: refused without another card
    let refused = asking().await.unwrap().unwrap_err();
    assert!(refused.contains("earlier in this task"), "{refused}");
    let id = run.run_id.clone();
    let open = state
        .db()
        .call(move |conn| db::pending_approvals_for_run(conn, &id))
        .await
        .unwrap();
    assert!(open.is_empty(), "no new card");

    let record = decisions(&state);
    assert!(record.contains("declined on its card"), "{record}");
    assert!(record.contains("approved on its card"), "{record}");
    assert!(record.contains("lock: ask each time"), "{record}");
    assert!(
        !record.contains("- **Item:** approval"),
        "one entry per release, not a second one for its card: {record}"
    );
    assert!(!record.contains(VALUE));
    state.shutdown();
}

#[tokio::test]
async fn ask_each_time_without_a_run_releases_nothing() {
    let state = state_with_agent();
    add_key(&state, "github", rules(&["api.github.com"], LockLevel::Ask)).await;
    let refused = release(&state, AGENT, "github", &url("https://api.github.com/")).await;
    assert!(refused.unwrap_err().contains("no run to ask in"));
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_card_nobody_answers_releases_nothing() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    let summary = {
        let id = run.run_id.clone();
        state
            .db()
            .call(move |conn| db::get_run(conn, &id))
            .await
            .unwrap()
            .unwrap()
    };
    let answer = approvals::ask(
        &state,
        &summary,
        serde_json::json!({ "key": "github" }),
        Duration::from_millis(50),
    )
    .await;
    assert_eq!(answer, approvals::Answer::Declined(None));
    let id = run.run_id.clone();
    let open = state
        .db()
        .call(move |conn| db::pending_approvals_for_run(conn, &id))
        .await
        .unwrap();
    assert!(open.is_empty(), "the card closed as expired");
    state.shutdown();
}

#[tokio::test]
async fn locked_waits_for_the_os_unlock_and_locks_again_after_its_time() {
    let state = state_with_agent();
    start_run(&state).await;
    let unlock = FakeUnlock::new(Some("Windows Hello"), UnlockOutcome::Verified);
    state.lock_box().set_unlocker(unlock.clone());
    add_key(
        &state,
        "bank",
        rules(&["api.github.com"], LockLevel::Locked),
    )
    .await;
    let to = url("https://api.github.com/");

    assert!(release(&state, AGENT, "bank", &to).await.is_ok());
    assert_eq!(unlock.asked(), 1, "the OS prompt asked");
    let message = unlock.messages.lock().unwrap()[0].clone();
    assert!(
        message.contains("'bank'") && message.contains("api.github.com"),
        "{message}"
    );
    assert!(message.contains("Writer"), "{message}");
    assert!(!message.contains(VALUE));
    // Unlocked for its time: no prompt
    assert!(release(&state, AGENT, "bank", &to).await.is_ok());
    assert_eq!(unlock.asked(), 1);
    assert!(overview(&state).await.unwrap().keys[0]
        .unlocked_until
        .is_some());

    // Past its 15 minutes it is locked again
    state.lock_box().advance(15 * 60_000 + 1);
    assert!(overview(&state).await.unwrap().keys[0]
        .unlocked_until
        .is_none());
    *unlock.outcome.lock().unwrap() = UnlockOutcome::Declined;
    let refused = release(&state, AGENT, "bank", &to).await;
    assert!(refused.unwrap_err().contains("didn't unlock"));
    assert_eq!(unlock.asked(), 2);

    let record = decisions(&state);
    assert!(
        record.contains("lock: locked; unlocked for 15 minutes"),
        "{record}"
    );
    assert!(record.contains("with Windows Hello"), "{record}");
    assert!(record.contains("Windows Hello not unlocked"), "{record}");
    assert!(!record.contains(VALUE));
    state.shutdown();
}

#[tokio::test]
async fn changing_a_key_s_rules_locks_it_again() {
    let state = state_with_agent();
    let unlock = FakeUnlock::new(Some("Windows Hello"), UnlockOutcome::Verified);
    state.lock_box().set_unlocker(unlock.clone());
    add_key(
        &state,
        "bank",
        rules(&["api.github.com"], LockLevel::Locked),
    )
    .await;
    unlock_now(&state, "bank".into()).await.unwrap();
    assert!(overview(&state).await.unwrap().keys[0]
        .unlocked_until
        .is_some());
    update(
        &state,
        "bank".into(),
        rules(&["api.github.com"], LockLevel::Locked),
    )
    .await
    .unwrap();
    assert!(overview(&state).await.unwrap().keys[0]
        .unlocked_until
        .is_none());
    // And Lock now does too
    unlock_now(&state, "bank".into()).await.unwrap();
    lock_now(&state, "bank".into()).await.unwrap();
    assert!(overview(&state).await.unwrap().keys[0]
        .unlocked_until
        .is_none());
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_without_an_os_unlock_asks_each_time_and_says_so() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    let unlock = FakeUnlock::new(None, UnlockOutcome::Unavailable);
    state.lock_box().set_unlocker(unlock.clone());
    add_key(
        &state,
        "bank",
        rules(&["api.github.com"], LockLevel::Locked),
    )
    .await;
    assert_eq!(overview(&state).await.unwrap().system_unlock, None);
    assert!(unlock_now(&state, "bank".into())
        .await
        .unwrap_err()
        .contains("asks each time instead"));

    let asking = tokio::spawn({
        let state = state.clone();
        async move { release(&state, AGENT, "bank", &url("https://api.github.com/")).await }
    });
    let card = pending_key_approval(&state, &run.run_id).await;
    assert_eq!(card.args["noSystemUnlock"], true);
    assert_eq!(unlock.asked(), 0, "no weaker unlock was invented");
    crate::services::chat::approve_action(&state, None, card.id, ApprovalDecision::Approved)
        .await
        .unwrap();
    assert!(asking.await.unwrap().is_ok());
    assert!(decisions(&state).contains("no system unlock on this computer"));
    state.shutdown();
}

#[tokio::test]
async fn a_key_s_approval_is_never_given_from_a_phone() {
    let reason = crate::services::own_devices::desktop_only::reason(
        KEY_TOOL,
        &serde_json::json!({ "key": "github" }),
    );
    assert!(reason.is_some());
}

#[tokio::test]
async fn the_prompt_names_usable_keys_and_never_a_value() {
    let state = state_with_agent();
    add_key(&state, "github", rules(&["api.github.com"], LockLevel::Ask)).await;
    add_key(&state, "nowhere", rules(&[], LockLevel::Ask)).await;
    let text = prompt(&state, AGENT).await;
    assert!(text.contains("`github`: api.github.com"), "{text}");
    assert!(
        !text.contains("nowhere"),
        "a key bound nowhere isn't offered"
    );
    assert!(!text.contains(VALUE));
    assert_eq!(prompt(&state, "reviewer").await, "");
    state.shutdown();
}

#[tokio::test]
async fn a_declined_os_unlock_isnt_raised_again_in_the_run() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    let unlock = FakeUnlock::new(Some("Windows Hello"), UnlockOutcome::Declined);
    state.lock_box().set_unlocker(unlock.clone());
    add_key(
        &state,
        "bank",
        rules(&["api.github.com"], LockLevel::Locked),
    )
    .await;
    let to = url("https://api.github.com/");

    let first = release(&state, AGENT, "bank", &to).await.unwrap_err();
    assert!(first.contains("didn't unlock"), "{first}");
    for _ in 0..3 {
        let again = release(&state, AGENT, "bank", &to).await.unwrap_err();
        assert!(again.contains("earlier in this task"), "{again}");
    }
    assert_eq!(unlock.asked(), 1, "the prompt was raised once");

    // A new run may ask again
    state
        .bus()
        .turn_done(&run, crate::model::RunStatus::Completed, None)
        .await
        .unwrap();
    start_run(&state).await;
    *unlock.outcome.lock().unwrap() = UnlockOutcome::Verified;
    assert!(release(&state, AGENT, "bank", &to).await.is_ok());
    assert_eq!(unlock.asked(), 2);
    state.shutdown();
}

#[tokio::test]
async fn a_repeated_refusal_is_recorded_once_and_counted_at_run_end() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    add_key(
        &state,
        "github",
        rules(&["api.github.com"], LockLevel::Open),
    )
    .await;
    let evil = url("https://evil.example.com/");
    for _ in 0..5 {
        assert!(release(&state, AGENT, "github", &evil).await.is_err());
    }
    // Another refusal is its own entry
    assert!(release(
        &state,
        "reviewer",
        "github",
        &url("https://api.github.com/")
    )
    .await
    .is_err());
    let subject = "key 'github' for agent 'writer' to evil.example.com";
    assert_eq!(decisions(&state).matches(subject).count(), 1);
    assert_eq!(decisions(&state).matches("agent 'reviewer'").count(), 1);

    state
        .bus()
        .turn_done(&run, crate::model::RunStatus::Completed, None)
        .await
        .unwrap();
    let record = decisions(&state);
    assert_eq!(record.matches(subject).count(), 2, "{record}");
    assert!(
        record.contains("site not bound; the same refusal 4 more times in this run"),
        "{record}"
    );
    assert!(
        !record.contains("1 more time"),
        "no summary for a single refusal"
    );
    state.shutdown();
}

#[tokio::test]
async fn a_key_that_asks_first_is_never_unlocked_for_an_ask_from_a_phone() {
    let state = state_with_agent();
    let run = start_run(&state).await;
    let id = run.run_id.clone();
    state
        .db()
        .call(move |conn| {
            conn.execute(
                "UPDATE runs SET source_type = 'user', source_id = 'device.1' WHERE id = ?1",
                [&id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    add_key(&state, "github", rules(&["api.github.com"], LockLevel::Ask)).await;
    let refused = release(&state, AGENT, "github", &url("https://api.github.com/"))
        .await
        .unwrap_err();
    assert!(refused.contains("phone"), "{refused}");

    // A key left open still works on its site
    add_key(&state, "open", rules(&["api.github.com"], LockLevel::Open)).await;
    assert!(
        release(&state, AGENT, "open", &url("https://api.github.com/"))
            .await
            .is_ok()
    );
}
