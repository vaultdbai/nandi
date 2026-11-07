//! The connector hub end to end, in-process over the HTTP API (no relay,
//! no model: the canned engine; nothing reaches the internet):
//!
//! 1. Nandi A makes a connector from an MCP server in Studio and publishes
//!    it to a local (file://) registry: a tag in A's agents repo, the entry
//!    and the index written there.
//! 2. Nandi B adds that registry, reads the install dialog (fetched from
//!    A's repo at the tag and checked against the entry) and installs it,
//!    the API key going to B's keyring.
//! 3. An agent on B calls the connector: the run pauses with an access
//!    request. Allow for this agent (saved) records a grant and the call
//!    runs; the grant is on the record and in settings.yaml.
//! 4. The grant is removed: the next call asks again, and Don't allow
//!    refuses it. A policy that forbids the connector refuses it outright.

use serde_json::{json, Value};

use crate::offline_suite::{boot, call, canned, get, post, until};
use crate::services::relay::BuiltInRelay;

const TRACKER_MD_TOOL: &str = "tracker__list_issues";

fn script() -> Value {
    json!({
        "agent": [
            {
                "text": "Let me look at the open issues.",
                "toolCalls": [{
                    "tool": TRACKER_MD_TOOL,
                    "args": { "project": "web" },
                    "result": "[{\"id\": 7, \"title\": \"Login is slow\"}]"
                }]
            },
            { "text": "There is one open issue: Login is slow." }
        ],
        "studio": [],
        "tokenDelayMs": 0
    })
}

/// Sends a chat message and waits for the run to settle (done or paused)
async fn run(router: &axum::Router, agent: &str, text: &str) -> (String, Value) {
    let sent = post(
        router,
        &format!("agents/{agent}/messages"),
        json!({ "text": text }),
    )
    .await;
    let run_id = sent["runId"].as_str().expect("a run ID").to_string();
    let detail = until(router, &format!("runs/{run_id}"), |body| {
        body["run"]["status"] != "running" && body["run"]["status"] != "queued"
    })
    .await;
    (run_id, detail)
}

async fn pending_access(router: &axum::Router) -> Value {
    let pending = until(router, "approvals/pending", |body| {
        body.as_array()
            .is_some_and(|list| list.iter().any(|a| a["tool"] == "connector_access"))
    })
    .await;
    pending
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["tool"] == "connector_access")
        .cloned()
        .unwrap()
}

/// The tool result the model got for the connector call in `run_id`
async fn tool_result(router: &axum::Router, run_id: &str) -> Value {
    let detail = until(router, &format!("runs/{run_id}"), |body| {
        body["run"]["status"] == "completed" || body["run"]["status"] == "failed"
    })
    .await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    detail
}

