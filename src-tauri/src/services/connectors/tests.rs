//! Registry and service tests. Pieces have their own tests next to them
//! (`tools`, `filesystem`, `web_search`, `mcp`, `audit`).

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;

use super::*;
use crate::services::events::tests::RecordingSink;

fn manifest_with_folder(dir: &std::path::Path) -> ConnectorManifest {
    let yaml = format!(
        "connectors:\n  - id: filesystem\n    kind: filesystem\n    config:\n      folders: [{{id: shared, path: '{}', access: write}}]\n",
        dir.display()
    );
    ConnectorManifest::from_yaml(&yaml).unwrap()
}

/// The `filesystem` config sharing `dir` as `shared`, read and write
fn shared_folder_config(dir: &std::path::Path) -> BTreeMap<String, serde_yaml::Value> {
    folders::folders_config(&[folders::SharedFolder {
        id: "shared".into(),
        path: dir.display().to_string(),
        access: folders::Access::Write,
    }])
}

fn declared(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn definition(id: &str, sensitive: &[&str]) -> ConnectorDefinition {
    ConnectorDefinition {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        kind: catalog::ConnectorKind::Mcp,
        sensitive_tools: sensitive.iter().map(|s| s.to_string()).collect(),
    }
}

/// A connector with one `ping` tool whose health can be switched.
struct Fake {
    healthy: AtomicBool,
    slow: bool,
}

impl Fake {
    fn new(healthy: bool) -> Arc<Self> {
        Arc::new(Self {
            healthy: AtomicBool::new(healthy),
            slow: false,
        })
    }
}

impl Connector for Fake {
    fn list_tools(&self) -> Vec<ToolDef> {
        vec![ToolDef {
            name: "ping".into(),
            description: "Pong".into(),
            input_schema: json!({"type": "object"}),
            sensitive: false,
        }]
    }

    fn call_tool<'a>(
        &'a self,
        _name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { Ok(json!({"pong": args})) })
    }

    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if self.slow {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            if self.healthy.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("down".into())
            }
        })
    }
}

fn fake_manifest(ids: &[&str]) -> ConnectorManifest {
    let mut yaml = String::from("connectors:\n");
    for id in ids {
        yaml.push_str(&format!("  - {{id: '{id}', kind: mcp}}\n"));
    }
    ConnectorManifest::from_yaml(&yaml).unwrap()
}

#[tokio::test]
async fn starts_declared_connectors_on_first_use_and_prefixes_their_tools() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ConnectorRegistry::default();
    let manifest = manifest_with_folder(dir.path());
    let taken = HashSet::from(["read_file".to_string(), "shell".to_string()]);

    let set = registry
        .agent_tools(
            &Secrets::in_memory(),
            &manifest,
            &declared(&["filesystem"]),
            &taken,
        )
        .await;

    assert!(set.warnings.is_empty(), "{:?}", set.warnings);
    let names: Vec<_> = set.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "filesystem__read_file",
            "filesystem__write_file",
            "filesystem__list_dir"
        ]
    );
    assert_eq!(
        set.sensitive,
        HashSet::from(["filesystem__write_file".to_string()])
    );
    assert!(registry.is_running("filesystem"));
    assert_eq!(set.changed.len(), 1, "started: one status change");
    assert!(set.changed[0].status.healthy);
}

