//! A run's child task: the engine it uses, its step loop, and pausing,
//! aborting and continuing it.

use super::*;

/// Which engine plans this run's steps.
pub(super) enum Engine {
    /// Scripted replies from settings (tests, demos)
    Canned(CannedEngine),
    /// A real model via `llm::stream`
    Llm,
}

/// How a run's child task stopped
#[derive(Debug, PartialEq)]
pub(super) enum RunEnd {
    /// Completed, failed, or suspended for approval; state is persisted
    Done,
    /// A middleware (compaction) moved the turn into this child run
    ContinuedIn(String),
}

/// The turn policy of `agent_id`'s agent.md, and the middleware pipeline
/// it declares (see `middleware.rs`), built once per `execute_run`.
///
/// Studio, and agents without a definition, get only the built-in limits:
/// memory, compaction and verify-on-stop work on an agent folder.
pub(super) fn run_pipeline(
    deps: &RunDeps,
    agent_id: &str,
) -> (TurnPolicy, Vec<Box<dyn TurnMiddleware>>) {
    let agent = (agent_id != STUDIO_AGENT_ID)
        .then(|| deps.store.agent(agent_id).ok())
        .flatten();
    let policy = agent
        .as_ref()
        .map(|agent| TurnPolicy::from_extra(agent.extra()))
        .unwrap_or_default();
    let mut pipeline = middleware::register_default_middlewares(deps, &policy);
    if agent.is_none() {
        pipeline.retain(|m| middleware::BUILTIN_MIDDLEWARES.contains(&m.name()));
    }
    (policy, pipeline)
}

/// Advances a run from its persisted state until it completes or suspends,
/// walking the run's middleware pipeline around each engine step.
pub(super) async fn execute_run(
    deps: &RunDeps,
    delegation: &DelegationSlot,
    agent_id: &str,
    run_id: &str,
) -> Result<RunEnd, String> {
    if harness_run(deps, agent_id, run_id).await? {
        return Ok(RunEnd::Done);
    }
    // Harness runs are attributed by the router; this one runs natively
    crate::services::harness::evidence::attribute_run(&deps.bus.db, run_id, "native", None, None)
        .await?;
    let (policy, mut pipeline) = run_pipeline(deps, agent_id);
    // The email and calendar checks' runs have no plan tools: their work
    // goes through the intake, and their notes writes need no plan
    if data_run(deps, run_id).await.is_some() {
        pipeline
            .retain(|m| ![middleware::PLAN_FIRST, middleware::FINISH_CHECK].contains(&m.name()));
    }
    let model = (agent_id != STUDIO_AGENT_ID)
        .then(|| deps.store.skill(agent_id).ok().map(|skill| skill.model))
        .flatten()
        .unwrap_or_else(|| default_model(&deps.store));
    let turn = TurnContext {
        agent_id: agent_id.to_string(),
        run_id: run_id.to_string(),
        context_window: middleware::context_window_for(&model),
        policy,
        ..TurnContext::default()
    };
    drive_run(deps, delegation, run_id, &pipeline, turn).await
}

/// Runs the run on the agent's harness, if it has one (`runtime:`), and
/// says whether it did. Native agents, Studio, and agents that fall back
/// return `false` and take the native path unchanged.
async fn harness_run(deps: &RunDeps, agent_id: &str, run_id: &str) -> Result<bool, String> {
    use runtime_router::RuntimeDecision;

    if agent_id == STUDIO_AGENT_ID {
        return Ok(false);
    }
    let Ok(agent) = deps.store.agent(agent_id) else {
        return Ok(false);
    };
    if agent.runtime().is_native() {
        return Ok(false);
    }
    let Some(app) = deps.harness.get().and_then(|weak| weak.upgrade()) else {
        return Ok(false);
    };
    let decision = runtime_router::resolve_runtime(&app, &agent).await;
    if matches!(decision, RuntimeDecision::Native) {
        return Ok(false);
    }
    let id = run_id.to_string();
    let run = deps
        .bus
        .db
        .call(move |conn| db::get_run(conn, &id))
        .await?
        .ok_or_else(|| format!("Run {run_id} not found"))?;
    let ctx = RunContext {
        agent_id: agent_id.to_string(),
        agent_name: display_name(&deps.store, agent_id),
        conversation_id: run.conversation_id,
        run_id: run_id.to_string(),
    };
    match decision {
        RuntimeDecision::Native => Ok(false),
        RuntimeDecision::Unavailable(reason) => {
            deps.bus
                .turn_done(&ctx, RunStatus::Failed, Some(reason))
                .await?;
            Ok(true)
        }
        RuntimeDecision::Harness(plan) => {
            let service = app.harnesses();
            match service.get_adapter(&plan.harness_id).await {
                Ok(adapter) => {
                    runtime_router::execute_harness_run(&deps.bus, service, adapter, &plan, &ctx)
                        .await?
                }
                Err(e) => deps.bus.turn_done(&ctx, RunStatus::Failed, Some(e)).await?,
            }
            Ok(true)
        }
    }
}

