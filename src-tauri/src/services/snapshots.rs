//! Snapshot commands: the only way the UI reads state. A view fetches a
//! snapshot when it needs one and applies `agent://*` / `dashboard://*`
//! events as deltas on top.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

use crate::db;
use crate::model::{
    ActivityAction, ActivityItem, AttentionItem, AttentionKind, ChatMessage, PendingApproval,
    QueueEntry, RunEventRecord, RunStatus, RunSummary, StudioConversationSummary, Timestamp,
    WorkflowSummary, STUDIO_AGENT_ID,
};
use crate::services::agents::{self, AgentInfo};
use crate::services::egress::capabilities::Capabilities;
use crate::services::events::{approval_attention, error_attention, truncate, RunContext};
use crate::services::runtimes::ContainerState;
use crate::services::triggers::schema::{TriggerFireSummary, TriggerStats};
use crate::services::triggers::store as trigger_store;
use crate::services::AppState;

const MESSAGE_LIMIT: u32 = 200;
const RECENT_RUNS: u32 = 10;
const RECENT_EVENTS: u32 = 200;
const RECENT_ACTIVITY: usize = 20;
const RECENT_OUTPUTS: usize = 6;
const RECENT_TRIGGER_FIRES: u32 = 6;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshot {
    pub conversation_id: String,
    /// The latest messages, oldest first
    pub messages: Vec<ChatMessage>,
    pub current_run: Option<RunSummary>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    /// `None` if the agent's folder no longer exists
    pub agent: Option<AgentInfo>,
    pub conversation: ConversationSnapshot,
    pub pending_approvals: Vec<PendingApproval>,
    pub recent_runs: Vec<RunSummary>,
    /// Runs waiting for the current turn, next to run first
    pub queue: Vec<QueueEntry>,
    /// The agent's running or approval-blocked turn
    pub current_turn: Option<RunSummary>,
    /// Raw persisted events, for the Activity tab
    pub recent_events: Vec<RunEventRecord>,
    /// The agent's runtime container
    pub container: ContainerState,
    /// What the egress proxy lets the agent reach, for the Permissions
    /// panel. From skill.md, or the last recorded copy if it doesn't parse.
    pub capabilities: Option<Capabilities>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub needs_attention: u32,
    pub running_agents: u32,
    pub runs_today: u32,
    pub failed_last_day: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentHealth {
    pub agent: AgentInfo,
    pub last_run_at: Option<Timestamp>,
    /// 0–1 over finished runs in the last 7 days
    pub success_rate_week: Option<f64>,
    pub avg_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunOutput {
    pub run: RunSummary,
    pub agent_name: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub stats: DashboardStats,
    pub trigger_stats: TriggerStats,
    /// Latest trigger fires, newest first
    pub recent_trigger_fires: Vec<TriggerFireSummary>,
    pub attention_items: Vec<AttentionItem>,
    pub recent_activity: Vec<ActivityItem>,
    pub agent_health: Vec<AgentHealth>,
    pub recent_outputs: Vec<RunOutput>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub run: RunSummary,
    pub agent_name: String,
    pub messages: Vec<ChatMessage>,
    pub events: Vec<RunEventRecord>,
}

fn conversation_snapshot(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<ConversationSnapshot> {
    Ok(ConversationSnapshot {
        conversation_id: conversation_id.to_string(),
        messages: db::list_messages(conn, conversation_id, MESSAGE_LIMIT)?
            .iter()
            .filter(|m| !m.is_continuation())
            .map(db::MessageRow::to_chat)
            .collect(),
        current_run: db::active_run(conn, conversation_id)?,
    })
}

pub async fn get_agent_snapshot(
    state: &AppState,
    agent_id: String,
) -> Result<AgentSnapshot, String> {
    let db = state.db();
    agents::validate_agent_id(&agent_id)?;
    let (store, id) = (state.store().clone(), agent_id.clone());
    let (mut agent, parsed) = tokio::task::spawn_blocking(move || {
        let info = store.agent_info(&id);
        let capabilities = info
            .as_ref()
            .and_then(|_| store.skill(&id).ok())
            .map(|skill| skill.capabilities);
        (info, capabilities)
    })
    .await
    .map_err(|e| format!("Agent read failed: {e}"))?;
    if let Some(agent) = agent.as_mut() {
        agents::apply_run_status(db, std::slice::from_mut(agent)).await?;
    }

    let id = agent_id.clone();
    let (conversation, pending_approvals, recent_runs, queue, current_turn, recent_events, stored) =
        db.call(move |conn| {
            let conversation_id = db::ensure_agent_conversation(conn, &id)?;
            Ok((
                conversation_snapshot(conn, &conversation_id)?,
                db::pending_approvals(conn, Some(&id))?,
                db::list_runs(conn, Some(&id), RECENT_RUNS)?,
                db::list_queue(conn, &id)?,
                db::active_agent_run(conn, &id)?,
                db::list_agent_events(conn, &id, RECENT_EVENTS)?,
                crate::services::projection::agent_capabilities(conn, &id)?,
            ))
        })
        .await?;
    let capabilities =
        parsed.or_else(|| stored.and_then(|value| serde_json::from_value(value).ok()));

    // A container stopped outside Nandi shouldn't still read `running`
    state.runtimes().reconcile(&agent_id).await;

    Ok(AgentSnapshot {
        agent,
        conversation,
        pending_approvals,
        recent_runs,
        queue,
        current_turn,
        recent_events,
        container: state.runtimes().state(&agent_id),
        capabilities,
    })
}

/// A Studio conversation's messages and active run.
pub async fn get_conversation(
    state: &AppState,
    conversation_id: String,
) -> Result<ConversationSnapshot, String> {
    let db = state.db();
    let id = conversation_id.clone();
    let owner = db
        .call(move |conn| db::conversation_agent(conn, &id))
        .await?;
    if owner.as_deref() != Some(STUDIO_AGENT_ID) {
        return Err("That conversation was not found".to_string());
    }
    db.call(move |conn| conversation_snapshot(conn, &conversation_id))
        .await
}

pub async fn get_studio_conversations(
    state: &AppState,
) -> Result<Vec<StudioConversationSummary>, String> {
    let db = state.db();
    db.call(|conn| db::list_studio_conversations(conn)).await
}

pub async fn get_workflows(state: &AppState) -> Result<Vec<WorkflowSummary>, String> {
    let db = state.db();
    db.call(|conn| db::list_workflows(conn)).await
}

/// Where an agent's container sees its workdir
const CONTAINER_WORKDIR: &str = "/agent/workdir";

/// The host file a tool call's `path` names, given the agent's host
/// `workdir` (mounted at `/agent/workdir`). Relative and container paths
/// resolve inside the workdir and may not climb out of it (`..`); an
/// absolute host path (a CLI runtime on this computer) is taken as is.
pub fn workdir_file(workdir: &Path, path: &str) -> Option<PathBuf> {
    use std::path::Component;
    let path = path.trim();
    let inside = match path.strip_prefix(CONTAINER_WORKDIR) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest.trim_start_matches('/'),
        _ if Path::new(path).is_absolute() => return Some(PathBuf::from(path)),
        _ => path,
    };
    let relative = Path::new(inside);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(workdir.join(relative))
}

/// The host path of a file the run touched, if it's there (Open, Reveal,
/// Copy path in Run Detail)
pub async fn run_file_path(
    state: &AppState,
    run_id: String,
    path: String,
) -> Result<Option<String>, String> {
    let id = run_id.clone();
    let run = state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await?
        .ok_or_else(|| format!("Run {run_id} not found"))?;
    let workdir = crate::utils::nandi_config::work_dir()?.join(&run.agent_id);
    Ok(workdir_file(&workdir, &path)
        .filter(|file| file.exists())
        .map(|file| file.to_string_lossy().into_owned()))
}

/// The run's plan, if it has one (its checklist card)
pub async fn get_run_plan(
    state: &AppState,
    run_id: String,
) -> Result<Option<crate::services::run_plan::RunPlan>, String> {
    state
        .db()
        .call(move |conn| crate::services::run_plan::store::current(conn, &run_id))
        .await
}

/// Screenshots shown as thumbnails are at most this big
const MAX_SCREENSHOT_BYTES: u64 = 8 * 1024 * 1024;

/// One of the run's screenshots (`browser/<run>/…`) as a `data:` URL, for
/// its plan card; `None` once it's gone (removed after 30 days)
pub async fn run_screenshot(
    state: &AppState,
    run_id: String,
    path: String,
) -> Result<Option<String>, String> {
    use base64::Engine;
    if !crate::services::run_plan::screenshots::is_run_screenshot(&run_id, &path) {
        return Err(format!("'{path}' isn't one of this run's screenshots"));
    }
    let id = run_id.clone();
    let run = state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await?
        .ok_or_else(|| format!("Run {run_id} not found"))?;
    let workdir = crate::utils::nandi_config::work_dir()?.join(&run.agent_id);
    let Some(file) = workdir_file(&workdir, &path) else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        let Ok(meta) = std::fs::metadata(&file) else {
            return Ok(None);
        };
        if !meta.is_file() || meta.len() > MAX_SCREENSHOT_BYTES {
            return Ok(None);
        }
        let bytes = std::fs::read(&file).map_err(|e| format!("Couldn't read {path}: {e}"))?;
        let mime = if path.to_ascii_lowercase().ends_with(".png") {
            "image/png"
        } else {
            "image/jpeg"
        };
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(Some(format!("data:{mime};base64,{data}")))
    })
    .await
    .map_err(|e| format!("Reading the screenshot failed: {e}"))?
}

