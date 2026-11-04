//! Per-agent queues: what's waiting for each agent, and what it's working
//! on now. Changes go through the agent's orchestrator, so they're ordered
//! with the turns themselves; each one emits `agent://queue_changed`.

use crate::db;
use crate::model::{QueueEntry, RunSummary, STUDIO_AGENT_ID};
use crate::services::orchestrator::OrchestratorCommand;
use crate::services::{agents, AppState};

fn validate_agent(agent_id: &str) -> Result<(), String> {
    if agent_id == STUDIO_AGENT_ID {
        Ok(())
    } else {
        agents::validate_agent_id(agent_id)
    }
}

/// The agent's queued runs, next to run first.
pub async fn get_agent_queue(
    state: &AppState,
    agent_id: String,
) -> Result<Vec<QueueEntry>, String> {
    validate_agent(&agent_id)?;
    state
        .db()
        .call(move |conn| db::list_queue(conn, &agent_id))
        .await
}

/// The agent's running or approval-blocked turn, or `None` if it's idle.
pub async fn get_current_turn(
    state: &AppState,
    agent_id: String,
) -> Result<Option<RunSummary>, String> {
    validate_agent(&agent_id)?;
    state
        .db()
        .call(move |conn| db::active_agent_run(conn, &agent_id))
        .await
}

/// The agent whose queue holds `run_id`.
async fn queued_agent(state: &AppState, run_id: &str) -> Result<String, String> {
    let id = run_id.to_string();
    state
        .db()
        .call(move |conn| db::queued_run(conn, &id))
        .await?
        .map(|entry| entry.agent_id)
        .ok_or_else(|| "This run is not queued".to_string())
}

/// Removes a queued run (it ends as `cancelled`).
pub async fn cancel_queued_run(state: &AppState, run_id: String) -> Result<(), String> {
    let agent_id = queued_agent(state, &run_id).await?;
    state
        .orchestrators()
        .request(&agent_id, |reply| OrchestratorCommand::CancelQueued {
            run_id,
            reply,
        })
        .await
}