/// The loop behind `execute_run`, with the pipeline passed in (tests use
/// their own middlewares).
///
/// Each iteration: run the latest step's pending tool calls (suspending on
/// approval; plan mode blocks mutating ones); a step with no tool calls is
/// the final answer, unless a `post_turn` asked to continue. Otherwise run
/// every `pre_turn` (which may abort the run, pause it for approval, or
/// move it into a child run), ask the engine for the next step, then run
/// every `post_turn` in reverse.
///
/// `turn` lives for this call only: a run resumed after an approval starts
/// a fresh one, rebuilt from the DB (only middleware-private flags such as
/// `verify_fired` don't survive the suspension).
pub(super) async fn drive_run(
    deps: &RunDeps,
    delegation: &DelegationSlot,
    run_id: &str,
    pipeline: &[Box<dyn TurnMiddleware>],
    mut turn: TurnContext,
) -> Result<RunEnd, String> {
    let bus = &deps.bus;
    let agent_id = turn.agent_id.clone();
    let agent_id = agent_id.as_str();
    // TurnLimit stops the run cleanly after `max_turns` steps; this bound
    // only guards against a middleware that keeps continuing without one
    let max_iterations = turn.policy.max_turns.saturating_add(2);
    for _ in 0..max_iterations {
        let id = run_id.to_string();
        let (engine, run, messages) = bus
            .db
            .call(move |conn| {
                let engine = if db::uses_canned_engine(conn)? {
                    Engine::Canned(CannedEngine::load(conn)?)
                } else {
                    Engine::Llm
                };
                let run = db::get_run(conn, &id)?;
                let messages = db::list_run_messages(conn, &id)?;
                Ok((engine, run, messages))
            })
            .await?;
        let run = run.ok_or_else(|| format!("Run {run_id} not found"))?;
        if run.status != RunStatus::Running {
            return Ok(RunEnd::Done);
        }
        let ctx = RunContext {
            agent_id: agent_id.to_string(),
            agent_name: display_name(&deps.store, agent_id),
            conversation_id: run.conversation_id.clone(),
            run_id: run.id.clone(),
        };

        let steps: Vec<AssistantMeta> = messages
            .iter()
            .filter(|m| m.role == role::ASSISTANT)
            .filter_map(|m| serde_json::from_value(m.metadata.clone()).ok())
            .collect();

        let (run, messages) = match steps.last() {
            Some(step) if step.planned_calls.is_empty() => {
                if !continued_after_last_step(&messages) {
                    bus.turn_done(&ctx, RunStatus::Completed, None).await?;
                    return Ok(RunEnd::Done);
                }
                (run, messages)
            }
            Some(step) => {
                for call in &step.planned_calls {
                    if !run_call(deps, delegation, &engine, &ctx, &messages, call, pipeline).await?
                    {
                        return Ok(RunEnd::Done); // suspended for approval
                    }
                }
                // The middlewares see the results (and cost) so far
                let id = run_id.to_string();
                let (run, messages) = bus
                    .db
                    .call(move |conn| {
                        Ok((db::get_run(conn, &id)?, db::list_run_messages(conn, &id)?))
                    })
                    .await?;
                (
                    run.ok_or_else(|| format!("Run {run_id} not found"))?,
                    messages,
                )
            }
            None => (run, messages),
        };

        let step_index = steps.len() as u32;
        refresh_turn(&mut turn, &run, &messages, step_index);

        // --- pre_turn, in order -------------------------------------------
        let pauses = pause_statuses(bus, run_id, step_index, pipeline).await?;
        let approved: Vec<&str> = pauses
            .iter()
            .filter(|(_, status)| *status == ApprovalStatus::Approved)
            .map(|(name, _)| *name)
            .collect();
        if let Some(stop) = middleware::run_pre_turns(pipeline, &mut turn, &approved).await {
            match stop.decision {
                MiddlewareDecision::Abort(reason) => {
                    abort_run(deps, &ctx, stop.middleware, reason).await?;
                    return Ok(RunEnd::Done);
                }
                MiddlewareDecision::PauseForApproval(reason) => {
                    let status = pauses
                        .iter()
                        .find(|(name, _)| *name == stop.middleware)
                        .map(|(_, status)| *status);
                    match status {
                        None => {
                            let call = pause_call(run_id, step_index, stop.middleware, &reason);
                            bus.approval_request(&ctx, &call).await?;
                        }
                        Some(ApprovalStatus::Pending) => {}
                        // Rejected (or expired): the user said no
                        Some(_) => {
                            let reason = format!("Stopped: {reason} (not approved)");
                            abort_run(deps, &ctx, stop.middleware, reason).await?;
                        }
                    }
                    return Ok(RunEnd::Done);
                }
                MiddlewareDecision::Continue => {}
            }
        }
        if turn.run_id != run_id {
            // Compaction ended this run and started a child: the
            // orchestrator moves its current turn there (see `step_finished`)
            return Ok(RunEnd::ContinuedIn(turn.run_id.clone()));
        }

        let message_id = uuid::Uuid::new_v4().to_string();
        let meta = match &engine {
            Engine::Canned(canned) => {
                let Some(step) = canned.plan(agent_id, &run.input, step_index) else {
                    bus.turn_done(&ctx, RunStatus::Completed, None).await?;
                    return Ok(RunEnd::Done);
                };
                for chunk in chunk_text(&step.text) {
                    bus.token(&ctx, &message_id, step_index, chunk).await?;
                    tokio::time::sleep(canned.token_delay()).await;
                }
                // A2A: the agent's policy decides the approval
                let policy = a2a::tool::policy(&deps.store, agent_id);
                let planned_calls = step
                    .calls
                    .into_iter()
                    .map(|mut call| {
                        call.requires_approval |= policy.requires_approval_for(&call.tool);
                        call
                    })
                    .collect();
                AssistantMeta {
                    step: step_index,
                    planned_calls,
                    reasoning: None,
                }
            }
            Engine::Llm => {
                llm_step(
                    deps,
                    delegation,
                    &ctx,
                    &message_id,
                    step_index,
                    &turn.system_extras,
                )
                .await?
            }
        };
        let mut meta = meta;
        // Studio starting a Request: the plan is checked, and the call
        // waits for the user's OK when a step acts (`studio::start`).
        // Saving a definition always waits (`studio::save`).
        if agent_id == STUDIO_AGENT_ID {
            studio::start::gate(app_state(deps).as_ref(), &mut meta.planned_calls).await;
            studio::save::gate(&mut meta.planned_calls);
        } else {
            // A proposed playbook always waits for the user
            playbooks::gate(&deps.store, agent_id, &mut meta.planned_calls);
            // So does an Inbox chat's decision (`inbox::tool`)
            inbox::tool::gate(&mut meta.planned_calls);
        }
        let planned = meta.planned_calls.clone();
        bus.finish_step(&ctx, &message_id, meta).await?;

        // --- post_turn, in reverse ----------------------------------------
        let id = message_id.clone();
        let text = bus
            .db
            .call(move |conn| db::get_message(conn, &id))
            .await?
            .map(|m| m.content)
            .filter(|text| !text.is_empty());
        let mut result = TurnResult {
            final_message: text,
            tool_calls_made: planned.iter().map(|c| c.tool.clone()).collect(),
            files_modified: planned
                .iter()
                .filter_map(|c| middleware::written_path(&c.tool, &c.args))
                .collect(),
            exit_reason: if planned.is_empty() {
                ExitReason::Completed
            } else {
                ExitReason::ToolCalls
            },
            continue_with: None,
        };
        middleware::run_post_turns(pipeline, &mut turn, &mut result).await;
        if let Some(text) = result.continue_with {
            append_continuation(bus, &ctx, text).await?;
        }
    }
    Err(format!("Stopped after {max_iterations} steps"))
}

