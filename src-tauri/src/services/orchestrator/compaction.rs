//! Threshold compaction.
//!
//! When a run's token estimate nears the context window, the run ends as
//! `compacted` and a child run continues it from a summary (merged with the
//! parent's summary when `iterative_merge`) plus the most recent turns.
//! The child's `runs.parent_run_id` and `runs.compaction_summary` link it.
//!
//! How a compaction goes (`compact_using`):
//!
//! 1. The run's messages split into the turns to drop and the most recent
//!    `preserve_recent_percent` of them to keep. The split never falls
//!    inside a step: an assistant message stays with its tool calls.
//! 2. One model call (the agent's model, `SUMMARY_MAX_TOKENS`) turns the
//!    previous summary (the one this run was itself started from, when
//!    `iterative_merge`) plus the dropped turns into an updated summary.
//! 3. In one transaction the run ends as `compacted` and the child starts
//!    (`running`; same agent, conversation, input, source, and priority)
//!    with the summary as a synthetic assistant message followed by copies
//!    of the kept messages. Copies are tagged (`db::COMPACTION_COPY_KEY`)
//!    so the conversation doesn't show them twice; tool call IDs are
//!    remapped consistently so the child's step history stays intact.
//! 4. `agent://run` for both runs, `agent://queue_changed`, and
//!    `agent://run_compacted` are emitted.
//!
//! The middleware never kills a run: if compaction fails it logs and the
//! run goes on uncompacted.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::middleware::{
    MiddlewareDecision, MiddlewareError, TurnContext, TurnMiddleware, TurnResult,
};
use super::RunDeps;
use crate::db::{self, role, AssistantMeta, MessageRow, ToolCallMeta};
use crate::llm::{self, CallSource, LlmRequest, Message};
use crate::model::{RunStatus, RunSummary, STUDIO_AGENT_ID};
use crate::services::events::{self, AgentQueueChanged, AgentRunUpdated, NandiEvent};
use crate::services::llm_keys;
use crate::services::secrets::Secrets;

/// Tokens kept free below the context window when `trigger_tokens` is unset
pub const DEFAULT_BUFFER_TOKENS: u32 = 13_000;
pub const DEFAULT_PRESERVE_RECENT_PERCENT: u8 = 30;
/// Output budget for the summary call
pub const SUMMARY_MAX_TOKENS: u32 = 500;

/// Per-message cap in the summary prompt, so one huge tool result can't
/// crowd out the rest
const MAX_TRANSCRIPT_MESSAGE_CHARS: usize = 2_000;
/// Overall cap on the dropped turns sent to the model (the most recent
/// are kept when it's exceeded)
const MAX_TRANSCRIPT_CHARS: usize = 60_000;

const SUMMARY_SYSTEM: &str = "You maintain the working summary of a long-running agent task. \
The agent's earlier turns are being removed from its context; your summary replaces them. \
Be concrete: keep file paths, commands, identifiers, decisions, and open problems. Reply with the summary only.";

/// skill.md `compaction:`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct CompactionPolicy {
    /// `None`: the context window minus `DEFAULT_BUFFER_TOKENS`
    pub trigger_tokens: Option<u32>,
    pub preserve_recent_percent: u8,
    pub iterative_merge: bool,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            trigger_tokens: None,
            preserve_recent_percent: DEFAULT_PRESERVE_RECENT_PERCENT,
            iterative_merge: true,
        }
    }
}

impl CompactionPolicy {
    /// The token estimate at which a run compacts, or `None` when it can't
    /// be known (no `trigger_tokens` and an unknown context window).
    ///
    /// Without `trigger_tokens` it's the window minus
    /// `DEFAULT_BUFFER_TOKENS`, but never below half the window, so a small
    /// window doesn't compact on every step.
    pub fn threshold(&self, context_window: u32) -> Option<u32> {
        match self.trigger_tokens {
            Some(tokens) => Some(tokens),
            None if context_window == 0 => None,
            None => Some(
                context_window
                    .saturating_sub(DEFAULT_BUFFER_TOKENS)
                    .max(context_window / 2),
            ),
        }
    }