#[tokio::test]
async fn a_connector_is_published_installed_granted_and_revoked() {
    let registry_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        registry_dir.path().join("index.json"),
        "{\n  \"schema_version\": 1,\n  \"agents\": [],\n  \"packs\": []\n}\n",
    )
    .unwrap();
    let registry_url = format!(
        "file:///{}",
        registry_dir
            .path()
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );

    // --- 1. Nandi A: Studio › New connector, then Publish ---------------
    let (state_a, a) = boot(BuiltInRelay::new(None, None));
    let info = post(
        &a,
        "connectors/scaffold",
        json!({
            "id": "tracker",
            "name": "Issue tracker",
            "description": "Read the issues in your projects.",
            "version": "1.0.0",
            "run": "npx -y @acme/tracker-mcp@1.2.3",
            "auth": { "kind": "api_key", "env": ["TRACKER_TOKEN"], "label": "Tracker API key" },
            "hosts": ["api.tracker.example"],
            "tools": [
                { "name": "list_issues", "description": "List open issues", "access": "read" },
                { "name": "create_issue", "description": "File an issue", "access": "write" }
            ],
            "reads": ["Issues in your projects"],
            "writes": ["New issues you ask for"],
            "tags": ["dev"],
            "instructions": "Use it to look up issues. Never file one without asking.",
        }),
    )
    .await;
    assert_eq!(info["package"]["version"], "1.0.0", "{info}");
    assert_eq!(info["package"]["runs"], "npx -y @acme/tracker-mcp@1.2.3");
    let md = std::fs::read_to_string(
        state_a
            .store()
            .layout()
            .root
            .join(".nandi/connectors/tracker/connector.md"),
    )
    .unwrap();
    assert!(md.contains("run: npx -y @acme/tracker-mcp@1.2.3"), "{md}");

    post(
        &a,
        "marketplace/registries",
        json!({ "url": registry_url, "kind": "personal" }),
    )
    .await;
    let published = post(
        &a,
        "connectors/publish",
        json!({ "connectorId": "tracker", "registryUrl": registry_url }),
    )
    .await;
    assert_eq!(published["tag"], "tracker-v1.0.0", "{published}");
    assert_eq!(published["merged"], true);
    let entry: Value = serde_json::from_str(
        &std::fs::read_to_string(registry_dir.path().join("connectors/tracker.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(entry["kind"], "mcp-stdio");
    assert_eq!(entry["placement"], "container");
    assert_eq!(entry["run"]["package"], "@acme/tracker-mcp");
    assert_eq!(entry["commit"].as_str().map(str::len), Some(40));
    let index: Value = serde_json::from_str(
        &std::fs::read_to_string(registry_dir.path().join("index.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(index["connectors"][0]["id"], "tracker", "{index}");

    // --- 2. Nandi B: the install dialog, then Install --------------------
    let (state_b, b) = boot(BuiltInRelay::new(None, None));
    post(
        &b,
        "marketplace/registries",
        json!({ "url": registry_url, "kind": "personal" }),
    )
    .await;
    let consent = post(
        &b,
        "registry-connectors/consent",
        json!({ "registryUrl": registry_url, "connectorId": "tracker" }),
    )
    .await;
    assert_eq!(
        consent["package"]["reads"][0], "Issues in your projects",
        "{consent}"
    );
    assert_eq!(consent["package"]["writes"][0], "New issues you ask for");
    assert_eq!(consent["package"]["hosts"][0], "api.tracker.example");
    assert_eq!(consent["package"]["auth"]["kind"], "api_key");
    assert_eq!(consent["package"]["placement"], "container");
    assert!(consent["installedVersion"].is_null());
    post(
        &b,
        "registry-connectors/install",
        json!({ "registryUrl": registry_url, "connectorId": "tracker", "apiKey": "tk_secret" }),
    )
    .await;
    assert_eq!(
        state_b
            .secrets()
            .get("nandi.connector.tracker.key")
            .unwrap()
            .as_deref(),
        Some("tk_secret"),
        "the key is in the keyring"
    );
    let yaml = std::fs::read_to_string(state_b.store().layout().connectors_path()).unwrap();
    assert!(!yaml.contains("tk_secret"), "never in a file: {yaml}");
    assert!(yaml.contains("@acme/tracker-mcp@1.2.3"), "{yaml}");
    let installed = get(&b, "connectors").await;
    let tracker = installed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "tracker")
        .cloned()
        .unwrap();
    assert_eq!(tracker["package"]["registry"], registry_url, "{tracker}");
    assert!(
        get(&b, "connector-grants")
            .await
            .as_array()
            .unwrap()
            .is_empty(),
        "installing grants nothing"
    );

    // --- 3. An ungranted call pauses; Allow for this agent (saved) -------
    canned(&state_b, script()).await;
    post(&b, "agents", json!({ "id": "triager", "name": "Triager" })).await;
    let (status, body) = call(
        &b,
        "PUT",
        "agents/triager/connectors",
        json!({ "connectors": [{ "id": "tracker" }] }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    let (run_id, detail) = run(&b, "triager", "What's open?").await;
    assert_eq!(detail["run"]["status"], "awaitingApproval", "{detail}");
    let request = pending_access(&b).await;
    assert_eq!(request["args"]["family"], "connector");
    assert_eq!(request["args"]["access"], "read");
    assert_eq!(request["args"]["connectors"][0]["name"], "Issue tracker");
    assert_eq!(request["args"]["details"][0], "Issues in your projects");
    post(
        &b,
        &format!("access-requests/{}", request["id"].as_str().unwrap()),
        json!({ "choice": "allowSaved" }),
    )
    .await;
    let detail = tool_result(&b, &run_id).await;
    let text = detail.to_string();
    assert!(text.contains("Login is slow"), "the call ran: {text}");
    // On the record: the request carries the choice and who made it
    assert!(text.contains("allowSaved"), "{text}");
    let grants = get(&b, "connector-grants").await;
    assert_eq!(
        grants,
        json!([{ "agent": "triager", "connector": "tracker", "access": "read", "scope": "saved" }])
    );
    let settings = std::fs::read_to_string(state_b.store().layout().settings_path()).unwrap();
    assert!(
        settings.contains("connector: tracker"),
        "saved in the gold copy: {settings}"
    );

    // Granted: the next run doesn't ask
    let (_, detail) = run(&b, "triager", "Again?").await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");

    // --- 4. Remove the grant: the next call asks again; Don't allow ------
    post(&b, "connector-grants/revoke", grants[0].clone()).await;
    assert!(get(&b, "connector-grants")
        .await
        .as_array()
        .unwrap()
        .is_empty());
    let (run_id, detail) = run(&b, "triager", "And now?").await;
    assert_eq!(
        detail["run"]["status"], "awaitingApproval",
        "asks again: {detail}"
    );
    let request = pending_access(&b).await;
    post(
        &b,
        &format!("access-requests/{}", request["id"].as_str().unwrap()),
        json!({ "choice": "deny" }),
    )
    .await;
    let detail = tool_result(&b, &run_id).await;
    let text = detail.to_string();
    // The model got the refusal instead of the connector's answer
    assert!(
        text.contains("the person didn't let this agent read with Issue tracker"),
        "refused: {text}"
    );

    // A policy that forbids it: refused without asking
    std::fs::create_dir_all(state_b.store().layout().policies_dir()).unwrap();
    std::fs::write(
        state_b.store().layout().policy_path("no-tracker"),
        "---\napplies_to: ['agent:triager']\nconnectors: { forbid: [tracker] }\n---\nNo issue tracker for the triager.\n",
    )
    .unwrap();
    let (run_id, _) = run(&b, "triager", "One more?").await;
    let detail = tool_result(&b, &run_id).await;
    assert!(
        detail.to_string().contains("the policy 'no-tracker'"),
        "{detail}"
    );

    // Uninstalling removes the key and every grant
    post(&b, "connectors/tracker/remove", Value::Null).await;
    assert_eq!(
        state_b
            .secrets()
            .get("nandi.connector.tracker.key")
            .unwrap(),
        None
    );
    state_a.shutdown();
    state_b.shutdown();
}

#[tokio::test]
async fn allow_once_lasts_for_the_run_only() {
    let (state, router) = boot(BuiltInRelay::new(None, None));
    post(
        &router,
        "connectors/scaffold",
        json!({
            "id": "tracker", "name": "Issue tracker", "description": "Issues.",
            "run": "uvx tracker-mcp==1.0", "hosts": ["api.tracker.example"],
            "tools": [{ "name": "list_issues", "description": "List issues", "access": "read" }],
        }),
    )
    .await;
    canned(&state, script()).await;
    post(
        &router,
        "agents",
        json!({ "id": "triager", "name": "Triager" }),
    )
    .await;
    call(
        &router,
        "PUT",
        "agents/triager/connectors",
        json!({ "connectors": [{ "id": "tracker" }] }),
    )
    .await;
    let (run_id, _) = run(&router, "triager", "What's open?").await;
    let request = pending_access(&router).await;
    post(
        &router,
        &format!("access-requests/{}", request["id"].as_str().unwrap()),
        json!({ "choice": "allowOnce" }),
    )
    .await;
    let detail = tool_result(&router, &run_id).await;
    assert!(detail.to_string().contains("Login is slow"), "{detail}");
    assert!(
        get(&router, "connector-grants")
            .await
            .as_array()
            .unwrap()
            .is_empty(),
        "no grant kept"
    );
    // The next run asks again
    let (_, detail) = run(&router, "triager", "Again?").await;
    assert_eq!(detail["run"]["status"], "awaitingApproval", "{detail}");
    state.shutdown();
}
