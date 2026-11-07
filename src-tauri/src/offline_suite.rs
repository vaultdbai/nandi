//! The offline suite (`pnpm run test:offline`): Nandi works with no relay.
//!
//! Each test boots the app core in-process with a fresh home (temporary
//! folder, in-memory database) and drives it over the HTTP API, the same
//! surface `pnpm smoke:api` and the web UI use. No relay is configured, or
//! the build's default relay points at a port nothing listens on. No model
//! is called (the canned engine) and nothing goes to the internet.
//!
//! What it proves (AGENTS.md, "Works Without the Relay"):
//! - every screen's read endpoints answer
//! - an agent runs, asks for approval, and finishes once approved
//! - a workflow runs
//! - a starter pack installs from a local folder
//! - person-to-person endpoints answer the typed `RelayUnavailable`
//! - a build with an unreachable default relay and join code behaves the same

use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::http::auth::WebToken;
use crate::services::relay::{BuiltInRelay, RELAY_UNAVAILABLE, RELAY_URL_ENV};
use crate::services::AppState;

const TOKEN: &str = "offline-token-0123456789abcdef0123456789";

/// How long any one read may take: nothing may wait on a relay
const READ_BUDGET: Duration = Duration::from_secs(5);

/// The parameterless GET routes behind every screen (`scripts/api-smoke.ps1`),
/// minus the ones that read another service on purpose: the starter-pack
/// registry on GitHub (`marketplace/*`; local folders are tested below),
/// AWS and Bedrock, and the coding-agent CLIs on this machine. None of
/// those is the relay.
const SCREEN_ROUTES: &[&str] = &[
    "agents",
    "agents/overviews",
    "approvals/pending",
    "ask/history",
    "capabilities",
    "capabilities/declared",
    "capability-registries",
    "companion/snapshot",
    "config/llm",
    "connectors",
    "default-model",
    "delegation/history",
    "delegation/pending-clarifications",
    "delegation/providers",
    "delegation/returns",
    "egress/status",
    "email",
    "connector-grants",
    "workspace",
    "gateway/callers",
    "gateway/config",
    "gateway/holds",
    "gateway/manifest",
    "gateway/status",
    "identity/status",
    "llm-keys",
    "marketplace/installed",
    "mascot",
    "network/snapshot",
    "palette/search",
    "pilots",
    "preferences",
    "relay/agents",
    "relay/status",
    "remote/status",
    "repo/git/status",
    "repo/status",
    "runs",
    "shared/by-me",
    "shared/capabilities",
    "shared/with-me",
    "studio/push-targets",
    "tasks",
    "teams",
    "tools",
    "tools/usage",
    "triggers",
    "trust/policies",
    "trust/requests",
    "work",
    "work/dismissed",
    "workflows",
    "workflows/manual-steps",
];

/// A core with no relay of its own: `built_in` is what the build ships
pub(crate) fn boot(built_in: BuiltInRelay) -> (AppState, Router) {
    assert!(
        crate::utils::env::var(RELAY_URL_ENV).is_none(),
        "Unset {RELAY_URL_ENV} to run the offline suite"
    );
    let state = AppState::for_tests_with_relay(vec![], built_in);
    // What `AppState::start` does on a fresh home: the repo is a git repo
    crate::services::repo_bootstrap::bootstrap(state.store().layout()).unwrap();
    let router = crate::http::router(state.clone(), WebToken::new(TOKEN));
    (state, router)
}

/// A build that ships a default relay and join code nobody can reach: a
/// port that was free a moment ago
fn unreachable_default_relay() -> BuiltInRelay {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    BuiltInRelay::new(
        Some(&format!("ws://127.0.0.1:{port}/relay")),
        Some("offline-join-code"),
    )
}

