//! Workflows: multi-agent pipelines.
//!
//! A workflow is a YAML file in the agents repo at `workflows/{id}.yaml`:
//!
//! ```yaml
//! id: daily-report
//! name: Daily Report
//! steps:
//!   - id: research
//!     agent: research-agent
//!     input: "Find today's AI news"
//!   - id: summarize
//!     agent: writer-agent
//!     input: "Summarize: {{research.output}}"
//!     critical: true
//!   - id: publish
//!     agent: publisher-agent
//!     input: "Send to #ai-news: {{summarize.output}}"
//!     requires_approval: true
//! ```
//!
//! The file is the source of truth; the `workflows` table mirrors it so the
//! sidebar can list workflows and runs can reference them.
//!
//! A run executes steps in order. Each step is a message to the named
//! agent's orchestrator, labelled "<workflow name> · step <n>"; if the
//! agent is busy it waits in the agent's queue (`critical` steps at a higher
//! priority than normal ones). The next step starts once that agent run's
//! `agent://turn_done` arrives, with `{{step.output}}` replaced by earlier
//! steps' final replies. Run state (every step's status, input, and output)
//! lives in `workflow_runs.context` and is re-read before each step, so a
//! run paused for approval survives a restart:
//!
//! ```text
//! pending ─▶ (requires_approval, not yet approved) ─▶ awaiting_approval ─▶ approved ─▶ pending
//!         ─▶ running (agent run in flight) ─▶ done ─▶ next step … ─▶ workflow done
//!                                          ─▶ failed ─▶ workflow failed
//! ```

mod asks;
mod driver;
// Steps that wait for a person
pub mod manual_steps;
mod store;
pub mod templates;

// The parts of the engine see each other as one module did
use self::store::*;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Duration;

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use strum::{EnumString, IntoStaticStr};

use crate::db::{self, Db};
use crate::model::{ApprovalDecision, RunStatus, RunSummary, Timestamp};
use crate::services::chat;
use crate::services::domain::{self, WorkflowId};
use crate::services::domain::{ChangedBy, EntityKind};
use crate::services::events::{self, BroadcastSink, EventBus, NandiEvent};
use crate::services::orchestrator::{Orchestrators, RunSource};
use crate::services::relay::{unavailable, RelayClient};
use crate::services::repo_store::RepoStore;
use crate::services::run_waiter::RunWaiter;
use crate::services::AppState;

/// Runs listed per workflow, most recent first
const RUN_HISTORY: u32 = 20;
/// How often a step re-checks its agent run if no event arrives
const RECHECK_INTERVAL: Duration = Duration::from_secs(30);
/// How often a gateway-initiated run is checked (`run_for_gateway`)
const GATEWAY_POLL_INTERVAL: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------------------
// Definition
// ---------------------------------------------------------------------------

/// A workflow as written in `workflows/{id}.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    /// What it does, from the file's `description:`
    #[serde(default)]
    pub description: Option<String>,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowStep {
    pub id: String,
    /// Agent ID (folder name); empty for a manual step
    pub agent: String,
    /// `manual`: waits for a person
    #[serde(default)]
    pub manual: bool,
    /// What the person should do (manual steps)
    #[serde(default)]
    pub instructions: Option<String>,
    /// Manual steps: due this many hours after the run reaches them
    #[serde(default)]
    pub deadline_hours: Option<u32>,
    /// Message sent to the agent; `{{step.output}}` is an earlier step's reply
    pub input: String,
    /// Pause for the user's approval before sending this step's message
    #[serde(default, alias = "requires_approval")]
    pub requires_approval: bool,
    /// Later steps depend on it: queue ahead of normal workflow work
    #[serde(default)]
    pub critical: bool,
    /// `on_failure: continue`: a failure doesn't stop the workflow; later
    /// steps see an empty output
    #[serde(default)]
    pub continue_on_failure: bool,
    /// Asks a colleague instead of messaging a local agent (network v1)
    #[serde(default)]
    pub ask: Option<WorkflowAsk>,
}

/// An ask step's `ask:` block (`domain::workflow::AskStep`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowAsk {
    /// A topic: the person whose responsibilities match best
    #[serde(default)]
    pub find: Option<String>,
    /// An instance ID
    #[serde(default)]
    pub person: Option<String>,
    /// Any provider of this capability
    #[serde(default)]
    pub capability: Option<String>,
    pub input: String,
    #[serde(default)]
    pub due: Option<String>,
}

impl From<&domain::workflow::AskStep> for WorkflowAsk {
    fn from(ask: &domain::workflow::AskStep) -> Self {
        Self {
            find: ask.find.clone(),
            person: ask.person.clone(),
            capability: ask.capability.clone(),
            input: ask.input.clone(),
            due: ask.due.clone(),
        }
    }
}

/// `{{ step.output }}`
static OUTPUT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z0-9_-]+)\.output\s*\}\}").unwrap());

#[cfg(test)]
impl WorkflowDefinition {
    /// Parses and validates a workflow file (rules in `domain::workflow`).
    pub fn parse(yaml: &str) -> Result<Self, String> {
        Ok(Self::from(&domain::Workflow::from_yaml(yaml)?))
    }
}

impl From<&domain::Workflow> for WorkflowDefinition {
    fn from(workflow: &domain::Workflow) -> Self {
        Self {
            id: workflow.id().to_string(),
            name: workflow.name().to_string(),
            description: workflow.description().map(str::to_string),
            steps: workflow
                .steps()
                .iter()
                .map(|step| WorkflowStep {
                    id: step.id.to_string(),
                    agent: step
                        .agent
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    manual: step.is_manual(),
                    instructions: step.instructions.clone(),
                    deadline_hours: step.deadline_hours,
                    input: step.input_template.clone(),
                    requires_approval: step.requires_approval,
                    critical: step.critical,
                    continue_on_failure: step.on_failure
                        == domain::workflow::FailurePolicy::Continue,
                    ask: step.ask.as_ref().map(WorkflowAsk::from),
                })
                .collect(),
        }
    }
}

/// Replaces `{{step.output}}` with that step's output.
fn render_input(template: &str, outputs: &HashMap<&str, &str>) -> String {
    OUTPUT_REF
        .replace_all(template, |caps: &regex::Captures| {
            outputs
                .get(&caps[1])
                .copied()
                .unwrap_or_default()
                .to_string()
        })
        .into_owned()
}

/// `{{ input }}` and `{{ input.<field> }}`
static INPUT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*input(?:\.([A-Za-z0-9_-]+))?\s*\}\}").unwrap());

/// Replaces `{{input}}` with a delegated task's input (its `text` when it
/// is an object with one, a string as is, anything else as JSON) and
/// `{{input.<field>}}` with that field. Missing fields render empty.
pub fn render_task_input(template: &str, input: &Value) -> String {
    let text = |value: &Value| match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    INPUT_REF
        .replace_all(template, |caps: &regex::Captures| match caps.get(1) {
            Some(field) => input.get(field.as_str()).map(text).unwrap_or_default(),
            None => match input.get("text") {
                Some(inner) if input.is_object() => text(inner),
                _ => text(input),
            },
        })
        .into_owned()
}

/// An ended run's result: the last non-empty step output, or its error
pub fn run_output(run: &WorkflowRun) -> Result<String, String> {
    match run.status {
        WorkflowRunStatus::Cancelled => Err("The workflow was cancelled".to_string()),
        WorkflowRunStatus::Failed => Err(run
            .error
            .clone()
            .unwrap_or_else(|| "The workflow failed".to_string())),
        _ => Ok(run
            .steps
            .iter()
            .rev()
            .find_map(|s| s.output.clone().filter(|o| !o.trim().is_empty()))
            .unwrap_or_default()),
    }
}

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

