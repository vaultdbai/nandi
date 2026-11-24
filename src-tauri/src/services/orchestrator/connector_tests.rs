//! Tests: the tools an agent is given and how its calls are checked (the
//! catalog, policies and connectors)

use super::*;
use crate::services::domain::{Agent, AgentId, EntityKind};
use crate::services::repos::Repository;
use crate::services::AppState;
use serde_json::json;

fn deps(state: &AppState) -> RunDeps {
    RunDeps {
        bus: state.bus().clone(),
        secrets: state.secrets().clone(),
        runtimes: state.runtimes().clone(),
        store: state.store().clone(),
        connectors: Some(state.connectors().clone()),
        harness: Default::default(),
    }
}

fn save_agent(state: &AppState, id: &str, frontmatter: &str) {
    let agent = Agent::from_agent_md(
        AgentId::parse(id).unwrap(),
        &format!("---\nname: {id}\n{frontmatter}---\nHelp the user.\n"),
    )
    .unwrap();
    state.store().repos().agents.save(&agent).unwrap();
    state.store().commit(EntityKind::Agent, id).unwrap();
}

fn context(agent_id: &str) -> RunContext {
    RunContext {
        agent_id: agent_id.into(),
        agent_name: agent_id.into(),
        conversation_id: "c1".into(),
        run_id: "r1".into(),
    }
}

fn container_tools() -> Vec<ToolSpec> {
    ["read_file", "shell"]
        .into_iter()
        .map(|name| ToolSpec {
            name: name.into(),
            description: String::new(),
            input_schema: json!({"type": "object"}),
        })
        .collect()
}

#[tokio::test]
async fn a_tool_switched_off_in_the_catalog_doesnt_run() {
    let state = AppState::for_tests(vec![]);
    save_agent(&state, "writer", "");
    state
        .tools()
        .unwrap()
        .set_enabled("shell", false)
        .await
        .unwrap();
    let call = PlannedCall {
        id: "t1".into(),
        tool: "shell".into(),
        args: json!({"cmd": "echo hi"}),
        requires_approval: false,
    };
    let outcome = execute_tool(&deps(&state), &context("writer"), &call)
        .await
        .unwrap();
    assert!(outcome.is_error);
    assert!(
        outcome.result.contains("switched off"),
        "{}",
        outcome.result
    );
    let disabled = state
        .db()
        .call(|conn| crate::services::tools::disabled_tool_names(conn))
        .await
        .unwrap();
    assert_eq!(disabled, HashSet::from(["shell".to_string()]));
    state.shutdown();
}

#[tokio::test]
async fn a_tool_or_pilot_a_policy_denies_is_refused_not_run() {
    let state = AppState::for_tests(vec![]);
    save_agent(
        &state,
        "writer",
        "pilots: [browser]
",
    );
    let policy = state.store().layout().policy_path("locked-down");
    std::fs::create_dir_all(policy.parent().unwrap()).unwrap();
    std::fs::write(
        &policy,
        "---
applies_to: [agent:writer]
tools: { deny: [shell] }
pilots: { allow: [] }
---
",
    )
    .unwrap();
    let call = |tool: &str| PlannedCall {
        id: "t1".into(),
        tool: tool.into(),
        args: json!({}),
        requires_approval: false,
    };

    // `run_call` refuses it before any approval, as the policy's
    let skill = state.store().skill("writer").unwrap();
    let refused =
        call_gate::denied_by_policy(&deps(&state), &context("writer"), &call("shell"), &skill)
            .expect("refused by the policy");
    assert!(refused.is_error);
    assert!(
        refused.result.contains("a policy doesn't allow it"),
        "{}",
        refused.result
    );
    // and the runtime never gets it either
    let shell = execute_tool(&deps(&state), &context("writer"), &call("shell"))
        .await
        .unwrap();
    assert!(shell.is_error);
    assert!(
        shell
            .result
            .contains("not allowed by this agent's agent.md or its policies"),
        "{}",
        shell.result
    );
    assert_eq!(
        state.runtimes().state("writer").status,
        crate::services::runtimes::ContainerStatus::Idle,
        "no container was started for it"
    );

    let pilot = pilots::tools::tool_name("browser", "navigate");
    let navigate = execute_tool(&deps(&state), &context("writer"), &call(&pilot))
        .await
        .unwrap();
    assert!(navigate.is_error);
    assert!(
        navigate
            .result
            .contains("a policy doesn't allow this pilot"),
        "{}",
        navigate.result
    );
    state.shutdown();
}

