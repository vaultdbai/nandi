//! Why a Request is stuck and who has to act (work-model §2.3), from the
//! task's state, its typed `reason_kind`, and its child run. A transition
//! without a kind (e.g. a provider's result reported as a failure) falls
//! back to its reason text, read only here. The UI never shows reason text
//! as the main message. Pure.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::RunStatus;
use crate::services::a2a::task::{ReasonKind, TaskState};

/// work-model §2.3
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BlockerKind {
    NeedsMyApproval,
    NeedsTheirApproval,
    NeedsMyAnswer,
    NeedsTheirAnswer,
    NotTrusted,
    NotAccepting,
    PausedOrBlocked,
    Queued,
    Offline,
    NoModel,
    RuntimeAuth,
    TimedOut,
    Failed,
    /// The provider's owner said no to a held request (task 5)
    Declined,
}

/// Who has to act: me, the other side, or nobody (it's waiting on time)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BlockerWho {
    Me,
    Them,
    Nobody,
}

/// Which side of the task this Nandi is on
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// Both ends on this Nandi
    Local,
    /// Another Nandi asked this one
    Incoming,
    /// This Nandi asked another
    Outgoing,
}

/// What the mapping looks at
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockerInput<'a> {
    pub direction: Option<Direction>,
    /// The task's state (the latest attempt's, for a failover chain)
    pub state: Option<TaskState>,
    /// The latest event's reason or the task's error
    pub reason: Option<&'a str>,
    /// The child agent run, when one ran
    pub run_status: Option<RunStatus>,
    /// Its summary: the error, once failed
    pub run_summary: Option<&'a str>,
    /// A workflow step of this Request waits for approval
    pub workflow_step_awaiting_approval: bool,
    /// A question for the human is pending on this task
    pub pending_clarification: bool,
    /// The provider is offline (outgoing, still submitted)
    pub provider_offline: bool,
    /// Why, as the side that decided recorded it: a waiting note on the
    /// task's current state, or the kind its last transition carries.
    /// `None` when the transition carries none (the reason text is read
    /// instead).
    pub reason_kind: Option<ReasonKind>,
}

/// One row of the table
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockerMatch {
    pub kind: BlockerKind,
    pub who: BlockerWho,
    /// Nothing will move it on by itself (it ended)
    pub terminal: bool,
}

const fn row(kind: BlockerKind, who: BlockerWho, terminal: bool) -> Option<BlockerMatch> {
    Some(BlockerMatch {
        kind,
        who,
        terminal,
    })
}

fn has(text: &str, needles: &[&str]) -> bool {
    let text = text.to_ascii_lowercase();
    needles.iter().any(|n| text.contains(n))
}

/// The executor's and gateway's trust refusals (`executor.rs::admit`)
fn is_trust(reason: &str) -> bool {
    has(reason, &["requires trust", "trust policy", "trusted peers"])
}

/// Paused or blocked requesters (`executor.rs::check_member_trust`,
/// `gateway/policy.rs`)
fn is_paused_or_blocked(reason: &str) -> bool {
    has(reason, &["are paused", "is blocked", "are turned off"])
}

/// An agent that doesn't take requests or this capability
fn is_not_accepting(reason: &str) -> bool {
    has(
        reason,
        &[
            "does not accept external delegation",
            "doesn't take",
            "doesn't serve",
            "isn't offered",
        ],
    )
}

/// The gateway's "needs the owner's approval" refusal (`router.rs`)
fn is_needs_owner_approval(reason: &str) -> bool {
    has(reason, &["needs the owner's approval"])
}

/// The no-key errors of `llm_keys` and the gateway router
fn is_no_model(text: &str) -> bool {
    has(
        text,
        &[
            "no llm api key",
            "api key not configured",
            "add an api key in settings",
            "settings › models",
        ],
    )
}

/// A CLI runtime that couldn't sign in (harness auth errors)
fn is_runtime_auth(text: &str) -> bool {
    has(
        text,
        &[
            "not logged in",
            "couldn't sign in",
            "authentication failed",
            "invalid api key",
            "requires api key",
            "please run /login",
        ],
    )
}

