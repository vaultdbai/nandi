//! No secret in the repo (DT-04), in-process over the HTTP API in the
//! offline suite's style. Every kind of definition is saved with its
//! secrets set the way the app sets them: a provider key and AWS keys
//! (settings), a connector's API key, an agent's site password for its
//! browser, a webhook trigger's secret, the backup remote's token, and a
//! workflow next to them. Then the repo's files and its whole git history
//! are searched for every secret value: files hold `secret_refs` only, the
//! values are in the keychain. A secret written to the repo fails CI.

use serde_json::json;

use crate::offline_suite::{boot, rpc};
use crate::services::domain::EntityKind;
use crate::services::relay::BuiltInRelay;
use crate::services::AppState;

/// Every file under `dir` but `.git`, as text where it is text
fn files_under(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path.is_dir() {
            files_under(&path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.push((
                path.display().to_string(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
    }
}

/// Every version of every file the repo's git history holds
fn history(repo: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--all",
            "-p",
            "--binary",
            "--no-color",
            "--full-history",
        ])
        .output()
        .expect("git is installed");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// An agent with the visible browser, so it can hold a site password
fn add_shopper(state: &AppState) {
    let path = state.store().layout().skill_path("shopper");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "---\nname: Shopper\npilots: [browser]\nnetwork:\n  allow:\n    - host: shop.example.com\n---\nShop.\n",
    )
    .unwrap();
    state.store().commit(EntityKind::Agent, "shopper").unwrap();
}

#[tokio::test]
async fn no_secret_reaches_the_repo_or_its_history() {
    let (state, router) = boot(BuiltInRelay::default());
    let mut secrets: Vec<(&str, String)> = Vec::new();

    // Settings: a provider key and AWS keys
    let provider = "sk-ant-api03-dt04-provider-0123456789";
    rpc(
        &router,
        "set_llm_api_key",
        json!({ "provider": "anthropic", "apiKey": provider }),
    )
    .await;
    secrets.push(("provider key", provider.into()));
    let aws_secret = "dt04AwsSecretKey0123456789abcdefghijklmn";
    rpc(
        &router,
        "set_aws_credentials",
        json!({ "accessKey": "AKIADT04EXAMPLE00000", "secret": aws_secret }),
    )
    .await;
    secrets.push(("AWS access key", "AKIADT04EXAMPLE00000".into()));
    secrets.push(("AWS secret key", aws_secret.into()));

    // A connector and its API key
    rpc(
        &router,
        "scaffold_connector",
        json!({ "input": {
            "id": "tracker", "name": "Issue tracker", "description": "Issues.",
            "version": "1.0.0", "run": "npx -y @acme/tracker-mcp@1.2.3",
            "auth": { "kind": "api_key", "env": ["TRACKER_TOKEN"], "label": "Tracker API key" },
            "hosts": ["api.tracker.example"],
            "tools": [{ "name": "list_issues", "description": "List issues", "access": "read" }],
        } }),
    )
    .await;
    let connector_key = "tk_dt04_connector_secret_0123456789";
    rpc(
        &router,
        "set_connector_secret",
        json!({ "id": "tracker", "key": "TRACKER_TOKEN", "value": connector_key }),
    )
    .await;
    secrets.push(("connector key", connector_key.into()));

    // An agent and its browser's site password
    add_shopper(&state);
    let site_password = "dt04-site-password-Hunter2!";
    rpc(
        &router,
        "add_pilot_secret",
        json!({
            "agentId": "shopper", "pilotId": "browser",
            "name": "SITE_PASSWORD", "value": site_password,
        }),
    )
    .await;
    secrets.push(("site password", site_password.into()));

    // A workflow, and a webhook trigger with its generated secret
    rpc(
        &router,
        "create_workflow",
        json!({ "input": { "yaml": "id: dt04-flow\nname: DT-04 flow\nsteps:\n  - id: shop\n    agent: shopper\n    input: \"Check the order\"\n" } }),
    )
    .await;
    let saved = rpc(
        &router,
        "create_trigger",
        json!({ "input": {
            "id": "dt04-hook", "name": "DT-04 hook", "sourceType": "webhook",
            "sourceConfig": {}, "targetType": "workflow", "targetId": "dt04-flow",
        }}),
    )
    .await;
    let hook_secret = saved["secret"]
        .as_str()
        .unwrap_or_else(|| panic!("a webhook gets a secret: {saved}"))
        .to_string();
    secrets.push(("webhook secret", hook_secret));

    // Settings › Backup: the remote's token
    let remote_token = "glpat-dt04-remote-token-0123456789";
    rpc(
        &router,
        "configure_remote",
        json!({
            "url": "https://git.example.com/acme/agents.git",
            "credential": { "kind": "token", "token": remote_token },
            "slug": "dev",
        }),
    )
    .await;
    secrets.push(("remote token", remote_token.into()));

    // They were really kept: in the keychain, with a ref in the file
    assert_eq!(
        state
            .secrets()
            .get("nandi.llm.anthropic")
            .unwrap()
            .as_deref(),
        Some(provider)
    );
    assert_eq!(
        state
            .secrets()
            .get("nandi.connector.tracker.key")
            .unwrap()
            .as_deref(),
        Some(connector_key)
    );
    let skill = std::fs::read_to_string(state.store().layout().skill_path("shopper")).unwrap();
    assert!(
        skill.contains("SITE_PASSWORD"),
        "the ref is in skill.md: {skill}"
    );

    // Everything into the history, as a backup would commit it
    rpc(
        &router,
        "repo_git_commit",
        json!({ "message": "DT-04: everything saved" }),
    )
    .await;

    let repo = state.store().layout().root.clone();
    let mut files = Vec::new();
    files_under(&repo, &mut files);
    assert!(
        files
            .iter()
            .any(|(path, _)| path.ends_with("settings.yaml")),
        "the settings file is there to search"
    );
    let log = history(&repo);
    assert!(log.contains("tracker"), "the history holds the saves");
    for (name, value) in &secrets {
        for (path, text) in &files {
            assert!(!text.contains(value.as_str()), "the {name} is in {path}");
        }
        assert!(
            !log.contains(value.as_str()),
            "the {name} is in the git history"
        );
    }
    state.shutdown();
}
