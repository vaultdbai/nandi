//! The floating companion: a pocket Home + Studio (v1 task 16). Its lists
//! come from Home's queries in the webview; this module keeps the orb's
//! snapshot, the companion's preferences, and its chat. Showing, hiding,
//! and pinning the window is Tauri-only, so it lives in
//! `desktop/companion.rs`; this module is transport-independent.
//!
//! The companion's chat is one Studio conversation, created on first use
//! (titled "Companion", so Studio lists it under that name) and remembered
//! in `local_settings` ([`CONVERSATION_KEY`]). Studio looks things up with
//! its own read-only tools (`studio::tools`); the companion adds only a
//! structured scope (the Request the conversation is about, [`SCOPE_KEY`]),
//! which Studio's prompt reads ([`prompt_context`]). "@Agent …" sends the
//! rest of the message to that agent instead.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::chat::{self, SendMessageResult};
use super::orchestrator::RunSource;
use super::runtimes::ContainerStatus;
use super::AppState;
use crate::db;
use crate::model::{PendingApproval, RunStatus, RunSummary, STUDIO_AGENT_ID};

/// What the companion shows at a glance.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompanionSnapshot {
    pub running: u32,
    /// Runs waiting for an approval
    pub waiting: u32,
    pub completed_today: u32,
    pub failed_today: u32,
    /// `healthy` | `degraded` | `down`
    pub health: String,
    /// Pending approval requests (a run can wait on more than one)
    pub pending_approvals: u32,
    /// The "keep on top" preference
    pub pinned: bool,
    /// The companion's Studio conversation, once the first message created it
    pub conversation_id: Option<String>,
    /// Something that stops work everywhere (the orb's red state)
    pub blocker: Option<CompanionBlocker>,
    /// The "Notify me" preference (on by default)
    pub notify: bool,
    /// "Notify me when requests come back" (on by default)
    pub notify_results: bool,
    /// The Request the conversation is scoped to, if any
    pub scope_request_id: Option<String>,
    /// The bison stays on the desktop when the card is closed (on by
    /// default)
    pub bubble: bool,
    /// Nandi starts with only the bison (the main window hidden)
    pub start_bison_only: bool,
}

/// Why nothing can run right now
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CompanionBlocker {
    /// No model can be called (`work-model.md` §2.3 `no_model`)
    NoModel,
}

/// `local_settings` key for the "keep on top" preference
pub const PINNED_KEY: &str = "companion.pinned";
/// `local_settings` key for the companion's Studio conversation ID
pub const CONVERSATION_KEY: &str = "companion.conversation_id";
/// `local_settings` key: the bison stays on the desktop (absent: on)
pub const BUBBLE_KEY: &str = "companion.bubble";
/// `local_settings` key: Nandi starts with only the bison (absent: off)
pub const START_BISON_ONLY_KEY: &str = "companion.start_bison_only";
/// `local_settings` key: the screen corner the companion sits in, and the
/// monitor's name (`{corner, monitor}`)
pub const CORNER_KEY: &str = "companion.corner";
/// `local_settings` key for "Notify me" (absent: on)
pub const NOTIFY_KEY: &str = "companion.notify";
/// `local_settings` key: the Request the companion conversation is about
pub const SCOPE_KEY: &str = "companion.scope";
/// The companion conversation's name in Studio's list
pub const CONVERSATION_TITLE: &str = "Companion";

pub const HEALTHY: &str = "healthy";
pub const DEGRADED: &str = "degraded";
pub const DOWN: &str = "down";

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Agent health inputs that don't come from the runs table.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AgentHealthCounts {
    pub agents: usize,
    /// Agents whose orchestrator or container is in error
    pub erroring: usize,
}

