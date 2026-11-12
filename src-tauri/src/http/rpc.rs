//! `POST /api/rpc/{name}`: any command declared with `rpc::commands!`. The
//! JSON body is the command's arguments (camelCase keys, as over Tauri IPC);
//! the answer is its result, or an error as [`ApiResult`] maps it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use super::routes::ApiResult;
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