    /// Whether a step with this context should compact first
    pub fn should_compact(&self, token_estimate: u32, context_window: u32) -> bool {
        self.threshold(context_window)
            .is_some_and(|threshold| token_estimate >= threshold)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    pub new_run_id: String,
    pub summary: String,
    pub parent_run_id: String,
}

/// `agent://run_compacted`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunCompacted {
    pub agent_id: String,
    /// The run that ended as `compacted`
    pub run_id: String,
    /// The child run continuing it
    pub new_run_id: String,
}

impl NandiEvent for AgentRunCompacted {
    const NAME: &'static str = "agent://run_compacted";
}

/// One summary model call
pub struct SummaryRequest<'a> {
    pub agent_id: &'a str,
    /// `<provider>:<model>`
    pub model: &'a str,
    pub system: &'a str,
    pub prompt: &'a str,
    pub max_tokens: u32,
}

/// The summary call, injectable so tests never reach the network.
pub trait Summarizer: Send + Sync {
    fn summarize<'a>(
        &'a self,
        request: SummaryRequest<'a>,
    ) -> BoxFuture<'a, Result<String, String>>;
}

/// The real model: `llm::complete` with the key from the OS keychain.
pub struct LlmSummarizer {
    secrets: Secrets,
}

impl LlmSummarizer {
    pub fn new(secrets: Secrets) -> Self {
        Self { secrets }
    }
}

impl Summarizer for LlmSummarizer {
    fn summarize<'a>(
        &'a self,
        request: SummaryRequest<'a>,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            llm_keys::usable_model(&self.secrets, request.model)?;
            let llm_request = LlmRequest {
                model: request.model.to_string(),
                system: Some(request.system.to_string()),
                messages: vec![Message::User {
                    content: request.prompt.to_string(),
                }],
                tools: Vec::new(),
                max_tokens: Some(request.max_tokens),
            };
            let response = llm::complete(
                &self.secrets,
                &llm_request,
                Some(request.agent_id),
                CallSource::Orchestrator,
            )
            .await?;
            Ok(response.content)
        })
    }
}

/// `compact_with`, with the summary call injected.
pub async fn compact_using(
    deps: &RunDeps,
    run_id: &str,
    policy: &CompactionPolicy,
    summarizer: &dyn Summarizer,
) -> Result<CompactionResult, String> {
    let id = run_id.to_string();
    let (run, messages) = deps
        .bus
        .db
        .call(move |conn| Ok((db::get_run(conn, &id)?, db::list_run_messages(conn, &id)?)))
        .await?;
    let run = run.ok_or_else(|| format!("Run {run_id} not found"))?;
    if run.status != RunStatus::Running {
        return Err(format!(
            "Run {run_id} is {}, not running: it can't be compacted",
            run.status.as_str()
        ));
    }

    // The synthetic summary is carried by `compaction_summary`, not replayed
    let body: Vec<&MessageRow> = messages
        .iter()
        .filter(|m| m.metadata.get(db::COMPACTION_SUMMARY_KEY).is_none())
        .collect();
    let split = split_point(&body, policy.preserve_recent_percent);
    let (dropped, kept) = body.split_at(split);
    if dropped.is_empty() {
        return Err(format!("Run {run_id} has no turns old enough to compact"));
    }

    let previous = if policy.iterative_merge {
        run.compaction_summary.as_deref()
    } else {
        None
    };
    let prompt = summary_prompt(previous, dropped);
    let model = agent_model(deps, &run.agent_id);
    let summary = summarizer
        .summarize(SummaryRequest {
            agent_id: &run.agent_id,
            model: &model,
            system: SUMMARY_SYSTEM,
            prompt: &prompt,
            max_tokens: SUMMARY_MAX_TOKENS,
        })
        .await?
        .trim()
        .to_string();
    if summary.is_empty() {
        return Err("The model returned an empty summary".to_string());
    }

    let child_id = uuid::Uuid::new_v4().to_string();
    let child = RunSummary {
        id: child_id.clone(),
        agent_id: run.agent_id.clone(),
        conversation_id: run.conversation_id.clone(),
        status: RunStatus::Running,
        input: run.input.clone(),
        summary: None,
        started_at: db::ts(db::now_ms()),
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: None,
        model: None,
        source_type: run.source_type,
        source_id: run.source_id.clone(),
        source_label: run.source_label.clone(),
        priority: run.priority,
        queued_at: None,
        parent_run_id: Some(run.id.clone()),
        compaction_summary: Some(summary.clone()),
    };
    let rows = child_messages(&run, &child_id, &summary, kept);
    let (dropped_count, kept_count) = (dropped.len(), kept.len());

    let (parent, child) = deps
        .bus
        .db
        .call(move |conn| db::start_compacted_child(conn, &child, &rows))
        .await?;

    for updated in [&parent, &child] {
        events::emit(
            &deps.bus,
            &AgentRunUpdated {
                agent_id: updated.agent_id.clone(),
                conversation_id: updated.conversation_id.clone(),
                run: updated.clone(),
            },
        );
    }
    let agent_id = run.agent_id.clone();
    match deps
        .bus
        .db
        .call(move |conn| {
            Ok((
                db::list_queue(conn, &agent_id)?,
                db::active_agent_run(conn, &agent_id)?,
            ))
        })
        .await
    {
        Ok((queue, current)) => events::emit(
            &deps.bus,
            &AgentQueueChanged {
                agent_id: run.agent_id.clone(),
                queue,
                current,
            },
        ),
        Err(e) => log::warn!("Couldn't announce the queue after compaction: {e}"),
    }
    events::emit(
        &deps.bus,
        &AgentRunCompacted {
            agent_id: run.agent_id.clone(),
            run_id: run.id.clone(),
            new_run_id: child_id.clone(),
        },
    );
    log::info!(
        "Compacted run {} into {child_id} ({dropped_count} messages summarized, {kept_count} kept)",
        run.id
    );

    Ok(CompactionResult {
        new_run_id: child_id,
        summary,
        parent_run_id: run.id,
    })
}

