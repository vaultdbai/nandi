//! Workflows as capability providers.
//!
//! A workflow YAML may declare `provides: [capability-id, ...]`, like an
//! agent. When selection picks a workflow provider, the provider-side
//! executor (`a2a::executor::run_task`) calls `resolve` before it admits
//! an agent: `Some` means a workflow runs the task (`run`), `None` means
//! the target is an agent and the existing path runs it.
//!
//! - Providers: `refresh` keeps this instance's `capability_providers`
//!   rows with `provider_kind = 'workflow'` in step with the workflows'
//!   `provides:` (at startup and on every workflow or agent change), and
//!   emits `provider://registered` / `provider://unregistered`. Agent rows
//!   are never touched.
//! - A workflow provider's tools are its step agents' tools: their own
//!   `capability_providers` rows' `provided_tools`, plus the core runtime
//!   tools their skill.md allows.
//! - Targets: `<instance>/<workflow_id>` (the provider key selection
//!   ranks) or `<instance>/workflow:<workflow_id>`. A bare ID that is also
//!   a local agent stays with the agent; the prefix forces the workflow.
//! - The task's input fills `{{input}}` / `{{input.<field>}}` in the step
//!   templates; the result is the last step's output.
//! - Each run leaves a `workflow_steps` task message (per-step timings),
//!   which becomes the evidence row's `step_breakdown` (`step_breakdown`;
//!   the aggregation job back-fills rows recorded without it).
//! - Schema checks only warn: a capability may be defined after the
//!   workflow, or never locally.
//!
//! The executor's deps don't carry the workflow engine, so `init`
//! registers each instance's engine here (like
//! `a2a::executor::set_gateway_handler`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::definition::{CapabilityDefinition, ProviderKind};
use super::{PROVIDER_REGISTERED, PROVIDER_UNREGISTERED};
use crate::db::{self, Db};
use crate::services::a2a::{task, A2aDeps, Task, TaskStatus};
use crate::services::domain::{self, Agent, ChangedBy, DomainEvent, EntityKind, Problem};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::network;
use crate::services::orchestrator::tool_search::CORE_TOOLS;
use crate::services::repo_store::{EventPublisher, RepoStore};
use crate::services::tools::definition::normalize_id;
use crate::services::workflows::{self, WorkflowEngine, WorkflowRun, WorkflowStepStatus};
use crate::services::AppState;

/// Forces a target to mean the workflow: `<instance>/workflow:<id>`
pub const WORKFLOW_TARGET_PREFIX: &str = "workflow:";
/// `task_messages.kind` of a workflow run's per-step timings
pub const WORKFLOW_STEPS_KIND: &str = "workflow_steps";

/// What a workflow run produced for a task
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDispatchOutcome {
    pub status: TaskStatus,
    pub output: Value,
    pub error: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    /// Per-step timings for `capability_evidence.step_breakdown`
    pub step_breakdown: Option<Value>,
}

impl WorkflowDispatchOutcome {
    fn failed(error: String) -> Self {
        Self {
            status: TaskStatus::Failed,
            output: Value::Null,
            error: Some(error),
            tokens_in: 0,
            tokens_out: 0,
            step_breakdown: None,
        }
    }
}

/// A local workflow that provides a capability
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowProvider {
    pub workflow_id: String,
    pub capability_id: String,
    /// Schema mismatches against the capability (warnings, not errors)
    pub warnings: Vec<String>,
}

/// `provider://registered` / `provider://unregistered` payload
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderChange {
    pub capability_id: String,
    pub instance_id: String,
    pub provider_kind: ProviderKind,
    pub provider_id: String,
}

#[derive(Serialize)]
#[serde(transparent)]
struct ProviderRegistered(ProviderChange);

impl NandiEvent for ProviderRegistered {
    const NAME: &'static str = PROVIDER_REGISTERED;
}

#[derive(Serialize)]
#[serde(transparent)]
struct ProviderUnregistered(ProviderChange);

impl NandiEvent for ProviderUnregistered {
    const NAME: &'static str = PROVIDER_UNREGISTERED;
}