pub(crate) async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(format!("/api/{uri}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let response = tokio::time::timeout(READ_BUDGET, router.clone().oneshot(request))
        .await
        .unwrap_or_else(|_| panic!("{method} /api/{uri} took over {READ_BUDGET:?}"))
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub(crate) async fn get(router: &Router, uri: &str) -> Value {
    let (status, body) = call(router, "GET", uri, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "GET /api/{uri}: {body}");
    body
}

pub(crate) async fn post(router: &Router, uri: &str, body: Value) -> Value {
    let (status, answer) = call(router, "POST", uri, body).await;
    assert_eq!(status, StatusCode::OK, "POST /api/{uri}: {answer}");
    answer
}

/// Polls `GET uri` until `done` holds (up to 10s)
pub(crate) async fn until(router: &Router, uri: &str, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let body = get(router, uri).await;
        if done(&body) {
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "GET /api/{uri} never got there: {body}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Agents answer from the canned engine, without delays
pub(crate) async fn canned(state: &AppState, script: Value) {
    state
        .db()
        .call(move |conn| {
            crate::db::set_setting(conn, crate::db::ENGINE_KEY, &json!("canned"))?;
            crate::db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
        })
        .await
        .unwrap();
}

async fn every_screen_answers(router: &Router) {
    for route in SCREEN_ROUTES {
        get(router, route).await;
    }
}

async fn relay_is_disconnected(router: &Router) -> Value {
    let status = get(router, "relay/status").await;
    assert_eq!(status["connected"], false, "{status}");
    status
}

/// Asking someone else, and the relay's own instance list, answer the
/// typed "not connected" (503, the error starting with the stable code)
async fn person_to_person_is_not_connected(router: &Router) {
    let (status, body) = call(
        router,
        "POST",
        "delegation/tasks",
        json!({
            "capability": "gateway",
            "input": { "text": "Can you review my plan?" },
            "target": "someone-elses-nandi/gateway",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with(RELAY_UNAVAILABLE),
        "{body}"
    );
}

/// Create an agent, chat, approve its tool call, see the run complete
async fn an_agent_runs_and_its_request_is_approved(state: &AppState, router: &Router) {
    let mut script = crate::engine::default_script();
    script["tokenDelayMs"] = json!(0);
    canned(state, script).await;
    post(
        router,
        "agents",
        json!({ "id": "offline-assistant", "name": "Offline Assistant" }),
    )
    .await;
    let sent = post(
        router,
        "agents/offline-assistant/messages",
        json!({ "text": "How long is the report?" }),
    )
    .await;
    let run_id = sent["runId"].as_str().expect("a run ID").to_string();

    let pending = until(router, "approvals/pending", |body| {
        body.as_array().is_some_and(|list| !list.is_empty())
    })
    .await;
    let approval = &pending[0];
    assert_eq!(approval["agentId"], "offline-assistant");
    post(
        router,
        "agents/offline-assistant/approve",
        json!({ "approvalId": approval["id"], "decision": "approved" }),
    )
    .await;

    let detail = until(router, &format!("runs/{run_id}"), |body| {
        body["run"]["status"] != "running" && body["run"]["status"] != "awaitingApproval"
    })
    .await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
}

/// Two steps, two agents, the second reading the first's output
async fn a_workflow_runs(state: &AppState, router: &Router) {
    canned(
        state,
        json!({ "agent": [{ "text": "[{{input}}]" }], "studio": [], "tokenDelayMs": 0 }),
    )
    .await;
    for id in ["offline-researcher", "offline-writer"] {
        post(router, "agents", json!({ "id": id, "name": id })).await;
    }
    post(
        router,
        "workflows",
        json!({ "yaml": "id: offline-brief\nname: Offline brief\nsteps:\n  - id: research\n    agent: offline-researcher\n    input: \"Gather the notes\"\n  - id: write\n    agent: offline-writer\n    input: \"Write up: {{research.output}}\"\n" }),
    )
    .await;
    post(router, "workflows/offline-brief/run", Value::Null).await;
    let workflow = until(router, "workflows/offline-brief", |body| {
        body["runs"][0]["status"]
            .as_str()
            .is_some_and(|status| status != "running")
    })
    .await;
    let run = &workflow["runs"][0];
    assert_eq!(run["status"], "completed", "{run}");
}

/// `test-fixtures/offline-pack`: inspected for the consent dialog, then
/// installed through the same checks as a registry pack, as an unverified
/// local install whose agent then exists
async fn a_starter_pack_installs_from_a_local_folder(router: &Router) {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test-fixtures/offline-pack")
        .to_string_lossy()
        .into_owned();
    let manifest = post(
        router,
        "marketplace/local/inspect",
        json!({ "path": folder }),
    )
    .await;
    assert_eq!(manifest["id"], "offline-pack", "{manifest}");
    assert_eq!(manifest["verified"], false, "{manifest}");
    let installed = post(
        router,
        "marketplace/local/install",
        json!({ "path": folder }),
    )
    .await;
    assert_eq!(installed["id"], "offline-pack", "{installed}");
    let items = get(router, "marketplace/installed").await;
    let pack = items
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "offline-pack")
        .unwrap_or_else(|| panic!("not listed as installed: {items}"));
    assert_eq!(pack["signatureVerified"], false, "{pack}");
    let agents = get(router, "agents").await;
    assert!(
        agents
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "offline-helper"),
        "{agents}"
    );
}

#[tokio::test]
async fn with_no_relay_a_starter_pack_installs_from_a_folder() {
    let (_state, router) = boot(BuiltInRelay::default());
    a_starter_pack_installs_from_a_local_folder(&router).await;
}

#[tokio::test]
async fn with_no_relay_every_screen_answers() {
    let (_state, router) = boot(BuiltInRelay::default());
    let status = relay_is_disconnected(&router).await;
    assert!(status["relayUrl"].is_null(), "{status}");
    every_screen_answers(&router).await;
}

#[tokio::test]
async fn with_no_relay_an_agent_runs_and_waits_for_approval() {
    let (state, router) = boot(BuiltInRelay::default());
    an_agent_runs_and_its_request_is_approved(&state, &router).await;
}

#[tokio::test]
async fn with_no_relay_a_workflow_runs() {
    let (state, router) = boot(BuiltInRelay::default());
    a_workflow_runs(&state, &router).await;
}

#[tokio::test]
async fn with_no_relay_person_to_person_answers_not_connected() {
    let (_state, router) = boot(BuiltInRelay::default());
    person_to_person_is_not_connected(&router).await;
    let (status, body) = call(&router, "GET", "relay/instances", Value::Null).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with(RELAY_UNAVAILABLE),
        "{body}"
    );
}

/// A build with a default relay and join code it can't reach (or never
/// joined) works exactly the same; only the relay status says so
#[tokio::test]
async fn an_unreachable_default_relay_changes_nothing_local() {
    let started = Instant::now();
    let (state, router) = boot(unreachable_default_relay());
    let status = relay_is_disconnected(&router).await;
    // Only offered: nothing dials or enrolls there until the user joins
    assert_eq!(status["relayUrlSource"], Value::Null, "{status}");
    assert!(status["builtInUrl"].is_string(), "{status}");
    every_screen_answers(&router).await;
    an_agent_runs_and_its_request_is_approved(&state, &router).await;
    a_workflow_runs(&state, &router).await;
    a_starter_pack_installs_from_a_local_folder(&router).await;
    person_to_person_is_not_connected(&router).await;
    // Joining it answers calmly that it can't be reached, and saves nothing
    let joined = post(
        &router,
        "relay/join",
        json!({ "url": status["builtInUrl"] }),
    )
    .await;
    assert_eq!(joined["kind"], "unreachable", "{joined}");
    // Nothing waited for the enrollment or the connection to give up
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    relay_is_disconnected(&router).await;
}
