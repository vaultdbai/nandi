//! Turn middleware: policies that wrap each step of a run.
//!
//! The built-in middlewares live here, and
//! the pipeline in `orchestrator.rs`: every `pre_turn` in order before the
//! engine step, every `post_turn` in reverse after it. Other middlewares
//! live in their own modules; the list below registers them.
//!
//! "Turn" here is one engine step of a run (an LLM call and its tool calls);
//! `TurnContext::turn_number` is the step index.

use std::collections::BTreeMap;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::compaction::{CompactionMiddleware, CompactionPolicy};
use super::run_plan::{FinishCheckMiddleware, PlanFirstMiddleware};
use super::verify_on_stop::VerifyOnStopMiddleware;
use super::RunDeps;
use crate::services::agent_memory::MemoryMiddleware;
use crate::services::events::NandiEvent;
use crate::services::run_plan::{classify::Effect, RunPlan};

pub type MiddlewareError = String;

/// skill.md `max_turns` default
pub const DEFAULT_MAX_TURNS: u32 = 50;
/// skill.md `max_cost_usd` default
pub const DEFAULT_MAX_COST_USD: f64 = 5.0;

/// skill.md `middlewares` names
pub const VERIFY_ON_STOP: &str = "verify_on_stop";
pub const COMPACTION: &str = "compaction";
pub const READ_ONLY: &str = "read_only";
/// The run plan's rules (`run_plan.rs`)
pub const PLAN_FIRST: &str = "plan_first";
pub const FINISH_CHECK: &str = "finish_check";

/// What a `pre_turn` lets happen next
#[derive(Debug, Clone, PartialEq)]
pub enum MiddlewareDecision {
    Continue,
    /// Block for human approval (the reason is shown); resume when granted
    #[allow(dead_code)]
    // The approval path is wired and tested; no shipped middleware pauses yet
    PauseForApproval(String),
    /// Stop the run cleanly with this message (`agent://turn_aborted`)
    Abort(String),
}

/// A tool result the middlewares can inspect (verify-on-stop's evidence)
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolResultSummary {
    /// The step it ran in
    pub turn: u32,
    pub tool: String,
    pub args: serde_json::Value,
    pub is_error: bool,
}

/// State shared by the middlewares across the steps of one run
#[derive(Debug, Clone, Default)]
pub struct TurnContext {
    pub agent_id: String,
    /// A `pre_turn` (compaction) may replace it with a child run: the loop
    /// then continues in that run
    pub run_id: String,
    pub turn_number: u32,
    pub accumulated_cost: f64,
    /// chars / 4 over the run's messages
    pub token_estimate: u32,
    /// The model's context window, in tokens
    pub context_window: u32,
    /// Most recent last
    pub last_tool_results: Vec<ToolResultSummary>,
    /// Paths written this run, in order
    pub files_touched: Vec<String>,
    /// Extra system prompt sections for the next LLM call (e.g. memory)
    pub system_extras: Vec<String>,
    /// Set by verify-on-stop once it has fired this run
    pub verify_fired: bool,
    /// The finish check's continuation is in the run already (rebuilt from
    /// its messages, so it holds across a suspension)
    pub finish_check_fired: bool,
    pub policy: TurnPolicy,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ExitReason {
    /// The step planned tool calls: the run goes on
    #[default]
    ToolCalls,
    /// A text-only step: the run would end
    Completed,
}

/// What one step produced
#[derive(Debug, Clone, Default)]
pub struct TurnResult {
    pub final_message: Option<String>,
    pub tool_calls_made: Vec<String>,
    pub files_modified: Vec<String>,
    pub exit_reason: ExitReason,
    /// Set by a `post_turn` to keep the run going: appended as a system
    /// message, and the loop takes another step instead of ending
    pub continue_with: Option<String>,
}

impl TurnResult {}

pub trait TurnMiddleware: Send + Sync {
    fn name(&self) -> &'static str;

    fn pre_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>>;

    fn post_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
        result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>>;

    /// Plan mode: whether a tool call may run at all
    fn allows_tool(&self, _tool: &str) -> bool {
        true
    }

