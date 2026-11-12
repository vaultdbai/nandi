//! Requests, the Chief of Staff's queue and digest, and Inbox threads.

use crate::services::ask::ActionPlan;
use crate::services::chat::SendMessageResult;
use crate::services::inbox::thread::{self as inbox_thread, InboxThread};
use crate::services::intake::rules::AutoApproveRule;
use crate::services::intake::{self, thread::IntakeThread};
use crate::services::requests::digest::{self, ChiefOfStaffDigest};
use crate::services::requests::{self, local, Request};

super::commands! {
    /// One Request with its Steps; `id` is any of its task IDs
    fn get_request(state, id: String) -> Request => requests::get_request(&state, id).await;

    /// The Request a run is part of, if any
    fn request_for_run(state, run_id: String) -> Option<String> => requests::request_for_run(&state, run_id).await;

    /// A failed outgoing Request tried again on the next provider (attempt
    /// n + 1, linked to the last one)
    fn retry_task(state, id: String) -> Request => requests::retry_task(&state, id).await;

    /// Saves an ask's approved plan as a Request and starts it; returns its ID
    fn start_request(state, plan: ActionPlan, text: String) -> String => local::start_request(&state, plan, text).await;

    /// What the Chief of Staff did since the user last looked, and the work
    /// waiting on them
    fn get_chief_of_staff_digest(state) -> ChiefOfStaffDigest => digest::get(&state).await;

    /// The user has seen the digest: only what waits on them stays
    fn mark_chief_of_staff_digest_seen(state) -> () => digest::mark_seen(&state).await;

    /// Approves work that arrived from outside: it starts, or its news reaches
    /// its Request. Returns the Request's ID. Blocked items can't be approved.
    fn approve_queued_work(state, id: String) -> String => intake::approve(&state, id).await;

    /// Drops work that arrived from outside without doing it
    fn dismiss_queued_work(state, id: String) -> () => intake::dismiss(&state, id).await;

    /// An intake thread (`intake:<id>`): what was queued in it and the
    /// Requests it became. Its messages come from `get_conversation`.
    fn get_intake_thread(state, conversation_id: String) -> IntakeThread => intake::thread::get(&state, conversation_id).await;

    /// The user's message in an intake thread: a Chief of Staff turn there
    fn send_intake_message(state, conversation_id: String, text: String, client_message_id: Option<String>) -> SendMessageResult => inbox_thread::send(&state, conversation_id, text, client_message_id).await;

    /// An Inbox item's Ask chat (`inbox:<kind>:<id>`, or an intake item's
    /// `intake:<id>`): made the first time it's opened. Its messages come from
    /// `get_conversation`.
    fn open_inbox_thread(state, conversation_id: String) -> InboxThread => inbox_thread::open(&state, conversation_id).await;

    /// The user's message in an Inbox item's chat: a Chief of Staff turn there
    fn send_inbox_message(state, conversation_id: String, text: String, client_message_id: Option<String>) -> SendMessageResult => inbox_thread::send(&state, conversation_id, text, client_message_id).await;

    /// What may skip the approval queue (`intake::rules`)
    fn get_auto_approve_rules(state) -> Vec<AutoApproveRule> => intake::get_rules(&state).await;

    /// Replaces the auto-approve rules (checked first)
    fn set_auto_approve_rules(state, rules: Vec<AutoApproveRule>) -> () => intake::set_rules(&state, rules).await;

    /// Marks a step of a local Request that waits on the user done
    fn complete_request_step(state, id: String, position: u32) -> () => local::complete_request_step(&state, id, position).await;

    /// Stops a local Request and what its steps started
    fn cancel_request(state, id: String) -> () => local::cancel_request(&state, id).await;

    /// Do this again: the same plan as a new Request; returns its ID
    fn rerun_request(state, id: String) -> String => local::rerun_request(&state, id).await;

    /// Save as workflow: a workflow file drafted from a Request's plan, for the
    /// user to confirm (`createWorkflow` validates it)
    fn draft_workflow_from_request(state, id: String) -> String => local::draft_workflow_from_request(&state, id).await;
}