/// A workflow with its full definition and recent runs.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<WorkflowStep>,
    /// The YAML file's contents
    pub source: String,
    pub path: String,
    /// Most recent first
    pub runs: Vec<WorkflowRun>,
}

/// Imports a workflow file into the agents repo.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowInput {
    /// The YAML file's contents
    pub yaml: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum WorkflowRunStatus {
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    /// Stopped between steps by the user (Work); resumable
    Paused,
    /// Stopped for good by the user (Work)
    Cancelled,
    /// Suspended at a manual step until a person completes it
    Waiting,
}

impl WorkflowRunStatus {
    /// The text stored in the database
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowStepStatus {
    Pending,
    AwaitingApproval,
    /// A manual step waiting for a person
    AwaitingInput,
    Running,
    Done,
    Failed,
    Rejected,
}

/// One step of a run. The step's definition is copied in at start, so
/// editing the YAML doesn't change runs in progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepRun {
    pub step_id: String,
    pub agent_id: String,
    pub template: String,
    pub requires_approval: bool,
    #[serde(default)]
    pub critical: bool,
    #[serde(default)]
    pub continue_on_failure: bool,
    #[serde(default)]
    pub approved: bool,
    /// Waits for a person instead of messaging an agent
    #[serde(default)]
    pub manual: bool,
    /// What the person should do (manual steps), rendered when reached
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub deadline_hours: Option<u32>,
    /// When a manual step is due (epoch ms), set when the run reaches it
    #[serde(default)]
    pub deadline: Option<Timestamp>,
    /// Asks a colleague (network v1): who and what
    #[serde(default)]
    pub ask: Option<WorkflowAsk>,
    /// The ask's provider card ID, once resolved (`None`: any provider)
    #[serde(default)]
    pub ask_target: Option<String>,
    #[serde(default)]
    pub ask_capability: Option<String>,
    /// Who it asks, in words ("Lena Fischer")
    #[serde(default)]
    pub asked: Option<String>,
    /// The delegated task carrying the ask
    #[serde(default)]
    pub task_id: Option<String>,
    pub status: WorkflowStepStatus,
    /// The message sent, once rendered
    pub input: Option<String>,
    /// The agent's final reply
    pub output: Option<String>,
    /// The agent run executing this step
    pub agent_run_id: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
}

/// What started a run. Kept in `workflow_runs.context`, so runs from
/// before it was recorded read as `None` (shown as "Unknown").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum WorkflowRunSource {
    /// The Run button (or the API)
    Manual,
    /// A trigger firing: the trigger's ID
    Trigger(String),
    /// Another Nandi's request through the gateway: the gateway task's ID
    Gateway(String),
    /// A delegated task this workflow serves directly: the task's ID
    Task(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub status: WorkflowRunStatus,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub steps: Vec<WorkflowStepRun>,
    pub error: Option<String>,
    /// What started it (`None` for runs recorded before this was kept)
    pub source: Option<WorkflowRunSource>,
}

/// What `workflow_runs.context` holds.
///
/// Deliberate default: `{}` is the column's `DEFAULT` (a run inserted
/// before its first step), so a missing field reads as empty. Text that
/// isn't JSON, or a field of the wrong shape, still fails the read.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RunContext {
    steps: Vec<WorkflowStepRun>,
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<WorkflowRunSource>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

macro_rules! event {
    ($ty:ident, $name:literal) => {
        impl NandiEvent for $ty {
            const NAME: &'static str = $name;
        }
    };
}

/// A step's message was sent to its agent.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepStarted {
    pub workflow_id: String,
    pub run_id: String,
    pub step_id: String,
    pub agent_id: String,
    pub agent_run_id: String,
    pub input: String,
}
event!(WorkflowStepStarted, "workflow://step_started");

/// A step is waiting for `approve_workflow_step`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepAwaitingApproval {
    pub workflow_id: String,
    pub run_id: String,
    pub step_id: String,
    pub agent_id: String,
    /// The message the step will send if approved
    pub input: String,
}
event!(
    WorkflowStepAwaitingApproval,
    "workflow://step_awaiting_approval"
);

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepDone {
    pub workflow_id: String,
    pub run_id: String,
    pub step_id: String,
    pub output: String,
}
event!(WorkflowStepDone, "workflow://step_done");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDone {
    pub workflow_id: String,
    pub run_id: String,
}
event!(WorkflowDone, "workflow://done");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowFailed {
    pub workflow_id: String,
    pub run_id: String,
    /// The step that failed (or was rejected), if any
    pub step_id: Option<String>,
    pub error: String,
}
event!(WorkflowFailed, "workflow://failed");

/// Workflows were added, changed, or removed; reload the list.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsChanged {}
event!(WorkflowsChanged, "workflow://changed");

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Runs workflows. One driver task per active run; runs paused for approval
/// have none until approved.
pub struct WorkflowEngine {
    db: Db,
    bus: EventBus,
    orchestrators: Orchestrators,
    broadcast: BroadcastSink,
    runtime: tokio::runtime::Handle,
    /// The gold copy: `{agents repo}/workflows/*.yaml` (and agent-scoped
    /// workflows), projected into the `workflows` table
    store: RepoStore,
    /// Runs with a driver task, so approvals and resumes never double-drive
    driving: Mutex<HashSet<String>>,
    /// The relay an ask by topic searches (set by `network::start`)
    relay: RwLock<Option<Arc<dyn RelayClient>>>,
}

impl WorkflowEngine {
    pub fn new(
        db: Db,
        bus: EventBus,
        orchestrators: Orchestrators,
        broadcast: BroadcastSink,
        runtime: tokio::runtime::Handle,
        store: RepoStore,
    ) -> Self {
        Self {
            db,
            bus,
            orchestrators,
            broadcast,
            runtime,
            store,
            driving: Mutex::default(),
            relay: RwLock::default(),
        }
    }

    /// The relay ask steps find people on
    pub fn use_relay(&self, client: Arc<dyn RelayClient>) {
        *self.relay.write().unwrap_or_else(|e| e.into_inner()) = Some(client);
    }

