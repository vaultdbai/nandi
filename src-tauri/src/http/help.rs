//! The built-in help guide's routes (`/api/help/...`), mirroring
//! `commands/help.rs`.

use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::help::{self, HelpAnswer, HelpReason, HelpTopic};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns every `HelpTopic`
        .route("/help/topics", get(topics))
        // Body `{ text }`; returns `HelpAnswer`
        .route("/help/answer", post(answer))
}

async fn topics() -> ApiResult<Vec<HelpTopic>> {
    ApiResult(Ok(help::topics()))
}

#[derive(Deserialize)]
struct AnswerBody {
    text: String,
}

async fn answer(Json(body): Json<AnswerBody>) -> ApiResult<HelpAnswer> {
    ApiResult(Ok(help::answer(&body.text, HelpReason::NoModel, None)))
}