    /// The run plan's rules: why this call may not run now, if it may not
    fn blocks_call(&self, _call: &CallCheck<'_>) -> Option<String> {
        None
    }
}

/// One tool call about to run, with what it does and the run's plan
#[derive(Debug, Clone, Copy)]
pub struct CallCheck<'a> {
    pub tool: &'a str,
    pub effect: Effect,
    /// The browser's screenshot tool
    pub screenshot: bool,
    pub plan: Option<&'a RunPlan>,
}

/// The turn policy declared in skill.md (unknown frontmatter keys, read
/// from `Agent::extra`):
///
/// ```yaml
/// middlewares: [verify_on_stop, compaction, plan_first, finish_check]   # [] disables them
/// max_turns: 50
/// max_cost_usd: 5.0
/// compaction: { trigger_tokens: null, preserve_recent_percent: 30, iterative_merge: true }
/// memory_disabled: false
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TurnPolicy {
    /// `None`: not declared (the defaults: verify_on_stop, compaction,
    /// plan_first and finish_check)
    pub middlewares: Option<Vec<String>>,
    pub max_turns: u32,
    pub max_cost_usd: f64,
    pub compaction: CompactionPolicy,
    pub memory_disabled: bool,
}

impl Default for TurnPolicy {
    fn default() -> Self {
        Self {
            middlewares: None,
            max_turns: DEFAULT_MAX_TURNS,
            max_cost_usd: DEFAULT_MAX_COST_USD,
            compaction: CompactionPolicy::default(),
            memory_disabled: false,
        }
    }
}

impl TurnPolicy {
    /// From an agent's unknown frontmatter keys (snake_case, as written in
    /// skill.md). A missing key takes its default; an invalid one also
    /// does, with a warning in the log.
    pub fn from_extra(extra: &BTreeMap<String, serde_yaml::Value>) -> Self {
        let mut policy = Self::default();
        if let Some(value) = extra.get("middlewares") {
            policy.middlewares = parse_middlewares(value);
        }
        if let Some(value) = extra.get("max_turns") {
            match value.as_u64().and_then(|n| u32::try_from(n).ok()) {
                Some(n) if n > 0 => policy.max_turns = n,
                _ => invalid("max_turns", value, DEFAULT_MAX_TURNS),
            }
        }
        if let Some(value) = extra.get("max_cost_usd") {
            match value.as_f64() {
                Some(cost) if cost.is_finite() && cost >= 0.0 => policy.max_cost_usd = cost,
                _ => invalid("max_cost_usd", value, DEFAULT_MAX_COST_USD),
            }
        }
        if let Some(value) = extra.get("compaction") {
            policy.compaction = parse_compaction(value);
        }
        if let Some(value) = extra.get("memory_disabled") {
            match value.as_bool() {
                Some(disabled) => policy.memory_disabled = disabled,
                None => invalid("memory_disabled", value, false),
            }
        }
        policy
    }

    /// Whether skill.md enables the optional middleware `name`
    pub fn enables(&self, name: &str) -> bool {
        match &self.middlewares {
            Some(list) => list.iter().any(|m| m == name),
            None => DEFAULT_MIDDLEWARES.contains(&name),
        }
    }
}

fn invalid(key: &str, value: &serde_yaml::Value, default: impl std::fmt::Debug) {
    log::warn!("skill.md `{key}: {value:?}` is invalid; using the default ({default:?})");
}

/// `middlewares:` must be a list of names; anything else means "not
/// declared" (the defaults)
fn parse_middlewares(value: &serde_yaml::Value) -> Option<Vec<String>> {
    let Some(items) = value.as_sequence() else {
        invalid("middlewares", value, DEFAULT_MIDDLEWARES.join(", "));
        return None;
    };
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        match item.as_str() {
            Some(name) => {
                if !OPTIONAL_MIDDLEWARES.contains(&name) {
                    log::warn!("skill.md `middlewares` names an unknown middleware '{name}'");
                }
                names.push(name.to_string());
            }
            None => log::warn!("skill.md `middlewares` entry {item:?} isn't a name; ignored"),
        }
    }
    Some(names)
}

