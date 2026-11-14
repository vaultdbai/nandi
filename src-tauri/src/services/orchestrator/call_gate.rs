//! The checks a planned tool call passes when it is about to run.
//!
//! [`CallGate::decide`] runs them in this order and stops at the first that
//! doesn't pass:
//!
//! 1. **Policy**: a tool the agent's policies deny (`tools.deny`) is refused
//!    before anything else, whatever the engine. Nobody is asked.
//! 2. **Middleware**: plan mode (or another middleware) blocks the tool.
//! 3. **Run plan**: plan first, risky steps are the user's, no screenshots
//!    while the person has the browser.
//! 4. **Access**: a connector the agent hasn't been granted. The person
//!    chooses first (`services::access`); grants are read on every call.
//! 5. **Playbook**: a proposed playbook breaks the domain rules, or the turn
//!    may not propose one.
//! 6. **Approval**: a call planned as needing approval waits for the
//!    person's yes, and that yes must be for this call's tool.
//!
//! Whether a call needs approval is decided when the model plans it
//! (`PlannedCall::requires_approval`), from the skill, the turn's pilot
//! tools and the delegation policy, and is stored with the step so a run
//! resumed after a restart sees the same decision. The gate reads that flag.
//!
//! The checks themselves are a [`CallChecks`]: [`LiveChecks`] reads them
//! from the run, and the tests set them by hand.

use crate::db::{self, PlannedCall};
use crate::engine::ToolOutcome;
use crate::model::{ApprovalStatus, STUDIO_AGENT_ID};
use crate::services::agents::SkillConfig;
use crate::services::events::RunContext;
use crate::services::{a2a, pilots, playbooks};

use super::middleware::{self, CallCheck, TurnMiddleware};
use super::{app_state, data_run, tool_search, RunDeps};

/// What the gate decided for one call
#[derive(Debug)]
pub(super) enum Verdict {
    /// Run it, with what the checks found
    Run(Clearance),
    /// Don't run it: the reason is recorded as the call's error result, for
    /// the model to read
    Refuse(String),
    /// A person has been asked (access or approval): the run suspends
    Wait,
    /// The person said no to its approval, or the approval expired. Nothing
    /// is recorded for the call: the model is told when it plans again
    Declined,
}

/// What a call that may run carries from its checks
#[derive(Debug)]
pub(super) struct Clearance {
    /// For account connectors, the ones this call may use
    pub connectors: Option<Vec<String>>,
    /// What the run plan's rules looked at
    pub plan: PlanCheck,
}

/// What the access check says about one call
#[derive(Debug, PartialEq)]
pub(super) enum AccessGate {
    /// Go ahead; for account connectors, the ones this call may use
    Go(Option<Vec<String>>),
    /// An access request is open
    Wait,
    /// Refused (a policy, or the person said no); the model reads why
    Refused(String),
}

/// One method per check, in the gate's order. A method that finds nothing
/// against the call returns `None` (or `Go`).
pub(super) trait CallChecks {
    /// 1. The refusal, if the agent's policies deny the tool
    fn policy_denial(&self, call: &PlannedCall) -> Option<String>;
    /// 2. Why a middleware blocks the tool outright
    fn middleware_block(&self, call: &PlannedCall) -> Option<String>;
    /// 3. What the run plan's rules look at, and why they block the call
    async fn plan(&self, call: &PlannedCall) -> (PlanCheck, Option<String>);
    /// 4. Whether the agent may use the connectors the call reaches
    async fn access(&self, call: &PlannedCall) -> Result<AccessGate, String>;
    /// 5. Why a proposed playbook is refused
    async fn playbook_refusal(&self, call: &PlannedCall) -> Option<String>;
    /// 6. The approval found for the call's run and call ID
    async fn approval(&self, call: &PlannedCall) -> Result<Option<db::ApprovalRow>, String>;
    /// Asks the person to approve the call
    async fn request_approval(&self, call: &PlannedCall) -> Result<(), String>;
}

/// Runs a call's checks in order (see the module docs)
pub(super) struct CallGate<C> {
    checks: C,
}

impl<C: CallChecks> CallGate<C> {
    pub fn new(checks: C) -> Self {
        Self { checks }
    }