    fn relay(&self) -> Option<Arc<dyn RelayClient>> {
        self.relay.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[cfg(test)]
    fn path(&self, id: &str) -> std::path::PathBuf {
        self.store
            .repos()
            .workflows
            .global_dir()
            .join(format!("{id}.yaml"))
    }

    /// Brings the `workflows` projection up to date with the files (new,
    /// changed, and removed). Invalid files are skipped and logged; run
    /// history is never touched.
    pub async fn sync_from_disk(&self) -> Result<(), String> {
        let store = self.store.clone();
        // Publishes `repo://workflow-*` (and `workflow://changed`) itself
        tokio::task::spawn_blocking(move || store.rescan(EntityKind::Workflow, ChangedBy::Watcher))
            .await
            .map_err(|e| format!("Workflow scan failed: {e}"))?;
        Ok(())
    }

    /// Loads a workflow's file (refreshing its row) and recent runs.
    pub async fn get_workflow(&self, id: &str) -> Result<Workflow, String> {
        let workflow_id = WorkflowId::parse(id)?;
        let store = self.store.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            store.reload(
                EntityKind::Workflow,
                workflow_id.as_str(),
                ChangedBy::Watcher,
            );
            store.repos().workflows.load_source(&workflow_id)
        })
        .await
        .map_err(|e| format!("Workflow read failed: {e}"))?;
        let loaded = loaded?;
        let definition = WorkflowDefinition::from(&loaded.workflow);
        let (source, path) = (loaded.source, loaded.path);
        let workflow_id = id.to_string();
        let runs = self
            .db
            .call(move |conn| list_runs(conn, &workflow_id, RUN_HISTORY))
            .await?;
        Ok(Workflow {
            id: definition.id,
            name: definition.name,
            description: definition.description,
            steps: definition.steps,
            source,
            path: path.display().to_string(),
            runs,
        })
    }

    /// Validates a YAML workflow and saves it as `workflows/{id}.yaml`
    /// (staged), then commits it to the aggregate and projection, which
    /// publishes `repo://workflow-created`. Re-importing identical content
    /// is a no-op; a different workflow with the same ID is refused rather
    /// than overwritten. Steps naming agents that don't exist yet are
    /// allowed (and warned about): runs check them.
    pub async fn create_workflow(&self, yaml: &str) -> Result<Workflow, String> {
        let workflow = domain::Workflow::from_yaml(yaml)?;
        let id = workflow.id().to_string();
        let (store, source) = (self.store.clone(), yaml.to_string());
        tokio::task::spawn_blocking(move || {
            let entity = domain::Entity::Workflow(workflow.clone());
            match store.read(|repo| repo.check_references(&entity)) {
                Err(domain::DomainError::DanglingReference { from, to }) => {
                    log::warn!("{from} refers to {to}, which doesn't exist yet");
                }
                Err(e) => return Err(e.to_string()),
                Ok(()) => {}
            }
            store.repos().workflows.create_source(&workflow, &source)?;
            store.commit(EntityKind::Workflow, workflow.id().as_str())?;
            Ok(())
        })
        .await
        .map_err(|e| format!("Workflow write failed: {e}"))??;
        self.get_workflow(&id).await
    }

    /// Starts a run of workflow `id` and returns its ID. Steps execute in
    /// the background; progress arrives as `workflow://*` events.
    pub async fn run_workflow(self: &Arc<Self>, id: &str) -> Result<String, String> {
        self.start_run(id, None, WorkflowRunSource::Manual).await
    }

    /// `run_workflow`, recording what started it (a trigger)
    pub async fn run_workflow_from(
        self: &Arc<Self>,
        id: &str,
        source: WorkflowRunSource,
    ) -> Result<String, String> {
        self.start_run(id, None, source).await
    }

    /// `run_workflow`, with `{{input}}` / `{{input.<field>}}` in the step
    /// templates replaced by a delegated task's `input`
    async fn start_run(
        self: &Arc<Self>,
        id: &str,
        input: Option<&Value>,
        source: WorkflowRunSource,
    ) -> Result<String, String> {
        let workflow = self.get_workflow(id).await?;
        let run = WorkflowRun {
            id: uuid::Uuid::new_v4().to_string(),
            workflow_id: workflow.id,
            status: WorkflowRunStatus::Running,
            started_at: db::ts(db::now_ms()),
            ended_at: None,
            steps: workflow
                .steps
                .into_iter()
                .map(|step| WorkflowStepRun {
                    step_id: step.id,
                    agent_id: step.agent,
                    template: match input {
                        Some(input) => render_task_input(&step.input, input),
                        None => step.input,
                    },
                    requires_approval: step.requires_approval,
                    critical: step.critical,
                    continue_on_failure: step.continue_on_failure,
                    approved: false,
                    manual: step.manual,
                    instructions: match (input, step.instructions) {
                        (Some(input), Some(text)) => Some(render_task_input(&text, input)),
                        (_, instructions) => instructions,
                    },
                    deadline_hours: step.deadline_hours,
                    deadline: None,
                    ask: step.ask.map(|ask| WorkflowAsk {
                        input: match input {
                            Some(input) => render_task_input(&ask.input, input),
                            None => ask.input,
                        },
                        ..ask
                    }),
                    ask_target: None,
                    ask_capability: None,
                    asked: None,
                    task_id: None,
                    status: WorkflowStepStatus::Pending,
                    input: None,
                    output: None,
                    agent_run_id: None,
                    error: None,
                    started_at: None,
                    ended_at: None,
                })
                .collect(),
            error: None,
            source: Some(source),
        };
        let run_id = run.id.clone();
        self.db.call(move |conn| insert_run(conn, &run)).await?;
        log::info!("Started workflow '{id}' run {run_id}");
        self.spawn_driver(run_id.clone());
        Ok(run_id)
    }

    /// A run the gateway started for a delegated task:
    /// starts `id` and waits for it to end, calling
    /// `on_progress(settled, total, step_id)` as steps settle. Returns the
    /// last step's output; a failed run is an `Err`. A step waiting for
    /// approval keeps it waiting (the caller's deadline applies).
    pub async fn run_for_gateway(
        self: &Arc<Self>,
        id: &str,
        task_id: &str,
        on_started: impl FnOnce(&str) + Send,
        on_progress: impl Fn(usize, usize, &str) + Send,
    ) -> Result<String, String> {
        let run_id = self
            .start_run(id, None, WorkflowRunSource::Gateway(task_id.to_string()))
            .await?;
        // The gateway records the run on its task (Request page, task 5)
        on_started(&run_id);
        let run = self.wait_for_end(&run_id, on_progress).await?;
        run_output(&run)
    }

    /// A run for a delegated task whose provider is this workflow
    /// (`capability::workflow_provider`): `input` fills `{{input}}`, and
    /// the ended run comes back whole (for its step breakdown). Progress as
    /// in `run_for_gateway`.
    pub async fn run_for_task(
        self: &Arc<Self>,
        id: &str,
        task_id: &str,
        input: &Value,
        on_progress: impl Fn(usize, usize, &str) + Send,
    ) -> Result<WorkflowRun, String> {
        let run_id = self
            .start_run(
                id,
                Some(input),
                WorkflowRunSource::Task(task_id.to_string()),
            )
            .await?;
        self.wait_for_end(&run_id, on_progress).await
    }

    /// Polls run `run_id` until it completes or fails
    async fn wait_for_end(
        &self,
        run_id: &str,
        on_progress: impl Fn(usize, usize, &str) + Send,
    ) -> Result<WorkflowRun, String> {
        let run_id = run_id.to_string();
        let mut reported = 0;
        loop {
            tokio::time::sleep(GATEWAY_POLL_INTERVAL).await;
            let id = run_id.clone();
            let run = self
                .db
                .call(move |conn| get_run(conn, &id))
                .await?
                .ok_or_else(|| format!("Workflow run {run_id} not found"))?;
            let settled: Vec<&WorkflowStepRun> = run
                .steps
                .iter()
                .filter(|s| {
                    matches!(
                        s.status,
                        WorkflowStepStatus::Done
                            | WorkflowStepStatus::Failed
                            | WorkflowStepStatus::Rejected
                    )
                })
                .collect();
            if settled.len() > reported {
                reported = settled.len();
                if let Some(last) = settled.last() {
                    on_progress(reported, run.steps.len(), &last.step_id);
                }
            }
            match run.status {
                WorkflowRunStatus::Completed
                | WorkflowRunStatus::Failed
                | WorkflowRunStatus::Cancelled => return Ok(run),
                WorkflowRunStatus::Running
                | WorkflowRunStatus::AwaitingApproval
                | WorkflowRunStatus::Paused
                | WorkflowRunStatus::Waiting => {}
            }
        }
    }

    /// Resolves a step waiting for approval: approved steps run, rejected
    /// ones fail the workflow.
    pub async fn approve_step(
        self: &Arc<Self>,
        run_id: &str,
        step_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let id = run_id.to_string();
        let mut run = self
            .db
            .call(move |conn| get_run(conn, &id))
            .await?
            .ok_or("Workflow run not found")?;
        let step = run
            .steps
            .iter_mut()
            .find(|s| s.step_id == step_id)
            .filter(|s| s.status == WorkflowStepStatus::AwaitingApproval)
            .ok_or("This step is not waiting for approval")?;
        match decision {
            ApprovalDecision::Approved => {
                step.approved = true;
                step.status = WorkflowStepStatus::Pending;
                run.status = WorkflowRunStatus::Running;
                self.save(&run).await?;
                self.spawn_driver(run.id);
            }
            ApprovalDecision::Rejected => {
                step.status = WorkflowStepStatus::Rejected;
                step.ended_at = Some(db::ts(db::now_ms()));
                let error = format!("Step '{step_id}' was rejected");
                self.fail(&mut run, Some(step_id.to_string()), error)
                    .await?;
            }
        }
        Ok(())
    }

    /// Completes a manual step a run is waiting on: its output (what the
    /// person handed back) is `{{step.output}}` for later steps, and the
    /// run picks up where it stopped
    pub async fn complete_manual_step(
        self: &Arc<Self>,
        run_id: &str,
        step_id: &str,
        output: String,
    ) -> Result<WorkflowRun, String> {
        let id = run_id.to_string();
        let mut run = self
            .db
            .call(move |conn| get_run(conn, &id))
            .await?
            .ok_or("Workflow run not found")?;
        if run.status != WorkflowRunStatus::Waiting {
            return Err(format!(
                "This run is {}, not waiting on a person",
                run.status.as_str().replace('_', " ")
            ));
        }
        let now = db::now_ms();
        let step = run
            .steps
            .iter_mut()
            .find(|s| s.step_id == step_id)
            .filter(|s| s.status == WorkflowStepStatus::AwaitingInput)
            .ok_or("This step is not waiting for anyone")?;
        step.status = WorkflowStepStatus::Done;
        step.output = Some(output.clone());
        step.ended_at = Some(db::ts(now));
        run.status = WorkflowRunStatus::Running;
        self.save(&run).await?;
        manual_steps::close(&self.db, run_id, step_id, manual_steps::COMPLETED).await?;
        log::info!("Workflow run {run_id}: manual step '{step_id}' completed");
        events::emit(
            &self.bus,
            &manual_steps::ManualStepCompleted {
                run_id: run.id.clone(),
                workflow_id: run.workflow_id.clone(),
                step_id: step_id.to_string(),
                completed_at: db::ts(now),
            },
        );
        events::emit(
            &self.bus,
            &WorkflowStepDone {
                workflow_id: run.workflow_id.clone(),
                run_id: run.id.clone(),
                step_id: step_id.to_string(),
                output,
            },
        );
        self.spawn_driver(run.id.clone());
        Ok(run)
    }

    /// Pauses a running run between steps: a step in flight finishes, the
    /// next one waits for `resume_run`
    pub async fn pause_run(&self, run_id: &str) -> Result<WorkflowRun, String> {
        self.transition(
            run_id,
            &[WorkflowRunStatus::Running],
            WorkflowRunStatus::Paused,
        )
        .await
    }

    /// Picks a paused run up where it stopped
    pub async fn resume_run(self: &Arc<Self>, run_id: &str) -> Result<WorkflowRun, String> {
        let run = self
            .transition(
                run_id,
                &[WorkflowRunStatus::Paused],
                WorkflowRunStatus::Running,
            )
            .await?;
        self.spawn_driver(run.id.clone());
        Ok(run)
    }

    /// Stops a run for good, and the agent turn of its step in flight
    pub async fn cancel_run(&self, run_id: &str) -> Result<WorkflowRun, String> {
        let run = self
            .transition(
                run_id,
                &[
                    WorkflowRunStatus::Running,
                    WorkflowRunStatus::AwaitingApproval,
                    WorkflowRunStatus::Paused,
                    WorkflowRunStatus::Waiting,
                ],
                WorkflowRunStatus::Cancelled,
            )
            .await?;
        manual_steps::close_pending(&self.db, run_id, manual_steps::CANCELLED).await?;
        for step in run
            .steps
            .iter()
            .filter(|s| s.status == WorkflowStepStatus::Running)
        {
            if let Some(agent_run_id) = &step.agent_run_id {
                let command = crate::services::orchestrator::OrchestratorCommand::Cancel {
                    run_id: agent_run_id.clone(),
                };
                if let Err(e) = self.orchestrators.send(&step.agent_id, command).await {
                    log::warn!("Workflow run {run_id}: the step's agent run wasn't stopped: {e}");
                }
            }
        }
        log::info!("Workflow run {run_id} cancelled");
        Ok(run)
    }

    /// One run
    pub async fn run(&self, run_id: &str) -> Result<Option<WorkflowRun>, String> {
        let id = run_id.to_string();
        self.db.call(move |conn| get_run(conn, &id)).await
    }

    /// Runs not over yet, and runs started since `since_ms`, newest first
    pub async fn runs_in_flight_or_since(&self, since_ms: i64) -> Result<Vec<WorkflowRun>, String> {
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {RUN_COLUMNS} FROM workflow_runs
                     WHERE status IN ('running', 'awaiting_approval', 'paused', 'waiting')
                        OR started_at >= ?1 OR ended_at >= ?1
                     ORDER BY started_at DESC, rowid DESC LIMIT 200"
                ))?;
                let rows = stmt.query_map([since_ms], run_row)?;
                rows.collect()
            })
            .await
    }

    /// Resumes runs that were mid-flight when the app last stopped.
    pub async fn resume_interrupted(self: &Arc<Self>) -> Result<(), String> {
        let ids = self.db.call(|conn| running_run_ids(conn)).await?;
        if !ids.is_empty() {
            log::info!("Resuming {} workflow runs", ids.len());
        }
        for id in ids {
            self.spawn_driver(id);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Service API (shared by the Tauri commands and HTTP routes)
// ---------------------------------------------------------------------------

pub async fn get_workflow(state: &AppState, id: String) -> Result<Workflow, String> {
    state.workflows().get_workflow(&id).await
}

pub async fn create_workflow(
    state: &AppState,
    input: CreateWorkflowInput,
) -> Result<Workflow, String> {
    state.workflows().create_workflow(&input.yaml).await
}

/// Creates a workflow from a bundled template (`templates`)
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateWorkflowInput {
    pub template_id: String,
    /// Empty keeps the template's name
    pub name: String,
    /// Step ID → agent ID, for each agent step
    pub agents: HashMap<String, String>,
}

/// The bundled templates "New workflow › From a template" offers
pub fn list_workflow_templates() -> Vec<templates::WorkflowTemplate> {
    templates::list()
}

/// Builds the template with the chosen agents (each must exist), names it
/// with a free ID derived from the name, and saves it like an import
pub async fn create_workflow_from_template(
    state: &AppState,
    input: TemplateWorkflowInput,
) -> Result<Workflow, String> {
    for (step, agent) in &input.agents {
        state
            .store()
            .require_agent(agent)
            .map_err(|e| format!("Step '{step}': {e}"))?;
    }
    let name = input.name.trim();
    let id = templates::id_for(
        if name.is_empty() {
            &input.template_id
        } else {
            name
        },
        |id| state.store().read(|repo| repo.workflow(id).is_some()),
    );
    let workflow = templates::build(&input.template_id, &id, name, &input.agents)?;
    state.workflows().create_workflow(&workflow.to_yaml()).await
}

/// Checks every agent step's agent exists (manual and ask steps have
/// none), then starts a run.
pub async fn run_workflow(state: &AppState, id: String) -> Result<String, String> {
    let workflow = state.workflows().get_workflow(&id).await?;
    for step in workflow
        .steps
        .iter()
        .filter(|step| !step.manual && step.ask.is_none())
    {
        state
            .store()
            .require_agent(&step.agent)
            .map_err(|e| format!("Step '{}': {e}", step.id))?;
    }
    state.workflows().run_workflow(&id).await
}

pub async fn approve_workflow_step(
    state: &AppState,
    run_id: String,
    step_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    state
        .workflows()
        .approve_step(&run_id, &step_id, decision)
        .await?;
    crate::services::inbox::audit::record(
        state,
        crate::services::inbox::audit::Decided::new(
            "workflowApproval",
            format!("{run_id}/{step_id}"),
            if decision == ApprovalDecision::Approved {
                "approved"
            } else {
                "refused"
            },
        ),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::CANNED_SCRIPT_KEY;
    use crate::services::events::tests::RecordingSink;
    use serde_json::json;

    #[test]
    fn a_driver_save_never_undoes_a_pause_or_cancel() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let run = WorkflowRun {
            id: "r1".into(),
            workflow_id: "w".into(),
            status: WorkflowRunStatus::Running,
            started_at: 1.0,
            ended_at: None,
            steps: Vec::new(),
            error: None,
            source: Some(WorkflowRunSource::Trigger("morning".into())),
        };
        insert_run(&conn, &run).unwrap();
        // The source survives the context column
        assert_eq!(
            get_run(&conn, "r1").unwrap().unwrap().source,
            Some(WorkflowRunSource::Trigger("morning".into()))
        );
        // A run recorded before sources were kept reads as unknown
        conn.execute(
            "INSERT INTO workflow_runs (id, workflow_id, status, started_at, context)
             VALUES ('old', 'w', 'completed', 1, '{\"steps\":[],\"error\":null}')",
            [],
        )
        .unwrap();
        assert_eq!(get_run(&conn, "old").unwrap().unwrap().source, None);
        for stopped in ["paused", "cancelled"] {
            conn.execute(
                "UPDATE workflow_runs SET status = ?1, ended_at = 5 WHERE id = 'r1'",
                [stopped],
            )
            .unwrap();
            // The driver finishing a step saves its `running` copy
            update_run(&conn, &run).unwrap();
            let saved = get_run(&conn, "r1").unwrap().unwrap();
            assert_eq!(saved.status.as_str(), stopped);
        }
        // Anything else still goes through
        let mut done = run.clone();
        done.status = WorkflowRunStatus::Completed;
        update_run(&conn, &done).unwrap();
        assert_eq!(
            get_run(&conn, "r1").unwrap().unwrap().status,
            WorkflowRunStatus::Completed
        );
    }

    #[test]
    fn a_bad_run_context_fails_the_read_and_names_the_run() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let cases = [
            ("not JSON", "{oops"),
            ("not an object", "[1, 2]"),
            ("steps of the wrong shape", r#"{"steps": 5}"#),
        ];
        for (case, context) in cases {
            conn.execute(
                "INSERT OR REPLACE INTO workflow_runs (id, workflow_id, status, started_at, context)
                 VALUES ('r1', 'w', 'completed', 1, ?1)",
                [context],
            )
            .unwrap();
            let error = get_run(&conn, "r1").unwrap_err().to_string();
            assert!(error.contains("workflow_runs row 'r1'"), "{case}: {error}");
            assert!(error.contains("column 'context'"), "{case}: {error}");
        }
    }

    #[test]
    fn the_column_default_context_reads_as_an_empty_run() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs (id, workflow_id, status, started_at)
             VALUES ('r1', 'w', 'running', 1)",
            [],
        )
        .unwrap();
        let run = get_run(&conn, "r1").unwrap().unwrap();
        assert!(run.steps.is_empty());
        assert_eq!(run.error, None);
        assert_eq!(run.source, None);
    }

    const TWO_STEPS: &str = r#"
id: two-step
name: Two Step
steps:
  - id: research
    agent: research-agent
    input: "Find today's AI news"
  - id: summarize
    agent: writer-agent
    input: "Summarize: {{research.output}}"
"#;

    /// Mocked agents: the canned engine echoes each message back, wrapped
    /// in brackets, with no tool calls and no model.
    async fn mocked_agents(state: &AppState) {
        state
            .db()
            .call(|conn| {
                db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
                db::set_setting(
                    conn,
                    CANNED_SCRIPT_KEY,
                    &json!({
                        "agent": [{ "text": "[{{input}}]" }],
                        "studio": [],
                        "tokenDelayMs": 0
                    }),
                )
            })
            .await
            .unwrap();
    }

    async fn wait_for_end(state: &AppState, run_id: &str) -> WorkflowRun {
        for _ in 0..500 {
            let id = run_id.to_string();
            let run = state
                .db()
                .call(move |conn| get_run(conn, &id))
                .await
                .unwrap()
                .unwrap();
            if !matches!(run.status, WorkflowRunStatus::Running) {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("workflow run {run_id} didn't finish");
    }

    fn workflow_events(recorder: &RecordingSink) -> Vec<(String, String)> {
        recorder
            .take()
            .into_iter()
            .filter(|(name, _)| name.starts_with("workflow://") && *name != "workflow://changed")
            .map(|(name, payload)| {
                let step = payload["stepId"].as_str().unwrap_or_default().to_string();
                (name.to_string(), step)
            })
            .collect()
    }

    #[test]
    fn parses_the_documented_format() {
        let definition = WorkflowDefinition::parse(
            r#"
id: daily-report
name: Daily Report
steps:
  - id: research
    agent: research-agent
    input: "Find today's AI news"
  - id: publish
    agent: publisher-agent
    input: "Send to #ai-news: {{research.output}}"
    requires_approval: true
"#,
        )
        .unwrap();
        assert_eq!(definition.id, "daily-report");
        assert_eq!(definition.steps.len(), 2);
        assert!(!definition.steps[0].requires_approval);
        assert!(definition.steps[1].requires_approval);
    }

    #[test]
    fn rejects_invalid_definitions() {
        let cases = [
            ("id: ../x\nname: X\nsteps: [{id: a, agent: b, input: hi}]", "Invalid workflow id"),
            ("id: x\nname: X\nsteps: []", "no steps"),
            (
                "id: x\nname: X\nsteps: [{id: a, agent: b, input: '{{later.output}}'}, {id: later, agent: b, input: hi}]",
                "no earlier step",
            ),
            (
                "id: x\nname: X\nsteps: [{id: a, agent: b, input: hi}, {id: a, agent: b, input: hi}]",
                "Duplicate step",
            ),
            ("id: x\nname: X\nsteps: [{id: a, agent: b, input: hi, typo: 1}]", "unknown field"),
        ];
        for (yaml, expected) in cases {
            let error = WorkflowDefinition::parse(yaml).unwrap_err();
            assert!(error.contains(expected), "{yaml}: {error}");
        }
    }

    #[test]
    fn renders_step_outputs() {
        let outputs = HashMap::from([("a", "one"), ("b", "two")]);
        assert_eq!(
            render_input("{{a.output}} and {{ b.output }}, {{other}}", &outputs),
            "one and two, {{other}}"
        );
    }

    #[test]
    fn renders_task_input() {
        let template = "Review {{input}} ({{ input.lang }}){{input.missing}}";
        assert_eq!(
            render_task_input(template, &json!({"text": "main.py", "lang": "python"})),
            "Review main.py (python)"
        );
        assert_eq!(
            render_task_input(template, &json!("x.rs")),
            "Review x.rs ()"
        );
        assert_eq!(
            render_task_input("{{input}}", &json!({"a": 1})),
            r#"{"a":1}"#
        );
        // Step references are left for the driver
        assert_eq!(
            render_task_input("{{a.output}}", &json!("x")),
            "{{a.output}}"
        );
    }

    #[tokio::test]
    async fn a_task_run_fills_the_input_and_returns_the_run() {
        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine
            .create_workflow(
                "id: reviewer\nname: Reviewer\nprovides: [code-review.python]\nsteps:\n  - {id: review, agent: research-agent, input: 'Review {{input}}'}\n  - {id: sum, agent: writer-agent, input: 'Sum: {{review.output}}'}\n",
            )
            .await
            .unwrap();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let seen = progress.clone();
        let run = engine
            .run_for_task(
                "reviewer",
                "t1",
                &json!({"text": "main.py"}),
                move |done, _, _| {
                    seen.lock().unwrap().push(done);
                },
            )
            .await
            .unwrap();
        assert_eq!(run.status, WorkflowRunStatus::Completed, "{:?}", run.error);
        assert_eq!(run.steps[0].input.as_deref(), Some("Review main.py"));
        assert_eq!(run_output(&run).unwrap(), "[Sum: [Review main.py]]");
        assert_eq!(progress.lock().unwrap().last(), Some(&2));
        assert_eq!(run.source, Some(WorkflowRunSource::Task("t1".into())));
        state.shutdown();
    }

    #[tokio::test]
    async fn a_gateway_run_reports_progress_and_returns_the_last_output() {
        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine.create_workflow(TWO_STEPS).await.unwrap();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let seen = progress.clone();
        let started = Arc::new(Mutex::new(None::<String>));
        let run = started.clone();
        let output = engine
            .run_for_gateway(
                "two-step",
                "gw-task",
                move |run_id| *run.lock().unwrap() = Some(run_id.to_string()),
                move |done, total, step| {
                    seen.lock().unwrap().push((done, total, step.to_string()));
                },
            )
            .await
            .unwrap();
        assert_eq!(output, "[Summarize: [Find today's AI news]]");
        let progress = progress.lock().unwrap().clone();
        assert_eq!(progress.last(), Some(&(2, 2, "summarize".to_string())));
        // The run is named as soon as it starts (the gateway records it)
        let run_id = started.lock().unwrap().clone().unwrap();
        // ... and knows which gateway task started it
        let run = engine.run(&run_id).await.unwrap().unwrap();
        assert_eq!(
            run.source,
            Some(WorkflowRunSource::Gateway("gw-task".into()))
        );
        assert!(engine
            .run_for_gateway("missing", "gw-task", |_| {}, |_, _, _| {})
            .await
            .is_err());
        state.shutdown();
    }

    #[tokio::test]
    async fn runs_two_steps_in_order() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine.create_workflow(TWO_STEPS).await.unwrap();
        recorder.take();

        let run_id = engine.run_workflow("two-step").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;

        assert_eq!(run.status, WorkflowRunStatus::Completed, "{:?}", run.error);
        assert_eq!(run.source, Some(WorkflowRunSource::Manual));
        let [research, summarize] = &run.steps[..] else {
            panic!("expected two steps");
        };
        assert_eq!(research.status, WorkflowStepStatus::Done);
        assert_eq!(research.output.as_deref(), Some("[Find today's AI news]"));
        // The second step received the first step's output
        assert_eq!(
            summarize.input.as_deref(),
            Some("Summarize: [Find today's AI news]")
        );
        assert_eq!(
            summarize.output.as_deref(),
            Some("[Summarize: [Find today's AI news]]")
        );
        assert!(research.ended_at.unwrap() <= summarize.started_at.unwrap());

        // Each agent got exactly its step's message, as its own run
        for step in [research, summarize] {
            let agent_run_id = step.agent_run_id.clone().unwrap();
            let agent_run = state
                .db()
                .call(move |conn| db::get_run(conn, &agent_run_id))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(agent_run.agent_id, step.agent_id);
            assert_eq!(Some(&agent_run.input), step.input.as_ref());
        }

        let s = |name: &str, step: &str| (name.to_string(), step.to_string());
        assert_eq!(
            workflow_events(&recorder),
            vec![
                s("workflow://step_started", "research"),
                s("workflow://step_done", "research"),
                s("workflow://step_started", "summarize"),
                s("workflow://step_done", "summarize"),
                s("workflow://done", ""),
            ]
        );
        assert_eq!(engine.get_workflow("two-step").await.unwrap().runs.len(), 1);
        state.shutdown();
    }

    #[tokio::test]
    async fn pauses_for_approval() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine
            .create_workflow(&TWO_STEPS.replace(
                "input: \"Summarize: {{research.output}}\"",
                "input: \"Summarize: {{research.output}}\"\n    requires_approval: true",
            ))
            .await
            .unwrap();

        let run_id = engine.run_workflow("two-step").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::AwaitingApproval);
        assert_eq!(run.steps[1].status, WorkflowStepStatus::AwaitingApproval);
        assert!(run.steps[1].agent_run_id.is_none());
        assert!(engine
            .approve_step(&run_id, "research", ApprovalDecision::Approved)
            .await
            .is_err());

        engine
            .approve_step(&run_id, "summarize", ApprovalDecision::Approved)
            .await
            .unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Completed, "{:?}", run.error);
        assert_eq!(run.steps[1].status, WorkflowStepStatus::Done);

        // Rejecting fails the run without messaging the agent
        let run_id = engine.run_workflow("two-step").await.unwrap();
        wait_for_end(&state, &run_id).await;
        engine
            .approve_step(&run_id, "summarize", ApprovalDecision::Rejected)
            .await
            .unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert_eq!(run.steps[1].status, WorkflowStepStatus::Rejected);
        assert!(run.steps[1].agent_run_id.is_none());
        state.shutdown();
    }

    const MANUAL_STEP: &str = r#"
