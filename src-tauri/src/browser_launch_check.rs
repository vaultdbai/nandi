//! The visible browser pilot on this computer, for real (ignored: it needs
//! Node.js, `@playwright/mcp` and Chrome or Edge, and opens a window):
//!
//! ```text
//! cd src-tauri
//! cargo test --lib browser_launch_check -- --ignored --nocapture
//! ```
//!
//! It boots a throwaway core (its own temporary home, never a real one)
//! with the egress proxy, adds agents that declare
//! `pilots: [{id: browser, allow: [interact]}]`, grants the pilot, and then
//! goes the way a run does: the agent's pilot tools (found installed,
//! turned on by this first use, offered past deferred loading), a
//! `browser_navigate` to an allowed site that loads, and one to a site
//! outside the list that raises the site access request. The Social
//! Manager's own skill.md is checked the same way when the social pack is
//! cloned next to this repo.

use serde_json::json;

use crate::offline_suite::boot;
use crate::services::access::{self, Decision};
use crate::services::domain::EntityKind;
use crate::services::orchestrator::tool_search;
use crate::services::pilots::{self, browser, catalog, tools};
use crate::services::relay::BuiltInRelay;
use crate::services::AppState;

const AGENT: &str = "launch-check";
const SKILL: &str = "---\nname: Launch check\npilots:\n  - id: browser\n    allow: [interact]\nnetwork:\n  allow:\n    - host: example.com\n---\nOpen pages.\n";
const SOCIAL_SKILL: &str = "../../nandi-social-pack/agents/social-manager/skill.md";

fn add_agent(state: &AppState, id: &str, skill: &str) {
    let path = state.store().layout().skill_path(id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, skill).unwrap();
    state.store().commit(EntityKind::Agent, id).unwrap();
}

async fn grant_browser(state: &AppState, id: &str) {
    let pilot = catalog::find(browser::BROWSER).unwrap();
    for capability in &pilot.capabilities {
        pilots::grant(state.db(), id, &pilot, capability)
            .await
            .unwrap();
    }
}

/// What a run's step offers: the agent's pilot tools after 20 others,
/// through deferred loading
async fn offered(state: &AppState, id: &str) -> Vec<String> {
    let agent = state.store().agent(id).unwrap();
    let set = tools::for_agent(state.bus(), &agent).await;
    assert!(set.warnings.is_empty(), "{id}: {:?}", set.warnings);
    let mut all: Vec<_> = (0..20)
        .map(|i| crate::llm::ToolSpec {
            name: format!("other_{i}"),
            description: String::new(),
            input_schema: json!({"type": "object"}),
        })
        .collect();
    all.extend(set.tools);
    tool_search::step_tools(&format!("launch-check-{id}"), all)
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "launches the real browser pilot (Node.js, @playwright/mcp, Chrome or Edge)"]
async fn the_visible_browser_launches_and_navigates() {
    let (state, _router) = boot(BuiltInRelay::new(None, None));
    crate::services::egress::start_server(&state);
    add_agent(&state, AGENT, SKILL);
    grant_browser(&state, AGENT).await;

    // Found installed (npm global), turned on by this first use, offered
    let names = offered(&state, AGENT).await;
    println!("offered: {names:?}");
    for tool in [
        "pilot_browser__browser_navigate",
        "pilot_browser__browser_snapshot",
        "pilot_browser__browser_type",
        "pilot_browser__browser_hand_over",
    ] {
        assert!(names.iter().any(|n| n == tool), "{tool} in {names:?}");
    }
    let status = pilots::status_of(state.db(), None, browser::BROWSER)
        .await
        .unwrap();
    assert!(status.installed && status.running, "{status:?}");

    // An allowed site loads, through the egress proxy
    let agent = state.store().agent(AGENT).unwrap();
    let opened = tools::call(
        state.bus(),
        &agent,
        "c1",
        "pilot_browser__browser_navigate",
        &json!({"url": "https://example.com/"}),
    )
    .await;
    println!("navigate example.com: {}", opened.result);
    assert!(!opened.is_error, "{}", opened.result);
    assert!(
        opened.result.contains("Example Domain"),
        "{}",
        opened.result
    );

    // A site outside the list: the run would pause on a site request
    let skill = state.store().skill(AGENT).ok();
    let outside = json!({"url": "https://www.wikipedia.org/", "reason": "Look it up"});
    let decision = access::decide(
        &state,
        skill.as_ref(),
        AGENT,
        "launch-check-run",
        "pilot_browser__browser_navigate",
        &outside,
    )
    .await;
    match decision {
        Some(Decision::Ask(request)) => {
            let site = request.site.expect("a site request");
            assert_eq!(site.host, "www.wikipedia.org");
            assert_eq!(site.pilot, browser::BROWSER);
        }
        other => panic!("expected a site request, got {other:?}"),
    }
    // Called anyway (no answer): refused with words, never silently
    let refused = tools::call(
        state.bus(),
        &agent,
        "c2",
        "pilot_browser__browser_navigate",
        &outside,
    )
    .await;
    println!("navigate wikipedia: {}", refused.result);
    assert!(refused.is_error && refused.result.contains("allowed sites"));

    // The Social Manager, as the pack ships it
    if let Ok(social) = std::fs::read_to_string(SOCIAL_SKILL) {
        add_agent(&state, "social-manager", &social);
        grant_browser(&state, "social-manager").await;
        let names = offered(&state, "social-manager").await;
        println!("social-manager offered: {names:?}");
        for tool in [
            "pilot_browser__browser_navigate",
            "pilot_browser__browser_hand_over",
        ] {
            assert!(names.iter().any(|n| n == tool), "{tool} in {names:?}");
        }
    } else {
        println!("nandi-social-pack isn't cloned next to this repo: skipped");
    }

    browser::stop_all(browser::BROWSER).await;
    state.shutdown();
}