    pub async fn decide(&self, call: &PlannedCall) -> Result<Verdict, String> {
        if let Some(reason) = self.checks.policy_denial(call) {
            return Ok(Verdict::Refuse(reason));
        }
        if let Some(reason) = self.checks.middleware_block(call) {
            return Ok(Verdict::Refuse(reason));
        }
        let (plan, blocked) = self.checks.plan(call).await;
        if let Some(reason) = blocked {
            return Ok(Verdict::Refuse(reason));
        }
        let connectors = match self.checks.access(call).await? {
            AccessGate::Go(connectors) => connectors,
            AccessGate::Wait => return Ok(Verdict::Wait),
            AccessGate::Refused(reason) => return Ok(Verdict::Refuse(reason)),
        };
        if let Some(reason) = self.checks.playbook_refusal(call).await {
            return Ok(Verdict::Refuse(reason));
        }
        if needs_approval(call) {
            let approval = self.checks.approval(call).await?;
            match approval_status(approval.as_ref(), call) {
                None => {
                    self.checks.request_approval(call).await?;
                    return Ok(Verdict::Wait);
                }
                Some(ApprovalStatus::Pending) => return Ok(Verdict::Wait),
                Some(ApprovalStatus::Approved) => {}
                Some(ApprovalStatus::Rejected | ApprovalStatus::Expired) => {
                    return Ok(Verdict::Declined)
                }
            }
        }
        Ok(Verdict::Run(Clearance { connectors, plan }))
    }
}

/// Whether `call` waits for a person's yes. `tool_search` only reads the
/// tool list, and `ask_requester` is decided on the requester's side: never
/// gated.
fn needs_approval(call: &PlannedCall) -> bool {
    call.requires_approval
        && call.tool != tool_search::TOOL_SEARCH_NAME
        && call.tool != a2a::tool::ASK_REQUESTER
}

/// Where `call` stands with `approval`, the one found for its run and call
/// ID. An approval is for one tool: a call reusing the ID for another tool
/// is refused, never run on someone else's yes.
pub(super) fn approval_status(
    approval: Option<&db::ApprovalRow>,
    call: &PlannedCall,
) -> Option<ApprovalStatus> {
    approval.map(|approval| {
        if approval.tool == call.tool {
            approval.status
        } else {
            log::warn!(
                "Call {} asks for '{}' but its approval was for '{}': refused",
                call.id,
                call.tool,
                approval.tool
            );
            ApprovalStatus::Rejected
        }
    })
}

// ---------------------------------------------------------------------------
// The checks, read from the run
// ---------------------------------------------------------------------------

/// The checks of a call in a live run
pub(super) struct LiveChecks<'a> {
    pub deps: &'a RunDeps,
    pub ctx: &'a RunContext,
    pub pipeline: &'a [Box<dyn TurnMiddleware>],
}

impl CallChecks for LiveChecks<'_> {
    /// A broken policy stops the call later, in `execute_tool`
    fn policy_denial(&self, call: &PlannedCall) -> Option<String> {
        if self.ctx.agent_id == STUDIO_AGENT_ID {
            return None;
        }
        let skill = self.deps.store.skill(&self.ctx.agent_id).ok()?;
        denied_by_policy(self.deps, self.ctx, call, &skill).map(|outcome| outcome.result)
    }

    fn middleware_block(&self, call: &PlannedCall) -> Option<String> {
        middleware::blocking_middleware(self.pipeline, &call.tool)
            .map(|name| middleware::blocked_tool_result(name, &call.tool))
    }

    async fn plan(&self, call: &PlannedCall) -> (PlanCheck, Option<String>) {
        let check = plan_check(self.deps, self.ctx, call).await;
        let view = CallCheck {
            tool: &call.tool,
            effect: check.effect,
            screenshot: check.screenshot,
            plan: check.plan.as_ref(),
        };
        let blocked = middleware::blocking_call(self.pipeline, &view).map(|(_, why)| why);
        (check, blocked)
    }

    async fn access(&self, call: &PlannedCall) -> Result<AccessGate, String> {
        if self.ctx.agent_id == STUDIO_AGENT_ID {
            return Ok(AccessGate::Go(None));
        }
        match app_state(self.deps) {
            Some(state) => access_gate(&state, self.deps, self.ctx, call).await,
            None => Ok(AccessGate::Go(None)),
        }
    }

    /// The domain rules, an agent made here, never in a check-started run
    async fn playbook_refusal(&self, call: &PlannedCall) -> Option<String> {
        if !playbooks::is_propose(&call.tool) {
            return None;
        }
        let (deps, ctx) = (self.deps, self.ctx);
        if ctx.agent_id == STUDIO_AGENT_ID || data_run(deps, &ctx.run_id).await.is_some() {
            Some(format!(
                "Tool '{}' is not available in this turn",
                call.tool
            ))
        } else {
            playbooks::check(&deps.store, &ctx.agent_id, &call.args).err()
        }
    }

    async fn approval(&self, call: &PlannedCall) -> Result<Option<db::ApprovalRow>, String> {
        let (run_id, call_id) = (self.ctx.run_id.clone(), call.id.clone());
        self.deps
            .bus
            .db
            .call(move |conn| db::approval_for_call(conn, &run_id, &call_id))
            .await
    }

    async fn request_approval(&self, call: &PlannedCall) -> Result<(), String> {
        self.deps.bus.approval_request(self.ctx, call).await?;
        // The person finishes a hand-over in the browser: show it
        if pilots::handover::is_hand_over(&call.tool) {
            pilots::handover::bring_forward(&self.ctx.agent_id).await;
        }
        Ok(())
    }
}