/// `compaction: { trigger_tokens, preserve_recent_percent, iterative_merge }`,
/// each key falling back to its default on its own
fn parse_compaction(value: &serde_yaml::Value) -> CompactionPolicy {
    let mut policy = CompactionPolicy::default();
    let Some(map) = value.as_mapping() else {
        invalid("compaction", value, &policy);
        return policy;
    };
    if let Some(v) = map.get("trigger_tokens") {
        match v {
            serde_yaml::Value::Null => policy.trigger_tokens = None,
            _ => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
                Some(n) if n > 0 => policy.trigger_tokens = Some(n),
                _ => invalid("compaction.trigger_tokens", v, policy.trigger_tokens),
            },
        }
    }
    if let Some(v) = map.get("preserve_recent_percent") {
        match v.as_u64().and_then(|n| u8::try_from(n).ok()) {
            Some(n) if n <= 100 => policy.preserve_recent_percent = n,
            _ => invalid(
                "compaction.preserve_recent_percent",
                v,
                policy.preserve_recent_percent,
            ),
        }
    }
    if let Some(v) = map.get("iterative_merge") {
        match v.as_bool() {
            Some(merge) => policy.iterative_merge = merge,
            None => invalid("compaction.iterative_merge", v, policy.iterative_merge),
        }
    }
    policy
}

/// The names skill.md `middlewares:` may list
pub const OPTIONAL_MIDDLEWARES: [&str; 5] = [
    VERIFY_ON_STOP,
    COMPACTION,
    READ_ONLY,
    PLAN_FIRST,
    FINISH_CHECK,
];

/// The optional middlewares an agent gets when skill.md doesn't declare any
pub const DEFAULT_MIDDLEWARES: [&str; 4] = [VERIFY_ON_STOP, COMPACTION, PLAN_FIRST, FINISH_CHECK];

/// The middlewares that hold for every run, Studio included: limits that
/// need no agent definition
pub const BUILTIN_MIDDLEWARES: [&str; 3] = [TURN_LIMIT, COST_CAP, TOKEN_CAP];

pub const TURN_LIMIT: &str = "turn_limit";
pub const COST_CAP: &str = "cost_cap";
pub const TOKEN_CAP: &str = "token_cap";

/// Context window assumed for models Nandi doesn't know better (tokens)
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;
/// DeepSeek's chat models (the default provider)
pub const DEEPSEEK_CONTEXT_WINDOW: u32 = 128_000;

/// The context window of `model` (`provider:name`), for the token cap and
/// compaction's trigger
pub fn context_window_for(model: &str) -> u32 {
    if model.to_ascii_lowercase().starts_with("deepseek") {
        DEEPSEEK_CONTEXT_WINDOW
    } else {
        DEFAULT_CONTEXT_WINDOW
    }
}

