//! Studio's tools: what it looks at before it answers about this Nandi, the
//! guides on how to do things (`guides`), and the check it runs on every
//! file it proposes. Offered only to the Studio agent; they run on the
//! host, read existing services, change nothing, and never need approval.
//! The exceptions are `nandi_start_request` (`start`), which starts a
//! Request once the user approves its plan, and `nandi_save_artifact`
//! (`save`), which saves a definition once the user approves it. Results are compact JSON the
//! model reads (and the UI shows under "Looked at").

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::model::{RunSummary, STUDIO_AGENT_ID};
use crate::services::ask::ArtifactRef;
use crate::services::llm_keys::{self, KeyedProvider};
use crate::services::{network, snapshots, AppState};

use super::prompt::{CompanionFacts, PromptFacts};
use super::validate::{self, ValidationFacts};

pub const LIST_RUNS: &str = "nandi_list_runs";
pub const GET_RUN: &str = "nandi_get_run";
pub const LIST_REQUESTS: &str = "nandi_list_requests";
pub const GET_REQUEST: &str = "nandi_get_request";
pub const LIST_AGENTS: &str = "nandi_list_agents";
pub const GET_AGENT: &str = "nandi_get_agent";
pub const LIST_WORKFLOWS: &str = "nandi_list_workflows";
pub const GET_WORKFLOW: &str = "nandi_get_workflow";
pub const LIST_TRIGGERS: &str = "nandi_list_triggers";
pub const LIST_TOOLS: &str = "nandi_list_tools";
pub const LIST_CAPABILITIES: &str = "nandi_list_capabilities";
pub const LIST_CONNECTORS: &str = "nandi_list_connectors";
pub const LIST_MODELS: &str = "nandi_list_models";
pub const NETWORK_SNAPSHOT: &str = "nandi_network_snapshot";
pub const CHECK_ARTIFACT: &str = "nandi_check_artifact";
pub const LIST_GUIDES: &str = "nandi_list_guides";
pub const GET_GUIDE: &str = "nandi_get_guide";
pub use super::save::SAVE_ARTIFACT;
pub use super::start::START_REQUEST;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Runs listed at most, newest first
const MAX_RUNS: usize = 50;
/// Characters of a run's input or output in a listing
const EXCERPT_CHARS: usize = 160;
/// Characters of one tool result the model reads
const MAX_RESULT_CHARS: usize = 12_000;

pub fn is_studio_tool(name: &str) -> bool {
    specs().iter().any(|spec| spec.name == name)
}

fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }),
    }
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            LIST_RUNS,
            "List agent runs in a time window, newest first: agent, status, start and end, and a short excerpt of the input and output.",
            json!({
                "window": {"type": "string", "enum": ["today", "24h", "7d", "30d"], "description": "Default: today"},
                "agent": {"type": "string", "description": "Agent ID or name to filter by"},
                "status": {"type": "string", "enum": ["running", "completed", "failed", "cancelled", "awaitingApproval", "queued"]}
            }),
            &[],
        ),
        spec(
            GET_RUN,
            "One run in full: messages, tool calls, approvals and error.",
            json!({"run_id": {"type": "string"}}),
            &["run_id"],
        ),
        spec(
            LIST_REQUESTS,
            "Work in flight and finished in the last day (workflow runs and delegated tasks): title, state, what it's waiting for.",
            json!({}),
            &[],
        ),
        spec(
            GET_REQUEST,
            "One Request: its steps, which one it's on, why it's waiting and who has to act.",
            json!({"id": {"type": "string"}}),
            &["id"],
        ),
        spec(
            LIST_AGENTS,
            "Every agent: id, name, description, model, runtime, tools, whether it accepts requests.",
            json!({}),
            &[],
        ),
        spec(
            GET_AGENT,
            "One agent's skill.md text.",
            json!({"id": {"type": "string"}}),
            &["id"],
        ),
        spec(LIST_WORKFLOWS, "Every workflow: id, name, steps.", json!({}), &[]),
        spec(
            GET_WORKFLOW,
            "One workflow's YAML.",
            json!({"id": {"type": "string"}}),
            &["id"],
        ),
        spec(
            LIST_TRIGGERS,
            "Every trigger: id, name, on or off, what fires it, what it runs.",
            json!({}),
            &[],
        ),
        spec(
            LIST_TOOLS,
            "Tools agents can use (built-in, connector, pilot) and whether each is switched on.",
            json!({}),
            &[],
        ),
        spec(
            LIST_CONNECTORS,
            "Connectors (tools from outside services, shared folders and MCP servers): id, name, installed, the tools each gives, and what's missing before it can start.",
            json!({}),
            &[],
        ),
        spec(
            LIST_CAPABILITIES,
            "Capabilities (what agents can do for others): id, name, whether an agent here provides it, how many providers are online.",
            json!({}),
            &[],
        ),
        spec(
            LIST_MODELS,
            "Which model providers have a key, which can be called now (a key, or the user's Claude Code sign-in for Anthropic), the default model, and the model calls really use.",
            json!({}),
            &[],
        ),
        spec(
            NETWORK_SNAPSHOT,
            "People on the user's network: name, handle, online, whether they take requests.",
            json!({}),
            &[],
        ),
        spec(
            CHECK_ARTIFACT,
            "Check a skill.md or workflow before showing it: the same parsing saving uses, plus unknown tools, models without a key and missing agents. Returns {ok, errors, warnings}.",
            json!({
                "kind": {"type": "string", "enum": ["agent", "workflow"]},
                "id": {"type": "string", "description": "The agent's ID (folder name); for a workflow, its `id:`"},
                "content": {"type": "string", "description": "The complete file"}
            }),
            &["kind", "content"],
        ),
        spec(
            LIST_GUIDES,
            "Guides on how to do things in Nandi: id, title, and when to read each.",
            json!({}),
            &[],
        ),
        spec(
            GET_GUIDE,
            "One guide's text. Read the matching guide before proposing an agent, workflow or trigger, or explaining a feature.",
            json!({"id": {"type": "string"}}),
            &["id"],
        ),
        spec(
            SAVE_ARTIFACT,
            "Save what you built into the user's repo, creating it or replacing it: an agent (skill.md), workflow (YAML), capability or policy (`content`), a trigger (`trigger`), a connector (`connector`), or only an agent's instructions.md (`instructions`, with `content`; the way to change an installed agent's rules, which updates keep). The user approves every save. Check agents and workflows with nandi_check_artifact first.",
            json!({
                "kind": {"type": "string", "enum": super::save::KINDS},
                "id": {"type": "string", "description": "The agent's folder name, workflow, capability, policy or connector ID. A workflow's defaults to its `id:`; a trigger's is in `trigger`"},
                "content": {"type": "string", "description": "agent, workflow, capability, policy, instructions: the complete file"},
                "message": {"type": "string", "description": "Commit message (optional)"},
                "trigger": {
                    "type": "object",
                    "description": "kind trigger: {id?, name, enabled?, sourceType: cron|calendar|file|webhook|device|event, sourceConfig, targetType: agent|workflow, targetId, inputTemplate?, priority?, rateLimitSeconds?}. sourceConfig is {schedule} for cron, {times: ['09:00'], days?: [mon..sun], skipDates?, people?, rotate?: day|week, startDate?, byDay?: {mon: name}, cover?: [{person, from, to, backup?}], backup?, backupAfterMinutes?} for calendar (who is responsible each day, with a backup who takes over), {path, patterns?, events?} for file, {} for webhook, {deviceId} for device, {eventName, filter?} for event. An existing id is updated."
                },
                "connector": {
                    "type": "object",
                    "description": "kind connector: leave out to install a built-in by id. For an MCP server: {kind: \"mcp\", config: {name, command, args, safe_tools} or {name, url}, secret_refs?: {ENV_NAME: \"nandi.connector.<id>.<name>\"}}. Never put a secret in config; the user enters secrets in Settings › Connectors."
                }
            }),
            &["kind"],
        ),
        spec(
            START_REQUEST,
            "Start work for the user as a Request. Call it first with only `text`: it returns a plan (steps, targets, time, cost, warnings) and starts nothing. Then call it again with the same `text` and that `plan` to start it; the user approves the plan first when a step acts. To run an existing workflow, the plan is one `workflow` step with its ID.",
            json!({
                "text": {"type": "string", "description": "The user's ask, self-contained"},
                "plan": {"type": "object", "description": "The plan the first call returned, unchanged"}
            }),
            &["text"],
        ),
    ]
}