async fn access_gate(
    state: &crate::services::AppState,
    deps: &RunDeps,
    ctx: &RunContext,
    call: &PlannedCall,
) -> Result<AccessGate, String> {
    use crate::services::access::{self, Decision};
    let skill = deps.store.skill(&ctx.agent_id).ok();
    let Some(decision) = access::decide(
        state,
        skill.as_ref(),
        &ctx.agent_id,
        &ctx.run_id,
        &call.tool,
        &call.args,
    )
    .await
    else {
        return Ok(AccessGate::Go(None));
    };
    match decision {
        Decision::Allowed(ids) => Ok(AccessGate::Go(Some(ids))),
        Decision::Refused(reason) => Ok(AccessGate::Refused(reason)),
        Decision::Ask(request) => {
            let ids: Vec<String> = request.connectors.iter().map(|c| c.id.clone()).collect();
            let access_call = PlannedCall {
                id: format!("{}#access", call.id),
                tool: access::ACCESS_TOOL.to_string(),
                args: serde_json::to_value(&request).map_err(|e| e.to_string())?,
                requires_approval: true,
            };
            let (run_id, call_id) = (ctx.run_id.clone(), access_call.id.clone());
            let approval = deps
                .bus
                .db
                .call(move |conn| db::approval_for_call(conn, &run_id, &call_id))
                .await?;
            match approval.map(|a| a.status) {
                None => {
                    deps.bus.approval_request(ctx, &access_call).await?;
                    Ok(AccessGate::Wait)
                }
                Some(ApprovalStatus::Pending) => Ok(AccessGate::Wait),
                Some(ApprovalStatus::Approved) => Ok(AccessGate::Go(Some(ids))),
                Some(_) => Ok(AccessGate::Refused(format!(
                    "Tool '{}' is not allowed: the person didn't allow it",
                    call.tool
                ))),
            }
        }
    }
}

/// What the run plan's rules look at for one call
#[derive(Debug)]
pub(super) struct PlanCheck {
    pub effect: crate::services::run_plan::classify::Effect,
    pub screenshot: bool,
    pub plan: Option<crate::services::run_plan::RunPlan>,
}

/// Classifies `call` and reads the run's plan (Studio has neither)
async fn plan_check(deps: &RunDeps, ctx: &RunContext, call: &PlannedCall) -> PlanCheck {
    use crate::services::run_plan::classify::{self, Effect};
    if ctx.agent_id == STUDIO_AGENT_ID {
        return PlanCheck {
            effect: Effect::None,
            screenshot: false,
            plan: None,
        };
    }
    let browser_tool = pilots::tools::parse_tool_name(&call.tool)
        .filter(|(pilot, _)| pilots::browser::is_browser(pilot));
    let action = browser_tool.map(|(pilot, tool)| {
        let secret_names: Vec<String> = deps
            .store
            .agent(&ctx.agent_id)
            .ok()
            .and_then(|agent| {
                agent
                    .capabilities()
                    .pilot_options(pilot)
                    .map(|o| o.secrets.keys().cloned().collect())
            })
            .unwrap_or_default();
        pilots::browser::action(
            tool,
            &call.args,
            pilots::browser::is_read_tool(tool),
            &secret_names,
        )
    });
    PlanCheck {
        effect: classify::effect(&call.tool, &call.args, action),
        screenshot: browser_tool.is_some_and(|(_, tool)| tool == pilots::browser::SCREENSHOT_TOOL),
        plan: crate::services::run_plan::tool::current(&deps.bus, &ctx.run_id).await,
    }
}

