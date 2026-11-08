//! Deferred tool loading (wave final, Track E). FROZEN signatures.
//!
//! An agent with more than `DEFERRED_LOAD_THRESHOLD` tools starts with the
//! core tools plus `tool_search`; the model searches (BM25 over tool name +
//! description) and the matching tools join the next step's tool list.
//!
//! The per-run state (every tool the agent has, and the ones `tool_search`
//! loaded) lives in memory here, keyed by run ID: `llm_step` records the
//! full list through [`step_tools`] and `run_call` answers `tool_search`
//! through [`call`]. It doesn't survive a restart; a run resumed after one
//! starts again from the core tools, and the model can search again.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::services::events::{self, EventBus, NandiEvent, RunContext};

pub type ToolDef = ToolSpec;

pub const DEFERRED_LOAD_THRESHOLD: usize = 15;
pub const TOOL_SEARCH_NAME: &str = "tool_search";
/// Always loaded when deferring
pub const CORE_TOOLS: &[&str] = &["shell", "read_file", "write_file", "list_dir"];

/// BM25 term-frequency saturation
const K1: f64 = 1.2;
/// BM25 length normalization
const B: f64 = 0.75;
/// `tool_search`'s `limit` when the model leaves it out, and its ceiling
const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
/// Runs whose tool state is kept; the oldest is dropped first
const MAX_TRACKED_RUNS: usize = 128;

/// A BM25 index over tool name + description
#[derive(Debug, Clone, Default)]
pub struct ToolIndex {
    pub tools: Vec<ToolDef>,
}

pub fn build_index(tools: &[ToolDef]) -> ToolIndex {
    ToolIndex {
        tools: tools.to_vec(),
    }
}

/// Lowercase words of `text`, split on anything but letters and digits
/// (so on `_` and `-` too) and at camelCase humps.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The best `limit` matches for `query`, best first
pub fn search<'a>(index: &'a ToolIndex, query: &str, limit: usize) -> Vec<&'a ToolDef> {
    let query: HashSet<String> = tokenize(query).into_iter().collect();
    if query.is_empty() || index.tools.is_empty() || limit == 0 {
        return Vec::new();
    }
    let docs: Vec<Vec<String>> = index
        .tools
        .iter()
        .map(|tool| tokenize(&format!("{} {}", tool.name, tool.description)))
        .collect();
    let n = docs.len() as f64;
    let avg_len = (docs.iter().map(Vec::len).sum::<usize>() as f64 / n).max(1.0);
    let mut df: HashMap<&str, f64> = HashMap::new();
    for doc in &docs {
        let unique: HashSet<&str> = doc.iter().map(String::as_str).collect();
        for term in unique {
            *df.entry(term).or_default() += 1.0;
        }
    }
    let mut scored: Vec<(f64, &ToolDef)> = docs
        .iter()
        .zip(&index.tools)
        .filter_map(|(doc, tool)| {
            let len = doc.len() as f64;
            let score: f64 = query
                .iter()
                .filter_map(|term| {
                    let tf = doc.iter().filter(|t| *t == term).count() as f64;
                    let df = *df.get(term.as_str())?;
                    let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
                    Some(idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / avg_len)))
                })
                .sum();
            (score > 0.0).then_some((score, tool))
        })
        .collect();
    scored.sort_by(|(a, ta), (b, tb)| {
        b.partial_cmp(a)
            .unwrap_or(Ordering::Equal)
            .then_with(|| ta.name.cmp(&tb.name))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, tool)| tool)
        .collect()
}

/// Whether an agent with `tool_count` tools starts with only the core ones
pub fn should_defer(tool_count: usize) -> bool {
    tool_count > DEFERRED_LOAD_THRESHOLD
}

/// The `tool_search` tool the model sees while deferring
pub fn tool_search_spec() -> ToolDef {
    ToolSpec {
        name: TOOL_SEARCH_NAME.to_string(),
        description: "Search this agent's other tools by keyword. Only a few tools are loaded \
up front; the matches (name, description, input schema) are returned and become callable from \
your next step."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords describing what you need, e.g. \"create github issue\""
                },
                "limit": {
                    "type": "integer",
                    "description": format!(
                        "How many tools to load (default {DEFAULT_LIMIT}, at most {MAX_LIMIT})"
                    ),
                    "minimum": 1,
                    "maximum": MAX_LIMIT
                }
            },
            "required": ["query"]
        }),
    }
}

/// Whether `name` is always offered, never behind `tool_search`: the core
/// tools, and the tools of the pilots the agent declares and may use. A
/// browser alone has twenty-odd tools: deferred, the agent saw only its
/// file tools and didn't know to search for the browser
pub fn is_core(name: &str) -> bool {
    CORE_TOOLS.contains(&name) || crate::services::pilots::tools::is_pilot_tool(name)
}