/// Whether a `post_turn` continuation (a user-role message) follows the
/// run's last assistant step, so a text-only step doesn't end the run.
fn continued_after_last_step(messages: &[MessageRow]) -> bool {
    let last = |role: &str| messages.iter().rposition(|m| m.role == role);
    match (last(role::ASSISTANT), last(role::USER)) {
        (Some(step), Some(user)) => user > step,
        _ => false,
    }
}

/// Refreshes what the middlewares see before step `step_index`.
pub(super) fn refresh_turn(
    turn: &mut TurnContext,
    run: &RunSummary,
    messages: &[MessageRow],
    step_index: u32,
) {
    turn.run_id = run.id.clone();
    turn.turn_number = step_index;
    turn.accumulated_cost = run.cost_usd.unwrap_or(0.0);
    turn.system_extras.clear();

    let chars: usize = messages
        .iter()
        .map(|m| {
            let meta = if m.role == role::USER {
                0
            } else {
                m.metadata.to_string().chars().count()
            };
            m.content.chars().count() + meta
        })
        .sum();
    turn.token_estimate = middleware::estimate_tokens(chars);

    // The step each call was planned in
    let mut step_of = HashMap::new();
    for meta in messages
        .iter()
        .filter(|m| m.role == role::ASSISTANT)
        .filter_map(|m| serde_json::from_value::<AssistantMeta>(m.metadata.clone()).ok())
    {
        for call in meta.planned_calls {
            step_of.insert(call.id, meta.step);
        }
    }
    turn.finish_check_fired = messages
        .iter()
        .any(|m| m.role == role::USER && m.content.starts_with(run_plan::FINISH_CHECK_LEAD));
    turn.last_tool_results.clear();
    turn.files_touched.clear();
    for message in messages.iter().filter(|m| m.role == role::TOOL_CALL) {
        let Ok(meta) = serde_json::from_value::<ToolCallMeta>(message.metadata.clone()) else {
            continue;
        };
        if meta.status != ToolCallStatus::Done {
            continue;
        }
        if !meta.is_error {
            if let Some(path) = middleware::written_path(&meta.tool, &meta.args) {
                if !turn.files_touched.contains(&path) {
                    turn.files_touched.push(path);
                }
            }
        }
        turn.last_tool_results.push(ToolResultSummary {
            turn: step_of.get(&message.id).copied().unwrap_or(0),
            tool: meta.tool,
            args: meta.args,
            is_error: meta.is_error,
        });
    }
}

