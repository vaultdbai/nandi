//! `POST /api/rpc/{name}`: any command declared with `rpc::commands!`. The
//! JSON body is the command's arguments (camelCase keys, as over Tauri IPC);
//! the answer is its result, or an error as [`ApiResult`] maps it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};

use crate::services::relay::RelayUnavailable;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new().route("/rpc/{name}", post(call))
}

async fn call(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(args): Json<Value>,
) -> Response {
    match crate::rpc::call(state, &name, args).await {
        Some(result) => ApiResult(result).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("No command {name}") })),
        )
            .into_response(),
    }
}

/// A command's result as an HTTP response: `200` with the value, `400
/// {"error": "..."}` (the same message Tauri IPC returns), `503` for
/// [`RelayUnavailable`], or `501` for a frozen service not built yet.
struct ApiResult<T>(Result<T, String>);

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