#[tokio::test]
async fn each_agent_reaches_only_the_folders_its_skill_names() {
    let (docs, secret) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(docs.path().join("a.md"), "doc").unwrap();
    std::fs::write(secret.path().join("s.md"), "no").unwrap();
    let yaml = format!(
        "connectors:\n  - id: filesystem\n    kind: filesystem\n    config:\n      folders:\n        - {{id: docs, path: '{}', access: read}}\n        - {{id: secret, path: '{}', access: write}}\n",
        docs.path().display(),
        secret.path().display()
    );
    let manifest = ConnectorManifest::from_yaml(&yaml).unwrap();
    let registry = ConnectorRegistry::with_audit_log(docs.path().join("audit.log"));
    let agent = domain::Agent::from_skill_md(
        domain::AgentId::parse("reader").unwrap(),
        "---\nname: Reader\nconnectors:\n  - {id: filesystem, folders: [docs]}\n---\nRead.\n",
    )
    .unwrap();
    let declared = declared_connectors(&agent);
    let options = declared_options(&agent);
    assert_eq!(declared, ["filesystem"]);
    registry
        .agent_tools(&Secrets::in_memory(), &manifest, &declared, &HashSet::new())
        .await;

    let call = |path: &'static str| {
        let (registry, declared, options) = (&registry, &declared, &options);
        async move {
            registry
                .call_agent_tool_with(
                    "reader",
                    declared,
                    options,
                    "filesystem__read_file",
                    &json!({ "path": path }),
                )
                .await
                .unwrap()
        }
    };
    assert_eq!(call("docs/a.md").await.result, "doc");
    assert!(call("secret/s.md").await.is_error);

    // The container mounts the same folders
    let mounted = agent_folders(&agent, &manifest);
    let ids: Vec<&str> = mounted.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["docs"]);
}

#[tokio::test]
async fn agents_only_get_the_connectors_they_declare() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ConnectorRegistry::default();
    let manifest = manifest_with_folder(dir.path());
    let set = registry
        .agent_tools(&Secrets::in_memory(), &manifest, &[], &HashSet::new())
        .await;
    assert!(set.tools.is_empty());
    assert!(
        !registry.is_running("filesystem"),
        "nothing starts by default"
    );

    let call = registry
        .call_agent_tool(
            "agent-1",
            &[],
            "filesystem__read_file",
            &json!({"path": "a"}),
        )
        .await;
    assert!(
        call.is_none(),
        "an undeclared connector's tools aren't routed"
    );
}

#[tokio::test]
async fn warns_about_missing_stopped_and_clashing_connectors() {
    let registry = ConnectorRegistry::default();
    let manifest = fake_manifest(&["a.b", "a_b", "stopped"]);
    registry.insert("a.b", Fake::new(true), &definition("a.b", &[]));
    registry.insert("a_b", Fake::new(true), &definition("a_b", &[]));
    registry.insert("stopped", Fake::new(true), &definition("stopped", &[]));
    registry.stop("stopped").await;

    let set = registry
        .agent_tools(
            &Secrets::in_memory(),
            &manifest,
            &declared(&["a.b", "a_b", "stopped", "missing"]),
            &HashSet::new(),
        )
        .await;
    let names: Vec<_> = set.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["a_b__ping"]);
    assert_eq!(set.warnings.len(), 3, "{:?}", set.warnings);
    assert!(set.warnings[0].contains("same tool prefix"));
    assert!(set.warnings[1].contains("stopped"));
    assert!(set.warnings[2].contains("isn't installed"));
}

#[tokio::test]
async fn container_tools_win_name_collisions() {
    let registry = ConnectorRegistry::default();
    registry.insert("x", Fake::new(true), &definition("x", &[]));
    let taken = HashSet::from(["x__ping".to_string()]);
    let set = registry
        .agent_tools(
            &Secrets::in_memory(),
            &fake_manifest(&["x"]),
            &declared(&["x"]),
            &taken,
        )
        .await;
    assert!(set.tools.is_empty());
}

#[tokio::test]
async fn unhealthy_connectors_are_left_out_with_a_warning() {
    let registry = ConnectorRegistry::default();
    let fake = Fake::new(true);
    registry.insert("x", fake.clone(), &definition("x", &["ping"]));
    let manifest = fake_manifest(&["x"]);
    let ids = declared(&["x"]);

    let set = registry
        .agent_tools(&Secrets::in_memory(), &manifest, &ids, &HashSet::new())
        .await;
    assert_eq!(set.tools.len(), 1);
    assert!(
        set.sensitive.contains("x__ping"),
        "marked in the definition"
    );

    fake.healthy.store(false, Ordering::SeqCst);
    let set = registry
        .agent_tools(&Secrets::in_memory(), &manifest, &ids, &HashSet::new())
        .await;
    assert!(set.tools.is_empty());
    assert!(set.warnings[0].contains("unhealthy"));
    assert_eq!(set.changed.len(), 1);
    assert!(!set.changed[0].status.healthy);
    assert_eq!(set.changed[0].status.error.as_deref(), Some("down"));
}

