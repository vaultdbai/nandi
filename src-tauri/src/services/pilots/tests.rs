//! Integration tests: real loopback servers standing in for pilots.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use super::*;
use crate::services::domain::{Agent, AgentId};
use crate::services::events::tests::{bus, RecordingSink};

/// Serves `/health` (and a minimal MCP server at `/mcp`) on `port` until
/// the returned handle is aborted.
async fn fake_pilot(port: u16) -> (tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/mcp",
            post(mcp).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap_or_else(|e| panic!("port {port} is busy: {e}"));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (server, calls)
}

/// Just enough JSON-RPC for initialize, tools/list, and tools/call.
async fn mcp(State(calls): State<Arc<AtomicUsize>>, Json(request): Json<Value>) -> Response {
    let Some(id) = request.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response(); // a notification
    };
    let result = match request["method"].as_str() {
        Some("initialize") => json!({
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-pilot", "version": "1.0.0"}
        }),
        Some("tools/list") => json!({"tools": [
            {"name": "click", "description": "Click", "inputSchema": {"type": "object"}},
            {"name": "screenshot", "description": "Look", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true}}
        ]}),
        Some("tools/call") => {
            calls.fetch_add(1, Ordering::SeqCst);
            let tool = request["params"]["name"].as_str().unwrap_or_default();
            json!({"content": [{"type": "text", "text": format!("{tool} done")}], "isError": false})
        }
        _ => json!({}),
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn definition(id: &str) -> PilotDefinition {
    catalog::all().into_iter().find(|p| p.id == id).unwrap()
}