/// chars / 4, the estimate `TurnContext::token_estimate` uses
pub fn estimate_tokens(chars: usize) -> u32 {
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

/// Tools that change files or run arbitrary commands: what plan mode
/// blocks. Matched on the name after a connector prefix (`fs__write_file`).
pub const MUTATING_TOOLS: [&str; 13] = [
    "write_file",
    "edit_file",
    "create_file",
    "append_file",
    "delete_file",
    "remove_file",
    "move_file",
    "rename_file",
    "copy_file",
    "make_dir",
    "apply_patch",
    "shell",
    "run_command",
];

/// Tools that write the file at their `path` argument
const WRITE_TOOLS: [&str; 5] = [
    "write_file",
    "edit_file",
    "create_file",
    "append_file",
    "apply_patch",
];

/// A tool's name without its connector prefix (`<connector>__<tool>`)
fn base_tool_name(tool: &str) -> &str {
    tool.rsplit_once("__").map_or(tool, |(_, base)| base)
}

pub fn is_mutating_tool(tool: &str) -> bool {
    MUTATING_TOOLS.contains(&base_tool_name(tool))
}

/// The path a write-like tool call writes, from its `path` (or `file_path`)
/// argument
pub fn written_path(tool: &str, args: &serde_json::Value) -> Option<String> {
    if !WRITE_TOOLS.contains(&base_tool_name(tool)) {
        return None;
    }
    ["path", "file_path"]
        .iter()
        .find_map(|key| args.get(key).and_then(|v| v.as_str()))
        .map(str::to_string)
}

/// A `pre_turn` that didn't continue: which middleware, and what it decided
#[derive(Debug, Clone, PartialEq)]
pub struct PreTurnStop {
    pub middleware: &'static str,
    pub decision: MiddlewareDecision,
}

/// Runs every `pre_turn` in order, stopping at the first that doesn't
/// continue. A middleware error is logged and treated as `Continue`, so a
/// broken optional middleware never takes a run down. `approved` names
/// the middlewares whose pause this step was already approved: their
/// `PauseForApproval` counts as `Continue`.
pub async fn run_pre_turns(
    pipeline: &[Box<dyn TurnMiddleware>],
    ctx: &mut TurnContext,
    approved: &[&str],
) -> Option<PreTurnStop> {
    for middleware in pipeline {
        let name = middleware.name();
        match middleware.pre_turn(ctx).await {
            Ok(MiddlewareDecision::Continue) => {}
            Ok(MiddlewareDecision::PauseForApproval(_)) if approved.contains(&name) => {}
            Ok(decision) => {
                return Some(PreTurnStop {
                    middleware: name,
                    decision,
                })
            }
            Err(e) => log::warn!("Middleware '{name}' pre_turn failed: {e}"),
        }
    }
    None
}

/// Runs every `post_turn` in reverse order (the onion unwinds). Errors are
/// logged and skipped, as in `run_pre_turns`.
pub async fn run_post_turns(
    pipeline: &[Box<dyn TurnMiddleware>],
    ctx: &mut TurnContext,
    result: &mut TurnResult,
) {
    for middleware in pipeline.iter().rev() {
        if let Err(e) = middleware.post_turn(ctx, result).await {
            log::warn!("Middleware '{}' post_turn failed: {e}", middleware.name());
        }
    }
}

/// What the model is told when a middleware blocks one of its tool calls
pub fn blocked_tool_result(middleware: &str, tool: &str) -> String {
    if middleware == READ_ONLY {
        format!(
            "Blocked by plan mode (read-only): '{tool}' can change files or run commands, so it was not run. Describe the change instead."
        )
    } else {
        format!("Blocked by the '{middleware}' middleware: '{tool}' was not run.")
    }
}

/// The first middleware whose run plan rules block `call`, and why
pub fn blocking_call(
    pipeline: &[Box<dyn TurnMiddleware>],
    call: &CallCheck<'_>,
) -> Option<(&'static str, String)> {
    pipeline
        .iter()
        .find_map(|m| m.blocks_call(call).map(|why| (m.name(), why)))
}

/// The first middleware that blocks `tool`, if any
pub fn blocking_middleware(
    pipeline: &[Box<dyn TurnMiddleware>],
    tool: &str,
) -> Option<&'static str> {
    pipeline
        .iter()
        .find(|m| !m.allows_tool(tool))
        .map(|m| m.name())
}

/// `agent://turn_aborted`: a middleware stopped the run
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnAborted {
    pub agent_id: String,
    pub run_id: String,
    pub middleware: String,
    pub reason: String,
}

impl NandiEvent for AgentTurnAborted {
    const NAME: &'static str = "agent://turn_aborted";
}

/// Aborts once `turn_number` passes `max_turns`
#[derive(Debug, Default)]
pub struct TurnLimitMiddleware {
    pub max_turns: u32,
}

/// Aborts once `accumulated_cost` passes `max_cost_usd`
#[derive(Debug, Default)]
pub struct CostCapMiddleware {
    pub max_cost_usd: f64,
}

/// Aborts once the context can't fit the next step even after compaction
/// (`token_estimate > context_window`)
#[derive(Debug, Default)]
pub struct TokenCapMiddleware {}

/// Plan mode: rejects tool calls that mutate files
#[derive(Debug, Default)]
pub struct ReadOnlyMiddleware {}

fn no_post_turn<'a>() -> BoxFuture<'a, Result<(), MiddlewareError>> {
    Box::pin(async { Ok(()) })
}

fn decide<'a>(
    decision: MiddlewareDecision,
) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
    Box::pin(async move { Ok(decision) })
}

impl TurnMiddleware for TurnLimitMiddleware {
    fn name(&self) -> &'static str {
        TURN_LIMIT
    }

    /// `turn_number` is the 0-based index of the step about to run, so a
    /// run takes at most `max_turns` steps
    fn pre_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        decide(if ctx.turn_number >= self.max_turns {
            let max = self.max_turns;
            MiddlewareDecision::Abort(format!(
                "Stopped: this run reached its limit of {max} turns (skill.md `max_turns`)."
            ))
        } else {
            MiddlewareDecision::Continue
        })
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        no_post_turn()
    }
}

