//! Mascot routes (`/api/mascot`), mirroring `commands/mascot.rs`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::{mascot, AppState};

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `MascotInfo`; PUT `{ name }` (null resets) renames
        .route("/mascot", get(read).put(write))
        // Body `{ input }`; returns `PhraseResponse` or `null`
        .route("/mascot/phrase", post(phrase))
}

#[derive(Deserialize)]
struct NameBody {
    name: Option<String>,
}

#[derive(Deserialize)]
struct PhraseBody {
    input: String,
}

async fn read(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(Ok::<_, String>(mascot::read(state.nandi_dir())))
}

async fn write(State(state): State<AppState>, Json(body): Json<NameBody>) -> impl IntoResponse {
    ApiResult(mascot::write(state.nandi_dir(), body.name.as_deref()))
}

async fn phrase(State(state): State<AppState>, Json(body): Json<PhraseBody>) -> impl IntoResponse {
    let name = mascot::read(state.nandi_dir()).name;
    ApiResult(Ok::<_, String>(mascot::handle_phrase(&body.input, &name)))
}