async fn wait_for(what: &str, mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..200 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

fn changes(sink: &RecordingSink) -> Vec<Value> {
    sink.take()
        .into_iter()
        .filter(|(name, _)| *name == "pilot://changed")
        .map(|(_, payload)| payload)
        .collect()
}

#[tokio::test]
async fn adopts_a_running_server_and_notices_when_it_dies() {
    // A pilot already answering on its port: nothing is spawned
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let (server, _) = fake_pilot(port).await;
    let (bus, sink) = bus();
    let registry = Arc::new(PilotRegistry::new());
    let pilot = PilotDefinition {
        id: "adopted".into(),
        mcp_port: port,
        ..definition("opendesk")
    };

    let pid = start_definition(&bus, &registry, &pilot).await.unwrap();
    assert_eq!(pid, 0, "adopted, not spawned");
    let status = status_of(&bus.db, Some(&registry), "adopted")
        .await
        .unwrap();
    assert!(status.running && status.healthy, "{status:?}");
    assert_eq!(status.pid, None);
    assert!(changes(&sink)
        .iter()
        .any(|c| c["status"]["healthy"] == true));

    // The server goes away: unhealthy within one health interval
    server.abort();
    let _ = server.await;
    wait_for("unhealthy", async || {
        !status_of(&bus.db, Some(&registry), "adopted")
            .await
            .unwrap()
            .healthy
    })
    .await;
    // The event follows the status; on a busy machine, a moment later
    wait_for("the unhealthy event", async || {
        changes(&sink)
            .iter()
            .any(|c| c["pilotId"] == "adopted" && c["status"]["healthy"] == false)
    })
    .await;

    stop_definition(&bus, &registry, &pilot).await.unwrap();
    let stopped = status_of(&bus.db, Some(&registry), "adopted")
        .await
        .unwrap();
    assert!(!stopped.running && !stopped.healthy);
}

#[tokio::test]
async fn spawns_and_stops_a_pilot_process() {
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let pilot = PilotDefinition {
        id: "sleeper".into(),
        start_command: if cfg!(windows) {
            "ping -n 30 127.0.0.1".into()
        } else {
            "sleep 30".into()
        },
        mcp_port: port,
        ..definition("opendesk")
    };
    let (bus, _) = bus();
    let registry = Arc::new(PilotRegistry::new());

    let pid = start_definition(&bus, &registry, &pilot).await.unwrap();
    assert!(pid > 0);
    // Starting again returns the same process
    assert_eq!(
        start_definition(&bus, &registry, &pilot).await.unwrap(),
        pid
    );
    let status = status_of(&bus.db, Some(&registry), "sleeper")
        .await
        .unwrap();
    assert!(status.running && !status.healthy);
    assert_eq!(status.pid, Some(pid));

    stop_definition(&bus, &registry, &pilot).await.unwrap();
    assert!(!registry.is_running("sleeper"));
    assert!(!status_of(&bus.db, None, "sleeper").await.unwrap().running);
}

#[tokio::test]
async fn install_records_success_and_emits_output() {
    let (bus, sink) = bus();
    let pilot = PilotDefinition {
        install_command: "echo pilot-installed".into(),
        ..definition("opendesk")
    };
    install_definition(&bus, &pilot).await.unwrap();
    let status = status_of(&bus.db, None, "opendesk").await.unwrap();
    assert!(status.installed && !status.running);
    let names = sink.names();
    assert!(names.contains(&"pilot://install_output"));
    assert!(names.contains(&"pilot://changed"));

    let failing = PilotDefinition {
        id: "broken".into(),
        install_command: "exit 1".into(),
        ..pilot
    };
    assert!(install_definition(&bus, &failing).await.is_err());
    assert!(!status_of(&bus.db, None, "broken").await.unwrap().installed);
}

fn agent(id: &str, pilots: &str) -> Agent {
    Agent::from_skill_md(
        AgentId::parse(id).unwrap(),
        &format!("---\nname: {id}\npilots: [{pilots}]\n---\nDo things"),
    )
    .unwrap()
}

#[test]
fn reads_the_skill_md_pilot_block() {
    assert_eq!(
        tools::declared(&agent("a", "opendesk, other-pilot")),
        ["opendesk", "other-pilot"]
    );
    assert!(tools::declared(&agent("b", "")).is_empty());
    // Duplicates and bad IDs are rejected when skill.md is parsed
    let id = AgentId::parse("c").unwrap();
    assert!(Agent::from_skill_md(id.clone(), "---\npilots: [x, x]\n---\n").is_err());
    assert!(Agent::from_skill_md(id, "---\npilots: [\".hidden\"]\n---\n").is_err());
}

#[tokio::test]
async fn a_browser_not_installed_offers_one_setup_tool() {
    use super::detect::tests::FakeProbe;
    let (bus, sink) = bus();
    let shopper = agent("shopper", "browser");

    let set = tools::for_agent_with(&bus, &shopper, &FakeProbe(None)).await;
    let names: Vec<&str> = set.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["pilot_browser__setup"]);
    assert!(set.requires_approval("pilot_browser__setup", &json!({})));
    assert!(
        set.warnings[0].contains("isn't installed"),
        "{:?}",
        set.warnings
    );
    assert!(set.consent_needed.is_empty());
    assert!(!status_of(&bus.db, None, "browser").await.unwrap().installed);

    // Found on this computer (installed by hand): recorded and announced,
    // and the next thing missing is the user's consent
    let set = tools::for_agent_with(&bus, &shopper, &FakeProbe(Some("npm global"))).await;
    assert!(set.tools.is_empty());
    let pilot = catalog::find("browser").unwrap();
    assert_eq!(
        set.consent_needed,
        [("browser".to_string(), pilot.capabilities.clone())]
    );
    assert!(status_of(&bus.db, None, "browser").await.unwrap().installed);
    assert!(changes(&sink)
        .iter()
        .any(|c| c["pilotId"] == "browser" && c["status"]["installed"] == true));
}

#[tokio::test]
async fn an_approved_setup_installs_once() {
    use super::detect::tests::FakeProbe;
    let (bus, _sink) = bus();
    let pilot = PilotDefinition {
        id: "browser-setup-test".into(),
        install_command: "echo installed".into(),
        ..definition("browser")
    };
    let (text, is_error) = tools::setup(&bus, &FakeProbe(None), &pilot).await.unwrap();
    assert!(!is_error && text.starts_with("Installed Browser"), "{text}");
    assert!(
        status_of(&bus.db, None, "browser-setup-test")
            .await
            .unwrap()
            .installed
    );
    // Already there: nothing runs again
    let broken = PilotDefinition {
        install_command: "exit 7".into(),
        ..pilot
    };
    let (text, is_error) = tools::setup(&bus, &FakeProbe(None), &broken).await.unwrap();
    assert!(!is_error && text.contains("is installed"), "{text}");

    // A failed install is the agent's to report, not a loop
    let failing = PilotDefinition {
        id: "browser-setup-fails".into(),
        ..broken
    };
    let (text, is_error) = tools::setup(&bus, &FakeProbe(None), &failing)
        .await
        .unwrap();
    assert!(is_error && text.contains("code 7"), "{text}");
}