impl TurnMiddleware for CostCapMiddleware {
    fn name(&self) -> &'static str {
        COST_CAP
    }

    fn pre_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        decide(if ctx.accumulated_cost > self.max_cost_usd {
            let (cost, max) = (ctx.accumulated_cost, self.max_cost_usd);
            MiddlewareDecision::Abort(format!(
                "Stopped: this run has cost ${cost:.2}, over its limit of ${max:.2} (skill.md `max_cost_usd`)."
            ))
        } else {
            MiddlewareDecision::Continue
        })
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        no_post_turn()
    }
}

impl TurnMiddleware for TokenCapMiddleware {
    fn name(&self) -> &'static str {
        TOKEN_CAP
    }

    /// A hard stop: compaction normally starts a child run well before
    fn pre_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        decide(
            if ctx.context_window > 0 && ctx.token_estimate > ctx.context_window {
                let (tokens, window) = (ctx.token_estimate, ctx.context_window);
                MiddlewareDecision::Abort(format!(
                    "Stopped: this run's context (about {tokens} tokens) no longer fits the model's {window}-token window."
                ))
            } else {
                MiddlewareDecision::Continue
            },
        )
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        no_post_turn()
    }
}

impl TurnMiddleware for ReadOnlyMiddleware {
    fn name(&self) -> &'static str {
        READ_ONLY
    }

    fn pre_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        decide(MiddlewareDecision::Continue)
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        no_post_turn()
    }

    fn allows_tool(&self, tool: &str) -> bool {
        !is_mutating_tool(tool)
    }
}