// ---------------------------------------------------------------------------
// Engines per instance
// ---------------------------------------------------------------------------

static ENGINES: LazyLock<Mutex<HashMap<String, Weak<WorkflowEngine>>>> =
    LazyLock::new(Mutex::default);

/// Makes `instance_id`'s executors run workflow providers on `engine`
/// (replacing any earlier one)
pub fn register_engine(instance_id: &str, engine: &Arc<WorkflowEngine>) {
    ENGINES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(instance_id.to_string(), Arc::downgrade(engine));
}

fn engine_for(instance_id: &str) -> Option<Arc<WorkflowEngine>> {
    ENGINES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(instance_id)
        .and_then(Weak::upgrade)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// A task the executor hands to a workflow
pub struct WorkflowTarget {
    pub workflow_id: String,
    engine: Arc<WorkflowEngine>,
}

/// The workflow `task` targets, when it is a local workflow provider of
/// the task's capability (and the instance's engine is registered)
pub async fn resolve(deps: &A2aDeps, task: &Task) -> Option<WorkflowTarget> {
    let (instance, provider) = task.target.split_once('/')?;
    if instance != deps.instance_id {
        return None;
    }
    let (workflow_id, forced) = match provider.strip_prefix(WORKFLOW_TARGET_PREFIX) {
        Some(id) => (id.to_string(), true),
        None => (provider.to_string(), false),
    };
    let (capability, instance_id, id) = (
        task.capability.clone(),
        deps.instance_id.clone(),
        workflow_id.clone(),
    );
    let found = deps
        .db
        .call(move |conn| {
            let provides = provides_row(conn, &capability, &instance_id, &id)?;
            let agent = !forced && local_agent_exists(conn, &id)?;
            Ok::<_, rusqlite::Error>(provides && !agent)
        })
        .await;
    match found {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => {
            log::warn!("Task {}: workflow provider lookup failed: {e}", task.id);
            return None;
        }
    }
    let Some(engine) = engine_for(&deps.instance_id) else {
        log::warn!(
            "Task {}: workflow '{workflow_id}' provides '{}' but no workflow engine is registered",
            task.id,
            task.capability
        );
        return None;
    };
    Some(WorkflowTarget {
        workflow_id,
        engine,
    })
}

fn provides_row(
    conn: &Connection,
    capability: &str,
    instance_id: &str,
    workflow_id: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM capability_providers
         WHERE capability_id = ?1 AND instance_id = ?2 AND provider_id = ?3
           AND provider_kind = 'workflow' AND COALESCE(enabled, 1) = 1",
        params![capability, instance_id, workflow_id],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
}

fn local_agent_exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT 1 FROM agents WHERE id = ?1", [id], |_| Ok(()))
        .optional()
        .map(|row| row.is_some())
}

/// Runs `task` as `target`'s workflow: `on_progress(settled, total,
/// step)` as steps settle, then the outcome, with the per-step breakdown
/// recorded as a `workflow_steps` task message. The caller applies the
/// deadline.
pub async fn run(
    db: &Db,
    task: &Task,
    target: &WorkflowTarget,
    on_progress: impl Fn(usize, usize, &str) + Send,
) -> WorkflowDispatchOutcome {
    let run = match target
        .engine
        .run_for_task(&target.workflow_id, &task.id, &task.input, on_progress)
        .await
    {
        Ok(run) => run,
        Err(e) => return WorkflowDispatchOutcome::failed(e),
    };
    let breakdown = step_breakdown_of(&run);
    if let Err(e) = task::add_message(db, &task.id, WORKFLOW_STEPS_KIND, breakdown.clone()).await {
        log::warn!("Task {}: step breakdown not recorded: {e}", task.id);
    }
    let run_ids: Vec<String> = run
        .steps
        .iter()
        .filter_map(|s| s.agent_run_id.clone())
        .collect();
    let (tokens_in, tokens_out) = db
        .call(move |conn| run_tokens(conn, &run_ids))
        .await
        .unwrap_or_default();
    let (status, output, error) = match workflows::run_output(&run) {
        Ok(output) => (TaskStatus::Completed, Value::String(output), None),
        Err(e) => (TaskStatus::Failed, Value::Null, Some(e)),
    };
    WorkflowDispatchOutcome {
        status,
        output,
        error,
        tokens_in,
        tokens_out,
        step_breakdown: Some(breakdown),
    }
}

