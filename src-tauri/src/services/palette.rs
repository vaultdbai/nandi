//! The global command palette: ranked search across agents, workflows,
//! recent runs, marketplace entries, tools, and built-in actions, plus
//! running those actions (wave 2B redesign, Track E).
//!
//! - A plain query searches everything; a query starting with `>` searches
//!   actions only (built-ins plus per-entity ones such as "Pause Writer").
//!   An empty query returns suggestions: a few actions and the latest runs.
//! - Ranking ([`score`]): exact name > name prefix > word prefix > name
//!   substring > description/keyword match. Runs get a recency bonus.
//! - Every result carries the `View` that opens it (`navigate`) or, for
//!   actions, the `args` to pass to [`execute_palette_action`].
//! - Pause/resume stop/start the agent's runtime container
//!   (`agents::stop_agent` / `agents::start_agent`). The service remembers
//!   which agents the palette paused, so "Resume all" restarts only those.
//! - The marketplace catalog is read through the cached registry listing
//!   (read-only), with a short timeout so an unreachable registry never
//!   stalls typing.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::marketplace::manifest::{AgentManifest, PackManifest};
use super::marketplace::MarketplaceFilter;
use super::runs::RunFilters;
use super::{agents, chat, connectors, marketplace, pilots, runs, snapshots, workflows, AppState};
use crate::db;
use crate::model::{RunSummary, WorkflowSummary};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PaletteResult {
    /// `action` | `agent` | `workflow` | `run` | `package` | `tool`
    pub kind: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub score: f64,
    /// For `kind == "action"`: the arguments to run it with
    #[serde(default)]
    pub args: Option<Value>,
    /// The navigation-store `View` (as JSON) that opens this result
    #[serde(default)]
    pub navigate: Option<Value>,
}

/// A built-in action the palette can run (`>` mode).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
}

/// What running an action did: a message to show, and/or a view the UI
/// should navigate to (a navigation-store `View` as JSON).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PaletteActionOutcome {
    pub message: Option<String>,
    pub navigate: Option<Value>,
}

/// Palette state: the agents the palette paused (for "Resume all").
#[derive(Default)]
pub struct PaletteService {
    paused: Mutex<BTreeSet<String>>,
}