pub async fn get_run_detail(state: &AppState, run_id: String) -> Result<RunDetail, String> {
    let db = state.db();
    let id = run_id.clone();
    let (run, messages, events) = db
        .call(move |conn| {
            let run = db::get_run(conn, &id)?;
            let messages = db::list_run_messages(conn, &id)?;
            let mut stmt = conn.prepare(
                "SELECT id, run_id, kind, payload, created_at FROM run_events WHERE run_id = ?1 ORDER BY id",
            )?;
            let events = stmt
                .query_map([&id], |row| {
                    let payload: String = row.get(3)?;
                    Ok(RunEventRecord {
                        id: row.get::<_, i64>(0)? as f64,
                        run_id: row.get(1)?,
                        kind: row.get(2)?,
                        payload: serde_json::from_str(&payload).unwrap_or_default(),
                        created_at: db::ts(row.get(4)?),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((run, messages, events))
        })
        .await?;
    let run = run.ok_or_else(|| format!("Run {run_id} not found"))?;
    let agent_name = if run.agent_id == STUDIO_AGENT_ID {
        "Nandi".to_string()
    } else {
        state.store().display_name(&run.agent_id)
    };
    Ok(RunDetail {
        run,
        agent_name,
        messages: messages
            .iter()
            .filter(|m| !m.is_continuation())
            .map(db::MessageRow::to_chat)
            .collect(),
        events,
    })
}

/// `day_start` is local midnight in ms, from the UI (Rust doesn't know the
/// user's timezone), for the "Runs Today" count.
pub async fn get_dashboard_snapshot(
    state: &AppState,
    day_start: f64,
) -> Result<DashboardSnapshot, String> {
    let db = state.db();
    let store = state.store().clone();
    let mut agents = tokio::task::spawn_blocking(move || store.list_agents())
        .await
        .map_err(|e| format!("Agent scan failed: {e}"))?;
    agents::apply_run_status(db, &mut agents).await?;
    let now = db::now_ms();
    db.call(move |conn| build_dashboard(conn, agents, now, day_start as i64))
        .await
}

fn build_dashboard(
    conn: &Connection,
    agents: Vec<AgentInfo>,
    now: i64,
    day_start: i64,
) -> rusqlite::Result<DashboardSnapshot> {
    let names: HashMap<String, String> = agents
        .iter()
        .map(|a| (a.id.clone(), a.name.clone()))
        .collect();
    let name_of = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_string());
    let context = |run: &RunSummary| RunContext {
        agent_id: run.agent_id.clone(),
        agent_name: name_of(&run.agent_id),
        conversation_id: run.conversation_id.clone(),
        run_id: run.id.clone(),
    };

    let week = db::runs_since(conn, now - 7 * DAY_MS)?;
    let pending: Vec<PendingApproval> = db::pending_approvals(conn, None)?
        .into_iter()
        .filter(|a| a.agent_id != STUDIO_AGENT_ID)
        .collect();

    let failed_today: Vec<&RunSummary> = week
        .iter()
        .filter(|r| {
            r.status == RunStatus::Failed
                && r.ended_at.unwrap_or(r.started_at) as i64 >= now - DAY_MS
        })
        .collect();

    let mut attention_items: Vec<AttentionItem> = pending
        .iter()
        .map(|a| {
            let ctx = RunContext {
                agent_id: a.agent_id.clone(),
                agent_name: name_of(&a.agent_id),
                conversation_id: a.conversation_id.clone(),
                run_id: a.run_id.clone(),
            };
            approval_attention(&ctx, &a.id, &a.tool, &a.args, a.created_at as i64)
        })
        .chain(
            failed_today
                .iter()
                .map(|run| error_attention(&context(run), run, Some(format!("error-{}", run.id)))),
        )
        .collect();
    // Workflows waiting on a person (wave EF): "Onboarding needs you"
    let manual_steps = crate::services::workflows::manual_steps::pending_conn(conn)?;
    attention_items.extend(manual_steps.iter().map(|step| AttentionItem {
        id: format!("manual-{}-{}", step.run_id, step.step_id),
        kind: AttentionKind::ManualStep,
        agent_id: String::new(),
        agent_name: step.workflow_name.clone(),
        run_id: step.run_id.clone(),
        message_id: None,
        detail: truncate(&step.instructions, 200),
        created_at: step.ready_at,
    }));
    attention_items.sort_by(|a, b| b.created_at.total_cmp(&a.created_at));

    let running_agents: HashSet<&str> = week
        .iter()
        .filter(|r| r.status.is_active())
        .map(|r| r.agent_id.as_str())
        .collect();

    let stats = DashboardStats {
        needs_attention: (pending.len() + manual_steps.len()) as u32,
        running_agents: running_agents.len() as u32,
        runs_today: week
            .iter()
            .filter(|r| r.started_at as i64 >= day_start)
            .count() as u32,
        failed_last_day: failed_today.len() as u32,
    };

    // Activity: run starts/ends plus resolved approvals, newest first
    let mut recent_activity: Vec<ActivityItem> = Vec::new();
    for run in week.iter().take(RECENT_ACTIVITY * 2) {
        let ctx = context(run);
        let item = |action: ActivityAction, at: Timestamp, key: &str| ActivityItem {
            id: format!("{}:{key}", run.id),
            agent_id: ctx.agent_id.clone(),
            agent_name: ctx.agent_name.clone(),
            run_id: run.id.clone(),
            action,
            subject: truncate(&run.input, 120),
            created_at: at,
        };
        recent_activity.push(item(ActivityAction::Started, run.started_at, "started"));
        if let Some(ended) = run.ended_at {
            let (action, key) = match run.status {
                RunStatus::Completed => (ActivityAction::Completed, "completed"),
                RunStatus::Cancelled => (ActivityAction::Cancelled, "cancelled"),
                _ => (ActivityAction::Failed, "failed"),
            };
            recent_activity.push(item(action, ended, key));
        }
    }
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, run_id, tool, status, resolved_at FROM approvals
          WHERE status IN ('approved', 'rejected') AND resolved_at >= ?1 AND agent_id != ?2
          ORDER BY resolved_at DESC LIMIT ?3",
    )?;
    let resolved = stmt.query_map(
        params![now - 7 * DAY_MS, STUDIO_AGENT_ID, RECENT_ACTIVITY as i64],
        |row| {
            let status: String = row.get(4)?;
            let agent_id: String = row.get(1)?;
            Ok(ActivityItem {
                id: format!("{}:{status}", row.get::<_, String>(0)?),
                agent_name: name_of(&agent_id),
                agent_id,
                run_id: row.get(2)?,
                action: if status == "approved" {
                    ActivityAction::Approved
                } else {
                    ActivityAction::Rejected
                },
                subject: row.get(3)?,
                created_at: db::ts(row.get(5)?),
            })
        },
    )?;
    for item in resolved {
        recent_activity.push(item?);
    }
    recent_activity.sort_by(|a, b| b.created_at.total_cmp(&a.created_at));
    recent_activity.truncate(RECENT_ACTIVITY);

    let agent_health = agents
        .into_iter()
        .map(|agent| {
            let runs: Vec<&RunSummary> = week.iter().filter(|r| r.agent_id == agent.id).collect();
            let finished: Vec<&&RunSummary> =
                runs.iter().filter(|r| r.status.is_finished()).collect();
            let last_run_at = db::list_runs(conn, Some(&agent.id), 1)?
                .first()
                .map(|r| r.started_at);
            Ok(AgentHealth {
                success_rate_week: (!finished.is_empty()).then(|| {
                    finished
                        .iter()
                        .filter(|r| r.status == RunStatus::Completed)
                        .count() as f64
                        / finished.len() as f64
                }),
                avg_duration_ms: (!finished.is_empty()).then(|| {
                    finished
                        .iter()
                        .map(|r| r.ended_at.unwrap_or(r.started_at) - r.started_at)
                        .sum::<f64>()
                        / finished.len() as f64
                }),
                last_run_at,
                agent,
            })
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let recent_outputs = db::list_runs(conn, None, 50)?
        .into_iter()
        .filter(|r| r.status == RunStatus::Completed)
        .take(RECENT_OUTPUTS)
        .map(|run| RunOutput {
            agent_name: name_of(&run.agent_id),
            run,
        })
        .collect();

    Ok(DashboardSnapshot {
        stats,
        trigger_stats: trigger_store::stats(conn, day_start)?,
        recent_trigger_fires: trigger_store::recent_fires(conn, RECENT_TRIGGER_FIRES)?,
        attention_items,
        recent_activity,
        agent_health,
        recent_outputs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::start_run;
    use crate::services::agents::AgentStatus;

    #[test]
    fn run_files_resolve_inside_the_workdir_only() {
        let workdir = Path::new("/w/writer");
        let cases: [(&str, Option<&str>); 9] = [
            ("notes.md", Some("/w/writer/notes.md")),
            ("./out/report.md", Some("/w/writer/./out/report.md")),
            (
                "/agent/workdir/out/report.md",
                Some("/w/writer/out/report.md"),
            ),
            ("/agent/workdir", None),
            ("../escape.txt", None),
            ("out/../../escape.txt", None),
            ("", None),
            // Not the workdir mount, just a name that starts like it
            ("/agent/workdirx/a.txt", Some("/agent/workdirx/a.txt")),
            // A CLI runtime on this computer names host paths
            (
                "/home/ada/project/src/app.ts",
                Some("/home/ada/project/src/app.ts"),
            ),
        ];
        for (path, expected) in cases {
            // Windows reads `/home/...` as drive-relative, not absolute
            if cfg!(windows) && expected.is_some_and(|e| !e.starts_with("/w/")) {
                continue;
            }
            let expected = expected.map(PathBuf::from);
            assert_eq!(workdir_file(workdir, path), expected, "{path:?}");
        }
    }

    fn agent(id: &str) -> AgentInfo {
        AgentInfo {
            id: id.into(),
            name: id.to_uppercase(),
            description: String::new(),
            skill_path: String::new(),
            status: AgentStatus::Idle,
            current_task: None,
            origin: crate::services::agents::AgentOrigin::Local,
            builtin: false,
        }
    }

    #[test]
    fn dashboard_aggregates_runs_and_approvals() {
        let db = db::Db::in_memory();
        let conn = db.conn().unwrap();
        start_run(&conn, "a", "r1");
        db::set_run_status(&conn, "r1", RunStatus::Completed, Some("All done")).unwrap();
        start_run(&conn, "a", "r2");
        db::set_run_status(&conn, "r2", RunStatus::Failed, Some("boom")).unwrap();
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
                args: serde_json::json!({"cmd": "ls"}),
                status: crate::model::ApprovalStatus::Pending,
                created_at: db::now_ms(),
                expires_at: None,
                decided_by: None,
            },
        )
        .unwrap();

        let now = db::now_ms();
        let dashboard =
            build_dashboard(&conn, vec![agent("a"), agent("b")], now, now - 1000).unwrap();

        assert_eq!(dashboard.stats.needs_attention, 1);
        assert_eq!(dashboard.stats.running_agents, 1);
        assert_eq!(dashboard.stats.runs_today, 3);
        assert_eq!(dashboard.stats.failed_last_day, 1);
        let kinds: Vec<_> = dashboard
            .attention_items
            .iter()
            .map(|i| (i.id.as_str(), i.agent_name.as_str()))
            .collect();
        assert!(kinds.contains(&("ap", "B")));
        assert!(kinds.contains(&("r2", "A")));
        assert_eq!(dashboard.recent_outputs.len(), 1);
        assert_eq!(
            dashboard.recent_outputs[0].run.summary.as_deref(),
            Some("All done")
        );

        let health_a = dashboard
            .agent_health
            .iter()
            .find(|h| h.agent.id == "a")
            .unwrap();
        assert_eq!(health_a.success_rate_week, Some(0.5));
        assert!(health_a.last_run_at.is_some());
        let actions: HashSet<_> = dashboard
            .recent_activity
            .iter()
            .map(|i| i.id.clone())
            .collect();
        assert!(actions.contains("r1:completed"));
        assert!(actions.contains("r2:failed"));
        assert!(actions.contains("r3:started"));
    }
}