/// Where the kept (most recent) messages start: the last `percent`% of
/// them, moved back so a step's tool calls and approvals stay with the
/// assistant message that planned them.
pub fn split_point(messages: &[&MessageRow], percent: u8) -> usize {
    let len = messages.len();
    let keep = (len * usize::from(percent.min(100))).div_ceil(100);
    let mut start = len - keep;
    while start > 0 && start < len && is_step_continuation(&messages[start].role) {
        start -= 1;
    }
    start
}

fn is_step_continuation(role_name: &str) -> bool {
    role_name == role::TOOL_CALL || role_name == role::APPROVAL
}

/// The agent's skill.md model, or the default (Studio, or an unreadable
/// skill.md)
fn agent_model(deps: &RunDeps, agent_id: &str) -> String {
    if agent_id == STUDIO_AGENT_ID {
        return super::default_model(&deps.store);
    }
    deps.store
        .skill(agent_id)
        .map(|skill| skill.model)
        .unwrap_or_else(|_| super::default_model(&deps.store))
}

/// The prompt for the summary call.
pub fn summary_prompt(previous: Option<&str>, dropped: &[&MessageRow]) -> String {
    let turns = dropped
        .iter()
        .filter(|m| m.role == role::USER || m.role == role::ASSISTANT)
        .count()
        .max(1);
    let previous = previous
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(none: this is the first compaction)");
    let transcript = transcript(dropped);
    format!(
        "Given the previous summary and the last {turns} turns, produce an updated summary. \
Preserve still-true decisions. Remove stale information. \
Format: Objective / Important Details / Work State / Next Move.\n\n\
## Previous summary\n\n{previous}\n\n## Last {turns} turns\n\n{transcript}"
    )
}

fn transcript(messages: &[&MessageRow]) -> String {
    let lines: Vec<String> = messages.iter().filter_map(|m| transcript_line(m)).collect();
    // The most recent lines within the overall cap
    let mut total = 0;
    let mut kept = Vec::new();
    for line in lines.into_iter().rev() {
        total += line.len() + 1;
        if total > MAX_TRANSCRIPT_CHARS && !kept.is_empty() {
            kept.push("[… earlier turns omitted]".to_string());
            break;
        }
        kept.push(line);
    }
    kept.reverse();
    kept.join("\n")
}

