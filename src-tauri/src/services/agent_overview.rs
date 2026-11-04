//! What the Agents list shows per agent: how it's set up (runtime, where a
//! CLI runtime runs, its model, whether it takes requests) and how it did
//! over the last seven days, one cell per day. Read-only: definitions come
//! from the repo aggregate, runs from SQLite.

use serde::Serialize;
use specta::Type;

use crate::db;
use crate::model::{RunStatus, RunSummary, Timestamp, STUDIO_AGENT_ID};
use crate::services::domain::{Agent, AgentRuntimeKind};
use crate::services::AppState;

const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
/// Days in the health window (today included)
pub const DAYS: usize = 7;

/// One day's runs
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentDay {
    pub completed: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentOverview {
    pub agent_id: String,
    /// `native`, `claude-code`, `codex`, `gemini-cli`, `custom:<id>`
    pub runtime: String,
    /// The agent's own model (`<provider>:<model>`); `None`: the default.
    /// For a CLI runtime, its `runtime_config.model`
    pub model: Option<String>,
    pub accepts_requests: bool,
    pub runs: u32,
    pub completed: u32,
    pub failed: u32,
    /// Finished runs only
    pub avg_duration_ms: Option<f64>,
    pub cost_usd: f64,
    /// Oldest first; the last cell is today
    pub days: Vec<AgentDay>,
    pub last_run_at: Option<Timestamp>,
}

/// How `agent` is set up, with no runs counted yet
fn setup(agent: &Agent) -> AgentOverview {
    let native = matches!(agent.runtime(), AgentRuntimeKind::Native);
    let config = agent.runtime_config();
    AgentOverview {
        agent_id: agent.id().to_string(),
        runtime: agent.runtime().as_str().into_owned(),
        model: if native {
            agent.model().map(ToString::to_string)
        } else {
            config.and_then(|c| c.model.clone())
        },
        accepts_requests: agent.capabilities().accepts_delegation(),
        runs: 0,
        completed: 0,
        failed: 0,
        avg_duration_ms: None,
        cost_usd: 0.0,
        days: vec![AgentDay::default(); DAYS],
        last_run_at: None,
    }
}

/// Adds the runs of the window ending with `today_start`'s day. Pure.
pub fn count_runs(overview: &mut AgentOverview, runs: &[RunSummary], today_start: f64) {
    let window_start = today_start - (DAYS as f64 - 1.0) * DAY_MS;
    let mut durations = Vec::new();
    for run in runs
        .iter()
        .filter(|r| r.agent_id == overview.agent_id && r.started_at >= window_start)
    {
        overview.runs += 1;
        overview.cost_usd += run.cost_usd.unwrap_or(0.0);
        overview.last_run_at = Some(
            overview
                .last_run_at
                .map_or(run.started_at, |last| last.max(run.started_at)),
        );
        let day = (((run.started_at - window_start) / DAY_MS) as usize).min(DAYS - 1);
        match run.status {
            RunStatus::Completed | RunStatus::Compacted => {
                overview.completed += 1;
                overview.days[day].completed += 1;
            }
            RunStatus::Failed => {
                overview.failed += 1;
                overview.days[day].failed += 1;
            }
            _ => {}
        }
        if let Some(ended) = run.ended_at {
            durations.push(ended - run.started_at);
        }
    }
    overview.avg_duration_ms =
        (!durations.is_empty()).then(|| durations.iter().sum::<f64>() / durations.len() as f64);
}

/// Every agent, with the last seven days ending today (`today_start`:
/// local midnight, epoch ms)
pub async fn list_agent_overviews(
    state: &AppState,
    today_start: f64,
) -> Result<Vec<AgentOverview>, String> {
    let since = (today_start - (DAYS as f64 - 1.0) * DAY_MS) as i64;
    let runs = state
        .db()
        .call(move |conn| db::runs_since(conn, since))
        .await?;
    Ok(state.store().read(|repo| {
        repo.agents()
            .filter(|agent| agent.id().as_str() != STUDIO_AGENT_ID)
            .map(|agent| {
                let mut overview = setup(agent);
                count_runs(&mut overview, &runs, today_start);
                overview
            })
            .collect()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceType;
    use crate::services::domain::AgentId;

    fn agent(skill_md: &str) -> Agent {
        Agent::from_skill_md(AgentId::parse("writer").unwrap(), skill_md).unwrap()
    }

    fn run(status: RunStatus, started_at: f64, ended_at: Option<f64>) -> RunSummary {
        RunSummary {
            id: format!("r{started_at}"),
            agent_id: "writer".into(),
            conversation_id: "agent:writer".into(),
            status,
            input: "x".into(),
            summary: None,
            started_at,
            ended_at,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Some(0.5),
            model: None,
            source_type: SourceType::User,
            source_id: None,
            source_label: None,
            priority: 10,
            queued_at: None,
            parent_run_id: None,
            compaction_summary: None,
        }
    }

    #[test]
    fn setup_names_runtime_and_model() {
        let native = setup(&agent(
            "---\nname: W\nmodel: claude:claude-sonnet-5\n---\nB\n",
        ));
        assert_eq!(native.runtime, "native");
        assert_eq!(native.model.as_deref(), Some("claude:claude-sonnet-5"));
        assert!(!native.accepts_requests);

        let cli = setup(&agent(
            "---\nname: W\nruntime: claude-code\nruntime_config: {model: sonnet}\naccepts_delegation: true\n---\nB\n",
        ));
        assert_eq!(cli.runtime, "claude-code");
        assert_eq!(cli.model.as_deref(), Some("sonnet"));
        assert!(cli.accepts_requests);
    }

    #[test]
    fn runs_land_in_their_day_and_old_ones_are_left_out() {
        let today = 100.0 * DAY_MS;
        let mut overview = setup(&agent("---\nname: W\n---\nB\n"));
        let runs = [
            run(RunStatus::Completed, today + 10.0, Some(today + 1010.0)),
            run(
                RunStatus::Failed,
                today - DAY_MS + 5.0,
                Some(today - DAY_MS + 3005.0),
            ),
            run(RunStatus::Running, today - 6.0 * DAY_MS, None),
            // Eight days ago: outside the window
            run(RunStatus::Failed, today - 7.0 * DAY_MS, None),
        ];
        count_runs(&mut overview, &runs, today);
        assert_eq!(
            (overview.runs, overview.completed, overview.failed),
            (3, 1, 1)
        );
        assert_eq!(
            overview.days[6],
            AgentDay {
                completed: 1,
                failed: 0
            }
        );
        assert_eq!(
            overview.days[5],
            AgentDay {
                completed: 0,
                failed: 1
            }
        );
        assert_eq!(overview.days[0], AgentDay::default());
        assert_eq!(overview.avg_duration_ms, Some(2000.0));
        assert_eq!(overview.cost_usd, 1.5);
        assert_eq!(overview.last_run_at, Some(today + 10.0));
    }

    #[test]
    fn no_runs_means_no_average() {
        let mut overview = setup(&agent("---\nname: W\n---\nB\n"));
        count_runs(&mut overview, &[], 0.0);
        assert_eq!(overview.runs, 0);
        assert_eq!(overview.avg_duration_ms, None);
        assert_eq!(overview.days.len(), DAYS);
    }
}