/// Splits `tools` into the core set (the [`is_core`] tools present, then
/// `tool_search`) and the deferred rest, each in its original order.
pub fn split_core(tools: Vec<ToolDef>) -> (Vec<ToolDef>, Vec<ToolDef>) {
    let (mut core, deferred): (Vec<_>, Vec<_>) = tools
        .into_iter()
        .filter(|tool| tool.name != TOOL_SEARCH_NAME)
        .partition(|tool| is_core(&tool.name));
    core.push(tool_search_spec());
    (core, deferred)
}

/// One run's tools: all of them, and the deferred ones loaded so far
#[derive(Default)]
struct RunTools {
    all: Vec<ToolDef>,
    loaded: BTreeSet<String>,
}

#[derive(Default)]
struct Registry {
    runs: HashMap<String, RunTools>,
    order: VecDeque<String>,
}

fn registry() -> MutexGuard<'static, Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The tools to send for this step of `run_id`, given every tool the
/// agent has: all of them up to the threshold, else the core tools plus
/// `tool_search` plus what `tool_search` loaded for this run.
pub fn step_tools(run_id: &str, tools: Vec<ToolDef>) -> Vec<ToolDef> {
    let mut registry = registry();
    if !registry.runs.contains_key(run_id) {
        registry.order.push_back(run_id.to_string());
        while registry.order.len() > MAX_TRACKED_RUNS {
            if let Some(oldest) = registry.order.pop_front() {
                registry.runs.remove(&oldest);
            }
        }
    }
    let entry = registry.runs.entry(run_id.to_string()).or_default();
    entry.all = tools.clone();
    if !should_defer(tools.len()) {
        return tools;
    }
    let (mut core, deferred) = split_core(tools);
    core.extend(
        deferred
            .into_iter()
            .filter(|tool| entry.loaded.contains(&tool.name)),
    );
    core
}

/// The tools `tool_search` loaded for `run_id`, by name
pub fn loaded_tools(run_id: &str) -> Vec<String> {
    registry()
        .runs
        .get(run_id)
        .map(|run| run.loaded.iter().cloned().collect())
        .unwrap_or_default()
}

/// Answers a `tool_search` call locally (no container, no approval):
/// searches the run's tools, loads the matches for the next step, and
/// emits `agent://tools_loaded`.
pub fn call(bus: &EventBus, ctx: &RunContext, args: &Value) -> ToolOutcome {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if query.is_empty() {
        return ToolOutcome {
            result: "tool_search needs a non-empty \"query\".".to_string(),
            is_error: true,
        };
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| usize::try_from(n).unwrap_or(MAX_LIMIT))
        .clamp(1, MAX_LIMIT);

    let matches: Vec<ToolDef> = {
        let mut registry = registry();
        let Some(run) = registry.runs.get_mut(&ctx.run_id) else {
            return ToolOutcome {
                result: "No tools are known for this run yet; search again from your next step."
                    .to_string(),
                is_error: true,
            };
        };
        let index = build_index(&run.all);
        let matches: Vec<ToolDef> = search(&index, query, limit).into_iter().cloned().collect();
        run.loaded
            .extend(matches.iter().map(|tool| tool.name.clone()));
        matches
    };

    if matches.is_empty() {
        return ToolOutcome {
            result: format!("No tools match \"{query}\". Try other keywords."),
            is_error: false,
        };
    }
    events::emit(
        bus,
        &AgentToolsLoaded {
            agent_id: ctx.agent_id.clone(),
            run_id: ctx.run_id.clone(),
            tools: matches.iter().map(|tool| tool.name.clone()).collect(),
        },
    );
    let listing: Vec<Value> = matches
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();
    let count = listing.len();
    let body = serde_json::to_string_pretty(&listing).unwrap_or_default();
    ToolOutcome {
        result: format!("Loaded {count} tool(s); you can call them from your next step:\n{body}"),
        is_error: false,
    }
}

/// `agent://tools_loaded`: `tool_search` added tools to the run
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolsLoaded {
    pub agent_id: String,
    pub run_id: String,
    pub tools: Vec<String>,
}

impl NandiEvent for AgentToolsLoaded {
    const NAME: &'static str = "agent://tools_loaded";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::{bus, ctx};