fn transcript_line(message: &MessageRow) -> Option<String> {
    let line = match message.role.as_str() {
        role::USER => format!("User: {}", message.content.trim()),
        role::ASSISTANT => {
            let meta: AssistantMeta =
                serde_json::from_value(message.metadata.clone()).unwrap_or_default();
            let text = message.content.trim();
            if meta.planned_calls.is_empty() {
                if text.is_empty() {
                    return None;
                }
                format!("Assistant: {text}")
            } else {
                let calls: Vec<&str> = meta.planned_calls.iter().map(|c| c.tool.as_str()).collect();
                format!("Assistant: {text} [calls: {}]", calls.join(", "))
            }
        }
        role::TOOL_CALL => {
            let meta: ToolCallMeta = serde_json::from_value(message.metadata.clone()).ok()?;
            let outcome = match (&meta.result, meta.is_error) {
                (Some(result), true) => format!("error: {result}"),
                (Some(result), false) => result.clone(),
                (None, _) => "(no result)".to_string(),
            };
            format!("Tool {}({}) → {outcome}", meta.tool, meta.args)
        }
        role::ERROR => format!("Error: {}", message.content.trim()),
        _ => return None,
    };
    Some(events::truncate(&line, MAX_TRANSCRIPT_MESSAGE_CHARS))
}

/// The child run's messages: the summary, then copies of `kept`.
///
/// Message IDs are primary keys, so every copy gets a new one. A tool call
/// message's ID is its call's ID, so it's remapped in the planning assistant
/// message too; calls without a result message keep their ID so their
/// approvals still resolve.
fn child_messages(
    parent: &RunSummary,
    child_id: &str,
    summary: &str,
    kept: &[&MessageRow],
) -> Vec<MessageRow> {
    let call_ids: HashMap<&str, String> = kept
        .iter()
        .filter(|m| m.role == role::TOOL_CALL)
        .map(|m| {
            (
                m.id.as_str(),
                format!("cmp_{}", uuid::Uuid::new_v4().simple()),
            )
        })
        .collect();

    let mut summary_message = MessageRow::new(
        uuid::Uuid::new_v4().to_string(),
        &parent.conversation_id,
        Some(child_id),
        role::ASSISTANT,
        summary,
        serde_json::json!({ db::COMPACTION_SUMMARY_KEY: parent.id }),
    );
    // Sorts before the copies, which keep their original times
    if let Some(first) = kept.first() {
        summary_message.created_at = first.created_at - 1;
    }

    let mut out = vec![summary_message];
    for message in kept {
        let id = call_ids
            .get(message.id.as_str())
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut metadata = match &message.metadata {
            Value::Object(map) => map.clone(),
            _ => serde_json::Map::new(),
        };
        if message.role == role::ASSISTANT {
            if let Some(Value::Array(calls)) = metadata.get_mut("plannedCalls") {
                for call in calls {
                    let mapped = call
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|old| call_ids.get(old))
                        .cloned();
                    if let Some(mapped) = mapped {
                        call["id"] = Value::String(mapped);
                    }
                }
            }
        }
        metadata.insert(
            db::COMPACTION_COPY_KEY.to_string(),
            Value::String(message.id.clone()),
        );
        out.push(MessageRow {
            id,
            conversation_id: message.conversation_id.clone(),
            run_id: Some(child_id.to_string()),
            role: message.role.clone(),
            content: message.content.clone(),
            created_at: message.created_at,
            metadata: Value::Object(metadata),
        });
    }
    out
}

/// chars / 4 over the messages and their tool results
fn estimate_tokens(messages: &[MessageRow]) -> u32 {
    let chars: usize = messages
        .iter()
        .map(|m| {
            let result = m
                .metadata
                .get("result")
                .and_then(Value::as_str)
                .map_or(0, |r| r.chars().count());
            m.content.chars().count() + result
        })
        .sum();
    u32::try_from(chars / 4).unwrap_or(u32::MAX)
}

