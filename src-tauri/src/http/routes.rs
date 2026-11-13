//! REST endpoints. Each handler maps 1:1 onto a service call; errors become
//! `400 {"error": "..."}` (the same messages the Tauri commands return),
//! or `503` for [`RelayUnavailable`].

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::ApprovalDecision;
use crate::services::agents::{self, CreateAgentInput};
use crate::services::relay::RelayUnavailable;
use crate::services::workflows::{self, CreateWorkflowInput};
use crate::services::{attachments, chat, recovery, snapshots, AppState};

/// A service result as an HTTP response.
pub(super) struct ApiResult<T>(pub(super) Result<T, String>);

impl<T: Serialize> IntoResponse for ApiResult<T> {
    fn into_response(self) -> Response {
        match self.0 {
            Ok(value) => Json(value).into_response(),
            // A frozen service its track hasn't implemented yet
            Err(error) if error == crate::services::redesign::NOT_IMPLEMENTED => {
                (StatusCode::NOT_IMPLEMENTED, Json(json!({ "error": error }))).into_response()
            }
            // A person-to-person action with no relay connected: the same
            // text as the Tauri command, which the UI recognises
            Err(error) if RelayUnavailable::is(&error) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": error })),
            )
                .into_response(),
            Err(error) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
            }
        }
    }
}

/// Recovery errors are typed; send them as-is so the client can match.
struct RecoveryResult<T>(Result<T, crate::types::RecoveryError>);

impl<T: Serialize> IntoResponse for RecoveryResult<T> {
    fn into_response(self) -> Response {
        match self.0 {
            Ok(value) => Json(value).into_response(),
            Err(error) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
            }
        }
    }
}

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        // `?todayStart=<local midnight ms>`; returns `AgentOverview[]`
        .route("/agents/overviews", get(agent_overviews))
        .route("/agents/{id}", delete(delete_agent))
        .route("/agents/{id}/start", post(start_agent))
        .route("/agents/{id}/stop", post(stop_agent))
        .route("/agents/{id}/restart", post(restart_agent))
        .route("/agents/{id}/messages", post(send_message))
        .route("/agents/{id}/approve", post(approve_action))
        // Returns `PendingApproval[]` (every agent's, oldest first)
        .route("/approvals/pending", get(pending_approvals))
        .route("/approvals/{id}/hand-over", post(answer_hand_over))
        .route("/agents/{id}/cancel", post(cancel_turn))
        .route("/snapshots/agent/{id}", get(agent_snapshot))
        .route("/snapshots/dashboard", get(dashboard_snapshot))
        .route("/runs/{id}", get(run_detail))
        // Body `{ path }`; returns the host path or null
        .route("/runs/{id}/file", post(run_file))
        // Returns `RunPlan | null`
        .route("/runs/{id}/plan", get(run_plan))
        // Body `{ path }`; returns a `data:` URL or null
        .route("/runs/{id}/screenshot", post(run_screenshot))
        .route("/workflows", get(list_workflows).post(create_workflow))
        .route("/workflows/{id}", get(get_workflow))
        .route("/workflows/{id}/run", post(run_workflow))
        // GET: `WorkflowTemplate[]`; POST `TemplateWorkflowInput`: `Workflow`
        .route(
            "/workflow-templates",
            get(workflow_templates).post(create_from_template),
        )
        .route("/workflow-runs/{id}/approve", post(approve_workflow_step))
        .route(
            "/studio/conversations",
            get(studio_conversations).post(create_studio_conversation),
        )
        .route(
            "/studio/conversations/{id}/messages",
            post(send_studio_message),
        )
        .route("/conversations/{id}", get(conversation))
        .route("/attachments/{id}", delete(discard_attachment))
        .route("/emergency-data/cleanup", post(cleanup_emergency_data))
        .route(
            "/emergency-data/{filename}",
            get(load_emergency_data).put(save_emergency_data),
        )
}

// --- Agents ----------------------------------------------------------------

async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(agents::list_agents(&state).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverviewQuery {
    #[serde(default)]
    today_start: f64,
}

async fn agent_overviews(
    State(state): State<AppState>,
    Query(query): Query<OverviewQuery>,
) -> impl IntoResponse {
    ApiResult(
        crate::services::agent_overview::list_agent_overviews(&state, query.today_start).await,
    )
}

async fn create_agent(
    State(state): State<AppState>,
    Json(input): Json<CreateAgentInput>,
) -> impl IntoResponse {
    ApiResult(agents::create_agent(&state, input).await)
}

async fn delete_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(agents::delete_agent(&state, &id).await)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContainerStarted {
    container_id: String,
}

async fn start_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(
        agents::start_agent(&state, &id)
            .await
            .map(|container_id| ContainerStarted { container_id }),
    )
}