/// `resolve` then `run`, without lifecycle messages or a deadline (tests,
/// and callers without a provider row). `None` for agent targets.
#[cfg(test)]
pub async fn try_dispatch(deps: &A2aDeps, task: &Task) -> Option<WorkflowDispatchOutcome> {
    let target = resolve(deps, task).await?;
    Some(run(&deps.db, task, &target, |_, _, _| {}).await)
}

/// Per-step timings of an ended run (`capability_evidence.step_breakdown`)
pub fn step_breakdown_of(run: &WorkflowRun) -> Value {
    let steps: Vec<Value> = run
        .steps
        .iter()
        .map(|step| {
            let duration_ms = match (step.started_at, step.ended_at) {
                (Some(start), Some(end)) => Some((end - start).max(0.0)),
                _ => None,
            };
            json!({
                "stepId": step.step_id,
                "agentId": step.agent_id,
                "status": step_status(step.status),
                "durationMs": duration_ms,
                "error": step.error,
            })
        })
        .collect();
    json!({
        "workflowId": run.workflow_id,
        "runId": run.id,
        "status": run.status.as_str(),
        "steps": steps,
    })
}

fn step_status(status: WorkflowStepStatus) -> &'static str {
    match status {
        WorkflowStepStatus::Pending => "pending",
        WorkflowStepStatus::AwaitingApproval => "awaiting_approval",
        WorkflowStepStatus::AwaitingInput => "awaiting_input",
        WorkflowStepStatus::Running => "running",
        WorkflowStepStatus::Done => "done",
        WorkflowStepStatus::Failed => "failed",
        WorkflowStepStatus::Rejected => "rejected",
    }
}

/// The step breakdown recorded for `task_id`, if a workflow ran it (for
/// its evidence row)
#[cfg(test)]
pub fn step_breakdown(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<Value>> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM task_messages WHERE task_id = ?1 AND kind = ?2
             ORDER BY id DESC LIMIT 1",
            params![task_id, WORKFLOW_STEPS_KIND],
            |row| row.get(0),
        )
        .optional()?;
    Ok(payload.and_then(|p| serde_json::from_str(&p).ok()))
}