/// The tool name of the synthetic call a `PauseForApproval` asks about
pub const PAUSE_TOOL: &str = "continue_run";

/// One pause per run, step and middleware, so a decision is found again
/// when the run resumes
pub(super) fn pause_call_id(run_id: &str, step_index: u32, middleware: &str) -> String {
    format!("pause:{run_id}:{step_index}:{middleware}")
}

fn pause_call(run_id: &str, step_index: u32, middleware: &str, reason: &str) -> PlannedCall {
    PlannedCall {
        id: pause_call_id(run_id, step_index, middleware),
        tool: PAUSE_TOOL.to_string(),
        args: serde_json::json!({ "middleware": middleware, "reason": reason }),
        requires_approval: true,
    }
}

/// The approval status of each middleware's pause before step `step_index`,
/// for those that asked
async fn pause_statuses(
    bus: &EventBus,
    run_id: &str,
    step_index: u32,
    pipeline: &[Box<dyn TurnMiddleware>],
) -> Result<Vec<(&'static str, ApprovalStatus)>, String> {
    let names: Vec<&'static str> = pipeline.iter().map(|m| m.name()).collect();
    let ids: Vec<String> = names
        .iter()
        .map(|name| pause_call_id(run_id, step_index, name))
        .collect();
    let run = run_id.to_string();
    let statuses = bus
        .db
        .call(move |conn| {
            ids.iter()
                .map(|id| Ok(db::approval_for_call(conn, &run, id)?.map(|a| a.status)))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await?;
    Ok(names
        .into_iter()
        .zip(statuses)
        .filter_map(|(name, status)| Some((name, status?)))
        .collect())
}

/// Ends the run because a middleware said so: it fails with `reason` (an
/// error message in the chat, and a dashboard attention item), then
/// `agent://turn_aborted` names the middleware.
async fn abort_run(
    deps: &RunDeps,
    ctx: &RunContext,
    middleware: &str,
    reason: String,
) -> Result<(), String> {
    log::info!(
        "Middleware '{middleware}' stopped run {} of '{}': {reason}",
        ctx.run_id,
        ctx.agent_id
    );
    deps.bus
        .turn_done(ctx, RunStatus::Failed, Some(reason.clone()))
        .await?;
    // Kept in the run's log: Run Detail shows why it stopped, and a run a
    // policy stopped is never tried again on its own (`run_retry`)
    deps.bus
        .record(
            &ctx.run_id,
            AgentTurnAborted {
                agent_id: ctx.agent_id.clone(),
                run_id: ctx.run_id.clone(),
                middleware: middleware.to_string(),
                reason,
            },
        )
        .await
}

/// Appends a `post_turn` continuation to the run as a user-role message
/// (the role the model history replays). Chats leave it out.
async fn append_continuation(bus: &EventBus, ctx: &RunContext, text: String) -> Result<(), String> {
    let message = MessageRow::new(
        uuid::Uuid::new_v4().to_string(),
        &ctx.conversation_id,
        Some(&ctx.run_id),
        role::USER,
        text,
        serde_json::json!({ "attachments": [], "continuation": true }),
    );
    bus.add_continuation(ctx, message).await
}
