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
//! - an Inbox item opens as its Ask chat, and its decision is recorded
//! - a starter pack installs from a local folder
//! - person-to-person endpoints answer the typed `RelayUnavailable`
//! - a build with an unreachable default relay and join code behaves the same
//! - Settings › This computer's checklist answers (its image check pointed
//!   where nothing listens)
//! - with no model at all (no key, the free trial's gateway unreachable),
//!   the ask box answers from the built-in help guide instead of an error

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
    "capabilities",
    "companion/snapshot",
    "connectors",
    "default-model",
    "email",
    "gateway/callers",
    "gateway/config",
    "gateway/holds",
    "gateway/manifest",
    "gateway/status",
    "llm-keys",
    "marketplace/installed",
    "pilots",
    "relay/agents",
    "relay/status",
    "repo/git/status",
    "repo/status",
    "triggers",
    "workflows",
];

/// The commands without arguments behind every screen, over `/api/rpc`.
/// The rest of `scripts/api-smoke.ps1` (its `rpc:` rows).
const SCREEN_COMMANDS: &[&str] = &[
    "get_auto_approve_rules",
    "get_chief_of_staff_digest",
    "get_egress_status",
    "get_identity_status",
    "get_know_me",
    "get_my_notes",
    "get_network_snapshot",
    "get_pending_clarifications",
    "get_remote_status",
    "get_repo_llm_config",
    "get_tool_usage_stats",
    "get_workspace_status",
    "help_topics",
    "list_agents_shared_by_me",
    "list_agents_shared_with_me",
    "list_ask_history",
    "list_capability_providers",
    "list_capability_registries",
    "list_connector_grants",
    "list_decision_months",
    "list_declared_capabilities",
    "list_dismissed_work",
    "list_groups",
    "list_note_suggestions",
    "list_pending_manual_steps",
    "list_push_targets",
    "list_recent_changes",
    "list_request_returns",
    "list_tasks",
    "list_teams",
    "list_tools",
    "list_trust_policies",
    "list_trust_requests",
    "list_work",
    "load_preferences",
    "mascot_get",
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

/// Runs a command over `POST /api/rpc/{name}`; its arguments are a JSON object
pub(crate) async fn rpc(router: &Router, name: &str, args: Value) -> Value {
    post(router, &format!("rpc/{name}"), args).await
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
    for command in SCREEN_COMMANDS {
        rpc(router, command, json!({})).await;
    }
    // The run history's first page, with no filters
    let first_page = json!({ "filters": {}, "limit": 50, "offset": 0 });
    rpc(router, "list_all_runs", first_page).await;
    // Delegation: every provider, and the history's first page
    rpc(
        router,
        "list_available_providers",
        json!({ "capability": "" }),
    )
    .await;
    rpc(router, "list_delegation_history", json!({ "limit": 200 })).await;
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
        "rpc/delegate_task",
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

/// Settings › This computer: the checklist answers, read-only, with every
/// row a state and the image check's "no answer" kept to its own row
#[tokio::test]
async fn with_no_relay_the_computer_checklist_answers() {
    crate::services::machine::use_registry_for_tests("http://127.0.0.1:9");
    let (_state, router) = boot(BuiltInRelay::default());
    // Local commands (WSL, PowerShell, df) take their own short timeouts;
    // none of them is the relay
    let request = Request::builder()
        .method("POST")
        .uri("/api/rpc/check_machine")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"fresh":false}"#))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(30), router.oneshot(request))
        .await
        .expect("the checklist answers within 30 s")
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    let checks = report["checks"].as_array().expect("checks");
    let ids: Vec<&str> = checks.iter().filter_map(|c| c["id"].as_str()).collect();
    for id in ["dockerInstalled", "dockerRunning", "disk", "images"] {
        assert!(ids.contains(&id), "{id} missing: {report}");
    }
    for check in checks {
        assert!(
            check["state"].is_string() && check["reason"].is_string(),
            "{check}"
        );
    }
    assert!(report["dockerReady"].is_boolean(), "{report}");
    let images = checks.iter().find(|c| c["id"] == "images").unwrap();
    assert!(
        ["downloaded", "unreachable"].contains(&images["reason"].as_str().unwrap()),
        "{images}"
    );
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

/// A local Inbox item opens as its Ask chat with nothing but this Nandi,
/// and deciding it (here from its card) lands in the decisions file
async fn an_inbox_item_opens_as_a_chat(state: &AppState, router: &Router) {
    let mut script = crate::engine::default_script();
    script["tokenDelayMs"] = json!(0);
    canned(state, script).await;
    post(
        router,
        "agents",
        json!({ "id": "offline-assistant", "name": "Offline Assistant" }),
    )
    .await;
    post(
        router,
        "agents/offline-assistant/messages",
        json!({ "text": "How long is the report?" }),
    )
    .await;
    let pending = until(router, "approvals/pending", |body| {
        body.as_array().is_some_and(|list| !list.is_empty())
    })
    .await;
    let id = pending[0]["id"].as_str().unwrap().to_string();
    let thread = format!("inbox:approval:{id}");

    let opened = rpc(
        router,
        "open_inbox_thread",
        json!({ "conversationId": thread }),
    )
    .await;
    assert_eq!(opened["kind"], "approval", "{opened}");
    assert_eq!(opened["waiting"], true, "{opened}");
    assert_eq!(opened["tools"], json!(["inbox_approve", "inbox_refuse"]));

    post(
        router,
        "agents/offline-assistant/approve",
        json!({ "approvalId": id, "decision": "approved" }),
    )
    .await;
    // The run takes the decision a moment later: then nothing waits
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let thread = rpc(
            router,
            "open_inbox_thread",
            json!({ "conversationId": thread }),
        )
        .await;
        if thread["waiting"] == false {
            break;
        }
        assert!(Instant::now() < deadline, "still waiting: {thread}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    let file = state
        .store()
        .layout()
        .decisions_dir()
        .join(format!("{month}.md"));
    let text = std::fs::read_to_string(file).unwrap();
    assert!(
        text.contains(&format!("- **Item:** approval `{id}`")),
        "{text}"
    );
    assert!(text.contains("- **Decision:** approved"), "{text}");
}

#[tokio::test]
async fn with_no_relay_an_inbox_item_opens_as_a_chat() {
    let (state, router) = boot(BuiltInRelay::default());
    an_inbox_item_opens_as_a_chat(&state, &router).await;
}

/// Getting to know you with nothing but this Nandi: the to-do, its chat,
/// and a confirmed proposal written to NANDI.md and My notes
#[tokio::test]
async fn with_no_relay_getting_to_know_you_works() {
    let (state, router) = boot(BuiltInRelay::default());
    let notes = tempfile::tempdir().unwrap();
    crate::services::connectors::share_folder(
        &state,
        crate::services::connectors::folders::SharedFolder {
            id: crate::services::notes::NOTES_FOLDER_ID.into(),
            path: notes.path().to_string_lossy().to_string(),
            access: crate::services::connectors::folders::Access::Write,
        },
    )
    .await
    .unwrap();
    crate::services::knowme::start(&state).await.unwrap();
    assert_eq!(
        rpc(&router, "get_know_me", json!({})).await["todoOpen"],
        true
    );

    let opened = rpc(
        &router,
        "open_inbox_thread",
        json!({ "conversationId": "inbox:knowMe:you" }),
    )
    .await;
    assert_eq!(opened["kind"], "knowMe", "{opened}");
    assert_eq!(opened["tools"], json!(["knowme_propose"]));

    // What the confirmed `knowme_propose` does
    crate::services::knowme::apply(
        &state,
        &json!({
            "summary": "Runs the books",
            "responsibilities": [
                {"text": "Month-end close", "kind": "work"},
                {"text": "Grandma's medicines", "kind": "personal"}
            ]
        }),
    )
    .await
    .unwrap();
    let manifest = get(&router, "gateway/manifest").await;
    assert_eq!(manifest["responsibilities"], json!(["Month-end close"]));
    let private =
        std::fs::read_to_string(notes.path().join("private").join("about-me.md")).unwrap();
    assert!(private.contains("- Grandma's medicines"), "{private}");
    let status = rpc(&router, "get_know_me", json!({})).await;
    assert_eq!(status["todoOpen"], false, "{status}");
    assert_eq!(status["recent"][0]["kind"], "knowMe", "{status}");
    let later = rpc(&router, "know_me_todo", json!({ "action": "later" })).await;
    assert_eq!(later["todoOpen"], false, "{later}");
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

/// A build whose public relay nobody can reach (not live yet, or no
/// network): a port that was free a moment ago
fn unreachable_public_relay() -> BuiltInRelay {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    BuiltInRelay::default().with_public(Some(&format!("ws://127.0.0.1:{port}/relay")))
}

/// Onboarding's own calls, from Email to done, saying "Not now" to the
/// network step: nothing on it touches a relay
async fn onboarding_completes(router: &Router) {
    for step in ["name", "model", "computer", "network", "packs"] {
        rpc(router, "set_onboarding_step", json!({ "step": step })).await;
    }
    rpc(
        router,
        "set_display_name",
        json!({ "name": "Ada Lovelace" }),
    )
    .await;
    rpc(router, "set_onboarding_step", json!({ "step": "done" })).await;
    let identity = rpc(router, "get_identity_status", json!({})).await;
    assert_eq!(identity["onboarded"], true, "{identity}");
    assert_eq!(identity["onboardingStep"], "done", "{identity}");
}

/// With no relay at all, onboarding completes ("Not now") and saves none
#[tokio::test]
async fn with_no_relay_onboarding_completes_with_not_now() {
    let (_state, router) = boot(BuiltInRelay::default());
    onboarding_completes(&router).await;
    let status = relay_is_disconnected(&router).await;
    assert_eq!(status["relayUrl"], Value::Null, "{status}");
}

/// Choosing "Join the public relay" while it can't be reached: the offer
/// and joining answer calmly at once, nothing is saved, onboarding
/// completes, and everything local works the same
#[tokio::test]
async fn an_unreachable_public_relay_never_blocks_onboarding() {
    let started = Instant::now();
    let (state, router) = boot(unreachable_public_relay());
    let status = relay_is_disconnected(&router).await;
    assert!(status["publicRelayUrl"].is_string(), "{status}");
    assert_eq!(status["relayUrl"], Value::Null, "{status}");

    let offer = get(&router, "relay/public").await;
    assert_eq!(offer["kind"], "unreachable", "{offer}");
    // No email connected: joining says to connect one, at once
    let joined = post(&router, "relay/public/join", Value::Null).await;
    assert_eq!(joined["kind"], "needsEmail", "{joined}");
    // No account to show
    let account = get(&router, "relay/public/account").await;
    assert_eq!(account["kind"], "notPublic", "{account}");

    onboarding_completes(&router).await;
    every_screen_answers(&router).await;
    an_agent_runs_and_its_request_is_approved(&state, &router).await;
    person_to_person_is_not_connected(&router).await;
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    let status = relay_is_disconnected(&router).await;
    assert_eq!(status["relayUrl"], Value::Null, "{status}");
}

/// No key and no free trial: the ask box answers from the built-in help
/// guide, and the guide's topics are served
#[tokio::test]
async fn with_no_model_the_ask_box_answers_from_the_help_guide() {
    let (_state, router) = boot(BuiltInRelay::default());
    let answer = rpc(
        &router,
        "ask",
        json!({ "request": { "text": "How do I install Docker?" } }),
    )
    .await;
    assert_eq!(answer["kind"], "help", "{answer}");
    assert_eq!(answer["data"]["topic"]["id"], "install-docker", "{answer}");
    assert_eq!(answer["data"]["reason"], "noModel", "{answer}");

    let topics = rpc(&router, "help_topics", json!({})).await;
    assert!(topics.as_array().is_some_and(|t| t.len() >= 10), "{topics}");
    let chat = rpc(&router, "help_answer", json!({ "text": "join a relay" })).await;
    assert_eq!(chat["topic"]["id"], "join-relay", "{chat}");
}

/// The free trial's gateway can't be reached (offline): the ask still gets
/// a help answer, and the next one doesn't wait on the gateway again
#[tokio::test]
async fn with_the_trial_gateway_unreachable_the_ask_box_still_helps() {
    use crate::services::llm_trial::{use_test_trial, Trial};
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    use_test_trial(Some(std::sync::Arc::new(Trial::new(
        format!("http://127.0.0.1:{port}"),
        "offline-instance".into(),
    ))));
    let (_state, router) = boot(BuiltInRelay::default());
    let model = get(&router, "default-model").await;
    assert!(
        model["effectiveModel"].is_string(),
        "the trial is offered: {model}"
    );

    let first = rpc(
        &router,
        "ask",
        json!({ "request": { "text": "What is an agent?" } }),
    )
    .await;
    assert_eq!(first["kind"], "help", "{first}");
    assert_eq!(first["data"]["topic"]["id"], "agents", "{first}");
    assert_eq!(first["data"]["reason"], "modelFailed", "{first}");

    // Now known unreachable: no model is offered, the guide answers at once
    let model = get(&router, "default-model").await;
    assert!(model["effectiveModel"].is_null(), "{model}");
    assert_eq!(model["trial"]["unreachable"], true, "{model}");
    let second = rpc(
        &router,
        "ask",
        json!({ "request": { "text": "How do I connect a model?" } }),
    )
    .await;
    assert_eq!(second["data"]["topic"]["id"], "connect-model", "{second}");
    assert_eq!(second["data"]["reason"], "noModel", "{second}");
    use_test_trial(None);
}
