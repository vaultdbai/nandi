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
    let refused = tools::call(&bus, &writer, "pilot_opendesk__click", &json!({})).await;
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

    let done = tools::call(&bus, &writer, "pilot_opendesk__click", &json!({"x": 1})).await;
    assert!(!done.is_error, "{}", done.result);
    assert_eq!(done.result, "click done");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Another agent with the same declaration but no consent is refused
    let other = agent("other", "opendesk");
    assert!(
        tools::call(&bus, &other, "pilot_opendesk__click", &json!({}))
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
        tools::call(&bus, &writer, "pilot_opendesk__click", &json!({}))
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
