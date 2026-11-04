//! My notes routes (`/api/notes`), mirroring `commands/notes.rs`. A note's
//! name is a query parameter (`?name=runbooks/etl.md`): it may hold `/`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::notes;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `MyNotes` (creates and shares the folder on first use)
        .route("/notes", get(list))
        // GET returns the text; PUT body `{"content": "..."}` returns
        // `NoteFile`; DELETE removes it
        .route("/notes/file", get(read).put(save).delete(delete))
        // PUT body `{"allow": bool}`: agents may update the notes; returns
        // `MyNotes`
        .route("/notes/agents-can-write", axum::routing::put(set_write))
}

#[derive(Deserialize)]
struct WriteBody {
    #[serde(default)]
    allow: bool,
}

async fn set_write(
    State(state): State<AppState>,
    Json(body): Json<WriteBody>,
) -> impl IntoResponse {
    ApiResult(notes::set_agents_can_write(&state, body.allow).await)
}

#[derive(Deserialize)]
struct NameQuery {
    #[serde(default)]
    name: String,
}

// A missing field is a readable 400 from the service, not a bare 422
#[derive(Deserialize)]
struct SaveBody {
    #[serde(default)]
    content: String,
}

async fn list(State(state): State<AppState>) -> impl IntoResponse {
    ApiResult(notes::my_notes(&state).await)
}

async fn read(State(state): State<AppState>, Query(query): Query<NameQuery>) -> impl IntoResponse {
    ApiResult(notes::read_note(&state, query.name).await)
}

async fn save(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
    Json(body): Json<SaveBody>,
) -> impl IntoResponse {
    ApiResult(notes::save_note(&state, query.name, body.content).await)
}

async fn delete(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
) -> impl IntoResponse {
    ApiResult(notes::delete_note(&state, query.name).await)
}