id: onboarding
name: Onboarding
steps:
  - id: research
    agent: research-agent
    input: "Find the new hire's team"
  - id: sign-off
    step_type: manual
    instructions: "Check {{research.output}} and add the laptop tag"
    deadline_hours: 48
  - id: welcome
    agent: writer-agent
    input: "Welcome them; laptop: {{sign-off.output}}"
"#;

    // -----------------------------------------------------------------------
    // Ask steps (network v1, Part 5)
    // -----------------------------------------------------------------------

    const ASK_STEP: &str = r#"
id: evidence
name: Evidence
steps:
  - id: ask
    ask:
      person: lena
      input: "Please send the FR-07 approval log"
      due: 5 days
  - id: file
    step_type: manual
    instructions: "File this: {{ask.output}}"
"#;

    /// The ask's history row once the background send has written it (it
    /// stays `Running` while the request waits for its answer)
    async fn ask_row(state: &AppState, task_id: &str) -> crate::services::a2a::TaskResult {
        for _ in 0..500 {
            if let Some(row) = crate::services::a2a::history::get(state.db(), task_id)
                .await
                .unwrap()
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the ask {task_id} was never recorded");
    }

    #[tokio::test]
    async fn an_ask_step_waits_for_the_answer_and_passes_it_on() {
        let state = AppState::for_tests(vec![]);
        let engine = state.workflows().clone();
        engine.create_workflow(ASK_STEP).await.unwrap();
        let run_id = engine.run_workflow("evidence").await.unwrap();

        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Waiting);
        let step = &run.steps[0];
        assert_eq!(step.status, WorkflowStepStatus::AwaitingInput);
        assert_eq!(step.ask_target.as_deref(), Some("lena/gateway"));
        assert_eq!(step.ask_capability.as_deref(), Some("ask-me"));
        // Due in 5 days, as the step says
        let due = step.deadline.unwrap() - step.started_at.unwrap();
        assert!((due as i64 - 5 * 86_400_000).abs() < 60_000, "{due}");
        let task_id = step.task_id.clone().unwrap();

        // Days later, the answer comes back (here: written as it would be)
        let mut answer = ask_row(&state, &task_id).await;
        answer.status = crate::services::a2a::TaskStatus::Completed;
        answer.output = json!({"text": "Log attached: fr07-oct.csv"});
        answer.error = None;
        crate::services::a2a::history::insert(state.db(), &answer)
            .await
            .unwrap();

        // A restarted engine settles it, and the next step gets the answer
        let restarted = Arc::new(WorkflowEngine::new(
            state.db().clone(),
            state.bus().clone(),
            state.orchestrators().clone(),
            state.broadcast().clone(),
            state.runtime().clone(),
            state.store().clone(),
        ));
        assert_eq!(restarted.settle_asks().await.unwrap(), 1);
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.steps[0].status, WorkflowStepStatus::Done);
        assert_eq!(
            run.steps[0].output.as_deref(),
            Some("Log attached: fr07-oct.csv")
        );
        assert_eq!(run.steps[1].status, WorkflowStepStatus::AwaitingInput);
        assert_eq!(
            run.steps[1].instructions.as_deref(),
            Some("File this: Log attached: fr07-oct.csv")
        );
        // Settled once only
        assert_eq!(restarted.settle_asks().await.unwrap(), 0);
        state.shutdown();
    }

    #[tokio::test]
    async fn an_ask_that_fails_or_expires_fails_its_step() {
        let state = AppState::for_tests(vec![]);
        let engine = state.workflows().clone();
        engine.create_workflow(ASK_STEP).await.unwrap();
        let run_id = engine.run_workflow("evidence").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        let task_id = run.steps[0].task_id.clone().unwrap();

        let mut expired = ask_row(&state, &task_id).await;
        expired.status = crate::services::a2a::TaskStatus::TimedOut;
        expired.error = Some("No answer by its due date".into());
        crate::services::a2a::history::insert(state.db(), &expired)
            .await
            .unwrap();
        assert_eq!(engine.settle_asks().await.unwrap(), 1);
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert_eq!(run.steps[0].status, WorkflowStepStatus::Failed);
        assert!(run.error.unwrap().contains("due date"));
        state.shutdown();
    }

    #[tokio::test]
    async fn an_ask_by_topic_finds_the_owner_and_asks_first() {
        let state = AppState::for_tests(vec![]);
        let engine = state.workflows().clone();
        engine
            .create_workflow(
                r#"
id: find
name: Find
steps:
  - id: ask
    ask:
      find: "SOC 1 evidence"
      input: "Send your SOC 1 evidence list"
"#,
            )
            .await
            .unwrap();

        // No relay: people search is "not connected", never a guess
        let run_id = engine.run_workflow("find").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        let error = run.steps[0].error.clone().unwrap();
        assert!(
            crate::services::relay::RelayUnavailable::is(&error),
            "{error}"
        );

        // Connected, and nobody lists it: the step fails and says why
        use crate::services::network::directory_tests::DirectoryRelay;
        let relay = Arc::new(DirectoryRelay::listing(Vec::new()));
        engine.use_relay(relay.clone());
        let run_id = engine.run_workflow("find").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert!(run.steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("Nobody on your network lists 'SOC 1 evidence'"));

        // The relay finds Lena: the owner approves who it goes to before
        // it's sent
        let lena = crate::services::relay::protocol::PeopleMatch {
            instance_id: "lena".into(),
            name: "Lena Fischer".into(),
            handle: None,
            online: true,
            last_seen_ms: None,
            takes_requests: true,
            ask_capability: "ask-me".into(),
            responsibilities: vec!["SOC 1 evidence".into()],
            description: String::new(),
            matched_on: vec!["responsibilities".into()],
            excerpt: None,
            agents: Vec::new(),
            agents_online: 0,
            availability: Default::default(),
        };
        *relay.people.lock().unwrap() = Ok(crate::services::relay::client_api::PeopleSearch {
            matches: vec![lena],
            searched: 1,
            unstated: 0,
            total: 1,
        });
        let run_id = engine.run_workflow("find").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::AwaitingApproval);
        let step = &run.steps[0];
        assert_eq!(step.status, WorkflowStepStatus::AwaitingApproval);
        assert_eq!(step.asked.as_deref(), Some("Lena Fischer"));
        assert_eq!(step.ask_target.as_deref(), Some("lena/gateway"));
        assert!(step.task_id.is_none(), "nothing is sent before approval");

        engine
            .approve_step(&run_id, "ask", ApprovalDecision::Approved)
            .await
            .unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Waiting);
        assert!(run.steps[0].task_id.is_some());
        state.shutdown();
    }

    #[test]
    fn an_ask_step_names_exactly_one_target_and_something_to_ask() {
        let parse = |ask: &str| {
            WorkflowDefinition::parse(&format!(
                "id: f\nname: F\nsteps:\n  - id: a\n    ask: {ask}\n"
            ))
        };
        assert!(parse("{person: lena, input: hi}").is_ok());
        assert!(parse("{input: hi}")
            .unwrap_err()
            .contains("exactly one of find, person or capability"));
        assert!(parse("{person: lena, capability: review-pr, input: hi}")
            .unwrap_err()
            .contains("exactly one"));
        assert!(parse("{person: lena, input: '  '}")
            .unwrap_err()
            .contains("nothing to ask"));
        assert!(parse("{person: lena, input: hi, typo: 1}").is_err());
        let with_agent = WorkflowDefinition::parse(
            "id: f\nname: F\nsteps:\n  - id: a\n    agent: writer\n    ask: {person: lena, input: hi}\n",
        );
        assert!(with_agent.unwrap_err().contains("can't have an agent"));
    }

    #[tokio::test]
    async fn a_manual_step_waits_for_a_person_and_survives_a_restart() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine.create_workflow(MANUAL_STEP).await.unwrap();

        let run_id = engine.run_workflow("onboarding").await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Waiting);
        let step = &run.steps[1];
        assert_eq!(step.status, WorkflowStepStatus::AwaitingInput);
        assert!(step.agent_run_id.is_none(), "no agent is messaged");
        assert_eq!(
            step.instructions.as_deref(),
            Some("Check [Find the new hire's team] and add the laptop tag")
        );
        let due = step.deadline.unwrap() - step.started_at.unwrap();
        assert_eq!(due as i64, 48 * 3_600_000);
        assert_eq!(run.steps[2].status, WorkflowStepStatus::Pending);

        let pending = manual_steps::pending(state.db()).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].workflow_name, "Onboarding");
        assert_eq!(pending[0].step_id, "sign-off");
        let ready: Vec<String> = recorder
            .take()
            .into_iter()
            .filter(|(name, _)| *name == manual_steps::MANUAL_STEP_READY)
            .map(|(_, payload)| payload["stepId"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(ready, ["sign-off"]);

        // Only the waiting step can be completed
        assert!(engine
            .complete_manual_step(&run_id, "welcome", "x".into())
            .await
            .is_err());

        // A day later, after a restart: a fresh engine on the same database
        let id = run_id.clone();
        state
            .db()
            .call(move |conn| {
                conn.execute(
                    "UPDATE workflow_runs SET started_at = started_at - 86400000 WHERE id = ?1",
                    [&id],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let restarted = Arc::new(WorkflowEngine::new(
            state.db().clone(),
            state.bus().clone(),
            state.orchestrators().clone(),
            state.broadcast().clone(),
            state.runtime().clone(),
            state.store().clone(),
        ));
        restarted.resume_interrupted().await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Waiting, "still waiting");

        restarted
            .complete_manual_step(&run_id, "sign-off", "LT-0042".into())
            .await
            .unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Completed, "{:?}", run.error);
        assert_eq!(run.steps[1].output.as_deref(), Some("LT-0042"));
        assert_eq!(
            run.steps[2].output.as_deref(),
            Some("[Welcome them; laptop: LT-0042]")
        );
        assert!(manual_steps::pending(state.db()).await.unwrap().is_empty());
        let names: Vec<&str> = recorder.take().into_iter().map(|(name, _)| name).collect();
        assert!(names.contains(&manual_steps::MANUAL_STEP_COMPLETED));
        assert!(names.contains(&"workflow://done"));
        state.shutdown();
    }

    #[tokio::test]
    async fn the_run_check_skips_ask_steps() {
        // Regression (live test 2026-10-31): starting from the API refused
        // an ask step for having no agent
        let state = AppState::for_tests(vec![]);
        state.workflows().create_workflow(ASK_STEP).await.unwrap();
        let run_id = run_workflow(&state, "evidence".into()).await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Waiting);
        state.shutdown();
    }

    #[tokio::test]
    async fn the_run_check_skips_manual_steps() {
        use crate::services::domain::{Agent, AgentId, EntityKind};
        use crate::services::repos::Repository;

        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        for id in ["research-agent", "writer-agent"] {
            let agent = Agent::from_skill_md(
                AgentId::parse(id).unwrap(),
                &format!("---\nname: {id}\n---\nHelp.\n"),
            )
            .unwrap();
            state.store().repos().agents.save(&agent).unwrap();
            state.store().commit(EntityKind::Agent, id).unwrap();
        }
        state
            .workflows()
            .create_workflow(MANUAL_STEP)
            .await
            .unwrap();

        let run_id = run_workflow(&state, "onboarding".into()).await.unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Waiting);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_cancelled_run_drops_its_manual_step() {
        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine.create_workflow(MANUAL_STEP).await.unwrap();
        let run_id = engine.run_workflow("onboarding").await.unwrap();
        assert_eq!(
            wait_for_end(&state, &run_id).await.status,
            WorkflowRunStatus::Waiting
        );
        engine.cancel_run(&run_id).await.unwrap();
        assert!(manual_steps::pending(state.db()).await.unwrap().is_empty());
        assert!(engine
            .complete_manual_step(&run_id, "sign-off", "late".into())
            .await
            .is_err());
        state.shutdown();
    }

    #[test]
    fn manual_steps_need_instructions_and_agent_steps_an_agent() {
        let manual = WorkflowDefinition::parse(MANUAL_STEP).unwrap();
        assert!(manual.steps[1].manual);
        assert_eq!(manual.steps[1].agent, "");
        assert_eq!(manual.steps[1].deadline_hours, Some(48));
        // Existing workflows (no step_type) are agent steps
        let plain = WorkflowDefinition::parse(TWO_STEPS).unwrap();
        assert!(plain.steps.iter().all(|s| !s.manual));

        let no_instructions = MANUAL_STEP.replace(
            "    instructions: \"Check {{research.output}} and add the laptop tag\"\n",
            "",
        );
        let error = WorkflowDefinition::parse(&no_instructions).unwrap_err();
        assert!(error.contains("no instructions"), "{error}");
        let no_agent = TWO_STEPS.replace("    agent: research-agent\n", "");
        let error = WorkflowDefinition::parse(&no_agent).unwrap_err();
        assert!(error.contains("no agent"), "{error}");
        // A manual step's instructions may only refer to earlier steps
        let forward = MANUAL_STEP.replace("{{research.output}}", "{{welcome.output}}");
        assert!(WorkflowDefinition::parse(&forward).is_err());
    }

    #[tokio::test]
    async fn steps_queue_behind_a_busy_agent_and_a_cancelled_step_fails_the_run() {
        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine
            .create_workflow(&TWO_STEPS.replace(
                "input: \"Find today's AI news\"",
                "input: \"Find today's AI news\"
    critical: true",
            ))
            .await
            .unwrap();

        // The first agent is busy, so step one waits in its queue
        state
            .db()
            .call(|conn| {
                crate::db::tests::start_run(conn, "research-agent", "busy");
                Ok(())
            })
            .await
            .unwrap();
        let run_id = engine.run_workflow("two-step").await.unwrap();
        let mut queue = Vec::new();
        for _ in 0..500 {
            queue = crate::services::queue::get_agent_queue(&state, "research-agent".into())
                .await
                .unwrap();
            if !queue.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let [entry] = &queue[..] else {
            panic!("expected one queued step: {queue:?}");
        };
        assert_eq!(entry.source_type, crate::model::SourceType::Workflow);
        assert_eq!(entry.source_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(entry.source_label.as_deref(), Some("Two Step · step 1"));
        assert_eq!(
            entry.priority,
            crate::services::orchestrator::PRIORITY_WORKFLOW_CRITICAL
        );

        // Removing it from the queue fails the step, and with it the run
        crate::services::queue::cancel_queued_run(&state, entry.run_id.clone())
            .await
            .unwrap();
        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert_eq!(run.steps[0].status, WorkflowStepStatus::Failed);
        assert_eq!(run.steps[1].status, WorkflowStepStatus::Pending);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_step_that_may_fail_lets_the_run_continue() {
        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine
            .create_workflow(&TWO_STEPS.replace(
                "input: \"Find today's AI news\"",
                "input: \"Find today's AI news\"
    on_failure: continue",
            ))
            .await
            .unwrap();
        state
            .db()
            .call(|conn| {
                crate::db::tests::start_run(conn, "research-agent", "busy");
                Ok(())
            })
            .await
            .unwrap();
        let run_id = engine.run_workflow("two-step").await.unwrap();
        let mut queue = Vec::new();
        for _ in 0..500 {
            queue = crate::services::queue::get_agent_queue(&state, "research-agent".into())
                .await
                .unwrap();
            if !queue.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        crate::services::queue::cancel_queued_run(&state, queue[0].run_id.clone())
            .await
            .unwrap();

        let run = wait_for_end(&state, &run_id).await;
        assert_eq!(run.status, WorkflowRunStatus::Completed, "{:?}", run.error);
        assert_eq!(run.steps[0].status, WorkflowStepStatus::Failed);
        assert_eq!(run.steps[1].status, WorkflowStepStatus::Done);
        // The failed step's output is empty in later inputs
        assert_eq!(run.steps[1].input.as_deref(), Some("Summarize: "));
        state.shutdown();
    }

    #[tokio::test]
    async fn syncs_files_into_the_table() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let engine = state.workflows().clone();
        // No folder yet: nothing happens
        engine.sync_from_disk().await.unwrap();

        std::fs::create_dir_all(engine.store.repos().workflows.global_dir()).unwrap();
        std::fs::write(engine.path("two-step"), TWO_STEPS).unwrap();
        std::fs::write(
            engine
                .store
                .repos()
                .workflows
                .global_dir()
                .join("broken.yaml"),
            "not: [valid",
        )
        .unwrap();
        std::fs::write(
            engine
                .store
                .repos()
                .workflows
                .global_dir()
                .join("wrong-name.yaml"),
            TWO_STEPS,
        )
        .unwrap();
        engine.sync_from_disk().await.unwrap();
        let listed = state
            .db()
            .call(|conn| db::list_workflows(conn))
            .await
            .unwrap();
        assert_eq!(
            listed.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["two-step"]
        );
        assert!(recorder.names().contains(&"workflow://changed"));

        // Importing a different workflow under an existing id is refused
        let error = engine
            .create_workflow(&TWO_STEPS.replace("Two Step", "Other"))
            .await
            .unwrap_err();
        assert!(error.contains("already exists"));

        std::fs::remove_file(engine.path("two-step")).unwrap();
        engine.sync_from_disk().await.unwrap();
        assert!(state
            .db()
            .call(|conn| db::list_workflows(conn))
            .await
            .unwrap()
            .is_empty());
        state.shutdown();
    }
}
