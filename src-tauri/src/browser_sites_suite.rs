//! A browser pilot asking to open a site outside the agent's allowed sites,
//! end to end over the HTTP API (no relay, no model, no browser: the canned
//! engine answers the pilot call once the access gate lets it through):
//!
//! - the run pauses with a `site` access request naming the agent, the
//!   host (decoded and raw) and the agent's reason;
//! - Allow once lets this run open it (and only this run: the proxy's
//!   grant ends with the run, and the next run asks again);
//! - Always allow writes the host into the agent's skill.md, so later runs
//!   don't ask; with subdomains, `*.host` too;
//! - Don't allow fails the call with words the agent can pass on;
//! - addresses, local names and ports are never asked about.

use serde_json::{json, Value};

use crate::offline_suite::{boot, canned, rpc, until};
use crate::services::domain::EntityKind;
use crate::services::egress::capabilities::Capabilities;
use crate::services::pilots::sites;
use crate::services::relay::BuiltInRelay;
use crate::services::AppState;

const AGENT: &str = "shopper";

/// The agent opens `url`, then answers
fn script(url: &str) -> Value {
    json!({
        "agent": [
            {
                "text": "Opening the shop.",
                "toolCalls": [{
                    "tool": "pilot_browser__browser_navigate",
                    "args": { "url": url, "reason": "Check the order status" },
                    "result": "Page: Your order has shipped"
                }]
            },
            { "text": "Your order has shipped." }
        ],
        "studio": [],
        "tokenDelayMs": 0
    })
}

/// An agent of the user's own with the visible browser and one allowed site
fn add_agent(state: &AppState) {
    let path = state.store().layout().skill_path(AGENT);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "---\nname: Shopper\npilots: [browser]\nnetwork:\n  allow:\n    - host: example.com\n---\nShop.\n",
    )
    .unwrap();
    state.store().commit(EntityKind::Agent, AGENT).unwrap();
}

async fn start(url: &str) -> (AppState, axum::Router) {
    let (state, router) = boot(BuiltInRelay::new(None, None));
    add_agent(&state);
    canned(&state, script(url)).await;
    (state, router)
}

/// Sends a message and waits for the run to settle (done or paused)
async fn run(router: &axum::Router) -> (String, Value) {
    let sent = rpc(
        router,
        "send_message",
        json!({ "agentId": AGENT, "text": "Where's my order?", "attachments": [], "clientMessageId": null }),
    )
    .await;
    let run_id = sent["runId"].as_str().expect("a run ID").to_string();
    let detail = until(
        router,
        "get_run_detail",
        json!({ "runId": run_id }),
        |body| body["run"]["status"] != "running" && body["run"]["status"] != "queued",
    )
    .await;
    (run_id, detail)
}

async fn pending_site(router: &axum::Router) -> Value {
    let pending = until(router, "list_pending_approvals", json!({}), |body| {
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

async fn answer(router: &axum::Router, request: &Value, choice: &str, subdomains: bool) {
    let args = json!({ "approvalId": request["id"], "choice": choice, "subdomains": subdomains });
    rpc(router, "answer_access_request", args).await;
}

async fn finished(router: &axum::Router, run_id: &str) -> Value {
    until(
        router,
        "get_run_detail",
        json!({ "runId": run_id }),
        |body| body["run"]["status"] == "completed" || body["run"]["status"] == "failed",
    )
    .await
}

fn allowed_hosts(state: &AppState) -> Vec<String> {
    state
        .store()
        .agent(AGENT)
        .unwrap()
        .capabilities()
        .network()
        .allow
        .iter()
        .map(|rule| rule.host.clone())
        .collect()
}

fn run_grant_allows(host: &str) -> bool {
    sites::with_run_sites(AGENT, Capabilities::default()).allows_tunnel(host, 443)
}

#[tokio::test]
async fn allow_once_opens_the_site_for_this_run_only() {
    let (state, router) = start("https://shop.example.org/orders/7").await;
    let (run_id, detail) = run(&router).await;
    assert_eq!(detail["run"]["status"], "awaitingApproval", "{detail}");
    let request = pending_site(&router).await;
    let args = &request["args"];
    assert_eq!(args["family"], "site", "{args}");
    assert_eq!(args["agent"], AGENT);
    assert_eq!(args["site"]["host"], "shop.example.org");
    assert_eq!(args["site"]["display"], "shop.example.org");
    assert_eq!(args["site"]["pilot"], "browser");
    assert_eq!(args["site"]["reason"], "Check the order status");
    assert_eq!(args["site"]["canSave"], true);

    answer(&router, &request, "allowOnce", false).await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    assert!(
        detail.to_string().contains("Page: Your order has shipped"),
        "{detail}"
    );
    assert_eq!(allowed_hosts(&state), ["example.com"], "nothing saved");
    // The run ended: the browser and the proxy can't reach it any more
    assert!(!run_grant_allows("shop.example.org"));

    // The next run asks again
    let (_, detail) = run(&router).await;
    assert_eq!(detail["run"]["status"], "awaitingApproval", "{detail}");
    state.shutdown();
}

#[tokio::test]
async fn always_allow_saves_the_site_to_the_agent() {
    let (state, router) = start("https://shop.example.org/").await;
    let (run_id, _) = run(&router).await;
    let request = pending_site(&router).await;
    answer(&router, &request, "allowSaved", true).await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    assert_eq!(
        allowed_hosts(&state),
        ["example.com", "shop.example.org", "*.shop.example.org"]
    );
    let skill_md = std::fs::read_to_string(state.store().layout().skill_path(AGENT)).unwrap();
    assert!(skill_md.contains("shop.example.org"), "{skill_md}");
    // Later runs open it without asking
    let (_, detail) = run(&router).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    state.shutdown();
}

#[tokio::test]
async fn dont_allow_fails_the_call() {
    let (state, router) = start("https://shop.example.org/").await;
    let (run_id, _) = run(&router).await;
    let request = pending_site(&router).await;
    answer(&router, &request, "deny", false).await;
    let detail = finished(&router, &run_id).await;
    assert_eq!(detail["run"]["status"], "completed", "{detail}");
    let text = detail.to_string();
    assert!(text.contains("isn't allowed"), "{text}");
    assert!(
        !text.contains("Page: Your order"),
        "the page never opened: {text}"
    );
    assert_eq!(allowed_hosts(&state), ["example.com"]);
    state.shutdown();
}

#[tokio::test]
async fn a_site_is_never_allowed_for_this_computer_only() {
    let (state, router) = start("https://shop.example.org/").await;
    run(&router).await;
    let request = pending_site(&router).await;
    let args = json!({ "approvalId": request["id"], "choice": "allowLocal", "subdomains": false });
    let (status, body) =
        crate::offline_suite::call(&router, "POST", "rpc/answer_access_request", args).await;
    assert_ne!(status, axum::http::StatusCode::OK, "{body}");
    state.shutdown();
}

#[tokio::test]
async fn allowed_sites_and_never_asked_targets_dont_prompt() {
    for url in [
        "https://example.com/orders",
        "http://127.0.0.1:8080/",
        "http://169.254.169.254/latest/meta-data",
        "http://localhost/",
        "http://printer.local/",
        "https://shop.example.org:8443/",
        "file:///etc/passwd",
    ] {
        let (state, router) = start(url).await;
        let (_, detail) = run(&router).await;
        assert_eq!(detail["run"]["status"], "completed", "{url}: {detail}");
        let pending = rpc(&router, "list_pending_approvals", json!({})).await;
        assert!(pending.as_array().unwrap().is_empty(), "{url}: {pending}");
        state.shutdown();
    }
}
