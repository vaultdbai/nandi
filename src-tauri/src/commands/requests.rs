//! Request commands (Tauri adapter for `services::requests`).

use tauri::State;

pub use crate::services::requests::digest::ChiefOfStaffDigest;
pub use crate::services::requests::Request;

use crate::services::ask::ActionPlan;
use crate::services::chat::SendMessageResult;
use crate::services::inbox::thread::InboxThread;
use crate::services::intake;
use crate::services::intake::rules::AutoApproveRule;
use crate::services::intake::thread::IntakeThread;
use crate::services::requests::{self, digest, local};
use crate::services::AppState;

/// One Request with its Steps; `id` is any of its task IDs
#[tauri::command]
#[specta::specta]
pub async fn get_request(state: State<'_, AppState>, id: String) -> Result<Request, String> {
    requests::get_request(&state, id).await
}

/// The Request a run is part of, if any
#[tauri::command]
#[specta::specta]
pub async fn request_for_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<String>, String> {
    requests::request_for_run(&state, run_id).await
}

/// A failed outgoing Request tried again on the next provider (attempt
/// n + 1, linked to the last one)
#[tauri::command]
#[specta::specta]
pub async fn retry_task(state: State<'_, AppState>, id: String) -> Result<Request, String> {
    requests::retry_task(&state, id).await
}

/// Saves an ask's approved plan as a Request and starts it; returns its ID
#[tauri::command]
#[specta::specta]
pub async fn start_request(
    state: State<'_, AppState>,
    plan: ActionPlan,
    text: String,
) -> Result<String, String> {
    local::start_request(&state, plan, text).await
}

/// What the Chief of Staff did since the user last looked, and the work
/// waiting on them
#[tauri::command]
#[specta::specta]
pub async fn get_chief_of_staff_digest(
    state: State<'_, AppState>,
) -> Result<ChiefOfStaffDigest, String> {
    digest::get(&state).await
}

/// The user has seen the digest: only what waits on them stays
#[tauri::command]
#[specta::specta]
pub async fn mark_chief_of_staff_digest_seen(state: State<'_, AppState>) -> Result<(), String> {
    digest::mark_seen(&state).await
}

/// Approves work that arrived from outside: it starts, or its news reaches
/// its Request. Returns the Request's ID. Blocked items can't be approved.
#[tauri::command]
#[specta::specta]
pub async fn approve_queued_work(state: State<'_, AppState>, id: String) -> Result<String, String> {
    intake::approve(&state, id).await
}

/// An intake thread (`intake:<id>`): what was queued in it and the
/// Requests it became. Its messages come from `get_conversation`.
#[tauri::command]
#[specta::specta]
pub async fn get_intake_thread(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<IntakeThread, String> {
    intake::thread::get(&state, conversation_id).await
}

/// The user's message in an intake thread: a Chief of Staff turn there
#[tauri::command]
#[specta::specta]
pub async fn send_intake_message(
    state: State<'_, AppState>,
    conversation_id: String,
    text: String,
    client_message_id: Option<String>,
) -> Result<SendMessageResult, String> {
    crate::services::inbox::thread::send(&state, conversation_id, text, client_message_id).await
}

/// An Inbox item's Ask chat (`inbox:<kind>:<id>`, or an intake item's
/// `intake:<id>`): made the first time it's opened. Its messages come from
/// `get_conversation`.
#[tauri::command]
#[specta::specta]
pub async fn open_inbox_thread(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<InboxThread, String> {
    crate::services::inbox::thread::open(&state, conversation_id).await
}

/// The user's message in an Inbox item's chat: a Chief of Staff turn there
#[tauri::command]
#[specta::specta]
pub async fn send_inbox_message(
    state: State<'_, AppState>,
    conversation_id: String,
    text: String,
    client_message_id: Option<String>,
) -> Result<SendMessageResult, String> {
    crate::services::inbox::thread::send(&state, conversation_id, text, client_message_id).await
}

/// What may skip the approval queue (`intake::rules`)
#[tauri::command]
#[specta::specta]
pub async fn get_auto_approve_rules(
    state: State<'_, AppState>,
) -> Result<Vec<AutoApproveRule>, String> {
    intake::get_rules(&state).await
}

/// Replaces the auto-approve rules (checked first)
#[tauri::command]
#[specta::specta]
pub async fn set_auto_approve_rules(
    state: State<'_, AppState>,
    rules: Vec<AutoApproveRule>,
) -> Result<(), String> {
    intake::set_rules(&state, rules).await
}

/// Drops work that arrived from outside without doing it
#[tauri::command]
#[specta::specta]
pub async fn dismiss_queued_work(state: State<'_, AppState>, id: String) -> Result<(), String> {
    intake::dismiss(&state, id).await
}

/// Marks a step of a local Request that waits on the user done
#[tauri::command]
#[specta::specta]
pub async fn complete_request_step(
    state: State<'_, AppState>,
    id: String,
    position: u32,
) -> Result<(), String> {
    local::complete_request_step(&state, id, position).await
}

/// Stops a local Request and what its steps started
#[tauri::command]
#[specta::specta]
pub async fn cancel_request(state: State<'_, AppState>, id: String) -> Result<(), String> {
    local::cancel_request(&state, id).await
}

/// Do this again: the same plan as a new Request; returns its ID
#[tauri::command]
#[specta::specta]
pub async fn rerun_request(state: State<'_, AppState>, id: String) -> Result<String, String> {
    local::rerun_request(&state, id).await
}

/// Save as workflow: a workflow file drafted from a Request's plan, for the
/// user to confirm (`createWorkflow` validates it)
#[tauri::command]
#[specta::specta]
pub async fn draft_workflow_from_request(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    local::draft_workflow_from_request(&state, id).await
}
