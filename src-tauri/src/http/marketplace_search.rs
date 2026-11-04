//! Marketplace search route, mirroring `commands/marketplace_search.rs`;
//! errors are `400 {"error": "..."}`.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use super::routes::ApiResult;
use crate::services::marketplace::search;
use crate::services::AppState;

pub fn api() -> Router<AppState> {
    Router::new()
        // `?q=`; returns `MarketplaceSearchHit[]`
        .route("/marketplace/search", get(search_catalog))
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

async fn search_catalog(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    ApiResult(search::search(&state, &query.q).await)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn a_blank_query_answers_no_hits() {
        let state = AppState::for_tests(vec![]);
        let app = api().with_state(state.clone());
        let response = app
            .oneshot(
                Request::get("/marketplace/search?q=%20")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"[]");
        state.shutdown();
    }
}
