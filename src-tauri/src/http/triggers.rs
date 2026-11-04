//! Trigger routes: CRUD (token auth, like every route) and the webhook and
//! device receivers, which authenticate with the trigger's own secret
//! instead (see `auth::is_trigger_receiver`) and answer `202` without
//! waiting for the dispatch.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use super::routes::ApiResult;
use crate::services::triggers::device::DeviceEvent;
use crate::services::triggers::schema::TriggerInput;
use crate::services::triggers::webhook::{ReceiveError, SECRET_HEADER};
use crate::services::triggers::{self, MAX_PAYLOAD_BYTES};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/triggers", get(list).post(create))
        // Body `{ schedule }`; returns `CronPreview`
        .route("/triggers/cron-preview", post(cron_preview))
        // Body: a calendar config; returns `CalendarPreview`
        .route("/triggers/calendar-preview", post(calendar_preview))
        .route("/triggers/{id}", get(get_one).put(update).delete(remove))
        .route("/triggers/{id}/enable", post(enable))
        .route("/triggers/{id}/disable", post(disable))
        .route("/triggers/{id}/secret", post(rotate_secret))
        .route("/triggers/{id}/test", post(test_fire))
        .route("/triggers/{id}/fires", get(fires))
        .route("/triggers/webhook/{id}", post(webhook))
        .route("/triggers/device/{device_id}", post(device))
}

#[derive(Deserialize)]
struct CronPreviewBody {
    #[serde(default)]
    schedule: String,
}

async fn cron_preview(Json(body): Json<CronPreviewBody>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(triggers::cron::preview(
        &body.schedule,
        chrono::Local::now(),
        triggers::cron::PREVIEW_COUNT,
    )))
}

async fn calendar_preview(Json(config): Json<serde_json::Value>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(triggers::calendar::preview(
        &config,
        chrono::Local::now(),
        triggers::calendar::PREVIEW_COUNT,
    )))
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(triggers::list_triggers(&state).await)
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<TriggerInput>,
) -> impl IntoResponse {
    ApiResult(triggers::create_trigger(&state, input).await)
}

async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(triggers::get_trigger(&state, id).await)
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<TriggerInput>,
) -> impl IntoResponse {
    ApiResult(triggers::update_trigger(&state, id, input).await)
}

async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(triggers::delete_trigger(&state, id).await)
}

async fn enable(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(triggers::set_trigger_enabled(&state, id, true).await)
}

async fn disable(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(triggers::set_trigger_enabled(&state, id, false).await)
}

async fn rotate_secret(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    ApiResult(triggers::rotate_trigger_secret(&state, id).await)
}

async fn test_fire(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> impl IntoResponse {
    ApiResult(triggers::test_fire_trigger(&state, id, body.map(|Json(v)| v)).await)
}

#[derive(Deserialize)]
struct FiresQuery {
    limit: Option<u32>,
}

async fn fires(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<FiresQuery>,
) -> impl IntoResponse {
    ApiResult(triggers::list_trigger_fires(&state, id, query.limit).await)
}

// --- Receivers ---------------------------------------------------------------

fn refused(error: ReceiveError) -> Response {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::BAD_REQUEST);
    (status, Json(json!({ "error": error.message() }))).into_response()
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

fn secret(headers: &HeaderMap) -> Option<&str> {
    headers.get(SECRET_HEADER)?.to_str().ok()
}

/// The JSON body (an empty body is `{}`), or the response refusing it.
fn payload(body: &Bytes) -> Result<Value, Box<Response>> {
    if body.len() > MAX_PAYLOAD_BYTES {
        return Err(Box::new(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({ "error": "Payload is too large" })),
            )
                .into_response(),
        ));
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    serde_json::from_slice(body).map_err(|_| Box::new(bad_request("Body must be JSON")))
}

async fn webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let payload = match payload(&body) {
        Ok(payload) => payload,
        Err(response) => return *response,
    };
    match state
        .triggers()
        .receive_webhook(&id, secret(&headers), payload)
    {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response(),
        Err(error) => refused(error),
    }
}

async fn device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let event: DeviceEvent = match payload(&body).map(serde_json::from_value) {
        Ok(Ok(event)) => event,
        Ok(Err(_)) => return bad_request("Body must be {\"event\": \"…\", \"data\": {…}}"),
        Err(response) => return *response,
    };
    match state
        .triggers()
        .receive_device(&device_id, secret(&headers), event)
    {
        Ok(fired) => (
            StatusCode::ACCEPTED,
            Json(json!({ "accepted": true, "fired": fired })),
        )
            .into_response(),
        Err(error) => refused(error),
    }
}