pub struct CompactionMiddleware {
    pub deps: RunDeps,
    pub policy: CompactionPolicy,
    /// The summary call (the agent's model unless a test injects one)
    pub summarizer: Arc<dyn Summarizer>,
}

impl CompactionMiddleware {
    pub fn new(deps: RunDeps, policy: CompactionPolicy) -> Self {
        let summarizer = Arc::new(LlmSummarizer::new(deps.secrets.clone()));
        Self {
            deps,
            policy,
            summarizer,
        }
    }

    #[cfg(test)]
    pub fn with_summarizer(mut self, summarizer: Arc<dyn Summarizer>) -> Self {
        self.summarizer = summarizer;
        self
    }

    /// Compacts when the context passed the threshold; on success the loop
    /// continues in the child run. Failures only log.
    async fn maybe_compact(&self, ctx: &mut TurnContext) {
        if !self
            .policy
            .should_compact(ctx.token_estimate, ctx.context_window)
        {
            return;
        }
        let result = compact_using(
            &self.deps,
            &ctx.run_id,
            &self.policy,
            self.summarizer.as_ref(),
        )
        .await;
        match result {
            Ok(result) => {
                ctx.run_id = result.new_run_id.clone();
                let id = result.new_run_id;
                match self
                    .deps
                    .bus
                    .db
                    .call(move |conn| db::list_run_messages(conn, &id))
                    .await
                {
                    Ok(messages) => ctx.token_estimate = estimate_tokens(&messages),
                    Err(e) => log::warn!("Couldn't re-estimate tokens after compaction: {e}"),
                }
            }
            Err(e) => log::warn!(
                "Compaction of run {} skipped (the run goes on): {e}",
                ctx.run_id
            ),
        }
    }
}