/// A call to a tool its policies deny (`tools.deny`), recorded as the
/// policy's decision. The gate's first check, whatever the engine, so the
/// canned one obeys policies too.
pub(super) fn denied_by_policy(
    deps: &RunDeps,
    ctx: &RunContext,
    call: &PlannedCall,
    skill: &SkillConfig,
) -> Option<ToolOutcome> {
    if !skill.denied_tools.contains(&call.tool) {
        return None;
    }
    crate::services::policies::record_refused(
        &deps.store,
        &ctx.agent_id,
        &call.id,
        &format!("tool {}", call.tool),
    );
    Some(ToolOutcome {
        result: format!(
            "Tool '{}' is not available: a policy doesn't allow it",
            call.tool
        ),
        is_error: true,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::db::ApprovalRow;
    use crate::services::run_plan::classify::Effect;

    fn call(tool: &str, requires_approval: bool) -> PlannedCall {
        PlannedCall {
            id: "call_0".into(),
            tool: tool.into(),
            args: serde_json::json!({}),
            requires_approval,
        }
    }

    fn approval(tool: &str, status: ApprovalStatus) -> ApprovalRow {
        ApprovalRow {
            id: "ap1".into(),
            agent_id: "a".into(),
            run_id: "r1".into(),
            message_id: "ap1".into(),
            tool_call_id: "call_0".into(),
            tool: tool.into(),
            args: serde_json::json!({}),
            status,
            created_at: 0,
            expires_at: None,
            decided_by: None,
        }
    }

    /// Checks set by hand. Records which checks the gate read, in order.
    #[derive(Default)]
    struct Fake {
        policy: Option<&'static str>,
        middleware: Option<&'static str>,
        plan: Option<&'static str>,
        access: Option<AccessGate>,
        playbook: Option<&'static str>,
        approval: Option<ApprovalStatus>,
        approval_tool: Option<&'static str>,
        read: RefCell<Vec<&'static str>>,
    }

    impl Fake {
        fn read(&self, check: &'static str) {
            self.read.borrow_mut().push(check);
        }
    }

    impl CallChecks for Fake {
        fn policy_denial(&self, _: &PlannedCall) -> Option<String> {
            self.read("policy");
            self.policy.map(str::to_string)
        }

        fn middleware_block(&self, _: &PlannedCall) -> Option<String> {
            self.read("middleware");
            self.middleware.map(str::to_string)
        }

        async fn plan(&self, _: &PlannedCall) -> (PlanCheck, Option<String>) {
            self.read("plan");
            let check = PlanCheck {
                effect: Effect::None,
                screenshot: false,
                plan: None,
            };
            (check, self.plan.map(str::to_string))
        }

        async fn access(&self, _: &PlannedCall) -> Result<AccessGate, String> {
            self.read("access");
            Ok(match &self.access {
                None => AccessGate::Go(None),
                Some(AccessGate::Go(ids)) => AccessGate::Go(ids.clone()),
                Some(AccessGate::Wait) => AccessGate::Wait,
                Some(AccessGate::Refused(why)) => AccessGate::Refused(why.clone()),
            })
        }

        async fn playbook_refusal(&self, _: &PlannedCall) -> Option<String> {
            self.read("playbook");
            self.playbook.map(str::to_string)
        }

        async fn approval(&self, call: &PlannedCall) -> Result<Option<ApprovalRow>, String> {
            self.read("approval");
            let tool = self.approval_tool.unwrap_or(&call.tool);
            Ok(self.approval.map(|status| approval(tool, status)))
        }

        async fn request_approval(&self, _: &PlannedCall) -> Result<(), String> {
            self.read("ask");
            Ok(())
        }
    }

    /// The verdict in short, for the table
    fn short(verdict: &Verdict) -> String {
        match verdict {
            Verdict::Run(clearance) => match &clearance.connectors {
                Some(ids) => format!("run with {}", ids.join(",")),
                None => "run".into(),
            },
            Verdict::Refuse(reason) => format!("refuse: {reason}"),
            Verdict::Wait => "wait".into(),
            Verdict::Declined => "declined".into(),
        }
    }

    const UP_TO_PLAYBOOK: &[&str] = &["policy", "middleware", "plan", "access", "playbook"];
    const UP_TO_APPROVAL: &[&str] = &[
        "policy",
        "middleware",
        "plan",
        "access",
        "playbook",
        "approval",
    ];

    #[tokio::test]
    async fn the_gate_stops_at_the_first_check_that_fails_in_order() {
        // (case, the checks, needs approval, verdict, the checks read)
        let cases: Vec<(&str, Fake, bool, &str, &[&str])> = vec![
            (
                "nothing against it",
                Fake::default(),
                false,
                "run",
                UP_TO_PLAYBOOK,
            ),
            (
                "a policy denies it, even when every other check would refuse too",
                Fake {
                    policy: Some("policy"),
                    middleware: Some("plan mode"),
                    plan: Some("plan first"),
                    access: Some(AccessGate::Refused("no access".into())),
                    playbook: Some("bad playbook"),
                    approval: Some(ApprovalStatus::Approved),
                    ..Fake::default()
                },
                true,
                "refuse: policy",
                &["policy"],
            ),
            (
                "plan mode blocks it before the run plan is read",
                Fake {
                    middleware: Some("plan mode"),
                    plan: Some("plan first"),
                    ..Fake::default()
                },
                true,
                "refuse: plan mode",
                &["policy", "middleware"],
            ),
            (
                "the run plan blocks it before access is decided",
                Fake {
                    plan: Some("plan first"),
                    access: Some(AccessGate::Wait),
                    ..Fake::default()
                },
                true,
                "refuse: plan first",
                &["policy", "middleware", "plan"],
            ),
            (
                "access refused",
                Fake {
                    access: Some(AccessGate::Refused("no access".into())),
                    playbook: Some("bad playbook"),
                    ..Fake::default()
                },
                true,
                "refuse: no access",
                &["policy", "middleware", "plan", "access"],
            ),
            (
                "an open access request waits, without asking for approval too",
                Fake {
                    access: Some(AccessGate::Wait),
                    ..Fake::default()
                },
                true,
                "wait",
                &["policy", "middleware", "plan", "access"],
            ),
            (
                "a bad playbook is refused before the person is asked",
                Fake {
                    playbook: Some("bad playbook"),
                    ..Fake::default()
                },
                true,
                "refuse: bad playbook",
                UP_TO_PLAYBOOK,
            ),
            (
                "no approval yet: the person is asked",
                Fake::default(),
                true,
                "wait",
                &[
                    "policy",
                    "middleware",
                    "plan",
                    "access",
                    "playbook",
                    "approval",
                    "ask",
                ],
            ),
            (
                "a pending approval waits without asking again",
                Fake {
                    approval: Some(ApprovalStatus::Pending),
                    ..Fake::default()
                },
                true,
                "wait",
                UP_TO_APPROVAL,
            ),
            (
                "approved, with the connectors access allowed",
                Fake {
                    access: Some(AccessGate::Go(Some(vec!["google-drive".into()]))),
                    approval: Some(ApprovalStatus::Approved),
                    ..Fake::default()
                },
                true,
                "run with google-drive",
                UP_TO_APPROVAL,
            ),
            (
                "rejected",
                Fake {
                    approval: Some(ApprovalStatus::Rejected),
                    ..Fake::default()
                },
                true,
                "declined",
                UP_TO_APPROVAL,
            ),
            (
                "expired",
                Fake {
                    approval: Some(ApprovalStatus::Expired),
                    ..Fake::default()
                },
                true,
                "declined",
                UP_TO_APPROVAL,
            ),
            (
                "a yes for another tool is never this call's yes",
                Fake {
                    approval: Some(ApprovalStatus::Approved),
                    approval_tool: Some("read_file"),
                    ..Fake::default()
                },
                true,
                "declined",
                UP_TO_APPROVAL,
            ),
        ];
        for (case, fake, requires_approval, verdict, read) in cases {
            let gate = CallGate::new(fake);
            let decided = gate
                .decide(&call("shell", requires_approval))
                .await
                .unwrap();
            assert_eq!(short(&decided), verdict, "{case}");
            assert_eq!(*gate.checks.read.borrow(), read, "{case}");
        }
    }

    #[tokio::test]
    async fn tool_search_and_ask_requester_are_never_held_for_approval() {
        for tool in [tool_search::TOOL_SEARCH_NAME, a2a::tool::ASK_REQUESTER] {
            let gate = CallGate::new(Fake::default());
            let decided = gate.decide(&call(tool, true)).await.unwrap();
            assert_eq!(short(&decided), "run", "{tool}");
            assert_eq!(*gate.checks.read.borrow(), UP_TO_PLAYBOOK, "{tool}");
        }
    }

    #[test]
    fn an_approval_only_lets_the_tool_it_was_for_run() {
        let call = PlannedCall {
            args: serde_json::json!({"cmd": "rm -rf ."}),
            ..call("shell", true)
        };
        // (the approval found for the call, where the call stands)
        let cases = [
            (None, None),
            (
                Some(approval("shell", ApprovalStatus::Approved)),
                Some(ApprovalStatus::Approved),
            ),
            (
                Some(approval("shell", ApprovalStatus::Pending)),
                Some(ApprovalStatus::Pending),
            ),
            (
                Some(approval("read_file", ApprovalStatus::Approved)),
                Some(ApprovalStatus::Rejected),
            ),
        ];
        for (found, expected) in cases {
            assert_eq!(
                approval_status(found.as_ref(), &call),
                expected,
                "{found:?}"
            );
        }
    }
}