fn excerpt(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= EXCERPT_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(EXCERPT_CHARS).collect();
    format!("{cut}…")
}

fn ok(value: Value) -> ToolOutcome {
    let mut result = value.to_string();
    if result.len() > MAX_RESULT_CHARS {
        let cut: String = result.chars().take(MAX_RESULT_CHARS).collect();
        result = format!("{cut}… (truncated)");
    }
    ToolOutcome {
        result,
        is_error: false,
    }
}

fn error(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        result: message.into(),
        is_error: true,
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// Local midnight in ms, from the offset Rust knows (`chrono::Local`)
fn local_midnight(now_ms: i64) -> i64 {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(now_ms)
        .single()
        .and_then(|now| now.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|midnight| Local.from_local_datetime(&midnight).single())
        .map(|midnight| midnight.timestamp_millis())
        .unwrap_or(now_ms - DAY_MS)
}

/// The start of a `window` (today / 24h / 7d / 30d)
pub fn window_start(window: Option<&str>, now_ms: i64) -> i64 {
    match window {
        Some("24h") => now_ms - DAY_MS,
        Some("7d") => now_ms - 7 * DAY_MS,
        Some("30d") => now_ms - 30 * DAY_MS,
        _ => local_midnight(now_ms),
    }
}

/// Runs to list: agents' (never Studio's own), newest first, filtered
pub fn select_runs(
    mut runs: Vec<RunSummary>,
    agent: Option<&str>,
    status: Option<&str>,
    name_of: impl Fn(&str) -> String,
) -> Vec<Value> {
    runs.sort_by(|a, b| b.started_at.total_cmp(&a.started_at));
    runs.into_iter()
        .filter(|run| run.agent_id != STUDIO_AGENT_ID)
        .filter(|run| {
            agent.is_none_or(|agent| {
                run.agent_id == agent || name_of(&run.agent_id).eq_ignore_ascii_case(agent)
            })
        })
        .filter(|run| {
            status.is_none_or(|status| {
                serde_json::to_value(run.status)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .is_some_and(|s| s.eq_ignore_ascii_case(status))
            })
        })
        .take(MAX_RUNS)
        .map(|run| {
            json!({
                "id": run.id,
                "agent": name_of(&run.agent_id),
                "agentId": run.agent_id,
                "status": run.status,
                "startedAt": run.started_at,
                "endedAt": run.ended_at,
                "input": excerpt(&run.input),
                "output": run.summary.as_deref().map(excerpt),
            })
        })
        .collect()
}

/// Runs one Studio tool. Unknown names and bad arguments come back as tool
/// errors the model can read.
pub async fn call(state: &AppState, name: &str, args: &Value) -> ToolOutcome {
    match name {
        LIST_RUNS => {
            let now = chrono::Utc::now().timestamp_millis();
            let since = window_start(arg(args, "window"), now);
            let runs = match state
                .db()
                .call(move |conn| crate::db::runs_since(conn, since))
                .await
            {
                Ok(runs) => runs,
                Err(e) => return error(e),
            };
            let store = state.store();
            let rows = select_runs(runs, arg(args, "agent"), arg(args, "status"), |id| {
                store.display_name(id)
            });
            ok(json!({ "window": arg(args, "window").unwrap_or("today"), "count": rows.len(), "runs": rows }))
        }
        GET_RUN => match arg(args, "run_id") {
            None => error("run_id is required"),
            Some(id) => match snapshots::get_run_detail(state, id.to_string()).await {
                Ok(detail) => ok(serde_json::to_value(detail).unwrap_or_default()),
                Err(e) => error(e),
            },
        },
        LIST_REQUESTS => match crate::services::work::list_work(state, None).await {
            Ok(items) => ok(json!(items
                .iter()
                .map(|item| json!({
                    "title": item.title,
                    "kind": item.kind,
                    "state": item.state,
                    "waitingFor": item.next_action.as_ref().map(|n| n.description.clone()),
                    "progress": item.progress,
                    "startedAt": item.started_at,
                }))
                .collect::<Vec<_>>())),
            Err(e) => error(e.to_string()),
        },
        GET_REQUEST => match arg(args, "id") {
            None => error("id is required"),
            Some(id) => match crate::services::requests::get_request(state, id.to_string()).await {
                Ok(request) => ok(serde_json::to_value(request).unwrap_or_default()),
                Err(e) => error(e),
            },
        },
        LIST_AGENTS => {
            let default =crate::services::orchestrator::default_model(state.store());
            ok(state.store().read(|repo| {
                json!(repo
                    .agents()
                    .map(|agent| {
                        let tools = agent.capabilities().tools();
                        json!({
                            "id": agent.id().as_str(),
                            "name": agent.name(),
                            "description": agent.description(),
                            "model": agent.model().map(ToString::to_string).unwrap_or_else(|| format!("{default} (default)")),
                            "runtime": agent.runtime().as_str(),
                            "allowedTools": tools.allowed(),
                            "requiresApproval": tools.approval_list(),
                            "acceptsRequests": agent.capabilities().accepts_delegation(),
                        })
                    })
                    .collect::<Vec<_>>())
            }))
        }
        GET_AGENT => match arg(args, "id") {
            None => error("id is required"),
            Some(id) => match state.store().read(|repo| repo.agent(id).map(|a| a.to_skill_md())) {
                Some(text) => ok(json!({ "id": id, "skillMd": text })),
                None => error(format!("No agent '{id}'. Call {LIST_AGENTS} for the IDs.")),
            },
        },
        LIST_WORKFLOWS => ok(state.store().read(|repo| {
            json!(repo
                .workflows()
                .map(|workflow| json!({
                    "id": workflow.id().as_str(),
                    "name": workflow.name(),
                    "steps": workflow.steps().iter().map(|step| json!({
                        "id": step.id.as_str(),
                        "agent": step.agent.as_ref().map(|a| a.as_str()),
                    })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>())
        })),
        GET_WORKFLOW => match arg(args, "id") {
            None => error("id is required"),
            Some(id) => match state.store().read(|repo| repo.workflow(id).map(|w| w.to_yaml())) {
                Some(yaml) => ok(json!({ "id": id, "yaml": yaml })),
                None => error(format!("No workflow '{id}'. Call {LIST_WORKFLOWS} for the IDs.")),
            },
        },
        LIST_TRIGGERS => ok(state.store().read(|repo| {
            json!(repo
                .triggers()
                .map(|trigger| json!({
                    "id": trigger.id().as_str(),
                    "name": trigger.name(),
                    "enabled": trigger.enabled(),
                    "source": trigger.source().type_name(),
                    "target": trigger.target().type_name(),
                }))
                .collect::<Vec<_>>())
        })),
        LIST_TOOLS => match state.tools() {
            Some(tools) => match tools.list_all().await {
                Ok(entries) => ok(json!(entries
                    .iter()
                    .map(|entry| json!({
                        "id": entry.definition.name,
                        "description": entry.definition.description,
                        "enabled": entry.enabled,
                    }))
                    .collect::<Vec<_>>())),
                Err(e) => error(e),
            },
            None => ok(json!(crate::services::tools::builtin::builtin_tools()
                .iter()
                .map(|tool| json!({ "id": tool.name, "description": tool.description, "enabled": true }))
                .collect::<Vec<_>>())),
        },
        LIST_CONNECTORS => match crate::services::connectors::list_connectors(state).await {
            Ok(infos) => ok(json!(infos
                .iter()
                .map(|c| json!({
                    "id": c.id,
                    "name": c.name,
                    "description": excerpt(&c.description),
                    "installed": c.status.installed,
                    "comingSoon": c.availability == crate::services::connectors::ConnectorAvailability::ComingSoon,
                    "tools": c.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
                    "config": c.config,
                    "setup": c.setup_hint,
                }))
                .collect::<Vec<_>>())),
            Err(e) => error(e),
        },
        LIST_CAPABILITIES => match crate::services::capability::list_capabilities(state).await {
            Ok(listings) => ok(json!(listings
                .iter()
                .map(|c| json!({
                    "id": c.definition.id,
                    "name": c.definition.name,
                    "description": excerpt(&c.definition.description),
                    "providedHere": c.provided,
                    "onlineProviders": c.online_providers,
                }))
                .collect::<Vec<_>>())),
            Err(e) => error(e),
        },
        LIST_MODELS => {
            let keys = llm_keys::get_llm_key_status(state).await.unwrap_or_default();
            match llm_keys::get_default_model(state).await {
                Ok(model) => ok(json!({
                    "keys": keys,
                    // What calls can use, with Claude Code's sign-in counted
                    "callable": KeyedProvider::all()
                        .into_iter()
                        .filter(|p| llm_keys::has_access(state.secrets(), *p))
                        .collect::<Vec<_>>(),
                    "defaultModel": model.default_model,
                    "modelInUse": model.effective_model,
                })),
                Err(e) => error(e),
            }
        }
        NETWORK_SNAPSHOT => ok(json!(network::known_members(state.db())
            .iter()
            .map(|member| json!({
                "name": member.display_name,
                "handle": member.handle,
                "online": member.online,
                "takesRequests": member.gateway.accepts_delegation,
            }))
            .collect::<Vec<_>>())),
        CHECK_ARTIFACT => {
            let Some(content) = args.get("content").and_then(Value::as_str) else {
                return error("content is required");
            };
            let artifact = match arg(args, "kind") {
                Some("workflow") => ArtifactRef::Workflow(arg(args, "id").unwrap_or_default().to_string()),
                Some("agent") => ArtifactRef::Agent(arg(args, "id").unwrap_or("new-agent").to_string()),
                _ => return error("kind must be 'agent' or 'workflow'"),
            };
            let facts = validation_facts(state).await;
            ok(serde_json::to_value(validate::validate(&artifact, content, &facts)).unwrap_or_default())
        }
        LIST_GUIDES => ok(json!(super::guides::all()
            .iter()
            .map(|guide| json!({ "id": guide.id, "title": guide.title, "when": guide.when }))
            .collect::<Vec<_>>())),
        GET_GUIDE => match arg(args, "id") {
            None => error("id is required"),
            Some(id) => match super::guides::get(id) {
                Some(guide) => ok(json!({ "id": guide.id, "title": guide.title, "body": guide.body })),
                None => error(format!("No guide '{id}'. Call {LIST_GUIDES} for the IDs.")),
            },
        },
        START_REQUEST => super::start::call(state, args).await,
        SAVE_ARTIFACT => super::save::call(state, args).await,
        other => error(format!("Unknown Studio tool '{other}'")),
    }
}

/// Tool IDs in the catalog (the built-ins when the catalog isn't running)
async fn tool_ids(state: &AppState) -> Vec<String> {
    let listed = match state.tools() {
        Some(tools) => tools.list_all().await.ok(),
        None => None,
    };
    match listed {
        Some(entries) if !entries.is_empty() => {
            entries.into_iter().map(|e| e.definition.name).collect()
        }
        _ => crate::services::tools::builtin::builtin_tools()
            .into_iter()
            .map(|t| t.name)
            .collect(),
    }
}

/// Model prefixes calls can use now: providers with a key, and Ollama
fn usable_providers(state: &AppState) -> BTreeSet<String> {
    // A key, or for Anthropic the user's Claude Code sign-in: the same
    // check model calls use (`llm_keys::usable_model`)
    let mut usable: BTreeSet<String> = KeyedProvider::all()
        .into_iter()
        .filter(|p| llm_keys::has_access(state.secrets(), *p))
        .map(|p| {
            match p {
                KeyedProvider::Anthropic => "claude",
                KeyedProvider::OpenAi => "gpt",
                KeyedProvider::DeepSeek => "deepseek",
            }
            .to_string()
        })
        .collect();
    // The free trial, when the user has no key of their own
    if usable.is_empty() && crate::services::llm_trial::available(state.secrets()) {
        usable.insert("deepseek".to_string());
    }
    usable.insert("ollama".to_string());
    usable
}

pub async fn validation_facts(state: &AppState) -> ValidationFacts {
    let manifest = state.store().read(|repo| repo.connectors().clone());
    let folders = manifest
        .connectors()
        .iter()
        .filter(|c| c.kind == "filesystem")
        .map(|c| {
            let ids = crate::services::connectors::folders::shared_folders(c)
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.id)
                .collect();
            (c.id.to_string(), ids)
        })
        .collect();
    ValidationFacts {
        connectors: manifest
            .connectors()
            .iter()
            .map(|c| c.id.to_string())
            .collect(),
        folders,
        tool_ids: tool_ids(state).await.into_iter().collect(),
        usable_providers: usable_providers(state),
        agents: state
            .store()
            .read(|repo| repo.agents().map(|a| a.id().to_string()).collect()),
    }
}

/// What the Studio prompt needs this turn; `conversation_id` tells whether
/// it's the companion's conversation
pub async fn prompt_facts(state: &AppState, conversation_id: &str) -> PromptFacts {
    let default = crate::services::orchestrator::default_model(state.store());
    let usable = usable_providers(state);
    // The user's default first (if callable), then each keyed provider's
    let mut usable_models: Vec<String> = Vec::new();
    if default
        .split_once(':')
        .is_some_and(|(prefix, _)| usable.contains(prefix) && prefix != "ollama")
    {
        usable_models.push(default.clone());
    }
    for (prefix, model) in [
        ("claude", "claude:claude-sonnet-5"),
        ("gpt", "gpt:gpt-5"),
        ("deepseek", "deepseek:deepseek-flash"),
    ] {
        if usable.contains(prefix) && !usable_models.iter().any(|m| m == model) {
            usable_models.push(model.to_string());
        }
    }
    let default_model = llm_keys::usable_model(state.secrets(), &default).unwrap_or(default);
    let companion = crate::services::companion::prompt_context(state, conversation_id)
        .await
        .map(|context| CompanionFacts {
            request_id: context.request_id,
        });
    PromptFacts {
        tool_ids: tool_ids(state).await,
        default_model,
        usable_models,
        network_section: crate::services::a2a::tool::studio_network_section(
            &network::known_members(state.db()),
        ),
        // The companion floats over other apps: Studio's editor isn't "this"
        open_artifact: companion
            .is_none()
            .then(|| state.studio().and_then(|studio| studio.open_artifact()))
            .flatten(),
        companion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RunStatus;

    fn run_summary(id: &str, agent_id: &str, status: RunStatus) -> RunSummary {
        RunSummary {
            id: id.into(),
            agent_id: agent_id.into(),
            conversation_id: format!("agent:{agent_id}"),
            status,
            input: "Do the thing".into(),
            summary: None,
            started_at: 0.0,
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
        }
    }

    #[test]
    fn every_tool_is_read_only_and_named_once() {
        let names: Vec<String> = specs().into_iter().map(|s| s.name).collect();
        let unique: BTreeSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        assert!(names.iter().all(|n| n.starts_with("nandi_")));
        assert!(is_studio_tool(LIST_RUNS) && !is_studio_tool("shell"));
    }

    #[test]
    fn runs_are_listed_newest_first_without_studio() {
        let mut a = run_summary("r1", "writer", RunStatus::Completed);
        a.started_at = 1.0;
        let mut b = run_summary("r2", "reader", RunStatus::Failed);
        b.started_at = 2.0;
        let studio = run_summary("r3", STUDIO_AGENT_ID, RunStatus::Completed);
        let name = |id: &str| {
            if id == "writer" {
                "Writer".to_string()
            } else {
                id.to_string()
            }
        };

        let rows = select_runs(vec![a.clone(), b.clone(), studio], None, None, name);
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["r2", "r1"]);

        let by_name = select_runs(vec![a.clone(), b.clone()], Some("writer"), None, name);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0]["agent"], "Writer");
        let failed = select_runs(vec![a, b], None, Some("failed"), name);
        assert_eq!(failed[0]["id"], "r2");
    }

    #[test]
    fn windows_start_where_they_say() {
        let now = 10 * DAY_MS;
        assert_eq!(window_start(Some("24h"), now), now - DAY_MS);
        assert_eq!(window_start(Some("7d"), now), now - 7 * DAY_MS);
        let today = window_start(None, now);
        assert!(today <= now && today > now - DAY_MS - 3_600_000);
    }

    #[test]
    fn excerpts_are_short() {
        assert_eq!(excerpt("  hi "), "hi");
        assert_eq!(excerpt(&"x".repeat(500)).chars().count(), EXCERPT_CHARS + 1);
    }
}