/// The Blocker a typed reason means, from this side of the task. `who` is
/// the side that has to act; a refusal's fix is on the provider's side
/// (`them` for the requester, `me` for the provider).
pub fn from_reason_kind(kind: ReasonKind, outgoing: bool, terminal: bool) -> BlockerMatch {
    use BlockerWho::*;
    let provider = if outgoing { Them } else { Me };
    let (kind, who) = match kind {
        ReasonKind::NeedsMyApproval => (BlockerKind::NeedsMyApproval, Me),
        ReasonKind::NeedsTheirApproval => (BlockerKind::NeedsTheirApproval, Them),
        ReasonKind::NeedsMyAnswer => (BlockerKind::NeedsMyAnswer, Me),
        ReasonKind::NeedsTheirAnswer => (BlockerKind::NeedsTheirAnswer, Them),
        ReasonKind::NotTrusted => (BlockerKind::NotTrusted, provider),
        ReasonKind::NotAccepting => (BlockerKind::NotAccepting, provider),
        ReasonKind::PausedOrBlocked => (BlockerKind::PausedOrBlocked, provider),
        ReasonKind::Queued => (BlockerKind::Queued, Nobody),
        ReasonKind::Offline => (BlockerKind::Offline, Nobody),
        ReasonKind::NoModel => (BlockerKind::NoModel, provider),
        ReasonKind::RuntimeAuth => (BlockerKind::RuntimeAuth, provider),
        ReasonKind::TimedOut => (BlockerKind::TimedOut, Me),
        ReasonKind::Failed => (BlockerKind::Failed, Me),
        ReasonKind::Declined => (BlockerKind::Declined, provider),
    };
    BlockerMatch {
        kind,
        who,
        terminal,
    }
}