#[tokio::test]
async fn health_checks_time_out_after_two_seconds() {
    let registry = ConnectorRegistry::default();
    let slow = Arc::new(Fake {
        healthy: AtomicBool::new(true),
        slow: true,
    });
    registry.insert("slow", slow, &definition("slow", &[]));
    let started = Instant::now();
    let error = registry.check_health("slow").await.unwrap_err();
    assert!(error.contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!registry.cached_status("slow", true).healthy);
}

#[tokio::test]
async fn routes_calls_and_audits_them() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("connectors.log");
    let registry = ConnectorRegistry::with_audit_log(log.clone());
    registry.insert("x", Fake::new(true), &definition("x", &[]));

    let outcome = registry
        .call_agent_tool(
            "agent-1",
            &declared(&["x"]),
            "x__ping",
            &json!({"q": "secret"}),
        )
        .await
        .unwrap();
    assert!(!outcome.is_error);
    assert_eq!(outcome.result, r#"{"pong":{"q":"secret"}}"#);

    let missing = registry
        .call_agent_tool("agent-1", &declared(&["x"]), "x__nope", &json!({}))
        .await
        .unwrap();
    assert!(missing.is_error);
    assert!(registry
        .call_agent_tool("agent-1", &declared(&["x"]), "shell", &json!({}))
        .await
        .is_none());

    let text = std::fs::read_to_string(&log).unwrap();
    let entry: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(entry["agentId"], "agent-1");
    assert_eq!(entry["connectorId"], "x");
    assert_eq!(entry["tool"], "ping");
    assert_eq!(entry["outcome"], "ok");
    assert!(!text.contains("secret"), "argument values aren't logged");
}

#[tokio::test]
async fn external_mcp_servers_plug_in() {
    let registry = ConnectorRegistry::default();
    let mcp = super::mcp::tests::fake_server().await;
    registry.insert("notes", Arc::new(mcp), &definition("notes", &[]));
    let set = registry
        .agent_tools(
            &Secrets::in_memory(),
            &fake_manifest(&["notes"]),
            &declared(&["notes"]),
            &HashSet::new(),
        )
        .await;
    let names: Vec<_> = set.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["notes__echo", "notes__delete"]);
    assert_eq!(set.sensitive, HashSet::from(["notes__delete".to_string()]));
    let echoed = registry
        .call_agent_tool(
            "a",
            &declared(&["notes"]),
            "notes__echo",
            &json!({"text": "hi"}),
        )
        .await
        .unwrap();
    assert_eq!(echoed.result, "hi");
    registry.shutdown_all().await;
    assert!(!registry.is_running("notes"));
}

#[tokio::test]
async fn start_and_stop_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ConnectorRegistry::default();
    let manifest = manifest_with_folder(dir.path());
    let decl = manifest.get("filesystem").unwrap();
    let secrets = Secrets::in_memory();

    registry.start(decl, &secrets).await.unwrap();
    registry.check_health("filesystem").await.unwrap();
    assert!(registry.cached_status("filesystem", true).healthy);

    registry.stop("filesystem").await;
    let status = registry.cached_status("filesystem", true);
    assert!(!status.running && !status.healthy);
    let set = registry
        .agent_tools(
            &secrets,
            &manifest,
            &declared(&["filesystem"]),
            &HashSet::new(),
        )
        .await;
    assert!(set.tools.is_empty(), "a stopped connector stays stopped");

    registry.start(decl, &secrets).await.unwrap();
    assert!(!registry.is_stopped("filesystem"));
}

/// `filesystem__read_file` of `shared/a.txt` through the instance an agent
/// with `on_host` uses
async fn read_shared(registry: &ConnectorRegistry, on_host: &[String]) -> ToolOutcome {
    let agent = domain::Agent::from_skill_md(
        domain::AgentId::parse("desk").unwrap(),
        "---
name: Desk
connectors:
  - {id: filesystem, folders: [shared]}
---
Work.
",
    )
    .unwrap();
    registry
        .call_agent_tool_placed(
            "desk",
            &declared_connectors(&agent),
            &declared_options(&agent),
            on_host,
            "filesystem__read_file",
            &json!({ "path": "shared/a.txt" }),
        )
        .await
        .expect("a connector tool")
}

