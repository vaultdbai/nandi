//! Request routes (`/api/requests/...`), mirroring `commands/requests.rs`.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::ask::ActionPlan;
use crate::services::requests::local;
use crate::services::{requests, AppState};

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `Request` (any of its task IDs works)
        .route("/requests/{id}", get(detail))
        // Returns `Request` after a new attempt on the next provider
        .route("/requests/{id}/retry", post(retry))
        // Returns the Request ID a run is part of, or `null`
        .route("/runs/{id}/request", get(for_run))
        // Body `{ plan, text }`; returns the new Request's ID
        .route("/requests", post(start))
        // Returns `null` once the step waiting on the user is done
        .route(
            "/requests/{id}/steps/{position}/complete",
            post(complete_step),
        )
        // Returns `null` once the local Request is stopped
        .route("/requests/{id}/cancel", post(cancel))
        // Returns the new Request's ID (the same plan again)
        .route("/requests/{id}/rerun", post(rerun))
        // Returns the drafted workflow YAML
        .route("/requests/{id}/workflow-draft", post(draft_workflow))
        // Returns `ChiefOfStaffDigest`
        .route("/chief-of-staff/digest", get(digest))
        // Returns `null` once it's marked seen
        .route("/chief-of-staff/digest/seen", post(digest_seen))
        // Returns the Request's ID once the queued work starts or reaches it
        .route("/chief-of-staff/queue/{id}/approve", post(approve_queued))
        // Returns `null` once it's dismissed
        .route("/chief-of-staff/queue/{id}/dismiss", post(dismiss_queued))
        // Returns `IntakeThread` (`id`: the thread's conversation ID)
        .route("/chief-of-staff/threads/{id}", get(intake_thread))
        // Body `{ text, clientMessageId? }`; returns `SendMessageResult`
        .route(
            "/chief-of-staff/threads/{id}/messages",
            post(intake_message),
        )
        // Returns `InboxThread`, made the first time (`id`: the item's
        // conversation ID, `inbox:<kind>:<id>` or `intake:<id>`)
        .route("/inbox/threads/{id}", post(open_inbox_thread))
        // Body `{ text, clientMessageId? }`; returns `SendMessageResult`
        .route("/inbox/threads/{id}/messages", post(inbox_message))
        // Returns the auto-approve rules; PUT a list to replace them
        .route(
            "/chief-of-staff/auto-approve",
            get(auto_approve_rules).put(set_auto_approve_rules),
        )
}

async fn auto_approve_rules(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(crate::services::intake::get_rules(&state).await)
}

async fn set_auto_approve_rules(
    State(state): State<AppState>,
    Json(rules): Json<Vec<crate::services::intake::rules::AutoApproveRule>>,
) -> impl IntoResponse {
    ApiResult(crate::services::intake::set_rules(&state, rules).await)
}

async fn intake_thread(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(crate::services::intake::thread::get(&state, id).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadMessageBody {
    text: String,
    client_message_id: Option<String>,
}

async fn intake_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ThreadMessageBody>,
) -> impl IntoResponse {
    ApiResult(
        crate::services::inbox::thread::send(&state, id, body.text, body.client_message_id).await,
    )
}

async fn open_inbox_thread(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(crate::services::inbox::thread::open(&state, id).await)
}

async fn inbox_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ThreadMessageBody>,
) -> impl IntoResponse {
    ApiResult(
        crate::services::inbox::thread::send(&state, id, body.text, body.client_message_id).await,
    )
}

async fn approve_queued(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(crate::services::intake::approve(&state, id).await)
}

async fn dismiss_queued(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(crate::services::intake::dismiss(&state, id).await)
}

async fn digest(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(requests::digest::get(&state).await)
}

async fn digest_seen(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(requests::digest::mark_seen(&state).await)
}

#[derive(Deserialize)]
struct StartBody {
    plan: ActionPlan,
    text: String,
}

async fn start(State(state): State<AppState>, Json(body): Json<StartBody>) -> impl IntoResponse {
    ApiResult(local::start_request(&state, body.plan, body.text).await)
}

async fn complete_step(
    State(state): State<AppState>,
    Path((id, position)): Path<(String, u32)>,
) -> impl IntoResponse {
    ApiResult(local::complete_request_step(&state, id, position).await)
}

async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(local::cancel_request(&state, id).await)
}

async fn rerun(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(local::rerun_request(&state, id).await)
}

async fn draft_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(local::draft_workflow_from_request(&state, id).await)
}

async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(requests::get_request(&state, id).await)
}

async fn retry(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(requests::retry_task(&state, id).await)
}

async fn for_run(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(requests::request_for_run(&state, id).await)
}