/// The snapshot from a window of runs (Studio runs are ignored), pending
/// approvals, and agent health. Pure, for tests.
pub fn compute_snapshot(
    runs: &[RunSummary],
    pending: &[PendingApproval],
    health: AgentHealthCounts,
    day_start: i64,
) -> CompanionSnapshot {
    let agent_runs = || runs.iter().filter(|r| r.agent_id != STUDIO_AGENT_ID);
    let count = |status: RunStatus| agent_runs().filter(|r| r.status == status).count() as u32;
    let ended_today = |status: RunStatus| {
        agent_runs()
            .filter(|r| r.status == status && r.ended_at.is_some_and(|t| t as i64 >= day_start))
            .count() as u32
    };
    let failed_today = ended_today(RunStatus::Failed);
    let health = if health.agents > 0 && health.erroring >= health.agents {
        DOWN
    } else if health.erroring > 0 || failed_today > 0 {
        DEGRADED
    } else {
        HEALTHY
    };
    CompanionSnapshot {
        running: count(RunStatus::Running),
        waiting: count(RunStatus::AwaitingApproval),
        completed_today: ended_today(RunStatus::Completed),
        failed_today,
        health: health.to_string(),
        pending_approvals: pending
            .iter()
            .filter(|a| a.agent_id != STUDIO_AGENT_ID)
            .count() as u32,
        pinned: false,
        conversation_id: None,
        blocker: None,
        notify: true,
        notify_results: true,
        scope_request_id: None,
        bubble: true,
        start_bison_only: false,
    }
}

/// Runs that may count today: started since the day before `day_start`
/// (a run can start yesterday and end today), plus anything still active.
fn snapshot_runs(conn: &Connection, day_start: i64) -> rusqlite::Result<Vec<RunSummary>> {
    let mut runs = db::runs_since(conn, day_start - DAY_MS)?;
    for run in db::active_runs(conn)? {
        if !runs.iter().any(|r| r.id == run.id) {
            runs.push(run);
        }
    }
    Ok(runs)
}

fn bool_setting(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    bool_setting_or(conn, key, false)
}

fn bool_setting_or(conn: &Connection, key: &str, default: bool) -> rusqlite::Result<bool> {
    Ok(db::get_setting(conn, key)?
        .and_then(|v| v.as_bool())
        .unwrap_or(default))
}

fn scope_setting(conn: &Connection) -> rusqlite::Result<Option<String>> {
    Ok(db::get_setting(conn, SCOPE_KEY)?
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|id| !id.is_empty()))
}

fn put_setting(conn: &Connection, key: &str, value: &serde_json::Value) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

/// The remembered companion conversation, if it still exists
fn stored_conversation(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let Some(id) =
        db::get_setting(conn, CONVERSATION_KEY)?.and_then(|v| v.as_str().map(str::to_string))
    else {
        return Ok(None);
    };
    Ok((db::conversation_agent(conn, &id)?.as_deref() == Some(STUDIO_AGENT_ID)).then_some(id))
}