#[tokio::test]
async fn run_on_host_keeps_a_host_instance_of_its_own() {
    let (shared, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(shared.path().join("a.txt"), "hi").unwrap();
    let registry = ConnectorRegistry::with_audit_log(logs.path().join("audit.log"));
    let manifest = manifest_with_folder(shared.path());
    let secrets = Secrets::in_memory();
    let ids = declared(&["filesystem"]);
    let on_host = declared(&["filesystem"]);

    // An agent allowed to run it on the host starts the host instance only
    let set = registry
        .agent_tools_placed(&secrets, &manifest, &ids, &on_host, &HashSet::new())
        .await;
    assert!(set.warnings.is_empty(), "{:?}", set.warnings);
    let host = registry
        .running(&host_key("filesystem"))
        .expect("the host instance");
    assert!(!registry.is_running("filesystem"));
    assert_eq!(read_shared(&registry, &on_host).await.result, "hi");
    assert!(
        read_shared(&registry, &[]).await.is_error,
        "other agents don't reach the host instance"
    );

    // It's kept: the next turn reuses it
    let again = registry
        .agent_tools_placed(&secrets, &manifest, &ids, &on_host, &HashSet::new())
        .await;
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    let kept = registry.running(&host_key("filesystem")).unwrap();
    assert!(Arc::ptr_eq(&host, &kept), "not started again");

    // Other agents get their own instance next to it
    let other = registry
        .agent_tools(&secrets, &manifest, &ids, &HashSet::new())
        .await;
    assert!(other.warnings.is_empty(), "{:?}", other.warnings);
    assert!(registry.is_running("filesystem") && registry.is_running(&host_key("filesystem")));
    assert_eq!(read_shared(&registry, &[]).await.result, "hi");

    // Stopping the connector stops both
    registry.stop("filesystem").await;
    assert!(!registry.is_running("filesystem") && !registry.is_running(&host_key("filesystem")));
}

#[tokio::test]
async fn start_failures_are_reported_in_the_status() {
    let registry = ConnectorRegistry::default();
    let manifest =
        ConnectorManifest::from_yaml("connectors:\n  - {id: filesystem, kind: filesystem}\n")
            .unwrap();
    let error = registry
        .start(manifest.get("filesystem").unwrap(), &Secrets::in_memory())
        .await
        .unwrap_err();
    assert!(error.contains("Add a folder"), "{error}");
    let status = registry.cached_status("filesystem", true);
    assert!(!status.running);
    assert_eq!(status.error.as_deref(), Some(error.as_str()));
}

#[test]
fn reads_connectors_from_skill_md() {
    let agent = domain::Agent::from_skill_md(
        domain::AgentId::parse("helper").unwrap(),
        "---\nname: Helper\nconnectors:\n  - filesystem\n  - {id: web_search}\n  - filesystem\n---\nHelp.\n",
    )
    .unwrap();
    assert_eq!(declared_connectors(&agent), ["filesystem", "web_search"]);

    let plain = domain::Agent::from_skill_md(
        domain::AgentId::parse("plain").unwrap(),
        "---\nname: Plain\n---\nHi.\n",
    )
    .unwrap();
    assert!(declared_connectors(&plain).is_empty());
}

#[test]
fn lists_the_catalog_and_declared_connectors() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Secrets::in_memory();
    let infos = connector_infos(&manifest_with_folder(dir.path()), None, &secrets);
    let ids: Vec<_> = infos.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        ["filesystem", "web_search", "github", "aws", "s3_backup",]
    );
    let fs = &infos[0];
    assert!(fs.status.installed && !fs.status.running);
    assert_eq!(fs.kind, "mcp");
    assert_eq!(fs.tools.len(), 3);
    assert!(fs
        .tools
        .iter()
        .any(|t| t.name == "write_file" && t.sensitive));
    assert!(fs.config.contains_key("folders"));
    assert_eq!(fs.setup_hint, None);

    // Web search needs no key
    let search = &infos[1];
    assert!(!search.status.installed);
    assert!(search.secrets.is_empty() && search.setup_hint.is_none());

    let github = &infos[2];
    assert_eq!(github.kind, "mcp");
    assert_eq!(github.secrets[0].keyring_entry, "connectors.github.token");
    // Optional: the GitHub sign-in (or the GitHub CLI's) stands in for it
    assert!(!github.secrets[0].required);
    assert!(github
        .tools
        .iter()
        .any(|t| t.name == "post_review" && t.sensitive));
    assert_eq!(
        github.setup_hint.as_deref(),
        Some("Sign in to GitHub, or add a GitHub token")
    );
    assert_eq!(infos[3].id, "aws");
    let backup = &infos[4];
    assert_eq!(backup.id, "s3_backup");
    assert!(backup
        .tools
        .iter()
        .any(|t| t.name == "create_upload_user" && t.sensitive));
    assert!(backup
        .tools
        .iter()
        .any(|t| t.name == "backup_folder" && !t.sensitive));
}

