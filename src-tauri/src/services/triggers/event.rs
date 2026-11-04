//! Event source: fires on Nandi's own events, read from the broadcast bus.
//!
//! | `event_name`      | Payload                                                       |
//! | ----------------- | ------------------------------------------------------------- |
//! | `agent.turn_done` | `agent_id, run_id, status, error, source_type, source_id`     |
//! | `workflow.done`   | `workflow_id, run_id`                                         |
//! | `workflow.failed` | `workflow_id, run_id, step_id, error`                         |
//!
//! `filter` keys must equal the payload's, e.g. `{agent_id: "research-agent",
//! status: "failed"}` for "when research fails, retry elsewhere". A trigger
//! never fires on a run it dispatched itself (see `dispatch`).

use serde_json::{json, Map, Value};

use crate::db::{self, Db};
use crate::services::events::{AgentTurnDone, NandiEvent, WireEvent};
use crate::services::workflows::{WorkflowDone, WorkflowFailed};

/// Maps a bus event onto `(event_name, payload)`, if triggers can watch it.
pub async fn translate(db: &Db, event: &WireEvent) -> Option<(&'static str, Value)> {
    let p = &event.payload;
    let field = |name: &str| p.get(name).cloned().unwrap_or(Value::Null);
    match event.event {
        AgentTurnDone::NAME => {
            // Who started the run, so chains can filter on it (and a trigger
            // can ignore its own runs)
            let run_id = p.get("runId")?.as_str()?.to_string();
            let run = db
                .call(move |conn| db::get_run(conn, &run_id))
                .await
                .ok()
                .flatten();
            Some((
                "agent.turn_done",
                json!({
                    "agent_id": field("agentId"),
                    "run_id": field("runId"),
                    "status": field("status"),
                    "error": field("error"),
                    "source_type": run.as_ref().map(|r| r.source_type.as_str()),
                    "source_id": run.and_then(|r| r.source_id),
                }),
            ))
        }
        WorkflowDone::NAME => Some((
            "workflow.done",
            json!({ "workflow_id": field("workflowId"), "run_id": field("runId") }),
        )),
        WorkflowFailed::NAME => Some((
            "workflow.failed",
            json!({
                "workflow_id": field("workflowId"),
                "run_id": field("runId"),
                "step_id": field("stepId"),
                "error": field("error"),
            }),
        )),
        _ => None,
    }
}

/// Every filter entry equals the payload's value (strings compare to
/// strings; `"failed"` doesn't match `null`).
pub fn filter_matches(filter: &Map<String, Value>, payload: &Value) -> bool {
    filter.iter().all(|(key, expected)| {
        let actual = payload.get(key).unwrap_or(&Value::Null);
        // A YAML/JSON filter of `5` or `true` also matches the string form
        actual == expected
            || matches!(actual, Value::String(text) if !expected.is_string()
                && serde_json::from_str::<Value>(text).ok().as_ref() == Some(expected))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_on_payload_fields() {
        let payload = json!({"agent_id": "research-agent", "status": "failed", "error": null});
        let filter = |v: Value| v.as_object().unwrap().clone();
        assert!(filter_matches(&Map::new(), &payload));
        assert!(filter_matches(
            &filter(json!({"agent_id": "research-agent", "status": "failed"})),
            &payload
        ));
        assert!(!filter_matches(
            &filter(json!({"status": "completed"})),
            &payload
        ));
        assert!(!filter_matches(&filter(json!({"missing": "x"})), &payload));
        assert!(filter_matches(&filter(json!({"error": null})), &payload));
    }

    #[tokio::test]
    async fn translates_turn_done_with_the_run_source() {
        let db = Db::in_memory();
        db.call(|conn| {
            crate::db::tests::start_run(conn, "research-agent", "r1");
            Ok(())
        })
        .await
        .unwrap();
        let event = WireEvent {
            event: AgentTurnDone::NAME,
            payload: json!({"agentId": "research-agent", "runId": "r1", "status": "failed", "error": "boom"}),
        };
        let (name, payload) = translate(&db, &event).await.unwrap();
        assert_eq!(name, "agent.turn_done");
        assert_eq!(payload["status"], "failed");
        assert_eq!(payload["source_type"], "user");

        let other = WireEvent {
            event: "agent://token",
            payload: json!({}),
        };
        assert!(translate(&db, &other).await.is_none());
    }
}