/// The companion conversation, created (untitled) on first use
fn ensure_conversation(conn: &Connection) -> rusqlite::Result<String> {
    if let Some(id) = stored_conversation(conn)? {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    // Named up front, so Studio lists it as the companion's
    db::insert_studio_conversation(conn, &id, CONVERSATION_TITLE)?;
    put_setting(conn, CONVERSATION_KEY, &serde_json::json!(id))?;
    Ok(id)
}

async fn health_counts(state: &AppState) -> Result<AgentHealthCounts, String> {
    let store = state.store().clone();
    let agent_ids: Vec<String> = tokio::task::spawn_blocking(move || {
        store.list_agents().into_iter().map(|a| a.id).collect()
    })
    .await
    .map_err(|e| format!("Agent scan failed: {e}"))?;
    let orchestrator_errors: Vec<String> = state
        .db()
        .call(|conn| db::agents_in_error(conn))
        .await?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let erroring = agent_ids
        .iter()
        .filter(|id| {
            orchestrator_errors.contains(id)
                || state.runtimes().state(id).status == ContainerStatus::Error
        })
        .count();
    Ok(AgentHealthCounts {
        agents: agent_ids.len(),
        erroring,
    })
}

/// `day_start`: local midnight, epoch milliseconds (like the dashboard)
pub async fn get_companion_snapshot(
    state: &AppState,
    day_start: f64,
) -> Result<CompanionSnapshot, String> {
    let health = health_counts(state).await?;
    let blocker = crate::services::llm_keys::get_default_model(state)
        .await
        .ok()
        .and_then(|model| {
            model
                .effective_model
                .is_none()
                .then_some(CompanionBlocker::NoModel)
        });
    let day_start = day_start as i64;
    state
        .db()
        .call(move |conn| {
            let runs = snapshot_runs(conn, day_start)?;
            let pending = db::pending_approvals(conn, None)?;
            Ok(CompanionSnapshot {
                pinned: bool_setting(conn, PINNED_KEY)?,
                conversation_id: stored_conversation(conn)?,
                notify: bool_setting_or(conn, NOTIFY_KEY, true)?,
                notify_results: bool_setting_or(
                    conn,
                    crate::services::a2a::returns::NOTIFY_RESULTS_KEY,
                    true,
                )?,
                scope_request_id: scope_setting(conn)?,
                bubble: bool_setting_or(conn, BUBBLE_KEY, true)?,
                start_bison_only: bool_setting(conn, START_BISON_ONLY_KEY)?,
                blocker,
                ..compute_snapshot(&runs, &pending, health, day_start)
            })
        })
        .await
}

/// Saves "Notify me"
pub async fn set_notify(state: &AppState, notify: bool) -> Result<(), String> {
    state
        .db()
        .call(move |conn| put_setting(conn, NOTIFY_KEY, &serde_json::json!(notify)))
        .await
}

/// Saves "Notify me when requests come back"
pub async fn set_notify_results(state: &AppState, notify: bool) -> Result<(), String> {
    state
        .db()
        .call(move |conn| {
            put_setting(
                conn,
                crate::services::a2a::returns::NOTIFY_RESULTS_KEY,
                &serde_json::json!(notify),
            )
        })
        .await
}

/// Saves the "keep on top" preference
pub async fn set_pinned(state: &AppState, pinned: bool) -> Result<(), String> {
    state
        .db()
        .call(move |conn| put_setting(conn, PINNED_KEY, &serde_json::json!(pinned)))
        .await
}

/// Saves whether the bison stays on the desktop
pub async fn set_bubble(state: &AppState, bubble: bool) -> Result<(), String> {
    state
        .db()
        .call(move |conn| put_setting(conn, BUBBLE_KEY, &serde_json::json!(bubble)))
        .await
}

/// Saves whether Nandi starts with only the bison
pub async fn set_start_bison_only(state: &AppState, on: bool) -> Result<(), String> {
    state
        .db()
        .call(move |conn| put_setting(conn, START_BISON_ONLY_KEY, &serde_json::json!(on)))
        .await
}

/// Whether to start with only the bison: the setting, with the bison on
pub async fn starts_bison_only(state: &AppState) -> Result<bool, String> {
    state
        .db()
        .call(|conn| {
            Ok(bool_setting(conn, START_BISON_ONLY_KEY)?
                && bool_setting_or(conn, BUBBLE_KEY, true)?)
        })
        .await
}

pub async fn is_bubble(state: &AppState) -> Result<bool, String> {
    state
        .db()
        .call(|conn| bool_setting_or(conn, BUBBLE_KEY, true))
        .await
}

/// Where the companion sits: a screen corner on a named monitor
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CornerPlace {
    pub corner: Corner,
    /// `None`: the primary monitor
    #[serde(default)]
    pub monitor: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

/// The saved corner (bottom-right of the primary monitor until moved)
pub async fn corner(state: &AppState) -> Result<CornerPlace, String> {
    state
        .db()
        .call(|conn| {
            Ok(db::get_setting(conn, CORNER_KEY)?
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default())
        })
        .await
}

pub async fn set_corner(state: &AppState, place: CornerPlace) -> Result<(), String> {
    state
        .db()
        .call(move |conn| {
            put_setting(
                conn,
                CORNER_KEY,
                &serde_json::to_value(&place).unwrap_or_default(),
            )
        })
        .await
}

pub async fn is_pinned(state: &AppState) -> Result<bool, String> {
    state.db().call(|conn| bool_setting(conn, PINNED_KEY)).await
}

/// "@Research Assistant summarize this" → `(agent id, "summarize this")`.
/// The longest agent name (or ID) right after `@` wins, case-insensitively,
/// and must end at a word boundary. `agents` are `(id, name)`. Pure.
pub fn mention_target(text: &str, agents: &[(String, String)]) -> Option<(String, String)> {
    let rest = text.trim_start().strip_prefix('@')?;
    agents
        .iter()
        .flat_map(|(id, name)| [(id, name.trim()), (id, id.as_str())])
        .filter(|(_, label)| !label.is_empty())
        .filter_map(|(id, label)| {
            let head = rest.get(..label.len())?;
            let tail = &rest[label.len()..];
            let boundary = tail
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || c == ',' || c == ':');
            (boundary && head.to_lowercase() == label.to_lowercase()).then_some((id, label, tail))
        })
        .max_by_key(|(_, label, _)| label.len())
        .map(|(id, _, tail)| {
            let message = tail.trim_start_matches([',', ':']).trim().to_string();
            (id.clone(), message)
        })
}

/// What Studio's prompt adds when the turn is the companion's conversation
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompanionPromptContext {
    /// The Request the user pinned the conversation to
    pub request_id: Option<String>,
}

/// The companion context for `conversation_id`, if it's the companion's
pub async fn prompt_context(
    state: &AppState,
    conversation_id: &str,
) -> Option<CompanionPromptContext> {
    let conversation_id = conversation_id.to_string();
    state
        .db()
        .call(move |conn| {
            let ours = stored_conversation(conn)?.as_deref() == Some(conversation_id.as_str());
            Ok(ours
                .then(|| scope_setting(conn))
                .transpose()?
                .map(|request_id| CompanionPromptContext { request_id }))
        })
        .await
        .ok()
        .flatten()
}

/// A companion chat turn in the companion's Studio conversation (or
/// `conversation_id`), which answers from its tools. `scope_request_id`
/// pins the conversation to a Request (`None` clears it). A message
/// starting "@Agent" goes to that agent's own conversation instead.
pub async fn send_companion_message(
    state: &AppState,
    text: &str,
    conversation_id: Option<&str>,
    scope_request_id: Option<&str>,
) -> Result<SendMessageResult, String> {
    let question = text.trim().to_string();
    if question.is_empty() {
        return Err("Message is empty".to_string());
    }
    let agents: Vec<(String, String)> = state
        .store()
        .list_agents()
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect();
    if let Some((agent_id, message)) = mention_target(&question, &agents) {
        if message.is_empty() {
            return Err("Say what to ask after the agent's name".to_string());
        }
        return chat::send_message(state, agent_id, message, Vec::new(), None).await;
    }

    let requested = conversation_id.map(str::to_string);
    let scope = scope_request_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let (conversation_id, owner) = state
        .db()
        .call(move |conn| {
            let id = match requested {
                Some(id) => id,
                None => ensure_conversation(conn)?,
            };
            put_setting(
                conn,
                SCOPE_KEY,
                &scope.map_or(serde_json::Value::Null, serde_json::Value::String),
            )?;
            Ok((id.clone(), db::conversation_agent(conn, &id)?))
        })
        .await?;
    if owner.as_deref() != Some(STUDIO_AGENT_ID) {
        return Err("That conversation was not found".to_string());
    }

    chat::submit(
        state.orchestrators(),
        STUDIO_AGENT_ID,
        conversation_id,
        question,
        Vec::new(),
        None,
        RunSource::user(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::start_run;
    use crate::model::{ApprovalStatus, SourceType};

    fn run(id: &str, agent: &str, status: RunStatus, ended_at: Option<i64>) -> RunSummary {
        RunSummary {
            id: id.into(),
            agent_id: agent.into(),
            conversation_id: format!("agent:{agent}"),
            status,
            input: format!("task {id}"),
            summary: (status == RunStatus::Completed).then(|| format!("done {id}")),
            started_at: 1_000.0,
            ended_at: ended_at.map(|t| t as f64),
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

    fn approval(id: &str, agent: &str) -> PendingApproval {
        PendingApproval {
            id: id.into(),
            agent_id: agent.into(),
            run_id: "r".into(),
            conversation_id: format!("agent:{agent}"),
            tool: "shell".into(),
            args: serde_json::json!({}),
            created_at: 0.0,
            expires_at: None,
        }
    }

    const DAY_START: i64 = 10_000;

    fn fixture() -> (Vec<RunSummary>, Vec<PendingApproval>) {
        (
            vec![
                run("r1", "a", RunStatus::Running, None),
                run("r2", "b", RunStatus::Running, None),
                run("r3", "b", RunStatus::AwaitingApproval, None),
                run("r4", "a", RunStatus::Completed, Some(DAY_START + 5)),
                run("r5", "a", RunStatus::Completed, Some(DAY_START + 9)),
                // Yesterday: not today's
                run("r6", "a", RunStatus::Completed, Some(DAY_START - 5)),
                run("r7", "c", RunStatus::Cancelled, Some(DAY_START + 1)),
                // Studio is the companion's own chat: never counted
                run("s1", STUDIO_AGENT_ID, RunStatus::Running, None),
                run(
                    "s2",
                    STUDIO_AGENT_ID,
                    RunStatus::Completed,
                    Some(DAY_START + 1),
                ),
            ],
            vec![
                approval("ap1", "b"),
                approval("ap2", "b"),
                approval("ap3", STUDIO_AGENT_ID),
            ],
        )
    }

    const THREE_HEALTHY: AgentHealthCounts = AgentHealthCounts {
        agents: 3,
        erroring: 0,
    };

    #[test]
    fn snapshot_counts_runs_and_approvals() {
        let (runs, pending) = fixture();
        let snapshot = compute_snapshot(&runs, &pending, THREE_HEALTHY, DAY_START);
        assert_eq!(
            snapshot,
            CompanionSnapshot {
                running: 2,
                waiting: 1,
                completed_today: 2,
                failed_today: 0,
                health: HEALTHY.into(),
                pending_approvals: 2,
                pinned: false,
                conversation_id: None,
                blocker: None,
                notify: true,
                notify_results: true,
                scope_request_id: None,
                bubble: true,
                start_bison_only: false,
            }
        );
    }

    #[test]
    fn health_degrades_on_failures_and_errors_and_is_down_when_all_error() {
        let (mut runs, pending) = fixture();
        // A failure yesterday doesn't count
        runs.push(run("f0", "a", RunStatus::Failed, Some(DAY_START - 1)));
        let snapshot = compute_snapshot(&runs, &pending, THREE_HEALTHY, DAY_START);
        assert_eq!(
            (snapshot.failed_today, snapshot.health.as_str()),
            (0, HEALTHY)
        );

        runs.push(run("f1", "a", RunStatus::Failed, Some(DAY_START + 1)));
        let snapshot = compute_snapshot(&runs, &pending, THREE_HEALTHY, DAY_START);
        assert_eq!(
            (snapshot.failed_today, snapshot.health.as_str()),
            (1, DEGRADED)
        );

        let (runs, _) = fixture();
        let erroring = |erroring| AgentHealthCounts {
            agents: 3,
            erroring,
        };
        assert_eq!(
            compute_snapshot(&runs, &[], erroring(1), DAY_START).health,
            DEGRADED
        );
        assert_eq!(
            compute_snapshot(&runs, &[], erroring(3), DAY_START).health,
            DOWN
        );
        // No agents yet is not "down"
        assert_eq!(
            compute_snapshot(&[], &[], AgentHealthCounts::default(), DAY_START).health,
            HEALTHY
        );
    }

    #[test]
    fn mentions_route_to_the_named_agent() {
        let agents = vec![
            ("research".to_string(), "Research".to_string()),
            ("research-pro".to_string(), "Research Assistant".to_string()),
            ("ünï".to_string(), "Ünï Bot".to_string()),
        ];
        let target = |text: &str| mention_target(text, &agents);
        // The longest matching name wins
        assert_eq!(
            target("@Research Assistant summarize this"),
            Some(("research-pro".into(), "summarize this".into()))
        );
        assert_eq!(
            target("  @research: what's new?"),
            Some(("research".into(), "what's new?".into()))
        );
        // By ID, and with non-ASCII names
        assert_eq!(
            target("@research-pro go"),
            Some(("research-pro".into(), "go".into()))
        );
        assert_eq!(target("@ünï bot hi"), Some(("ünï".into(), "hi".into())));
        // A name must end at a word boundary; no @ is a plain question
        assert_eq!(target("@Researchers unite"), None);
        assert_eq!(target("@nobody hi"), None);
        assert_eq!(target("what did Research do?"), None);
        assert_eq!(
            target("@Research"),
            Some(("research".into(), String::new()))
        );
    }

    #[test]
    fn snapshot_reads_the_database() {
        let db = db::Db::in_memory();
        let conn = db.conn().unwrap();
        let day_start = db::now_ms() - 1_000;
        start_run(&conn, "a", "r1");
        start_run(&conn, "a", "r2");
        db::set_run_status(&conn, "r2", RunStatus::Completed, Some("ok")).unwrap();
        start_run(&conn, "b", "r3");
        db::set_run_status(&conn, "r3", RunStatus::AwaitingApproval, None).unwrap();
        db::insert_approval(
            &conn,
            &db::ApprovalRow {
                id: "ap".into(),
                agent_id: "b".into(),
                run_id: "r3".into(),
                message_id: "ap".into(),
                tool_call_id: "c".into(),
                tool: "shell".into(),
                args: serde_json::json!({}),
                status: ApprovalStatus::Pending,
                created_at: db::now_ms(),
                expires_at: None,
                decided_by: None,
            },
        )
        .unwrap();

        let runs = snapshot_runs(&conn, day_start).unwrap();
        let pending = db::pending_approvals(&conn, None).unwrap();
        let healthy = AgentHealthCounts {
            agents: 2,
            erroring: 0,
        };
        let snapshot = compute_snapshot(&runs, &pending, healthy, day_start);
        assert_eq!(snapshot.running, 1);
        assert_eq!(snapshot.waiting, 1);
        assert_eq!(snapshot.completed_today, 1);
        assert_eq!(snapshot.pending_approvals, 1);
    }

    #[test]
    fn settings_round_trip_and_conversation_is_created_once() {
        let db = db::Db::in_memory();
        let conn = db.conn().unwrap();
        assert!(!bool_setting(&conn, PINNED_KEY).unwrap());
        put_setting(&conn, PINNED_KEY, &serde_json::json!(true)).unwrap();
        assert!(bool_setting(&conn, PINNED_KEY).unwrap());

        assert_eq!(stored_conversation(&conn).unwrap(), None);
        let first = ensure_conversation(&conn).unwrap();
        assert_eq!(ensure_conversation(&conn).unwrap(), first);
        assert_eq!(stored_conversation(&conn).unwrap(), Some(first.clone()));
        assert_eq!(
            db::conversation_agent(&conn, &first).unwrap().as_deref(),
            Some(STUDIO_AGENT_ID)
        );
    }

    #[tokio::test]
    async fn service_snapshot_pin_and_send() {
        let state = AppState::for_tests(vec![]);
        assert!(!is_pinned(&state).await.unwrap());
        set_pinned(&state, true).await.unwrap();
        assert!(is_pinned(&state).await.unwrap());
        // The bison is on by default, and the corner is bottom-right
        assert!(is_bubble(&state).await.unwrap());
        set_bubble(&state, false).await.unwrap();
        assert!(!is_bubble(&state).await.unwrap());
        assert_eq!(corner(&state).await.unwrap(), CornerPlace::default());
        // Starting with only the bison needs the bison
        set_start_bison_only(&state, true).await.unwrap();
        assert!(
            !starts_bison_only(&state).await.unwrap(),
            "the bison is off"
        );
        set_bubble(&state, true).await.unwrap();
        assert!(starts_bison_only(&state).await.unwrap());
        set_bubble(&state, false).await.unwrap();
        let place = CornerPlace {
            corner: Corner::TopLeft,
            monitor: Some("DELL".into()),
        };
        set_corner(&state, place.clone()).await.unwrap();
        assert_eq!(corner(&state).await.unwrap(), place);

        let snapshot = get_companion_snapshot(&state, 0.0).await.unwrap();
        assert!(snapshot.pinned);
        assert!(!snapshot.bubble);
        assert_eq!(snapshot.conversation_id, None);
        assert_eq!(snapshot.health, HEALTHY);
        // No key in tests: nothing can run
        assert_eq!(snapshot.blocker, Some(CompanionBlocker::NoModel));
        assert!(snapshot.notify, "notifications are on by default");
        set_notify(&state, false).await.unwrap();
        assert!(!get_companion_snapshot(&state, 0.0).await.unwrap().notify);

        assert!(send_companion_message(&state, "  ", None, None)
            .await
            .is_err());
        assert!(
            send_companion_message(&state, "hi", Some("agent:nope"), None)
                .await
                .is_err()
        );
        let sent = send_companion_message(&state, "Why is it stuck?", None, Some("task-1"))
            .await
            .unwrap();
        // Named "Companion" up front: the question doesn't rename it
        assert_eq!(sent.conversation_title, None);
        let snapshot = get_companion_snapshot(&state, 0.0).await.unwrap();
        let id = snapshot.conversation_id.expect("created");
        assert_eq!(snapshot.scope_request_id.as_deref(), Some("task-1"));
        let conversations = crate::services::snapshots::get_studio_conversations(&state)
            .await
            .unwrap();
        assert_eq!(
            conversations
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.title.as_str()),
            Some(CONVERSATION_TITLE)
        );
        // Studio's prompt reads the scope for this conversation only
        assert_eq!(
            prompt_context(&state, &id).await,
            Some(CompanionPromptContext {
                request_id: Some("task-1".into())
            })
        );
        assert_eq!(prompt_context(&state, "other").await, None);

        // No scope clears it
        send_companion_message(&state, "Anything else?", None, None)
            .await
            .unwrap();
        assert_eq!(
            prompt_context(&state, &id).await,
            Some(CompanionPromptContext::default())
        );
        state.shutdown();
    }
}
