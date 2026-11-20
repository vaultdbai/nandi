//! Running a planned tool call once its checks pass (`call_gate`).

use super::*;

/// Runs one planned call if its checks pass (`call_gate`). Returns `false`
/// if the run is now waiting on a person.
pub(super) async fn run_call(
    deps: &RunDeps,
    delegation: &DelegationSlot,
    engine: &Engine,
    ctx: &RunContext,
    messages: &[MessageRow],
    call: &PlannedCall,
    pipeline: &[Box<dyn TurnMiddleware>],
) -> Result<bool, String> {
    let bus = &deps.bus;
    let existing = messages.iter().find(|m| m.id == call.id);
    let done = existing
        .and_then(|m| serde_json::from_value::<ToolCallMeta>(m.metadata.clone()).ok())
        .is_some_and(|meta| meta.status == ToolCallStatus::Done);
    if done {
        return Ok(true);
    }

    let gate = CallGate::new(LiveChecks {
        deps,
        ctx,
        pipeline,
    });
    let Clearance {
        connectors: allowed_connectors,
        plan: check,
    } = match gate.decide(call).await? {
        Verdict::Run(clearance) => clearance,
        Verdict::Refuse(reason) => {
            if existing.is_none() {
                bus.tool_call(ctx, call).await?;
            }
            let outcome = ToolOutcome {
                result: reason,
                is_error: true,
            };
            bus.tool_result(ctx, call, &outcome).await?;
            return Ok(true);
        }
        Verdict::Wait => return Ok(false),
        // Rejected or expired: skip (the model is told when it replans)
        Verdict::Declined => return Ok(true),
    };

    if existing.is_none() {
        bus.tool_call(ctx, call).await?;
    }
    let data_run = if ctx.agent_id == STUDIO_AGENT_ID {
        None
    } else {
        data_run(deps, &ctx.run_id).await
    };
    // Screenshots go into the run's files, named for the current step
    let screenshot = check
        .screenshot
        .then(|| screenshot_call(ctx, call, check.plan.as_ref()))
        .flatten();
    let call = screenshot.as_ref().map_or(call, |(call, _)| call);
    let outcome = match engine {
        // A check-started run: nothing but the notes and that check's
        // tools, even if a model names another one
        _ if data_run.is_some_and(|source| !data_run_allows(source, &call.tool)) => ToolOutcome {
            result: format!(
                "Tool '{}' is not available in a turn started by the {} check",
                call.tool,
                if data_run == Some(SourceType::CalendarCheck) {
                    "calendar"
                } else {
                    "email"
                }
            ),
            is_error: true,
        },
        // Calendars and files (read-only): on the host, like mail
        _ if ctx.agent_id != STUDIO_AGENT_ID
            && crate::services::workspace::tool::is_workspace_tool(&call.tool) =>
        {
            match app_state(deps) {
                Some(state) => {
                    let allowed = allowed_connectors.clone().unwrap_or_default();
                    crate::services::workspace::tool::call(&state, &allowed, &call.tool, &call.args)
                        .await
                }
                None => ToolOutcome {
                    result: format!("Tool '{}' is not available right now", call.tool),
                    is_error: true,
                },
            }
        }
        // An Inbox chat's decision, confirmed by the user: the same service
        // call as the item's button (`inbox::tool`)
        _ if inbox::tool::is_decision_tool(&call.tool) => match app_state(deps) {
            Some(state) if ctx.agent_id == DEFAULT_AGENT => {
                // A know-me card may have edited the proposal before the
                // confirm: the approval's arguments are the confirmed ones
                let args =
                    inbox::tool::confirmed_args(&state, &ctx.run_id, &call.id, &call.args).await;
                inbox::tool::call(&state, &ctx.conversation_id, &call.tool, &args).await
            }
            _ => ToolOutcome {
                result: format!(
                    "Tool '{}' only works in an Inbox item's chat with Nandi",
                    call.tool
                ),
                is_error: true,
            },
        },
        // Nandi's work tools: on the host
        _ if ctx.agent_id == DEFAULT_AGENT && requests::tool::is_work_tool(&call.tool) => {
            match app_state(deps) {
                Some(state) => {
                    // In an intake thread: its source, and everything queues
                    let thread = intake::thread::is_thread(&ctx.conversation_id)
                        .then(|| ctx.conversation_id.clone());
                    let thread_source = match &thread {
                        Some(id) => {
                            let id = id.clone();
                            state
                                .db()
                                .call(move |conn| Ok(intake::thread::source(conn, &id)))
                                .await
                                .ok()
                                .flatten()
                        }
                        None => None,
                    };
                    let source =
                        thread_source.unwrap_or_else(|| requests::tool::source_of(data_run));
                    // Doing a step of a plan: what it starts waits for the user
                    let in_self_step = requests::self_step::is_conversation(&ctx.conversation_id)
                        || request_step(deps, &ctx.run_id).await.is_some();
                    requests::tool::call(
                        &state,
                        source,
                        thread,
                        in_self_step,
                        &call.tool,
                        &call.args,
                    )
                    .await
                }
                None => ToolOutcome {
                    result: format!("Tool '{}' is not available right now", call.tool),
                    is_error: true,
                },
            }
        }
        // Nandi's to-do tools: on the host, no approval (like the notes)
        _ if ctx.agent_id == DEFAULT_AGENT && todos::tool::is_todo_tool(&call.tool) => {
            match app_state(deps) {
                // A run for someone else never reaches the user's list
                Some(_) if !todos::tool::allowed_in(run_source(deps, &ctx.run_id).await) => {
                    ToolOutcome {
                        result: todos::tool::REFUSED_FOR_OTHERS.to_string(),
                        is_error: true,
                    }
                }
                Some(state) => {
                    todos::tool::call(&state, &ctx.agent_id, &call.tool, &call.args).await
                }
                None => ToolOutcome {
                    result: format!("Tool '{}' is not available right now", call.tool),
                    is_error: true,
                },
            }
        }
        // Mail (read-only): on the host, for an agent the user let read it
        _ if ctx.agent_id != STUDIO_AGENT_ID && email::tool::is_mail_tool(&call.tool) => {
            match app_state(deps) {
                Some(state) => {
                    let allowed = allowed_connectors.unwrap_or_default();
                    email::tool::call(&state, &allowed, &call.tool, &call.args).await
                }
                None => ToolOutcome {
                    result: format!("Tool '{}' is not available right now", call.tool),
                    is_error: true,
                },
            }
        }
        // Deferred loading: answered here, never in a container
        _ if call.tool == tool_search::TOOL_SEARCH_NAME => tool_search::call(bus, ctx, &call.args),
        // The run plan: on the host, for every agent
        _ if ctx.agent_id != STUDIO_AGENT_ID
            && crate::services::run_plan::tool::is_plan_tool(&call.tool) =>
        {
            crate::services::run_plan::tool::call(bus, ctx, &call.tool, &call.args).await
        }
        // A proposed playbook the user approved: written on the host
        _ if ctx.agent_id != STUDIO_AGENT_ID && playbooks::is_propose(&call.tool) => {
            playbooks::apply(&deps.store, &ctx.agent_id, &call.args)
        }
        // Playbooks (read-only): on the host, from the agent's `skills/`
        _ if ctx.agent_id != STUDIO_AGENT_ID && playbooks::is_playbook_tool(&call.tool) => {
            playbooks::call(&deps.store, &ctx.agent_id, &call.tool, &call.args)
        }
        // Studio's read-only tools: answered on the host, whatever the engine
        _ if ctx.agent_id == STUDIO_AGENT_ID && studio::tools::is_studio_tool(&call.tool) => {
            match app_state(deps) {
                Some(state) => studio::tools::call(&state, &call.tool, &call.args).await,
                None => ToolOutcome {
                    result: format!("Tool '{}' is not available right now", call.tool),
                    is_error: true,
                },
            }
        }
        // A2A: runs on the delegator whatever the engine
        _ if call.tool == a2a::tool::NAME => {
            a2a::tool::call(delegation.get(), &deps.store, &ctx.agent_id, &call.args).await
        }
        // People tools: read the network and the task log on the host
        _ if ctx.agent_id != STUDIO_AGENT_ID && a2a::people::is_people_tool(&call.tool) => {
            match app_state(deps) {
                Some(state) => {
                    a2a::people::call(&state, &deps.store, &ctx.agent_id, &call.tool, &call.args)
                        .await
                }
                None => ToolOutcome {
                    result: format!("Tool '{}' is not available right now", call.tool),
                    is_error: true,
                },
            }
        }
        // Clarification: blocks until the requester answers
        _ if call.tool == a2a::tool::ASK_REQUESTER => {
            a2a::tool::ask_requester(
                delegation.clarifier(),
                &ctx.run_id,
                &ctx.agent_id,
                &call.args,
            )
            .await
        }
        Engine::Canned(canned) => {
            tokio::time::sleep(canned.token_delay() * 4).await;
            canned.execute(call)
        }
        Engine::Llm => execute_tool(deps, ctx, call).await?,
    };
    bus.tool_result(ctx, call, &outcome).await?;
    if let (Some((_, file)), false) = (&screenshot, outcome.is_error) {
        crate::services::run_plan::tool::link_screenshot(bus, ctx, file).await;
    }
    Ok(true)
}

