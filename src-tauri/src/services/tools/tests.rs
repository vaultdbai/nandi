use std::sync::Arc;

use serde_json::json;

use super::definition::LatencyClass;
use super::*;
use crate::db::{self, MessageRow};
use crate::services::events::tests::RecordingSink;
use crate::services::events::EventSink;

fn service() -> (ToolService, Arc<RecordingSink>, Db) {
    let db = Db::in_memory();
    let recorder = Arc::new(RecordingSink::default());
    let sink: Arc<dyn EventSink> = recorder.clone();
    let bus = EventBus::new(db.clone(), sink);
    (
        ToolService::new(db.clone(), bus, None).unwrap(),
        recorder,
        db,
    )
}

fn tool(id: &str, source: ToolSource) -> ToolDefinition {
    let name = id.rsplit('.').next().unwrap().to_string();
    ToolDefinition {
        id: id.to_string(),
        name,
        description: String::new(),
        source,
        input_schema: None,
        requires_approval: false,
        cost_estimate: None,
        latency_class: LatencyClass::Fast,
        tags: Vec::new(),
    }
}

fn pilot(id: &str) -> ToolSource {
    ToolSource::Pilot {
        pilot_id: id.to_string(),
    }
}

fn ids(entries: &[ToolEntry]) -> Vec<String> {
    entries.iter().map(|e| e.definition.id.clone()).collect()
}

fn with_builtins(extra: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
    let mut all = builtin::builtin_tools();
    all.extend(extra);
    all
}

#[tokio::test]
async fn starts_with_the_built_ins_persisted() {
    let (tools, recorder, db) = service();
    let listed = tools.list_all().await.unwrap();
    assert_eq!(listed.len(), builtin::builtin_tools().len());
    assert!(listed.iter().all(|e| e.enabled));
    let rows: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM tool_catalog WHERE source = 'builtin'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows as usize, listed.len());
    // Seeding isn't a change anyone needs to hear about
    assert!(recorder.names().is_empty());
}

#[tokio::test]
async fn a_rebuild_with_new_tools_emits_once() {
    let (tools, recorder, _db) = service();
    let gmail = tool(
        "gmail.send_email",
        ToolSource::Connector {
            connector_id: "gmail".into(),
        },
    );
    assert!(tools
        .rebuild_from(with_builtins(vec![gmail.clone()]), |_| false)
        .await
        .unwrap());
    assert!(!tools
        .rebuild_from(with_builtins(vec![gmail]), |_| false)
        .await
        .unwrap());
    assert_eq!(recorder.names(), [TOOL_CATALOG_CHANGED]);
    assert!(tools.catalog().get("gmail__send_email").is_some());
}

#[tokio::test]
async fn a_stopped_pilot_keeps_its_tools_and_an_uninstalled_one_loses_them() {
    let (tools, _recorder, _db) = service();
    let click = tool("opendesk.click", pilot("opendesk"));
    tools
        .rebuild_from(with_builtins(vec![click]), |_| false)
        .await
        .unwrap();
    assert!(tools.catalog().get("pilot_opendesk__click").is_some());

    // Installed but not running: listed by nobody, kept
    tools
        .rebuild_from(with_builtins(vec![]), |source| *source == pilot("opendesk"))
        .await
        .unwrap();
    assert!(tools.catalog().get("opendesk.click").is_some());

    // Uninstalled
    tools
        .rebuild_from(with_builtins(vec![]), |_| false)
        .await
        .unwrap();
    assert!(tools.catalog().get("opendesk.click").is_none());
}

#[tokio::test]
async fn disabling_a_tool_keeps_it_in_the_catalog_across_rebuilds() {
    let (tools, recorder, _db) = service();
    let entry = tools.set_enabled("shell", false).await.unwrap();
    assert_eq!(entry.definition.id, "builtin.shell");
    assert!(!entry.enabled);
    assert_eq!(recorder.names(), [TOOL_CATALOG_CHANGED]);

    tools
        .rebuild_from(builtin::builtin_tools(), |_| false)
        .await
        .unwrap();
    let listed = tools.list_all().await.unwrap();
    assert!(ids(&listed).contains(&"builtin.shell".to_string()));
    let enabled: Vec<_> = listed.iter().filter(|e| e.enabled).collect();
    assert_eq!(enabled.len(), listed.len() - 1);
    assert!(!tools.get("builtin.shell").await.unwrap().enabled);

    assert!(tools.set_enabled("nope", false).await.is_err());
}

#[tokio::test]
async fn a_capability_requiring_shell_resolves() {
    let (tools, _recorder, _db) = service();
    assert!(tools.missing_tools(&["shell".to_string()]).is_empty());
    assert_eq!(
        tools.missing_tools(&["shell".to_string(), "slack.post".to_string()]),
        ["slack.post"]
    );
}

#[tokio::test]
async fn usage_comes_from_tool_call_messages() {
    let (tools, _recorder, db) = service();
    {
        let conn = db.conn().unwrap();
        conn.execute(
            "INSERT INTO conversations (id, agent_id, created_at) VALUES ('c', 'a', 1)",
            [],
        )
        .unwrap();
        for (i, (name, is_error)) in [("shell", false), ("shell", true), ("read_file", false)]
            .into_iter()
            .enumerate()
        {
            let meta = json!({ "tool": name, "args": {}, "status": "done", "isError": is_error });
            let message =
                MessageRow::new(format!("m{i}"), "c", None, db::role::TOOL_CALL, "", meta);
            db::insert_message(&conn, &message).unwrap();
        }
    }
    let stats = tools.usage_stats().await.unwrap();
    let shell = stats.iter().find(|s| s.tool_id == "builtin.shell").unwrap();
    assert_eq!((shell.calls, shell.failures), (2, 1));
    assert!(shell.last_used_at.is_some());
    let listed = tools.list_all().await.unwrap();
    let usage = |id: &str| {
        listed
            .iter()
            .find(|e| e.definition.id == id)
            .unwrap()
            .usage_count
    };
    assert_eq!(usage("builtin.shell"), 2);
    assert_eq!(usage("builtin.read_file"), 1);
    assert_eq!(usage("builtin.write_file"), 0);
}

#[tokio::test]
async fn installing_a_connector_adds_its_tools_to_the_app_catalog() {
    let state = AppState::for_tests(vec![]);
    let tools = state.tools().expect("tool catalog initialized").clone();
    tools.rebuild().await.unwrap();
    assert!(tools.catalog().by_source("connector").is_empty());

    crate::services::connectors::install_connector(&state, "filesystem")
        .await
        .unwrap();
    tools.rebuild().await.unwrap();
    let catalog = tools.catalog();
    let connector_tools: Vec<_> = catalog
        .by_source("connector")
        .into_iter()
        .map(|d| d.id.clone())
        .collect();
    assert!(
        connector_tools.contains(&"filesystem.read_file".to_string()),
        "{connector_tools:?}"
    );
    assert!(
        catalog
            .get("filesystem__write_file")
            .unwrap()
            .definition
            .requires_approval
    );

    let listed = list_tools(&state).await.unwrap();
    assert!(ids(&listed).contains(&"builtin.shell".to_string()));
}