/// Moves a queued run: lower priorities run sooner.
pub async fn reprioritize_queued_run(
    state: &AppState,
    run_id: String,
    new_priority: i32,
) -> Result<(), String> {
    let agent_id = queued_agent(state, &run_id).await?;
    state
        .orchestrators()
        .request(&agent_id, |reply| OrchestratorCommand::Reprioritize {
            run_id,
            priority: new_priority,
            reply,
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::model::{RunStatus, SourceType};
    use crate::services::chat::{self, SendMessageResult};
    use crate::services::events::tests::RecordingSink;
    use crate::services::orchestrator::{
        Orchestrators, RunDeps, RunSource, PRIORITY_USER, PRIORITY_WORKFLOW_NORMAL,
    };

    const AGENT: &str = "writer";

    /// The canned engine with no token delay: each run suspends on an
    /// approval, so it stays the agent's current turn until resolved.
    async fn app() -> (AppState, Arc<RecordingSink>) {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        state
            .db()
            .call(|conn| {
                db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
                let mut script = crate::engine::default_script();
                script["tokenDelayMs"] = json!(0);
                db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
            })
            .await
            .unwrap();
        (state, recorder)
    }

    async fn send_via(o: &Orchestrators, text: &str, source: RunSource) -> SendMessageResult {
        let conversation = o
            .bus()
            .db
            .call(|conn| db::ensure_agent_conversation(conn, AGENT))
            .await
            .unwrap();
        chat::submit(o, AGENT, conversation, text.into(), vec![], None, source)
            .await
            .unwrap()
    }

    async fn send(state: &AppState, text: &str, source: RunSource) -> SendMessageResult {
        send_via(state.orchestrators(), text, source).await
    }

    async fn status(state: &AppState, run_id: &str) -> RunStatus {
        let id = run_id.to_string();
        state
            .db()
            .call(move |conn| db::get_run(conn, &id))
            .await
            .unwrap()
            .unwrap()
            .status
    }

    async fn queued(state: &AppState) -> Vec<String> {
        get_agent_queue(state, AGENT.into())
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.run_id)
            .collect()
    }

    /// Waits until the agent's current turn is `run_id` and has suspended.
    async fn wait_until_current(state: &AppState, run_id: &str) {
        for _ in 0..500 {
            let current = get_current_turn(state, AGENT.into()).await.unwrap();
            if current.is_some_and(|r| r.id == run_id && r.status == RunStatus::AwaitingApproval) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{run_id} never became the current turn");
    }

    fn count(recorder: &RecordingSink, name: &str) -> usize {
        recorder.names().iter().filter(|n| **n == name).count()
    }

    #[tokio::test]
    async fn send_when_idle_starts_immediately() {
        let (state, recorder) = app().await;
        let first = send(&state, "hello", RunSource::user()).await;

        assert!(!first.queued);
        assert_ne!(status(&state, &first.run_id).await, RunStatus::Queued);
        assert!(queued(&state).await.is_empty());
        let current = get_current_turn(&state, AGENT.into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.id, first.run_id);
        assert_eq!(current.source_type, SourceType::User);
        assert_eq!(current.priority, PRIORITY_USER);
        assert!(current.queued_at.is_none());
        assert!(recorder.names().contains(&"agent://queue_changed"));
        state.shutdown();
    }

    #[tokio::test]
    async fn send_during_a_turn_queues_instead_of_starting_a_second() {
        let (state, recorder) = app().await;

        // Rapid succession: the first starts, the second waits
        let first = send(&state, "first", RunSource::user()).await;
        let second = send(&state, "second", RunSource::user()).await;
        assert!(!first.queued);
        assert!(second.queued);
        assert_eq!(count(&recorder, "agent://queue_changed"), 2);

        wait_until_current(&state, &first.run_id).await;
        assert_eq!(status(&state, &second.run_id).await, RunStatus::Queued);
        assert_eq!(queued(&state).await, [second.run_id.as_str()]);
        let active = state.db().call(|conn| db::active_runs(conn)).await.unwrap();
        assert_eq!(active.len(), 1, "only one turn runs at a time");

        // The queued message joins the conversation only when its run starts
        let messages = state
            .db()
            .call(|conn| db::list_messages(conn, &db::agent_conversation_id(AGENT), 50))
            .await
            .unwrap();
        assert!(!messages.iter().any(|m| m.id == second.message_id));

        // The latest event carries the whole state
        let events = recorder.take();
        let (_, last) = events
            .iter()
            .rev()
            .find(|(name, _)| *name == "agent://queue_changed")
            .unwrap();
        assert_eq!(last["agentId"], AGENT);
        assert_eq!(last["current"]["id"], first.run_id.as_str());
        assert_eq!(last["queue"][0]["runId"], second.run_id.as_str());
        assert_eq!(last["queue"][0]["message"], "second");
        state.shutdown();
    }

    #[tokio::test]
    async fn the_next_run_starts_by_priority_when_the_turn_ends() {
        let (state, recorder) = app().await;
        let first = send(&state, "first", RunSource::user()).await;
        let workflow = send(
            &state,
            "workflow step",
            RunSource::workflow("wf-run", "Report · step 1".into(), false),
        )
        .await;
        let user = send(&state, "user follow-up", RunSource::user()).await;
        assert_eq!(
            queued(&state).await,
            [user.run_id.clone(), workflow.run_id.clone()],
            "the user's message jumps ahead of background work"
        );
        let entries = get_agent_queue(&state, AGENT.into()).await.unwrap();
        assert_eq!(entries[1].priority, PRIORITY_WORKFLOW_NORMAL);
        assert_eq!(entries[1].source_label.as_deref(), Some("Report · step 1"));

        wait_until_current(&state, &first.run_id).await;
        chat::cancel_turn(&state, AGENT.into(), first.run_id.clone())
            .await
            .unwrap();
        wait_until_current(&state, &user.run_id).await;
        assert_eq!(queued(&state).await, [workflow.run_id.as_str()]);

        // The dequeued run's message joined the conversation
        let id = user.message_id.clone();
        let message = state
            .db()
            .call(move |conn| db::get_message(conn, &id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.run_id.as_deref(), Some(user.run_id.as_str()));
        assert!(recorder
            .take()
            .iter()
            .any(|(name, payload)| *name == "agent://message"
                && payload["message"]["id"] == user.message_id.as_str()));
        state.shutdown();
    }

    #[tokio::test]
    async fn cancel_queued_removes_only_that_run() {
        let (state, recorder) = app().await;
        let first = send(&state, "first", RunSource::user()).await;
        let second = send(&state, "second", RunSource::user()).await;
        let third = send(&state, "third", RunSource::user()).await;
        recorder.take();

        cancel_queued_run(&state, second.run_id.clone())
            .await
            .unwrap();
        assert_eq!(status(&state, &second.run_id).await, RunStatus::Cancelled);
        assert_eq!(queued(&state).await, [third.run_id.as_str()]);
        let names = recorder.names();
        assert!(names.contains(&"agent://turn_done"));
        assert!(names.contains(&"agent://queue_changed"));

        // No longer queued; the current turn was never queued
        assert!(cancel_queued_run(&state, second.run_id.clone())
            .await
            .is_err());
        assert!(cancel_queued_run(&state, first.run_id.clone())
            .await
            .is_err());
        assert_ne!(status(&state, &first.run_id).await, RunStatus::Cancelled);
        state.shutdown();
    }

    #[tokio::test]
    async fn reprioritize_reorders_the_queue() {
        let (state, recorder) = app().await;
        send(&state, "first", RunSource::user()).await;
        let second = send(&state, "second", RunSource::user()).await;
        let third = send(&state, "third", RunSource::user()).await;
        assert_eq!(
            queued(&state).await,
            [second.run_id.clone(), third.run_id.clone()]
        );
        recorder.take();

        reprioritize_queued_run(&state, third.run_id.clone(), 1)
            .await
            .unwrap();
        assert_eq!(
            queued(&state).await,
            [third.run_id.clone(), second.run_id.clone()]
        );
        assert_eq!(count(&recorder, "agent://queue_changed"), 1);
        assert!(reprioritize_queued_run(&state, "missing".into(), 1)
            .await
            .is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn approve_and_cancel_never_touch_queued_runs() {
        let (state, _recorder) = app().await;
        let first = send(&state, "first", RunSource::user()).await;
        let second = send(&state, "second", RunSource::user()).await;
        wait_until_current(&state, &first.run_id).await;

        assert!(
            chat::cancel_turn(&state, AGENT.into(), second.run_id.clone())
                .await
                .is_err()
        );
        // Even sent straight to the orchestrator, Cancel ignores it
        state
            .orchestrators()
            .send(
                AGENT,
                OrchestratorCommand::Cancel {
                    run_id: second.run_id.clone(),
                },
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(status(&state, &second.run_id).await, RunStatus::Queued);
        assert_eq!(queued(&state).await, [second.run_id.as_str()]);
        state.shutdown();
    }

    #[tokio::test]
    async fn the_queue_survives_a_restart_and_drains_in_order() {
        let (state, _recorder) = app().await;
        let first = send(&state, "first", RunSource::user()).await;
        let low = send(
            &state,
            "low",
            RunSource::workflow("wf", "WF · step 1".into(), false),
        )
        .await;
        let high = send(&state, "high", RunSource::user()).await;
        wait_until_current(&state, &first.run_id).await;
        state.shutdown();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // A fresh registry over the same DB, as after an app restart: the
        // approval-blocked run is still current, the rest still queued
        let restarted = Orchestrators::new(
            RunDeps {
                bus: state.bus().clone(),
                secrets: state.secrets().clone(),
                runtimes: state.runtimes().clone(),
                store: state.store().clone(),
                connectors: state.connectors().cloned(),
                harness: Default::default(),
            },
            tokio::runtime::Handle::current(),
        );
        restarted.ensure(AGENT);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            queued(&state).await,
            [high.run_id.clone(), low.run_id.clone()]
        );

        restarted
            .send(
                AGENT,
                OrchestratorCommand::Cancel {
                    run_id: first.run_id.clone(),
                },
            )
            .await
            .unwrap();
        wait_until_current(&state, &high.run_id).await;
        assert_eq!(queued(&state).await, [low.run_id.as_str()]);

        // A new message after the restart still queues behind the turn
        let late = send_via(&restarted, "late", RunSource::user()).await;
        assert!(late.queued);
        assert_eq!(
            queued(&state).await,
            [late.run_id.clone(), low.run_id.clone()]
        );
        restarted.shutdown_all();
    }
}
