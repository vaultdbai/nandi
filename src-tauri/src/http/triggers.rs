//! The webhook and device receivers. They authenticate with the trigger's
//! own secret instead of the web token (see `auth::is_trigger_receiver`) and
//! answer `202` without waiting for the dispatch. Managing triggers is
//! `rpc/triggers.rs`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::services::triggers::device::DeviceEvent;
use crate::services::triggers::webhook::{ReceiveError, SECRET_HEADER};
use crate::services::triggers::MAX_PAYLOAD_BYTES;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/triggers/webhook/{id}", post(webhook))
        .route("/triggers/device/{device_id}", post(device))
}

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