/// Tokens the step agents' runs used
fn run_tokens(conn: &Connection, run_ids: &[String]) -> rusqlite::Result<(u32, u32)> {
    let (mut tokens_in, mut tokens_out) = (0u32, 0u32);
    for id in run_ids {
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT input_tokens, output_tokens FROM runs WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((i, o)) = row {
            tokens_in = tokens_in.saturating_add(i.clamp(0, u32::MAX as i64) as u32);
            tokens_out = tokens_out.saturating_add(o.clamp(0, u32::MAX as i64) as u32);
        }
    }
    Ok((tokens_in, tokens_out))
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// One workflow's `capability_providers` rows to be
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowProviderSpec {
    pub workflow_id: String,
    pub capabilities: Vec<String>,
    pub provided_tools: Vec<String>,
}

/// (capability ID, workflow ID) pairs added and removed
type ProviderDiff = (Vec<(String, String)>, Vec<(String, String)>);

/// Replaces this instance's workflow rows with `specs` (keeping
/// `registered_at` and `enabled` of rows that stay). Returns the pairs
/// added and removed.
pub fn write_providers(
    conn: &mut Connection,
    instance_id: &str,
    specs: &[WorkflowProviderSpec],
) -> rusqlite::Result<ProviderDiff> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let existing: HashSet<(String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT capability_id, provider_id FROM capability_providers
             WHERE instance_id = ?1 AND provider_kind = 'workflow'",
        )?;
        let rows = stmt.query_map([instance_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut wanted = HashSet::new();
    let now = db::now_ms();
    for spec in specs {
        let tools = serde_json::to_string(&spec.provided_tools).unwrap_or_else(|_| "[]".into());
        for capability in &spec.capabilities {
            wanted.insert((capability.clone(), spec.workflow_id.clone()));
            tx.execute(
                "INSERT INTO capability_providers
                     (capability_id, capability_version, instance_id, provider_kind,
                      provider_id, enabled, max_concurrent, provided_tools, registered_at)
                 VALUES (?1, NULL, ?2, 'workflow', ?3, 1, NULL, ?4, ?5)
                 ON CONFLICT (capability_id, instance_id, provider_id) DO UPDATE SET
                     provided_tools = excluded.provided_tools
                 WHERE provider_kind = 'workflow'",
                params![capability, instance_id, spec.workflow_id, tools, now],
            )?;
        }
    }
    let mut removed: Vec<(String, String)> = existing.difference(&wanted).cloned().collect();
    for (capability, workflow_id) in &removed {
        tx.execute(
            "DELETE FROM capability_providers
             WHERE capability_id = ?1 AND instance_id = ?2 AND provider_id = ?3
               AND provider_kind = 'workflow'",
            params![capability, instance_id, workflow_id],
        )?;
    }
    tx.commit()?;
    let mut added: Vec<(String, String)> = wanted.difference(&existing).cloned().collect();
    added.sort();
    removed.sort();
    Ok((added, removed))
}

/// A workflow provider's tools: the step agents' advertised tools
/// (`agent_tools`, by agent ID) plus the core runtime tools each step
/// agent's skill.md allows. Sorted, unique, dotted IDs.
pub fn workflow_tools(
    step_agents: &[Option<&Agent>],
    agent_tools: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut tools = BTreeSet::new();
    for agent in step_agents.iter().flatten() {
        let policy = agent.capabilities().tools();
        for core in CORE_TOOLS {
            if policy.allows(core) {
                tools.insert(normalize_id(core));
            }
        }
        if let Some(advertised) = agent_tools.get(agent.id().as_str()) {
            tools.extend(advertised.iter().map(|t| normalize_id(t)));
        }
    }
    tools.into_iter().collect()
}

/// `{{ input }}` / `{{ input.<field> }}` (as `workflows::render_task_input`)
static INPUT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*input(?:\.([A-Za-z0-9_-]+))?\s*\}\}").unwrap());

/// Whether a template reads the whole input, and the fields it reads
fn input_refs(template: &str) -> (bool, Vec<String>) {
    let mut whole = false;
    let mut fields = Vec::new();
    for caps in INPUT_REF.captures_iter(template) {
        match caps.get(1) {
            Some(field) => fields.push(field.as_str().to_string()),
            None => whole = true,
        }
    }
    (whole, fields)
}

/// Where `workflow` doesn't match `capability`'s schemas. Warnings only.
pub fn schema_warnings(
    workflow: &domain::Workflow,
    capability_id: &str,
    capability: Option<&CapabilityDefinition>,
) -> Vec<String> {
    let Some(capability) = capability else {
        return vec![format!(
            "Capability '{capability_id}' has no definition on this instance yet"
        )];
    };
    let mut warnings = Vec::new();
    let (mut whole, mut fields) = (false, BTreeSet::new());
    for step in workflow.steps() {
        let (w, f) = input_refs(&step.input_template);
        whole |= w;
        fields.extend(f);
    }
    if let Some(input) = &capability.schemas.input {
        let properties: BTreeSet<String> = input
            .get("properties")
            .and_then(Value::as_object)
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default();
        if !whole && fields.is_empty() {
            warnings.push(format!(
                "'{capability_id}' has an input schema, but no step reads {{{{input}}}}"
            ));
        }
        for field in &fields {
            if !properties.is_empty() && !properties.contains(field) {
                warnings.push(format!(
                    "A step reads input.{field}, which the input schema of '{capability_id}' doesn't declare"
                ));
            }
        }
        if !whole && !fields.is_empty() {
            let required = input
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            for name in required {
                if !fields.contains(name) {
                    warnings.push(format!(
                        "No step reads the required input '{name}' of '{capability_id}'"
                    ));
                }
            }
        }
    }
    if let Some(kind) = capability
        .schemas
        .output
        .as_ref()
        .and_then(|output| output.get("type"))
        .and_then(Value::as_str)
        .filter(|kind| *kind != "string")
    {
        warnings.push(format!(
            "'{capability_id}' expects {kind} output; a workflow returns its last step's text"
        ));
    }
    warnings
}

