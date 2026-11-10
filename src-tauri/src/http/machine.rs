//! "Get your computer ready" routes (`/api/machine/...`), mirroring
//! `commands/machine.rs`.

use axum::extract::Query;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::machine::{self, MachineReport};
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // Returns `MachineReport`; `?fresh=true` re-runs the slower checks
        .route("/machine/readiness", get(readiness))
        // Launches Docker Desktop (no admin rights)
        .route("/machine/start-docker", post(start_docker))
}

#[derive(Deserialize)]
struct ReadinessQuery {
    #[serde(default)]
    fresh: bool,
}

async fn readiness(Query(query): Query<ReadinessQuery>) -> ApiResult<MachineReport> {
    ApiResult(Ok(machine::check(query.fresh).await))
}

async fn start_docker() -> ApiResult<()> {
    ApiResult(machine::start_docker().await)
}