#[tokio::test]
async fn secrets_are_saved_write_only_and_gate_start() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    install_connector(&state, "github").await.unwrap();
    crate::services::connectors::github::set_cli_token_for_tests(None);

    // Missing token: Start says what's missing instead of trying
    let error = start_connector(&state, "github").await.unwrap_err();
    assert!(error.starts_with("Sign in to GitHub"), "{error}");

    set_connector_secret(&state, "github", "token", "  ghp-123  ")
        .await
        .unwrap();
    assert_eq!(
        state
            .secrets()
            .get("connectors.github.token")
            .unwrap()
            .as_deref(),
        Some("ghp-123")
    );
    let github = list_connectors(&state)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.id == "github")
        .unwrap();
    assert!(github.secrets[0].is_set && github.setup_hint.is_none());
    // The listing never carries the value
    let listed = serde_json::to_string(&github).unwrap();
    assert!(!listed.contains("ghp-123"));
    // Nor does the repo
    let file = std::fs::read_to_string(state.store().layout().connectors_path()).unwrap();
    assert!(!file.contains("ghp-123"));
    assert!(recorder.names().contains(&"connectors://changed"));

    // Empty clears it
    set_connector_secret(&state, "github", "token", "")
        .await
        .unwrap();
    assert_eq!(
        state.secrets().get("connectors.github.token").unwrap(),
        None
    );
    // Unknown keys can't be set
    assert!(set_connector_secret(&state, "github", "nope", "x")
        .await
        .is_err());
    // Web search has no key at all
    assert!(set_connector_secret(&state, "web_search", "api_key", "x")
        .await
        .is_err());
    state.shutdown();
}

#[tokio::test]
async fn an_agents_folders_are_set_from_its_page() {
    let (docs, reports) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let state = AppState::for_tests(vec![]);
    install_connector(&state, "filesystem").await.unwrap();
    let list = [
        folders::SharedFolder {
            id: "docs".into(),
            path: docs.path().display().to_string(),
            access: folders::Access::Read,
        },
        folders::SharedFolder {
            id: "reports".into(),
            path: reports.path().display().to_string(),
            access: folders::Access::Write,
        },
    ];
    configure_connector(&state, "filesystem", folders::folders_config(&list))
        .await
        .unwrap();
    // A bad list is refused before it's saved
    let bad = configure_connector(
        &state,
        "filesystem",
        serde_yaml::from_str("folders: [{id: docs, path: /a}, {id: docs, path: /b}]").unwrap(),
    )
    .await;
    assert!(bad.unwrap_err().contains("Two folders"));

    let layout = state.store().layout().clone();
    std::fs::create_dir_all(layout.skill_path("writer").parent().unwrap()).unwrap();
    std::fs::write(
        layout.skill_path("writer"),
        "---\nname: Writer\n---\nWrite.\n",
    )
    .unwrap();
    state.store().load().unwrap();

    let choice = AgentConnectorChoice {
        id: "filesystem".into(),
        folders: vec![
            FolderChoice {
                id: "docs".into(),
                access: None,
            },
            FolderChoice {
                id: "reports".into(),
                access: Some(folders::Access::Read),
            },
        ],
    };
    set_agent_connectors(&state, "writer", vec![choice.clone()])
        .await
        .unwrap();
    assert_eq!(agent_connectors(&state, "writer").unwrap(), [choice]);
    // The folder list says who has each folder
    assert_eq!(
        folder_users(&state, "filesystem").unwrap(),
        [
            FolderUsers {
                folder: "docs".into(),
                agents: vec!["Writer".into()],
            },
            FolderUsers {
                folder: "reports".into(),
                agents: vec!["Writer".into()],
            },
        ]
    );
    let skill = std::fs::read_to_string(layout.skill_path("writer")).unwrap();
    assert!(skill.contains("folders:"), "{skill}");
    let skill = state.store().skill("writer").unwrap();
    let got: Vec<(&str, folders::Access)> = skill
        .folders
        .iter()
        .map(|f| (f.id.as_str(), f.access))
        .collect();
    assert_eq!(
        got,
        [
            ("docs", folders::Access::Read),
            ("reports", folders::Access::Read)
        ]
    );

    // A folder that isn't on the list is refused; none clears the key
    let unknown = set_agent_connectors(
        &state,
        "writer",
        vec![AgentConnectorChoice {
            id: "filesystem".into(),
            folders: vec![FolderChoice {
                id: "photos".into(),
                access: None,
            }],
        }],
    )
    .await;
    assert!(unknown.unwrap_err().contains("photos"));
    set_agent_connectors(&state, "writer", vec![])
        .await
        .unwrap();
    assert!(agent_connectors(&state, "writer").unwrap().is_empty());
    state.shutdown();
}

