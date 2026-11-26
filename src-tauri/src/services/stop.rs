//! Stop means stop (owner, 2026-11-26): the Stop button, or the person
//! typing just "stop" in a chat with a running run ([`is_stop`]), ends the
//! run and everything it started:
//!
//! - the call in flight: the turn is cancelled, as Stop always did;
//! - the runs it delegated to other local agents, and theirs;
//! - the replies queued behind it in the same chat (dropped, said in the
//!   outcome and the record).
//!
//! If that agent's visible browser still has pages open, the person is asked
//! "Close the browser too?" ([`answer_browser_question`]); with no answer in
//! [`BROWSER_WAIT`] it closes, and the sites allowed once go with it. Keep it
//! open leaves the window and its sites (they last until it closes). The
//! record says who stopped and which runs.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::oneshot;

use crate::db;
use crate::model::{RunStatus, RunSummary};
use crate::services::domain::AgentCard;
use crate::services::inbox::audit::{self, Decided};
use crate::services::orchestrator::OrchestratorCommand;
use crate::services::AppState;

/// How long "Close the browser too?" waits before it closes it
pub const BROWSER_WAIT: Duration = Duration::from_secs(120);

/// The record's kind for a stop
const RECORD_KIND: &str = "stop";

/// What a stop did
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StopOutcome {
    /// The runs stopped: the one asked for first, then what it started
    pub stopped: Vec<String>,
    /// The replies that waited behind it, dropped
    pub dropped: Vec<String>,
    /// "Close the browser too?" is waiting for this answer, when the
    /// agent's browser still has pages open
    pub browser_question: Option<String>,
}

/// What a stop of one run touches, read from the database
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StopPlan {
    /// Going or waiting for an approval: their turns are cancelled
    pub cancel: Vec<RunSummary>,
    /// Waiting in a queue: they're taken out of it
    pub dequeue: Vec<RunSummary>,
}

/// Whether the person's message is just a stop, said plainly. No model:
/// "stop", "stop it", "stop now", "stop everything", "please stop", any
/// case, with trailing punctuation.
pub fn is_stop(text: &str) -> bool {
    let plain: String = text
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim()
        .to_lowercase();
    matches!(
        plain.as_str(),
        "stop" | "stop it" | "stop now" | "stop everything" | "stop that" | "please stop"
    )
}

fn is_live(run: &RunSummary) -> bool {
    matches!(run.status, RunStatus::Running | RunStatus::AwaitingApproval)
}

/// The run, the runs it delegated (and theirs) that haven't ended, and the
/// queued runs in its chat. Nothing when it has already ended.
pub fn plan(
    conn: &rusqlite::Connection,
    run_id: &str,
    instance: &str,
) -> rusqlite::Result<StopPlan> {
    let Some(run) = db::get_run(conn, run_id)? else {
        return Ok(StopPlan::default());
    };
    if !is_live(&run) {
        return Ok(StopPlan::default());
    }
    let mut plan = StopPlan::default();
    let mut seen = HashSet::from([run.id.clone()]);
    let mut todo = vec![run.clone()];
    plan.cancel.push(run.clone());
    while let Some(parent) = todo.pop() {
        let card = AgentCard::card_id(instance, &parent.agent_id);
        let children = db::delegated_runs(
            conn,
            &parent.agent_id,
            &card,
            parent.started_at as i64,
            parent.ended_at.map(|t| t as i64),
        )?;
        for child in children {
            if !seen.insert(child.id.clone()) {
                continue;
            }
            if is_live(&child) {
                plan.cancel.push(child.clone());
                todo.push(child);
            } else if child.status == RunStatus::Queued {
                plan.dequeue.push(child);
            }
        }
    }
    let mut stmt = conn.prepare(
        "SELECT id FROM runs WHERE conversation_id = ?1 AND status = 'queued'
          ORDER BY started_at",
    )?;
    let queued: Vec<String> = stmt
        .query_map([&run.conversation_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in queued {
        if seen.insert(id.clone()) {
            if let Some(queued) = db::get_run(conn, &id)? {
                plan.dequeue.push(queued);
            }
        }
    }
    Ok(plan)
}

/// Stops `run_id` and everything it started; `by` is who stopped it (the
/// person, or their phone)
pub async fn stop_run(state: &AppState, run_id: String, by: String) -> Result<StopOutcome, String> {
    let instance = crate::services::network::local_instance_id(state.db())?;
    let id = run_id.clone();
    let plan = state
        .db()
        .call(move |conn| plan(conn, &id, &instance))
        .await?;
    let Some(first) = plan.cancel.first().cloned() else {
        return Err("This run has already ended".to_string());
    };
    let mut outcome = StopOutcome::default();
    for run in &plan.cancel {
        let sent = state
            .orchestrators()
            .send(
                &run.agent_id,
                OrchestratorCommand::Cancel {
                    run_id: run.id.clone(),
                },
            )
            .await;
        match sent {
            Ok(()) => outcome.stopped.push(run.id.clone()),
            Err(e) => log::warn!("Stop: couldn't cancel run {}: {e}", run.id),
        }
    }
    for run in &plan.dequeue {
        match crate::services::queue::cancel_queued_run(state, run.id.clone()).await {
            Ok(()) => {
                if run.conversation_id == first.conversation_id {
                    outcome.dropped.push(run.id.clone());
                } else {
                    outcome.stopped.push(run.id.clone());
                }
            }
            Err(e) => log::warn!("Stop: couldn't drop queued run {}: {e}", run.id),
        }
    }
    let agent_name = state.store().display_name(&first.agent_id);
    audit::record(
        state,
        Decided::new(RECORD_KIND, &first.id, "stopped")
            .subject(format!(
                "{agent_name}'s run and {} it started",
                outcome.stopped.len().saturating_sub(1)
            ))
            .note(Some(&format!(
                "stopped: {}; queued replies dropped: {}",
                outcome.stopped.join(", "),
                if outcome.dropped.is_empty() {
                    "none".to_string()
                } else {
                    outcome.dropped.join(", ")
                }
            )))
            .by(by),
    );
    if crate::services::pilots::browser::window_open(&first.agent_id) {
        outcome.browser_question =
            Some(ask_browser(first.agent_id.clone(), BROWSER_WAIT, |agent| {
                Box::pin(
                    async move { crate::services::pilots::browser::close_window(&agent).await },
                )
            }));
    }
    Ok(outcome)
}

type Closer = fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// The questions waiting for an answer: question → Close (`true`) or Keep
static QUESTIONS: LazyLock<Mutex<HashMap<String, oneshot::Sender<bool>>>> =
    LazyLock::new(Default::default);

/// Asks "Close the browser too?" for `agent`: closes it on Close or after
/// `wait` with no answer; Keep it open leaves it. Returns the question's ID.
pub fn ask_browser(agent: String, wait: Duration, close: Closer) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = oneshot::channel();
    QUESTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), sender);
    let question = id.clone();
    tokio::spawn(async move {
        let close_it = match tokio::time::timeout(wait, receiver).await {
            Ok(Ok(close)) => close,
            // No answer in time: it closes
            Ok(Err(_)) | Err(_) => true,
        };
        QUESTIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&question);
        if close_it {
            close(agent).await;
        }
    });
    id
}