impl PaletteService {
    fn paused(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.paused.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub fn init(_state: &AppState) -> Option<Arc<PaletteService>> {
    Some(Arc::new(PaletteService::default()))
}

// ---------------------------------------------------------------------------
// Action registry
// ---------------------------------------------------------------------------

pub const OPEN_VIEW: &str = "open_view";
pub const PAUSE_AGENT: &str = "pause_agent";
pub const RESUME_AGENT: &str = "resume_agent";
pub const PAUSE_ALL: &str = "pause_all";
pub const RESUME_ALL: &str = "resume_all";
pub const INSTALL_AGENT: &str = "install_agent";
pub const RUN_WORKFLOW: &str = "run_workflow";
pub const NEW_STUDIO_CHAT: &str = "new_studio_chat";

fn def(id: &str, name: &str, description: &str, keywords: &[&str]) -> ActionDef {
    ActionDef {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
    }
}

/// Every built-in action. `open_view`, `pause_agent`, `resume_agent`,
/// `install_agent`, and `run_workflow` take arguments, so search lists
/// them per page / agent / workflow / marketplace entry.
pub fn actions() -> Vec<ActionDef> {
    vec![
        def(
            OPEN_VIEW,
            "Open view",
            "Go to a page of the app",
            &["go", "open", "navigate", "view", "page"],
        ),
        def(
            PAUSE_AGENT,
            "Pause agent",
            "Stop an agent's runtime container",
            &["pause", "stop", "agent", "container"],
        ),
        def(
            RESUME_AGENT,
            "Resume agent",
            "Start an agent's runtime container",
            &["resume", "start", "agent", "container"],
        ),
        def(
            PAUSE_ALL,
            "Pause all agents",
            "Stop every agent's runtime container",
            &["pause", "stop", "all", "agents"],
        ),
        def(
            RESUME_ALL,
            "Resume all agents",
            "Restart the agents the palette paused",
            &["resume", "start", "all", "agents"],
        ),
        def(
            INSTALL_AGENT,
            "Install agent",
            "Install an agent from the marketplace",
            &["install", "marketplace", "add", "agent"],
        ),
        def(
            RUN_WORKFLOW,
            "Run workflow",
            "Start a run of a workflow",
            &["run", "start", "workflow", "execute"],
        ),
        def(
            NEW_STUDIO_CHAT,
            "New chat in Ask",
            "Start a new conversation in Ask",
            &["ask", "studio", "chat", "new", "conversation"],
        ),
    ]
}

/// A page `open_view` lists
struct PageView {
    title: &'static str,
    keywords: &'static [&'static str],
    view: Value,
}

fn page(title: &'static str, keywords: &'static [&'static str], view: Value) -> PageView {
    PageView {
        title,
        keywords,
        view,
    }
}

/// Library › Browse, narrowed to tools (connectors and pilots)
fn tools_view() -> Value {
    json!({ "kind": "library", "tab": "browse", "filter": "tools" })
}

/// Library › Browse, where marketplace agents and packs are
fn browse_view() -> Value {
    json!({ "kind": "library", "tab": "browse" })
}

/// The pages `open_view` lists
fn pages() -> Vec<PageView> {
    vec![
        page(
            "Home",
            &["dashboard", "overview", "work"],
            json!({ "kind": "home" }),
        ),
        page(
            "Ask",
            &["chat", "conversations", "studio"],
            json!({ "kind": "studio", "conversationId": null }),
        ),
        page("Agents", &["agents"], json!({ "kind": "agents" })),
        page(
            "All activity",
            &["history", "runs", "delegation"],
            json!({ "kind": "home-activity" }),
        ),
        page(
            "Workflows",
            &["automations", "pipelines"],
            json!({ "kind": "automations", "tab": "workflows" }),
        ),
        page(
            "Triggers",
            &["automations", "schedule", "cron", "webhook"],
            json!({ "kind": "automations", "tab": "triggers" }),
        ),
        page(
            "Library",
            &["marketplace", "store", "packages", "install"],
            browse_view(),
        ),
        page(
            "Tools",
            &["connectors", "pilots", "mcp", "integrations"],
            tools_view(),
        ),
        page(
            "Shared with me",
            &["library", "shared"],
            json!({ "kind": "library", "tab": "shared" }),
        ),
        page(
            "People",
            &["network", "members", "team"],
            json!({ "kind": "network", "tab": "people" }),
        ),
        page(
            "Connection",
            &["network", "relay"],
            json!({ "kind": "network", "tab": "connection" }),
        ),
        page(
            "Trust graph",
            &["network", "trust", "policy"],
            json!({ "kind": "network", "tab": "sharing", "sharing": "callers", "callers": "graph" }),
        ),
        page(
            "Shared by me",
            &["network", "shared"],
            json!({ "kind": "network", "tab": "sharing", "sharing": "shared" }),
        ),
        page(
            "Internet access",
            &["settings", "egress", "proxy", "domains"],
            json!({ "kind": "settings", "section": "internet" }),
        ),
        page(
            "Logs",
            &["settings", "audit"],
            json!({ "kind": "settings", "section": "logs" }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

pub const SCORE_EXACT: f64 = 100.0;
pub const SCORE_PREFIX: f64 = 80.0;
pub const SCORE_WORD: f64 = 60.0;
pub const SCORE_CONTAINS: f64 = 40.0;
pub const SCORE_DESCRIPTION: f64 = 20.0;
/// Taken off a run's match: more than the largest [`recency_bonus`]
const RUN_DISCOUNT: f64 = 15.0;

/// How well `query` matches an item, or 0 for no match. An empty query
/// matches everything weakly.
pub fn score(query: &str, name: &str, description: &str, keywords: &[String]) -> f64 {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 1.0;
    }
    let name = name.to_lowercase();
    if name == query {
        return SCORE_EXACT;
    }
    if name.starts_with(&query) {
        return SCORE_PREFIX;
    }
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens
        .iter()
        .all(|token| words.iter().any(|word| word.starts_with(token)))
    {
        return SCORE_WORD;
    }
    if name.contains(&query) {
        return SCORE_CONTAINS;
    }
    // Every token somewhere: the name's words, the description, a keyword
    let description = description.to_lowercase();
    let anywhere = |token: &&str| {
        words.iter().any(|word| word.starts_with(token))
            || description.contains(token)
            || keywords.iter().any(|k| k.to_lowercase().starts_with(token))
    };
    if tokens.iter().all(anywhere) {
        return SCORE_DESCRIPTION;
    }
    0.0
}

/// Up to 10 points for a run started just now, halving every day.
pub fn recency_bonus(started_at_ms: f64, now_ms: f64) -> f64 {
    let age_days = ((now_ms - started_at_ms) / 86_400_000.0).max(0.0);
    10.0 * 0.5_f64.powf(age_days)
}

/// Drops misses; highest score first, ties by kind order, then name.
pub fn rank(results: &mut Vec<PaletteResult>) {
    results.retain(|r| r.score > 0.0);
    results.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| kind_order(&a.kind).cmp(&kind_order(&b.kind)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn kind_order(kind: &str) -> u8 {
    match kind {
        "action" => 0,
        "agent" => 1,
        "workflow" => 2,
        "run" => 3,
        "package" => 4,
        "tool" => 5,
        _ => 6,
    }
}

/// Results kept per kind, so one kind can't crowd out the rest
const PER_KIND: usize = 8;
/// Results in `>` mode once something is typed
const MAX_ACTIONS: usize = 20;

fn cap_per_kind(results: Vec<PaletteResult>) -> Vec<PaletteResult> {
    let mut counts = HashMap::<String, usize>::new();
    results
        .into_iter()
        .filter(|r| {
            let count = counts.entry(r.kind.clone()).or_default();
            *count += 1;
            *count <= PER_KIND
        })
        .collect()
}

struct Candidate {
    kind: &'static str,
    id: String,
    name: String,
    description: String,
    keywords: Vec<String>,
    args: Option<Value>,
    navigate: Option<Value>,
}

impl Candidate {
    fn scored(self, query: &str) -> PaletteResult {
        let score = score(query, &self.name, &self.description, &self.keywords);
        PaletteResult {
            kind: self.kind.to_string(),
            id: self.id,
            name: self.name,
            description: self.description,
            score,
            args: self.args,
            navigate: self.navigate,
        }
    }

    fn action(
        id: &str,
        name: String,
        description: String,
        keywords: Vec<String>,
        args: Value,
    ) -> Self {
        Self {
            kind: "action",
            id: id.to_string(),
            name,
            description,
            keywords,
            args: Some(args),
            navigate: None,
        }
    }
}

/// Actions that run without arguments, then one "Go to" per page.
fn standalone_actions() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = actions()
        .into_iter()
        .filter(|d| [NEW_STUDIO_CHAT, PAUSE_ALL, RESUME_ALL].contains(&d.id.as_str()))
        .map(|d| Candidate {
            kind: "action",
            id: d.id,
            name: d.name,
            description: d.description,
            keywords: d.keywords,
            args: None,
            navigate: None,
        })
        .collect();
    for PageView {
        title,
        keywords,
        view,
    } in pages()
    {
        let mut words: Vec<String> = keywords.iter().map(|k| k.to_string()).collect();
        words.extend(["go".to_string(), "open".to_string()]);
        out.push(Candidate {
            kind: "action",
            id: OPEN_VIEW.to_string(),
            name: format!("Go to {title}"),
            description: format!("Open {title}"),
            keywords: words,
            args: Some(json!({ "view": view })),
            navigate: Some(view),
        });
    }
    out
}

/// "Pause Writer", "Run Daily Report", "Install Risk Monitor", …
fn per_entity_actions(
    agent_list: &[agents::AgentInfo],
    workflow_list: &[WorkflowSummary],
    installable: &[AgentManifest],
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for agent in agent_list {
        let args = json!({ "agentId": agent.id });
        out.push(Candidate::action(
            PAUSE_AGENT,
            format!("Pause {}", agent.name),
            "Stop the agent's runtime container".to_string(),
            vec!["pause".into(), "stop".into(), agent.id.clone()],
            args.clone(),
        ));
        out.push(Candidate::action(
            RESUME_AGENT,
            format!("Resume {}", agent.name),
            "Start the agent's runtime container".to_string(),
            vec!["resume".into(), "start".into(), agent.id.clone()],
            args,
        ));
    }
    for workflow in workflow_list {
        out.push(Candidate::action(
            RUN_WORKFLOW,
            format!("Run {}", workflow.name),
            "Start a run of this workflow".to_string(),
            vec!["run".into(), "workflow".into(), workflow.id.clone()],
            json!({ "workflowId": workflow.id }),
        ));
    }
    for entry in installable {
        out.push(Candidate::action(
            INSTALL_AGENT,
            format!("Install {}", entry.name),
            format!("From {} (v{})", entry.publisher, entry.latest_version),
            vec!["install".into(), "marketplace".into(), entry.id.clone()],
            json!({ "agentId": entry.id }),
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// How long the palette waits for the (cached) registry listing
const MARKETPLACE_TIMEOUT: Duration = Duration::from_millis(800);

async fn marketplace_agents(state: &AppState) -> Vec<AgentManifest> {
    let filter = MarketplaceFilter::default();
    match tokio::time::timeout(
        MARKETPLACE_TIMEOUT,
        marketplace::list_available_agents(state, &filter),
    )
    .await
    {
        Ok(Ok(list)) => list,
        _ => Vec::new(),
    }
}

async fn marketplace_packs(state: &AppState) -> Vec<PackManifest> {
    match tokio::time::timeout(
        MARKETPLACE_TIMEOUT,
        marketplace::list_available_packs(state),
    )
    .await
    {
        Ok(Ok(list)) => list,
        _ => Vec::new(),
    }
}

fn workflow_description(workflow: &WorkflowSummary) -> String {
    workflow
        .definition
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn truncate(text: &str, max: usize) -> String {
    let text = text.trim().replace('\n', " ");
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

fn run_result(run: &RunSummary, agent_name: &str, base: f64, now: f64) -> PaletteResult {
    let short: String = run.id.chars().take(8).collect();
    let status = serde_json::to_value(run.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let what = if run.input.trim().is_empty() {
        run.source_label.as_deref().unwrap_or(agent_name)
    } else {
        run.input.as_str()
    };
    PaletteResult {
        kind: "run".to_string(),
        id: run.id.clone(),
        name: format!("{agent_name} · run {short}"),
        description: truncate(&format!("{status} · {what}"), 90),
        score: base + recency_bonus(run.started_at, now),
        args: None,
        navigate: Some(json!({ "kind": "run-detail", "runId": run.id })),
    }
}

fn agent_names(agent_list: &[agents::AgentInfo]) -> HashMap<&str, &str> {
    agent_list
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect()
}

/// Ranked results; a query starting with `>` searches only actions
pub async fn search_palette(state: &AppState, query: &str) -> Result<Vec<PaletteResult>, String> {
    let trimmed = query.trim();
    if let Some(rest) = trimmed.strip_prefix('>') {
        return search_actions(state, rest.trim()).await;
    }
    if trimmed.is_empty() {
        return suggestions(state).await;
    }

    let agent_list = agents::list_agents(state).await?;
    let workflow_list = snapshots::get_workflows(state).await?;

    let mut candidates = standalone_actions();
    candidates.extend(agent_list.iter().map(|agent| Candidate {
        kind: "agent",
        id: agent.id.clone(),
        name: agent.name.clone(),
        description: agent.description.clone(),
        keywords: vec![agent.id.clone()],
        args: None,
        navigate: Some(json!({ "kind": "agent", "agentId": agent.id })),
    }));
    candidates.extend(workflow_list.iter().map(|workflow| Candidate {
        kind: "workflow",
        id: workflow.id.clone(),
        name: workflow.name.clone(),
        description: workflow_description(workflow),
        keywords: vec![workflow.id.clone()],
        args: None,
        navigate: Some(json!({ "kind": "workflow", "workflowId": workflow.id })),
    }));
    for entry in marketplace_agents(state).await {
        let mut keywords = entry.tags;
        keywords.push(entry.publisher);
        candidates.push(Candidate {
            kind: "package",
            id: entry.id,
            name: entry.name,
            description: entry.description,
            keywords,
            args: None,
            navigate: Some(browse_view()),
        });
    }
    for pack in marketplace_packs(state).await {
        let mut keywords = pack.tags;
        keywords.extend([pack.publisher, "pack".to_string()]);
        candidates.push(Candidate {
            kind: "package",
            id: pack.id,
            name: pack.name,
            description: pack.description,
            keywords,
            args: None,
            navigate: Some(browse_view()),
        });
    }
    for connector in connectors::list_connectors(state).await.unwrap_or_default() {
        candidates.push(Candidate {
            kind: "tool",
            id: connector.id,
            name: connector.name,
            description: connector.description,
            keywords: vec!["connector".into(), connector.kind],
            args: None,
            navigate: Some(tools_view()),
        });
    }
    for pilot in pilots::list_pilots() {
        let mut keywords = pilot.capabilities;
        keywords.push("pilot".into());
        candidates.push(Candidate {
            kind: "tool",
            id: pilot.id,
            name: pilot.name,
            description: pilot.description,
            keywords,
            args: None,
            navigate: Some(tools_view()),
        });
    }

    let mut results: Vec<PaletteResult> =
        candidates.into_iter().map(|c| c.scored(trimmed)).collect();

    // Runs: the SQL search matches input, summary, source, and agent name
    let names = agent_names(&agent_list);
    let now = db::now_ms() as f64;
    let page = runs::list_all_runs(
        state.db(),
        RunFilters {
            search: Some(trimmed.to_string()),
            ..RunFilters::default()
        },
        PER_KIND as u32,
        0,
    )
    .await?;
    for run in &page.rows {
        let agent_name = names
            .get(run.agent_id.as_str())
            .copied()
            .unwrap_or(run.agent_id.as_str());
        // Scored by its agent's name, so a run sits below that match:
        // recency orders runs, never lifts one over the agent
        let text = score(trimmed, agent_name, &run.input, &[]);
        results.push(run_result(
            run,
            agent_name,
            text.max(SCORE_DESCRIPTION) - RUN_DISCOUNT,
            now,
        ));
    }

    rank(&mut results);
    Ok(cap_per_kind(results))
}

/// `>` mode: built-in actions, plus per-entity ones once the user has
/// typed something (so the bare list stays short)
async fn search_actions(state: &AppState, query: &str) -> Result<Vec<PaletteResult>, String> {
    let mut candidates = standalone_actions();
    if !query.is_empty() {
        let agent_list = agents::list_agents(state).await?;
        let workflow_list = snapshots::get_workflows(state).await?;
        let installable: Vec<_> = marketplace_agents(state)
            .await
            .into_iter()
            .filter(|a| a.installed_version.is_none())
            .collect();
        candidates.extend(per_entity_actions(
            &agent_list,
            &workflow_list,
            &installable,
        ));
    }
    let mut results: Vec<PaletteResult> = candidates.into_iter().map(|c| c.scored(query)).collect();
    rank(&mut results);
    if !query.is_empty() {
        results.truncate(MAX_ACTIONS);
    }
    Ok(results)
}

/// The actions an empty palette suggests, by name
const SUGGESTED: &[&str] = &[
    "New chat in Ask",
    "Go to Home",
    "Go to Library",
    "Go to All activity",
    "Pause all agents",
];

/// Empty query: a few actions, then the latest runs.
async fn suggestions(state: &AppState) -> Result<Vec<PaletteResult>, String> {
    let mut results: Vec<PaletteResult> = standalone_actions()
        .into_iter()
        .filter(|c| SUGGESTED.contains(&c.name.as_str()))
        .map(|c| c.scored(""))
        .collect();
    let agent_list = agents::list_agents(state).await?;
    let names = agent_names(&agent_list);
    let now = db::now_ms() as f64;
    let page = runs::list_all_runs(state.db(), RunFilters::default(), 5, 0).await?;
    for run in &page.rows {
        let agent_name = names
            .get(run.agent_id.as_str())
            .copied()
            .unwrap_or(run.agent_id.as_str());
        results.push(run_result(run, agent_name, 1.0, now));
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ActionArgs {
    agent_id: Option<String>,
    workflow_id: Option<String>,
    view: Option<Value>,
}

fn require(value: Option<String>, action_id: &str, field: &str) -> Result<String, String> {
    value
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("Action '{action_id}' needs '{field}'"))
}

fn service(state: &AppState) -> Result<&Arc<PaletteService>, String> {
    state
        .palette()
        .ok_or_else(|| "The command palette isn't ready".to_string())
}

fn message(text: String) -> PaletteActionOutcome {
    PaletteActionOutcome {
        message: Some(text),
        navigate: None,
    }
}

/// An `open_view` target: an object with a non-empty string `kind`.
fn check_view(view: Option<Value>) -> Result<Value, String> {
    let view = view.ok_or_else(|| format!("Action '{OPEN_VIEW}' needs 'view'"))?;
    match view.get("kind").and_then(Value::as_str) {
        Some(kind) if !kind.is_empty() => Ok(view),
        _ => Err("'view' needs a string 'kind'".to_string()),
    }
}

/// Runs a built-in action. `open_view` is UI-side (it only returns
/// `navigate`); the rest run here and may also return a view to open.
pub async fn execute_palette_action(
    state: &AppState,
    action_id: &str,
    args: Value,
) -> Result<PaletteActionOutcome, String> {
    let args: ActionArgs = if args.is_null() {
        ActionArgs::default()
    } else {
        serde_json::from_value(args).map_err(|e| format!("Invalid arguments: {e}"))?
    };
    match action_id {
        OPEN_VIEW => Ok(PaletteActionOutcome {
            message: None,
            navigate: Some(check_view(args.view)?),
        }),
        PAUSE_AGENT => {
            let id = require(args.agent_id, action_id, "agentId")?;
            state.store().require_agent(&id)?;
            agents::stop_agent(state, &id).await?;
            service(state)?.paused().insert(id.clone());
            Ok(message(format!(
                "Paused {}",
                state.store().display_name(&id)
            )))
        }
        RESUME_AGENT => {
            let id = require(args.agent_id, action_id, "agentId")?;
            agents::start_agent(state, &id).await?;
            service(state)?.paused().remove(&id);
            Ok(message(format!(
                "Resumed {}",
                state.store().display_name(&id)
            )))
        }
        PAUSE_ALL => {
            let agent_list = agents::list_agents(state).await?;
            for agent in &agent_list {
                agents::stop_agent(state, &agent.id).await?;
                service(state)?.paused().insert(agent.id.clone());
            }
            Ok(message(format!("Paused {} agent(s)", agent_list.len())))
        }
        RESUME_ALL => {
            let paused: Vec<String> = service(state)?.paused().iter().cloned().collect();
            if paused.is_empty() {
                return Ok(message(
                    "No agents were paused from the palette".to_string(),
                ));
            }
            let mut resumed = 0;
            let mut failures = Vec::new();
            for id in paused {
                match agents::start_agent(state, &id).await {
                    Ok(_) => {
                        service(state)?.paused().remove(&id);
                        resumed += 1;
                    }
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
            if failures.is_empty() {
                Ok(message(format!("Resumed {resumed} agent(s)")))
            } else {
                Err(format!(
                    "Resumed {resumed} agent(s); failed: {}",
                    failures.join("; ")
                ))
            }
        }
        INSTALL_AGENT => {
            let id = require(args.agent_id, action_id, "agentId")?;
            let installed = marketplace::install_agent(state, &id).await?;
            Ok(PaletteActionOutcome {
                message: Some(format!("Installed {} v{}", installed.id, installed.version)),
                navigate: Some(json!({ "kind": "agent", "agentId": installed.id })),
            })
        }
        RUN_WORKFLOW => {
            let id = require(args.workflow_id, action_id, "workflowId")?;
            workflows::run_workflow(state, id.clone()).await?;
            Ok(PaletteActionOutcome {
                message: Some("Workflow run started".to_string()),
                navigate: Some(json!({ "kind": "workflow", "workflowId": id })),
            })
        }
        NEW_STUDIO_CHAT => {
            let conversation = chat::create_studio_conversation(state).await?;
            Ok(PaletteActionOutcome {
                message: None,
                navigate: Some(json!({ "kind": "studio", "conversationId": conversation.id })),
            })
        }
        other => Err(format!("Unknown palette action '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn result(kind: &str, name: &str, score: f64) -> PaletteResult {
        PaletteResult {
            kind: kind.into(),
            id: name.into(),
            name: name.into(),
            description: String::new(),
            score,
            args: None,
            navigate: None,
        }
    }

    #[test]
    fn ranks_exact_over_prefix_over_word_over_description() {
        let exact = score("writer", "Writer", "", &[]);
        let prefix = score("writ", "Writer", "", &[]);
        let word = score("analyst", "Portfolio Analyst", "", &[]);
        let contains = score("folio", "Portfolio Analyst", "", &[]);
        let description = score("markets", "Risk Monitor", "Watches markets", &[]);
        let keyword = score("cron", "Triggers", "", &kw(&["cron"]));
        assert_eq!(exact, SCORE_EXACT);
        assert_eq!(prefix, SCORE_PREFIX);
        assert_eq!(word, SCORE_WORD);
        assert_eq!(contains, SCORE_CONTAINS);
        assert_eq!(description, SCORE_DESCRIPTION);
        assert_eq!(keyword, SCORE_DESCRIPTION);
        assert!(exact > prefix && prefix > word && word > contains && contains > description);
        assert_eq!(score("zzz", "Writer", "writes", &kw(&["text"])), 0.0);
        assert_eq!(score("WRITER", "writer", "", &[]), SCORE_EXACT);
    }

    #[test]
    fn multi_word_queries_match_word_prefixes() {
        assert_eq!(score("port an", "Portfolio Analyst", "", &[]), SCORE_WORD);
        assert_eq!(score("pause writ", "Pause Writer", "", &[]), SCORE_PREFIX);
        assert_eq!(score("writer pause", "Pause Writer", "", &[]), SCORE_WORD);
        // One token in the name, one only in the keywords
        assert_eq!(
            score("writer stop", "Pause Writer", "", &kw(&["stop"])),
            SCORE_DESCRIPTION
        );
    }

    #[test]
    fn recent_runs_rank_higher() {
        let day = 86_400_000.0;
        let now = 10.0 * day;
        assert!((recency_bonus(now, now) - 10.0).abs() < 1e-9);
        assert!((recency_bonus(now - day, now) - 5.0).abs() < 1e-9);
        assert!(recency_bonus(now - 3.0 * day, now) < recency_bonus(now - day, now));
        // A clock skew can't push the bonus past the maximum
        assert!((recency_bonus(now + day, now) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn rank_sorts_and_drops_misses() {
        let mut results = vec![
            result("agent", "b", 40.0),
            result("tool", "miss", 0.0),
            result("workflow", "a", 80.0),
            result("action", "c", 40.0),
        ];
        rank(&mut results);
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "c", "b"]);
    }

    #[test]
    fn caps_results_per_kind() {
        let results: Vec<PaletteResult> = (0..20)
            .map(|i| result("agent", &format!("a{i}"), 50.0))
            .chain([result("tool", "t", 10.0)])
            .collect();
        let capped = cap_per_kind(results);
        assert_eq!(
            capped.iter().filter(|r| r.kind == "agent").count(),
            PER_KIND
        );
        assert!(capped.iter().any(|r| r.kind == "tool"));
    }

    #[test]
    fn registry_has_every_action() {
        let ids: Vec<String> = actions().into_iter().map(|a| a.id).collect();
        for id in [
            OPEN_VIEW,
            PAUSE_AGENT,
            RESUME_AGENT,
            PAUSE_ALL,
            RESUME_ALL,
            INSTALL_AGENT,
            RUN_WORKFLOW,
            NEW_STUDIO_CHAT,
        ] {
            assert!(ids.iter().any(|i| i == id), "{id}");
        }
    }

    #[test]
    fn per_entity_actions_match_verb_and_name() {
        let agent = agents::AgentInfo {
            id: "writer".into(),
            name: "Writer".into(),
            description: String::new(),
            skill_path: String::new(),
            status: agents::AgentStatus::Idle,
            current_task: None,
            origin: agents::AgentOrigin::Local,
            builtin: false,
        };
        let mut results: Vec<PaletteResult> = per_entity_actions(&[agent], &[], &[])
            .into_iter()
            .map(|c| c.scored("pause wri"))
            .collect();
        rank(&mut results);
        assert_eq!(results.len(), 1, "resume doesn't match 'pause'");
        assert_eq!(results[0].id, PAUSE_AGENT);
        assert_eq!(results[0].args, Some(json!({ "agentId": "writer" })));
    }

    #[test]
    fn go_to_actions_open_their_view() {
        let mut results: Vec<PaletteResult> = standalone_actions()
            .into_iter()
            .map(|c| c.scored("trust"))
            .collect();
        rank(&mut results);
        let trust_graph = json!({ "kind": "network", "tab": "sharing", "sharing": "callers", "callers": "graph" });
        assert_eq!(results[0].name, "Go to Trust graph");
        assert_eq!(results[0].navigate, Some(trust_graph.clone()));
        assert_eq!(results[0].args, Some(json!({ "view": trust_graph })));
    }

    #[tokio::test]
    async fn action_mode_returns_only_actions() {
        let state = AppState::for_tests(vec![]);
        let results = search_palette(&state, ">").await.unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == "action"));

        let results = search_palette(&state, "> library").await.unwrap();
        assert!(results.iter().all(|r| r.kind == "action"));
        assert_eq!(results[0].name, "Go to Library");
        state.shutdown();
    }

    #[tokio::test]
    async fn finds_agents_by_name_and_description() {
        let state = AppState::for_tests(vec![]);
        agents::create_agent(
            &state,
            agents::CreateAgentInput {
                id: "portfolio".into(),
                name: "Portfolio Analyst".into(),
                description: Some("Watches the bond markets".into()),
                model: None,
                instructions: None,
            },
        )
        .await
        .unwrap();
        let results = search_palette(&state, "portfolio").await.unwrap();
        let first = &results[0];
        assert_eq!(
            (first.kind.as_str(), first.id.as_str()),
            ("agent", "portfolio")
        );
        assert_eq!(
            first.navigate,
            Some(json!({ "kind": "agent", "agentId": "portfolio" }))
        );

        // A run started just now still sits below its agent
        state
            .db()
            .call(|conn| {
                let conversation_id = db::ensure_agent_conversation(conn, "portfolio")?;
                db::insert_run(
                    conn,
                    &RunSummary {
                        id: "r1".into(),
                        agent_id: "portfolio".into(),
                        conversation_id,
                        status: crate::model::RunStatus::Completed,
                        input: "Check the yields".into(),
                        summary: None,
                        started_at: db::now_ms() as f64,
                        ended_at: None,
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: None,
                        model: None,
                        source_type: crate::model::SourceType::User,
                        source_id: None,
                        source_label: None,
                        priority: 10,
                        queued_at: None,
                        parent_run_id: None,
                        compaction_summary: None,
                    },
                )
            })
            .await
            .unwrap();
        for query in ["portfolio analyst", "portfolio"] {
            let results = search_palette(&state, query).await.unwrap();
            assert_eq!(results[0].kind, "agent", "{query}: {:?}", results[0]);
            assert!(results.iter().any(|r| r.kind == "run"), "{query}");
        }

        let results = search_palette(&state, "bond").await.unwrap();
        assert!(results
            .iter()
            .any(|r| r.kind == "agent" && r.score == SCORE_DESCRIPTION));

        let results = search_palette(&state, "> pause portf").await.unwrap();
        assert_eq!(results[0].id, PAUSE_AGENT);
        assert_eq!(results[0].args, Some(json!({ "agentId": "portfolio" })));
        state.shutdown();
    }

    #[tokio::test]
    async fn empty_query_suggests_actions() {
        let state = AppState::for_tests(vec![]);
        let results = search_palette(&state, "  ").await.unwrap();
        assert!(results.iter().any(|r| r.id == NEW_STUDIO_CHAT));
        assert!(results
            .iter()
            .all(|r| r.kind == "action" || r.kind == "run"));
        state.shutdown();
    }

    #[tokio::test]
    async fn executes_actions_and_rejects_bad_arguments() {
        let state = AppState::for_tests(vec![]);
        let view = json!({ "kind": "library", "tab": "browse" });
        let outcome = execute_palette_action(&state, OPEN_VIEW, json!({ "view": view }))
            .await
            .unwrap();
        assert_eq!(outcome.navigate, Some(view));

        let error = |r: Result<PaletteActionOutcome, String>| r.unwrap_err();
        assert!(
            error(execute_palette_action(&state, OPEN_VIEW, Value::Null).await).contains("view")
        );
        assert!(
            execute_palette_action(&state, OPEN_VIEW, json!({ "view": {} }))
                .await
                .is_err()
        );
        assert!(
            error(execute_palette_action(&state, PAUSE_AGENT, json!({})).await).contains("agentId")
        );
        assert!(
            execute_palette_action(&state, PAUSE_AGENT, json!({ "agentId": "ghost" }))
                .await
                .is_err()
        );
        assert!(
            error(execute_palette_action(&state, "nope", Value::Null).await).contains("Unknown")
        );
        assert!(
            error(execute_palette_action(&state, OPEN_VIEW, json!("x")).await).contains("Invalid")
        );

        let outcome = execute_palette_action(&state, RESUME_ALL, Value::Null)
            .await
            .unwrap();
        assert!(outcome.message.unwrap().contains("No agents"));

        let outcome = execute_palette_action(&state, NEW_STUDIO_CHAT, Value::Null)
            .await
            .unwrap();
        assert_eq!(outcome.navigate.unwrap()["kind"], "studio");
        state.shutdown();
    }
}
