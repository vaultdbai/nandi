//! Emergency-data routes, for the browser. Recovery's errors are typed
//! ([`RecoveryError`]), which `rpc::commands!` doesn't carry, so these and
//! the Tauri commands in `commands/recovery.rs` stay hand-written. The error
//! is sent as-is so the client can match on it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};

use crate::services::{recovery, AppState};
use crate::types::RecoveryError;

pub fn api() -> Router<AppState> {
    Router::new()
        .route("/emergency-data/cleanup", post(cleanup))
        .route("/emergency-data/{filename}", get(load).put(save))
}

struct RecoveryResult<T>(Result<T, RecoveryError>);

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

async fn save(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    Json(data): Json<Value>,
) -> impl IntoResponse {
    RecoveryResult(recovery::save_emergency_data(&state, filename, data).await)
}

async fn load(State(state): State<AppState>, Path(filename): Path<String>) -> impl IntoResponse {
    RecoveryResult(recovery::load_emergency_data(&state, filename).await)
}

async fn cleanup(State(state): State<AppState>) -> impl IntoResponse {
    RecoveryResult(recovery::cleanup_old_recovery_files(&state).await)
}