impl TurnMiddleware for CompactionMiddleware {
    fn name(&self) -> &'static str {
        super::middleware::COMPACTION
    }

    fn pre_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        Box::pin(async move {
            self.maybe_compact(ctx).await;
            Ok(MiddlewareDecision::Continue)
        })
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::db::PlannedCall;
    use crate::model::{SourceType, ToolCallStatus};
    use crate::services::events::tests::RecordingSink;
    use crate::services::AppState;
    use serde_json::json;

    type Reply = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

    /// Records prompts; answers with `reply(prompt)`
    struct FakeSummarizer {
        prompts: Mutex<Vec<(String, String)>>,
        reply: Reply,
    }

    impl FakeSummarizer {
        fn new(
            reply: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                prompts: Mutex::new(Vec::new()),
                reply: Box::new(reply),
            })
        }

        fn fixed(text: &str) -> Arc<Self> {
            let text = text.to_string();
            Self::new(move |_| Ok(text.clone()))
        }

        fn prompts(&self) -> Vec<(String, String)> {
            self.prompts.lock().unwrap().clone()
        }
    }

    impl Summarizer for FakeSummarizer {
        fn summarize<'a>(
            &'a self,
            request: SummaryRequest<'a>,
        ) -> BoxFuture<'a, Result<String, String>> {
            self.prompts
                .lock()
                .unwrap()
                .push((request.model.to_string(), request.prompt.to_string()));
            let reply = (self.reply)(request.prompt);
            Box::pin(async move { reply })
        }
    }

    fn row(
        id: &str,
        conversation: &str,
        run: &str,
        role_name: &str,
        content: &str,
        meta: Value,
        at: i64,
    ) -> MessageRow {
        let mut row = MessageRow::new(id, conversation, Some(run), role_name, content, meta);
        row.created_at = at;
        row
    }

    /// A user message, then `steps` steps that each write a file
    fn conversation(conversation: &str, run: &str, steps: usize) -> Vec<MessageRow> {
        let base = 1_000_000;
        let mut rows = vec![row(
            &format!("{run}-user"),
            conversation,
            run,
            role::USER,
            "Refactor the parser and keep the tests green",
            json!({}),
            base,
        )];
        for i in 0..steps {
            let call_id = format!("{run}-call-{i}");
            let at = base + 10 * (i as i64 + 1);
            rows.push(row(
                &format!("{run}-step-{i}"),
                conversation,
                run,
                role::ASSISTANT,
                &format!("Step {i}: editing src/parser_{i}.rs {}", "x".repeat(200)),
                serde_json::to_value(AssistantMeta {
                    step: i as u32,
                    planned_calls: vec![PlannedCall {
                        id: call_id.clone(),
                        tool: "write_file".into(),
                        args: json!({ "path": format!("src/parser_{i}.rs") }),
                        requires_approval: false,
                    }],
                    reasoning: None,
                })
                .unwrap(),
                at,
            ));
            rows.push(row(
                &call_id,
                conversation,
                run,
                role::TOOL_CALL,
                "",
                serde_json::to_value(ToolCallMeta {
                    tool: "write_file".into(),
                    args: json!({ "path": format!("src/parser_{i}.rs") }),
                    status: ToolCallStatus::Done,
                    result: Some(format!("wrote parser_{i}")),
                    is_error: false,
                })
                .unwrap(),
                at + 1,
            ));
        }
        rows
    }

    async fn seed(state: &AppState, run_id: &str, steps: usize, previous: Option<&str>) -> String {
        let (run_id, previous) = (run_id.to_string(), previous.map(str::to_string));
        state
            .db()
            .call(move |conn| {
                let conversation_id = db::tests::start_run(conn, "writer", &run_id);
                conn.execute(
                    "UPDATE runs SET compaction_summary = ?2, source_type = 'workflow', priority = 30 WHERE id = ?1",
                    rusqlite::params![run_id, previous],
                )?;
                for message in conversation(&conversation_id, &run_id, steps) {
                    db::insert_message(conn, &message)?;
                }
                Ok(conversation_id)
            })
            .await
            .unwrap()
    }

    fn context(run_id: &str, token_estimate: u32) -> TurnContext {
        TurnContext {
            agent_id: "writer".into(),
            run_id: run_id.into(),
            turn_number: 21,
            token_estimate,
            context_window: 64_000,
            ..TurnContext::default()
        }
    }

    fn middleware(
        state: &AppState,
        policy: CompactionPolicy,
        summarizer: Arc<FakeSummarizer>,
    ) -> CompactionMiddleware {
        CompactionMiddleware::new(state.orchestrators().deps().clone(), policy)
            .with_summarizer(summarizer)
    }

    #[test]
    fn the_threshold_is_the_window_minus_the_buffer_or_trigger_tokens() {
        let policy = CompactionPolicy::default();
        assert_eq!(policy.threshold(64_000), Some(51_000));
        assert!(!policy.should_compact(50_999, 64_000));
        assert!(policy.should_compact(51_000, 64_000));
        // An unknown window never compacts by default
        assert_eq!(policy.threshold(0), None);
        assert!(!policy.should_compact(u32::MAX, 0));
        // A small window keeps half of it
        assert_eq!(policy.threshold(16_000), Some(8_000));

        let explicit = CompactionPolicy {
            trigger_tokens: Some(1_000),
            ..CompactionPolicy::default()
        };
        assert!(!explicit.should_compact(999, 64_000));
        assert!(explicit.should_compact(1_000, 64_000));
        assert!(explicit.should_compact(1_000, 0));
    }

    #[test]
    fn the_split_keeps_whole_steps() {
        let rows = conversation("c", "r", 4); // user + 4 × (assistant, tool call) = 9
        let refs: Vec<&MessageRow> = rows.iter().collect();
        // 30% of 9 → keep 3, starting at index 6 (a tool call) → back to 5
        let split = split_point(&refs, 30);
        assert_eq!(split, 5);
        assert_eq!(refs[split].role, role::ASSISTANT);
        assert_eq!(split_point(&refs, 100), 0);
        assert_eq!(split_point(&refs, 0), 9);
    }

    #[tokio::test]
    async fn below_the_threshold_nothing_happens() {
        let state = AppState::for_tests(vec![]);
        seed(&state, "r1", 10, None).await;
        let summarizer = FakeSummarizer::fixed("unused");
        let mw = middleware(&state, CompactionPolicy::default(), summarizer.clone());

        let mut ctx = context("r1", 50_000);
        assert_eq!(
            mw.pre_turn(&mut ctx).await.unwrap(),
            MiddlewareDecision::Continue
        );
        assert_eq!(ctx.run_id, "r1");
        assert!(summarizer.prompts().is_empty());
    }

    #[tokio::test]
    async fn a_long_run_continues_in_a_compacted_child() {
        let sink = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![sink.clone()]);
        let conversation = seed(&state, "r1", 20, None).await;
        let summarizer = FakeSummarizer::fixed(
            "Objective: refactor the parser\nImportant Details: tests in tests/\nWork State: 14 files edited\nNext Move: run cargo test",
        );
        let policy = CompactionPolicy {
            trigger_tokens: Some(1_000),
            ..CompactionPolicy::default()
        };
        let mw = middleware(&state, policy, summarizer.clone());

        let mut ctx = context("r1", 5_000);
        assert_eq!(
            mw.pre_turn(&mut ctx).await.unwrap(),
            MiddlewareDecision::Continue
        );
        let child_id = ctx.run_id.clone();
        assert_ne!(child_id, "r1");
        assert!(ctx.token_estimate < 5_000);

        // The summary call: the agent's model, the dropped turns
        let prompts = summarizer.prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(
            prompts[0].0,
            crate::services::domain::settings::DEFAULT_MODEL
        );
        assert!(prompts[0].1.contains("produce an updated summary"));
        assert!(prompts[0].1.contains("Refactor the parser"));
        assert!(prompts[0]
            .1
            .contains("(none: this is the first compaction)"));

        let id = child_id.clone();
        let conv = conversation.clone();
        let (parent, child, child_rows, visible) = state
            .db()
            .call(move |conn| {
                Ok((
                    db::get_run(conn, "r1")?.unwrap(),
                    db::get_run(conn, &id)?.unwrap(),
                    db::list_run_messages(conn, &id)?,
                    db::list_messages(conn, &conv, 1_000)?,
                ))
            })
            .await
            .unwrap();

        assert_eq!(parent.status, RunStatus::Compacted);
        assert!(parent.ended_at.is_some());
        assert_eq!(child.status, RunStatus::Running);
        assert_eq!(child.parent_run_id.as_deref(), Some("r1"));
        assert!(child
            .compaction_summary
            .as_deref()
            .unwrap()
            .starts_with("Objective: refactor the parser"));
        assert_eq!(child.conversation_id, conversation);
        assert_eq!(child.input, parent.input);
        assert_eq!(child.source_type, SourceType::Workflow);
        assert_eq!(child.priority, 30);

        // Summary first, then whole steps, the last 30% of 41 messages
        assert_eq!(child_rows[0].role, role::ASSISTANT);
        assert!(child_rows[0]
            .metadata
            .get(db::COMPACTION_SUMMARY_KEY)
            .is_some());
        assert_eq!(child_rows[1].role, role::ASSISTANT);
        let copies = &child_rows[1..];
        assert_eq!(copies.len(), 14);
        assert!(copies
            .iter()
            .all(|m| m.metadata.get(db::COMPACTION_COPY_KEY).is_some()));
        assert_eq!(
            copies.last().unwrap().metadata[db::COMPACTION_COPY_KEY],
            json!("r1-call-19")
        );
        // Each copied step's call IDs point at its copied results
        for pair in copies.chunks(2) {
            let meta: AssistantMeta = serde_json::from_value(pair[0].metadata.clone()).unwrap();
            assert_eq!(meta.planned_calls[0].id, pair[1].id);
            assert_eq!(pair[1].role, role::TOOL_CALL);
        }

        // The conversation doesn't show the copies twice
        assert_eq!(visible.len(), 41);
        assert!(visible.iter().all(|m| m.run_id.as_deref() == Some("r1")));

        let names = sink.names();
        assert!(names.contains(&AgentRunCompacted::NAME));
        let compacted = sink
            .take()
            .into_iter()
            .find(|(name, _)| *name == AgentRunCompacted::NAME)
            .unwrap()
            .1;
        assert_eq!(compacted["runId"], json!("r1"));
        assert_eq!(compacted["newRunId"], json!(child_id));
        assert_eq!(compacted["agentId"], json!("writer"));
    }

    #[tokio::test]
    async fn iterative_merge_carries_the_previous_summary() {
        let state = AppState::for_tests(vec![]);
        let previous = "Objective: migrate the DB\nImportant Details: keep v7 compatible";
        seed(&state, "r1", 10, Some(previous)).await;
        // A model that keeps what's still true from the previous summary
        let summarizer = FakeSummarizer::new(|prompt| {
            let kept = prompt
                .split("## Previous summary\n\n")
                .nth(1)
                .and_then(|rest| rest.split("\n\n## Last").next())
                .unwrap_or_default()
                .to_string();
            Ok(format!("{kept}\nWork State: migration written"))
        });
        let policy = CompactionPolicy {
            trigger_tokens: Some(10),
            ..CompactionPolicy::default()
        };
        let mw = middleware(&state, policy, summarizer.clone());

        let mut ctx = context("r1", 100);
        mw.pre_turn(&mut ctx).await.unwrap();

        assert!(summarizer.prompts()[0].1.contains(previous));
        let id = ctx.run_id.clone();
        let child = state
            .db()
            .call(move |conn| db::get_run(conn, &id))
            .await
            .unwrap()
            .unwrap();
        let summary = child.compaction_summary.unwrap();
        assert!(summary.contains("Objective: migrate the DB"));
        assert!(summary.contains("keep v7 compatible"));
        assert!(summary.contains("Work State: migration written"));

        // Compacting the child again merges into its summary
        let mut ctx2 = context(&ctx.run_id, 100);
        // The child's copies alone are few; preserve less so some drop
        let again = CompactionMiddleware::new(
            state.orchestrators().deps().clone(),
            CompactionPolicy {
                trigger_tokens: Some(10),
                preserve_recent_percent: 10,
                iterative_merge: true,
            },
        )
        .with_summarizer(summarizer.clone());
        again.pre_turn(&mut ctx2).await.unwrap();
        assert_ne!(ctx2.run_id, ctx.run_id);
        assert!(summarizer.prompts()[1]
            .1
            .contains("Work State: migration written"));
    }

    #[tokio::test]
    async fn without_iterative_merge_the_previous_summary_is_not_sent() {
        let state = AppState::for_tests(vec![]);
        seed(&state, "r1", 10, Some("Objective: old")).await;
        let summarizer = FakeSummarizer::fixed("fresh");
        let policy = CompactionPolicy {
            trigger_tokens: Some(10),
            iterative_merge: false,
            ..CompactionPolicy::default()
        };
        let mut ctx = context("r1", 100);
        middleware(&state, policy, summarizer.clone())
            .pre_turn(&mut ctx)
            .await
            .unwrap();
        assert!(!summarizer.prompts()[0].1.contains("Objective: old"));
    }

    #[tokio::test]
    async fn a_failed_summary_leaves_the_run_going() {
        let state = AppState::for_tests(vec![]);
        seed(&state, "r1", 10, None).await;
        let summarizer = FakeSummarizer::new(|_| Err("rate limited".into()));
        let policy = CompactionPolicy {
            trigger_tokens: Some(10),
            ..CompactionPolicy::default()
        };
        let mut ctx = context("r1", 100);
        let decision = middleware(&state, policy, summarizer)
            .pre_turn(&mut ctx)
            .await
            .unwrap();
        assert_eq!(decision, MiddlewareDecision::Continue);
        assert_eq!(ctx.run_id, "r1");
        let (run, children) = state
            .db()
            .call(|conn| {
                Ok((
                    db::get_run(conn, "r1")?.unwrap(),
                    db::child_runs(conn, "r1")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(run.status, RunStatus::Running);
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn compacting_a_finished_run_is_refused() {
        let state = AppState::for_tests(vec![]);
        seed(&state, "r1", 4, None).await;
        state
            .db()
            .call(|conn| db::set_run_status(conn, "r1", RunStatus::Completed, None))
            .await
            .unwrap();
        let error = compact_using(
            state.orchestrators().deps(),
            "r1",
            &CompactionPolicy::default(),
            FakeSummarizer::fixed("x").as_ref(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("not running"));
    }
}