/// The person's answer to "Close the browser too?": `close` closes it now,
/// otherwise it stays open with its sites. `false` when the question has
/// gone (answered, or closed on its own).
pub fn answer_browser_question(question: &str, close: bool) -> bool {
    QUESTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(question)
        .is_some_and(|sender| sender.send(close).is_ok())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::model::SourceType;

    #[test]
    fn stop_said_plainly_is_a_stop() {
        for text in [
            "stop",
            "Stop",
            " STOP! ",
            "stop it.",
            "Stop now",
            "stop everything",
            "please stop",
        ] {
            assert!(is_stop(text), "{text}");
        }
        for text in [
            "stop the meeting reminder",
            "don't stop",
            "stopwatch",
            "",
            "cancel my 3pm",
        ] {
            assert!(!is_stop(text), "{text}");
        }
    }

    fn run(id: &str, agent: &str, status: RunStatus, at: f64, conversation: &str) -> RunSummary {
        RunSummary {
            id: id.into(),
            agent_id: agent.into(),
            conversation_id: conversation.into(),
            status,
            input: String::new(),
            summary: None,
            started_at: at,
            ended_at: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: None,
            model: None,
            source_type: SourceType::User,
            source_id: None,
            source_label: None,
            priority: 100,
            queued_at: None,
            parent_run_id: None,
            compaction_summary: None,
        }
    }

    #[test]
    fn a_stop_reaches_the_runs_it_started_and_the_replies_queued_behind_it() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let chat = db::chat_conversation_id("c1");
        db::insert_chat_conversation(&conn, "boss", &chat).unwrap();
        let task = db::ensure_task_conversation(&conn, "researcher", "t1", "Delegated").unwrap();
        let task2 = db::ensure_task_conversation(&conn, "writer", "t2", "Delegated").unwrap();
        db::insert_run(&conn, &run("p", "boss", RunStatus::Running, 10.0, &chat)).unwrap();
        let mut child = run("c", "researcher", RunStatus::AwaitingApproval, 20.0, &task);
        child.source_type = SourceType::Agent;
        child.source_id = Some("me/boss".into());
        db::insert_run(&conn, &child).unwrap();
        // The child delegated too
        let mut grandchild = run("g", "writer", RunStatus::Running, 25.0, &task2);
        grandchild.source_type = SourceType::Agent;
        grandchild.source_id = Some("researcher".into());
        db::insert_run(&conn, &grandchild).unwrap();
        // A reply queued behind it, and an ended run that stays as it is
        db::insert_run(&conn, &run("q", "boss", RunStatus::Queued, 30.0, &chat)).unwrap();
        db::insert_run(&conn, &run("old", "boss", RunStatus::Completed, 1.0, &chat)).unwrap();

        let plan = plan(&conn, "p", "me").unwrap();
        let ids = |runs: &[RunSummary]| runs.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&plan.cancel), ["p", "c", "g"]);
        assert_eq!(ids(&plan.dequeue), ["q"]);

        // Already ended: nothing to stop
        assert_eq!(plan_of(&conn, "old"), StopPlan::default());
    }

    fn plan_of(conn: &rusqlite::Connection, id: &str) -> StopPlan {
        plan(conn, id, "me").unwrap()
    }

    static CLOSED: AtomicUsize = AtomicUsize::new(0);

    fn counting_closer(
        _agent: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async {
            CLOSED.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[tokio::test]
    async fn the_browser_closes_unanswered_on_close_and_stays_on_keep() {
        let before = CLOSED.load(Ordering::SeqCst);
        // No answer: it closes when the wait ends
        let unanswered = ask_browser("a".into(), Duration::from_millis(30), counting_closer);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(CLOSED.load(Ordering::SeqCst), before + 1);
        assert!(
            !answer_browser_question(&unanswered, false),
            "gone once closed"
        );

        // Close: at once
        let close = ask_browser("a".into(), Duration::from_secs(60), counting_closer);
        assert!(answer_browser_question(&close, true));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(CLOSED.load(Ordering::SeqCst), before + 2);

        // Keep it open: never closed
        let keep = ask_browser("a".into(), Duration::from_millis(30), counting_closer);
        assert!(answer_browser_question(&keep, false));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(CLOSED.load(Ordering::SeqCst), before + 2);
    }
}