async fn stop_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(agents::stop_agent(&state, &id).await)
}

async fn restart_agent(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(
        agents::restart_agent(&state, &id)
            .await
            .map(|container_id| ContainerStarted { container_id }),
    )
}

// --- Chat ------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendBody {
    text: String,
    #[serde(default)]
    attachments: Vec<String>,
    #[serde(default)]
    client_message_id: Option<String>,
}

async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendBody>,
) -> impl IntoResponse {
    ApiResult(
        chat::send_message(
            &state,
            id,
            body.text,
            body.attachments,
            body.client_message_id,
        )
        .await,
    )
}

async fn send_studio_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendBody>,
) -> impl IntoResponse {
    ApiResult(
        chat::send_studio_message(
            &state,
            id,
            body.text,
            body.attachments,
            body.client_message_id,
        )
        .await,
    )
}

async fn create_studio_conversation(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(chat::create_studio_conversation(&state).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApproveBody {
    approval_id: String,
    decision: ApprovalDecision,
}

async fn pending_approvals(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(chat::list_pending_approvals(&state).await)
}

async fn approve_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> impl IntoResponse {
    ApiResult(chat::approve_action(&state, Some(&id), body.approval_id, body.decision).await)
}

async fn answer_hand_over(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(answer): Json<crate::services::pilots::handover::HandOverAnswer>,
) -> impl IntoResponse {
    ApiResult(chat::answer_hand_over(&state, id, answer).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelBody {
    turn_id: String,
}

async fn cancel_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CancelBody>,
) -> impl IntoResponse {
    ApiResult(chat::cancel_turn(&state, id, body.turn_id).await)
}

async fn discard_attachment(Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(attachments::discard_attachment(id).await)
}

// --- Snapshots -------------------------------------------------------------

async fn agent_snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ApiResult(snapshots::get_agent_snapshot(&state, id).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardQuery {
    day_start: f64,
}

async fn dashboard_snapshot(
    State(state): State<AppState>,
    Query(query): Query<DashboardQuery>,
) -> impl IntoResponse {
    ApiResult(snapshots::get_dashboard_snapshot(&state, query.day_start).await)
}

async fn run_detail(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(snapshots::get_run_detail(&state, id).await)
}

#[derive(Deserialize)]
struct RunFileBody {
    #[serde(default)]
    path: String,
}

/// The host path of a file the run touched (opening it is desktop-only)
async fn run_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunFileBody>,
) -> impl IntoResponse {
    ApiResult(snapshots::run_file_path(&state, id, body.path).await)
}

async fn run_plan(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(snapshots::get_run_plan(&state, id).await)
}

/// One of the run's screenshots, for its plan card
async fn run_screenshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunFileBody>,
) -> impl IntoResponse {
    ApiResult(snapshots::run_screenshot(&state, id, body.path).await)
}

async fn list_workflows(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(snapshots::get_workflows(&state).await)
}

async fn get_workflow(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(workflows::get_workflow(&state, id).await)
}

async fn create_workflow(
    State(state): State<AppState>,
    Json(input): Json<CreateWorkflowInput>,
) -> impl IntoResponse {
    ApiResult(workflows::create_workflow(&state, input).await)
}

async fn run_workflow(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(workflows::run_workflow(&state, id).await)
}

async fn workflow_templates() -> impl IntoResponse {
    ApiResult(Ok::<_, String>(workflows::list_workflow_templates()))
}

async fn create_from_template(
    State(state): State<AppState>,
    Json(input): Json<workflows::TemplateWorkflowInput>,
) -> impl IntoResponse {
    ApiResult(workflows::create_workflow_from_template(&state, input).await)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApproveStepBody {
    step_id: String,
    decision: ApprovalDecision,
}

async fn approve_workflow_step(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(body): Json<ApproveStepBody>,
) -> impl IntoResponse {
    ApiResult(workflows::approve_workflow_step(&state, run_id, body.step_id, body.decision).await)
}

async fn studio_conversations(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(snapshots::get_studio_conversations(&state).await)
}

async fn conversation(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(snapshots::get_conversation(&state, id).await)
}

// --- Settings --------------------------------------------------------------

async fn save_emergency_data(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    Json(data): Json<Value>,
) -> impl IntoResponse {
    RecoveryResult(recovery::save_emergency_data(&state, filename, data).await)
}

async fn load_emergency_data(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    RecoveryResult(recovery::load_emergency_data(&state, filename).await)
}

async fn cleanup_emergency_data(State(state): State<AppState>) -> impl IntoResponse {
    RecoveryResult(recovery::cleanup_old_recovery_files(&state).await)
}