#[tokio::test]
async fn pilot_tools_route_through_approval_and_consent() {
    // opendesk's port, so the catalog definition points at the fake
    let (server, calls) = fake_pilot(17899).await;
    let (bus, sink) = bus();
    {
        let conn = bus.db.conn().unwrap();
        store::save_installed(&conn, "opendesk", true).unwrap();
        store::save_running(&conn, "opendesk", None, 17899).unwrap();
        store::save_healthy(&conn, "opendesk", true).unwrap();
    }
    let writer = agent("writer", "opendesk");
    let pilot = catalog::find("opendesk").unwrap();

    // No consent yet: no tools, a warning, and calls are refused
    let set = tools::for_agent(&bus, &writer).await;
    assert!(set.tools.is_empty());
    assert!(set.warnings[0].contains("consent"), "{:?}", set.warnings);
    assert_eq!(
        set.consent_needed,
        [("opendesk".to_string(), pilot.capabilities.clone())]
    );
    let refused = tools::call(&bus, &writer, "c1", "pilot_opendesk__click", &json!({})).await;
    assert!(refused.is_error && refused.result.contains("consent"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    for capability in &pilot.capabilities {
        grant(&bus.db, "writer", &pilot, capability).await.unwrap();
    }
    assert!(grant(&bus.db, "writer", &pilot, "system_settings")
        .await
        .is_err());

    let set = tools::for_agent(&bus, &writer).await;
    assert!(set.warnings.is_empty(), "{:?}", set.warnings);
    assert!(set.consent_needed.is_empty());
    let names: Vec<&str> = set.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["pilot_opendesk__click", "pilot_opendesk__screenshot"]
    );
    // Sensitive tools go through the approval flow; read-only ones don't
    assert!(set.requires_approval("pilot_opendesk__click", &json!({})));
    assert!(!set.requires_approval("pilot_opendesk__screenshot", &json!({})));

    let done = tools::call(
        &bus,
        &writer,
        "c1",
        "pilot_opendesk__click",
        &json!({"x": 1}),
    )
    .await;
    assert!(!done.is_error, "{}", done.result);
    assert_eq!(done.result, "click done");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Another agent with the same declaration but no consent is refused
    let other = agent("other", "opendesk");
    assert!(
        tools::call(&bus, &other, "c1", "pilot_opendesk__click", &json!({}))
            .await
            .is_error
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Revoking one capability takes the pilot away again
    let first = pilot.capabilities[0].clone();
    revoke(&bus.db, "writer", &pilot, &first).await.unwrap();
    let set = tools::for_agent(&bus, &writer).await;
    assert!(set.tools.is_empty());
    assert_eq!(
        set.consent_needed,
        [("opendesk".to_string(), vec![first.clone()])]
    );
    assert!(
        tools::call(&bus, &writer, "c1", "pilot_opendesk__click", &json!({}))
            .await
            .is_error
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    grant(&bus.db, "writer", &pilot, &first).await.unwrap();

    // The server dies: the next listing marks it unhealthy and skips it
    server.abort();
    let _ = server.await;
    let set = tools::for_agent(&bus, &writer).await;
    assert!(set.tools.is_empty());
    assert!(set.warnings[0].contains("unhealthy"), "{:?}", set.warnings);
    assert!(
        !store::load(&bus.db.conn().unwrap(), "opendesk")
            .unwrap()
            .healthy
    );
    assert!(sink.names().contains(&"pilot://changed"));
}

/// A hand-over waits for the person a day, not an hour; Changed it approves
/// with the final text kept for the call's result; Skip rejects it
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hand_over_waits_for_the_person_and_takes_their_answer() {
    use crate::db;
    use crate::services::chat;
    use crate::services::domain::EntityKind;
    use crate::services::repos::Repository;
    use handover::{HandOverAnswer, HandOverOutcome};

    let state = crate::services::AppState::for_tests(vec![]);
    let root = state.store().layout().root.clone();
    std::fs::create_dir_all(&root).unwrap();
    git2::Repository::init(&root).unwrap();
    state.store().load().unwrap();
    let poster = Agent::from_skill_md(
        AgentId::parse("poster").unwrap(),
        // Without the run plan's rules: this is about the hand-over itself
        "---\nname: Poster\npilots: [browser]\nmiddlewares: [verify_on_stop]\n---\nYou post.\n",
    )
    .unwrap();
    state.store().repos().agents.save(&poster).unwrap();
    state.store().commit(EntityKind::Agent, "poster").unwrap();
    let hand_over = |draft: &str| {
        json!({
            "text": "Ready for you.",
            "toolCalls": [{
                "tool": "pilot_browser__browser_hand_over",
                "args": { "site": "LinkedIn", "summary": "Launch post", "draft": draft },
                "result": "answered",
                "requiresApproval": true
            }]
        })
    };
    let script = json!({
        "agent": [hand_over("Hello"), { "text": "Noted." }],
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
    let pending = || async {
        state
            .db()
            .call(|conn| db::pending_approvals(conn, None))
            .await
            .unwrap()
    };
    let wait = |what: &'static str| async move {
        for _ in 0..500 {
            if !pending().await.is_empty() {
                return pending().await.remove(0);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never saw {what}");
    };

    // The orchestrator closes an answered approval a moment later
    let closed = |id: String| {
        let state = state.clone();
        async move {
            for _ in 0..500 {
                let id = id.clone();
                let approval = state
                    .db()
                    .call(move |conn| db::get_approval(conn, &id))
                    .await
                    .unwrap()
                    .unwrap();
                if approval.status != crate::model::ApprovalStatus::Pending {
                    return approval;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("the approval never closed");
        }
    };

    chat::send_message(&state, "poster".into(), "Post it".into(), vec![], None)
        .await
        .unwrap();
    let first = wait("the hand-over").await;
    assert_eq!(first.tool, "pilot_browser__browser_hand_over");
    let waits = first.expires_at.unwrap() - first.created_at;
    assert!(waits > 23.0 * 3_600_000.0, "a day, not an hour: {waits}");
    // An approval that isn't a hand-over can't be answered as one, and a
    // changed answer needs its text
    let changed = |text: Option<&str>| HandOverAnswer {
        outcome: HandOverOutcome::Changed,
        final_text: text.map(str::to_string),
    };
    assert!(
        chat::answer_hand_over(&state, first.id.clone(), changed(None))
            .await
            .is_err()
    );
    chat::answer_hand_over(&state, first.id.clone(), changed(Some("Hello, world")))
        .await
        .unwrap();
    let decided = closed(first.id.clone()).await;
    assert_eq!(decided.status, crate::model::ApprovalStatus::Approved);
    // The canned engine answers the call itself; the answer waits for a
    // real browser call, which reads it once
    assert_eq!(
        handover::take(&decided.tool_call_id),
        Some(changed(Some("Hello, world")))
    );

    chat::send_message(&state, "poster".into(), "And this one".into(), vec![], None)
        .await
        .unwrap();
    let second = wait("the second hand-over").await;
    chat::answer_hand_over(
        &state,
        second.id.clone(),
        HandOverAnswer {
            outcome: HandOverOutcome::Skipped,
            final_text: None,
        },
    )
    .await
    .unwrap();
    let skipped = closed(second.id.clone()).await;
    assert_eq!(skipped.status, crate::model::ApprovalStatus::Rejected);
    state.shutdown();
}

/// Notices any log line that contains `needle` (the test logger is global:
/// it's installed once, and other tests' lines pass through it too)
struct LeakLogger {
    needle: &'static str,
    seen: std::sync::atomic::AtomicBool,
}

impl log::Log for LeakLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if record.args().to_string().contains(self.needle) {
            self.seen.store(true, Ordering::SeqCst);
        }
    }
    fn flush(&self) {}
}

/// A site secret the user adds: its name goes into skill.md and its value
/// into the keychain, and nowhere else: not a file, the Permissions view,
/// an event, the prompt, or a log line
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_site_secret_s_value_stays_in_the_keychain() {
    use crate::services::domain::EntityKind;
    use crate::services::events::EventSink;
    use crate::services::repos::Repository;

    const VALUE: &str = "s3cr3t-value-7731";
    static LOGGER: LeakLogger = LeakLogger {
        needle: VALUE,
        seen: std::sync::atomic::AtomicBool::new(false),
    };
    let watching = log::set_logger(&LOGGER).is_ok();
    if watching {
        log::set_max_level(log::LevelFilter::Trace);
    }

    let sink = Arc::new(RecordingSink::default());
    let state = crate::services::AppState::for_tests(vec![sink.clone() as Arc<dyn EventSink>]);
    let layout = state.store().layout().clone();
    std::fs::create_dir_all(&layout.root).unwrap();
    git2::Repository::init(&layout.root).unwrap();
    state.store().load().unwrap();
    let shopper = Agent::from_skill_md(
        AgentId::parse("shopper").unwrap(),
        "---
name: Shopper
pilots:
  - id: browser
    allow: [interact]
---
Shop.
",
    )
    .unwrap();
    state.store().repos().agents.save(&shopper).unwrap();
    state.store().commit(EntityKind::Agent, "shopper").unwrap();

    add_pilot_secret(&state, "shopper", "browser", "ACME_PASSWORD", VALUE)
        .await
        .unwrap();

    // skill.md names it; the keychain has the value under that entry
    let skill = std::fs::read_to_string(layout.skill_path("shopper")).unwrap();
    assert!(
        skill.contains("ACME_PASSWORD: nandi.pilot.shopper.acme-password"),
        "{skill}"
    );
    assert!(
        skill.contains("interact"),
        "the rest of the pilot entry stays"
    );
    assert_eq!(
        state
            .secrets()
            .get("nandi.pilot.shopper.acme-password")
            .unwrap()
            .as_deref(),
        Some(VALUE)
    );
    // Set, by name only
    let permissions = agent_pilot_permissions(&state, "shopper").await.unwrap();
    assert_eq!(
        permissions[0].secrets,
        [PilotSecretStatus {
            name: "ACME_PASSWORD".into(),
            is_set: true
        }]
    );
    assert!(permissions[0].browser);
    // Replaced, then removed, through the name
    set_pilot_secret(&state, "shopper", "browser", "ACME_PASSWORD", "other-7731")
        .await
        .unwrap();
    set_pilot_secret(&state, "shopper", "browser", "ACME_PASSWORD", "")
        .await
        .unwrap();
    assert!(!agent_pilot_permissions(&state, "shopper").await.unwrap()[0].secrets[0].is_set);
    add_pilot_secret(&state, "shopper", "browser", "ACME_PASSWORD", VALUE)
        .await
        .unwrap();

    // What the model is told: the name, never the value
    let agent = state.store().agent("shopper").unwrap();
    let note = tools::fill_note(agent.capabilities().pilot_options("browser").unwrap());
    assert!(
        note.contains("ACME_PASSWORD") && !note.contains(VALUE),
        "{note}"
    );
    // Nor in the Permissions view, an event, a file, or the logs
    let view = serde_json::to_string(&permissions).unwrap();
    assert!(!view.contains(VALUE));
    let events = serde_json::to_string(&*sink.0.lock().unwrap()).unwrap();
    assert!(!events.contains(VALUE));
    let mut dirs = vec![layout.root.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                assert!(!text.contains(VALUE), "{}", path.display());
            }
        }
    }
    if watching {
        assert!(
            !LOGGER.seen.load(Ordering::SeqCst),
            "a log line had the value"
        );
    }

    // Unknown pilots, empty values and lower-case names are refused
    for (pilot, name, value) in [
        ("opendesk", "X", VALUE),
        ("browser", "X", ""),
        ("browser", "acme", VALUE),
        ("browser-headless", "X", VALUE),
    ] {
        assert!(
            add_pilot_secret(&state, "shopper", pilot, name, value)
                .await
                .is_err(),
            "{pilot} {name}"
        );
    }
}
