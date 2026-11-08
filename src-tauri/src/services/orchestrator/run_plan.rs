//! The run plan's two middlewares (task "Run plans", step 1). Both are on
//! for every agent unless skill.md `middlewares:` leaves them out, like
//! verify-on-stop, and neither applies to a run the email or calendar check
//! started (those have no plan tools; `execute_run` drops them).
//!
//! - **Plan first** (`plan_first`): a call that changes something
//!   (`run_plan::classify`) is blocked with "make a plan first" while the
//!   run has no plan; a risky one (submit, pay, secret, hand-over, start
//!   work, delegate) also needs a `doing` step of a kind that covers it.
//!   A hand-over step refuses screenshots. Existing approvals apply on top.
//! - **Finish check** (`finish_check`): when a step would end the run, and
//!   steps are still open, a `screenshot` step has no evidence, or the
//!   agent's browser still has pages open, the run gets one continuation to
//!   finish, record evidence, close the browser and report each step. It
//!   fires at most once per run, also across an approval's suspension (the
//!   continuation is found in the run's messages).

use futures_util::future::BoxFuture;

use super::middleware::{
    CallCheck, ExitReason, MiddlewareDecision, MiddlewareError, TurnContext, TurnMiddleware,
    TurnResult, FINISH_CHECK, PLAN_FIRST,
};
use super::RunDeps;
use crate::services::pilots::browser;
use crate::services::run_plan::{self, classify, RunPlan};

/// How the finish check's continuation starts: found again in the run's
/// messages, so it fires once per run
pub const FINISH_CHECK_LEAD: &str = "Before you finish, close out your run plan.";

#[derive(Debug, Default)]
pub struct PlanFirstMiddleware {}

impl TurnMiddleware for PlanFirstMiddleware {
    fn name(&self) -> &'static str {
        PLAN_FIRST
    }

    fn pre_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        Box::pin(async { Ok(MiddlewareDecision::Continue) })
    }

    fn post_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
        _result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        Box::pin(async { Ok(()) })
    }

    fn blocks_call(&self, call: &CallCheck<'_>) -> Option<String> {
        classify::check_call(call.plan, call.tool, call.effect, call.screenshot)
    }
}

/// What the finish check found open at the end of a run
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Unfinished {
    pub open_steps: Vec<usize>,
    pub missing_evidence: Vec<usize>,
    pub browser_open: bool,
}

impl Unfinished {
    pub fn of(plan: Option<&RunPlan>, browser_open: bool) -> Self {
        Self {
            open_steps: plan.map(RunPlan::open_steps).unwrap_or_default(),
            missing_evidence: plan.map(RunPlan::missing_evidence).unwrap_or_default(),
            browser_open,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.open_steps.is_empty() && self.missing_evidence.is_empty() && !self.browser_open
    }
}

/// "Step 3 is" / "Steps 3, 4 are"
fn steps_are(numbers: &[usize], one: &str, many: &str) -> String {
    let list = numbers
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if numbers.len() == 1 {
        format!("Step {list} {one}")
    } else {
        format!("Steps {list} {many}")
    }
}

/// The continuation the finish check sends
pub fn finish_prompt(unfinished: &Unfinished) -> String {
    let mut text = FINISH_CHECK_LEAD.to_string();
    if !unfinished.open_steps.is_empty() {
        text.push_str(&format!(
            " {} still open: finish, or mark skipped or failed with the reason (`plan_update`).",
            steps_are(&unfinished.open_steps, "is", "are")
        ));
    }
    if !unfinished.missing_evidence.is_empty() {
        text.push_str(&format!(
            " {} a screenshot: take one if the page still shows the result, and link it.",
            steps_are(&unfinished.missing_evidence, "needs", "need")
        ));
    }
    if unfinished.browser_open {
        text.push_str(" Your browser still has pages open: close it (`browser_close`).");
    }
    text.push_str(
        " Then end with a short report: one line per step, with its result and evidence, and what's left for the user.",
    );
    text
}

pub struct FinishCheckMiddleware {
    pub deps: RunDeps,
}

impl FinishCheckMiddleware {
    pub fn new(deps: RunDeps) -> Self {
        Self { deps }
    }

    async fn check(&self, ctx: &mut TurnContext, result: &mut TurnResult) {
        if ctx.finish_check_fired || result.exit_reason != ExitReason::Completed {
            return;
        }
        let plan = run_plan::tool::current(&self.deps.bus, &ctx.run_id).await;
        let unfinished = Unfinished::of(plan.as_ref(), browser::has_open_pages(&ctx.agent_id));
        if unfinished.is_empty() {
            return;
        }
        log::info!(
            "Finish check fired for run {} (turn {}): {unfinished:?}",
            ctx.run_id,
            ctx.turn_number
        );
        ctx.finish_check_fired = true;
        // Another middleware's continuation (verify-on-stop) goes first;
        // both go in one message
        result.continue_with = Some(match result.continue_with.take() {
            Some(other) => format!("{}\n\n{other}", finish_prompt(&unfinished)),
            None => finish_prompt(&unfinished),
        });
    }
}

impl TurnMiddleware for FinishCheckMiddleware {
    fn name(&self) -> &'static str {
        FINISH_CHECK
    }

    fn pre_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        Box::pin(async { Ok(MiddlewareDecision::Continue) })
    }

    fn post_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
        result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        Box::pin(async move {
            self.check(ctx, result).await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_what_is_left() {
        let text = finish_prompt(&Unfinished {
            open_steps: vec![3, 4],
            missing_evidence: vec![1],
            browser_open: true,
        });
        assert!(text.starts_with(FINISH_CHECK_LEAD), "{text}");
        assert!(text.contains("Steps 3, 4 are still open"), "{text}");
        assert!(text.contains("Step 1 needs a screenshot"), "{text}");
        assert!(text.contains("close it (`browser_close`)"), "{text}");
        assert!(text.contains("one line per step"), "{text}");
        let text = finish_prompt(&Unfinished {
            browser_open: true,
            ..Unfinished::default()
        });
        assert!(!text.contains("still open:"), "{text}");
    }

    #[test]
    fn nothing_is_unfinished_without_a_plan_or_a_browser() {
        assert!(Unfinished::of(None, false).is_empty());
        assert!(!Unfinished::of(None, true).is_empty());
    }
}