/// The Blocker for a Request's state, or `None` while it moves on its own
/// (or ended well).
pub fn blocker(input: &BlockerInput<'_>) -> Option<BlockerMatch> {
    use BlockerKind::*;
    use BlockerWho::*;
    let reason = input.reason.unwrap_or_default();
    let outgoing = input.direction == Some(Direction::Outgoing);

    // Something here waits on the user
    if input.run_status == Some(RunStatus::AwaitingApproval)
        || input.workflow_step_awaiting_approval
    {
        return row(NeedsMyApproval, Me, false);
    }
    // The child run failed for a reason the user can fix here
    if input.run_status == Some(RunStatus::Failed) {
        let error = input.run_summary.unwrap_or_default();
        if is_no_model(error) {
            return row(NoModel, Me, true);
        }
        if is_runtime_auth(error) {
            return row(RuntimeAuth, Me, true);
        }
    }

    let state = input.state?;
    // Typed (task 5): the side that decided said why
    if let Some(kind) = input.reason_kind {
        let terminal = state.is_terminal();
        // A task that ended well, or moves on its own, has no blocker
        if state == TaskState::Completed
            || state == TaskState::Cancelled
            || (!terminal
                && !matches!(
                    kind,
                    ReasonKind::NeedsMyApproval
                        | ReasonKind::NeedsTheirApproval
                        | ReasonKind::Queued
                        | ReasonKind::Offline
                        | ReasonKind::NeedsMyAnswer
                        | ReasonKind::NeedsTheirAnswer
                ))
        {
            return None;
        }
        return Some(from_reason_kind(kind, outgoing, terminal));
    }

    match state {
        TaskState::InputRequired if outgoing && input.pending_clarification => {
            row(NeedsMyAnswer, Me, false)
        }
        // My agent is answering the provider's question by itself
        TaskState::InputRequired if outgoing => None,
        TaskState::InputRequired => row(NeedsTheirAnswer, Them, false),
        TaskState::Submitted if outgoing && input.provider_offline => row(Offline, Nobody, false),
        TaskState::Rejected => {
            let who = if outgoing { Them } else { Me };
            if is_trust(reason) {
                row(NotTrusted, who, true)
            } else if is_paused_or_blocked(reason) {
                // Only the provider side can resume; the requester just sees it
                row(PausedOrBlocked, who, true)
            } else if is_not_accepting(reason) {
                row(NotAccepting, who, true)
            } else if outgoing && is_needs_owner_approval(reason) {
                // Phase 1: the provider refuses; the honest message until holds
                row(NeedsTheirApproval, Them, true)
            } else if is_no_model(reason) {
                row(NoModel, if outgoing { Them } else { Me }, true)
            } else {
                row(Failed, Me, true)
            }
        }
        TaskState::TimedOut => row(TimedOut, Me, true),
        TaskState::Failed | TaskState::Abandoned => {
            if is_no_model(reason) && !outgoing {
                row(NoModel, Me, true)
            } else {
                row(Failed, Me, true)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlockerKind::*;
    use BlockerWho::*;

    fn input(direction: Direction, state: TaskState, reason: &str) -> BlockerInput<'_> {
        BlockerInput {
            direction: Some(direction),
            state: Some(state),
            reason: Some(reason),
            ..Default::default()
        }
    }

    fn expect(input: BlockerInput<'_>, kind: BlockerKind, who: BlockerWho, terminal: bool) {
        assert_eq!(
            blocker(&input),
            Some(BlockerMatch {
                kind,
                who,
                terminal
            }),
            "{input:?}"
        );
    }

    fn typed(direction: Direction, state: TaskState, kind: ReasonKind) -> BlockerInput<'static> {
        BlockerInput {
            direction: Some(direction),
            state: Some(state),
            // Text that would mislead the old parser: the kind wins
            reason: Some("Provider requires trust."),
            reason_kind: Some(kind),
            ..Default::default()
        }
    }

    /// Every typed reason, from both sides (task 5)
    #[test]
    fn every_reason_kind_maps_to_its_blocker() {
        use ReasonKind as R;
        let rows: [(R, BlockerKind, BlockerWho, BlockerWho); 14] = [
            // (kind, blocker, who when incoming, who when outgoing)
            (R::NeedsMyApproval, NeedsMyApproval, Me, Me),
            (R::NeedsTheirApproval, NeedsTheirApproval, Them, Them),
            (R::NeedsMyAnswer, NeedsMyAnswer, Me, Me),
            (R::NeedsTheirAnswer, NeedsTheirAnswer, Them, Them),
            (R::NotTrusted, NotTrusted, Me, Them),
            (R::NotAccepting, NotAccepting, Me, Them),
            (R::PausedOrBlocked, PausedOrBlocked, Me, Them),
            (R::Queued, Queued, Nobody, Nobody),
            (R::Offline, Offline, Nobody, Nobody),
            (R::NoModel, NoModel, Me, Them),
            (R::RuntimeAuth, RuntimeAuth, Me, Them),
            (R::TimedOut, TimedOut, Me, Me),
            (R::Failed, Failed, Me, Me),
            (R::Declined, Declined, Me, Them),
        ];
        assert_eq!(rows.len(), ReasonKind::ALL.len(), "a row per kind");
        for (kind, expected, incoming, outgoing) in rows {
            for (outgoing_side, who) in [(false, incoming), (true, outgoing)] {
                assert_eq!(
                    from_reason_kind(kind, outgoing_side, true),
                    BlockerMatch {
                        kind: expected,
                        who,
                        terminal: true
                    },
                    "{kind:?} outgoing={outgoing_side}"
                );
            }
        }
    }

    #[test]
    fn a_held_or_queued_task_waits_without_ending() {
        expect(
            typed(
                Direction::Incoming,
                TaskState::Delivered,
                ReasonKind::NeedsMyApproval,
            ),
            NeedsMyApproval,
            Me,
            false,
        );
        expect(
            typed(
                Direction::Outgoing,
                TaskState::Delivered,
                ReasonKind::NeedsTheirApproval,
            ),
            NeedsTheirApproval,
            Them,
            false,
        );
        expect(
            typed(
                Direction::Outgoing,
                TaskState::Delivered,
                ReasonKind::Queued,
            ),
            Queued,
            Nobody,
            false,
        );
    }

    #[test]
    fn a_typed_refusal_beats_its_text() {
        // The text says trust; the provider said its agent doesn't take requests
        expect(
            typed(
                Direction::Outgoing,
                TaskState::Rejected,
                ReasonKind::NotAccepting,
            ),
            NotAccepting,
            Them,
            true,
        );
        expect(
            typed(
                Direction::Outgoing,
                TaskState::Rejected,
                ReasonKind::Declined,
            ),
            Declined,
            Them,
            true,
        );
    }

    #[test]
    fn a_kind_on_a_task_that_moved_on_is_no_blocker() {
        // Held, approved, now working: the old note doesn't count
        assert_eq!(
            blocker(&typed(
                Direction::Incoming,
                TaskState::Working,
                ReasonKind::Failed
            )),
            None
        );
        assert_eq!(
            blocker(&typed(
                Direction::Outgoing,
                TaskState::Completed,
                ReasonKind::Queued
            )),
            None
        );
    }

    // One test per row of request-detail.md §5.2, with the strings the
    // executor and the gateway emitted before reasons were typed

    #[test]
    fn needs_my_approval_for_a_run_and_a_workflow_step() {
        let mut run = input(Direction::Incoming, TaskState::Working, "the turn started");
        run.run_status = Some(RunStatus::AwaitingApproval);
        expect(run, NeedsMyApproval, Me, false);
        let mut step = input(Direction::Local, TaskState::Working, "");
        step.workflow_step_awaiting_approval = true;
        expect(step, NeedsMyApproval, Me, false);
    }

    #[test]
    fn needs_their_approval_is_terminal_in_phase_1() {
        expect(
            input(
                Direction::Outgoing,
                TaskState::Rejected,
                "'finance.risk' needs the owner's approval before it runs; ask them directly",
            ),
            NeedsTheirApproval,
            Them,
            true,
        );
    }

    #[test]
    fn needs_my_answer_only_with_a_question_for_the_human() {
        let mut asked = input(Direction::Outgoing, TaskState::InputRequired, "");
        asked.pending_clarification = true;
        expect(asked, NeedsMyAnswer, Me, false);
        // The requester's agent answers by itself
        assert_eq!(
            blocker(&input(Direction::Outgoing, TaskState::InputRequired, "")),
            None
        );
    }

    #[test]
    fn needs_their_answer_on_an_incoming_question() {
        expect(
            input(Direction::Incoming, TaskState::InputRequired, ""),
            NeedsTheirAnswer,
            Them,
            false,
        );
    }

    #[test]
    fn not_trusted_both_ways() {
        let ask = "Provider requires trust. (The owner of 'Papers' needs to add 'ada' to their trusted peers, from your profile in their Network view.)";
        expect(
            input(Direction::Incoming, TaskState::Rejected, ask),
            NotTrusted,
            Me,
            true,
        );
        expect(
            input(Direction::Outgoing, TaskState::Rejected, ask),
            NotTrusted,
            Them,
            true,
        );
        let deny = "'Papers' doesn't take tasks from 'i2/w' (trust policy: deny)";
        expect(
            input(Direction::Outgoing, TaskState::Rejected, deny),
            NotTrusted,
            Them,
            true,
        );
    }

    #[test]
    fn not_accepting_both_ways() {
        for reason in [
            "Provider 'Papers' does not accept external delegation. (They need to set accepts_delegation: true.)",
            "'Papers' doesn't serve 'summarize'",
            "'finance.risk' isn't offered by this gateway",
        ] {
            expect(input(Direction::Incoming, TaskState::Rejected, reason), NotAccepting, Me, true);
            expect(input(Direction::Outgoing, TaskState::Rejected, reason), NotAccepting, Them, true);
        }
    }

    #[test]
    fn paused_or_blocked() {
        for reason in [
            "Delegations with desktop-1 are paused by this instance's owner",
            "desktop-1 is blocked by this instance's owner",
            "Delegations from Sam are turned off",
        ] {
            expect(
                input(Direction::Incoming, TaskState::Rejected, reason),
                PausedOrBlocked,
                Me,
                true,
            );
        }
    }

    #[test]
    fn queued_waits_for_task_5() {
        // Phase 1 refuses when busy: an honest terminal failure
        expect(
            input(
                Direction::Outgoing,
                TaskState::Rejected,
                "The gateway is busy; try again shortly",
            ),
            Failed,
            Me,
            true,
        );
        // The variant exists for task 5's slot queue
        assert_eq!(
            serde_json::to_value(Queued).unwrap(),
            serde_json::json!("queued")
        );
    }

    #[test]
    fn offline_only_while_the_provider_is_away() {
        let mut away = input(Direction::Outgoing, TaskState::Submitted, "");
        away.provider_offline = true;
        expect(away, Offline, Nobody, false);
        assert_eq!(
            blocker(&input(Direction::Outgoing, TaskState::Submitted, "")),
            None
        );
    }

    #[test]
    fn no_model_from_the_run_or_the_router() {
        let mut run = input(
            Direction::Incoming,
            TaskState::Failed,
            "The turn ended as failed",
        );
        run.run_status = Some(RunStatus::Failed);
        run.run_summary = Some(
            "No LLM API key is set. Add one (Anthropic, OpenAI, or DeepSeek) in Settings › Models & runtimes.",
        );
        expect(run, NoModel, Me, true);
        expect(
            input(
                Direction::Incoming,
                TaskState::Failed,
                "Add an API key in Settings › Models & runtimes so the gateway can route requests",
            ),
            NoModel,
            Me,
            true,
        );
    }

    #[test]
    fn runtime_auth_from_the_run() {
        let mut run = input(Direction::Incoming, TaskState::Failed, "");
        run.run_status = Some(RunStatus::Failed);
        run.run_summary = Some("Claude Code: Invalid API key · Please run /login");
        expect(run, RuntimeAuth, Me, true);
    }

    #[test]
    fn timed_out_and_failed() {
        expect(
            input(
                Direction::Outgoing,
                TaskState::TimedOut,
                "'Papers' didn't finish in time",
            ),
            TimedOut,
            Me,
            true,
        );
        expect(
            input(Direction::Outgoing, TaskState::Failed, "boom"),
            Failed,
            Me,
            true,
        );
        expect(
            input(Direction::Outgoing, TaskState::Abandoned, ""),
            Failed,
            Me,
            true,
        );
        // An unknown refusal still gets a banner
        expect(
            input(Direction::Incoming, TaskState::Rejected, "???"),
            Failed,
            Me,
            true,
        );
    }

    #[test]
    fn nothing_while_it_moves_or_once_it_ended_well() {
        for state in [
            TaskState::Delivered,
            TaskState::Acknowledged,
            TaskState::Working,
            TaskState::Completed,
            TaskState::Cancelled,
            TaskState::Reassigned,
        ] {
            assert_eq!(
                blocker(&input(Direction::Outgoing, state, "")),
                None,
                "{state:?}"
            );
        }
        assert_eq!(blocker(&BlockerInput::default()), None);
    }
}