/// Install the filesystem connector, point it at a folder, and let an
/// agent that declares it read a file through it: the full path the
/// orchestrator takes, with an explicit registry.
#[tokio::test]
async fn an_agent_uses_the_filesystem_connector_and_the_call_is_audited() {
    let state = AppState::for_tests(vec![]);
    let deps = deps(&state);
    let shared = tempfile::tempdir().unwrap();
    std::fs::write(shared.path().join("hello.txt"), "hello from the host").unwrap();
    connectors::install_connector(&state, "filesystem")
        .await
        .unwrap();
    let config = connectors::folders::folders_config(&[connectors::folders::SharedFolder {
        id: "shared".into(),
        path: shared.path().display().to_string(),
        access: connectors::folders::Access::Read,
    }]);
    connectors::configure_connector(&state, "filesystem", config)
        .await
        .unwrap();
    save_agent(
        &state,
        "files-agent",
        "connectors: [{id: filesystem, folders: [shared]}]\n",
    );

    let log = shared.path().join("audit").join("connectors.log");
    let registry = ConnectorRegistry::with_audit_log(log.clone());
    let mut tools = container_tools();
    let sensitive = merge_connector_tools(Some(&registry), &deps, "files-agent", &mut tools).await;
    let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "read_file",
            "shell",
            "filesystem__read_file",
            "filesystem__write_file",
            "filesystem__list_dir"
        ]
    );
    assert_eq!(
        sensitive,
        HashSet::from(["filesystem__write_file".to_string()])
    );

    let call = PlannedCall {
        id: "call-1".into(),
        tool: "filesystem__read_file".into(),
        args: json!({"path": "hello.txt"}),
        requires_approval: false,
    };
    let outcome = connector_call(Some(&registry), &deps, &context("files-agent"), &call)
        .await
        .expect("routed to the connector");
    assert!(!outcome.is_error, "{}", outcome.result);
    assert_eq!(outcome.result, "hello from the host");

    // Container tools are left to the container
    let shell = PlannedCall {
        tool: "shell".into(),
        ..call.clone()
    };
    assert!(
        connector_call(Some(&registry), &deps, &context("files-agent"), &shell)
            .await
            .is_none()
    );

    let audit = std::fs::read_to_string(&log).unwrap();
    let entry: serde_json::Value = serde_json::from_str(audit.lines().next().unwrap()).unwrap();
    assert_eq!(entry["agentId"], "files-agent");
    assert_eq!(entry["connectorId"], "filesystem");
    assert_eq!(entry["tool"], "read_file");
    assert_eq!(entry["outcome"], "ok");
    assert!(
        !audit.contains("hello.txt"),
        "argument values stay out of the log"
    );
    registry.shutdown_all().await;
    state.shutdown();
}

#[tokio::test]
async fn agents_that_dont_declare_a_connector_dont_get_it() {
    let state = AppState::for_tests(vec![]);
    let deps = deps(&state);
    connectors::install_connector(&state, "filesystem")
        .await
        .unwrap();
    save_agent(&state, "plain-agent", "");
    let registry = ConnectorRegistry::default();

    let mut tools = container_tools();
    let sensitive = merge_connector_tools(Some(&registry), &deps, "plain-agent", &mut tools).await;
    assert_eq!(tools.len(), 2);
    assert!(sensitive.is_empty());
    let call = PlannedCall {
        id: "call-1".into(),
        tool: "filesystem__read_file".into(),
        args: json!({"path": "x"}),
        requires_approval: false,
    };
    assert!(
        connector_call(Some(&registry), &deps, &context("plain-agent"), &call)
            .await
            .is_none()
    );
    state.shutdown();
}

#[tokio::test]
async fn a_connector_that_cant_start_warns_the_agent() {
    let recorder = Arc::new(crate::services::events::tests::RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    let deps = deps(&state);
    // Installed without a folder: it can't start
    connectors::install_connector(&state, "filesystem")
        .await
        .unwrap();
    save_agent(&state, "files-agent", "connectors: [filesystem]\n");
    let registry = ConnectorRegistry::default();

    let mut tools = container_tools();
    merge_connector_tools(Some(&registry), &deps, "files-agent", &mut tools).await;
    assert_eq!(tools.len(), 2, "its tools are left out");
    assert!(recorder.names().contains(&"agent://warning"));
    state.shutdown();
}