    fn tool(name: &str, description: &str) -> ToolDef {
        ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema: json!({"type": "object", "properties": {"x": {"type": "string"}}}),
        }
    }

    fn fixtures() -> Vec<ToolDef> {
        vec![
            tool("shell", "Run a shell command in the container"),
            tool("read_file", "Read a file from the workdir"),
            tool(
                "github__createIssue",
                "Create an issue in a GitHub repository",
            ),
            tool(
                "github__list_issues",
                "List the issues of a GitHub repository",
            ),
            tool("slack__post_message", "Post a message to a Slack channel"),
            tool("calendar__create_event", "Create a calendar event"),
        ]
    }

    /// `n` tools: the core ones first, then `extra_<i>`
    fn many(n: usize) -> Vec<ToolDef> {
        (0..n)
            .map(|i| match CORE_TOOLS.get(i) {
                Some(name) => tool(name, "core"),
                None => tool(&format!("extra_{i:02}"), &format!("Extra tool number {i}")),
            })
            .collect()
    }

    fn names(tools: &[&ToolDef]) -> Vec<String> {
        tools.iter().map(|t| t.name.clone()).collect()
    }

    #[test]
    fn tokenizes_snake_camel_and_punctuation() {
        assert_eq!(
            tokenize("github__createIssue: Post-it v2"),
            ["github", "create", "issue", "post", "it", "v2"]
        );
    }

    #[test]
    fn bm25_ranks_the_best_match_first() {
        let index = build_index(&fixtures());
        assert_eq!(
            names(&search(&index, "create github issue", 3)),
            [
                "github__createIssue",
                "calendar__create_event",
                "github__list_issues"
            ]
        );
        assert_eq!(
            names(&search(&index, "slack message", 5)),
            ["slack__post_message"]
        );
        // Same term frequency: the shorter description wins
        assert_eq!(
            names(&search(&index, "create", 1)),
            ["calendar__create_event"]
        );
        assert!(search(&index, "kubernetes", 5).is_empty());
        assert!(search(&index, "", 5).is_empty());
    }

    #[test]
    fn ties_break_by_name() {
        let index = build_index(&[tool("b_tool", "same words"), tool("a_tool", "same words")]);
        assert_eq!(names(&search(&index, "same", 5)), ["a_tool", "b_tool"]);
    }

    #[test]
    fn defers_above_the_threshold() {
        assert!(!should_defer(14));
        assert!(!should_defer(15));
        assert!(should_defer(16));
    }

    #[test]
    fn split_core_keeps_core_and_adds_tool_search() {
        let (core, deferred) = split_core(fixtures());
        let core: Vec<_> = core.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(core, ["shell", "read_file", TOOL_SEARCH_NAME]);
        assert_eq!(deferred.len(), 4);
        assert!(deferred.iter().all(|t| t.name != TOOL_SEARCH_NAME));
        let spec = tool_search_spec();
        assert_eq!(spec.input_schema["properties"]["query"]["type"], "string");
        assert_eq!(spec.input_schema["properties"]["limit"]["type"], "integer");
    }

    #[test]
    fn pilot_tools_are_never_deferred() {
        let mut tools = many(20);
        tools.push(tool("pilot_browser__browser_navigate", "Open a page"));
        tools.push(tool("pilot_browser__browser_hand_over", "Hand over"));
        let offered = step_tools("run-pilot", tools);
        let offered: Vec<_> = offered.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            offered,
            [
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                "pilot_browser__browser_navigate",
                "pilot_browser__browser_hand_over",
                TOOL_SEARCH_NAME
            ]
        );
    }

    #[test]
    fn small_agents_get_every_tool() {
        let tools = step_tools("run-small", many(15));
        assert_eq!(tools.len(), 15);
        assert!(tools.iter().all(|t| t.name != TOOL_SEARCH_NAME));
    }

    #[tokio::test]
    async fn tool_search_loads_matches_into_the_next_step() {
        let (bus, sink) = bus();
        let ctx = ctx("run-defer");
        let first = step_tools(&ctx.run_id, many(20));
        let first: Vec<_> = first.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            first,
            [
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                TOOL_SEARCH_NAME
            ]
        );

        let outcome = call(&bus, &ctx, &json!({"query": "number 7", "limit": 1}));
        assert!(!outcome.is_error, "{}", outcome.result);
        assert!(outcome.result.contains("\"extra_07\""));
        assert!(outcome.result.contains("input_schema"));
        assert!(outcome.result.contains("Extra tool number 7"));
        assert_eq!(loaded_tools(&ctx.run_id), ["extra_07"]);

        let events = sink.take();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "agent://tools_loaded");
        assert_eq!(
            events[0].1,
            json!({"agentId": "writer", "runId": "run-defer", "tools": ["extra_07"]})
        );

        let next = step_tools(&ctx.run_id, many(20));
        assert_eq!(next.len(), 6);
        assert_eq!(next.last().map(|t| t.name.as_str()), Some("extra_07"));
    }

    #[tokio::test]
    async fn tool_search_without_a_match_loads_nothing() {
        let (bus, sink) = bus();
        let ctx = ctx("run-nomatch");
        step_tools(&ctx.run_id, many(20));
        let none = call(&bus, &ctx, &json!({"query": "kubernetes"}));
        assert!(!none.is_error);
        assert!(none.result.contains("No tools match"));
        assert!(call(&bus, &ctx, &json!({})).is_error);
        let unknown = crate::services::events::tests::ctx("run-unknown");
        assert!(call(&bus, &unknown, &json!({"query": "extra"})).is_error);
        assert!(loaded_tools(&ctx.run_id).is_empty());
        assert!(sink.take().is_empty());
    }
}