/// `call` (the browser's screenshot) saving to `browser/<run>/<step>-<n>.png`
/// in the agent's work folder, with that name
fn screenshot_call(
    ctx: &RunContext,
    call: &PlannedCall,
    plan: Option<&crate::services::run_plan::RunPlan>,
) -> Option<(PlannedCall, String)> {
    use crate::services::run_plan::screenshots;
    let workdir = crate::utils::nandi_config::work_dir()
        .ok()?
        .join(&ctx.agent_id);
    let step = plan.and_then(|p| p.current()).map_or(0, |(n, _)| n);
    let name = screenshots::next_name(&workdir, &ctx.run_id, step);
    let mut call = call.clone();
    if !call.args.is_object() {
        call.args = serde_json::json!({});
    }
    call.args["filename"] = serde_json::Value::from(name.clone());
    call.args["type"] = serde_json::Value::from("png");
    Some((call, name))
}

/// Executes a model-requested tool in the agent's container.
pub(super) async fn execute_tool(
    deps: &RunDeps,
    ctx: &RunContext,
    call: &PlannedCall,
) -> Result<ToolOutcome, String> {
    let unavailable = |reason: &str| ToolOutcome {
        result: format!("Tool '{}' is not available: {reason}", call.tool),
        is_error: true,
    };
    if ctx.agent_id == STUDIO_AGENT_ID {
        return Ok(unavailable("Studio has no tools"));
    }
    // Not offered, but a model may still name it (e.g. from history)
    let name = call.tool.clone();
    let disabled = deps
        .bus
        .db
        .call(move |conn| Ok(crate::services::tools::disabled_tool_names(conn)?.contains(&name)))
        .await
        .unwrap_or(false);
    if disabled {
        return Ok(unavailable("it's switched off in the tool catalog"));
    }
    // Its skill.md narrowed by its policies; a broken policy stops it here
    let skill = deps.store.skill(&ctx.agent_id)?;
    // Pilots: host-side, governed by skill.md `pilots:` + consent
    if let Some((pilot, _)) = pilots::tools::parse_tool_name(&call.tool) {
        if !skill.pilots.iter().any(|p| p == pilot) {
            crate::services::policies::record_refused(
                &deps.store,
                &ctx.agent_id,
                &call.id,
                &format!("pilot {pilot}"),
            );
            return Ok(unavailable("a policy doesn't allow this pilot"));
        }
        let agent = deps.store.agent(&ctx.agent_id)?;
        return Ok(pilots::tools::call(&deps.bus, &agent, &call.id, &call.tool, &call.args).await);
    }
    if let Some(outcome) = connector_call(connector_registry(deps), deps, ctx, call).await {
        return Ok(outcome);
    }
    if !skill.allows_tool(&call.tool) {
        return Ok(unavailable(
            "not allowed by this agent's skill.md or its policies",
        ));
    }
    let runtime = deps
        .runtimes
        .get_or_start(&skill)
        .await
        .map_err(|e| format!("Couldn't start the agent's runtime: {e}"))?;
    if !runtime.tools.iter().any(|t| t.name == call.tool) {
        return Ok(unavailable("the runtime doesn't provide it"));
    }
    match runtime.mcp.call_tool(&call.tool, &call.args).await {
        Ok(outcome) => Ok(ToolOutcome {
            result: outcome.content,
            is_error: outcome.is_error,
        }),
        Err(e) => {
            // The session is broken; start fresh next time
            deps.runtimes.invalidate(&ctx.agent_id).await;
            Err(e.0)
        }
    }
}