/// The pipeline for one run, in `pre_turn` order: the built-in limits, then
/// memory, compaction, the finish check and verify-on-stop
/// when enabled, plan first when enabled, then plan mode when
/// declared. The finish check comes before verify-on-stop so its
/// `post_turn` runs after it and joins their continuations.
pub fn register_default_middlewares(
    deps: &RunDeps,
    policy: &TurnPolicy,
) -> Vec<Box<dyn TurnMiddleware>> {
    let mut list: Vec<Box<dyn TurnMiddleware>> = vec![
        Box::new(TurnLimitMiddleware {
            max_turns: policy.max_turns,
        }),
        Box::new(CostCapMiddleware {
            max_cost_usd: policy.max_cost_usd,
        }),
        Box::new(TokenCapMiddleware::default()),
    ];
    if !policy.memory_disabled {
        list.push(Box::new(MemoryMiddleware::new(deps.clone())));
    }
    if policy.enables(COMPACTION) {
        list.push(Box::new(CompactionMiddleware::new(
            deps.clone(),
            policy.compaction.clone(),
        )));
    }
    if policy.enables(FINISH_CHECK) {
        list.push(Box::new(FinishCheckMiddleware::new(deps.clone())));
    }
    if policy.enables(VERIFY_ON_STOP) {
        list.push(Box::new(VerifyOnStopMiddleware::default()));
    }
    if policy.enables(PLAN_FIRST) {
        list.push(Box::new(PlanFirstMiddleware::default()));
    }
    if policy.enables(READ_ONLY) {
        list.push(Box::new(ReadOnlyMiddleware::default()));
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn extra(yaml: &str) -> BTreeMap<String, serde_yaml::Value> {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn from_extra_defaults_when_nothing_is_declared() {
        assert_eq!(
            TurnPolicy::from_extra(&BTreeMap::new()),
            TurnPolicy::default()
        );
        let policy = TurnPolicy::default();
        assert!(policy.enables(VERIFY_ON_STOP) && policy.enables(COMPACTION));
        assert!(policy.enables(PLAN_FIRST) && policy.enables(FINISH_CHECK));
        assert!(!policy.enables(READ_ONLY));
    }

    #[test]
    fn from_extra_parses_every_key() {
        let policy = TurnPolicy::from_extra(&extra(
            "middlewares: [read_only, compaction]\n\
             max_turns: 12\n\
             max_cost_usd: 1.5\n\
             compaction: { trigger_tokens: 90000, preserve_recent_percent: 40, iterative_merge: false }\n\
             memory_disabled: true\n\
             unrelated: 1\n",
        ));
        assert_eq!(
            policy,
            TurnPolicy {
                middlewares: Some(vec![READ_ONLY.into(), COMPACTION.into()]),
                max_turns: 12,
                max_cost_usd: 1.5,
                compaction: CompactionPolicy {
                    trigger_tokens: Some(90_000),
                    preserve_recent_percent: 40,
                    iterative_merge: false,
                },
                memory_disabled: true,
            }
        );
        assert!(policy.enables(READ_ONLY) && policy.enables(COMPACTION));
        assert!(!policy.enables(VERIFY_ON_STOP));
        // An integer cost is fine too, and a null trigger means "derive it"
        let policy = TurnPolicy::from_extra(&extra(
            "max_cost_usd: 2\ncompaction: { trigger_tokens: null }\n",
        ));
        assert_eq!(policy.max_cost_usd, 2.0);
        assert_eq!(policy.compaction, CompactionPolicy::default());
    }

    #[test]
    fn an_empty_middlewares_list_disables_the_optional_ones() {
        let policy = TurnPolicy::from_extra(&extra("middlewares: []\n"));
        assert_eq!(policy.middlewares, Some(vec![]));
        for name in OPTIONAL_MIDDLEWARES {
            assert!(!policy.enables(name), "{name}");
        }
    }

    #[test]
    fn invalid_values_fall_back_to_the_defaults() {
        let policy = TurnPolicy::from_extra(&extra(
            "middlewares: read_only\n\
             max_turns: -3\n\
             max_cost_usd: lots\n\
             compaction: { trigger_tokens: big, preserve_recent_percent: 400, iterative_merge: maybe }\n\
             memory_disabled: sometimes\n",
        ));
        assert_eq!(policy, TurnPolicy::default());
        assert_eq!(
            TurnPolicy::from_extra(&extra("max_turns: 0\ncompaction: 5\n")),
            TurnPolicy::default()
        );
        // Non-name entries are dropped, names kept
        let policy = TurnPolicy::from_extra(&extra("middlewares: [read_only, 3]\n"));
        assert_eq!(policy.middlewares, Some(vec![READ_ONLY.to_string()]));
    }

    fn ctx() -> TurnContext {
        TurnContext {
            context_window: 1_000,
            ..TurnContext::default()
        }
    }

    async fn pre(middleware: &dyn TurnMiddleware, ctx: &mut TurnContext) -> MiddlewareDecision {
        middleware.pre_turn(ctx).await.unwrap()
    }

    #[tokio::test]
    async fn the_turn_limit_fires_at_its_threshold() {
        let limit = TurnLimitMiddleware { max_turns: 3 };
        let mut ctx = ctx();
        for turn in 0..3 {
            ctx.turn_number = turn;
            assert_eq!(pre(&limit, &mut ctx).await, MiddlewareDecision::Continue);
        }
        ctx.turn_number = 3;
        let MiddlewareDecision::Abort(reason) = pre(&limit, &mut ctx).await else {
            panic!("expected an abort");
        };
        assert!(reason.contains("limit of 3 turns"), "{reason}");
    }

    #[tokio::test]
    async fn the_cost_cap_fires_once_the_cost_passes_it() {
        let cap = CostCapMiddleware { max_cost_usd: 1.0 };
        let mut ctx = ctx();
        ctx.accumulated_cost = 1.0;
        assert_eq!(pre(&cap, &mut ctx).await, MiddlewareDecision::Continue);
        ctx.accumulated_cost = 1.01;
        let MiddlewareDecision::Abort(reason) = pre(&cap, &mut ctx).await else {
            panic!("expected an abort");
        };
        assert!(
            reason.contains("$1.01") && reason.contains("$1.00"),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn the_token_cap_fires_past_the_context_window() {
        let cap = TokenCapMiddleware::default();
        let mut ctx = ctx();
        ctx.token_estimate = 1_000;
        assert_eq!(pre(&cap, &mut ctx).await, MiddlewareDecision::Continue);
        ctx.token_estimate = 1_001;
        assert!(matches!(
            pre(&cap, &mut ctx).await,
            MiddlewareDecision::Abort(_)
        ));
        // An unknown window never stops a run
        ctx.context_window = 0;
        assert_eq!(pre(&cap, &mut ctx).await, MiddlewareDecision::Continue);
    }

    #[test]
    fn read_only_allows_reads_and_blocks_mutations() {
        let plan = ReadOnlyMiddleware::default();
        for tool in ["read_file", "list_dir", "search", "filesystem__read_file"] {
            assert!(plan.allows_tool(tool), "{tool}");
        }
        for tool in [
            "write_file",
            "edit_file",
            "shell",
            "delete_file",
            "filesystem__write_file",
        ] {
            assert!(!plan.allows_tool(tool), "{tool}");
        }
        assert!(blocked_tool_result(READ_ONLY, "shell").contains("plan mode (read-only)"));
    }

    #[test]
    fn written_paths_come_from_write_tools_only() {
        let args = serde_json::json!({ "path": "notes.md" });
        assert_eq!(
            written_path("write_file", &args).as_deref(),
            Some("notes.md")
        );
        assert_eq!(
            written_path("fs__edit_file", &args).as_deref(),
            Some("notes.md")
        );
        assert_eq!(written_path("read_file", &args), None);
        assert_eq!(written_path("write_file", &serde_json::json!({})), None);
        assert_eq!(estimate_tokens(9), 3);
        assert_eq!(
            context_window_for("deepseek:deepseek-flash"),
            DEEPSEEK_CONTEXT_WINDOW
        );
        assert_eq!(context_window_for("other:model"), DEFAULT_CONTEXT_WINDOW);
    }

    /// Records each call as "pre:<name>" / "post:<name>"
    struct Recorder {
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
        decision: MiddlewareDecision,
    }

    impl TurnMiddleware for Recorder {
        fn name(&self) -> &'static str {
            self.name
        }

        fn pre_turn<'a>(
            &'a self,
            _ctx: &'a mut TurnContext,
        ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("pre:{}", self.name));
            let decision = self.decision.clone();
            Box::pin(async move { Ok(decision) })
        }

        fn post_turn<'a>(
            &'a self,
            _ctx: &'a mut TurnContext,
            _result: &'a mut TurnResult,
        ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("post:{}", self.name));
            Box::pin(async { Ok(()) })
        }
    }

    type Calls = Arc<Mutex<Vec<String>>>;

    fn recorders(
        decisions: &[(&'static str, MiddlewareDecision)],
    ) -> (Vec<Box<dyn TurnMiddleware>>, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let pipeline = decisions
            .iter()
            .map(|(name, decision)| {
                Box::new(Recorder {
                    name,
                    calls: calls.clone(),
                    decision: decision.clone(),
                }) as Box<dyn TurnMiddleware>
            })
            .collect();
        (pipeline, calls)
    }

    #[tokio::test]
    async fn pre_turns_run_in_order_and_post_turns_in_reverse() {
        let (pipeline, calls) = recorders(&[
            ("a", MiddlewareDecision::Continue),
            ("b", MiddlewareDecision::Continue),
            ("c", MiddlewareDecision::Continue),
        ]);
        let mut ctx = ctx();
        assert_eq!(run_pre_turns(&pipeline, &mut ctx, &[]).await, None);
        let mut result = TurnResult::default();
        run_post_turns(&pipeline, &mut ctx, &mut result).await;
        assert_eq!(
            *calls.lock().unwrap(),
            ["pre:a", "pre:b", "pre:c", "post:c", "post:b", "post:a"]
        );
    }

    #[tokio::test]
    async fn the_first_stopping_pre_turn_wins() {
        let (pipeline, calls) = recorders(&[
            ("a", MiddlewareDecision::PauseForApproval("ok?".into())),
            ("b", MiddlewareDecision::Abort("no".into())),
            ("c", MiddlewareDecision::Continue),
        ]);
        let mut ctx = ctx();
        let stop = run_pre_turns(&pipeline, &mut ctx, &[]).await.unwrap();
        assert_eq!(stop.middleware, "a");
        assert_eq!(
            stop.decision,
            MiddlewareDecision::PauseForApproval("ok?".into())
        );
        // Once a's pause is approved, b's abort stops the step
        let stop = run_pre_turns(&pipeline, &mut ctx, &["a"]).await.unwrap();
        assert_eq!(stop.middleware, "b");
        assert_eq!(*calls.lock().unwrap(), ["pre:a", "pre:a", "pre:b"]);
    }
}