#[tokio::test]
async fn config_edits_reach_the_gold_copy() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::for_tests(vec![]);
    install_connector(&state, "filesystem").await.unwrap();
    configure_connector(&state, "filesystem", shared_folder_config(dir.path()))
        .await
        .unwrap();
    let file = std::fs::read_to_string(state.store().layout().connectors_path()).unwrap();
    assert!(file.contains("folders:"), "{file}");
    assert_eq!(
        catalog::setup_hint(manifest(&state).get("filesystem").unwrap(), state.secrets()),
        None
    );
    assert!(configure_connector(&state, "web_search", BTreeMap::new())
        .await
        .is_err());
    state.shutdown();
}

#[tokio::test]
async fn install_writes_the_gold_copy() {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);

    install_connector(&state, "filesystem").await.unwrap();
    install_connector(&state, "filesystem").await.unwrap(); // idempotent
    assert!(install_connector(&state, "nope").await.is_err());

    let file = std::fs::read_to_string(state.store().layout().connectors_path()).unwrap();
    assert!(file.contains("id: filesystem"));
    let infos = list_connectors(&state).await.unwrap();
    assert!(
        infos
            .iter()
            .find(|i| i.id == "filesystem")
            .unwrap()
            .status
            .installed
    );
    assert!(recorder.names().contains(&"connectors://changed"));
    assert!(recorder.names().contains(&"connector://changed"));

    let dir = tempfile::tempdir().unwrap();
    configure_connector(&state, "filesystem", shared_folder_config(dir.path()))
        .await
        .unwrap();
    let bad = BTreeMap::from([("api_token".to_string(), serde_yaml::Value::from("x"))]);
    let error = configure_connector(&state, "filesystem", bad)
        .await
        .unwrap_err();
    assert!(error.contains("credential"), "{error}");
    let fs = manifest(&state);
    assert_eq!(
        folders::shared_folders(fs.get("filesystem").unwrap())
            .unwrap()
            .len(),
        1
    );
    state.shutdown();
}

#[tokio::test]
async fn app_state_runs_connectors() {
    let state = AppState::for_tests(vec![]);
    install_connector(&state, "filesystem").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    configure_connector(&state, "filesystem", shared_folder_config(dir.path()))
        .await
        .unwrap();

    start_connector(&state, "filesystem").await.unwrap();
    let status = connector_status(&state, "filesystem").await.unwrap();
    assert!(status.installed && status.running, "{status:?}");
    stop_connector(&state, "filesystem").await.unwrap();
    let status = connector_status(&state, "filesystem").await.unwrap();
    assert!(status.installed && !status.running, "{status:?}");
    assert!(connector_status(&state, "nope").await.is_err());
    state.shutdown();
}