/// The projected definition of `id`, if one is projected
fn projected_definition(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<CapabilityDefinition>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT definition_json FROM capabilities_projection WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// Advertised tools of this instance's agent providers, by agent ID
fn agent_provider_tools(
    conn: &Connection,
    instance_id: &str,
) -> rusqlite::Result<HashMap<String, Vec<String>>> {
    let mut stmt = conn.prepare(
        "SELECT provider_id, provided_tools FROM capability_providers
         WHERE instance_id = ?1 AND provider_kind = 'agent'",
    )?;
    let rows = stmt.query_map([instance_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut tools: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (agent, json) = row?;
        let list: Vec<String> = json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        tools.entry(agent).or_default().extend(list);
    }
    Ok(tools)
}

/// Re-derives this instance's workflow provider rows from the loaded
/// workflows, emits the provider events, and returns every (workflow,
/// capability) pair with its schema warnings (logged too).
/// `refresh_local_providers` calls it after the agent rows.
pub async fn refresh(
    db: &Db,
    bus: &EventBus,
    store: &RepoStore,
    instance_id: &str,
) -> Result<Vec<WorkflowProvider>, String> {
    let (workflows, agents): (Vec<domain::Workflow>, HashMap<String, Agent>) = store.read(|repo| {
        (
            repo.workflows()
                .filter(|w| !w.provides().is_empty())
                .cloned()
                .collect(),
            repo.agents()
                .map(|a| (a.id().to_string(), a.clone()))
                .collect(),
        )
    });
    let instance = instance_id.to_string();
    let (diff, providers) = db
        .call(move |conn| {
            let agent_tools = agent_provider_tools(conn, &instance)?;
            let mut specs = Vec::new();
            let mut providers = Vec::new();
            for workflow in &workflows {
                let steps: Vec<Option<&Agent>> = workflow
                    .agents()
                    .map(|id| agents.get(id.as_str()))
                    .collect();
                specs.push(WorkflowProviderSpec {
                    workflow_id: workflow.id().to_string(),
                    capabilities: workflow.provides().to_vec(),
                    provided_tools: workflow_tools(&steps, &agent_tools),
                });
                for capability in workflow.provides() {
                    let definition = projected_definition(conn, capability)?;
                    providers.push(WorkflowProvider {
                        workflow_id: workflow.id().to_string(),
                        capability_id: capability.clone(),
                        warnings: schema_warnings(workflow, capability, definition.as_ref()),
                    });
                }
            }
            let diff = write_providers(conn, &instance, &specs)?;
            Ok::<_, rusqlite::Error>((diff, providers))
        })
        .await
        .map_err(|e| format!("Failed to refresh workflow providers: {e}"))?;
    let change = |(capability_id, provider_id): (String, String)| ProviderChange {
        capability_id,
        instance_id: instance_id.to_string(),
        provider_kind: ProviderKind::Workflow,
        provider_id,
    };
    let (added, removed) = diff;
    for pair in added {
        events::emit(bus, &ProviderRegistered(change(pair)));
    }
    for pair in removed {
        events::emit(bus, &ProviderUnregistered(change(pair)));
    }
    for provider in &providers {
        for warning in &provider.warnings {
            log::warn!("Workflow '{}': {warning}", provider.workflow_id);
        }
    }
    Ok(providers)
}

/// Refreshes the workflow providers when a workflow or agent changes
struct Refresher {
    db: Db,
    bus: EventBus,
    store: RepoStore,
    instance_id: String,
    runtime: tokio::runtime::Handle,
}

impl Refresher {
    async fn refresh(&self) {
        if let Err(e) = refresh(&self.db, &self.bus, &self.store, &self.instance_id).await {
            log::warn!("{e}");
        }
    }
}

impl EventPublisher for Refresher {
    fn changed(&self, event: &DomainEvent) {
        if !matches!(event.kind, EntityKind::Workflow | EntityKind::Agent) {
            return;
        }
        let me = Refresher {
            db: self.db.clone(),
            bus: self.bus.clone(),
            store: self.store.clone(),
            instance_id: self.instance_id.clone(),
            runtime: self.runtime.clone(),
        };
        self.runtime.spawn(async move { me.refresh().await });
    }

    fn invalid(&self, _problem: &Problem, _changed_by: ChangedBy) {}
}

/// Registers the instance's workflow engine for dispatch, projects the
/// workflow providers, and keeps them current. Must not block.
pub fn init(state: &AppState) {
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(id) => id,
        Err(e) => {
            log::error!("Workflow providers disabled: no instance ID ({e})");
            return;
        }
    };
    register_engine(&instance_id, state.workflows());
    let refresher = Arc::new(Refresher {
        db: state.db().clone(),
        bus: state.bus().clone(),
        store: state.store().clone(),
        instance_id,
        runtime: state.runtime().clone(),
    });
    state.store().subscribe(refresher.clone());
    state
        .runtime()
        .spawn(async move { refresher.refresh().await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent_card::OwnerRef;
    use crate::services::domain::AgentCard;

    fn workflow(yaml: &str) -> domain::Workflow {
        domain::Workflow::from_yaml(yaml).unwrap()
    }

    const REVIEWER: &str = "id: reviewer\nname: Reviewer\nprovides: [code-review.python]\nsteps:\n  - {id: review, agent: research-agent, input: 'Review {{input}}'}\n  - {id: sum, agent: writer-agent, input: 'Sum: {{review.output}}'}\n";

    /// The canned engine echoes each message back, in brackets
    async fn mocked_agents(state: &AppState) {
        state
            .db()
            .call(|conn| {
                db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
                db::set_setting(
                    conn,
                    crate::engine::CANNED_SCRIPT_KEY,
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

    fn provider_rows(state: &AppState) -> Vec<(String, String, String)> {
        let conn = state.db().conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT capability_id, provider_kind, provider_id FROM capability_providers
                 ORDER BY capability_id, provider_id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn requester() -> AgentCard {
        AgentCard {
            id: "me/requester".into(),
            name: "Requester".into(),
            description: String::new(),
            instance_id: "me".into(),
            endpoint: String::new(),
            capabilities: Vec::new(),
            accepts_delegation: false,
            max_concurrent: 1,
            current_load: 0,
            owner: OwnerRef::default(),
            signature: None,
            issued_at: 0.0,
        }
    }

    fn task_for(instance: &str, target: &str, capability: &str) -> Task {
        Task {
            id: format!("t-{}", uuid::Uuid::new_v4()),
            requester: requester(),
            target: format!("{instance}/{target}"),
            capability: capability.to_string(),
            input: json!({ "text": "main.py" }),
            created_at: db::ts(db::now_ms()),
            deadline: None,
        }
    }

    #[tokio::test]
    async fn a_workflow_with_provides_is_registered_as_a_provider() {
        let state = AppState::for_tests(vec![]);
        let engine = state.workflows().clone();
        engine.create_workflow(REVIEWER).await.unwrap();
        // Without provides: not a provider
        engine
            .create_workflow("id: plain\nname: Plain\nsteps:\n  - {id: a, agent: x, input: hi}\n")
            .await
            .unwrap();
        let instance = network::local_instance_id(state.db()).unwrap();
        let providers = refresh(state.db(), state.bus(), state.store(), &instance)
            .await
            .unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].workflow_id, "reviewer");
        assert_eq!(providers[0].capability_id, "code-review.python");
        // No definition projected: a warning, not an error
        assert!(providers[0].warnings[0].contains("no definition"));
        assert_eq!(
            provider_rows(&state),
            [(
                "code-review.python".to_string(),
                "workflow".to_string(),
                "reviewer".to_string()
            )]
        );

        // Agent rows are left alone; a dropped provides: unregisters
        state
            .db()
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO capability_providers (capability_id, instance_id, provider_kind, provider_id)
                 VALUES ('code-review.python', ?1, 'agent', 'research-agent')",
                [&instance],
            )
            .unwrap();
        let mut conn = state.db().conn().unwrap();
        let (added, removed) = write_providers(&mut conn, &instance, &[]).unwrap();
        drop(conn);
        assert!(added.is_empty());
        assert_eq!(
            removed,
            [("code-review.python".to_string(), "reviewer".to_string())]
        );
        let rows = provider_rows(&state);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "agent");
        state.shutdown();
    }

    #[tokio::test]
    async fn dispatch_to_a_workflow_provider_runs_the_workflow() {
        let state = AppState::for_tests(vec![]);
        mocked_agents(&state).await;
        let engine = state.workflows().clone();
        engine.create_workflow(REVIEWER).await.unwrap();
        let instance = network::local_instance_id(state.db()).unwrap();
        refresh(state.db(), state.bus(), state.store(), &instance)
            .await
            .unwrap();
        register_engine(&instance, &engine);
        let deps = A2aDeps::from_state(&state).unwrap();

        let task = task_for(&instance, "reviewer", "code-review.python");
        let done = try_dispatch(&deps, &task).await.expect("a workflow target");
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error);
        assert_eq!(done.output, json!("[Sum: [Review main.py]]"));
        let breakdown = done.step_breakdown.unwrap();
        assert_eq!(breakdown["workflowId"], "reviewer");
        assert_eq!(breakdown["status"], "completed");
        assert_eq!(breakdown["steps"].as_array().unwrap().len(), 2);
        assert_eq!(breakdown["steps"][0]["status"], "done");
        let conn = state.db().conn().unwrap();
        assert_eq!(step_breakdown(&conn, &task.id).unwrap(), Some(breakdown));
        drop(conn);

        // The prefix names the workflow too
        let forced = task_for(&instance, "workflow:reviewer", "code-review.python");
        assert!(resolve(&deps, &forced).await.is_some());
        // Agents, other capabilities, and other instances take the agent path
        for task in [
            task_for(&instance, "research-agent", "code-review.python"),
            task_for(&instance, "reviewer", "something-else"),
            task_for("other-instance", "reviewer", "code-review.python"),
        ] {
            assert!(resolve(&deps, &task).await.is_none(), "{}", task.target);
        }
        state.shutdown();
    }

    #[test]
    fn schema_mismatches_are_warnings() {
        let mut capability = CapabilityDefinition {
            id: "code-review.python".into(),
            ..CapabilityDefinition::default()
        };
        capability.schemas.input = Some(json!({
            "type": "object",
            "properties": { "file": { "type": "string" } },
            "required": ["file", "python_version"]
        }));
        capability.schemas.output = Some(json!({ "type": "object" }));
        let reads_fields = workflow(
            "id: w\nname: W\nsteps:\n  - {id: a, agent: x, input: '{{input.lang}} {{input.file}}'}\n",
        );
        let warnings = schema_warnings(&reads_fields, "code-review.python", Some(&capability));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("input.lang"));
        assert!(warnings[1].contains("python_version"));
        assert!(warnings[2].contains("object output"));

        let ignores_input = workflow("id: w\nname: W\nsteps:\n  - {id: a, agent: x, input: hi}\n");
        let warnings = schema_warnings(&ignores_input, "c", Some(&capability));
        assert!(warnings[0].contains("no step reads"), "{warnings:?}");

        capability.schemas = Default::default();
        assert!(schema_warnings(&ignores_input, "c", Some(&capability)).is_empty());
    }

    #[test]
    fn missing_step_agents_add_no_tools() {
        let tools = workflow_tools(
            &[None],
            &HashMap::from([("x".to_string(), vec!["gmail.send".to_string()])]),
        );
        assert!(tools.is_empty());
    }
}
